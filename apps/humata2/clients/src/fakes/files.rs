//! The file system fake the tests and the fake app run on.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::{FileError, FileSystem};

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
