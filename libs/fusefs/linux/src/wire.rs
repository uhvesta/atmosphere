//! Kernel FUSE wire codec: decodes per-opcode request bodies into [`FuseRequest`]
//! and encodes [`FuseReply`] values with the `<linux/fuse.h>` struct layouts.
//!
//! The codec is platform independent so it can be unit tested anywhere. It
//! speaks protocol 7.31 (`FUSE_KERNEL_MINOR_VERSION` below). The layouts used
//! are the ones that are stable from 7.9 up to and including 7.31:
//!
//! - `fuse_attr` is 88 bytes (`blksize` and `flags` at the end),
//! - `fuse_entry_out` is 128 bytes, `fuse_attr_out` is 104 bytes,
//! - `fuse_open_out` and `fuse_write_out` are 16 and 8 bytes,
//! - `fuse_kstatfs` is 80 bytes,
//! - `fuse_init_out` is 64 bytes,
//! - request bodies are the 7.9+ forms (for example `fuse_write_in` is 40 bytes
//!   and `fuse_setxattr_in` is 8 bytes because `FUSE_SETXATTR_EXT` is never
//!   negotiated).
//!
//! The server never sets `FUSE_POSIX_LOCKS`, `FUSE_FLOCK_LOCKS`,
//! `FUSE_ATOMIC_O_TRUNC` or `FUSE_DO_READDIRPLUS`, so the kernel handles locks
//! itself, truncates through SETATTR and only sends plain READDIR.
//!
//! `CREATE` and `MKNOD` (regular files only) go through
//! [`fusefs_vfs::Volume::create`]; other `MKNOD` types get `ENOSYS`.

use std::time::{SystemTime, UNIX_EPOCH};

use fusefs_vfs::{
    DirEntry, FileAttr, FileHandle, FileKind, NodeId, OpenFlags, SetAttr, StatFs, Timestamp,
};
use thiserror::Error;

use crate::fuse_abi::{
    FUSE_IN_HEADER_SIZE, FUSE_OUT_HEADER_SIZE, FuseErrno, FuseInHeader, FuseOpcode, FuseOutHeader,
    FuseReply, FuseRequest,
};

/// Protocol major version spoken with the kernel.
pub const FUSE_KERNEL_VERSION: u32 = 7;
/// Highest protocol minor version implemented.
pub const FUSE_KERNEL_MINOR_VERSION: u32 = 31;
/// Largest write payload advertised to the kernel in `fuse_init_out`.
pub const MAX_WRITE: u32 = 128 * 1024;

/// Size in bytes of `struct fuse_attr`.
pub const FUSE_ATTR_SIZE: usize = 88;
/// Size in bytes of `struct fuse_entry_out`.
pub const FUSE_ENTRY_OUT_SIZE: usize = 128;
/// Size in bytes of `struct fuse_attr_out`.
pub const FUSE_ATTR_OUT_SIZE: usize = 104;
/// Size in bytes of `struct fuse_init_out` (7.23 and newer).
pub const FUSE_INIT_OUT_SIZE: usize = 64;
/// Size in bytes of the fixed part of `struct fuse_dirent`.
pub const FUSE_DIRENT_HEADER_SIZE: usize = 24;

const FUSE_BIG_WRITES: u32 = 1 << 5;
const FOPEN_DIRECT_IO: u32 = 1;
/// Seconds the kernel may cache entries and attributes.
const VALID_SECS: u64 = 1;

const EIO: i32 = 5;
const EINVAL: i32 = 22;
const ENOSYS: i32 = 38;
const ERANGE: i32 = 34;

const S_IFMT: u32 = 0o170000;
const S_IFREG: u32 = 0o100000;
const S_IFDIR: u32 = 0o040000;
const S_IFLNK: u32 = 0o120000;
const DT_DIR: u32 = 4;
const DT_REG: u32 = 8;
const DT_LNK: u32 = 10;

const O_ACCMODE: u32 = 0o3;
const O_EXCL: u32 = 0o200;
const O_TRUNC: u32 = 0o1000;
const O_APPEND: u32 = 0o2000;

const FATTR_MODE: u32 = 1 << 0;
const FATTR_UID: u32 = 1 << 1;
const FATTR_GID: u32 = 1 << 2;
const FATTR_SIZE: u32 = 1 << 3;
const FATTR_ATIME: u32 = 1 << 4;
const FATTR_MTIME: u32 = 1 << 5;
const FATTR_ATIME_NOW: u32 = 1 << 7;
const FATTR_MTIME_NOW: u32 = 1 << 8;

/// A request frame that could not be decoded.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum WireError {
    /// The frame is shorter than the opcode's fixed body.
    #[error(
        "FUSE request opcode {opcode} is truncated: it needs {needed} bytes of body but carries {got}. The kernel and this helper disagree on the FUSE protocol version; restart the helper."
    )]
    Truncated {
        opcode: u32,
        needed: usize,
        got: usize,
    },
    /// A name in the request is not valid UTF-8.
    #[error(
        "FUSE request opcode {opcode} carries a name that is not valid UTF-8. Only UTF-8 names can be served; rename the entry."
    )]
    NotUtf8 { opcode: u32 },
    /// The frame's header is missing or its length field disagrees with the frame.
    #[error(
        "FUSE frame has a bad header: {got} bytes read, header says {declared}. Remount the filesystem."
    )]
    BadHeader { got: usize, declared: u32 },
}

impl WireError {
    /// The errno to answer the kernel with.
    pub const fn errno(&self) -> i32 {
        EINVAL
    }
}

