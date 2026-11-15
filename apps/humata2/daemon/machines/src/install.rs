//! Streaming humatad2 and its holder onto a machine (MP-7, MP-19): the
//! bytes go in on stdin, the far end checks their size and SHA-256 and only
//! then moves them into `~/.humata2/bin`.

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

/// A file Humata installs on every machine. The holder sits beside
/// humatad2, where humatad2 looks for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum InstalledFile {
    Humatad,
    Holder,
}

impl InstalledFile {
    pub(crate) const ALL: [Self; 2] = [Self::Humatad, Self::Holder];

    pub fn file_name(self) -> &'static str {
        match self {
            Self::Humatad => "humatad2",
            Self::Holder => "humata2_holder",
        }
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

/// Where setup gets the files to install: the daemon's own builds for each
/// platform it ships, or a test's fakes.
pub trait Builds {
    /// Every [`InstalledFile`] for `platform`; none when this Humata has no
    /// build for it.
    fn payloads(&self, platform: Platform) -> Option<Vec<Payload>>;
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
/// temporary file, checks size and checksum, and moves it into place. It
/// exits with one of the codes [`InstallProblem::from_exit`] reads. The file
/// is never left half-written where humatad2 would run it.
pub(crate) fn script(payload: &Payload, nonce: &str) -> String {
    let len = payload.bytes.len();
    let sha = &payload.sha256;
    let name = payload.file.file_name();
    format!(
        r#"set -u
umask 077
d="$HOME/{BIN_DIR}"
t="$d/.{name}.{nonce}.new"
mkdir -p "$d" || exit {WRITE_FAILED}
trap 'rm -f "$t"' EXIT
head -c {len} > "$t" || exit {WRITE_FAILED}
n=$(wc -c < "$t" | tr -d ' ')
[ "$n" = {len} ] || {{ echo "received $n of {len} bytes" >&2; exit {TRUNCATED}; }}
if command -v sha256sum >/dev/null 2>&1; then s=$(sha256sum "$t" | cut -d ' ' -f 1)
elif command -v shasum >/dev/null 2>&1; then s=$(shasum -a 256 "$t" | cut -d ' ' -f 1)
else echo "no sha256sum or shasum" >&2; exit {NO_CHECKSUM_TOOL}; fi
[ "$s" = {sha} ] || {{ echo "SHA-256 $s, expected {sha}" >&2; exit {CHECKSUM_MISMATCH}; }}
chmod 700 "$t" && mv -f "$t" "$d/{name}" || exit {WRITE_FAILED}
trap - EXIT
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

fn hex(bytes: &[u8]) -> String {
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
        let invocation = Invocation {
            program: "/bin/sh".to_owned(),
            args: vec!["-c".to_owned(), script(payload, &nonce())],
            env: vec![(
                "HOME".to_owned(),
                EnvValue::Value(home.display().to_string()),
            )],
            stdin: Some(Arc::from(stdin)),
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
    fn nonces_differ() {
        assert_ne!(nonce(), nonce());
    }
}
