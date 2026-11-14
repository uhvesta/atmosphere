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

/// `list`, on a thread for blocking work, as a big folder takes a while.
pub async fn list_apart(root: PathBuf) -> Result<pb::Files, HostError> {
    let path = root.clone();
    tokio::task::spawn_blocking(move || list(&root))
        .await
        .map_err(|source| HostError::ReadFiles { path, source })?
}

/// `read`, on a thread for blocking work.
pub async fn read_apart(root: PathBuf, path: String) -> Result<pb::FileContents, HostError> {
    let folder = root.clone();
    tokio::task::spawn_blocking(move || read(&root, &path))
        .await
        .map_err(|source| HostError::ReadFiles {
            path: folder,
            source,
        })?
}

/// `folder`, on a thread for blocking work, as it looks at each entry and
/// a network disk is slow to answer.
pub async fn folder_apart(root: PathBuf, directory: String) -> Result<pb::Folder, HostError> {
    let path = root.clone();
    tokio::task::spawn_blocking(move || folder(&root, &directory))
        .await
        .map_err(|source| HostError::ReadFiles { path, source })?
}

/// Every file under `root` but hidden ones and those in hidden folders,
/// such as `.git`, by path. A link to a file inside `root` is listed as
/// that file; a link to a folder, or one that leads out, isn't. It stops
/// at the most it lists, rather than walking the rest of a tree such as
/// `node_modules`.
pub fn list(root: &Path) -> Result<pb::Files, HostError> {
    // None if it can't be resolved, and then no link is listed.
    let resolved_root = root.canonicalize().ok();
    let mut files = Vec::new();
    let mut folders = vec![PathBuf::new()];
    let mut truncated = false;
    'walk: while let Some(folder) = folders.pop() {
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
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            // A linked folder is never walked: it could lead out of the
            // folder, or back into it for ever.
            if kind.is_dir() {
                folders.push(relative);
                continue;
            }
            // The file's own size and time, or those of the file a link
            // leads to when that is inside the folder.
            let metadata = if kind.is_file() {
                entry.metadata().ok()
            } else if kind.is_symlink() {
                let target = entry.path().canonicalize().ok();
                let inside = match (&target, &resolved_root) {
                    (Some(target), Some(root)) => target.starts_with(root),
                    _ => false,
                };
                let Some(metadata) = target
                    .filter(|_| inside)
                    .and_then(|target| std::fs::metadata(target).ok())
                    .filter(std::fs::Metadata::is_file)
                else {
                    continue;
                };
                Some(metadata)
            } else {
                continue;
            };
            if files.len() == MOST_FILES {
                truncated = true;
                break 'walk;
            }
            files.push(pb::ChatFile {
                path: slashed(&relative),
                size: metadata.as_ref().map_or(0, std::fs::Metadata::len),
                modified_unix_ms: metadata.as_ref().map_or(0, unix_ms),
            });
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
    // What a link leads to, so a link to a file is read as that file.
    let metadata = std::fs::metadata(&full).map_err(|source| HostError::ReadFile {
        path: full.clone(),
        source,
    })?;
    // A link, or a linked folder on the way, could still lead out.
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

/// `text`, a version of the file at `path`, as it's sent to be shown: cut
/// if it's large, or marked as binary.
pub fn version(path: &str, text: String) -> pb::FileContents {
    let size = text.len() as u64;
    if text.contains('\0') {
        return pb::FileContents {
            path: path.to_owned(),
            binary: true,
            size,
            ..pb::FileContents::default()
        };
    }
    let mut text = text;
    let truncated = size > LARGEST_FILE;
    if truncated {
        let mut end = usize::try_from(LARGEST_FILE).unwrap_or(usize::MAX);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    pb::FileContents {
        path: path.to_owned(),
        text,
        size,
        truncated,
        ..pb::FileContents::default()
    }
}

/// `write`, on a thread for blocking work.
pub async fn write_apart(root: PathBuf, path: String, text: String) -> Result<(), HostError> {
    let folder = root.clone();
    tokio::task::spawn_blocking(move || write(&root, &path, &text))
        .await
        .map_err(|source| HostError::ReadFiles {
            path: folder,
            source,
        })?
}

/// Writes `text` as the file at `path` under `root`, which it mustn't lead
/// out of, over what's there or as a new file in a folder that is.
pub fn write(root: &Path, path: &str, text: &str) -> Result<(), HostError> {
    let relative = Path::new(path);
    if path.is_empty() || !plain(relative) {
        return Err(HostError::NotInFolder {
            path: path.to_owned(),
        });
    }
    let full = root.join(relative);
    // A link whose target doesn't exist: writing through it would make the
    // target wherever the link points.
    if !full.exists() && std::fs::symlink_metadata(&full).is_ok() {
        return Err(HostError::NotInFolder {
            path: path.to_owned(),
        });
    }
    // What's there, or the folder it goes in, must be inside the root, so a
    // linked file or folder can't lead out.
    let there = if full.exists() {
        full.canonicalize()
    } else {
        full.parent()
            .map_or_else(|| root.canonicalize(), Path::canonicalize)
    };
    let inside = match (there, root.canonicalize()) {
        (Ok(there), Ok(root)) => there.starts_with(root),
        _ => false,
    };
    if !inside || full.is_dir() {
        return Err(HostError::NotInFolder {
            path: path.to_owned(),
        });
    }
    std::fs::write(&full, text).map_err(|source| HostError::WriteFile { path: full, source })
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
    fn a_version_is_written_in_the_folder_and_never_outside_it() {
        let dir = folder();
        write(dir.path(), "plan.md", "# Plan, again\n").expect("wrote");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("plan.md")).expect("read"),
            "# Plan, again\n"
        );
        write(dir.path(), "docs/new.md", "new").expect("wrote a new file");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("docs/new.md")).expect("read"),
            "new"
        );
        for outside in ["../out.md", "/tmp/out.md", "", "docs"] {
            assert!(
                matches!(
                    write(dir.path(), outside, "x"),
                    Err(HostError::NotInFolder { .. })
                ),
                "{outside:?}"
            );
        }
    }

    #[test]
    fn a_large_version_is_cut_and_a_binary_one_isnt_shown() {
        let large = "a".repeat(usize::try_from(LARGEST_FILE).expect("fits") + 10);
        let cut = version("big.md", large);
        assert!(cut.truncated);
        assert_eq!(cut.text.len() as u64, LARGEST_FILE);
        let binary = version("blob.md", "a\0b".to_owned());
        assert!(binary.binary);
        assert!(binary.text.is_empty());
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

    fn link(target: impl AsRef<Path>, at: impl AsRef<Path>) {
        std::os::unix::fs::symlink(target, at).expect("a link");
    }

    fn listed(root: &Path) -> Vec<String> {
        let files = list(root).expect("listed");
        files.files.into_iter().map(|file| file.path).collect()
    }

    #[test]
    fn a_link_to_a_file_inside_the_folder_is_read_and_listed() {
        let dir = folder();
        link("docs/notes/a.md", dir.path().join("current.md"));

        let contents = read(dir.path(), "current.md").expect("read");
        let files = list(dir.path()).expect("listed");

        assert_eq!(contents.text, "a");
        assert_eq!(contents.size, 1);
        let entry = files.files.iter().find(|file| file.path == "current.md");
        assert_eq!(entry.map(|file| file.size), Some(1));
    }

    #[test]
    fn a_link_that_leads_out_is_neither_read_nor_listed() {
        let outside = tempfile::tempdir().expect("a temporary folder");
        std::fs::write(outside.path().join("secret.txt"), "secret").expect("a file");
        let dir = folder();
        link(
            outside.path().join("secret.txt"),
            dir.path().join("out.txt"),
        );
        link(
            outside.path().join("gone.txt"),
            dir.path().join("nowhere.txt"),
        );

        assert!(matches!(
            read(dir.path(), "out.txt"),
            Err(HostError::NotInFolder { .. })
        ));
        assert!(read(dir.path(), "nowhere.txt").is_err());
        assert_eq!(listed(dir.path()), ["docs/notes/a.md", "plan.md"]);
    }

    #[test]
    fn a_link_to_a_folder_isnt_walked() {
        let outside = tempfile::tempdir().expect("a temporary folder");
        std::fs::write(outside.path().join("secret.txt"), "secret").expect("a file");
        let dir = folder();
        link(outside.path(), dir.path().join("elsewhere"));
        link("docs", dir.path().join("same"));
        link("..", dir.path().join("docs/up"));

        assert_eq!(listed(dir.path()), ["docs/notes/a.md", "plan.md"]);
        assert!(matches!(
            read(dir.path(), "same"),
            Err(HostError::NotInFolder { .. })
        ));
        assert!(matches!(
            read(dir.path(), "elsewhere/secret.txt"),
            Err(HostError::NotInFolder { .. })
        ));
    }

    #[test]
    fn writing_through_a_link_that_leads_nowhere_is_refused() {
        let outside = tempfile::tempdir().expect("a temporary folder");
        let dir = folder();
        link(outside.path().join("made.txt"), dir.path().join("out.txt"));
        link("docs/later.md", dir.path().join("inside.md"));

        for path in ["out.txt", "inside.md"] {
            assert!(
                matches!(
                    write(dir.path(), path, "x"),
                    Err(HostError::NotInFolder { .. })
                ),
                "{path}"
            );
        }
        assert!(!outside.path().join("made.txt").exists());
        assert!(!dir.path().join("docs/later.md").exists());
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
