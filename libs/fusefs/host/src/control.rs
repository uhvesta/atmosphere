//! Framed Unix-domain socket control runtime (`ControlProtocol`).
//!
//! Supports `Mount`, `Unmount`, `Remount`, `Status`, `HealthPing`,
//! `ListCapabilityRoots`, `RegisterCapabilityRoot`, `RevokeTurn`, and a generic
//! `Extension` request an embedding application answers with its own callback.

use std::io::{self, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};

use fusefs_vfs::VirtualPath;
use fusefs_vfs::error::errno;
use serde::{Deserialize, Serialize};

use crate::capability::{CapabilityEntry, CapabilityRouter};
use crate::supervisor::{MountState, MountStatus, MountSupervisor};

/// Maximum framed control message size (1 MiB).
pub const MAX_CONTROL_FRAME_BYTES: usize = 1024 * 1024;

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

/// Control RPC request sent over the host control Unix socket.
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
    /// An application-defined request. The host knows nothing of its shape:
    /// it hands `body` to the handler's extension callback (see
    /// [`ControlHandler::with_extension`]).
    Extension {
        body: serde_json::Value,
    },
}

/// Control RPC response returned by [`ControlServer`].
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
    /// The extension callback's answer to a [`ControlRequest::Extension`].
    ExtensionReply {
        body: serde_json::Value,
    },
    Error {
        errno: i32,
        message: String,
    },
}

/// Answers [`ControlRequest::Extension`] bodies: `Ok` is sent back as the
/// reply body, `Err` as an `EINVAL` error carrying the message.
pub type ExtensionCallback =
    Arc<dyn Fn(serde_json::Value) -> Result<serde_json::Value, String> + Send + Sync>;

/// Pure request dispatcher backing [`ControlServer`].
pub struct ControlHandler {
    supervisor: Arc<MountSupervisor>,
    router: Arc<CapabilityRouter>,
    extension: Option<ExtensionCallback>,
}

impl ControlHandler {
    pub fn new(supervisor: Arc<MountSupervisor>, router: Arc<CapabilityRouter>) -> Self {
        Self {
            supervisor,
            router,
            extension: None,
        }
    }

    /// Registers the callback that answers [`ControlRequest::Extension`].
    /// Without one, extension requests are refused with `ENOSYS`.
    pub fn with_extension(mut self, callback: ExtensionCallback) -> Self {
        self.extension = Some(callback);
        self
    }

    pub fn supervisor(&self) -> &Arc<MountSupervisor> {
        &self.supervisor
    }

    pub fn router(&self) -> &Arc<CapabilityRouter> {
        &self.router
    }

    pub fn handle(&self, req: ControlRequest) -> ControlResponse {
        match req {
            ControlRequest::Mount {
                mountpoint,
                nobrowse,
                resource_url,
            } => {
                self.supervisor
                    .update_config(mountpoint, nobrowse, resource_url);
                match self.supervisor.mount() {
                    Ok(mount) => ControlResponse::MountStatus { mount },
                    Err(err) => io_err_response(err),
                }
            }
            ControlRequest::Unmount { force } => match self.supervisor.unmount(force) {
                Ok(mount) => ControlResponse::MountStatus { mount },
                Err(err) => io_err_response(err),
            },
            ControlRequest::Remount => match self.supervisor.remount() {
                Ok(mount) => ControlResponse::MountStatus { mount },
                Err(err) => io_err_response(err),
            },
            ControlRequest::Status => ControlResponse::MountStatus {
                mount: self.supervisor.status(),
            },
            ControlRequest::HealthPing => {
                let status = self
                    .supervisor
                    .check_health_and_recover()
                    .unwrap_or_else(|_| self.supervisor.status());
                ControlResponse::Pong {
                    protocol_version: ControlProtocol::VERSION,
                    server_pid: std::process::id(),
                    healthy: status.state == MountState::Healthy,
                    state: status.state,
                }
            }
            ControlRequest::ListCapabilityRoots => ControlResponse::CapabilityRoots {
                roots: self.router.list_active(),
            },
            ControlRequest::RegisterCapabilityRoot {
                cap_id,
                root_path,
                turn_id,
                read_only,
            } => match VirtualPath::parse(&root_path)
                .and_then(|vp| self.router.register(cap_id, vp, turn_id, read_only))
            {
                Ok(entry) => ControlResponse::CapabilityRegistered { entry },
                Err(err) => ControlResponse::Error {
                    errno: err.errno(),
                    message: err.to_string(),
                },
            },
            ControlRequest::RevokeTurn { turn_id } => {
                let revoked_cap_ids = self.router.revoke_turn(&turn_id);
                ControlResponse::TurnRevoked {
                    turn_id,
                    revoked_cap_ids,
                }
            }
            ControlRequest::Extension { body } => match &self.extension {
                Some(callback) => match callback(body) {
                    Ok(body) => ControlResponse::ExtensionReply { body },
                    Err(message) => ControlResponse::Error {
                        errno: errno::EINVAL,
                        message,
                    },
                },
                None => ControlResponse::Error {
                    errno: errno::ENOSYS,
                    message: "this host has no extension handler".to_string(),
                },
            },
        }
    }
}

