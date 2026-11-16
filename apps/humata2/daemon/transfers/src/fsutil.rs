use std::fs;
use std::io::ErrorKind;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

/// Bytes in the files under `path`, not following links.
pub(crate) fn tree_size(path: &Path) -> std::io::Result<u64> {
    let meta = fs::symlink_metadata(path)?;
    if meta.is_dir() {
        let mut total = 0;
        for entry in fs::read_dir(path)? {
            total += tree_size(&entry?.path())?;
        }
        Ok(total)
    } else if meta.is_file() {
        Ok(meta.len())
    } else {
        Ok(0)
    }
}

fn make_writable(path: &Path) {
    let Ok(meta) = fs::symlink_metadata(path) else { return };
    if meta.is_dir() {
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
        if let Ok(entries) = fs::read_dir(path) {
            for entry in entries.flatten() {
                make_writable(&entry.path());
            }
        }
    }
}

/// Removes a file, link or folder tree; missing is fine.
pub(crate) fn remove_all(path: &Path) -> std::io::Result<()> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let removed = if meta.is_dir() {
        match fs::remove_dir_all(path) {
            Err(error) if error.kind() == ErrorKind::PermissionDenied => {
                make_writable(path);
                fs::remove_dir_all(path)
            }
            other => other,
        }
    } else {
        fs::remove_file(path)
    };
    match removed {
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        other => other,
    }
}
