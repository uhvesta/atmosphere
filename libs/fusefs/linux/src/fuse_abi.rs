//! Portable Linux FUSE 7.x ABI opcodes, wire headers, and typed request/reply definitions.

use fusefs_vfs::{
    DirEntry, FileAttr, FileHandle, FlockOp, NodeId, OpenFlags, SetAttr, StatFs, VfsError,
};
use serde::{Deserialize, Serialize};

/// Size in bytes of Linux `struct fuse_in_header`.
pub const FUSE_IN_HEADER_SIZE: usize = 40;

/// Size in bytes of Linux `struct fuse_out_header`.
pub const FUSE_OUT_HEADER_SIZE: usize = 16;

/// Linux FUSE 7.x ABI opcode numbers (matching `<linux/fuse.h>`).
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FuseOpcode {
    Lookup = 1,
    Forget = 2,
    GetAttr = 3,
    SetAttr = 4,
    ReadLink = 5,
    Symlink = 6,
    MkNod = 8,
    MkDir = 9,
    Unlink = 10,
    RmDir = 11,
    Rename = 12,
    Link = 13,
    Open = 14,
    Read = 15,
    Write = 16,
    StatFs = 17,
    Release = 18,
    Fsync = 20,
    SetXattr = 21,
    GetXattr = 22,
    ListXattr = 23,
    RemoveXattr = 24,
    Flush = 25,
    Init = 26,
    OpenDir = 27,
    ReadDir = 28,
    ReleaseDir = 29,
    Flock = 32,
    Access = 34,
    Create = 35,
    Interrupt = 36,
    Destroy = 38,
    BatchForget = 42,
    Rename2 = 45,
    CopyFileRange = 47,
}

impl FuseOpcode {
    /// Decodes a raw Linux FUSE ABI opcode integer.
    pub fn from_raw(raw: u32) -> Option<Self> {
        match raw {
            1 => Some(Self::Lookup),
            2 => Some(Self::Forget),
            3 => Some(Self::GetAttr),
            4 => Some(Self::SetAttr),
            5 => Some(Self::ReadLink),
            6 => Some(Self::Symlink),
            8 => Some(Self::MkNod),
            9 => Some(Self::MkDir),
            10 => Some(Self::Unlink),
            11 => Some(Self::RmDir),
            12 => Some(Self::Rename),
            13 => Some(Self::Link),
            14 => Some(Self::Open),
            15 => Some(Self::Read),
            16 => Some(Self::Write),
            17 => Some(Self::StatFs),
            18 => Some(Self::Release),
            20 => Some(Self::Fsync),
            21 => Some(Self::SetXattr),
            22 => Some(Self::GetXattr),
            23 => Some(Self::ListXattr),
            24 => Some(Self::RemoveXattr),
            25 => Some(Self::Flush),
            26 => Some(Self::Init),
            27 => Some(Self::OpenDir),
            28 => Some(Self::ReadDir),
            29 => Some(Self::ReleaseDir),
            32 | 33 => Some(Self::Flock),
            34 => Some(Self::Access),
            35 => Some(Self::Create),
            36 => Some(Self::Interrupt),
            38 => Some(Self::Destroy),
            42 => Some(Self::BatchForget),
            45 => Some(Self::Rename2),
            47 => Some(Self::CopyFileRange),
            _ => None,
        }
    }

    pub const fn as_raw(self) -> u32 {
        self as u32
    }
}

/// Wire-level Linux `struct fuse_in_header` (40 bytes, native/little-endian).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FuseInHeader {
    pub len: u32,
    pub opcode: u32,
    pub unique: u64,
    pub nodeid: u64,
    pub uid: u32,
    pub gid: u32,
    pub pid: u32,
    pub padding: u32,
}

