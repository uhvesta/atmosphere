//! The raw lines a session on another machine keeps (D195): the scrubbed
//! lines the host read from the machine's conversation file, as they came,
//! so a session whose machine is gone can resume from them.
//!
//! They sit beside the entries in `raw-<generation>.jsonl`. The session's
//! state counts the bytes of that file that are committed, in the same
//! write as the entries, so a crash never leaves raw bytes the state doesn't
//! count or the reverse. A new generation starts when the machine's file is
//! a new one; the old file stays until the next trim.

use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::MirrorError;

/// The raw lines of one session and where they stand.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct Raw {
    /// Whether the session keeps raw lines: set by its first take or by a
    /// backfill.
    pub kept: bool,
    /// Which `raw-<generation>.jsonl` holds the lines.
    pub generation: u64,
    /// Bytes of that file that count.
    pub committed_len: u64,
    /// While a backfill runs: where it reads the machine's file again from.
    pub backfill: Option<Cursor>,
}

/// A place in a machine's conversation file (D189): an offset, and what the
/// machine said the file was there.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct Cursor {
    pub offset: u64,
    pub source: String,
}

fn io(path: &Path) -> impl Fn(std::io::Error) -> MirrorError + '_ {
    |source| MirrorError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn corrupt(path: &Path, reason: impl std::fmt::Display) -> MirrorError {
    MirrorError::Corrupt {
        path: path.to_path_buf(),
        reason: reason.to_string(),
    }
}

fn file_of(dir: &Path, generation: u64) -> PathBuf {
    dir.join(format!("raw-{generation}.jsonl"))
}

/// The generation a folder entry's name holds, if it is a raw file's.
fn generation_of(name: &str) -> Option<u64> {
    name.strip_prefix("raw-")?
        .strip_suffix(".jsonl")?
        .parse()
        .ok()
}

/// Adds `bytes` after the committed bytes of `raw`'s file and returns how
/// many were written. The bytes count only once the state that covers them
/// is saved.
pub(crate) fn append(dir: &Path, raw: &Raw, bytes: &[u8]) -> Result<u64, MirrorError> {
    if bytes.is_empty() {
        return Ok(0);
    }
    fs::create_dir_all(dir).map_err(io(dir))?;
    let path = file_of(dir, raw.generation);
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(io(&path))?;
    file.set_len(raw.committed_len).map_err(io(&path))?;
    file.seek(SeekFrom::Start(raw.committed_len))
        .map_err(io(&path))?;
    file.write_all(bytes).map_err(io(&path))?;
    file.sync_data().map_err(io(&path))?;
    Ok(bytes.len() as u64)
}

/// Writes `bytes` as the whole of a new generation's file, synced, and
/// returns its length. Nothing counts until the state names the generation.
pub(crate) fn start(dir: &Path, generation: u64, bytes: &[u8]) -> Result<u64, MirrorError> {
    fs::create_dir_all(dir).map_err(io(dir))?;
    let path = file_of(dir, generation);
    let mut file = File::create(&path).map_err(io(&path))?;
    file.write_all(bytes).map_err(io(&path))?;
    file.sync_data().map_err(io(&path))?;
    Ok(bytes.len() as u64)
}

/// The committed bytes of `raw`'s file.
pub(crate) fn read(dir: &Path, raw: &Raw) -> Result<Vec<u8>, MirrorError> {
    let path = file_of(dir, raw.generation);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(io(&path)(error)),
    };
    let committed = usize::try_from(raw.committed_len).unwrap_or(usize::MAX);
    match bytes.get(..committed) {
        Some(kept) => Ok(kept.to_vec()),
        None => Err(corrupt(
            &path,
            format!(
                "it holds {} bytes but {} were saved",
                bytes.len(),
                raw.committed_len
            ),
        )),
    }
}

/// Drops what a crash left: the bytes of `raw`'s file past its committed
/// length, and the files of every other generation.
pub(crate) fn trim(dir: &Path, raw: &Raw) -> Result<(), MirrorError> {
    let listed = match fs::read_dir(dir) {
        Ok(listed) => listed,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(io(dir)(error)),
    };
    for item in listed {
        let item = item.map_err(io(dir))?;
        let stale = item
            .file_name()
            .to_str()
            .and_then(generation_of)
            .is_some_and(|generation| generation != raw.generation);
        if stale {
            let path = item.path();
            fs::remove_file(&path).map_err(io(&path))?;
        }
    }
    let path = file_of(dir, raw.generation);
    let len = match fs::metadata(&path) {
        Ok(meta) => meta.len(),
        Err(error) if error.kind() == ErrorKind::NotFound => 0,
        Err(error) => return Err(io(&path)(error)),
    };
    if len < raw.committed_len {
        return Err(corrupt(
            &path,
            format!(
                "it holds {len} bytes but {} were saved",
                raw.committed_len
            ),
        ));
    }
    if len > raw.committed_len {
        let file = OpenOptions::new()
            .write(true)
            .open(&path)
            .map_err(io(&path))?;
        file.set_len(raw.committed_len).map_err(io(&path))?;
        file.sync_all().map_err(io(&path))?;
    }
    Ok(())
}
