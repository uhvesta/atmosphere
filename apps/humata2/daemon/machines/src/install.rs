//! Streaming humatad2, its holder, the `humata` CLI, the `stack` CLI and the
//! FUSEFS helper onto a machine (MP-7, MP-19, TS-1, D411): the bytes go in on stdin, the far end
//! checks their size and SHA-256 and only then moves them into
//! `~/.humata2/bin`.
//!
//! A first install swaps the files in at once and writes the release's
//! `bundle.json` (UP-1). A machine that already runs humatad2 is never
//! upgraded by a connect: a new release is staged beside the old files as
//! `.<name>.next` ([`stage_next_remote`]), over this same path, and the
//! daemon's Upgrade call swaps it in once the person has said Migrate. A
//! running holder keeps the inode it started from, so renaming over its file
//! is safe; nothing waits for holders any more (D281 f).

use std::collections::hash_map::RandomState;
use std::fmt;
use std::hash::{BuildHasher, Hasher};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::bundle::{next_name, StagedRelease, BUNDLE_JSON};
use crate::error::{output_said, QuoteError, RunError};
use crate::link::{Link, BIN_DIR};
use crate::name::MachineName;
use crate::platform::Platform;
use crate::runner::{Exit, Purpose, Runner};
use crate::stack_config::{StackConfig, StackConfigError};

/// The install script's exit codes for what it found wrong.
const CHECKSUM_MISMATCH: i32 = 65;
const TRUNCATED: i32 = 66;
const WRITE_FAILED: i32 = 67;
const NO_CHECKSUM_TOOL: i32 = 68;

/// The most a streamed file may take: a release's tools are tens of MiB.
const INSTALL_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// How many output lines a failed stage keeps (MP-20).
const TAIL_LINES: usize = 40;

/// A file Humata installs on every machine, and on the host (TS-0). The
/// holder, `humata`, `stack` and the FUSEFS helper sit beside humatad2, where
/// it looks for them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum InstalledFile {
    Humatad,
    Holder,
    Cli,
    /// The stacked-PR CLI (D411).
    Stack,
    /// The FUSEFS mount's own process. It is a separate binary so that an exec
    /// swap of humatad2 never tears the mount down.
    FuseFs,
}

impl InstalledFile {
    pub const ALL: [Self; 5] = [Self::Humatad, Self::Holder, Self::Cli, Self::Stack, Self::FuseFs];

    pub fn file_name(self) -> &'static str {
        match self {
            Self::Humatad => "humatad2",
            Self::Holder => "humata2_holder",
            Self::Cli => "humata",
            Self::Stack => "stack",
            Self::FuseFs => "humata2_fusefs",
        }
    }

    /// The name a copy left by an older Humata waits under; installs remove
    /// it, and nothing makes one any more over SSH.
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

    /// The `bundle.json` a first install writes beside the files, whose
    /// sha256 is the release's bundle id (UP-1); none when this source has
    /// no release to name, as a test's fakes don't.
    fn bundle_json(&self, _platform: Platform) -> Result<Option<Arc<[u8]>>, BuildsError> {
        Ok(None)
    }

    /// The host's `config/stack.toml` with its secrets taken out, to put in
    /// `~/.humata2/config` on a machine beside the tools (D411); none when
    /// the host has no such file or this source has none to give, as a
    /// test's fakes don't. Read afresh each time, so a changed file is what
    /// the next link or setup sends.
    fn stack_config(&self) -> Result<Option<StackConfig>, StackConfigError> {
        Ok(None)
    }

    /// The host's `config/repos.toml` (RL-12), sent to a machine the way
    /// `stack_config` is, into its own file beside it.
    fn repos_config(&self) -> Result<Option<StackConfig>, StackConfigError> {
        Ok(None)
    }

    /// The host's `config/agents.toml` (AGENTSET), sent the same way.
    fn agents_config(&self) -> Result<Option<StackConfig>, StackConfigError> {
        Ok(None)
    }

    /// The host's `config/machine-scripts.toml` (MACHINESCRIPTS), sent the
    /// same way, into its own file beside the others.
    fn machine_scripts_config(&self) -> Result<Option<StackConfig>, StackConfigError> {
        Ok(None)
    }
}

