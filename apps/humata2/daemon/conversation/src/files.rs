//! File and text helpers the adapters share: reading a plain file, writing a
//! new one through a `.partial`, checking a staged name, and the text
//! rewrite a carry applies. Nothing here knows a CLI's layout.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};

use serde_json::Value;

use crate::ConversationError;

/// A conversation id must be one plain file name.
pub(crate) fn check_id(id: &str) -> Result<(), ConversationError> {
    let mut parts = Path::new(id).components();
    let plain = !id.contains(['/', '\0'])
        && matches!((parts.next(), parts.next()), (Some(Component::Normal(_)), None));
    if plain {
        Ok(())
    } else {
        Err(ConversationError::BadId { id: id.to_owned() })
    }
}

pub(crate) fn io_error(doing: &'static str, path: &Path, source: io::Error) -> ConversationError {
    ConversationError::Io {
        doing,
        path: path.to_path_buf(),
        source,
    }
}

/// The error for a complete line that isn't a record: `line` is 1-based.
pub(crate) fn malformed(path: &Path, line: u64) -> ConversationError {
    ConversationError::Malformed {
        path: path.to_path_buf(),
        line,
        source: None,
    }
}

/// Where each of the complete lines `numbers` (from 1) of `bytes` starts, in
/// bytes; the end of the complete lines for a number past the last one.
pub(crate) fn line_starts(bytes: &[u8], numbers: impl IntoIterator<Item = u64>) -> Vec<u64> {
    let mut starts = vec![0_u64];
    for line in complete_lines(bytes) {
        let last = starts.last().copied().unwrap_or(0);
        let length = u64::try_from(line.len()).unwrap_or(u64::MAX);
        starts.push(last.saturating_add(length).saturating_add(1));
    }
    let end = starts.last().copied().unwrap_or(0);
    numbers
        .into_iter()
        .map(|number| {
            usize::try_from(number)
                .ok()
                .and_then(|number| number.checked_sub(1))
                .and_then(|index| starts.get(index).copied())
                .unwrap_or(end)
        })
        .collect()
}

/// The complete lines of `bytes` without their `\n`. A last line without
/// one is still being written and is left out.
pub(crate) fn complete_lines(bytes: &[u8]) -> Vec<&[u8]> {
    bytes
        .split_inclusive(|byte| *byte == b'\n')
        .filter_map(|piece| piece.strip_suffix(b"\n"))
        .collect()
}

/// The bytes of a file a fork or a read takes: a plain file, not a link.
pub(crate) fn read_plain(path: &Path, max: u64) -> Result<Vec<u8>, ConversationError> {
    let meta = fs::symlink_metadata(path).map_err(|source| io_error("read a conversation file", path, source))?;
    if !meta.file_type().is_file() {
        return Err(ConversationError::NotAFile {
            path: path.to_path_buf(),
        });
    }
    if meta.len() > max {
        return Err(ConversationError::TooBig {
            path: path.to_path_buf(),
            size: meta.len(),
            max,
        });
    }
    fs::read(path).map_err(|source| io_error("read a conversation file", path, source))
}

/// A file a carry copies or places: a plain file within the cap, and text.
pub(crate) fn read_text(path: &Path, max: u64) -> Result<String, ConversationError> {
    let bytes = read_plain(path, max)?;
    match String::from_utf8(bytes) {
        Ok(text) if !text.contains('\0') => Ok(text),
        _ => Err(ConversationError::NotText {
            path: path.to_path_buf(),
        }),
    }
}

pub(crate) fn is_there(path: &Path) -> Result<bool, ConversationError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(io_error("check for a conversation file", path, source)),
    }
}

/// Whether `dir` exists and has anything in it.
pub(crate) fn holds_files(dir: &Path) -> Result<bool, ConversationError> {
    let mut entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(source) => return Err(io_error("list the folder", dir, source)),
    };
    match entries.next() {
        None => Ok(false),
        Some(Ok(_)) => Ok(true),
        Some(Err(source)) => Err(io_error("list the folder", dir, source)),
    }
}

