//! Typed VFS errors mapped cleanly to POSIX `errno` codes.

use std::fmt;

use thiserror::Error;

use crate::types::{FileHandle, NodeId};

/// Convenience result alias for VFS operations.
pub type VfsResult<T> = Result<T, VfsError>;

/// POSIX `errno` constants (platform-aware where macOS and Linux diverge).
pub mod errno {
    pub const EPERM: i32 = 1;
    pub const ENOENT: i32 = 2;
    pub const EIO: i32 = 5;
    pub const EBADF: i32 = 9;
    pub const EACCES: i32 = 13;
    pub const EEXIST: i32 = 17;
    pub const EXDEV: i32 = 18;
    pub const ENOTDIR: i32 = 20;
    pub const EISDIR: i32 = 21;
    pub const EINVAL: i32 = 22;
    pub const ENOSPC: i32 = 28;
    pub const EROFS: i32 = 30;
    pub const ERANGE: i32 = 34;

    #[cfg(target_os = "macos")]
    pub const EWOULDBLOCK: i32 = 35;
    #[cfg(not(target_os = "macos"))]
    pub const EWOULDBLOCK: i32 = 11;

    #[cfg(target_os = "macos")]
    pub const ENOTEMPTY: i32 = 66;
    #[cfg(not(target_os = "macos"))]
    pub const ENOTEMPTY: i32 = 39;

    #[cfg(target_os = "macos")]
    pub const ENOSYS: i32 = 78;
    #[cfg(not(target_os = "macos"))]
    pub const ENOSYS: i32 = 38;

    #[cfg(target_os = "macos")]
    pub const ENODATA: i32 = 93; // ENOATTR on macOS
    #[cfg(not(target_os = "macos"))]
    pub const ENODATA: i32 = 61;

    #[cfg(target_os = "macos")]
    pub const ENETUNREACH: i32 = 51;
    #[cfg(not(target_os = "macos"))]
    pub const ENETUNREACH: i32 = 101;

    #[cfg(target_os = "macos")]
    pub const ESTALE: i32 = 70;
    #[cfg(not(target_os = "macos"))]
    pub const ESTALE: i32 = 116;
}

/// What a failed VFS call was acting on, recorded as a typed value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VfsTarget {
    /// A node, by inode number.
    Node(NodeId),
    /// A file or folder, by its virtual path.
    Path(String),
    /// An open file or directory handle.
    Handle(FileHandle),
    /// A flush that made no progress on a node.
    Flush { ino: NodeId, bytes_flushed: u64 },
    /// A name supplied by the caller, such as a child name in a lookup.
    Name(String),
    /// An entry name inside a directory, by the directory's virtual path.
    Entry { dir: String, name: String },
    /// A capability root, by its `cap_<id>` identifier.
    Capability(String),
    /// An extended attribute on a path.
    Xattr { path: String, name: String },
    /// A kernel process, by PID.
    Pid(u32),
    /// The capability root a session was granted read-only.
    ReadOnlyRoot,
    /// A path as seen by one caller process.
    Caller { pid: u32, path: String },
    /// A path whose access was denied and recorded as a pending request.
    Request { id: String, path: String },
    /// An optimistic concurrency generation that did not match.
    Generation { expected: u64, actual: u64 },
    /// A record of a fixed kind, such as a JSON stub or status file.
    Record(&'static str),
}

impl fmt::Display for VfsTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Node(ino) => write!(f, "node {ino}"),
            Self::Path(path) => write!(f, "path '{path}'"),
            Self::Handle(fh) => write!(f, "file handle {fh}"),
            Self::Flush { ino, bytes_flushed } => write!(
                f,
                "node {ino} after flushing {bytes_flushed} bytes"
            ),
            Self::Name(name) => write!(f, "name '{name}'"),
            Self::Entry { dir, name } => write!(f, "'{name}' in '{dir}'"),
            Self::Capability(id) => write!(f, "capability root '{id}'"),
            Self::Xattr { path, name } => {
                write!(f, "extended attribute '{name}' on path '{path}'")
            }
            Self::Pid(pid) => write!(f, "process {pid}"),
            Self::ReadOnlyRoot => write!(f, "the read-only capability root"),
            Self::Caller { pid, path } => write!(f, "path '{path}' for process {pid}"),
            Self::Request { id, path } => write!(f, "path '{path}' (request '{id}')"),
            Self::Generation { expected, actual } => {
                write!(f, "generation {actual} (expected {expected})")
            }
            Self::Record(kind) => write!(f, "the {kind} record"),
        }
    }
}

