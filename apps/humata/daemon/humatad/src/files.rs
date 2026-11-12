//! The files in a session's folder or any folder on a host, to browse and
//! read.

use std::io::Read as _;
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use humata_wire::pb;

use crate::error::HostError;

/// The most files listed; a folder with more lists these and says so.
const MOST_FILES: usize = 2000;

/// The most of a text file read; a larger one is cut there.
const LARGEST_FILE: u64 = 1024 * 1024;

/// The largest image sent to be shown.
const LARGEST_IMAGE: u64 = 8 * 1024 * 1024;

/// Files shown as images, by extension.
const IMAGES: [&str; 9] = [
    "png", "jpg", "jpeg", "gif", "webp", "heic", "tif", "tiff", "bmp",
];

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
                    modified_unix_ms: metadata.as_ref().map_or(0, unix_ms),
                });
            }
        }
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(pb::Files { files, truncated })
}

/// One level of `directory` under `root`, which it mustn't lead out of:
/// folders first, then files, without hidden ones.
pub fn folder(root: &Path, directory: &str) -> Result<pb::Folder, HostError> {
    let relative = Path::new(directory);
    if !plain(relative) {
        return Err(HostError::NotInFolder {
            path: directory.to_owned(),
        });
    }
    let path = root.join(relative);
    let inside = match (path.canonicalize(), root.canonicalize()) {
        (Ok(folder), Ok(root)) => folder.starts_with(root),
        _ => false,
    };
    if !inside {
        return Err(HostError::NotInFolder {
            path: directory.to_owned(),
        });
    }
    let read = |source| HostError::ReadFolder {
        path: path.clone(),
        source,
    };
    let mut entries = Vec::new();
    let mut truncated = false;
    for entry in std::fs::read_dir(&path).map_err(read)? {
        let entry = entry.map_err(read)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        if entries.len() == MOST_FILES {
            truncated = true;
            break;
        }
        // A link is followed for what it is, but reading it is still
        // checked against the root.
        let Ok(metadata) = std::fs::metadata(entry.path()) else {
            continue;
        };
        entries.push(pb::FolderEntry {
            name,
            is_directory: metadata.is_dir(),
            size: if metadata.is_dir() { 0 } else { metadata.len() },
            modified_unix_ms: unix_ms(&metadata),
        });
    }
    entries.sort_by(|a, b| {
        b.is_directory
            .cmp(&a.is_directory)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(pb::Folder {
        root: root.to_string_lossy().into_owned(),
        directory: slashed(relative),
        entries,
        truncated,
    })
}

/// Whether `path` stays where it starts: no `..`, no root, no prefix.
fn plain(path: &Path) -> bool {
    path.components()
        .all(|component| matches!(component, Component::Normal(_)))
}

fn unix_ms(metadata: &std::fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |since| {
            i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
        })
}

/// The file at `path` under `root`, which it mustn't lead out of: its text,
/// cut if it's large, or an image's bytes.
pub fn read(root: &Path, path: &str) -> Result<pb::FileContents, HostError> {
    let relative = Path::new(path);
    if path.is_empty() || !plain(relative) {
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
    let size = metadata.len();
    let image = relative
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| IMAGES.contains(&extension.to_lowercase().as_str()));
    let failed = |source| HostError::ReadFile {
        path: full.clone(),
        source,
    };
    if image {
        let data = if size <= LARGEST_IMAGE {
            std::fs::read(&full).map_err(failed)?
        } else {
            Vec::new()
        };
        return Ok(pb::FileContents {
            path: path.to_owned(),
            binary: true,
            data,
            size,
            ..pb::FileContents::default()
        });
    }
    let mut bytes = Vec::new();
    std::fs::File::open(&full)
        .and_then(|file| file.take(LARGEST_FILE).read_to_end(&mut bytes))
        .map_err(failed)?;
    let truncated = size > LARGEST_FILE;
    // A cut can fall inside a character: what's before it is still text.
    let text = match std::str::from_utf8(&bytes) {
        Ok(text) => Some(text),
        Err(error) if truncated && error.error_len().is_none() => {
            std::str::from_utf8(&bytes[..error.valid_up_to()]).ok()
        }
        Err(_) => None,
    };
    Ok(pb::FileContents {
        path: path.to_owned(),
        text: text.unwrap_or_default().to_owned(),
        binary: text.is_none(),
        size,
        truncated: truncated && text.is_some(),
        ..pb::FileContents::default()
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
    fn a_folder_lists_one_level_folders_first() {
        let dir = folder();
        std::fs::write(dir.path().join("Zeta.txt"), "z").expect("a file");
        let listed = folder_at(dir.path(), "");
        let names: Vec<_> = listed
            .entries
            .iter()
            .map(|entry| (entry.name.as_str(), entry.is_directory))
            .collect();
        assert_eq!(
            names,
            [("docs", true), ("plan.md", false), ("Zeta.txt", false)]
        );
        let notes = folder_at(dir.path(), "docs/notes");
        assert_eq!(notes.entries.len(), 1);
        assert_eq!(notes.entries[0].size, 1);
        for outside in ["..", "docs/../..", "/etc"] {
            assert!(
                matches!(
                    super::folder(dir.path(), outside),
                    Err(HostError::NotInFolder { .. })
                ),
                "{outside}"
            );
        }
    }

    fn folder_at(root: &Path, directory: &str) -> pb::Folder {
        super::folder(root, directory).expect("listed")
    }

    #[test]
    fn a_large_text_file_is_cut_and_an_image_is_sent_whole() {
        let dir = folder();
        let large = "é".repeat(usize::try_from(LARGEST_FILE).expect("small") / 2 + 1);
        std::fs::write(dir.path().join("large.txt"), &large).expect("a file");
        let contents = read(dir.path(), "large.txt").expect("read");
        assert!(contents.truncated && !contents.binary);
        assert!(contents.text.len() < large.len() && large.starts_with(&contents.text));
        assert_eq!(contents.size, large.len() as u64);

        std::fs::write(dir.path().join("photo.PNG"), [0x89, b'P', b'N', b'G']).expect("a file");
        let image = read(dir.path(), "photo.PNG").expect("read");
        assert!(image.binary);
        assert_eq!(image.data, [0x89, b'P', b'N', b'G']);
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
