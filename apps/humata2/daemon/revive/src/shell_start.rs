//! Where a restarted shell starts (D480): the folder its program last
//! reported, when that is still a folder on this machine, else the folder
//! the tab belongs to, else home.

use std::path::{Path, PathBuf};

/// The folder for a new shell of a tab. `reported` is the tab's stored
/// folder (empty or `None` when the program never said), `folder` the
/// session's, `home` the last resort.
pub(crate) fn start_folder(reported: Option<&str>, folder: Option<PathBuf>, home: &Path) -> PathBuf {
    reported
        .filter(|reported| !reported.is_empty())
        .map(PathBuf::from)
        .filter(|reported| reported.is_dir())
        .or_else(|| folder.filter(|folder| folder.is_dir()))
        .unwrap_or_else(|| home.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_shell_starts_in_the_reported_folder() {
        let reported = tempfile::tempdir().unwrap();
        let session = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let start = start_folder(
            reported.path().to_str(),
            Some(session.path().to_owned()),
            home.path(),
        );
        assert_eq!(start, reported.path());
    }

    #[test]
    fn a_reported_folder_that_is_gone_falls_back_to_the_sessions() {
        let session = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let gone = session.path().join("deleted");
        let start = start_folder(gone.to_str(), Some(session.path().to_owned()), home.path());
        assert_eq!(start, session.path());
    }

    #[test]
    fn with_nothing_usable_it_starts_at_home() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(start_folder(None, None, home.path()), home.path());
        assert_eq!(start_folder(Some(""), None, home.path()), home.path());
        assert_eq!(
            start_folder(Some("/no/such/folder"), Some(PathBuf::from("/also/gone")), home.path()),
            home.path()
        );
    }
}