/// Typed VFS error enum with 1-to-1 POSIX `errno` mapping.
///
/// Every variant records the operation that failed (`op`) and the [`VfsTarget`] it acted on.
/// The `Json*` variants also keep the underlying `serde_json` error as their source.
#[derive(Debug, Error)]
pub enum VfsError {
    #[error(
        "{op} is not permitted on {target}. This entry is managed by Humata; change it through the Humata app or CLI instead."
    )]
    Eperm { op: &'static str, target: VfsTarget },

    #[error("{op} could not find {target}. Check the path, or list its parent folder to see what exists.")]
    Enoent { op: &'static str, target: VfsTarget },

    #[error("{op} hit an I/O error on {target}. Retry; if it keeps failing, check the Humata daemon log for the cause.")]
    Eio { op: &'static str, target: VfsTarget },

    #[error("{op} used a stale or unknown file handle for {target}. Close the file and open it again.")]
    Ebadf { op: &'static str, target: VfsTarget },

    #[error("{op} was denied for {target}. Check that this path is granted to the current session; ask the owner to grant it if not.")]
    Eacces { op: &'static str, target: VfsTarget },

    #[error("{op} cannot create {target} because it already exists. Choose another name, or open the existing entry.")]
    Eexist { op: &'static str, target: VfsTarget },

    #[error("{op} cannot move {target} across volumes. Copy the data to the destination and remove the original.")]
    CrossDevice { op: &'static str, target: VfsTarget },

    #[error("{op} needs a folder but {target} is not one. Use a path to a folder.")]
    Enotdir { op: &'static str, target: VfsTarget },

    #[error("{op} needs a file but {target} is a folder. Use a path to a file.")]
    Eisdir { op: &'static str, target: VfsTarget },

    #[error("{op} got an invalid argument for {target}. Fix the value and try again.")]
    Einval { op: &'static str, target: VfsTarget },

    #[error("{op} failed on {target}: the volume is full. Free space or remove unused files, then retry.")]
    Enospc { op: &'static str, target: VfsTarget },

    #[error("{op} cannot write to {target}: this location is read-only. Write under a writable session folder instead.")]
    Erofs { op: &'static str, target: VfsTarget },

    #[error("{op} produced a result too large for {target}. Request a smaller range.")]
    Erange { op: &'static str, target: VfsTarget },

    #[error("{op} conflicts with a lock held on {target}. Wait for the other holder to release it, then retry.")]
    Ewouldblock { op: &'static str, target: VfsTarget },

    #[error("{op} cannot remove {target} because it is not empty. Remove its contents first.")]
    Enotempty { op: &'static str, target: VfsTarget },

    #[error("{op} is not supported for {target} yet. Use an operation this entry supports.")]
    Enosys { op: &'static str, target: VfsTarget },

    #[error("{op} found no data for {target}: the attribute is not set. Check the name, or set it first.")]
    Enodata { op: &'static str, target: VfsTarget },

    #[error("{op} could not reach the machine that holds {target}. Check that the machine is online, then retry.")]
    Enetunreach { op: &'static str, target: VfsTarget },

    #[error("{op} used a file handle or lease that is out of date for {target}. Reopen the path to get a fresh one.")]
    Estale { op: &'static str, target: VfsTarget },

    #[error("{op} failed to read or write {target} as JSON. Retry; if it keeps failing, check the Humata daemon log for the cause.")]
    JsonIo {
        op: &'static str,
        target: VfsTarget,
        #[source]
        source: serde_json::Error,
    },

    #[error("{op} got malformed JSON for {target}. Check that the JSON is valid, or regenerate it, then retry.")]
    JsonInvalid {
        op: &'static str,
        target: VfsTarget,
        #[source]
        source: serde_json::Error,
    },
}

impl VfsError {
    /// Returns the POSIX `errno` integer corresponding to this error.
    pub fn errno(&self) -> i32 {
        match self {
            Self::Eperm { .. } => errno::EPERM,
            Self::Enoent { .. } => errno::ENOENT,
            Self::Eio { .. } => errno::EIO,
            Self::Ebadf { .. } => errno::EBADF,
            Self::Eacces { .. } => errno::EACCES,
            Self::Eexist { .. } => errno::EEXIST,
            Self::CrossDevice { .. } => errno::EXDEV,
            Self::Enotdir { .. } => errno::ENOTDIR,
            Self::Eisdir { .. } => errno::EISDIR,
            Self::Einval { .. } => errno::EINVAL,
            Self::Enospc { .. } => errno::ENOSPC,
            Self::Erofs { .. } => errno::EROFS,
            Self::Erange { .. } => errno::ERANGE,
            Self::Ewouldblock { .. } => errno::EWOULDBLOCK,
            Self::Enotempty { .. } => errno::ENOTEMPTY,
            Self::Enosys { .. } => errno::ENOSYS,
            Self::Enodata { .. } => errno::ENODATA,
            Self::Enetunreach { .. } => errno::ENETUNREACH,
            Self::Estale { .. } => errno::ESTALE,
            Self::JsonIo { .. } => errno::EIO,
            Self::JsonInvalid { .. } => errno::EINVAL,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errno_mapping_covers_core_posix_errors() {
        let target = || VfsTarget::Path("/root".into());
        let op = "test";
        assert_eq!(
            VfsError::Eperm { op, target: target() }.errno(),
            errno::EPERM
        );
        assert_eq!(
            VfsError::Enoent { op, target: target() }.errno(),
            errno::ENOENT
        );
        assert_eq!(
            VfsError::Eacces { op, target: target() }.errno(),
            errno::EACCES
        );
        assert_eq!(
            VfsError::Eexist { op, target: target() }.errno(),
            errno::EEXIST
        );
        assert_eq!(
            VfsError::Enotdir { op, target: target() }.errno(),
            errno::ENOTDIR
        );
        assert_eq!(
            VfsError::Eisdir { op, target: target() }.errno(),
            errno::EISDIR
        );
        assert_eq!(
            VfsError::Enotempty { op, target: target() }.errno(),
            errno::ENOTEMPTY
        );
        assert_eq!(
            VfsError::Einval { op, target: target() }.errno(),
            errno::EINVAL
        );
        assert_eq!(
            VfsError::Estale { op, target: target() }.errno(),
            errno::ESTALE
        );
        assert_eq!(
            VfsError::Enetunreach { op, target: target() }.errno(),
            errno::ENETUNREACH
        );
        assert_eq!(VfsError::Eio { op, target: target() }.errno(), errno::EIO);
        assert_eq!(
            VfsError::Enosys { op, target: target() }.errno(),
            errno::ENOSYS
        );
        assert_eq!(
            VfsError::CrossDevice { op, target: target() }.errno(),
            errno::EXDEV
        );
    }
}