/// The kernel's `FUSE_INIT` request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InitIn {
    pub major: u32,
    pub minor: u32,
    pub max_readahead: u32,
    pub flags: u32,
}

/// How a reply must be shaped, which depends on the request that produced it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplyShape {
    kind: ShapeKind,
    size: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShapeKind {
    Plain,
    OpenFile,
    OpenDir,
    Read,
    ReadDir,
    Xattr,
}

impl ReplyShape {
    const fn plain() -> Self {
        Self {
            kind: ShapeKind::Plain,
            size: 0,
        }
    }
}

/// What a request frame asks the server to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decoded {
    /// `FUSE_INIT`: answer with [`encode_init_reply`].
    Init(InitIn),
    /// Serve through the dispatcher and answer with [`encode_reply`].
    Dispatch(FuseRequest, ReplyShape),
    /// Answer success with an empty body (FLUSH, ACCESS, DESTROY).
    Ack,
    /// FORGET, BATCH_FORGET and INTERRUPT: the kernel expects no reply.
    NoReply,
    /// Opcode not supported: answer `ENOSYS`.
    Unsupported(u32),
}

struct Reader<'a> {
    opcode: u32,
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(opcode: u32, buf: &'a [u8]) -> Self {
        Self { opcode, buf, pos: 0 }
    }

    fn truncated(&self, needed: usize) -> WireError {
        WireError::Truncated {
            opcode: self.opcode,
            needed: self.pos.saturating_add(needed),
            got: self.buf.len(),
        }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], WireError> {
        let end = self.pos.checked_add(n).ok_or_else(|| self.truncated(n))?;
        let slice = self.buf.get(self.pos..end).ok_or_else(|| self.truncated(n))?;
        self.pos = end;
        Ok(slice)
    }

    fn u32(&mut self) -> Result<u32, WireError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self) -> Result<u64, WireError> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    fn skip(&mut self, n: usize) -> Result<(), WireError> {
        self.take(n).map(|_| ())
    }

    fn rest(&mut self) -> &'a [u8] {
        let rest = self.buf.get(self.pos..).unwrap_or(&[]);
        self.pos = self.buf.len();
        rest
    }

    /// Reads a NUL-terminated name.
    fn name(&mut self) -> Result<String, WireError> {
        let rest = self.buf.get(self.pos..).unwrap_or(&[]);
        let end = rest
            .iter()
            .position(|&b| b == 0)
            .ok_or_else(|| self.truncated(rest.len() + 1))?;
        let name = std::str::from_utf8(&rest[..end])
            .map_err(|_| WireError::NotUtf8 { opcode: self.opcode })?
            .to_string();
        self.pos += end + 1;
        Ok(name)
    }
}

fn open_flags(raw: u32) -> OpenFlags {
    let acc = raw & O_ACCMODE;
    OpenFlags {
        read: acc == 0 || acc == 2,
        write: acc == 1 || acc == 2,
        append: raw & O_APPEND != 0,
        truncate: raw & O_TRUNC != 0,
        create: false,
        exclusive: false,
    }
}

fn now_timestamp() -> Timestamp {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => Timestamp {
            secs: d.as_secs() as i64,
            nanos: d.subsec_nanos(),
        },
        Err(_) => Timestamp::ZERO,
    }
}

/// Splits `frame` into its header and decoded request.
pub fn decode_frame(frame: &[u8]) -> Result<(FuseInHeader, Decoded), WireError> {
    let header = FuseInHeader::decode(frame).ok_or(WireError::BadHeader {
        got: frame.len(),
        declared: 0,
    })?;
    let declared = header.len as usize;
    if declared < FUSE_IN_HEADER_SIZE || declared > frame.len() {
        return Err(WireError::BadHeader {
            got: frame.len(),
            declared: header.len,
        });
    }
    let body = &frame[FUSE_IN_HEADER_SIZE..declared];
    let decoded = decode_body(&header, body)?;
    Ok((header, decoded))
}

