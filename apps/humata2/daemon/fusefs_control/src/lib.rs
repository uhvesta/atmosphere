//! Thin control-socket client, wire DTOs, and mount-mode probe for
//! `humata2_fusefs`.
//!
//! Kept as a dependency-free leaf crate (only `serde` + `serde_json`) so
//! `humatad_core` can supervise and push topology updates to the external
//! `humata2_fusefs` helper without linking `fusefs_adapter` or `//libs/fusefs/*`.

use std::fmt;
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Maximum framed control message size (1 MiB), matching `fusefs_host::control`.
pub const MAX_CONTROL_FRAME_BYTES: usize = 1024 * 1024;

/// Active mount backend mode for `humata2_fusefs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FsMountMode {
    FsKit,
    Linux,
    Degraded,
}

impl fmt::Display for FsMountMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FsKit => f.write_str("fskit"),
            Self::Linux => f.write_str("linux"),
            Self::Degraded => f.write_str("degraded"),
        }
    }
}

/// Lifecycle state of a supervised FUSEFS mount (wire-compatible with
/// `fusefs_host::MountState`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MountState {
    Unmounted,
    Mounting,
    Healthy,
    Restarting,
    Failed,
}

/// Observable status snapshot reported by `humata2_fusefs` (wire-compatible
/// with `fusefs_host::MountStatus`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MountStatus {
    pub mountpoint: PathBuf,
    pub nobrowse: bool,
    pub resource_url: String,
    pub state: MountState,
    pub server_pid: Option<u32>,
    pub restart_count: u32,
    pub generation: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

/// Metadata describing a registered capability root (wire-compatible with
/// `fusefs_host::CapabilityEntry`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityEntry {
    pub cap_id: String,
    pub root_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    pub read_only: bool,
    pub revoked: bool,
}

/// Namespace marker documenting the control protocol version and framing.
pub struct ControlProtocol;

impl ControlProtocol {
    pub const VERSION: u32 = 1;

    /// Writes a 4-byte big-endian length-prefixed JSON frame.
    pub fn write_frame<T: Serialize>(stream: &mut UnixStream, msg: &T) -> io::Result<()> {
        let payload = serde_json::to_vec(msg)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        if payload.len() > MAX_CONTROL_FRAME_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "control frame exceeds MAX_CONTROL_FRAME_BYTES",
            ));
        }
        let len_be = (payload.len() as u32).to_be_bytes();
        stream.write_all(&len_be)?;
        stream.write_all(&payload)?;
        stream.flush()
    }

    /// Reads a 4-byte big-endian length-prefixed JSON frame.
    pub fn read_frame<T: for<'de> Deserialize<'de>>(stream: &mut UnixStream) -> io::Result<T> {
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf)?;
        let len = u32::from_be_bytes(len_buf) as usize;
        if len > MAX_CONTROL_FRAME_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("incoming control frame length {len} exceeds {MAX_CONTROL_FRAME_BYTES}"),
            ));
        }
        let mut payload = vec![0u8; len];
        stream.read_exact(&mut payload)?;
        serde_json::from_slice(&payload)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// Control RPC request sent over the host control Unix socket (wire-compatible
/// with `fusefs_host::ControlRequest`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ControlRequest {
    Mount {
        #[serde(default)]
        mountpoint: Option<PathBuf>,
        #[serde(default)]
        nobrowse: Option<bool>,
        #[serde(default)]
        resource_url: Option<String>,
    },
    Unmount {
        #[serde(default)]
        force: bool,
    },
    Remount,
    Status,
    HealthPing,
    ListCapabilityRoots,
    RegisterCapabilityRoot {
        cap_id: String,
        root_path: String,
        #[serde(default)]
        turn_id: Option<String>,
        #[serde(default)]
        read_only: bool,
    },
    RevokeTurn {
        turn_id: String,
    },
    Extension {
        body: serde_json::Value,
    },
}

/// Control RPC response returned by `humata2_fusefs` (wire-compatible with
/// `fusefs_host::ControlResponse`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ControlResponse {
    MountStatus {
        mount: MountStatus,
    },
    Pong {
        protocol_version: u32,
        server_pid: u32,
        healthy: bool,
        state: MountState,
    },
    CapabilityRoots {
        roots: Vec<CapabilityEntry>,
    },
    CapabilityRegistered {
        entry: CapabilityEntry,
    },
    TurnRevoked {
        turn_id: String,
        revoked_cap_ids: Vec<String>,
    },
    ExtensionReply {
        body: serde_json::Value,
    },
    Error {
        errno: i32,
        message: String,
    },
}

/// Client for issuing [`ControlRequest`] RPCs over the host control Unix socket.
#[derive(Debug, Clone)]
pub struct ControlClient {
    socket_path: PathBuf,
}

