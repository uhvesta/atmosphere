//! Writing a file so that a crash at any moment leaves either the old copy
//! or the new one, never a mix.

use std::fs::OpenOptions;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use humata2_durable::{File, create_dir_all, remove_file, rename, sync_dir};
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
        let mut options = OpenOptions::new();
        options.write(true).create_new(true).mode(0o600);
        let mut file = File::open(&options, &temp)?;
        file.append(bytes)?;
        file.sync_all()?;
        drop(file);
        rename(&temp, path)?;
        sync_dir(folder)
    })();
    if written.is_err() {
        // Already renamed or never made are both fine.
        let _ = remove_file(&temp);
    }
    written.map_err(failed)
}

/// Makes `path` and its missing parents, each readable only by this user,
/// and syncs the folder each was made in, so a power cut doesn't take a
/// folder away from under files synced in it.
pub(crate) fn create_dir(path: &Path) -> Result<(), StoreError> {
    let missing: Vec<&Path> = path
        .ancestors()
        .take_while(|folder| !folder.as_os_str().is_empty() && !folder.exists())
        .collect();
    let made = (|| {
        create_dir_all(path, 0o700)?;
        for folder in missing.iter().rev() {
            if let Some(parent) = folder.parent().filter(|parent| !parent.as_os_str().is_empty()) {
                sync_dir(parent)?;
            }
        }
        Ok(())
    })();
    made.map_err(|source| StoreError::CreateDir {
        path: path.to_owned(),
        source,
    })
}

/// Whether a folder entry is a write in progress or left by a crash.
pub(crate) fn is_partial(name: &str) -> bool {
    name.starts_with('.') && name.ends_with(PARTIAL)
}
