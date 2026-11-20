//! Core VFS types: node IDs, attributes, handles, caller context, and synthetic stubs.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::{VfsError, VfsResult, VfsTarget};
use crate::path::VirtualPath;

/// Reserved inode number for the volume root directory (`"/"`).
pub const ROOT_NODE_ID: NodeId = NodeId(1);

/// Conventional filename for synthetic omission / build-cache markers.
pub const OMITTED_STUB_FILENAME: &str = ".omitted.json";

/// Unique identifier for a VFS node (inode).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct NodeId(pub u64);

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ino:{}", self.0)
    }
}

/// Unique identifier for an open file handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct FileHandle(pub u64);

impl fmt::Display for FileHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "fh:{}", self.0)
    }
}

/// Kind of filesystem entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FileKind {
    Regular,
    Directory,
    Symlink,
}

/// POSIX-compatible timestamp (seconds + nanoseconds since UNIX epoch).
#[derive(
    Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct Timestamp {
    pub secs: i64,
    pub nanos: u32,
}

impl Timestamp {
    pub const ZERO: Self = Self { secs: 0, nanos: 0 };

    pub const fn from_secs(secs: i64) -> Self {
        Self { secs, nanos: 0 }
    }

    pub const fn from_millis(ms: u64) -> Self {
        Self {
            secs: (ms / 1000) as i64,
            nanos: ((ms % 1000) * 1_000_000) as u32,
        }
    }

    pub const fn as_millis(&self) -> u64 {
        if self.secs < 0 {
            0
        } else {
            (self.secs as u64)
                .saturating_mul(1000)
                .saturating_add((self.nanos / 1_000_000) as u64)
        }
    }
}

/// Metadata attributes for a VFS node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileAttr {
    pub ino: NodeId,
    pub kind: FileKind,
    pub size: u64,
    pub blocks: u64,
    pub atime: Timestamp,
    pub mtime: Timestamp,
    pub ctime: Timestamp,
    pub mode: u32,
    pub nlink: u32,
    pub uid: u32,
    pub gid: u32,
    pub generation: u64,
}

impl FileAttr {
    /// Computes 512-byte POSIX block count for `size`.
    pub const fn blocks_for_size(size: u64) -> u64 {
        size.div_ceil(512)
    }
}

/// A single directory entry returned by `readdir`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirEntry {
    pub ino: NodeId,
    pub kind: FileKind,
    pub name: String,
    pub offset: u64,
}

/// Filesystem-wide statistics returned by `statfs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatFs {
    pub blocks: u64,
    pub bfree: u64,
    pub bavail: u64,
    pub files: u64,
    pub ffree: u64,
    pub bsize: u32,
    pub namelen: u32,
    pub frsize: u32,
}

impl Default for StatFs {
    fn default() -> Self {
        Self {
            blocks: 1_048_576,
            bfree: 1_048_576,
            bavail: 1_048_576,
            files: 1_048_576,
            ffree: 1_048_576,
            bsize: 4096,
            namelen: 255,
            frsize: 4096,
        }
    }
}

/// Partial attribute update request for `setattr`.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetAttr {
    pub mode: Option<u32>,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
    pub size: Option<u64>,
    pub atime: Option<Timestamp>,
    pub mtime: Option<Timestamp>,
    /// Optional optimistic concurrency check against the node's current `generation`.
    pub expected_generation: Option<u64>,
}

/// Flags passed to `open`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OpenFlags {
    pub read: bool,
    pub write: bool,
    pub append: bool,
    pub truncate: bool,
    pub create: bool,
    pub exclusive: bool,
}

impl OpenFlags {
    pub const fn read_only() -> Self {
        Self {
            read: true,
            write: false,
            append: false,
            truncate: false,
            create: false,
            exclusive: false,
        }
    }

    pub const fn write_only() -> Self {
        Self {
            read: false,
            write: true,
            append: false,
            truncate: false,
            create: false,
            exclusive: false,
        }
    }

    pub const fn read_write() -> Self {
        Self {
            read: true,
            write: true,
            append: false,
            truncate: false,
            create: false,
            exclusive: false,
        }
    }

    pub const fn create_truncate() -> Self {
        Self {
            read: true,
            write: true,
            append: false,
            truncate: true,
            create: true,
            exclusive: false,
        }
    }

    /// Returns `true` if opening with these flags requests mutation capability.
    pub const fn requires_write(&self) -> bool {
        self.write || self.append || self.truncate || self.create
    }
}

/// Advisory file lock operation for `flock`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FlockOp {
    Shared,
    Exclusive,
    Unlock,
}

/// Identity and capability scope of the calling process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallerContext {
    pub pid: u32,
    pub uid: u32,
    pub gid: u32,
    pub capability_root: Option<String>,
}

impl CallerContext {
    pub fn new(pid: u32, uid: u32, gid: u32) -> Self {
        Self {
            pid,
            uid,
            gid,
            capability_root: None,
        }
    }

    pub fn with_capability_root(mut self, root: impl Into<String>) -> Self {
        self.capability_root = Some(root.into());
        self
    }

    /// Parses `capability_root` (if present) into a normalized [`VirtualPath`].
    pub fn capability_root_path(&self) -> VfsResult<Option<VirtualPath>> {
        match &self.capability_root {
            Some(raw) => Ok(Some(VirtualPath::parse(raw)?)),
            None => Ok(None),
        }
    }
}

/// Generic synthetic omission marker serialized into `.omitted.json` files
/// (e.g., for omitted build caches or policy-excluded directories).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyntheticStub {
    pub schema_version: u32,
    pub omitted_path: String,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_entries: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl SyntheticStub {
    pub fn new(omitted_path: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            schema_version: 1,
            omitted_path: omitted_path.into(),
            reason: reason.into(),
            original_bytes: None,
            original_entries: None,
            hint: None,
        }
    }

    pub fn with_original_bytes(mut self, bytes: u64) -> Self {
        self.original_bytes = Some(bytes);
        self
    }

    pub fn with_original_entries(mut self, entries: u64) -> Self {
        self.original_entries = Some(entries);
        self
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// Serializes the stub to deterministic pretty-printed JSON bytes with a trailing newline.
    pub fn to_json_bytes(&self) -> VfsResult<Vec<u8>> {
        let mut out = serde_json::to_vec_pretty(self).map_err(|source| VfsError::JsonIo {
            op: "serialize stub",
            target: VfsTarget::Record("synthetic stub"),
            source,
        })?;
        out.push(b'\n');
        Ok(out)
    }

    /// Deserializes a stub from JSON bytes.
    pub fn from_json_bytes(bytes: &[u8]) -> VfsResult<Self> {
        serde_json::from_slice(bytes).map_err(|source| VfsError::JsonInvalid {
            op: "parse stub",
            target: VfsTarget::Record("synthetic stub"),
            source,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_stub_round_trip() {
        let stub = SyntheticStub::new("/repo/target", "build_cache")
            .with_original_bytes(2_147_483_648)
            .with_original_entries(14_200)
            .with_hint("Run build locally to materialize artifacts");

        let bytes = stub.to_json_bytes().unwrap();
        assert!(bytes.ends_with(b"\n"));
        let decoded = SyntheticStub::from_json_bytes(&bytes).unwrap();
        assert_eq!(decoded, stub);
    }
}
