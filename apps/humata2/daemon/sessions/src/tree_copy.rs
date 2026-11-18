//! The one copy of a folder tree, for a carry (a move) and a fork alike: its
//! folders, its files with their modified times, and its links as links,
//! never followed (see [`Rebase::target`]). A socket or other special file
//! is left out, as no copy could carry it.

use std::fs;
use std::path::{Path, PathBuf};

/// What failed copying a tree, and where. A carry and a fork each report it
/// in their own error.
#[derive(Debug, thiserror::Error)]
#[error("couldn't {doing} {}: {source}", path.display())]
pub(crate) struct TreeCopyError {
    pub(crate) doing: &'static str,
    pub(crate) path: PathBuf,
    #[source]
    pub(crate) source: std::io::Error,
}

fn io(doing: &'static str, path: &Path) -> impl FnOnce(std::io::Error) -> TreeCopyError {
    let path = path.to_owned();
    move |source| TreeCopyError { doing, path, source }
}

/// Whether a copied folder gets the mode of the folder it is copied from.
#[derive(Clone, Copy)]
pub(crate) enum Dirs {
    /// As the system makes it.
    Default,
    /// The source folder's mode, set once its contents are in.
    KeepModes,
}

/// Copies `from` to `to`, which is to end up at `lands_at` (the same place,
/// unless the copy is made aside and renamed): a link into `from` points
/// into `lands_at` in the copy.
pub(crate) fn copy_tree(from: &Path, to: &Path, lands_at: &Path, dirs: Dirs) -> Result<(), TreeCopyError> {
    copy_under(from, to, &Rebase { from, to: lands_at }, dirs)
}

fn copy_under(from: &Path, to: &Path, rebase: &Rebase<'_>, dirs: Dirs) -> Result<(), TreeCopyError> {
    fs::create_dir_all(to).map_err(io("create", to))?;
    for entry in fs::read_dir(from).map_err(io("read the folder", from))? {
        let entry = entry.map_err(io("read the folder", from))?;
        let (source, target) = (entry.path(), to.join(entry.file_name()));
        if git_lock(&source, rebase.from) {
            continue;
        }
        left_out_if_gone(copy_entry(&source, &target, rebase, dirs))?;
    }
    if matches!(dirs, Dirs::KeepModes) {
        let meta = fs::metadata(from).map_err(io("look at", from))?;
        fs::set_permissions(to, meta.permissions()).map_err(io("set the mode of", to))?;
    }
    Ok(())
}

/// Copies the one entry `source` of a listed folder to `target`.
fn copy_entry(source: &Path, target: &Path, rebase: &Rebase<'_>, dirs: Dirs) -> Result<(), TreeCopyError> {
    let meta = fs::symlink_metadata(source).map_err(io("look at", source))?;
    if meta.is_dir() {
        copy_under(source, target, rebase, dirs)
    } else if meta.is_symlink() {
        copy_link(source, target, rebase)
    } else if meta.is_file() {
        copy_file(source, target, &meta)
    } else {
        Ok(())
    }
}

/// Whether `source`, in the tree copied from `root`, is one of git's lock
/// files (a `*.lock` under `.git`): held only while a git command runs, so
/// a copy of one would leave the copy's clone looking locked to every git
/// command after.
fn git_lock(source: &Path, root: &Path) -> bool {
    let inside_git = source
        .strip_prefix(root)
        .is_ok_and(|rest| rest.components().any(|part| part.as_os_str() == ".git"));
    inside_git && source.extension().is_some_and(|ext| ext == "lock")
}

/// A tree in use changes while it is copied: git's lock files come and go
/// in a clone whose status is read. An entry gone since its folder was
/// listed is no longer in the tree, so the copy leaves it out instead of
/// failing.
fn left_out_if_gone(copied: Result<(), TreeCopyError>) -> Result<(), TreeCopyError> {
    match copied {
        Err(error) if error.source.kind() == std::io::ErrorKind::NotFound && !error.path.exists() => {
            Ok(())
        }
        other => other,
    }
}