impl ControlClient {
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: socket_path.into(),
        }
    }

    pub fn call(&self, req: &ControlRequest) -> io::Result<ControlResponse> {
        let mut stream = UnixStream::connect(&self.socket_path)?;
        ControlProtocol::write_frame(&mut stream, req)?;
        ControlProtocol::read_frame(&mut stream)
    }

    pub fn mount(&self) -> io::Result<ControlResponse> {
        self.call(&ControlRequest::Mount {
            mountpoint: None,
            nobrowse: None,
            resource_url: None,
        })
    }

    pub fn unmount(&self, force: bool) -> io::Result<ControlResponse> {
        self.call(&ControlRequest::Unmount { force })
    }

    pub fn remount(&self) -> io::Result<ControlResponse> {
        self.call(&ControlRequest::Remount)
    }

    pub fn status(&self) -> io::Result<ControlResponse> {
        self.call(&ControlRequest::Status)
    }

    pub fn health_ping(&self) -> io::Result<ControlResponse> {
        self.call(&ControlRequest::HealthPing)
    }

    pub fn list_capability_roots(&self) -> io::Result<ControlResponse> {
        self.call(&ControlRequest::ListCapabilityRoots)
    }

    pub fn extension(&self, body: serde_json::Value) -> io::Result<ControlResponse> {
        self.call(&ControlRequest::Extension { body })
    }

    pub fn revoke_turn(&self, turn_id: impl Into<String>) -> io::Result<ControlResponse> {
        self.call(&ControlRequest::RevokeTurn {
            turn_id: turn_id.into(),
        })
    }
}

/// Machine entry projected under `/m/<machine_id>` and `/machines/<machine_id>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineProjection {
    pub machine_id: String,
    pub display_name: String,
    pub online: bool,
    pub machine_root: PathBuf,
}

/// Desk entry projected under `/desks/<desk_id>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeskProjection {
    pub desk_id: String,
    pub session_ids: Vec<String>,
}

/// Live connection status between `humata2_fusefs` and `humatad2`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum DaemonLinkState {
    Connected { daemon_pid: u32, epoch: u64 },
    Disconnected { last_daemon_pid: Option<u32>, epoch: u64 },
}

impl DaemonLinkState {
    pub fn is_connected(&self) -> bool {
        matches!(self, Self::Connected { .. })
    }

    pub fn epoch(&self) -> u64 {
        match self {
            Self::Connected { epoch, .. } | Self::Disconnected { epoch, .. } => *epoch,
        }
    }
}

/// A session as `humatad2` describes it to the helper: the plain facts of a
/// session projection, without files or loaders.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionWire {
    pub session_id: String,
    pub machine_id: String,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub desk: Option<String>,
    pub holder_pid: u32,
    pub machine_root: PathBuf,
}

/// The wire form of a topology snapshot pushed from `humatad2` to `humata2_fusefs`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopologyWire {
    pub local_machine_id: String,
    pub local_machine_root: PathBuf,
    #[serde(default)]
    pub machines: Vec<MachineProjection>,
    #[serde(default)]
    pub sessions: Vec<SessionWire>,
    #[serde(default)]
    pub desks: Vec<DeskProjection>,
}

/// What `humatad2` pushes to the helper through the control socket's
/// `Extension` request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "update", rename_all = "snake_case")]
pub enum HostUpdate {
    /// The link between the daemon and the helper changed.
    LinkState { state: DaemonLinkState },
    /// The daemon's machines and sessions, as they are now.
    Topology { topology: TopologyWire },
}

/// The mount mode this machine can run: FSKit on a macOS that has it, the
/// kernel FUSE device on Linux when it opens, else the degraded skeleton.
pub fn probe_mount_mode() -> FsMountMode {
    #[cfg(target_os = "macos")]
    {
        let version = std::process::Command::new("/usr/bin/sw_vers")
            .arg("-productVersion")
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string());
        match version {
            Some(version) if fskit_supported(&version) => FsMountMode::FsKit,
            _ => FsMountMode::Degraded,
        }
    }
    #[cfg(target_os = "linux")]
    {
        probe_dev_fuse_at(Path::new("/dev/fuse"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        FsMountMode::Degraded
    }
}

/// Probes a `/dev/fuse`-like device path for read/write availability.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn probe_dev_fuse_at(path: &Path) -> FsMountMode {
    match std::fs::OpenOptions::new().read(true).write(true).open(path) {
        Ok(_) => FsMountMode::Linux,
        Err(_) => FsMountMode::Degraded,
    }
}