/// Why a bundle can't give the files for a platform.
#[derive(Debug, Clone, Error)]
pub enum BuildsError {
    #[error(
        "this Humata has no build for {platform} (it ships {shipped}); use a machine of one of those platforms"
    )]
    NoBuild { platform: Platform, shipped: String },
    /// The app brought tools for `platform` that this daemon, still the older
    /// release, may not use: the host must be migrated first (UP-1, D370).
    #[error(
        "the Linux tools for {platform} that this version of Humata brought are newer than the daemon on This Mac, so this machine can't be set up yet. Migrate the daemon on This Mac first (the \"Migrate the daemon on This Mac\" item in the Inbox), then add the machine again"
    )]
    HostNeedsMigrate { platform: Platform },
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
            (Self::HostNeedsMigrate { platform: a }, Self::HostNeedsMigrate { platform: b }) => a == b,
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
        "couldn't write it under ~/.humata2 (a full disk or the wrong permissions); fix that on the machine and try again"
    )]
    WriteFailed,
    #[error(
        "the machine has neither `sha256sum` nor `shasum`, so Humata can't check what arrived; install coreutils there and try again"
    )]
    NoChecksumTool,
    #[error("the install script stopped ({exit}); {}", output_said(.printed))]
    Other {
        exit: Exit,
        /// Whether the script printed anything the failure shows (MP-20).
        printed: bool,
    },
}

impl InstallProblem {
    /// What an install script that exited with `exit` means; `printed` says
    /// whether it printed anything the failure can show.
    pub(crate) fn from_exit(exit: Exit, printed: bool) -> Self {
        match exit {
            Exit::Code(CHECKSUM_MISMATCH) => Self::ChecksumMismatch,
            Exit::Code(TRUNCATED) => Self::Truncated,
            Exit::Code(WRITE_FAILED) => Self::WriteFailed,
            Exit::Code(NO_CHECKSUM_TOOL) => Self::NoChecksumTool,
            exit => Self::Other { exit, printed },
        }
    }
}

/// What [`receive_script`] receives and where it puts it.
#[derive(Clone, Copy)]
pub(crate) struct Receive<'a> {
    /// The folder under `$HOME`.
    pub(crate) dir: &'a str,
    /// The name its temporary files carry.
    pub(crate) stem: &'a str,
    /// The file it ends as.
    pub(crate) target: &'a str,
    pub(crate) mode: &'a str,
    pub(crate) len: usize,
    pub(crate) sha: &'a str,
    pub(crate) nonce: &'a str,
    /// Shell run once the file is in place.
    pub(crate) after: &'a str,
}

/// A shell script that reads exactly `len` bytes from stdin into a
/// temporary file, checks size and checksum, gives it `mode` and moves it to
/// `~/<dir>/<target>` (`dir` is `.humata2/bin` or `.humata2/config`), then
/// runs `after`. It exits 0 when done, or
/// with one of the codes [`InstallProblem::from_exit`] reads. The file is
/// never left half-written where anything would run it: a kill at any point
/// leaves the old copy in place and at most a temporary file, which the next
/// receive of that `stem` removes (PER-27).
pub(crate) fn receive_script(receive: &Receive<'_>) -> String {
    let Receive {
        dir,
        stem,
        target,
        mode,
        len,
        sha,
        nonce,
        after,
    } = *receive;
    format!(
        r#"set -u
umask 077
d="$HOME/{dir}"
t="$d/.{stem}.{nonce}.new"
mkdir -p "$d" || exit {WRITE_FAILED}
rm -f "$d"/.{stem}.*.new
trap 'rm -f "$t"' EXIT
head -c {len} > "$t" || exit {WRITE_FAILED}
n=$(wc -c < "$t" | tr -d ' ')
[ "$n" = {len} ] || {{ echo "received $n of {len} bytes" >&2; exit {TRUNCATED}; }}
if command -v sha256sum >/dev/null 2>&1; then s=$(sha256sum "$t" | cut -d ' ' -f 1)
elif command -v shasum >/dev/null 2>&1; then s=$(shasum -a 256 "$t" | cut -d ' ' -f 1)
else echo "no sha256sum or shasum" >&2; exit {NO_CHECKSUM_TOOL}; fi
[ "$s" = {sha} ] || {{ echo "SHA-256 $s, expected {sha}" >&2; exit {CHECKSUM_MISMATCH}; }}
chmod {mode} "$t" || exit {WRITE_FAILED}
sync "$t" 2>/dev/null || sync
mv -f "$t" "$d/{target}" || exit {WRITE_FAILED}
trap - EXIT
{after}sync "$d" 2>/dev/null || sync
"#
    )
}

