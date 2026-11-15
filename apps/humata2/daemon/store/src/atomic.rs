//! Writing a file so that a crash at any moment leaves either the old copy
//! or the new one, never a mix.

use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::StoreError;

/// Ends the name of a file being written. Such a file was never renamed
/// into place, so it is left over from a crash and safe to delete.
const PARTIAL: &str = ".partial";

static WRITES: AtomicU64 = AtomicU64::new(0);

/// Writes `bytes` to `path`, readable only by this user (0600): to a temp
/// file beside it, fsynced, renamed over `path`, and the folder fsynced so
/// the rename itself survives a power cut.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let failed = |source| StoreError::Write {
        path: path.to_owned(),
        source,
    };
    let (Some(folder), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(failed(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "the path names no file in a folder",
        )));
    };
    let temp = folder.join(format!(
        ".{}.{}-{}{PARTIAL}",
        name.to_string_lossy(),
        std::process::id(),
        WRITES.fetch_add(1, Ordering::Relaxed),
    ));
    let written = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)?;
        File::open(folder)?.sync_all()
    })();
    if written.is_err() {
        // Already renamed or never made are both fine.
        let _ = fs::remove_file(&temp);
    }
    written.map_err(failed)
}

/// Makes `path` and its missing parents, each readable only by this user.
pub(crate) fn create_dir(path: &Path) -> Result<(), StoreError> {
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .map_err(|source| StoreError::CreateDir {
            path: path.to_owned(),
            source,
        })
}

/// Deletes what writes cut short by a crash left in `folder`.
pub(crate) fn remove_partials(folder: &Path) -> Result<(), StoreError> {
    let entries = fs::read_dir(folder).map_err(|source| StoreError::Read {
        path: folder.to_owned(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| StoreError::Read {
            path: folder.to_owned(),
            source,
        })?;
        if is_partial(&entry.file_name().to_string_lossy()) {
            let path = entry.path();
            fs::remove_file(&path).map_err(|source| StoreError::Remove { path, source })?;
        }
    }
    Ok(())
}

/// Whether a folder entry is a write in progress or left by a crash.
pub(crate) fn is_partial(name: &str) -> bool {
    name.starts_with('.') && name.ends_with(PARTIAL)
}
