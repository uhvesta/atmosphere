//! Generic Linux FUSE adapter and degraded fallback for FUSEFS.
//!
//! This crate is intentionally independent of any application-specific schema:
//! - [`probe`]: runtime `/dev/fuse` availability detection and mount mode selection.
//! - [`pid_bind`]: kernel PID/PGID/SID lineage binding to [`fusefs_vfs::CallerContext`]
//!   with fail-closed enforcement for unknown processes.
//! - [`fuse_abi`]: portable FUSE 7.x ABI opcodes, wire headers, and typed request/reply messages.
//! - [`dispatcher`]: generic [`FuseDispatcher`] translating FUSE requests into
//!   [`fusefs_vfs::Volume`] calls with kernel PID caller binding.
//! - [`degraded`]: explicit degraded native-symlink skeleton writer with `.degraded.json`
//!   markers, atomic refresh, and safe cleanup when `/dev/fuse` is unavailable.

pub mod degraded;
pub mod dispatcher;
pub mod fuse_abi;
pub mod pid_bind;
pub mod probe;

pub use degraded::{
    DEGRADED_MARKER_FILENAME, DegradedMaterialization, DegradedNotice, DegradedSkeletonError,
    DegradedSkeletonWriter, SkeletonEntry,
};
pub use dispatcher::FuseDispatcher;
#[cfg(target_os = "linux")]
pub use dispatcher::DevFuseSession;
pub use fuse_abi::{
    FuseErrno, FuseHeader, FuseInHeader, FuseOpcode, FuseOutHeader, FuseReply, FuseRequest,
    FuseResponse,
};
pub use pid_bind::{
    LineageResolver, PidBinding, PidLineage, PidRegistry, ProcfsLineageResolver,
    StaticLineageResolver,
};
pub use probe::{
    DEFAULT_FUSE_DEVICE_PATH, FuseAvailability, MountMode, MountProbeResult, probe_dev_fuse,
    probe_fuse_device_path, probe_mount_mode, probe_mount_mode_at, select_mount_mode,
};