fn decode_body(h: &FuseInHeader, body: &[u8]) -> Result<Decoded, WireError> {
    let node = NodeId(h.nodeid);
    let mut r = Reader::new(h.opcode, body);
    let Some(op) = FuseOpcode::from_raw(h.opcode) else {
        return Ok(Decoded::Unsupported(h.opcode));
    };
    let plain = ReplyShape::plain;
    let req = |request| Ok(Decoded::Dispatch(request, plain()));
    match op {
        FuseOpcode::Init => {
            let major = r.u32()?;
            let minor = r.u32()?;
            let max_readahead = r.u32()?;
            let flags = r.u32()?;
            Ok(Decoded::Init(InitIn {
                major,
                minor,
                max_readahead,
                flags,
            }))
        }
        FuseOpcode::Forget | FuseOpcode::BatchForget | FuseOpcode::Interrupt => {
            Ok(Decoded::NoReply)
        }
        FuseOpcode::Flush | FuseOpcode::Access | FuseOpcode::Destroy => Ok(Decoded::Ack),
        FuseOpcode::Lookup => req(FuseRequest::Lookup {
            parent: node,
            name: r.name()?,
        }),
        FuseOpcode::GetAttr => req(FuseRequest::GetAttr { ino: node }),
        FuseOpcode::SetAttr => {
            let valid = r.u32()?;
            r.skip(4)?;
            r.skip(8)?; // fh
            let size = r.u64()?;
            r.skip(8)?; // lock_owner
            let atime = r.u64()?;
            let mtime = r.u64()?;
            r.skip(8)?; // ctime
            let atimensec = r.u32()?;
            let mtimensec = r.u32()?;
            r.skip(4)?; // ctimensec
            let mode = r.u32()?;
            r.skip(4)?;
            let uid = r.u32()?;
            let gid = r.u32()?;
            let time = |secs: u64, nanos: u32| Timestamp {
                secs: secs as i64,
                nanos,
            };
            let attr = SetAttr {
                mode: (valid & FATTR_MODE != 0).then_some(mode & 0o7777),
                uid: (valid & FATTR_UID != 0).then_some(uid),
                gid: (valid & FATTR_GID != 0).then_some(gid),
                size: (valid & FATTR_SIZE != 0).then_some(size),
                atime: if valid & FATTR_ATIME_NOW != 0 {
                    Some(now_timestamp())
                } else {
                    (valid & FATTR_ATIME != 0).then(|| time(atime, atimensec))
                },
                mtime: if valid & FATTR_MTIME_NOW != 0 {
                    Some(now_timestamp())
                } else {
                    (valid & FATTR_MTIME != 0).then(|| time(mtime, mtimensec))
                },
                expected_generation: None,
            };
            req(FuseRequest::SetAttr { ino: node, attr })
        }
        FuseOpcode::ReadLink => req(FuseRequest::ReadLink { ino: node }),
        FuseOpcode::Symlink => {
            let name = r.name()?;
            let target = r.name()?;
            req(FuseRequest::Symlink {
                parent: node,
                name,
                target,
            })
        }
        FuseOpcode::MkDir => {
            let mode = r.u32()?;
            r.skip(4)?; // umask
            req(FuseRequest::MkDir {
                parent: node,
                name: r.name()?,
                mode: mode & 0o7777,
            })
        }
        FuseOpcode::Unlink => req(FuseRequest::Unlink {
            parent: node,
            name: r.name()?,
        }),
        FuseOpcode::RmDir => req(FuseRequest::RmDir {
            parent: node,
            name: r.name()?,
        }),
        FuseOpcode::Rename | FuseOpcode::Rename2 => {
            let new_parent = NodeId(r.u64()?);
            if op == FuseOpcode::Rename2 {
                let flags = r.u32()?;
                r.skip(4)?;
                if flags != 0 {
                    // RENAME_NOREPLACE / EXCHANGE / WHITEOUT are not supported.
                    return Ok(Decoded::Unsupported(h.opcode));
                }
            }
            let name = r.name()?;
            let new_name = r.name()?;
            req(FuseRequest::Rename {
                parent: node,
                name,
                new_parent,
                new_name,
            })
        }
        FuseOpcode::Link => {
            let old = NodeId(r.u64()?);
            req(FuseRequest::Link {
                ino: old,
                new_parent: node,
                new_name: r.name()?,
            })
        }
        FuseOpcode::Open | FuseOpcode::OpenDir => {
            let flags = r.u32()?;
            if op == FuseOpcode::OpenDir {
                Ok(Decoded::Dispatch(
                    FuseRequest::OpenDir { ino: node },
                    ReplyShape {
                        kind: ShapeKind::OpenDir,
                        size: 0,
                    },
                ))
            } else {
                Ok(Decoded::Dispatch(
                    FuseRequest::Open {
                        ino: node,
                        flags: open_flags(flags),
                    },
                    ReplyShape {
                        kind: ShapeKind::OpenFile,
                        size: 0,
                    },
                ))
            }
        }
        FuseOpcode::Read | FuseOpcode::ReadDir => {
            let fh = FileHandle(r.u64()?);
            let offset = r.u64()?;
            let size = r.u32()?;
            if op == FuseOpcode::Read {
                Ok(Decoded::Dispatch(
                    FuseRequest::Read {
                        ino: node,
                        fh,
                        offset,
                        size,
                    },
                    ReplyShape {
                        kind: ShapeKind::Read,
                        size,
                    },
                ))
            } else {
                Ok(Decoded::Dispatch(
                    FuseRequest::ReadDir {
                        ino: node,
                        fh,
                        offset,
                    },
                    ReplyShape {
                        kind: ShapeKind::ReadDir,
                        size,
                    },
                ))
            }
        }
        FuseOpcode::Write => {
            let fh = FileHandle(r.u64()?);
            let offset = r.u64()?;
            let size = r.u32()? as usize;
            r.skip(4 + 8 + 4 + 4)?; // write_flags, lock_owner, flags, padding
            let data = r.rest();
            let data = data.get(..size).ok_or(WireError::Truncated {
                opcode: h.opcode,
                needed: FUSE_IN_HEADER_SIZE + 40 + size,
                got: FUSE_IN_HEADER_SIZE + body.len(),
            })?;
            req(FuseRequest::Write {
                ino: node,
                fh,
                offset,
                data: data.to_vec(),
                expected_generation: None,
            })
        }
        FuseOpcode::StatFs => req(FuseRequest::StatFs { ino: node }),
        FuseOpcode::Release | FuseOpcode::ReleaseDir => {
            let fh = FileHandle(r.u64()?);
            if op == FuseOpcode::Release {
                req(FuseRequest::Release { ino: node, fh })
            } else {
                req(FuseRequest::ReleaseDir { ino: node, fh })
            }
        }
        FuseOpcode::Fsync => {
            let fh = FileHandle(r.u64()?);
            let flags = r.u32()?;
            req(FuseRequest::Fsync {
                ino: node,
                fh,
                datasync: flags & 1 != 0,
            })
        }
        FuseOpcode::GetXattr | FuseOpcode::ListXattr => {
            let size = r.u32()?;
            r.skip(4)?;
            let shape = ReplyShape {
                kind: ShapeKind::Xattr,
                size,
            };
            if op == FuseOpcode::GetXattr {
                Ok(Decoded::Dispatch(
                    FuseRequest::GetXattr {
                        ino: node,
                        name: r.name()?,
                    },
                    shape,
                ))
            } else {
                Ok(Decoded::Dispatch(FuseRequest::ListXattr { ino: node }, shape))
            }
        }
        FuseOpcode::SetXattr => {
            let size = r.u32()? as usize;
            r.skip(4)?; // flags
            let name = r.name()?;
            let value = r.rest();
            let value = value.get(..size).ok_or(WireError::Truncated {
                opcode: h.opcode,
                needed: size,
                got: value.len(),
            })?;
            req(FuseRequest::SetXattr {
                ino: node,
                name,
                value: value.to_vec(),
            })
        }
        FuseOpcode::RemoveXattr => req(FuseRequest::RemoveXattr {
            ino: node,
            name: r.name()?,
        }),
        FuseOpcode::CopyFileRange => {
            r.skip(8)?; // fh_in
            let offset_in = r.u64()?;
            let ino_out = NodeId(r.u64()?);
            r.skip(8)?; // fh_out
            let offset_out = r.u64()?;
            let len = r.u64()?;
            req(FuseRequest::CopyFileRange {
                ino_in: node,
                offset_in,
                ino_out,
                offset_out,
                len,
            })
        }
        FuseOpcode::Create => {
            let flags = r.u32()?;
            let mode = r.u32()?;
            r.skip(8)?; // umask, open_flags
            let mut open = open_flags(flags);
            open.create = true;
            open.exclusive = flags & O_EXCL != 0;
            Ok(Decoded::Dispatch(
                FuseRequest::Create {
                    parent: node,
                    name: r.name()?,
                    mode: mode & 0o7777,
                    flags: open,
                },
                ReplyShape {
                    kind: ShapeKind::OpenFile,
                    size: 0,
                },
            ))
        }
        FuseOpcode::MkNod => {
            let mode = r.u32()?;
            r.skip(12)?; // rdev, umask, padding
            if mode & S_IFMT != S_IFREG && mode & S_IFMT != 0 {
                return Ok(Decoded::Unsupported(h.opcode));
            }
            req(FuseRequest::MkNod {
                parent: node,
                name: r.name()?,
                mode: mode & 0o7777,
            })
        }
        // Locking is handled by the kernel because the lock flags are never negotiated.
        FuseOpcode::Flock => Ok(Decoded::Unsupported(h.opcode)),
    }
}