/// FSKit file systems need macOS 15.4 or later.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn fskit_supported(product_version: &str) -> bool {
    let mut parts = product_version.split('.').map(|part| part.parse::<u32>().ok());
    let (Some(Some(major)), minor) = (parts.next(), parts.next().flatten().unwrap_or(0)) else {
        return false;
    };
    (major, minor) >= (15, 4)
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixListener;
    use std::thread;

    use super::*;

    fn sample_topology() -> TopologyWire {
        TopologyWire {
            local_machine_id: "m1".into(),
            local_machine_root: PathBuf::from("/root/m1"),
            machines: vec![MachineProjection {
                machine_id: "m1".into(),
                display_name: "Mac".into(),
                online: true,
                machine_root: PathBuf::from("/root/m1"),
            }],
            sessions: vec![SessionWire {
                session_id: "s1".into(),
                machine_id: "m1".into(),
                channel: Some("main".into()),
                desk: None,
                holder_pid: 40,
                machine_root: PathBuf::from("/root/m1"),
            }],
            desks: vec![DeskProjection {
                desk_id: "d1".into(),
                session_ids: vec!["s1".into()],
            }],
        }
    }

    #[test]
    fn host_update_survives_json_round_trip() {
        let topo_update = HostUpdate::Topology {
            topology: sample_topology(),
        };
        let encoded = serde_json::to_value(&topo_update).unwrap();
        let decoded: HostUpdate = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded, topo_update);

        let link_update = HostUpdate::LinkState {
            state: DaemonLinkState::Connected {
                daemon_pid: 1234,
                epoch: 7,
            },
        };
        assert!(matches!(
            &link_update,
            HostUpdate::LinkState { state } if state.is_connected() && state.epoch() == 7
        ));
        let encoded = serde_json::to_value(&link_update).unwrap();
        let decoded: HostUpdate = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded, link_update);
    }

    #[test]
    fn control_client_speaks_framed_protocol_over_unix_socket() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("control.sock");
        let listener = UnixListener::bind(&sock).unwrap();

        let server = thread::spawn(move || {
            for _ in 0..3 {
                let (mut stream, _) = listener.accept().unwrap();
                let req: ControlRequest = ControlProtocol::read_frame(&mut stream).unwrap();
                let resp = match req {
                    ControlRequest::HealthPing => ControlResponse::Pong {
                        protocol_version: ControlProtocol::VERSION,
                        server_pid: 42,
                        healthy: true,
                        state: MountState::Healthy,
                    },
                    ControlRequest::Status => ControlResponse::MountStatus {
                        mount: MountStatus {
                            mountpoint: PathBuf::from("/tmp/mnt"),
                            nobrowse: true,
                            resource_url: "fusefs://test".into(),
                            state: MountState::Healthy,
                            server_pid: Some(42),
                            restart_count: 0,
                            generation: 1,
                            last_error: None,
                        },
                    },
                    ControlRequest::Extension { body } => ControlResponse::ExtensionReply { body },
                    other => ControlResponse::Error {
                        errno: 22,
                        message: format!("unexpected request: {other:?}"),
                    },
                };
                ControlProtocol::write_frame(&mut stream, &resp).unwrap();
            }
        });

        let client = ControlClient::new(&sock);
        assert!(matches!(
            client.health_ping().unwrap(),
            ControlResponse::Pong {
                protocol_version: 1,
                server_pid: 42,
                healthy: true,
                state: MountState::Healthy,
            }
        ));
        assert!(matches!(
            client.status().unwrap(),
            ControlResponse::MountStatus { mount } if mount.state == MountState::Healthy
        ));
        let update = HostUpdate::Topology {
            topology: sample_topology(),
        };
        let body = serde_json::to_value(&update).unwrap();
        assert_eq!(
            client.extension(body.clone()).unwrap(),
            ControlResponse::ExtensionReply { body }
        );
        server.join().unwrap();
    }

    #[test]
    fn fskit_needs_macos_15_4_or_later() {
        assert!(fskit_supported("15.4"));
        assert!(fskit_supported("15.6.1"));
        assert!(fskit_supported("26.0"));
        assert!(!fskit_supported("15.3"));
        assert!(!fskit_supported("14.7"));
        assert!(!fskit_supported("garbage"));
    }

    #[test]
    fn probe_dev_fuse_at_reports_linux_when_openable_and_degraded_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("no-fuse");
        assert_eq!(probe_dev_fuse_at(&missing), FsMountMode::Degraded);

        let fake_dev = dir.path().join("fuse");
        std::fs::write(&fake_dev, b"").unwrap();
        assert_eq!(probe_dev_fuse_at(&fake_dev), FsMountMode::Linux);
    }

    #[test]
    fn fs_mount_mode_display_matches_cli_flags() {
        assert_eq!(FsMountMode::FsKit.to_string(), "fskit");
        assert_eq!(FsMountMode::Linux.to_string(), "linux");
        assert_eq!(FsMountMode::Degraded.to_string(), "degraded");
    }
}