/// Every non-folder entry under `dir`, recursively. Links are listed, not
/// followed.
pub(crate) fn list_into(dir: &Path, found: &mut Vec<PathBuf>) -> Result<(), ConversationError> {
    let entries = fs::read_dir(dir).map_err(|source| io_error("list the folder", dir, source))?;
    for entry in entries {
        let path = entry
            .map_err(|source| io_error("list the folder", dir, source))?
            .path();
        let meta = fs::symlink_metadata(&path).map_err(|source| io_error("list the folder", &path, source))?;
        if meta.file_type().is_dir() {
            list_into(&path, found)?;
        } else {
            found.push(path);
        }
    }
    Ok(())
}

/// Writes a new file through a `.partial` beside it, renamed into place.
/// Refuses a file that is already there.
pub(crate) fn write_new(path: &Path, bytes: &[u8]) -> Result<(), ConversationError> {
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder).map_err(|source| io_error("create the folder", folder, source))?;
    }
    let partial = partial_name(path);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial)
        .map_err(|source| match source.kind() {
            io::ErrorKind::AlreadyExists => ConversationError::Exists {
                path: partial.clone(),
            },
            _ => io_error("write", &partial, source),
        })?;
    file.write_all(bytes)
        .map_err(|source| io_error("write", &partial, source))?;
    drop(file);
    fs::rename(&partial, path).map_err(|source| io_error("move into place", path, source))
}

fn partial_name(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".partial");
    PathBuf::from(name)
}

/// `rel` (with `/` between parts) under `root`.
pub(crate) fn under(root: &Path, rel: &str) -> PathBuf {
    rel.split('/').fold(root.to_path_buf(), |mut path, part| {
        path.push(part);
        path
    })
}

/// A staged name that stays inside its folder: non-empty, and only plain
/// parts (no `/` at the start, no `..`).
pub(crate) fn valid_rel(rel: &str) -> bool {
    !rel.is_empty() && Path::new(rel).components().all(|part| matches!(part, Component::Normal(_)))
}

/// `path` relative to `base`, with `/` between its parts.
pub(crate) fn relative_name(path: &Path, base: &Path) -> Result<String, ConversationError> {
    let bad = || ConversationError::BadPath {
        rel: path.display().to_string(),
    };
    let rest = path.strip_prefix(base).map_err(|_| bad())?;
    let mut parts = Vec::new();
    for component in rest.components() {
        let Component::Normal(part) = component else {
            return Err(bad());
        };
        parts.push(part.to_str().ok_or_else(bad)?);
    }
    Ok(parts.join("/"))
}

/// A path as text, lossily.
pub(crate) fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// `text` as it reads between the quotes of a JSON string.
pub(crate) fn json_escape(text: &str) -> String {
    match serde_json::to_string(text) {
        Ok(quoted) => quoted
            .strip_prefix('"')
            .and_then(|rest| rest.strip_suffix('"'))
            .unwrap_or(text)
            .to_owned(),
        Err(_) => text.to_owned(),
    }
}

/// `text` with every old text of `pairs` replaced by its new one, in order.
pub(crate) fn rewrite_text(text: &str, pairs: &[(String, String)]) -> String {
    pairs
        .iter()
        .fold(text.to_owned(), |acc, (old, new)| acc.replace(old.as_str(), new.as_str()))
}

/// The same rewrite as [`rewrite_text`], on every string of a record.
pub(crate) fn rewrite_value(value: &mut Value, pairs: &[(String, String)]) {
    match value {
        Value::String(text) => {
            for (old, new) in pairs {
                *text = text.replace(old.as_str(), new.as_str());
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|item| rewrite_value(item, pairs)),
        Value::Object(fields) => fields.values_mut().for_each(|field| rewrite_value(field, pairs)),
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

/// Whether a value holds a path: a string, at any depth, that starts with
/// `/` or `~` or holds `folder` (a dashed folder name).
pub(crate) fn holds_path(value: &Value, folder: &str) -> bool {
    match value {
        Value::String(text) => text.starts_with('/') || text.starts_with('~') || text.contains(folder),
        Value::Array(items) => items.iter().any(|item| holds_path(item, folder)),
        Value::Object(fields) => fields.values().any(|field| holds_path(field, folder)),
        Value::Null | Value::Bool(_) | Value::Number(_) => false,
    }
}
