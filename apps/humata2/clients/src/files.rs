//! The local file system, for what the app writes itself: bug reports and
//! exported prompts. Files on hosts come through the daemon.

use std::path::{Path, PathBuf};

/// Why a file couldn't be written.
#[derive(Debug, thiserror::Error)]
pub enum FileError {
    /// The write failed.
    #[error("couldn't write {path}: {source}. Check that its folder exists and you can write there")]
    Write {
        /// The file.
        path: PathBuf,
        /// What the OS said.
        #[source]
        source: std::io::Error,
    },
}

/// Files the app writes.
pub trait FileSystem: Send + Sync + 'static {
    /// Writes `bytes` to `path`, replacing it.
    ///
    /// # Errors
    ///
    /// [`FileError::Write`] when it can't be written.
    fn write(&self, path: &Path, bytes: &[u8]) -> Result<(), FileError>;
}
