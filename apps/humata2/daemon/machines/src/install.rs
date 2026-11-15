//! Streaming humatad2, its holder and the `humata` CLI onto a machine (MP-7,
//! MP-19, TS-1): the bytes go in on stdin, the far end checks their size and
//! SHA-256 and only then moves them into `~/.humata2/bin`.
//!
//! A file a running holder may use (the holder itself and `humata`) is never
//! swapped under it: while any `humata2_holder` of the user runs, the
//! verified copy waits beside the old one as `.<name>.staged`, and the next
//! connect (or [`crate::local`] on the host) moves it in once none runs.

use std::collections::hash_map::RandomState;
use std::fmt;
use std::hash::{BuildHasher, Hasher};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::link::BIN_DIR;
use crate::platform::Platform;
use crate::runner::Exit;

/// The install script's exit codes for what it found wrong.
const CHECKSUM_MISMATCH: i32 = 65;
const TRUNCATED: i32 = 66;
const WRITE_FAILED: i32 = 67;
const NO_CHECKSUM_TOOL: i32 = 68;
/// Not a problem: the file was verified and staged, because a holder runs.
const STAGED: i32 = 69;

/// A shell function that succeeds while any `humata2_holder` of this user
/// runs. `-x` matches the process name, so the script's own text (which
/// names the holder) never counts.
pub(crate) const HOLDERS_FN: &str = r#"holders() { if command -v pgrep >/dev/null 2>&1; then pgrep -u "$(id -u)" -x humata2_holder >/dev/null 2>&1; else ps -u "$(id -u)" -o comm= 2>/dev/null | grep -qx humata2_holder; fi; }
"#;

/// A file Humata installs on every machine, and on the host (TS-0). The
/// holder and `humata` sit beside humatad2, where it looks for them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum InstalledFile {
    Humatad,
    Holder,
    Cli,
}

impl InstalledFile {
    pub const ALL: [Self; 3] = [Self::Humatad, Self::Holder, Self::Cli];

    pub fn file_name(self) -> &'static str {
        match self {
            Self::Humatad => "humatad2",
            Self::Holder => "humata2_holder",
            Self::Cli => "humata",
        }
    }

    /// Whether a running holder may use this file, so it waits for no
    /// holder to run before it is swapped. humatad2 is swapped at once: the
    /// service step restarts it, and holders outlive it.
    pub fn waits_for_holders(self) -> bool {
        !matches!(self, Self::Humatad)
    }

    /// The name a verified copy waits under while a holder runs.
    pub(crate) fn staged_name(self) -> String {
        format!(".{}.staged", self.file_name())
    }
}

impl fmt::Display for InstalledFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.file_name())
    }
}

/// One file's bytes for one platform, with their SHA-256.
#[derive(Clone, PartialEq, Eq)]
pub struct Payload {
    file: InstalledFile,
    bytes: Arc<[u8]>,
    sha256: String,
}

impl Payload {
    pub fn new(file: InstalledFile, bytes: Arc<[u8]>) -> Self {
        let sha256 = hex(&Sha256::digest(&bytes));
        Self {
            file,
            bytes,
            sha256,
        }
    }

    pub fn file(&self) -> InstalledFile {
        self.file
    }

    pub fn bytes(&self) -> &Arc<[u8]> {
        &self.bytes
    }

    pub fn sha256(&self) -> &str {
        &self.sha256
    }
}

impl fmt::Debug for Payload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Payload")
            .field("file", &self.file)
            .field("len", &self.bytes.len())
            .field("sha256", &self.sha256)
            .finish()
    }
}

/// Where setup gets the files to install: the daemon's tools bundle
/// ([`crate::Bundle`]) or a test's fakes.
pub trait Builds {
    /// Every [`InstalledFile`] for `platform`, or why there are none.
    fn payloads(&self, platform: Platform) -> Result<Vec<Payload>, BuildsError>;
}

