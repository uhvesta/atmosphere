//! The enrollment key the peer link proves (GRPC.md §8, D83): when the host
//! adds a machine it makes a random key, writes it on the machine as
//! `~/.humata2/state/enrollment-key` and keeps its own copy in
//! `peer-keys/<machine>` in its state folder. Both are hex in a 0600 file,
//! written whole or not at all. The key goes to the machine on the
//! command's stdin, never in its arguments, and is never logged.

use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::install;
use crate::name::MachineName;

/// How many random bytes a key has (the machine needs at least 32).
const KEY_BYTES: usize = 32;

/// A fresh key, as the hex both files hold.
#[derive(Clone)]
pub(crate) struct Key(Arc<[u8]>);

impl Key {
    pub(crate) fn new() -> Self {
        let mut bytes = [0u8; KEY_BYTES];
        rand::fill(&mut bytes[..]);
        Self(Arc::from(install::hex(&bytes).into_bytes()))
    }

    /// The hex, for the machine's stdin.
    pub(crate) fn bytes(&self) -> Arc<[u8]> {
        Arc::clone(&self.0)
    }
}

impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Key(..)")
    }
}

/// The machine side: reads the key from stdin into a 0600 temp file beside
/// the key, then renames it over the key, so a reader sees the old key or
/// the new one and never half of one.
pub(crate) fn machine_script() -> String {
    r#"umask 077
d="$HOME/.humata2/state"
mkdir -p "$d" || exit 1
t="$d/.enrollment-key.$$.new"
if cat > "$t" && mv -f "$t" "$d/enrollment-key"; then exit 0; fi
rm -f "$t"
exit 1"#
        .to_owned()
}

/// Where the host keeps its copy of `machine`'s key.
pub(crate) fn host_path(dir: &Path, machine: &str) -> PathBuf {
    dir.join(machine)
}

/// Writes the host's copy: a 0600 temp file in a 0700 folder, synced, then
/// renamed into place.
pub(crate) fn write_host(dir: &Path, machine: &MachineName, key: &Key) -> io::Result<()> {
    DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    let temp = dir.join(format!(".{machine}.{}.new", install::nonce()));
    let written = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)?;
        file.write_all(&key.0)?;
        file.sync_all()?;
        fs::rename(&temp, host_path(dir, machine.as_str()))?;
        File::open(dir)?.sync_all()
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temp);
    }
    written
}

/// Removes the host's copy; one already gone is fine.
pub(crate) fn remove_host(dir: &Path, machine: &str) -> io::Result<()> {
    match fs::remove_file(host_path(dir, machine)) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn machine() -> MachineName {
        MachineName::new("box-1").unwrap()
    }

    #[test]
    fn keys_are_fresh_hex_of_32_bytes() {
        let (one, two) = (Key::new(), Key::new());
        assert_eq!(one.bytes().len(), 2 * KEY_BYTES);
        assert!(one.bytes().iter().all(u8::is_ascii_hexdigit));
        assert_ne!(one.bytes(), two.bytes());
    }

    #[test]
    fn the_host_copy_is_private_and_replaced_whole() {
        let dir = tempfile::tempdir().unwrap();
        let keys = dir.path().join("peer-keys");
        let (first, second) = (Key::new(), Key::new());

        write_host(&keys, &machine(), &first).unwrap();
        write_host(&keys, &machine(), &second).unwrap();

        let path = host_path(&keys, "box-1");
        assert_eq!(fs::read(&path).unwrap(), &second.bytes()[..]);
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(fs::metadata(&keys).unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(fs::read_dir(&keys).unwrap().count(), 1, "no temp file is left");
    }

    #[test]
    fn removing_a_copy_that_isnt_there_is_fine() {
        let dir = tempfile::tempdir().unwrap();
        let keys = dir.path().join("peer-keys");
        remove_host(&keys, "box-1").unwrap();
        write_host(&keys, &machine(), &Key::new()).unwrap();
        remove_host(&keys, "box-1").unwrap();
        assert!(!host_path(&keys, "box-1").exists());
    }
}
