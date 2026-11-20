//! Generic virtual filesystem (VFS) primitives for FUSEFS.
//!
//! This crate is intentionally independent of any application-specific schema:
//! - [`error`]: typed [`VfsError`] mapped to POSIX `errno` values.
//! - [`path`]: normalized [`VirtualPath`] with strict `..` capability-root clamping.
//! - [`types`]: core VFS identifiers, attributes, handles, caller context, and [`SyntheticStub`].
//! - [`lease`]: subtree [`WriteLease`] table and optimistic generation concurrency checks.
//! - [`volume`]: [`Volume`] and [`AccessHook`] traits separating zero-data metadata/control
//!   operations from chunk-data I/O, plus [`InMemoryVolume`] and [`TreeBuilder`].

pub mod error;
pub mod lease;
pub mod path;
pub mod types;
pub mod volume;

pub use error::{VfsError, VfsResult, VfsTarget};
pub use lease::{LeaseId, LeaseTable, WriteLease, check_generation};
pub use path::VirtualPath;
pub use types::{
    CallerContext, DirEntry, FileAttr, FileHandle, FileKind, FlockOp, NodeId,
    OMITTED_STUB_FILENAME, OpenFlags, ROOT_NODE_ID, SetAttr, StatFs, SyntheticStub, Timestamp,
};
pub use volume::{
    AccessHook, ChunkLoader, InMemoryVolume, MetadataOp, NoopAccessHook, TreeBuilder, Volume,
};
