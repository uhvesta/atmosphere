//! Gate T1 FSKit probe: local App Group Unix socket bridge, caller identity
//! (`audit_token_t` / `LOCAL_PEERPID`) extraction, and `/sbin/mount -F -o nobrowse`
//! command & mountpoint validation.

use std::io::{self, Read, Write};
use std::os::unix::io::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Maximum framed IPC message size (1 MiB) for the spike bridge.
const MAX_FRAME_BYTES: usize = 1024 * 1024;

/// Caller identity extracted from a Unix domain socket peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerIdentity {
    pub pid: i32,
    pub euid: u32,
    pub egid: u32,
    pub via_audit_token: bool,
}

/// Request sent by the FSKit `.appex` over the App Group Unix domain socket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum BridgeRequest {
    Ping,
    Lookup { path: String },
    Read { path: String, offset: u64, len: u32 },
}

/// Response returned by `humata2_fusefs` over the App Group Unix domain socket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum BridgeResponse {
    Pong {
        peer: PeerIdentity,
        server_pid: u32,
    },
    NotFound {
        path: String,
    },
    Error {
        errno: i32,
        message: String,
    },
}

/// Strategy for placing the FSKit mountpoint on macOS 26.7+.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MountStrategy {
    /// Mount directly at `~/.humata/fuse/main` with `-o nobrowse`.
    Direct { mountpoint: PathBuf },
    /// Mount at a hidden backing dir and expose `~/.humata/fuse/main` as a symlink.
    BackingWithSymlink {
        backing_mountpoint: PathBuf,
        canonical_symlink: PathBuf,
    },
}

/// Specification for invoking `/sbin/mount -F -o nobrowse -t humatafs ...`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountCommandSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
}

impl MountCommandSpec {
    /// Builds the `/sbin/mount` invocation for an `FSGenericURLResource` FSKit module.
    ///
    /// Uses `-F` (FSKit) and `-o nobrowse` (`MNT_DONTBROWSE`) so Finder and Desktop
    /// do not display spurious ejectable drives.
    pub fn for_fskit(resource_url: &str, mountpoint: &Path) -> io::Result<Self> {
        if !resource_url.starts_with("humata-fuse://") {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("expected humata-fuse:// resource URL, got {resource_url}"),
            ));
        }
        Ok(Self {
            program: PathBuf::from("/sbin/mount"),
            args: vec![
                "-F".to_owned(),
                "-o".to_owned(),
                "nobrowse".to_owned(),
                "-t".to_owned(),
                "humatafs".to_owned(),
                resource_url.to_owned(),
                mountpoint.to_string_lossy().into_owned(),
            ],
        })
    }
}

/// Extracts peer PID/EUID/EGID from a connected Unix domain socket.
///
/// On macOS, probes `SOL_LOCAL` + `LOCAL_PEERTOKEN` (`audit_token_t`, 8 x `u32`:
/// `[auid, euid, egid, ruid, rgid, pid, asid, pidversion]`) first and falls back
/// to `LOCAL_PEERPID` + `getpeereid`.
#[cfg(target_os = "macos")]
pub fn peer_identity(stream: &UnixStream) -> io::Result<PeerIdentity> {
    const SOL_LOCAL: libc::c_int = 0;
    const LOCAL_PEERPID: libc::c_int = 0x002;
    const LOCAL_PEERTOKEN: libc::c_int = 0x006;

    #[repr(C)]
    #[derive(Default)]
    struct AuditToken {
        val: [u32; 8],
    }

    let fd = stream.as_raw_fd();
    let mut token = AuditToken::default();
    let mut token_len = std::mem::size_of::<AuditToken>() as libc::socklen_t;

    // SAFETY: `fd` is an open Unix stream socket, and `token` / `token_len` match
    // Darwin's `audit_token_t` size for `LOCAL_PEERTOKEN`.
    let rc = unsafe {
        libc::getsockopt(
            fd,
            SOL_LOCAL,
            LOCAL_PEERTOKEN,
            (&raw mut token).cast::<libc::c_void>(),
            &raw mut token_len,
        )
    };
    if rc == 0 && token_len as usize == std::mem::size_of::<AuditToken>() {
        return Ok(PeerIdentity {
            euid: token.val[1],
            egid: token.val[2],
            pid: token.val[5] as i32,
            via_audit_token: true,
        });
    }

    let mut pid: libc::pid_t = 0;
    let mut pid_len = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
    // SAFETY: `fd` is an open Unix stream socket and `pid` holds `pid_t`.
    let pid_rc = unsafe {
        libc::getsockopt(
            fd,
            SOL_LOCAL,
            LOCAL_PEERPID,
            (&raw mut pid).cast::<libc::c_void>(),
            &raw mut pid_len,
        )
    };
    if pid_rc != 0 {
        return Err(io::Error::last_os_error());
    }

    let mut euid: libc::uid_t = 0;
    let mut egid: libc::gid_t = 0;
    // SAFETY: `fd` is a valid Unix stream socket.
    let eid_rc = unsafe { libc::getpeereid(fd, &raw mut euid, &raw mut egid) };
    if eid_rc != 0 {
        return Err(io::Error::last_os_error());
    }

    Ok(PeerIdentity {
        pid,
        euid,
        egid,
        via_audit_token: false,
    })
}