/// Why a bundle can't give the files for a platform.
#[derive(Debug, Clone, Error)]
pub enum BuildsError {
    #[error(
        "this Humata has no build for {platform} (it ships {shipped}); use a machine of one of those platforms"
    )]
    NoBuild { platform: Platform, shipped: String },
    #[error(
        "couldn't read Humata's tools manifest {path}: {source}. Humata's install is incomplete; reinstall Humata"
    )]
    ManifestUnreadable {
        path: std::path::PathBuf,
        #[source]
        source: Arc<std::io::Error>,
    },
    #[error(
        "Humata's tools manifest {path} isn't valid ({source}). Humata's install is damaged; reinstall Humata"
    )]
    ManifestInvalid {
        path: std::path::PathBuf,
        #[source]
        source: Arc<serde_json::Error>,
    },
    #[error(
        "the manifest has no {file} for {platform}, so Humata's install is incomplete; reinstall Humata"
    )]
    FileMissing { platform: Platform, file: InstalledFile },
    #[error(
        "couldn't read Humata's {file} for {platform} at {path}: {source}. Humata's install is incomplete; reinstall Humata"
    )]
    FileUnreadable {
        platform: Platform,
        file: InstalledFile,
        path: std::path::PathBuf,
        #[source]
        source: Arc<std::io::Error>,
    },
    #[error(
        "Humata's own {file} for {platform} at {path} has SHA-256 {found}, not the {expected} its manifest says, so it was not installed anywhere. The copy in Humata's install is damaged or was replaced; reinstall Humata from a release you trust rather than updating the manifest"
    )]
    Damaged {
        platform: Platform,
        file: InstalledFile,
        path: std::path::PathBuf,
        expected: String,
        found: String,
    },
}

impl PartialEq for BuildsError {
    /// Equal when they report the same thing; an I/O or JSON source counts
    /// by its kind and position only.
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::NoBuild { platform: a, shipped: x }, Self::NoBuild { platform: b, shipped: y }) => {
                a == b && x == y
            }
            (Self::ManifestUnreadable { path: a, source: x }, Self::ManifestUnreadable { path: b, source: y }) => {
                a == b && x.kind() == y.kind()
            }
            (Self::ManifestInvalid { path: a, source: x }, Self::ManifestInvalid { path: b, source: y }) => {
                a == b && x.line() == y.line() && x.column() == y.column()
            }
            (
                Self::FileMissing { platform: a, file: x },
                Self::FileMissing { platform: b, file: y },
            ) => a == b && x == y,
            (
                Self::FileUnreadable { platform: a, file: x, path: p, source: s },
                Self::FileUnreadable { platform: b, file: y, path: q, source: t },
            ) => a == b && x == y && p == q && s.kind() == t.kind(),
            (
                Self::Damaged { platform: a, file: x, path: p, expected: e, found: f },
                Self::Damaged { platform: b, file: y, path: q, expected: g, found: h },
            ) => a == b && x == y && p == q && e == g && f == h,
            _ => false,
        }
    }
}

impl Eq for BuildsError {}

/// How an install that didn't fail ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Installed {
    /// The new copy is in place.
    Swapped,
    /// A holder runs, so the verified copy waits as `.<name>.staged`.
    Staged,
}

impl Installed {
    /// What a successful exit means; none for a failure.
    pub(crate) fn from_exit(exit: Exit) -> Option<Self> {
        match exit {
            Exit::Code(0) => Some(Self::Swapped),
            Exit::Code(STAGED) => Some(Self::Staged),
            _ => None,
        }
    }
}

/// What went wrong installing one file.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum InstallProblem {
    #[error(
        "the copy that arrived has a different SHA-256 from the build Humata sent, so something on the way changed the bytes, and it was not installed. Try again; if it fails again, check that `connect` passes stdin to `{{cmd}}` unchanged (no banner, no terminal translation), and reinstall Humata in case its own copy is damaged"
    )]
    ChecksumMismatch,
    #[error(
        "only part of it arrived, so it was not installed; try again, and if it repeats check that `connect` passes stdin to `{{cmd}}`"
    )]
    Truncated,
    #[error(
        "couldn't write it to ~/.humata2/bin (a full disk or the wrong permissions); fix that on the machine and try again"
    )]
    WriteFailed,
    #[error(
        "the machine has neither `sha256sum` nor `shasum`, so Humata can't check what arrived; install coreutils there and try again"
    )]
    NoChecksumTool,
    #[error("the install script stopped ({exit}); its output is below")]
    Other { exit: Exit },
}

