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

/// Resolves when the process is asked to stop with SIGTERM.
pub async fn terminated() {
    match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        Ok(mut signal) => {
            signal.recv().await;
        }
        Err(_) => std::future::pending().await,
    }
}
