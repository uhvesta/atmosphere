//! What Humata keeps on a machine, measured, and the ways to free it (LD-13,
//! LD-14, LD-15): the size of a tree, the free space of a disk, the build
//! caches in a folder, clearing one and trimming logs. Everything here is
//! synchronous file work; call it on a blocking thread.

use std::collections::HashSet;
use std::fs;
use std::io::{ErrorKind, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use crate::fsutil::remove_all;

/// What the Storage page and the actions on it can fail at.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error(
        "couldn't {doing} {}; check that the folder belongs to you and still exists, then measure again",
        path.display()
    )]
    Io {
        doing: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "{} is a link, not a folder Humata made, so it wasn't cleared; delete the link by hand if it isn't needed",
        path.display()
    )]
    Link { path: PathBuf },

    #[error("{} has no parent folder to clear it in", path.display())]
    NoParent { path: PathBuf },
}

fn io(doing: &'static str, path: &Path) -> impl FnOnce(std::io::Error) -> StorageError {
    let path = path.to_owned();
    move |source| StorageError::Io { doing, path, source }
}

/// How much a tree holds on disk and when anything in it last changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Measured {
    /// What deleting the tree would free.
    pub bytes: u64,
    /// The newest modification time of any file in it, in Unix ms.
    pub last_used_unix_ms: Option<i64>,
}

/// A disk's space as an unprivileged user sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskSpace {
    pub free_bytes: u64,
    pub total_bytes: u64,
}

/// A folder a build recreates, found by [`build_caches`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildCache {
    /// Relative to the folder searched.
    pub path: PathBuf,
    pub bytes: u64,
    pub last_used_unix_ms: Option<i64>,
}

/// Prefix of a folder being removed by [`clear`].
const CLEARING: &str = ".humata-clearing-";

/// Suffix of the file a trim writes beside the log.
const TRIMMING: &str = ".humata-trimming";

/// The deepest a build cache is looked for, in levels below the folder.
const SEARCH_DEPTH: usize = 4;

fn modified_ms(meta: &fs::Metadata) -> i64 {
    meta.mtime()
        .saturating_mul(1000)
        .saturating_add(meta.mtime_nsec() / 1_000_000)
}

/// What deleting `path` would free: its allocated size, each hard-linked
/// file counted once, links not followed, and what vanishes while
/// measuring skipped. A missing path measures as 0. Reads no file
/// contents.
pub fn measure(path: &Path) -> Result<Measured, StorageError> {
    let mut bytes = 0u64;
    let mut last: Option<i64> = None;
    let mut seen: HashSet<(u64, u64)> = HashSet::new();
    let mut stack = vec![path.to_owned()];
    while let Some(next) = stack.pop() {
        let meta = match fs::symlink_metadata(&next) {
            Ok(meta) => meta,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => return Err(io("look at", &next)(error)),
        };
        if meta.is_dir() {
            bytes = bytes.saturating_add(meta.blocks().saturating_mul(512));
            let entries = match fs::read_dir(&next) {
                Ok(entries) => entries,
                Err(error) if error.kind() == ErrorKind::NotFound => continue,
                Err(error) => return Err(io("read the folder", &next)(error)),
            };
            for entry in entries {
                match entry {
                    Ok(entry) => stack.push(entry.path()),
                    Err(error) if error.kind() == ErrorKind::NotFound => {}
                    Err(error) => return Err(io("read the folder", &next)(error)),
                }
            }
            continue;
        }
        if meta.is_file() {
            let at = modified_ms(&meta);
            last = Some(last.map_or(at, |last| last.max(at)));
        }
        if meta.nlink() > 1 && !seen.insert((meta.dev(), meta.ino())) {
            continue;
        }
        bytes = bytes.saturating_add(meta.blocks().saturating_mul(512));
    }
    Ok(Measured {
        bytes,
        last_used_unix_ms: last,
    })
}

/// The free and total space of the disk `folder` is on: what an
/// unprivileged user can use.
pub fn disk_space(folder: &Path) -> Result<DiskSpace, StorageError> {
    let stat = rustix::fs::statvfs(folder).map_err(|errno| StorageError::Io {
        doing: "measure the free space of",
        path: folder.to_owned(),
        source: errno.into(),
    })?;
    Ok(DiskSpace {
        free_bytes: stat.f_bavail.saturating_mul(stat.f_frsize),
        total_bytes: stat.f_blocks.saturating_mul(stat.f_frsize),
    })
}

/// Whether a folder named `name` beside `siblings` is a build cache.
fn is_cache_name(name: &str, siblings: &HashSet<String>) -> bool {
    let has = |file: &str| siblings.contains(file);
    match name {
        "target" => has("Cargo.toml"),
        "node_modules" | ".next" => has("package.json"),
        ".build" => has("Package.swift"),
        ".gradle" | "build" => has("build.gradle") || has("build.gradle.kts"),
        "__pycache__" => true,
        ".venv" => has("pyproject.toml") || has("requirements.txt"),
        _ => false,
    }
}