/// The script that swaps `payload` into `~/.humata2/bin` at once, as a
/// first install does, and removes a copy an older Humata left waiting as
/// `.<name>.staged`. A running holder keeps the file it started from.
pub(crate) fn script(payload: &Payload, nonce: &str) -> String {
    let name = payload.file.file_name();
    let staged = payload.file.staged_name();
    receive_script(&Receive {
        dir: BIN_DIR,
        stem: name,
        target: name,
        mode: "700",
        len: payload.bytes.len(),
        sha: &payload.sha256,
        nonce,
        after: &format!("rm -f \"$d/{staged}\"\n"),
    })
}

/// The script that writes the release's `bundle.json` (`bytes` on stdin).
pub(crate) fn bundle_script(bytes: &[u8], nonce: &str) -> String {
    receive_script(&Receive {
        dir: BIN_DIR,
        stem: BUNDLE_JSON,
        target: BUNDLE_JSON,
        mode: "644",
        len: bytes.len(),
        sha: &hex(&Sha256::digest(bytes)),
        nonce,
        after: "",
    })
}

/// The script that stages `bytes` as `.<name>.next` (D281 f), checked the
/// same way, leaving what is installed alone.
fn next_script(name: &str, mode: &str, bytes: &[u8], nonce: &str) -> String {
    receive_script(&Receive {
        dir: BIN_DIR,
        stem: name,
        target: &next_name(name),
        mode,
        len: bytes.len(),
        sha: &hex(&Sha256::digest(bytes)),
        nonce,
        after: "",
    })
}

/// Why a release couldn't be staged on a machine.
#[derive(Debug, Error)]
pub enum RemoteStageError {
    #[error(
        "couldn't build the command that stages {file} on {machine}: {source}. This is a fault in Humata; report it"
    )]
    Command {
        machine: MachineName,
        file: String,
        #[source]
        source: QuoteError,
    },
    #[error(
        "couldn't reach {machine} to stage {file}: {source}. Check that the machine is up and reachable, then Migrate again; nothing installed there was changed"
    )]
    Reach {
        machine: MachineName,
        file: String,
        #[source]
        source: RunError,
    },
    #[error(
        "staging {file} on {machine} failed: {problem}. Nothing installed there was changed; fix that and Migrate again.\n{tail}"
    )]
    Stage {
        machine: MachineName,
        file: String,
        problem: InstallProblem,
        tail: String,
    },
}

