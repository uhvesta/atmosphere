//! Clean relative paths, so a grant on a folder can be compared with the
//! paths a call names.

use std::fmt;

/// A path relative to the root of a clone or folder, `/`-separated, with no
/// `.`, `..` or empty parts. The empty path is the root itself.
///
/// The daemon resolves `..` and symlinks before it builds one; a path that
/// leaves Humata's folders is a [`crate::Root::Outside`] place, not a
/// `RepoPath`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct RepoPath(String);

/// Why a string is not a clean relative path.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RepoPathError {
    #[error(
        "the path {path:?} starts with \"/\"; give it relative to the clone or folder root"
    )]
    Absolute { path: String },
    #[error(
        "the path {path:?} has a \".\" or \"..\" part; resolve it first, and treat a path that leaves Humata's folders as outside them"
    )]
    DotPart { path: String },
    #[error(
        "the path {path:?} has an empty part (\"//\" or a trailing \"/\"); remove the extra \"/\""
    )]
    EmptyPart { path: String },
}

impl RepoPath {
    /// The root of the clone or folder.
    pub fn root() -> Self {
        Self(String::new())
    }

    pub fn new(path: impl Into<String>) -> Result<Self, RepoPathError> {
        let path = path.into();
        if path.is_empty() {
            return Ok(Self(path));
        }
        if path.starts_with('/') {
            return Err(RepoPathError::Absolute { path });
        }
        for part in path.split('/') {
            if part.is_empty() {
                return Err(RepoPathError::EmptyPart { path });
            }
            if part == "." || part == ".." {
                return Err(RepoPathError::DotPart { path });
            }
        }
        Ok(Self(path))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// True when `other` is this path or lies under it.
    pub fn contains(&self, other: &RepoPath) -> bool {
        if self.0.is_empty() || self.0 == other.0 {
            return true;
        }
        other
            .0
            .strip_prefix(self.0.as_str())
            .is_some_and(|rest| rest.starts_with('/'))
    }

    /// The folder holding this path; the root for a top-level path and for
    /// the root itself.
    pub fn parent(&self) -> RepoPath {
        match self.0.rfind('/') {
            Some(at) => Self(self.0[..at].to_owned()),
            None => Self::root(),
        }
    }
}

impl fmt::Display for RepoPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            f.write_str("/")
        } else {
            f.write_str(&self.0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(s: &str) -> RepoPath {
        RepoPath::new(s).expect("a clean path")
    }

    #[test]
    fn a_folder_contains_itself_and_what_is_under_it() {
        assert!(path("src").contains(&path("src")));
        assert!(path("src").contains(&path("src/main.rs")));
        assert!(path("src").contains(&path("src/a/b")));
        assert!(RepoPath::root().contains(&path("anything/at/all")));
    }

    #[test]
    fn a_folder_does_not_contain_siblings_or_parents() {
        assert!(!path("src").contains(&path("srcx")));
        assert!(!path("src").contains(&path("srcx/a")));
        assert!(!path("src/a").contains(&path("src")));
        assert!(!path("src").contains(&RepoPath::root()));
    }

    #[test]
    fn parents() {
        assert_eq!(path("src/a/b.rs").parent(), path("src/a"));
        assert_eq!(path("README.md").parent(), RepoPath::root());
        assert_eq!(RepoPath::root().parent(), RepoPath::root());
    }

    #[test]
    fn unclean_paths_are_rejected() {
        assert!(matches!(
            RepoPath::new("/etc"),
            Err(RepoPathError::Absolute { .. })
        ));
        assert!(matches!(
            RepoPath::new("src/../.."),
            Err(RepoPathError::DotPart { .. })
        ));
        assert!(matches!(
            RepoPath::new("./src"),
            Err(RepoPathError::DotPart { .. })
        ));
        assert!(matches!(
            RepoPath::new("src//a"),
            Err(RepoPathError::EmptyPart { .. })
        ));
        assert!(matches!(
            RepoPath::new("src/"),
            Err(RepoPathError::EmptyPart { .. })
        ));
    }
}