/// The build caches under `folder`, largest first: folders that are safe to
/// delete because a build recreates them. Looks at most four levels down,
/// never inside `.git` or a cache found, and never through links.
pub fn build_caches(folder: &Path) -> Result<Vec<BuildCache>, StorageError> {
    let mut found: Vec<PathBuf> = Vec::new();
    let mut stack: Vec<(PathBuf, usize)> = vec![(folder.to_owned(), 0)];
    while let Some((dir, depth)) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) if depth > 0 => continue,
            Err(error) => return Err(io("read the folder", &dir)(error)),
        };
        let mut folders: Vec<String> = Vec::new();
        let mut names: HashSet<String> = HashSet::new();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                folders.push(name.clone());
            }
            names.insert(name);
        }
        for name in folders {
            if name == ".git" {
                continue;
            }
            let child = dir.join(&name);
            let tagged = fs::symlink_metadata(child.join("CACHEDIR.TAG"))
                .is_ok_and(|meta| meta.is_file());
            if tagged || is_cache_name(&name, &names) {
                found.push(child);
            } else if depth + 1 < SEARCH_DEPTH {
                stack.push((child, depth + 1));
            }
        }
    }
    let mut caches = Vec::with_capacity(found.len());
    for path in found {
        let measured = measure(&path)?;
        let relative = path.strip_prefix(folder).unwrap_or(&path).to_owned();
        caches.push(BuildCache {
            path: relative,
            bytes: measured.bytes,
            last_used_unix_ms: measured.last_used_unix_ms,
        });
    }
    caches.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.path.cmp(&b.path)));
    Ok(caches)
}

/// Removes the folder `path` and returns the bytes it held. It is renamed to
/// a sibling `.humata-clearing-<id>` first, one atomic step, so after a
/// crash the folder is either whole or on its way out;
/// [`sweep_cleared`] finishes what a crash left. A link is refused; a
/// missing path frees 0.
pub fn clear(path: &Path) -> Result<u64, StorageError> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(StorageError::Link {
                path: path.to_owned(),
            });
        }
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(io("look at", path)(error)),
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| StorageError::NoParent {
            path: path.to_owned(),
        })?;
    let bytes = measure(path)?.bytes;
    let moved = parent.join(format!("{CLEARING}{}", uuid::Uuid::new_v4().simple()));
    fs::rename(path, &moved).map_err(io("set aside", path))?;
    remove_all(&moved).map_err(io("remove", &moved))?;
    Ok(bytes)
}

/// Removes the `.humata-clearing-*` folders a [`clear`] that a crash
/// interrupted left in `folder`.
pub fn sweep_cleared(folder: &Path) -> Result<(), StorageError> {
    let entries = match fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(io("read the folder", folder)(error)),
    };
    for entry in entries {
        let entry = entry.map_err(io("read the folder", folder))?;
        if entry.file_name().to_string_lossy().starts_with(CLEARING) {
            let path = entry.path();
            remove_all(&path).map_err(io("remove", &path))?;
        }
    }
    Ok(())
}

/// For each regular file in `folder` and in its subfolders (one level),
/// keeps the last `keep_bytes_per_file` bytes, cut at a line start, and
/// returns the bytes freed. Files already small are untouched.
///
/// The tail is written to a temp file beside the log and renamed over it. A
/// process holding the old file open keeps writing to the old, now
/// unlinked file: its later lines are lost until it reopens the log. Trim
/// logs of a running daemon knowing that.
pub fn trim_logs(folder: &Path, keep_bytes_per_file: u64) -> Result<u64, StorageError> {
    let mut freed = 0u64;
    let mut folders = vec![(folder.to_owned(), true)];
    while let Some((dir, descend)) = folders.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => return Err(io("read the folder", &dir)(error)),
        };
        for entry in entries {
            let entry = entry.map_err(io("read the folder", &dir))?;
            let path = entry.path();
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_dir() {
                if descend {
                    folders.push((path, false));
                }
            } else if kind.is_file() {
                if path.to_string_lossy().ends_with(TRIMMING) {
                    remove_all(&path).map_err(io("remove", &path))?;
                } else {
                    freed = freed.saturating_add(trim_file(&path, keep_bytes_per_file)?);
                }
            }
        }
    }
    Ok(freed)
}

fn trim_file(path: &Path, keep: u64) -> Result<u64, StorageError> {
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(io("open", path)(error)),
    };
    let meta = file.metadata().map_err(io("look at", path))?;
    let len = meta.len();
    if len <= keep {
        return Ok(0);
    }
    // One byte before the tail tells whether it starts at a line start.
    let from = len - keep;
    let read_from = from.saturating_sub(1);
    file.seek(SeekFrom::Start(read_from))
        .map_err(io("read", path))?;
    let mut buf = Vec::with_capacity(usize::try_from(keep).unwrap_or(0).saturating_add(1));
    file.take(len - read_from)
        .read_to_end(&mut buf)
        .map_err(io("read", path))?;
    let skip = usize::try_from(from - read_from).unwrap_or(0);
    let starts_line = skip == 0 || buf.first() == Some(&b'\n');
    let tail = &buf[skip.min(buf.len())..];
    let tail = if starts_line {
        tail
    } else {
        match tail.iter().position(|byte| *byte == b'\n') {
            Some(at) => &tail[at + 1..],
            None => &tail[tail.len()..],
        }
    };
    let mut name = path.as_os_str().to_os_string();
    name.push(TRIMMING);
    let temp = PathBuf::from(name);
    let written = (|| {
        let mut out = fs::File::create(&temp)?;
        out.write_all(tail)?;
        out.set_permissions(fs::Permissions::from_mode(meta.mode() & 0o7777))?;
        out.sync_all()?;
        fs::rename(&temp, path)
    })();
    if let Err(source) = written {
        let _ = fs::remove_file(&temp);
        return Err(StorageError::Io {
            doing: "trim",
            path: path.to_owned(),
            source,
        });
    }
    Ok(len - u64::try_from(tail.len()).unwrap_or(len))
}