impl FuseInHeader {
    /// Encodes the header into 40 little-endian bytes.
    pub fn encode(&self) -> [u8; FUSE_IN_HEADER_SIZE] {
        let mut buf = [0u8; FUSE_IN_HEADER_SIZE];
        buf[0..4].copy_from_slice(&self.len.to_le_bytes());
        buf[4..8].copy_from_slice(&self.opcode.to_le_bytes());
        buf[8..16].copy_from_slice(&self.unique.to_le_bytes());
        buf[16..24].copy_from_slice(&self.nodeid.to_le_bytes());
        buf[24..28].copy_from_slice(&self.uid.to_le_bytes());
        buf[28..32].copy_from_slice(&self.gid.to_le_bytes());
        buf[32..36].copy_from_slice(&self.pid.to_le_bytes());
        buf[36..40].copy_from_slice(&self.padding.to_le_bytes());
        buf
    }

    /// Decodes 40 little-endian bytes into a [`FuseInHeader`].
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < FUSE_IN_HEADER_SIZE {
            return None;
        }
        Some(Self {
            len: u32::from_le_bytes(bytes[0..4].try_into().ok()?),
            opcode: u32::from_le_bytes(bytes[4..8].try_into().ok()?),
            unique: u64::from_le_bytes(bytes[8..16].try_into().ok()?),
            nodeid: u64::from_le_bytes(bytes[16..24].try_into().ok()?),
            uid: u32::from_le_bytes(bytes[24..28].try_into().ok()?),
            gid: u32::from_le_bytes(bytes[28..32].try_into().ok()?),
            pid: u32::from_le_bytes(bytes[32..36].try_into().ok()?),
            padding: u32::from_le_bytes(bytes[36..40].try_into().ok()?),
        })
    }

    /// Converts the wire header into a typed [`FuseHeader`].
    pub const fn to_header(&self) -> FuseHeader {
        FuseHeader {
            unique: self.unique,
            nodeid: NodeId(self.nodeid),
            uid: self.uid,
            gid: self.gid,
            pid: self.pid,
        }
    }
}

/// Wire-level Linux `struct fuse_out_header` (16 bytes, little-endian).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FuseOutHeader {
    pub len: u32,
    /// `0` on success, or negative `-errno` on failure per Linux FUSE ABI.
    pub error: i32,
    pub unique: u64,
}

impl FuseOutHeader {
    /// Creates a success header for a reply with `payload_len` body bytes.
    pub fn ok(unique: u64, payload_len: usize) -> Self {
        Self {
            len: (FUSE_OUT_HEADER_SIZE.saturating_add(payload_len)) as u32,
            error: 0,
            unique,
        }
    }

    /// Creates an error header carrying `-errno` with zero payload bytes.
    pub fn err(unique: u64, errno: i32) -> Self {
        let neg = if errno > 0 { -errno } else { errno };
        Self {
            len: FUSE_OUT_HEADER_SIZE as u32,
            error: neg,
            unique,
        }
    }

    /// Encodes the header into 16 little-endian bytes.
    pub fn encode(&self) -> [u8; FUSE_OUT_HEADER_SIZE] {
        let mut buf = [0u8; FUSE_OUT_HEADER_SIZE];
        buf[0..4].copy_from_slice(&self.len.to_le_bytes());
        buf[4..8].copy_from_slice(&self.error.to_le_bytes());
        buf[8..16].copy_from_slice(&self.unique.to_le_bytes());
        buf
    }

    /// Decodes 16 little-endian bytes into a [`FuseOutHeader`].
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < FUSE_OUT_HEADER_SIZE {
            return None;
        }
        Some(Self {
            len: u32::from_le_bytes(bytes[0..4].try_into().ok()?),
            error: i32::from_le_bytes(bytes[4..8].try_into().ok()?),
            unique: u64::from_le_bytes(bytes[8..16].try_into().ok()?),
        })
    }
}

/// Kernel-authenticated FUSE request header passed to the dispatcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FuseHeader {
    pub unique: u64,
    pub nodeid: NodeId,
    pub uid: u32,
    pub gid: u32,
    pub pid: u32,
}