fn header_bytes(unique: u64, error: i32, body_len: usize) -> [u8; FUSE_OUT_HEADER_SIZE] {
    FuseOutHeader {
        len: (FUSE_OUT_HEADER_SIZE + body_len) as u32,
        error,
        unique,
    }
    .encode()
}

fn frame(unique: u64, body: Vec<u8>) -> Vec<u8> {
    let mut out = Vec::with_capacity(FUSE_OUT_HEADER_SIZE + body.len());
    out.extend_from_slice(&header_bytes(unique, 0, body.len()));
    out.extend_from_slice(&body);
    out
}

/// A reply frame carrying only a header and a negative errno.
pub fn encode_error(unique: u64, errno: i32) -> Vec<u8> {
    header_bytes(unique, if errno > 0 { -errno } else { errno }, 0).to_vec()
}

/// A success reply with an empty body.
pub fn encode_ack(unique: u64) -> Vec<u8> {
    frame(unique, Vec::new())
}

/// The `ENOSYS` reply for an opcode this server does not implement.
pub fn encode_unsupported(unique: u64) -> Vec<u8> {
    encode_error(unique, ENOSYS)
}

/// The reply to `FUSE_INIT`: major 7, minor `min(kernel, 31)`.
pub fn encode_init_reply(unique: u64, init: &InitIn) -> Vec<u8> {
    let minor = init.minor.min(FUSE_KERNEL_MINOR_VERSION);
    let mut b = Vec::with_capacity(FUSE_INIT_OUT_SIZE);
    b.extend_from_slice(&FUSE_KERNEL_VERSION.to_le_bytes());
    b.extend_from_slice(&minor.to_le_bytes());
    b.extend_from_slice(&init.max_readahead.to_le_bytes());
    b.extend_from_slice(&(init.flags & FUSE_BIG_WRITES).to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes()); // max_background
    b.extend_from_slice(&12u16.to_le_bytes()); // congestion_threshold
    b.extend_from_slice(&MAX_WRITE.to_le_bytes());
    b.extend_from_slice(&1u32.to_le_bytes()); // time_gran
    b.extend_from_slice(&0u16.to_le_bytes()); // max_pages (default)
    b.extend_from_slice(&0u16.to_le_bytes()); // map_alignment
    b.extend_from_slice(&[0u8; 32]); // unused
    frame(unique, b)
}

fn kind_bits(kind: FileKind) -> u32 {
    match kind {
        FileKind::Regular => S_IFREG,
        FileKind::Directory => S_IFDIR,
        FileKind::Symlink => S_IFLNK,
    }
}

fn dirent_type(kind: FileKind) -> u32 {
    match kind {
        FileKind::Regular => DT_REG,
        FileKind::Directory => DT_DIR,
        FileKind::Symlink => DT_LNK,
    }
}

