//! Humata's own `known_hosts` file (MP-9). Each machine's key is kept under
//! the alias `humata2.<machine>`, whatever address reaches it. Humata never
//! reads or writes `~/.ssh/known_hosts`.

use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::Path;

use crate::error::KnownHostsError;
use crate::identity::HostKey;
use crate::install;
use crate::name::MachineName;

/// The name a machine's host key is kept under, so the pin holds whatever
/// address or alias reaches it.
pub(crate) fn alias(machine: &MachineName) -> String {
    format!("humata2.{machine}")
}

/// The key pinned for `machine`; none when there is none or the file
/// doesn't exist yet.
pub(crate) fn pinned(path: &Path, machine: &MachineName) -> Result<Option<HostKey>, KnownHostsError> {
    Ok(keys_for(&read(path)?, machine).into_iter().next())
}

/// The keys `text`, in `known_hosts` form, holds for `machine`.
pub(crate) fn keys_for(text: &str, machine: &MachineName) -> Vec<HostKey> {
    let alias = alias(machine);
    text.lines()
        .filter_map(|line| {
            let (host, key) = line.trim().split_once(char::is_whitespace)?;
            if host != alias {
                return None;
            }
            HostKey::parse(key.trim_start())
        })
        .collect()
}

/// Pins `key` as `machine`'s only host key, replacing any key pinned for
/// it before. The file (0600) is replaced whole, so ssh never reads half
/// of it. Two pins at once may lose one; the daemon runs them one at a
/// time.
pub(crate) fn pin(path: &Path, machine: &MachineName, key: &HostKey) -> Result<(), KnownHostsError> {
    let alias = alias(machine);
    let old = read(path)?;
    let mut text: String = old
        .lines()
        .filter(|line| line.split_whitespace().next() != Some(alias.as_str()))
        .flat_map(|line| [line, "\n"])
        .collect();
    text.push_str(&format!("{alias} {} {}\n", key.algorithm(), key.key()));

    let write_failed = |source| KnownHostsError::Write {
        path: path.to_owned(),
        source,
    };
    if let Some(folder) = path.parent().filter(|folder| !folder.as_os_str().is_empty()) {
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(folder)
            .map_err(write_failed)?;
    }
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(format!(".{}.new", install::nonce()));
    let result = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .and_then(|mut file| {
            file.write_all(text.as_bytes())?;
            file.sync_all()
        })
        .and_then(|()| fs::rename(&temporary, path));
    if let Err(source) = result {
        let _ = fs::remove_file(&temporary);
        return Err(write_failed(source));
    }
    Ok(())
}

fn read(path: &Path) -> Result<String, KnownHostsError> {
    match fs::read(path) {
        Ok(bytes) => Ok(String::from_utf8_lossy(&bytes).into_owned()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        Err(source) => Err(KnownHostsError::Read {
            path: path.to_owned(),
            source,
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    const KEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl";
    const OTHER: &str =
        "ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBEmKSENjQEezOmxkZMy7opKgwFB9nkt5YRrYMjNuG5N87uRgg6CLrbo5wAdT/y6v0mKV0U2w0WZ2YB/++Tpockg=";

    fn machine(name: &str) -> MachineName {
        MachineName::new(name).unwrap()
    }

    fn key(text: &str) -> HostKey {
        HostKey::parse(text).unwrap()
    }

    #[test]
    fn a_pin_is_read_back_and_replaces_the_old_one_only_for_its_machine() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("ssh/known_hosts");

        pin(&path, &machine("box-1"), &key(KEY)).unwrap();
        pin(&path, &machine("box-2"), &key(KEY)).unwrap();
        pin(&path, &machine("box-1"), &key(OTHER)).unwrap();

        assert_eq!(pinned(&path, &machine("box-1")).unwrap(), Some(key(OTHER)));
        assert_eq!(pinned(&path, &machine("box-2")).unwrap(), Some(key(KEY)));
        assert_eq!(pinned(&path, &machine("box-3")).unwrap(), None);
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
    }

    #[test]
    fn nothing_is_pinned_before_the_file_exists() {
        let folder = tempfile::tempdir().unwrap();
        assert_eq!(pinned(&folder.path().join("known_hosts"), &machine("box-1")).unwrap(), None);
    }

    #[test]
    fn lines_for_other_hosts_and_unreadable_keys_are_skipped() {
        let text = format!(
            "# comment\nhumata2.box-10 {KEY}\nhumata2.box-1 ssh-ed25519 notbase64!\nhumata2.box-1\t{OTHER} comment\n"
        );
        assert_eq!(keys_for(&text, &machine("box-1")), vec![key(OTHER)]);
    }

    #[test]
    fn fingerprints_match_openssh() {
        assert_eq!(
            key(KEY).fingerprint(),
            "SHA256:+DiY3wvvV6TuJJhbpZisF/zLDA0zPMSvHdkr4UvCOqU"
        );
    }

    #[test]
    fn a_key_whose_blob_names_another_algorithm_is_refused() {
        let (_, base64) = KEY.split_once(' ').unwrap();
        assert_eq!(HostKey::parse(&format!("ssh-rsa {base64}")), None);
    }
}
