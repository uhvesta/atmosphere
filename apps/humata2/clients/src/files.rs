//! The local file system, for what the app writes itself: bug reports and
//! exported prompts. Files on hosts come through the daemon.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

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

/// Files in memory.
#[derive(Clone, Debug, Default)]
pub struct FakeFileSystem {
    files: Arc<Mutex<BTreeMap<PathBuf, Vec<u8>>>>,
}

impl FakeFileSystem {
    /// What was written to `path`.
    #[must_use]
    pub fn read(&self, path: &Path) -> Option<Vec<u8>> {
        match self.files.lock() {
            Ok(files) => files.get(path).cloned(),
            Err(poisoned) => poisoned.into_inner().get(path).cloned(),
        }
    }
}

impl FileSystem for FakeFileSystem {
    fn write(&self, path: &Path, bytes: &[u8]) -> Result<(), FileError> {
        let mut files = match self.files.lock() {
            Ok(files) => files,
            Err(poisoned) => poisoned.into_inner(),
        };
        files.insert(path.to_owned(), bytes.to_vec());
        Ok(())
    }
}