impl FuseHeader {
    pub const fn new(unique: u64, nodeid: NodeId, uid: u32, gid: u32, pid: u32) -> Self {
        Self {
            unique,
            nodeid,
            uid,
            gid,
            pid,
        }
    }
}

/// Typed FUSE request enum covering all supported metadata and data opcodes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FuseRequest {
    Lookup {
        parent: NodeId,
        name: String,
    },
    GetAttr {
        ino: NodeId,
    },
    SetAttr {
        ino: NodeId,
        attr: SetAttr,
    },
    ReadLink {
        ino: NodeId,
    },
    MkDir {
        parent: NodeId,
        name: String,
        mode: u32,
    },
    /// `CREATE`: makes a regular file and opens it.
    Create {
        parent: NodeId,
        name: String,
        mode: u32,
        flags: OpenFlags,
    },
    /// `MKNOD` of a regular file: makes it without leaving it open.
    MkNod {
        parent: NodeId,
        name: String,
        mode: u32,
    },
    Unlink {
        parent: NodeId,
        name: String,
    },
    RmDir {
        parent: NodeId,
        name: String,
    },
    Symlink {
        parent: NodeId,
        name: String,
        target: String,
    },
    Rename {
        parent: NodeId,
        name: String,
        new_parent: NodeId,
        new_name: String,
    },
    Link {
        ino: NodeId,
        new_parent: NodeId,
        new_name: String,
    },
    Open {
        ino: NodeId,
        flags: OpenFlags,
    },
    Read {
        ino: NodeId,
        fh: FileHandle,
        offset: u64,
        size: u32,
    },
    Write {
        ino: NodeId,
        fh: FileHandle,
        offset: u64,
        data: Vec<u8>,
        expected_generation: Option<u64>,
    },
    StatFs {
        ino: NodeId,
    },
    Release {
        ino: NodeId,
        fh: FileHandle,
    },
    Fsync {
        ino: NodeId,
        fh: FileHandle,
        datasync: bool,
    },
    OpenDir {
        ino: NodeId,
    },
    ReadDir {
        ino: NodeId,
        fh: FileHandle,
        offset: u64,
    },
    ReleaseDir {
        ino: NodeId,
        fh: FileHandle,
    },
    GetXattr {
        ino: NodeId,
        name: String,
    },
    ListXattr {
        ino: NodeId,
    },
    SetXattr {
        ino: NodeId,
        name: String,
        value: Vec<u8>,
    },
    RemoveXattr {
        ino: NodeId,
        name: String,
    },
    CopyFileRange {
        ino_in: NodeId,
        offset_in: u64,
        ino_out: NodeId,
        offset_out: u64,
        len: u64,
    },
    Flock {
        ino: NodeId,
        fh: FileHandle,
        op: FlockOp,
        non_blocking: bool,
    },
}

impl FuseRequest {
    /// Returns the FUSE ABI [`FuseOpcode`] corresponding to this request.
    pub const fn opcode(&self) -> FuseOpcode {
        match self {
            Self::Lookup { .. } => FuseOpcode::Lookup,
            Self::GetAttr { .. } => FuseOpcode::GetAttr,
            Self::SetAttr { .. } => FuseOpcode::SetAttr,
            Self::ReadLink { .. } => FuseOpcode::ReadLink,
            Self::MkDir { .. } => FuseOpcode::MkDir,
            Self::Create { .. } => FuseOpcode::Create,
            Self::MkNod { .. } => FuseOpcode::MkNod,
            Self::Unlink { .. } => FuseOpcode::Unlink,
            Self::RmDir { .. } => FuseOpcode::RmDir,
            Self::Symlink { .. } => FuseOpcode::Symlink,
            Self::Rename { .. } => FuseOpcode::Rename,
            Self::Link { .. } => FuseOpcode::Link,
            Self::Open { .. } => FuseOpcode::Open,
            Self::Read { .. } => FuseOpcode::Read,
            Self::Write { .. } => FuseOpcode::Write,
            Self::StatFs { .. } => FuseOpcode::StatFs,
            Self::Release { .. } => FuseOpcode::Release,
            Self::Fsync { .. } => FuseOpcode::Fsync,
            Self::OpenDir { .. } => FuseOpcode::OpenDir,
            Self::ReadDir { .. } => FuseOpcode::ReadDir,
            Self::ReleaseDir { .. } => FuseOpcode::ReleaseDir,
            Self::GetXattr { .. } => FuseOpcode::GetXattr,
            Self::ListXattr { .. } => FuseOpcode::ListXattr,
            Self::SetXattr { .. } => FuseOpcode::SetXattr,
            Self::RemoveXattr { .. } => FuseOpcode::RemoveXattr,
            Self::CopyFileRange { .. } => FuseOpcode::CopyFileRange,
            Self::Flock { .. } => FuseOpcode::Flock,
        }
    }
}