/// Stages `release` on the machine `link` reaches, over the same SSH path
/// that installed it: each tool as `~/.humata2/bin/.<name>.next` (0755), then
/// `.bundle.json.next` (0644), every one checked against its sha256 on the
/// far end and moved in whole, so nothing installed or running is touched.
/// The bundle goes last, so it is there only when the files it names are.
/// Returns the bundle id, which the Upgrade call names. Pick the release for
/// the machine's platform ([`crate::Bundle::release`]); it can be staged
/// again if a try is cut short.
pub fn stage_next_remote(
    link: &Link,
    runner: &dyn Runner,
    release: &StagedRelease,
) -> Result<String, RemoteStageError> {
    let send = |name: &str, mode: &str, bytes: &Arc<[u8]>| -> Result<(), RemoteStageError> {
        let file = next_name(name);
        let machine = link.machine().clone();
        let invocation = link
            .script(
                &next_script(name, mode, bytes, &nonce()),
                Purpose::Install,
                Some(INSTALL_TIMEOUT),
                Some(Arc::clone(bytes)),
            )
            .map_err(|source| RemoteStageError::Command {
                machine: machine.clone(),
                file: file.clone(),
                source,
            })?;
        let output = runner.run(&invocation).map_err(|source| RemoteStageError::Reach {
            machine: machine.clone(),
            file: file.clone(),
            source,
        })?;
        if output.exit.success() {
            return Ok(());
        }
        let tail = output.tail(TAIL_LINES);
        Err(RemoteStageError::Stage {
            machine,
            file,
            problem: InstallProblem::from_exit(output.exit, !tail.trim().is_empty()),
            tail,
        })
    };
    for payload in release.files() {
        send(payload.file.file_name(), "755", &payload.bytes)?;
    }
    send(BUNDLE_JSON, "644", release.bundle_json())?;
    Ok(release.id().to_owned())
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
    fn the_fusefs_helper_is_installed_beside_the_daemon_under_its_own_name() {
        let home = tempfile::tempdir().unwrap();
        let payload = Payload::new(InstalledFile::FuseFs, Arc::from(&b"fusefs build"[..]));

        assert_eq!(install_here(home.path(), &payload, b"fusefs build"), Exit::Code(0));

        let installed = home.path().join(".humata2/bin/humata2_fusefs");
        assert_eq!(std::fs::read(&installed).unwrap(), b"fusefs build");
        let mode = std::fs::metadata(&installed).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
    }

    #[test]
    fn changed_bytes_are_refused_and_nothing_is_left_behind() {
        let home = tempfile::tempdir().unwrap();
        let payload = Payload::new(InstalledFile::Holder, Arc::from(&b"the real build"[..]));

        let exit = install_here(home.path(), &payload, b"the fake build");

        assert_eq!(InstallProblem::from_exit(exit, false), InstallProblem::ChecksumMismatch);
        assert!(files_in(&home.path().join(".humata2/bin")).is_empty());
    }

    #[test]
    fn a_short_file_is_refused_and_nothing_is_left_behind() {
        let home = tempfile::tempdir().unwrap();
        let payload = Payload::new(InstalledFile::Holder, Arc::from(&b"the whole build"[..]));

        let exit = install_here(home.path(), &payload, b"the who");

        assert_eq!(InstallProblem::from_exit(exit, false), InstallProblem::Truncated);
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
    fn a_file_a_holder_uses_is_swapped_in_while_a_holder_runs_and_a_leftover_staged_copy_goes() {
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join(".humata2/bin");
        let old = Payload::new(InstalledFile::Cli, Arc::from(&b"old cli"[..]));
        let new = Payload::new(InstalledFile::Cli, Arc::from(&b"new cli"[..]));
        assert_eq!(install_here(home.path(), &old, b"old cli"), Exit::Code(0));
        // A copy an older Humata left waiting, and a holder that runs.
        std::fs::write(bin.join(".humata.staged"), b"waiting").unwrap();
        std::fs::write(home.path().join("holder-running"), "").unwrap();

        assert_eq!(install_here(home.path(), &new, b"new cli"), Exit::Code(0));

        assert_eq!(files_in(&bin).keys().collect::<Vec<_>>(), ["humata"]);
        assert_eq!(std::fs::read(bin.join("humata")).unwrap(), b"new cli");
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
        // A holder file put back while another holder runs goes in too.
        let holder = Payload::new(InstalledFile::Holder, Arc::from(&b"holder"[..]));
        assert_eq!(install_here(home.path(), &holder, b"holder"), Exit::Code(0));
        assert_eq!(
            std::fs::read(home.path().join(".humata2/bin/humata2_holder")).unwrap(),
            b"holder"
        );
    }

    #[test]
    fn a_next_copy_waits_beside_the_installed_one_and_touches_nothing_else() {
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join(".humata2/bin");
        let old = Payload::new(InstalledFile::Humatad, Arc::from(&b"old daemon"[..]));
        assert_eq!(install_here(home.path(), &old, b"old daemon"), Exit::Code(0));

        let exit = run_here(home.path(), next_script("humatad2", "755", b"new daemon", &nonce()), Some(b"new daemon"));

        assert_eq!(exit, Exit::Code(0));
        assert_eq!(std::fs::read(bin.join("humatad2")).unwrap(), b"old daemon");
        let staged = bin.join(".humatad2.next");
        assert_eq!(std::fs::read(&staged).unwrap(), b"new daemon");
        assert_eq!(std::fs::metadata(&staged).unwrap().permissions().mode() & 0o777, 0o755);
        assert_eq!(files_in(&bin).len(), 2);
    }

    #[test]
    fn a_next_copy_that_arrives_changed_is_refused_and_leaves_nothing() {
        let home = tempfile::tempdir().unwrap();
        let script = next_script("humata", "755", b"the real build", &nonce());

        let exit = run_here(home.path(), script, Some(b"the fake build"));

        assert_eq!(InstallProblem::from_exit(exit, false), InstallProblem::ChecksumMismatch);
        assert!(files_in(&home.path().join(".humata2/bin")).is_empty());
    }

    #[test]
    fn a_first_install_writes_the_bundle_beside_the_files() {
        let home = tempfile::tempdir().unwrap();
        let bundle = br#"{"platforms":{}}"#;

        assert_eq!(
            run_here(home.path(), bundle_script(bundle, &nonce()), Some(bundle)),
            Exit::Code(0)
        );

        let written = home.path().join(".humata2/bin/bundle.json");
        assert_eq!(std::fs::read(&written).unwrap(), bundle);
        assert_eq!(std::fs::metadata(&written).unwrap().permissions().mode() & 0o777, 0o644);
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
    fn nonces_differ() {
        assert_ne!(nonce(), nonce());
    }
}
