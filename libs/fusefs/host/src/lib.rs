//! Generic host runtime for FUSEFS mounts.
//!
//! This crate is intentionally independent of any application-specific schema:
//! - [`capability`]: [`CapabilityRouter`] mapping `cap_<id>` root prefixes to
//!   [`fusefs_vfs::CallerContext`] with clamped [`fusefs_vfs::VirtualPath`] and
//!   per-turn revocation.
//! - [`supervisor`]: [`MountSupervisor`] state machine tracking mountpoint,
//!   `nobrowse`, server PID, health, and clean stale-mount recovery across
//!   server kill/restart cycles.
//! - [`fsync`]: progress-bounded `fsync` pass-through that never times out on
//!   fixed wall-clock limits while flush progress is advancing.
//! - [`control`]: framed Unix-domain socket [`ControlProtocol`] (`Mount`,
//!   `Unmount`, `Remount`, `Status`, `HealthPing`, `ListCapabilityRoots`,
//!   `RevokeTurn`).

pub mod capability;
pub mod control;
pub mod fsync;
pub mod supervisor;

pub use capability::{CAPABILITY_PREFIX, CapabilityEntry, CapabilityRouter, RoutedRequest};
pub use control::{
    ControlClient, ControlHandler, ControlProtocol, ControlRequest, ControlResponse, ControlServer,
    ControlServerHandle,
};
pub use fsync::{FsyncPolicy, FsyncProgress, FsyncResult, passthrough_fsync, progress_bounded_fsync};
pub use supervisor::{
    MountBackend, MountConfig, MountState, MountStatus, MountSupervisor, PosixMountBackend,
};
