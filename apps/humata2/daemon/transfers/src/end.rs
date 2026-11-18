//! Where files are read and written. The engine touches files only
//! through [`End`], so a remote machine can be added later.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::EndError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    File,
    Folder,
    Link,
}

/// One file, folder or link in a source tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// Empty for the source itself, else the `/`-joined path under it.
    pub rel: String,
    pub kind: Kind,
    pub size: u64,
    pub mode: u32,
    pub modified_unix_ms: i64,
    pub link_target: Option<PathBuf>,
}

/// Where bytes of one file are written.
pub trait Sink: Send {
    fn write(&mut self, bytes: &[u8]) -> Result<(), EndError>;
    /// Puts what was written on disk.
    fn sync(&mut self) -> Result<(), EndError>;
}

pub trait End: Send + Sync {
    fn label(&self) -> &str;
    /// Whether a move of `from` into `to_folder` on `other` may be one rename.
    fn same_disk(&self, other: &dyn End, from: &Path, to_folder: &Path) -> bool;

    /// The tree under `source`, folders before what they hold. A socket,
    /// pipe or device is [`EndError::Unsupported`].
    fn plan(&self, source: &Path) -> Result<Vec<Entry>, EndError>;
    fn open_read(&self, path: &Path, offset: u64) -> Result<Box<dyn std::io::Read + Send>, EndError>;

    fn free_bytes(&self, folder: &Path) -> Result<u64, EndError>;
    /// A dangling link exists.
    fn exists(&self, path: &Path) -> Result<bool, EndError>;
    /// Which of `sha256` are held as chunk files, `<folder>/<sha256>`, here
    /// (MV-17). A name that isn't 64 lowercase hex digits, and a folder that
    /// isn't there, hold nothing. A remote end answers in one call.
    fn have_chunks(&self, folder: &Path, sha256: &[String]) -> Result<Vec<String>, EndError> {
        let mut held = Vec::new();
        for name in sha256 {
            let hex = name.len() == 64 && name.bytes().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'));
            if hex && self.exists(&folder.join(name))? {
                held.push(name.clone());
            }
        }
        Ok(held)
    }
    fn make_folder(&self, path: &Path, mode: u32) -> Result<(), EndError>;
    fn make_link(&self, path: &Path, target: &Path) -> Result<(), EndError>;
    /// Creates the file if missing, cuts it to `offset` and appends there.
    fn open_write(&self, path: &Path, offset: u64) -> Result<Box<dyn Sink>, EndError>;
    /// SHA-256 of the first `len` bytes, computed where the file is.
    fn hash(&self, path: &Path, len: u64) -> Result<[u8; 32], EndError>;
    /// Sets the mode and modified time of a file that is complete, or of a
    /// folder once everything in it is written.
    fn finish_file(&self, path: &Path, mode: u32, modified_unix_ms: i64) -> Result<(), EndError>;
    /// Never replaces: fails if `to` exists.
    fn rename(&self, from: &Path, to: &Path) -> Result<(), EndError>;
    fn sync_folder(&self, folder: &Path) -> Result<(), EndError>;
    /// A missing path is fine.
    fn remove_tree(&self, path: &Path) -> Result<(), EndError>;
    fn trash(&self, path: &Path, description: &str, now_unix_ms: i64) -> Result<(), EndError>;
}