/// The folder a copy is made from and the folder it is made to.
pub(crate) struct Rebase<'a> {
    pub(crate) from: &'a Path,
    pub(crate) to: &'a Path,
}

impl Rebase<'_> {
    /// Where the copy of a link to `target` points: a relative target as it
    /// is, an absolute one inside the copied folder at the same place in the
    /// copy, and any other as it is.
    pub(crate) fn target(&self, target: &Path) -> PathBuf {
        if !target.is_absolute() {
            return target.to_owned();
        }
        match target.strip_prefix(self.from) {
            Ok(rest) if rest.as_os_str().is_empty() => self.to.to_owned(),
            Ok(rest) => self.to.join(rest),
            Err(_) => target.to_owned(),
        }
    }
}

/// Makes `target` a link like `source`, pointing where [`Rebase::target`]
/// says; the link is never followed.
fn copy_link(source: &Path, target: &Path, rebase: &Rebase<'_>) -> Result<(), TreeCopyError> {
    let points = fs::read_link(source).map_err(io("read the link", source))?;
    std::os::unix::fs::symlink(rebase.target(&points), target).map_err(io("make the link", target))
}

/// Copies the file `source` (its metadata `meta`) to `target` with its
/// modified time, so a later sync sees the copy as unchanged.
pub(crate) fn copy_file(source: &Path, target: &Path, meta: &fs::Metadata) -> Result<(), TreeCopyError> {
    fs::copy(source, target).map_err(io("copy", source))?;
    let modified = meta.modified().map_err(io("read the time of", source))?;
    fs::File::open(target)
        .and_then(|file| file.set_modified(modified))
        .map_err(io("set the time of", target))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_entry_gone_since_its_folder_was_listed_is_left_out() {
        let dir = tempfile::tempdir().expect("a temp folder");
        let (from, to) = (dir.path().join("from"), dir.path().join("to"));
        fs::create_dir_all(&from).expect("make the source");
        fs::create_dir_all(&to).expect("make the copy");
        let gone = from.join("index.lock");
        let copied = copy_entry(&gone, &to.join("index.lock"), &Rebase { from: &from, to: &to }, Dirs::Default);
        assert!(left_out_if_gone(copied).is_ok());
        assert!(!to.join("index.lock").exists());
    }

    #[test]
    fn gits_lock_files_are_not_copied_and_the_rest_is() {
        let dir = tempfile::tempdir().expect("a temp folder");
        let (from, to) = (dir.path().join("from"), dir.path().join("to"));
        fs::create_dir_all(from.join(".git/refs/heads")).expect("make the clone");
        fs::write(from.join(".git/index"), "index").expect("write the index");
        fs::write(from.join(".git/index.lock"), "held").expect("write a lock");
        fs::write(from.join(".git/refs/heads/main.lock"), "held").expect("write a ref lock");
        fs::write(from.join("Cargo.lock"), "pinned").expect("write a project lock file");
        copy_tree(&from, &to, &to, Dirs::Default).expect("the copy");
        assert!(to.join(".git/index").is_file());
        assert!(!to.join(".git/index.lock").exists());
        assert!(!to.join(".git/refs/heads/main.lock").exists());
        assert!(to.join("Cargo.lock").is_file(), "a lock file outside .git is the project's");
    }

    #[test]
    fn a_copy_that_fails_otherwise_still_fails() {
        let dir = tempfile::tempdir().expect("a temp folder");
        let from = dir.path().join("from");
        fs::create_dir_all(&from).expect("make the source");
        fs::write(from.join("file"), "x").expect("write a file");
        // The copy's folder is missing, so the file can't be written there.
        let target = dir.path().join("missing").join("file");
        let copied = copy_entry(&from.join("file"), &target, &Rebase { from: &from, to: dir.path() }, Dirs::Default);
        assert!(left_out_if_gone(copied).is_err());
    }
}
