//! Typed VFS errors mapped cleanly to POSIX `errno` codes.

use serde::{Deserialize, Serialize};
use thiserror::Error;

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

/// Typed VFS error enum with 1-to-1 POSIX `errno` mapping.
#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize, Deserialize)]
pub enum VfsError {
    #[error("operation not permitted: {0}")]
    Eperm(String),

    #[error("no such file or directory: {0}")]
    Enoent(String),

    #[error("input/output error: {0}")]
    Eio(String),

    #[error("bad file descriptor: {0}")]
    Ebadf(String),

    #[error("permission denied: {0}")]
    Eacces(String),

    #[error("file exists: {0}")]
    Eexist(String),

    #[error("cross-device link: {0}")]
    CrossDevice(String),

    #[error("not a directory: {0}")]
    Enotdir(String),

    #[error("is a directory: {0}")]
    Eisdir(String),

    #[error("invalid argument: {0}")]
    Einval(String),

    #[error("no space left on device: {0}")]
    Enospc(String),

    #[error("read-only file system: {0}")]
    Erofs(String),

    #[error("result too large: {0}")]
    Erange(String),

    #[error("resource temporarily unavailable: {0}")]
    Ewouldblock(String),

    #[error("directory not empty: {0}")]
    Enotempty(String),

    #[error("function not implemented: {0}")]
    Enosys(String),

    #[error("no data available: {0}")]
    Enodata(String),

    #[error("network is unreachable: {0}")]
    Enetunreach(String),

    #[error("stale file handle: {0}")]
    Estale(String),
}

impl VfsError {
    /// Returns the POSIX `errno` integer corresponding to this error.
    pub fn errno(&self) -> i32 {
        match self {
            Self::Eperm(_) => errno::EPERM,
            Self::Enoent(_) => errno::ENOENT,
            Self::Eio(_) => errno::EIO,
            Self::Ebadf(_) => errno::EBADF,
            Self::Eacces(_) => errno::EACCES,
            Self::Eexist(_) => errno::EEXIST,
            Self::CrossDevice(_) => errno::EXDEV,
            Self::Enotdir(_) => errno::ENOTDIR,
            Self::Eisdir(_) => errno::EISDIR,
            Self::Einval(_) => errno::EINVAL,
            Self::Enospc(_) => errno::ENOSPC,
            Self::Erofs(_) => errno::EROFS,
            Self::Erange(_) => errno::ERANGE,
            Self::Ewouldblock(_) => errno::EWOULDBLOCK,
            Self::Enotempty(_) => errno::ENOTEMPTY,
            Self::Enosys(_) => errno::ENOSYS,
            Self::Enodata(_) => errno::ENODATA,
            Self::Enetunreach(_) => errno::ENETUNREACH,
            Self::Estale(_) => errno::ESTALE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errno_mapping_covers_core_posix_errors() {
        assert_eq!(VfsError::Eperm("root".into()).errno(), errno::EPERM);
        assert_eq!(VfsError::Enoent("missing".into()).errno(), errno::ENOENT);
        assert_eq!(VfsError::Eacces("denied".into()).errno(), errno::EACCES);
        assert_eq!(VfsError::Eexist("dup".into()).errno(), errno::EEXIST);
        assert_eq!(VfsError::Enotdir("file".into()).errno(), errno::ENOTDIR);
        assert_eq!(VfsError::Eisdir("dir".into()).errno(), errno::EISDIR);
        assert_eq!(VfsError::Enotempty("dir".into()).errno(), errno::ENOTEMPTY);
        assert_eq!(VfsError::Einval("bad".into()).errno(), errno::EINVAL);
        assert_eq!(VfsError::Estale("gen".into()).errno(), errno::ESTALE);
        assert_eq!(
            VfsError::Enetunreach("offline".into()).errno(),
            errno::ENETUNREACH
        );
        assert_eq!(VfsError::Eio("io".into()).errno(), errno::EIO);
        assert_eq!(VfsError::Enosys("todo".into()).errno(), errno::ENOSYS);
        assert_eq!(VfsError::CrossDevice("xdev".into()).errno(), errno::EXDEV);
    }
}