fn io_err_response(err: io::Error) -> ControlResponse {
    ControlResponse::Error {
        errno: err.raw_os_error().unwrap_or(errno::EIO),
        message: err.to_string(),
    }
}

/// Unix-domain socket server exposing [`ControlProtocol`].
pub struct ControlServer {
    socket_path: PathBuf,
    listener: UnixListener,
    handler: Arc<ControlHandler>,
}

impl ControlServer {
    /// Binds a new control server at `socket_path` (removing any stale socket file first).
    pub fn bind(socket_path: impl Into<PathBuf>, handler: Arc<ControlHandler>) -> io::Result<Self> {
        let socket_path = socket_path.into();
        if socket_path.exists() {
            let _ = std::fs::remove_file(&socket_path);
        }
        let listener = UnixListener::bind(&socket_path)?;
        // Only this user may connect: the socket can mount and unmount.
        std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))?;
        Ok(Self {
            socket_path,
            listener,
            handler,
        })
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Accepts and serves one connection (handling all framed requests until EOF).
    pub fn serve_one(&self) -> io::Result<()> {
        let (mut stream, _) = self.listener.accept()?;
        loop {
            let req: ControlRequest = match ControlProtocol::read_frame(&mut stream) {
                Ok(req) => req,
                Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
                Err(err) => return Err(err),
            };
            let resp = self.handler.handle(req);
            ControlProtocol::write_frame(&mut stream, &resp)?;
        }
    }

    /// Spawns a background accept loop and returns a handle that stops cleanly on drop/shutdown.
    pub fn spawn(self) -> ControlServerHandle {
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_flag = Arc::clone(&shutdown);
        let socket_path = self.socket_path.clone();

        let join = thread::spawn(move || {
            for incoming in self.listener.incoming() {
                if shutdown_flag.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(mut stream) = incoming else {
                    if shutdown_flag.load(Ordering::SeqCst) {
                        break;
                    }
                    continue;
                };
                let handler = Arc::clone(&self.handler);
                thread::spawn(move || {
                    loop {
                        let req: ControlRequest = match ControlProtocol::read_frame(&mut stream) {
                            Ok(req) => req,
                            Err(_) => break,
                        };
                        let resp = handler.handle(req);
                        if ControlProtocol::write_frame(&mut stream, &resp).is_err() {
                            break;
                        }
                    }
                });
            }
        });

        ControlServerHandle {
            socket_path,
            shutdown,
            join: Some(join),
        }
    }
}