/// Typed reply payload for a successful FUSE request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FuseReply {
    Entry(FileAttr),
    /// Reply to `CREATE`: the new entry and its open handle.
    Created { attr: FileAttr, fh: FileHandle },
    Attr(FileAttr),
    ReadLink(String),
    Empty,
    Open { fh: FileHandle },
    Data(Vec<u8>),
    Written(u32),
    StatFs(StatFs),
    DirEntries(Vec<DirEntry>),
    XattrValue(Vec<u8>),
    XattrNames(Vec<String>),
    Copied(u64),
}

/// POSIX error representation returned by the FUSE dispatcher.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FuseErrno {
    /// Positive POSIX `errno` value (e.g., `ENOENT = 2`, `EACCES = 13`).
    pub errno: i32,
    pub message: String,
}

impl From<VfsError> for FuseErrno {
    fn from(err: VfsError) -> Self {
        Self {
            errno: err.errno(),
            message: err.to_string(),
        }
    }
}

/// Complete FUSE response pairing request `unique` ID with either a [`FuseReply`] or [`FuseErrno`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FuseResponse {
    pub unique: u64,
    pub result: Result<FuseReply, FuseErrno>,
}

impl FuseResponse {
    pub fn ok(unique: u64, reply: FuseReply) -> Self {
        Self {
            unique,
            result: Ok(reply),
        }
    }

    pub fn err(unique: u64, err: impl Into<FuseErrno>) -> Self {
        Self {
            unique,
            result: Err(err.into()),
        }
    }

    /// Builds the wire-level [`FuseOutHeader`] for this response.
    pub fn out_header(&self, payload_len: usize) -> FuseOutHeader {
        match &self.result {
            Ok(_) => FuseOutHeader::ok(self.unique, payload_len),
            Err(err) => FuseOutHeader::err(self.unique, err.errno),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_headers_round_trip() {
        let in_hdr = FuseInHeader {
            len: 56,
            opcode: FuseOpcode::Lookup.as_raw(),
            unique: 77,
            nodeid: 1,
            uid: 1000,
            gid: 1000,
            pid: 4321,
            padding: 0,
        };
        let encoded = in_hdr.encode();
        assert_eq!(FuseInHeader::decode(&encoded), Some(in_hdr));
        assert_eq!(FuseOpcode::from_raw(in_hdr.opcode), Some(FuseOpcode::Lookup));

        let out_ok = FuseOutHeader::ok(77, 128);
        assert_eq!(out_ok.len, (FUSE_OUT_HEADER_SIZE + 128) as u32);
        assert_eq!(out_ok.error, 0);
        assert_eq!(FuseOutHeader::decode(&out_ok.encode()), Some(out_ok));

        let out_err = FuseOutHeader::err(77, fusefs_vfs::error::errno::EACCES);
        assert_eq!(out_err.len, FUSE_OUT_HEADER_SIZE as u32);
        assert_eq!(out_err.error, -fusefs_vfs::error::errno::EACCES);
        assert_eq!(FuseOutHeader::decode(&out_err.encode()), Some(out_err));
    }
}