fn put_attr(b: &mut Vec<u8>, a: &FileAttr, owner: (u32, u32)) {
    let secs = |t: Timestamp| t.secs.max(0) as u64;
    b.extend_from_slice(&a.ino.0.to_le_bytes());
    b.extend_from_slice(&a.size.to_le_bytes());
    b.extend_from_slice(&a.blocks.to_le_bytes());
    b.extend_from_slice(&secs(a.atime).to_le_bytes());
    b.extend_from_slice(&secs(a.mtime).to_le_bytes());
    b.extend_from_slice(&secs(a.ctime).to_le_bytes());
    b.extend_from_slice(&a.atime.nanos.to_le_bytes());
    b.extend_from_slice(&a.mtime.nanos.to_le_bytes());
    b.extend_from_slice(&a.ctime.nanos.to_le_bytes());
    b.extend_from_slice(&(kind_bits(a.kind) | (a.mode & 0o7777)).to_le_bytes());
    b.extend_from_slice(&a.nlink.max(1).to_le_bytes());
    b.extend_from_slice(&owner.0.to_le_bytes());
    b.extend_from_slice(&owner.1.to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes()); // rdev
    b.extend_from_slice(&4096u32.to_le_bytes()); // blksize
    b.extend_from_slice(&0u32.to_le_bytes()); // flags
}