/// Extracts peer PID/EUID/EGID on Linux via `SO_PEERCRED`.
#[cfg(target_os = "linux")]
pub fn peer_identity(stream: &UnixStream) -> io::Result<PeerIdentity> {
    let fd = stream.as_raw_fd();
    let mut cred = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: `fd` is an open Unix stream socket and `cred` matches `SO_PEERCRED`.
    let rc = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&raw mut cred).cast::<libc::c_void>(),
            &raw mut len,
        )
    };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(PeerIdentity {
        pid: cred.pid,
        euid: cred.uid,
        egid: cred.gid,
        via_audit_token: false,
    })
}

/// Writes one 4-byte big-endian length-prefixed JSON frame.
pub fn write_frame<T: Serialize>(stream: &mut UnixStream, msg: &T) -> io::Result<()> {
    let payload = serde_json::to_vec(msg)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    if payload.len() > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "frame exceeds MAX_FRAME_BYTES",
        ));
    }
    let len_be = (payload.len() as u32).to_be_bytes();
    stream.write_all(&len_be)?;
    stream.write_all(&payload)?;
    stream.flush()
}

/// Reads one 4-byte big-endian length-prefixed JSON frame.
pub fn read_frame<T: for<'de> Deserialize<'de>>(stream: &mut UnixStream) -> io::Result<T> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("incoming frame length {len} exceeds {MAX_FRAME_BYTES}"),
        ));
    }
    let mut payload = vec![0u8; len];
    stream.read_exact(&mut payload)?;
    serde_json::from_slice(&payload).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

/// Serves a single framed request on `listener` and returns the extracted peer identity.
pub fn serve_one_request(listener: &UnixListener) -> io::Result<PeerIdentity> {
    let (mut stream, _) = listener.accept()?;
    let peer = peer_identity(&stream)?;
    let req: BridgeRequest = read_frame(&mut stream)?;
    let resp = match req {
        BridgeRequest::Ping => BridgeResponse::Pong {
            peer,
            server_pid: std::process::id(),
        },
        BridgeRequest::Lookup { path } => BridgeResponse::NotFound { path },
        BridgeRequest::Read { path, .. } => BridgeResponse::Error {
            errno: libc::ENOENT,
            message: format!("spike stub: {path} not backed"),
        },
    };
    write_frame(&mut stream, &resp)?;
    Ok(peer)
}

/// Sends a single framed request over the App Group Unix domain socket.
pub fn client_round_trip(socket_path: &Path, req: &BridgeRequest) -> io::Result<BridgeResponse> {
    let mut stream = UnixStream::connect(socket_path)?;
    write_frame(&mut stream, req)?;
    read_frame(&mut stream)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn mount_command_spec_enforces_fskit_and_nobrowse() {
        let spec = MountCommandSpec::for_fskit(
            "humata-fuse://main",
            Path::new("/Users/example/.humata/fuse/main"),
        )
        .expect("valid spec");
        assert_eq!(spec.program, PathBuf::from("/sbin/mount"));
        assert_eq!(
            spec.args,
            vec![
                "-F",
                "-o",
                "nobrowse",
                "-t",
                "humatafs",
                "humata-fuse://main",
                "/Users/example/.humata/fuse/main",
            ]
        );
    }

    #[test]
    fn mount_command_spec_rejects_non_humata_scheme() {
        let err = MountCommandSpec::for_fskit("file:///tmp", Path::new("/tmp/m"))
            .expect_err("non-humata scheme must fail");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn app_group_unix_socket_bridge_extracts_caller_pid_and_euid() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sock_path = dir.path().join("fusefs.sock");
        let listener = UnixListener::bind(&sock_path).expect("bind unix socket");

        let server = thread::spawn(move || serve_one_request(&listener));
        let resp = client_round_trip(&sock_path, &BridgeRequest::Ping).expect("ping round-trip");
        let observed_peer = server.join().expect("server thread").expect("server ok");

        let expected_pid = std::process::id() as i32;
        assert_eq!(observed_peer.pid, expected_pid);
        match resp {
            BridgeResponse::Pong { peer, server_pid } => {
                assert_eq!(peer, observed_peer);
                assert_eq!(server_pid, std::process::id());
            }
            other => panic!("expected Pong, got {other:?}"),
        }
    }

    #[test]
    fn backing_with_symlink_layout_resolves_cleanly() {
        let dir = tempfile::tempdir().expect("tempdir");
        let backing = dir.path().join("volumes_backing");
        let link = dir.path().join("main");
        std::fs::create_dir_all(&backing).expect("create backing");
        std::os::unix::fs::symlink(&backing, &link).expect("symlink");

        let strategy = MountStrategy::BackingWithSymlink {
            backing_mountpoint: backing.clone(),
            canonical_symlink: link.clone(),
        };
        match strategy {
            MountStrategy::BackingWithSymlink {
                backing_mountpoint,
                canonical_symlink,
            } => {
                assert_eq!(
                    std::fs::canonicalize(&canonical_symlink).expect("canonicalize"),
                    std::fs::canonicalize(&backing_mountpoint).expect("canonicalize backing")
                );
            }
            MountStrategy::Direct { .. } => unreachable!(),
        }
    }
}