impl InstallProblem {
    pub(crate) fn from_exit(exit: Exit) -> Self {
        match exit {
            Exit::Code(CHECKSUM_MISMATCH) => Self::ChecksumMismatch,
            Exit::Code(TRUNCATED) => Self::Truncated,
            Exit::Code(WRITE_FAILED) => Self::WriteFailed,
            Exit::Code(NO_CHECKSUM_TOOL) => Self::NoChecksumTool,
            exit => Self::Other { exit },
        }
    }
}

/// A shell script that reads exactly `payload`'s length from stdin into a
/// temporary file, checks size and checksum, and moves it into place, or
/// beside it as `.<name>.staged` when the file waits for holders and one
/// runs. It exits 0 when swapped, [`STAGED`] when staged, or one of the
/// codes [`InstallProblem::from_exit`] reads. The file is never left
/// half-written where anything would run it: a kill at any point leaves the
/// old copy in place and at most a temporary file, which the next install
/// of that file removes (PER-27).
pub(crate) fn script(payload: &Payload, nonce: &str) -> String {
    let len = payload.bytes.len();
    let sha = &payload.sha256;
    let name = payload.file.file_name();
    let staged = payload.file.staged_name();
    let waits = if payload.file.waits_for_holders() { "yes" } else { "no" };
    format!(
        r#"set -u
umask 077
{HOLDERS_FN}d="$HOME/{BIN_DIR}"
t="$d/.{name}.{nonce}.new"
mkdir -p "$d" || exit {WRITE_FAILED}
rm -f "$d"/.{name}.*.new
trap 'rm -f "$t"' EXIT
head -c {len} > "$t" || exit {WRITE_FAILED}
n=$(wc -c < "$t" | tr -d ' ')
[ "$n" = {len} ] || {{ echo "received $n of {len} bytes" >&2; exit {TRUNCATED}; }}
if command -v sha256sum >/dev/null 2>&1; then s=$(sha256sum "$t" | cut -d ' ' -f 1)
elif command -v shasum >/dev/null 2>&1; then s=$(shasum -a 256 "$t" | cut -d ' ' -f 1)
else echo "no sha256sum or shasum" >&2; exit {NO_CHECKSUM_TOOL}; fi
[ "$s" = {sha} ] || {{ echo "SHA-256 $s, expected {sha}" >&2; exit {CHECKSUM_MISMATCH}; }}
chmod 700 "$t" || exit {WRITE_FAILED}
sync "$t" 2>/dev/null || sync
if [ {waits} = yes ] && holders; then
  mv -f "$t" "$d/{staged}" || exit {WRITE_FAILED}
  trap - EXIT
  echo "a holder is running, so {name} waits as {staged}" >&2
  exit {STAGED}
fi
mv -f "$t" "$d/{name}" || exit {WRITE_FAILED}
trap - EXIT
rm -f "$d/{staged}"
"#
    )
}

/// Moves a staged copy of `file` with SHA-256 `sha` into place once no
/// holder runs, without sending it again. Exits like [`script`]: 0 when
/// swapped, [`STAGED`] while a holder still runs, and
/// [`CHECKSUM_MISMATCH`] (removing it) when the staged copy isn't `sha`.
pub(crate) fn promote_script(file: InstalledFile, sha: &str) -> String {
    let name = file.file_name();
    let staged = file.staged_name();
    format!(
        r#"set -u
{HOLDERS_FN}d="$HOME/{BIN_DIR}"
[ -f "$d/{staged}" ] || {{ echo "{staged} is gone" >&2; exit {TRUNCATED}; }}
if command -v sha256sum >/dev/null 2>&1; then s=$(sha256sum "$d/{staged}" | cut -d ' ' -f 1)
elif command -v shasum >/dev/null 2>&1; then s=$(shasum -a 256 "$d/{staged}" | cut -d ' ' -f 1)
else echo "no sha256sum or shasum" >&2; exit {NO_CHECKSUM_TOOL}; fi
[ "$s" = {sha} ] || {{ rm -f "$d/{staged}"; echo "staged SHA-256 $s, expected {sha}" >&2; exit {CHECKSUM_MISMATCH}; }}
if holders; then exit {STAGED}; fi
mv -f "$d/{staged}" "$d/{name}" || exit {WRITE_FAILED}
"#
    )
}

