//! Auto-translating macOS 26.7+ FSKit compatibility layer over [`fusefs_vfs::Volume`].
//!
//! This crate is intentionally generic and contains zero application-specific
//! imports:
//! - [`credentials`]: Darwin `audit_token_t` (`LOCAL_PEERTOKEN`) and POSIX peer
//!   credential extraction into [`fusefs_vfs::CallerContext`].
//! - [`mount_spec`]: [`MountSpec`] builder for `/sbin/mount -F -o nobrowse` and
//!   `FSGenericURLResource` URLs.
//! - [`protocol`]: framed FSKit wire protocol mirroring `FSUnaryFileSystem`,
//!   `FSVolume`, and `FSItem` operations.
//! - [`adapter`]: [`FsKitAdapter`] auto-translating FSKit operations into
//!   [`fusefs_vfs::Volume`] calls with `cap_<id>` root clamping.
//! - [`bridge`]: framed Unix-domain App Group socket server ([`FsKitBridgeServer`])
//!   and client ([`FsKitBridgeClient`]).

pub mod adapter;
pub mod bridge;
pub mod credentials;
pub mod mount_spec;
pub mod protocol;

pub use adapter::FsKitAdapter;
pub use bridge::{FsKitBridgeClient, FsKitBridgeHandle, FsKitBridgeServer};
pub use credentials::{AuditToken, PeerCredentials, extract_peer_credentials};
pub use mount_spec::{MountLayout, MountSpec};
pub use protocol::{
    FSKIT_PROTOCOL_VERSION, FsItemKind, FsKitEnvelope, FsKitRequest, FsKitResponse,
    MAX_BRIDGE_FRAME_BYTES, ProbeResult, read_frame, write_frame,
};