/// Handle to a running background [`ControlServer`].
pub struct ControlServerHandle {
    socket_path: PathBuf,
    shutdown: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl ControlServerHandle {
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Gracefully stops the background server and removes the socket file.
    pub fn shutdown(mut self) {
        self.stop_inner();
    }

    fn stop_inner(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        // Unblock `listener.accept()` by connecting briefly.
        let _ = UnixStream::connect(&self.socket_path);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

impl Drop for ControlServerHandle {
    fn drop(&mut self) {
        self.stop_inner();
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::supervisor::MountConfig;
    use crate::supervisor::tests::FakeMountBackend;

    #[test]
    fn loopback_control_socket_handles_all_rpcs_and_restart_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let sock_path = dir.path().join("control.sock");

        let backend = Arc::new(FakeMountBackend::new());
        let supervisor = Arc::new(MountSupervisor::new(
            MountConfig::new(dir.path().join("mnt"), "fusefs://main"),
            backend.clone(),
        ));
        let router = Arc::new(CapabilityRouter::new());
        let handler = Arc::new(ControlHandler::new(supervisor, router.clone()));

        let server = ControlServer::bind(&sock_path, handler).unwrap().spawn();
        let client = ControlClient::new(server.socket_path());

        // Initial status: Unmounted
        match client.status().unwrap() {
            ControlResponse::MountStatus { mount } => {
                assert_eq!(mount.state, MountState::Unmounted);
                assert!(mount.nobrowse);
            }
            other => panic!("unexpected response: {other:?}"),
        }

        // Mount
        match client.mount().unwrap() {
            ControlResponse::MountStatus { mount } => {
                assert_eq!(mount.state, MountState::Healthy);
                assert_eq!(mount.server_pid, Some(1000));
            }
            other => panic!("unexpected response: {other:?}"),
        }

        // HealthPing when healthy
        match client.health_ping().unwrap() {
            ControlResponse::Pong {
                protocol_version,
                healthy,
                state,
                ..
            } => {
                assert_eq!(protocol_version, ControlProtocol::VERSION);
                assert!(healthy);
                assert_eq!(state, MountState::Healthy);
            }
            other => panic!("unexpected response: {other:?}"),
        }

        // Register capability root, list it, then revoke by turn.
        match client
            .call(&ControlRequest::RegisterCapabilityRoot {
                cap_id: "cap_alpha".into(),
                root_path: "/workspaces/alpha".into(),
                turn_id: Some("turn-99".into()),
                read_only: false,
            })
            .unwrap()
        {
            ControlResponse::CapabilityRegistered { entry } => {
                assert_eq!(entry.cap_id, "cap_alpha");
                assert_eq!(entry.root_path.as_str(), "/workspaces/alpha");
            }
            other => panic!("unexpected response: {other:?}"),
        }

        match client.list_capability_roots().unwrap() {
            ControlResponse::CapabilityRoots { roots } => {
                assert_eq!(roots.len(), 1);
                assert_eq!(roots[0].cap_id, "cap_alpha");
            }
            other => panic!("unexpected response: {other:?}"),
        }

        match client.revoke_turn("turn-99").unwrap() {
            ControlResponse::TurnRevoked {
                turn_id,
                revoked_cap_ids,
            } => {
                assert_eq!(turn_id, "turn-99");
                assert_eq!(revoked_cap_ids, vec!["cap_alpha".to_string()]);
            }
            other => panic!("unexpected response: {other:?}"),
        }

        match client.list_capability_roots().unwrap() {
            ControlResponse::CapabilityRoots { roots } => assert!(roots.is_empty()),
            other => panic!("unexpected response: {other:?}"),
        }

        // Simulate abrupt server kill + stale mount; HealthPing triggers recovery.
        backend.simulate_server_kill(true);
        match client.health_ping().unwrap() {
            ControlResponse::Pong { healthy, state, .. } => {
                assert!(healthy);
                assert_eq!(state, MountState::Healthy);
            }
            other => panic!("unexpected response: {other:?}"),
        }
        match client.status().unwrap() {
            ControlResponse::MountStatus { mount } => {
                assert_eq!(mount.server_pid, Some(1001));
                assert_eq!(mount.restart_count, 1);
            }
            other => panic!("unexpected response: {other:?}"),
        }

        // Remount and Unmount
        match client.remount().unwrap() {
            ControlResponse::MountStatus { mount } => {
                assert_eq!(mount.state, MountState::Healthy);
                assert_eq!(mount.server_pid, Some(1002));
                assert_eq!(mount.restart_count, 2);
            }
            other => panic!("unexpected response: {other:?}"),
        }

        match client.unmount(false).unwrap() {
            ControlResponse::MountStatus { mount } => {
                assert_eq!(mount.state, MountState::Unmounted);
                assert_eq!(mount.server_pid, None);
            }
            other => panic!("unexpected response: {other:?}"),
        }

        server.shutdown();
    }

    fn bound_server(
        dir: &Path,
        extension: Option<ExtensionCallback>,
    ) -> (ControlServerHandle, PathBuf) {
        let sock_path = dir.join("control.sock");
        let supervisor = Arc::new(MountSupervisor::new(
            MountConfig::new(dir.join("mnt"), "fusefs://main"),
            Arc::new(FakeMountBackend::new()),
        ));
        let mut handler = ControlHandler::new(supervisor, Arc::new(CapabilityRouter::new()));
        if let Some(callback) = extension {
            handler = handler.with_extension(callback);
        }
        let server = ControlServer::bind(&sock_path, Arc::new(handler))
            .unwrap()
            .spawn();
        (server, sock_path)
    }

    #[test]
    fn the_control_socket_is_private_to_its_user() {
        let dir = tempfile::tempdir().unwrap();
        let (server, sock_path) = bound_server(dir.path(), None);
        let mode = std::fs::metadata(&sock_path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        server.shutdown();
    }

    #[test]
    fn an_extension_request_reaches_the_callback_and_its_answer_comes_back() {
        let dir = tempfile::tempdir().unwrap();
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen_in = Arc::clone(&seen);
        let callback: ExtensionCallback = Arc::new(move |body: serde_json::Value| {
            seen_in.lock().unwrap().push(body.clone());
            if body.get("bad").is_some() {
                Err("refused".to_string())
            } else {
                Ok(serde_json::json!({ "echo": body }))
            }
        });
        let (server, _) = bound_server(dir.path(), Some(callback));
        let client = ControlClient::new(server.socket_path());

        match client.extension(serde_json::json!({ "n": 1 })).unwrap() {
            ControlResponse::ExtensionReply { body } => {
                assert_eq!(body, serde_json::json!({ "echo": { "n": 1 } }));
            }
            other => panic!("unexpected response: {other:?}"),
        }
        match client.extension(serde_json::json!({ "bad": true })).unwrap() {
            ControlResponse::Error { errno: code, .. } => assert_eq!(code, errno::EINVAL),
            other => panic!("unexpected response: {other:?}"),
        }
        assert_eq!(seen.lock().unwrap().len(), 2);
        server.shutdown();
    }

    #[test]
    fn an_extension_request_without_a_callback_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (server, _) = bound_server(dir.path(), None);
        let client = ControlClient::new(server.socket_path());
        match client.extension(serde_json::json!({})).unwrap() {
            ControlResponse::Error { errno: code, .. } => assert_eq!(code, errno::ENOSYS),
            other => panic!("unexpected response: {other:?}"),
        }
        server.shutdown();
    }
}
