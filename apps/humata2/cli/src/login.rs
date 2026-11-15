//! This terminal's `humata login` (SEC-11): the token BeginCliSession gave,
//! kept for this terminal session only, in
//! `~/.humata2/cli-logins/<session id>`, readable by the user alone.
//!
//! The file is only where the token waits between runs: the daemon takes
//! it only from a process still in the terminal session the login bound,
//! so a copy used anywhere else is refused.

use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use crate::principal::Environment;

/// Where this terminal session's login is kept: none without HOME or a
/// terminal session.
pub(crate) fn path(environment: &Environment) -> Option<PathBuf> {
    let home = environment.home.as_ref()?;
    let session = environment.terminal_session?;
    Some(home.join(".humata2/cli-logins").join(session.to_string()))
}

/// The saved token, if there is one humata can send: anything else in the
/// file (empty, cut short, edited) counts as no login.
pub(crate) fn read(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let token = text.trim();
    let usable = !token.is_empty() && token.bytes().all(|byte| byte.is_ascii_hexdigit());
    usable.then(|| token.to_owned())
}

/// Saves `token` for this terminal session: the folder is the user's only
/// (0700), the file too (0600), and it's written whole or not at all.
pub(crate) fn save(path: &Path, token: &str) -> io::Result<()> {
    let folder = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no folder"))?;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(folder)?;
    let partial = path.with_extension(format!("partial-{}", std::process::id()));
    let written = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&partial)
        .and_then(|mut file| {
            file.write_all(token.as_bytes())?;
            file.sync_all()
        })
        .and_then(|()| fs::rename(&partial, path));
    if written.is_err() {
        let _ = fs::remove_file(&partial);
    }
    written
}

/// Forgets this terminal session's login; none saved is not an error.
pub(crate) fn remove(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[test]
    fn a_saved_login_reads_back_and_only_the_user_can_read_it() {
        let home = tempfile::tempdir().expect("temp dir");
        let path = home.path().join(".humata2/cli-logins/42");
        save(&path, "00ff").expect("save");
        assert_eq!(read(&path).as_deref(), Some("00ff"));
        let file = fs::metadata(&path).expect("file").permissions().mode() & 0o777;
        let folder = fs::metadata(path.parent().expect("folder"))
            .expect("folder")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!((file, folder), (0o600, 0o700));
    }

    #[test]
    fn a_file_that_isnt_a_token_is_no_login() {
        let home = tempfile::tempdir().expect("temp dir");
        let path = home.path().join("login");
        assert_eq!(read(&path), None);
        fs::write(&path, "not a token\n").expect("write");
        assert_eq!(read(&path), None);
        fs::write(&path, "").expect("write");
        assert_eq!(read(&path), None);
    }

    #[test]
    fn removing_a_login_that_isnt_there_succeeds() {
        let home = tempfile::tempdir().expect("temp dir");
        let path = home.path().join("login");
        remove(&path).expect("nothing to remove");
        save(&path, "ab").expect("save");
        remove(&path).expect("remove");
        assert_eq!(read(&path), None);
    }

    #[test]
    fn each_terminal_session_has_its_own_login() {
        let at = |session| {
            path(&Environment {
                home: Some("/home/me".into()),
                terminal_session: Some(session),
                ..Environment::default()
            })
        };
        assert_ne!(at(1), at(2));
        let no_session = Environment {
            home: Some("/home/me".into()),
            ..Environment::default()
        };
        assert_eq!(path(&no_session), None);
    }
}
