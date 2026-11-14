use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::Path;

use tokio::net::{UnixListener, UnixStream};

use crate::error::StartError;

/// Listens on `path`, readable and writable only by this user. A missing
/// folder is created private to this user; an existing one is left as it is.
pub async fn listen(path: &Path) -> Result<UnixListener, StartError> {
    if let Some(folder) = path
        .parent()
        .filter(|folder| !folder.as_os_str().is_empty())
        && !folder.exists()
    {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(folder)
            .map_err(|source| StartError::CreateSocketDirectory {
                path: folder.to_owned(),
                source,
            })?;
    }
    if path.exists() {
        if UnixStream::connect(path).await.is_ok() {
            return Err(StartError::AlreadyRunning {
                path: path.to_owned(),
            });
        }
        std::fs::remove_file(path).map_err(|source| StartError::RemoveStaleSocket {
            path: path.to_owned(),
            source,
        })?;
    }
    let listener = UnixListener::bind(path).map_err(|source| StartError::Listen {
        path: path.to_owned(),
        source,
    })?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|source| {
        StartError::ProtectSocket {
            path: path.to_owned(),
            source,
        }
    })?;
    Ok(listener)
}

/// Makes the folder at `path` private to this user: creates it, and any
/// missing folder above it, with mode 0700, and closes one that is there
/// and open to the group or to others. The sessions' chats are kept in it.
pub fn private_folder(path: &Path) -> std::io::Result<()> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)?;
    let mode = std::fs::metadata(path)?.permissions().mode();
    if mode & 0o077 != 0 {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Resolves when the process is asked to stop with SIGTERM.
pub async fn terminated() {
    match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        Ok(mut signal) => {
            signal.recv().await;
        }
        Err(_) => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(path: &Path) -> u32 {
        let metadata = std::fs::metadata(path).unwrap_or_else(|error| panic!("{error}"));
        metadata.permissions().mode() & 0o777
    }

    #[test]
    fn a_state_folder_is_made_private_and_one_open_to_others_is_closed() {
        let scratch = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let new = scratch.path().join("above").join("state");
        let open = scratch.path().join("open");
        std::fs::create_dir(&open).unwrap_or_else(|error| panic!("{error}"));
        std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o755))
            .unwrap_or_else(|error| panic!("{error}"));
        std::fs::write(open.join("kept"), "kept").unwrap_or_else(|error| panic!("{error}"));

        private_folder(&new).unwrap_or_else(|error| panic!("{error}"));
        private_folder(&open).unwrap_or_else(|error| panic!("{error}"));
        // Again, on one that is private already.
        private_folder(&open).unwrap_or_else(|error| panic!("{error}"));

        assert_eq!(mode(&new), 0o700);
        assert_eq!(mode(&open), 0o700);
        assert_eq!(
            std::fs::read_to_string(open.join("kept")).ok().as_deref(),
            Some("kept")
        );
    }
}
