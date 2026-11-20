//! Framed FSKit bridge wire protocol mirroring macOS 26.7+ `FSUnaryFileSystem`,
//! `FSVolume`, and `FSItem` operations.

use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;

use fusefs_vfs::{DirEntry, FileAttr, FileHandle, NodeId, OpenFlags, SetAttr, StatFs};
use serde::{Deserialize, Serialize};

use crate::credentials::{AuditToken, PeerCredentials};

/// Wire protocol version for the App Group FSKit bridge.
pub const FSKIT_PROTOCOL_VERSION: u32 = 1;

/// Maximum framed bridge message size (16 MiB) to accommodate large read/write payloads.
pub const MAX_BRIDGE_FRAME_BYTES: usize = 16 * 1024 * 1024;

/// Item kind requested by FSKit `createItem`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FsItemKind {
    Regular,
    Directory,
    Symlink,
}

/// Result of `FSUnaryFileSystem.probeResource`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "recognition", rename_all = "snake_case")]
pub enum ProbeResult {
    Usable { name: String, container_id: String },
    NotRecognized,
}

/// Request envelope sent by the FSKit `.appex` extension over the App Group Unix socket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsKitEnvelope {
    /// Optional `cap_<id>` root scoping this request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cap_id: Option<String>,
    /// Optional explicit Darwin `audit_token_t` (falls back to socket peer credentials).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audit_token: Option<AuditToken>,
    /// The FSKit operation to execute.
    pub request: FsKitRequest,
}

impl FsKitEnvelope {
    pub fn new(request: FsKitRequest) -> Self {
        Self {
            cap_id: None,
            audit_token: None,
            request,
        }
    }

    pub fn with_cap_id(mut self, cap_id: impl Into<String>) -> Self {
        self.cap_id = Some(cap_id.into());
        self
    }

    pub fn with_audit_token(mut self, token: AuditToken) -> Self {
        self.audit_token = Some(token);
        self
    }
}

/// FSKit `FSUnaryFileSystem` / `FSVolume` / `FSItem` operations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum FsKitRequest {
    Ping,
    ProbeResource {
        resource_url: String,
    },
    LookupItem {
        parent: NodeId,
        name: String,
    },
    GetAttributes {
        item: NodeId,
    },
    SetAttributes {
        item: NodeId,
        attr: SetAttr,
    },
    EnumerateDirectory {
        directory: NodeId,
        cookie: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_entries: Option<usize>,
    },
    ReadSymbolicLink {
        item: NodeId,
    },
    CreateItem {
        parent: NodeId,
        name: String,
        kind: FsItemKind,
        mode: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        symlink_target: Option<String>,
    },
    RemoveItem {
        parent: NodeId,
        name: String,
        is_directory: bool,
    },
    RenameItem {
        source_parent: NodeId,
        source_name: String,
        dest_parent: NodeId,
        dest_name: String,
    },
    OpenItem {
        item: NodeId,
        flags: OpenFlags,
    },
    CloseItem {
        item: NodeId,
        handle: FileHandle,
    },
    Read {
        item: NodeId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        handle: Option<FileHandle>,
        offset: u64,
        length: u32,
    },
    Write {
        item: NodeId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        handle: Option<FileHandle>,
        offset: u64,
        data: Vec<u8>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expected_generation: Option<u64>,
    },
    Synchronize {
        item: NodeId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        handle: Option<FileHandle>,
        #[serde(default)]
        datasync: bool,
    },
    VolumeStatistics {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        item: Option<NodeId>,
    },
}

/// Response returned by [`crate::FsKitAdapter`] and [`crate::FsKitBridgeServer`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum FsKitResponse {
    Pong {
        protocol_version: u32,
        peer: PeerCredentials,
        server_pid: u32,
    },
    Probe {
        result: ProbeResult,
    },
    ItemAttributes {
        attr: FileAttr,
    },
    DirectoryEntries {
        entries: Vec<DirEntry>,
        next_cookie: u64,
        eof: bool,
    },
    SymbolicLink {
        target: String,
    },
    Opened {
        handle: FileHandle,
    },
    ReadData {
        data: Vec<u8>,
    },
    Written {
        bytes_written: u32,
    },
    Statistics {
        stats: StatFs,
    },
    Ok,
    Error {
        errno: i32,
        message: String,
    },
}

/// Writes one 4-byte big-endian length-prefixed JSON frame.
pub fn write_frame<T: Serialize>(stream: &mut UnixStream, msg: &T) -> io::Result<()> {
    let payload = serde_json::to_vec(msg)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    if payload.len() > MAX_BRIDGE_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "bridge frame exceeds MAX_BRIDGE_FRAME_BYTES",
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
    if len > MAX_BRIDGE_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("incoming bridge frame length {len} exceeds {MAX_BRIDGE_FRAME_BYTES}"),
        ));
    }
    let mut payload = vec![0u8; len];
    stream.read_exact(&mut payload)?;
    serde_json::from_slice(&payload).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}