/// A fresh hex word for temporary names and takeover markers. Unique, not
/// secret.
pub(crate) fn nonce() -> String {
    let mut hasher = RandomState::new().build_hasher();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    hasher.write_u128(now);
    hasher.write_u32(std::process::id());
    format!("{:016x}", hasher.finish())
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::os::unix::fs::PermissionsExt;

    use super::*;
    use crate::runner::{EnvValue, Invocation, Purpose, Runner};
    use crate::system::SystemRunner;

    /// Runs the install script here, with `home` as `$HOME`.
    fn install_here(home: &std::path::Path, payload: &Payload, stdin: &[u8]) -> Exit {
        run_here(home, script(payload, &nonce()), Some(stdin))
    }

    /// Runs `script` here with `home` as `$HOME` and a `pgrep` that finds a
    /// holder while `$HOME/holder-running` exists.
    fn run_here(home: &std::path::Path, script: String, stdin: Option<&[u8]>) -> Exit {
        let fakes = home.join("fake-bin");
        std::fs::create_dir_all(&fakes).unwrap();
        let pgrep = fakes.join("pgrep");
        std::fs::write(&pgrep, "#!/bin/sh\n[ -e \"$HOME/holder-running\" ]\n").unwrap();
        std::fs::set_permissions(&pgrep, std::fs::Permissions::from_mode(0o755)).unwrap();
        let invocation = Invocation {
            program: "/bin/sh".to_owned(),
            args: vec!["-c".to_owned(), script],
            env: vec![
                ("HOME".to_owned(), EnvValue::Value(home.display().to_string())),
                (
                    "PATH".to_owned(),
                    EnvValue::Value(format!("{}:/usr/bin:/bin", fakes.display())),
                ),
            ],
            stdin: stdin.map(Arc::from),
            timeout: Some(std::time::Duration::from_secs(20)),
            purpose: Purpose::Install,
        };
        SystemRunner::new(&()).run(&invocation).unwrap().exit
    }

    fn files_in(dir: &std::path::Path) -> BTreeMap<String, Vec<u8>> {
        std::fs::read_dir(dir)
            .map(|entries| {
                entries
                    .map(|entry| {
                        let entry = entry.unwrap();
                        (
                            entry.file_name().to_string_lossy().into_owned(),
                            std::fs::read(entry.path()).unwrap(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn a_file_that_arrives_whole_is_installed_executable() {
        let home = tempfile::tempdir().unwrap();
        let bytes: Vec<u8> = (0..=255u8).cycle().take(70_000).collect();
        let payload = Payload::new(InstalledFile::Humatad, Arc::from(bytes.as_slice()));

        assert_eq!(install_here(home.path(), &payload, &bytes), Exit::Code(0));

        let installed = home.path().join(".humata2/bin/humatad2");
        assert_eq!(std::fs::read(&installed).unwrap(), bytes);
        let mode = std::fs::metadata(&installed).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
        assert_eq!(files_in(&home.path().join(".humata2/bin")).len(), 1);
    }

    #[test]
    fn changed_bytes_are_refused_and_nothing_is_left_behind() {
        let home = tempfile::tempdir().unwrap();
        let payload = Payload::new(InstalledFile::Holder, Arc::from(&b"the real build"[..]));

        let exit = install_here(home.path(), &payload, b"the fake build");

        assert_eq!(InstallProblem::from_exit(exit), InstallProblem::ChecksumMismatch);
        assert!(files_in(&home.path().join(".humata2/bin")).is_empty());
    }

    #[test]
    fn a_short_file_is_refused_and_nothing_is_left_behind() {
        let home = tempfile::tempdir().unwrap();
        let payload = Payload::new(InstalledFile::Holder, Arc::from(&b"the whole build"[..]));

        let exit = install_here(home.path(), &payload, b"the who");

        assert_eq!(InstallProblem::from_exit(exit), InstallProblem::Truncated);
        assert!(files_in(&home.path().join(".humata2/bin")).is_empty());
    }

    #[test]
    fn an_old_file_is_replaced() {
        let home = tempfile::tempdir().unwrap();
        let old = Payload::new(InstalledFile::Humatad, Arc::from(&b"old"[..]));
        let new = Payload::new(InstalledFile::Humatad, Arc::from(&b"new build"[..]));

        assert_eq!(install_here(home.path(), &old, b"old"), Exit::Code(0));
        assert_eq!(install_here(home.path(), &new, b"new build"), Exit::Code(0));

        assert_eq!(
            std::fs::read(home.path().join(".humata2/bin/humatad2")).unwrap(),
            b"new build"
        );
    }

    #[test]
    fn a_file_a_holder_uses_waits_beside_while_one_runs_then_is_moved_in() {
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join(".humata2/bin");
        let old = Payload::new(InstalledFile::Cli, Arc::from(&b"old cli"[..]));
        let new = Payload::new(InstalledFile::Cli, Arc::from(&b"new cli"[..]));
        assert_eq!(install_here(home.path(), &old, b"old cli"), Exit::Code(0));
        std::fs::write(home.path().join("holder-running"), "").unwrap();

        let exit = install_here(home.path(), &new, b"new cli");
        assert_eq!(Installed::from_exit(exit), Some(Installed::Staged));
        assert_eq!(std::fs::read(bin.join("humata")).unwrap(), b"old cli");
        assert_eq!(std::fs::read(bin.join(".humata.staged")).unwrap(), b"new cli");

        // Still running: the promote waits too.
        let promote = promote_script(InstalledFile::Cli, new.sha256());
        let exit = run_here(home.path(), promote.clone(), None);
        assert_eq!(Installed::from_exit(exit), Some(Installed::Staged));

        std::fs::remove_file(home.path().join("holder-running")).unwrap();
        let exit = run_here(home.path(), promote, None);
        assert_eq!(Installed::from_exit(exit), Some(Installed::Swapped));
        assert_eq!(std::fs::read(bin.join("humata")).unwrap(), b"new cli");
        assert_eq!(files_in(&bin).len(), 1);
    }

    #[test]
    fn humatad_is_swapped_even_while_a_holder_runs() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(home.path().join("holder-running"), "").unwrap();
        let payload = Payload::new(InstalledFile::Humatad, Arc::from(&b"daemon"[..]));

        assert_eq!(install_here(home.path(), &payload, b"daemon"), Exit::Code(0));
        assert_eq!(
            std::fs::read(home.path().join(".humata2/bin/humatad2")).unwrap(),
            b"daemon"
        );
    }

    #[test]
    fn a_temporary_file_left_by_a_killed_install_is_removed_by_the_next() {
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join(".humata2/bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join(".humata2_holder.feedfacefeedface.new"), b"half").unwrap();
        let payload = Payload::new(InstalledFile::Holder, Arc::from(&b"holder"[..]));

        assert_eq!(install_here(home.path(), &payload, b"holder"), Exit::Code(0));
        assert_eq!(files_in(&bin).keys().collect::<Vec<_>>(), ["humata2_holder"]);
    }

    #[test]
    fn a_staged_copy_that_changed_is_not_moved_in() {
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join(".humata2/bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join(".humata.staged"), b"something else").unwrap();
        let expected = Payload::new(InstalledFile::Cli, Arc::from(&b"cli"[..]));

        let exit = run_here(home.path(), promote_script(InstalledFile::Cli, expected.sha256()), None);

        assert_eq!(InstallProblem::from_exit(exit), InstallProblem::ChecksumMismatch);
        assert!(files_in(&bin).is_empty());
    }

    #[test]
    fn nonces_differ() {
        assert_ne!(nonce(), nonce());
    }
}
