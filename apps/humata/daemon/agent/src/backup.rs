//! A session's recovery copy: what sets it up again on a host that lost it,
//! or on another one. Its code isn't in here: it is in the snapshots its
//! history names, which travel as git objects.

use std::path::{Component, Path, PathBuf};

use crate::error::AgentError;
use crate::event::{Event, SessionRecord};
use crate::links::Links;

/// A session as it was: its record, its history, and the backend's own
/// state of it.
#[derive(Clone, Debug, PartialEq)]
pub struct Backup {
    pub record: SessionRecord,
    pub events: Vec<Event>,
    pub backend: Vec<BackendFile>,
}

/// A file of the backend's own state of a session, named by a relative
/// path the backend's runner chooses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackendFile {
    pub name: String,
    pub contents: Vec<u8>,
}

/// The name of a session's links among its backend files.
pub(crate) const LINKS: &str = "links.json";

/// The session's links file under `state`, if it has one, as a backend file.
pub(crate) fn links_file(state: &Path, session: &str) -> Result<Option<BackendFile>, AgentError> {
    Ok(
        read_optional(&Links::path(state, session))?.map(|contents| BackendFile {
            name: LINKS.to_owned(),
            contents,
        }),
    )
}

/// Puts back the session's links under `state`, as [`links_file`] read them.
pub(crate) fn write_links(state: &Path, session: &str, contents: &[u8]) -> Result<(), AgentError> {
    let name = Links::path(Path::new(""), session);
    write_under(state, &name.to_string_lossy(), contents)
}

/// The backend sessions the links of `files` name: the session's own and,
/// for a fork before its first turn, the one it forks.
pub(crate) fn linked(files: &[BackendFile]) -> Result<Vec<String>, AgentError> {
    let Some(file) = files.iter().find(|file| file.name == LINKS) else {
        return Ok(Vec::new());
    };
    let links: Links = serde_json::from_slice(&file.contents).map_err(|source| {
        AgentError::CorruptBackendState {
            path: PathBuf::from(LINKS),
            source,
        }
    })?;
    Ok(links
        .session
        .into_iter()
        .chain(links.fork.map(|fork| fork.source))
        .collect())
}

/// Every file under `dir`, named `prefix/` and its path under `dir`; none if
/// `dir` doesn't exist.
pub(crate) fn read_tree(dir: &Path, prefix: &str) -> Result<Vec<BackendFile>, AgentError> {
    let mut files = Vec::new();
    let mut folders = vec![dir.to_owned()];
    while let Some(folder) = folders.pop() {
        let entries = match std::fs::read_dir(&folder) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(source) => {
                return Err(AgentError::ReadDirectory {
                    path: folder,
                    source,
                });
            }
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                folders.push(path);
                continue;
            }
            let Ok(relative) = path.strip_prefix(dir) else {
                continue;
            };
            let contents = std::fs::read(&path).map_err(|source| AgentError::Read {
                path: path.clone(),
                source,
            })?;
            files.push(BackendFile {
                name: format!("{prefix}/{}", relative.to_string_lossy()),
                contents,
            });
        }
    }
    files.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(files)
}

/// The file's contents; None if there is none.
pub(crate) fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, AgentError> {
    match std::fs::read(path) {
        Ok(contents) => Ok(Some(contents)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(AgentError::Read {
            path: path.to_owned(),
            source,
        }),
    }
}

/// Writes `contents` to `relative` under `dir`, refusing a name that would
/// leave it.
pub(crate) fn write_under(dir: &Path, relative: &str, contents: &[u8]) -> Result<(), AgentError> {
    let relative = Path::new(relative);
    let inside = relative
        .components()
        .all(|component| matches!(component, Component::Normal(_)));
    if !inside {
        return Err(AgentError::BadBackendFile {
            name: relative.to_string_lossy().into_owned(),
        });
    }
    let path = dir.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| AgentError::CreateDirectory {
            path: parent.to_owned(),
            source,
        })?;
    }
    std::fs::write(&path, contents).map_err(|source| AgentError::Write { path, source })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tree_read_is_written_back_the_same() {
        let from = tempfile::tempdir().expect("a folder");
        write_under(from.path(), "a/b.txt", b"one").expect("write");
        write_under(from.path(), "c.txt", b"two").expect("write");
        let files = read_tree(from.path(), "x").expect("read");
        let to = tempfile::tempdir().expect("a folder");
        for file in &files {
            let name = file.name.strip_prefix("x/").expect("the prefix");
            write_under(to.path(), name, &file.contents).expect("write");
        }
        assert_eq!(read_tree(to.path(), "x").expect("read"), files);
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn a_name_that_leaves_the_folder_is_refused() {
        let dir = tempfile::tempdir().expect("a folder");
        for name in ["../out", "/etc/out", "a/../../out"] {
            assert!(matches!(
                write_under(dir.path(), name, b""),
                Err(AgentError::BadBackendFile { .. })
            ));
        }
    }
}