fn entry_body(a: &FileAttr, owner: (u32, u32)) -> Vec<u8> {
    let mut b = Vec::with_capacity(FUSE_ENTRY_OUT_SIZE);
    b.extend_from_slice(&a.ino.0.to_le_bytes());
    // The kernel treats a changed generation for a known nodeid as a different
    // inode and marks the old one bad, so the generation stays 0.
    b.extend_from_slice(&0u64.to_le_bytes());
    b.extend_from_slice(&VALID_SECS.to_le_bytes());
    b.extend_from_slice(&VALID_SECS.to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    put_attr(&mut b, a, owner);
    b
}

fn attr_body(a: &FileAttr, owner: (u32, u32)) -> Vec<u8> {
    let mut b = Vec::with_capacity(FUSE_ATTR_OUT_SIZE);
    b.extend_from_slice(&VALID_SECS.to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    put_attr(&mut b, a, owner);
    b
}

fn statfs_body(s: &StatFs) -> Vec<u8> {
    let mut b = Vec::with_capacity(80);
    for v in [s.blocks, s.bfree, s.bavail, s.files, s.ffree] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    for v in [s.bsize, s.namelen, s.frsize, 0u32] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.extend_from_slice(&[0u8; 24]);
    b
}

/// Encodes entries as `fuse_dirent` records padded to 8 bytes, stopping before
/// the record that would push the buffer past `limit`.
fn dirents_body(entries: &[DirEntry], limit: usize) -> Vec<u8> {
    let mut b = Vec::new();
    for e in entries {
        let name = e.name.as_bytes();
        let padded = (FUSE_DIRENT_HEADER_SIZE + name.len()).div_ceil(8) * 8;
        if b.len() + padded > limit {
            break;
        }
        b.extend_from_slice(&e.ino.0.to_le_bytes());
        b.extend_from_slice(&e.offset.to_le_bytes());
        b.extend_from_slice(&(name.len() as u32).to_le_bytes());
        b.extend_from_slice(&dirent_type(e.kind).to_le_bytes());
        b.extend_from_slice(name);
        b.resize(b.len() + (padded - FUSE_DIRENT_HEADER_SIZE - name.len()), 0);
    }
    b
}

fn xattr_reply(unique: u64, shape: &ReplyShape, data: Vec<u8>) -> Vec<u8> {
    if shape.size == 0 {
        let mut b = Vec::with_capacity(8);
        b.extend_from_slice(&(data.len() as u32).to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        frame(unique, b)
    } else if data.len() > shape.size as usize {
        encode_error(unique, ERANGE)
    } else {
        frame(unique, data)
    }
}

/// Encodes the dispatcher's answer to a request as a full reply frame.
///
/// `owner` is the `(uid, gid)` reported for every node: the mount is private to
/// one user and the kernel checks permissions against these ids
/// (`default_permissions`).
pub fn encode_reply(
    unique: u64,
    shape: &ReplyShape,
    owner: (u32, u32),
    result: Result<FuseReply, FuseErrno>,
) -> Vec<u8> {
    let reply = match result {
        Ok(reply) => reply,
        Err(err) => return encode_error(unique, if err.errno > 0 { err.errno } else { EIO }),
    };
    match reply {
        FuseReply::Entry(a) => frame(unique, entry_body(&a, owner)),
        FuseReply::Created { attr, fh } => {
            let mut b = entry_body(&attr, owner);
            b.extend_from_slice(&fh.0.to_le_bytes());
            b.extend_from_slice(&FOPEN_DIRECT_IO.to_le_bytes());
            b.extend_from_slice(&0u32.to_le_bytes());
            frame(unique, b)
        }
        FuseReply::Attr(a) => frame(unique, attr_body(&a, owner)),
        FuseReply::ReadLink(t) => frame(unique, t.into_bytes()),
        FuseReply::Empty => encode_ack(unique),
        FuseReply::Open { fh } => {
            let flags = if shape.kind == ShapeKind::OpenFile {
                FOPEN_DIRECT_IO
            } else {
                0
            };
            let mut b = Vec::with_capacity(16);
            b.extend_from_slice(&fh.0.to_le_bytes());
            b.extend_from_slice(&flags.to_le_bytes());
            b.extend_from_slice(&0u32.to_le_bytes());
            frame(unique, b)
        }
        FuseReply::Data(mut d) => {
            if shape.kind == ShapeKind::Read {
                d.truncate(shape.size as usize);
            }
            frame(unique, d)
        }
        FuseReply::Written(n) => write_out(unique, n),
        FuseReply::Copied(n) => write_out(unique, n.min(u64::from(u32::MAX)) as u32),
        FuseReply::StatFs(s) => frame(unique, statfs_body(&s)),
        FuseReply::DirEntries(entries) => {
            let limit = if shape.kind == ShapeKind::ReadDir {
                shape.size as usize
            } else {
                usize::MAX
            };
            frame(unique, dirents_body(&entries, limit))
        }
        FuseReply::XattrValue(v) => xattr_reply(unique, shape, v),
        FuseReply::XattrNames(names) => {
            let mut data = Vec::new();
            for n in names {
                data.extend_from_slice(n.as_bytes());
                data.push(0);
            }
            xattr_reply(unique, shape, data)
        }
    }
}

fn write_out(unique: u64, n: u32) -> Vec<u8> {
    let mut b = Vec::with_capacity(8);
    b.extend_from_slice(&n.to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    frame(unique, b)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fusefs_vfs::{InMemoryVolume, ROOT_NODE_ID};

    use super::*;
    use crate::dispatcher::FuseDispatcher;
    use crate::fuse_abi::FuseHeader;

    fn frame_for(opcode: FuseOpcode, nodeid: u64, body: &[u8]) -> Vec<u8> {
        raw_frame(opcode.as_raw(), nodeid, body)
    }

    fn raw_frame(opcode: u32, nodeid: u64, body: &[u8]) -> Vec<u8> {
        let header = FuseInHeader {
            len: (FUSE_IN_HEADER_SIZE + body.len()) as u32,
            opcode,
            unique: 42,
            nodeid,
            uid: 1000,
            gid: 1000,
            pid: 77,
            padding: 0,
        };
        let mut out = header.encode().to_vec();
        out.extend_from_slice(body);
        out
    }

    fn u32_at(b: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
    }

    fn u64_at(b: &[u8], at: usize) -> u64 {
        u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
    }

    fn dispatcher() -> FuseDispatcher<InMemoryVolume> {
        let vol = InMemoryVolume::builder()
            .file("/hello.txt", b"world")
            .file("/a-longer-name.txt", b"x")
            .build()
            .unwrap();
        FuseDispatcher::for_owner(Arc::new(vol), 1000)
    }

    /// Decodes `frame`, runs it through the dispatcher and returns the reply frame.
    fn roundtrip(d: &FuseDispatcher<InMemoryVolume>, frame: &[u8]) -> Option<Vec<u8>> {
        let (h, decoded) = decode_frame(frame).unwrap();
        match decoded {
            Decoded::Dispatch(request, shape) => {
                let header = FuseHeader::new(h.unique, NodeId(h.nodeid), h.uid, h.gid, h.pid);
                let response = d.dispatch(&header, request);
                Some(encode_reply(h.unique, &shape, (h.uid, h.gid), response.result))
            }
            Decoded::Init(init) => Some(encode_init_reply(h.unique, &init)),
            Decoded::Ack => Some(encode_ack(h.unique)),
            Decoded::NoReply => None,
            Decoded::Unsupported(_) => Some(encode_unsupported(h.unique)),
        }
    }

    #[test]
    fn init_reply_negotiates_minor_and_max_write() {
        let mut body = Vec::new();
        for v in [7u32, 40, 131072, 0xffff_ffff] {
            body.extend_from_slice(&v.to_le_bytes());
        }
        body.extend_from_slice(&[0u8; 48]);
        let reply = roundtrip(&dispatcher(), &frame_for(FuseOpcode::Init, 0, &body)).unwrap();
        let out = FuseOutHeader::decode(&reply).unwrap();
        assert_eq!(out.error, 0);
        assert_eq!(out.unique, 42);
        assert_eq!(out.len as usize, reply.len());
        assert_eq!(reply.len(), FUSE_OUT_HEADER_SIZE + FUSE_INIT_OUT_SIZE);
        let b = &reply[FUSE_OUT_HEADER_SIZE..];
        assert_eq!(u32_at(b, 0), 7);
        assert_eq!(u32_at(b, 4), FUSE_KERNEL_MINOR_VERSION);
        assert_eq!(u32_at(b, 8), 131072);
        assert_eq!(u32_at(b, 12), FUSE_BIG_WRITES);
        assert_eq!(u32_at(b, 20), MAX_WRITE);

        // An older kernel gets its own minor back.
        let mut old = Vec::new();
        for v in [7u32, 26, 4096, 0] {
            old.extend_from_slice(&v.to_le_bytes());
        }
        let reply = roundtrip(&dispatcher(), &frame_for(FuseOpcode::Init, 0, &old)).unwrap();
        assert_eq!(u32_at(&reply[FUSE_OUT_HEADER_SIZE..], 4), 26);
    }

    #[test]
    fn lookup_and_getattr_round_trip() {
        let d = dispatcher();
        let reply = roundtrip(&d, &frame_for(FuseOpcode::Lookup, 1, b"hello.txt\0")).unwrap();
        assert_eq!(FuseOutHeader::decode(&reply).unwrap().error, 0);
        assert_eq!(reply.len(), FUSE_OUT_HEADER_SIZE + FUSE_ENTRY_OUT_SIZE);
        let b = &reply[FUSE_OUT_HEADER_SIZE..];
        let nodeid = u64_at(b, 0);
        assert_ne!(nodeid, 0);
        assert_eq!(u64_at(b, 8), 0, "generation stays zero");
        assert_eq!(u64_at(b, 40 + 8), 5, "attr.size");
        let mode = u32_at(b, 40 + 60);
        assert_eq!(mode & 0o170000, S_IFREG);
        assert_eq!(u32_at(b, 40 + 68), 1000, "attr.uid is the owner");

        let mut getattr_body = vec![0u8; 16];
        getattr_body[0] = 0;
        let reply = roundtrip(&d, &frame_for(FuseOpcode::GetAttr, nodeid, &getattr_body)).unwrap();
        assert_eq!(FuseOutHeader::decode(&reply).unwrap().error, 0);
        assert_eq!(reply.len(), FUSE_OUT_HEADER_SIZE + FUSE_ATTR_OUT_SIZE);
        let b = &reply[FUSE_OUT_HEADER_SIZE..];
        assert_eq!(u64_at(b, 16 + 8), 5);
        assert_eq!(u64_at(b, 16), nodeid);

        let missing = roundtrip(&d, &frame_for(FuseOpcode::Lookup, 1, b"nope\0")).unwrap();
        assert_eq!(
            FuseOutHeader::decode(&missing).unwrap().error,
            -fusefs_vfs::error::errno::ENOENT
        );
    }

    #[test]
    fn readdir_entries_are_aligned_and_decode_back() {
        let d = dispatcher();
        let mut open_body = Vec::new();
        open_body.extend_from_slice(&0u32.to_le_bytes());
        open_body.extend_from_slice(&0u32.to_le_bytes());
        let reply = roundtrip(&d, &frame_for(FuseOpcode::OpenDir, 1, &open_body)).unwrap();
        let fh = u64_at(&reply[FUSE_OUT_HEADER_SIZE..], 0);

        let mut read_body = Vec::new();
        read_body.extend_from_slice(&fh.to_le_bytes());
        read_body.extend_from_slice(&0u64.to_le_bytes());
        read_body.extend_from_slice(&4096u32.to_le_bytes());
        read_body.extend_from_slice(&[0u8; 20]);
        let reply = roundtrip(&d, &frame_for(FuseOpcode::ReadDir, 1, &read_body)).unwrap();
        let body = &reply[FUSE_OUT_HEADER_SIZE..];
        assert_eq!(body.len() % 8, 0);

        let mut names = Vec::new();
        let mut at = 0;
        while at < body.len() {
            assert_eq!(at % 8, 0);
            let namelen = u32_at(body, at + 16) as usize;
            names.push(
                std::str::from_utf8(&body[at + 24..at + 24 + namelen])
                    .unwrap()
                    .to_string(),
            );
            at += (24 + namelen).div_ceil(8) * 8;
        }
        assert_eq!(at, body.len());
        for want in [".", "..", "hello.txt", "a-longer-name.txt"] {
            assert!(names.iter().any(|n| n == want), "{want} missing in {names:?}");
        }

        // A small buffer cuts the listing at a record boundary.
        let mut small = read_body.clone();
        small[16..20].copy_from_slice(&64u32.to_le_bytes());
        let reply = roundtrip(&d, &frame_for(FuseOpcode::ReadDir, 1, &small)).unwrap();
        let body = &reply[FUSE_OUT_HEADER_SIZE..];
        assert!(body.len() <= 64);
        assert_eq!(body.len() % 8, 0);
    }

    #[test]
    fn unknown_and_unsupported_opcodes_get_enosys() {
        let d = dispatcher();
        for opcode in [9999u32, FuseOpcode::Flock.as_raw()] {
            let reply = roundtrip(&d, &raw_frame(opcode, 1, &[0u8; 16])).unwrap();
            assert_eq!(FuseOutHeader::decode(&reply).unwrap().error, -ENOSYS);
            assert_eq!(reply.len(), FUSE_OUT_HEADER_SIZE);
        }
    }

    #[test]
    fn forget_gets_no_reply() {
        let d = dispatcher();
        let forget = frame_for(FuseOpcode::Forget, 5, &1u64.to_le_bytes());
        assert!(roundtrip(&d, &forget).is_none());
        let mut batch = Vec::new();
        batch.extend_from_slice(&1u32.to_le_bytes());
        batch.extend_from_slice(&0u32.to_le_bytes());
        batch.extend_from_slice(&5u64.to_le_bytes());
        batch.extend_from_slice(&1u64.to_le_bytes());
        assert!(roundtrip(&d, &frame_for(FuseOpcode::BatchForget, 0, &batch)).is_none());
    }

    #[test]
    fn truncated_body_is_a_decode_error() {
        let frame = frame_for(FuseOpcode::Read, 2, &[0u8; 4]);
        assert!(matches!(
            decode_frame(&frame),
            Err(WireError::Truncated { .. })
        ));
    }

    #[test]
    fn write_and_read_back_through_the_codec() {
        let d = dispatcher();
        let reply = roundtrip(&d, &frame_for(FuseOpcode::Lookup, 1, b"hello.txt\0")).unwrap();
        let nodeid = u64_at(&reply[FUSE_OUT_HEADER_SIZE..], 0);

        let mut open = Vec::new();
        open.extend_from_slice(&2u32.to_le_bytes()); // O_RDWR
        open.extend_from_slice(&0u32.to_le_bytes());
        let reply = roundtrip(&d, &frame_for(FuseOpcode::Open, nodeid, &open)).unwrap();
        assert_eq!(FuseOutHeader::decode(&reply).unwrap().error, 0);
        let fh = u64_at(&reply[FUSE_OUT_HEADER_SIZE..], 0);

        let mut write = Vec::new();
        write.extend_from_slice(&fh.to_le_bytes());
        write.extend_from_slice(&0u64.to_le_bytes());
        write.extend_from_slice(&3u32.to_le_bytes());
        write.extend_from_slice(&[0u8; 20]);
        write.extend_from_slice(b"abc");
        let reply = roundtrip(&d, &frame_for(FuseOpcode::Write, nodeid, &write)).unwrap();
        assert_eq!(FuseOutHeader::decode(&reply).unwrap().error, 0);
        assert_eq!(u32_at(&reply[FUSE_OUT_HEADER_SIZE..], 0), 3);

        let mut read = Vec::new();
        read.extend_from_slice(&fh.to_le_bytes());
        read.extend_from_slice(&0u64.to_le_bytes());
        read.extend_from_slice(&5u32.to_le_bytes());
        read.extend_from_slice(&[0u8; 20]);
        let reply = roundtrip(&d, &frame_for(FuseOpcode::Read, nodeid, &read)).unwrap();
        assert_eq!(&reply[FUSE_OUT_HEADER_SIZE..], b"abcld");
    }

    #[test]
    fn create_write_read_and_exclusive_conflict() {
        let d = dispatcher();
        let mut create = Vec::new();
        create.extend_from_slice(&(0o1u32 | 0o100).to_le_bytes()); // O_WRONLY|O_CREAT
        create.extend_from_slice(&0o100644u32.to_le_bytes());
        create.extend_from_slice(&[0u8; 8]);
        create.extend_from_slice(b"fresh.txt\0");
        let reply = roundtrip(&d, &frame_for(FuseOpcode::Create, 1, &create)).unwrap();
        assert_eq!(FuseOutHeader::decode(&reply).unwrap().error, 0);
        assert_eq!(
            reply.len(),
            FUSE_OUT_HEADER_SIZE + FUSE_ENTRY_OUT_SIZE + 16
        );
        let b = &reply[FUSE_OUT_HEADER_SIZE..];
        let nodeid = u64_at(b, 0);
        let fh = u64_at(b, FUSE_ENTRY_OUT_SIZE);

        let mut write = Vec::new();
        write.extend_from_slice(&fh.to_le_bytes());
        write.extend_from_slice(&0u64.to_le_bytes());
        write.extend_from_slice(&4u32.to_le_bytes());
        write.extend_from_slice(&[0u8; 20]);
        write.extend_from_slice(b"data");
        let reply = roundtrip(&d, &frame_for(FuseOpcode::Write, nodeid, &write)).unwrap();
        assert_eq!(u32_at(&reply[FUSE_OUT_HEADER_SIZE..], 0), 4);

        let mut open = Vec::new();
        open.extend_from_slice(&0u32.to_le_bytes());
        open.extend_from_slice(&0u32.to_le_bytes());
        let reply = roundtrip(&d, &frame_for(FuseOpcode::Open, nodeid, &open)).unwrap();
        let rfh = u64_at(&reply[FUSE_OUT_HEADER_SIZE..], 0);
        let mut read = Vec::new();
        read.extend_from_slice(&rfh.to_le_bytes());
        read.extend_from_slice(&0u64.to_le_bytes());
        read.extend_from_slice(&16u32.to_le_bytes());
        read.extend_from_slice(&[0u8; 20]);
        let reply = roundtrip(&d, &frame_for(FuseOpcode::Read, nodeid, &read)).unwrap();
        assert_eq!(&reply[FUSE_OUT_HEADER_SIZE..], b"data");

        let mut exclusive = Vec::new();
        exclusive.extend_from_slice(&(0o1u32 | 0o100 | 0o200).to_le_bytes());
        exclusive.extend_from_slice(&0o100644u32.to_le_bytes());
        exclusive.extend_from_slice(&[0u8; 8]);
        exclusive.extend_from_slice(b"hello.txt\0");
        let reply = roundtrip(&d, &frame_for(FuseOpcode::Create, 1, &exclusive)).unwrap();
        assert_eq!(
            FuseOutHeader::decode(&reply).unwrap().error,
            -fusefs_vfs::error::errno::EEXIST
        );
    }

    #[test]
    fn mknod_makes_regular_files_only() {
        let d = dispatcher();
        let mknod = |mode: u32, name: &[u8]| {
            let mut b = Vec::new();
            b.extend_from_slice(&mode.to_le_bytes());
            b.extend_from_slice(&[0u8; 12]);
            b.extend_from_slice(name);
            frame_for(FuseOpcode::MkNod, 1, &b)
        };
        let reply = roundtrip(&d, &mknod(0o100600, b"node.txt\0")).unwrap();
        assert_eq!(FuseOutHeader::decode(&reply).unwrap().error, 0);
        assert_eq!(reply.len(), FUSE_OUT_HEADER_SIZE + FUSE_ENTRY_OUT_SIZE);
        let reply = roundtrip(&d, &mknod(0o010600, b"fifo\0")).unwrap();
        assert_eq!(FuseOutHeader::decode(&reply).unwrap().error, -ENOSYS);
    }

    #[test]
    fn owner_policy_accepts_owner_and_rejects_others() {
        let d = dispatcher();
        let owner = FuseHeader::new(1, ROOT_NODE_ID, 1000, 1000, 5);
        assert!(
            d.dispatch(&owner, FuseRequest::GetAttr { ino: ROOT_NODE_ID })
                .result
                .is_ok()
        );
        let other = FuseHeader::new(2, ROOT_NODE_ID, 1001, 1000, 5);
        let err = d
            .dispatch(&other, FuseRequest::GetAttr { ino: ROOT_NODE_ID })
            .result
            .unwrap_err();
        assert_eq!(err.errno, fusefs_vfs::error::errno::EACCES);
    }
}
