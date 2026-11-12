//! The files in a session's folder, to browse and read.

use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use humata_wire::pb;

use crate::error::HostError;

/// The most files listed; a folder with more lists these and says so.
const MOST_FILES: usize = 2000;

/// The largest file read whole.
const LARGEST_FILE: u64 = 1024 * 1024;

/// Every file under `root` but hidden ones and those in hidden folders,
/// such as `.git`, by path.
pub fn list(root: &Path) -> Result<pb::Files, HostError> {
    let mut files = Vec::new();
    let mut folders = vec![PathBuf::new()];
    let mut truncated = false;
    while let Some(folder) = folders.pop() {
        let path = root.join(&folder);
        let entries = std::fs::read_dir(&path).map_err(|source| HostError::ReadFolder {
            path: path.clone(),
            source,
        })?;
        for entry in entries {
            let entry = entry.map_err(|source| HostError::ReadFolder {
                path: path.clone(),
                source,
            })?;
            let name = entry.file_name();
            if name.to_string_lossy().starts_with('.') {
                continue;
            }
            let relative = folder.join(&name);
            // Not followed: a link could lead out of the folder.
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                folders.push(relative);
            } else if kind.is_file() {
                if files.len() == MOST_FILES {
                    truncated = true;
                    continue;
                }
                let metadata = entry.metadata().ok();
                files.push(pb::ChatFile {
                    path: slashed(&relative),
                    size: metadata.as_ref().map_or(0, std::fs::Metadata::len),
                    modified_unix_ms: metadata
                        .and_then(|metadata| metadata.modified().ok())
                        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                        .map_or(0, |since| {
                            i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
                        }),
                });
            }
        }
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(pb::Files { files, truncated })
}

/// The file at `path` under `root`, which it mustn't lead out of.
pub fn read(root: &Path, path: &str) -> Result<pb::FileContents, HostError> {
    let relative = Path::new(path);
    let plain = relative
        .components()
        .all(|component| matches!(component, Component::Normal(_)));
    if path.is_empty() || !plain {
        return Err(HostError::NotInFolder {
            path: path.to_owned(),
        });
    }
    let full = root.join(relative);
    let metadata = std::fs::symlink_metadata(&full).map_err(|source| HostError::ReadFile {
        path: full.clone(),
        source,
    })?;
    // A linked folder on the way could still lead out.
    let inside = match (full.canonicalize(), root.canonicalize()) {
        (Ok(file), Ok(root)) => file.starts_with(root),
        _ => false,
    };
    if !metadata.is_file() || !inside {
        return Err(HostError::NotInFolder {
            path: path.to_owned(),
        });
    }
    if metadata.len() > LARGEST_FILE {
        return Err(HostError::FileTooLarge {
            path: full,
            size: metadata.len(),
        });
    }
    let bytes = std::fs::read(&full).map_err(|source| HostError::ReadFile {
        path: full.clone(),
        source,
    })?;
    Ok(match String::from_utf8(bytes) {
        Ok(text) => pb::FileContents {
            path: path.to_owned(),
            text,
            binary: false,
        },
        Err(_) => pb::FileContents {
            path: path.to_owned(),
            text: String::new(),
            binary: true,
        },
    })
}

fn slashed(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("a temporary folder");
        std::fs::create_dir_all(dir.path().join("docs/notes")).expect("folders");
        std::fs::create_dir_all(dir.path().join(".git")).expect("a hidden folder");
        std::fs::write(dir.path().join("plan.md"), "# Plan\n").expect("a file");
        std::fs::write(dir.path().join("docs/notes/a.md"), "a").expect("a file");
        std::fs::write(dir.path().join(".git/HEAD"), "ref").expect("a hidden file");
        std::fs::write(dir.path().join(".env"), "SECRET=1").expect("a hidden file");
        dir
    }

    #[test]
    fn lists_files_in_folders_but_not_hidden_ones() {
        let dir = folder();
        let files = list(dir.path()).expect("listed");
        let paths: Vec<_> = files.files.iter().map(|file| file.path.as_str()).collect();
        assert_eq!(paths, ["docs/notes/a.md", "plan.md"]);
        assert_eq!(files.files[1].size, 7);
        assert!(!files.truncated);
    }

    #[test]
    fn reads_a_file_by_its_listed_path() {
        let dir = folder();
        let contents = read(dir.path(), "docs/notes/a.md").expect("read");
        assert_eq!(contents.text, "a");
        assert!(!contents.binary);
    }

    #[test]
    fn a_path_leading_out_of_the_folder_is_refused() {
        let dir = folder();
        for path in ["../plan.md", "/etc/hosts", "docs/../../x", ""] {
            assert!(
                matches!(read(dir.path(), path), Err(HostError::NotInFolder { .. })),
                "{path}"
            );
        }
    }

    #[test]
    fn a_file_that_isnt_text_is_binary() {
        let dir = folder();
        std::fs::write(dir.path().join("image.png"), [0xff, 0xd8, 0x00]).expect("a file");
        let contents = read(dir.path(), "image.png").expect("read");
        assert!(contents.binary);
        assert!(contents.text.is_empty());
    }
}
