//! Generic FUSE opcode dispatcher translating FUSE requests into [`fusefs_vfs::Volume`] calls.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use fusefs_vfs::{FileHandle, FileKind, NodeId, VfsError, VfsResult, Volume};

use crate::fuse_abi::{FuseHeader, FuseReply, FuseRequest, FuseResponse};
use crate::pid_bind::PidRegistry;

#[derive(Debug, Default)]
struct DirHandleTable {
    next_fh: u64,
    handles: BTreeMap<FileHandle, NodeId>,
}

impl DirHandleTable {
    fn open(&mut self, ino: NodeId) -> FileHandle {
        self.next_fh = self.next_fh.saturating_add(1);
        let fh = FileHandle(self.next_fh);
        self.handles.insert(fh, ino);
        fh
    }

    fn validate(&self, fh: FileHandle, ino: NodeId) -> VfsResult<()> {
        match self.handles.get(&fh) {
            Some(&bound_ino) if bound_ino == ino => Ok(()),
            Some(_) => Err(VfsError::Ebadf(format!(
                "directory handle {fh} does not belong to {ino}"
            ))),
            None => Err(VfsError::Ebadf(format!("unknown directory handle {fh}"))),
        }
    }

    fn release(&mut self, fh: FileHandle, ino: NodeId) -> VfsResult<()> {
        self.validate(fh, ino)?;
        self.handles.remove(&fh);
        Ok(())
    }
}

/// Generic FUSE request dispatcher backed by a [`Volume`] and kernel [`PidRegistry`].
///
/// Every incoming request's kernel `(pid, uid, gid)` header is resolved through
/// [`PidRegistry`] before invoking the underlying [`Volume`]; unregistered PIDs
/// fail closed with `EACCES`.
pub struct FuseDispatcher<V: Volume> {
    volume: Arc<V>,
    pid_registry: Arc<PidRegistry>,
    dir_handles: RwLock<DirHandleTable>,
}

impl<V: Volume> FuseDispatcher<V> {
    pub fn new(volume: Arc<V>, pid_registry: Arc<PidRegistry>) -> Self {
        Self {
            volume,
            pid_registry,
            dir_handles: RwLock::new(DirHandleTable::default()),
        }
    }

    pub fn volume(&self) -> &Arc<V> {
        &self.volume
    }

    pub fn pid_registry(&self) -> &Arc<PidRegistry> {
        &self.pid_registry
    }

    /// Dispatches a [`FuseRequest`] and returns a [`FuseResponse`] with POSIX errno mapping.
    pub fn dispatch(&self, header: &FuseHeader, request: FuseRequest) -> FuseResponse {
        match self.dispatch_result(header, request) {
            Ok(reply) => FuseResponse::ok(header.unique, reply),
            Err(err) => FuseResponse::err(header.unique, err),
        }
    }

    /// Dispatches a [`FuseRequest`] and returns a typed [`VfsResult<FuseReply>`].
    pub fn dispatch_result(&self, header: &FuseHeader, request: FuseRequest) -> VfsResult<FuseReply> {
        let caller = self
            .pid_registry
            .resolve(header.pid, header.uid, header.gid)?;

        match request {
            FuseRequest::Lookup { parent, name } => {
                let attr = self.volume.lookup(&caller, parent, &name)?;
                Ok(FuseReply::Entry(attr))
            }
            FuseRequest::GetAttr { ino } => {
                let attr = self.volume.getattr(&caller, ino)?;
                Ok(FuseReply::Attr(attr))
            }
            FuseRequest::SetAttr { ino, attr } => {
                let updated = self.volume.setattr(&caller, ino, attr)?;
                Ok(FuseReply::Attr(updated))
            }
            FuseRequest::ReadLink { ino } => {
                let target = self.volume.readlink(&caller, ino)?;
                Ok(FuseReply::ReadLink(target))
            }
            FuseRequest::MkDir { parent, name, mode } => {
                let attr = self.volume.mkdir(&caller, parent, &name, mode)?;
                Ok(FuseReply::Entry(attr))
            }
            FuseRequest::Unlink { parent, name } => {
                self.volume.unlink(&caller, parent, &name)?;
                Ok(FuseReply::Empty)
            }
            FuseRequest::RmDir { parent, name } => {
                self.volume.rmdir(&caller, parent, &name)?;
                Ok(FuseReply::Empty)
            }
            FuseRequest::Symlink {
                parent,
                name,
                target,
            } => {
                let attr = self.volume.symlink(&caller, parent, &name, &target)?;
                Ok(FuseReply::Entry(attr))
            }
            FuseRequest::Rename {
                parent,
                name,
                new_parent,
                new_name,
            } => {
                self.volume
                    .rename(&caller, parent, &name, new_parent, &new_name)?;
                Ok(FuseReply::Empty)
            }
            FuseRequest::Link {
                ino,
                new_parent,
                new_name,
            } => {
                let attr = self.volume.link(&caller, ino, new_parent, &new_name)?;
                Ok(FuseReply::Entry(attr))
            }
            FuseRequest::Open { ino, flags } => {
                let fh = self.volume.open(&caller, ino, flags)?;
                Ok(FuseReply::Open { fh })
            }
            FuseRequest::Read {
                ino,
                fh,
                offset,
                size,
            } => {
                let bytes = self.volume.read(&caller, ino, fh, offset, size)?;
                Ok(FuseReply::Data(bytes))
            }
            FuseRequest::Write {
                ino,
                fh,
                offset,
                data,
                expected_generation,
            } => {
                let written =
                    self.volume
                        .write(&caller, ino, fh, offset, &data, expected_generation)?;
                Ok(FuseReply::Written(written))
            }
            FuseRequest::StatFs { ino } => {
                let stats = self.volume.statfs(&caller, ino)?;
                Ok(FuseReply::StatFs(stats))
            }
            FuseRequest::Release { ino, fh } => {
                self.volume.release(&caller, ino, fh)?;
                Ok(FuseReply::Empty)
            }
            FuseRequest::Fsync { ino, fh, datasync } => {
                self.volume.fsync(&caller, ino, fh, datasync)?;
                Ok(FuseReply::Empty)
            }
            FuseRequest::OpenDir { ino } => {
                let attr = self.volume.getattr(&caller, ino)?;
                if attr.kind != FileKind::Directory {
                    return Err(VfsError::Enotdir(format!("{ino} is not a directory")));
                }
                let fh = self
                    .dir_handles
                    .write()
                    .expect("dir handle table poisoned")
                    .open(ino);
                Ok(FuseReply::Open { fh })
            }
            FuseRequest::ReadDir { ino, fh, offset } => {
                self.dir_handles
                    .read()
                    .expect("dir handle table poisoned")
                    .validate(fh, ino)?;
                let entries = self.volume.readdir(&caller, ino, offset)?;
                Ok(FuseReply::DirEntries(entries))
            }
            FuseRequest::ReleaseDir { ino, fh } => {
                self.dir_handles
                    .write()
                    .expect("dir handle table poisoned")
                    .release(fh, ino)?;
                Ok(FuseReply::Empty)
            }
            FuseRequest::GetXattr { ino, name } => {
                let val = self.volume.getxattr(&caller, ino, &name)?;
                Ok(FuseReply::XattrValue(val))
            }
            FuseRequest::ListXattr { ino } => {
                let names = self.volume.listxattr(&caller, ino)?;
                Ok(FuseReply::XattrNames(names))
            }
            FuseRequest::SetXattr { ino, name, value } => {
                self.volume.setxattr(&caller, ino, &name, &value)?;
                Ok(FuseReply::Empty)
            }
            FuseRequest::RemoveXattr { ino, name } => {
                self.volume.removexattr(&caller, ino, &name)?;
                Ok(FuseReply::Empty)
            }
            FuseRequest::CopyFileRange {
                ino_in,
                offset_in,
                ino_out,
                offset_out,
                len,
            } => {
                let copied = self.volume.copy_file_range(
                    &caller,
                    ino_in,
                    offset_in,
                    ino_out,
                    offset_out,
                    len,
                )?;
                Ok(FuseReply::Copied(copied))
            }
            FuseRequest::Flock {
                ino,
                fh,
                op,
                non_blocking,
            } => {
                self.volume.flock(&caller, ino, fh, op, non_blocking)?;
                Ok(FuseReply::Empty)
            }
        }
    }
}

/// Linux `/dev/fuse` device session wrapper pairing an open device file with [`FuseDispatcher`].
#[cfg(target_os = "linux")]
pub struct DevFuseSession<V: Volume> {
    device: std::fs::File,
    dispatcher: FuseDispatcher<V>,
}

#[cfg(target_os = "linux")]
impl<V: Volume> DevFuseSession<V> {
    /// Opens `/dev/fuse` (or a custom FUSE device path) and binds it to `dispatcher`.
    pub fn open(path: &std::path::Path, dispatcher: FuseDispatcher<V>) -> std::io::Result<Self> {
        let device = crate::probe::open_dev_fuse(path)?;
        Ok(Self { device, dispatcher })
    }

    pub fn device(&self) -> &std::fs::File {
        &self.device
    }

    pub fn dispatcher(&self) -> &FuseDispatcher<V> {
        &self.dispatcher
    }

    /// Reads the next raw FUSE frame from `/dev/fuse` into `buf` and parses its [`FuseInHeader`].
    pub fn read_frame(
        &mut self,
        buf: &mut [u8],
    ) -> std::io::Result<(crate::fuse_abi::FuseInHeader, usize)> {
        use std::io::Read;
        let n = self.device.read(buf)?;
        let header = crate::fuse_abi::FuseInHeader::decode(&buf[..n]).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("short /dev/fuse frame: {n} bytes"),
            )
        })?;
        Ok((header, n))
    }

    /// Writes a FUSE reply header + optional payload back to `/dev/fuse`.
    pub fn write_reply(
        &mut self,
        header: crate::fuse_abi::FuseOutHeader,
        payload: &[u8],
    ) -> std::io::Result<()> {
        use std::io::Write;
        let mut frame = Vec::with_capacity(crate::fuse_abi::FUSE_OUT_HEADER_SIZE + payload.len());
        frame.extend_from_slice(&header.encode());
        frame.extend_from_slice(payload);
        self.device.write_all(&frame)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use fusefs_vfs::{
        FlockOp, InMemoryVolume, OpenFlags, ROOT_NODE_ID, SetAttr, error::errno,
    };

    use super::*;

    fn test_header(unique: u64, pid: u32) -> FuseHeader {
        FuseHeader::new(unique, ROOT_NODE_ID, 1000, 1000, pid)
    }

    #[test]
    fn dispatcher_rejects_unregistered_pid_fail_closed() {
        let vol = Arc::new(
            InMemoryVolume::builder()
                .file("/hello.txt", b"world")
                .build()
                .unwrap(),
        );
        let pids = Arc::new(PidRegistry::new());
        pids.register_pid(100, "/").unwrap();
        let dispatcher = FuseDispatcher::new(vol, pids);

        let resp = dispatcher.dispatch(
            &test_header(1, 999),
            FuseRequest::GetAttr { ino: ROOT_NODE_ID },
        );
        let err = resp.result.unwrap_err();
        assert_eq!(err.errno, errno::EACCES);
    }

    #[test]
    fn dispatcher_enforces_capability_root_on_dotdot_lookup() {
        let vol = Arc::new(
            InMemoryVolume::builder()
                .file("/secret/top.txt", b"classified")
                .file("/sandbox/work/app.rs", b"fn main() {}")
                .build()
                .unwrap(),
        );
        let pids = Arc::new(PidRegistry::new());
        pids.register_pid(500, "/sandbox").unwrap();
        let dispatcher = FuseDispatcher::new(vol, pids);
        let hdr = test_header(1, 500);

        let sandbox_attr = match dispatcher
            .dispatch_result(
                &hdr,
                FuseRequest::Lookup {
                    parent: ROOT_NODE_ID,
                    name: "sandbox".into(),
                },
            )
            .unwrap()
        {
            FuseReply::Entry(a) => a,
            other => panic!("unexpected reply: {other:?}"),
        };

        // `..` from `/sandbox` must stay clamped at `/sandbox`, never escaping to `/`.
        let clamped = match dispatcher
            .dispatch_result(
                &hdr,
                FuseRequest::Lookup {
                    parent: sandbox_attr.ino,
                    name: "..".into(),
                },
            )
            .unwrap()
        {
            FuseReply::Entry(a) => a,
            other => panic!("unexpected reply: {other:?}"),
        };
        assert_eq!(clamped.ino, sandbox_attr.ino);
    }

    #[test]
    fn dispatcher_metadata_ops_never_trigger_lazy_chunk_loader() {
        let reads = Arc::new(AtomicUsize::new(0));
        let reads_clone = Arc::clone(&reads);
        let loader = Arc::new(move |offset: u64, size: u32| {
            reads_clone.fetch_add(1, Ordering::SeqCst);
            let data = b"0123456789abcdef";
            let start = (offset as usize).min(data.len());
            let end = (start + size as usize).min(data.len());
            Ok(data[start..end].to_vec())
        });

        let vol = Arc::new(
            InMemoryVolume::builder()
                .lazy_file("/src.bin", 16, loader)
                .file("/dst.bin", b"")
                .file("/drop.txt", b"bye")
                .build()
                .unwrap(),
        );
        let pids = Arc::new(PidRegistry::new());
        pids.register_pid(700, "/").unwrap();
        let dispatcher = FuseDispatcher::new(vol, pids);
        let hdr = test_header(10, 700);

        // Lookup + GetAttr
        let src = match dispatcher
            .dispatch_result(
                &hdr,
                FuseRequest::Lookup {
                    parent: ROOT_NODE_ID,
                    name: "src.bin".into(),
                },
            )
            .unwrap()
        {
            FuseReply::Entry(a) => a,
            other => panic!("unexpected: {other:?}"),
        };
        let dst = match dispatcher
            .dispatch_result(
                &hdr,
                FuseRequest::Lookup {
                    parent: ROOT_NODE_ID,
                    name: "dst.bin".into(),
                },
            )
            .unwrap()
        {
            FuseReply::Entry(a) => a,
            other => panic!("unexpected: {other:?}"),
        };
        let attr = match dispatcher
            .dispatch_result(&hdr, FuseRequest::GetAttr { ino: src.ino })
            .unwrap()
        {
            FuseReply::Attr(a) => a,
            other => panic!("unexpected: {other:?}"),
        };
        assert_eq!(attr.size, 16);

        // OpenDir + ReadDir + ReleaseDir
        let dir_fh = match dispatcher
            .dispatch_result(&hdr, FuseRequest::OpenDir { ino: ROOT_NODE_ID })
            .unwrap()
        {
            FuseReply::Open { fh } => fh,
            other => panic!("unexpected: {other:?}"),
        };
        let entries = match dispatcher
            .dispatch_result(
                &hdr,
                FuseRequest::ReadDir {
                    ino: ROOT_NODE_ID,
                    fh: dir_fh,
                    offset: 0,
                },
            )
            .unwrap()
        {
            FuseReply::DirEntries(e) => e,
            other => panic!("unexpected: {other:?}"),
        };
        assert!(entries.iter().any(|e| e.name == "src.bin"));
        dispatcher
            .dispatch_result(
                &hdr,
                FuseRequest::ReleaseDir {
                    ino: ROOT_NODE_ID,
                    fh: dir_fh,
                },
            )
            .unwrap();

        // CopyFileRange (metadata-only lazy reference!)
        let copied = match dispatcher
            .dispatch_result(
                &hdr,
                FuseRequest::CopyFileRange {
                    ino_in: src.ino,
                    offset_in: 4,
                    ino_out: dst.ino,
                    offset_out: 0,
                    len: 6,
                },
            )
            .unwrap()
        {
            FuseReply::Copied(n) => n,
            other => panic!("unexpected: {other:?}"),
        };
        assert_eq!(copied, 6);

        // Rename + Unlink
        dispatcher
            .dispatch_result(
                &hdr,
                FuseRequest::Rename {
                    parent: ROOT_NODE_ID,
                    name: "src.bin".into(),
                    new_parent: ROOT_NODE_ID,
                    new_name: "src-renamed.bin".into(),
                },
            )
            .unwrap();
        dispatcher
            .dispatch_result(
                &hdr,
                FuseRequest::Unlink {
                    parent: ROOT_NODE_ID,
                    name: "drop.txt".into(),
                },
            )
            .unwrap();

        // Zero chunk reads performed during all metadata-only operations!
        assert_eq!(reads.load(Ordering::SeqCst), 0);

        // Now read dst.bin to confirm the copied range resolves lazily on demand.
        let fh = match dispatcher
            .dispatch_result(
                &hdr,
                FuseRequest::Open {
                    ino: dst.ino,
                    flags: OpenFlags::read_only(),
                },
            )
            .unwrap()
        {
            FuseReply::Open { fh } => fh,
            other => panic!("unexpected: {other:?}"),
        };
        let data = match dispatcher
            .dispatch_result(
                &hdr,
                FuseRequest::Read {
                    ino: dst.ino,
                    fh,
                    offset: 0,
                    size: 16,
                },
            )
            .unwrap()
        {
            FuseReply::Data(b) => b,
            other => panic!("unexpected: {other:?}"),
        };
        assert_eq!(data, b"456789");
        assert_eq!(reads.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn dispatcher_data_and_extended_ops_work_end_to_end() {
        let vol = Arc::new(
            InMemoryVolume::builder()
                .file("/notes.txt", b"hello")
                .build()
                .unwrap(),
        );
        let pids = Arc::new(PidRegistry::new());
        pids.register_pid(800, "/").unwrap();
        let dispatcher = FuseDispatcher::new(vol, pids);
        let hdr = test_header(20, 800);

        let file = match dispatcher
            .dispatch_result(
                &hdr,
                FuseRequest::Lookup {
                    parent: ROOT_NODE_ID,
                    name: "notes.txt".into(),
                },
            )
            .unwrap()
        {
            FuseReply::Entry(a) => a,
            other => panic!("unexpected: {other:?}"),
        };

        // Open RW, Write, Fsync, Flock, Read, Release
        let fh = match dispatcher
            .dispatch_result(
                &hdr,
                FuseRequest::Open {
                    ino: file.ino,
                    flags: OpenFlags::read_write(),
                },
            )
            .unwrap()
        {
            FuseReply::Open { fh } => fh,
            other => panic!("unexpected: {other:?}"),
        };

        dispatcher
            .dispatch_result(
                &hdr,
                FuseRequest::Flock {
                    ino: file.ino,
                    fh,
                    op: FlockOp::Exclusive,
                    non_blocking: true,
                },
            )
            .unwrap();

        let written = match dispatcher
            .dispatch_result(
                &hdr,
                FuseRequest::Write {
                    ino: file.ino,
                    fh,
                    offset: 5,
                    data: b" fuse".to_vec(),
                    expected_generation: None,
                },
            )
            .unwrap()
        {
            FuseReply::Written(n) => n,
            other => panic!("unexpected: {other:?}"),
        };
        assert_eq!(written, 5);

        dispatcher
            .dispatch_result(
                &hdr,
                FuseRequest::Fsync {
                    ino: file.ino,
                    fh,
                    datasync: true,
                },
            )
            .unwrap();

        let content = match dispatcher
            .dispatch_result(
                &hdr,
                FuseRequest::Read {
                    ino: file.ino,
                    fh,
                    offset: 0,
                    size: 64,
                },
            )
            .unwrap()
        {
            FuseReply::Data(b) => b,
            other => panic!("unexpected: {other:?}"),
        };
        assert_eq!(content, b"hello fuse");

        dispatcher
            .dispatch_result(
                &hdr,
                FuseRequest::Flock {
                    ino: file.ino,
                    fh,
                    op: FlockOp::Unlock,
                    non_blocking: true,
                },
            )
            .unwrap();
        dispatcher
            .dispatch_result(&hdr, FuseRequest::Release { ino: file.ino, fh })
            .unwrap();

        // SetAttr, Xattr, MkDir, Symlink, ReadLink, Link, StatFs, RmDir
        let updated = match dispatcher
            .dispatch_result(
                &hdr,
                FuseRequest::SetAttr {
                    ino: file.ino,
                    attr: SetAttr {
                        mode: Some(0o600),
                        ..Default::default()
                    },
                },
            )
            .unwrap()
        {
            FuseReply::Attr(a) => a,
            other => panic!("unexpected: {other:?}"),
        };
        assert_eq!(updated.mode, 0o600);

        dispatcher
            .dispatch_result(
                &hdr,
                FuseRequest::SetXattr {
                    ino: file.ino,
                    name: "user.tag".into(),
                    value: b"v1".to_vec(),
                },
            )
            .unwrap();
        assert_eq!(
            dispatcher
                .dispatch_result(
                    &hdr,
                    FuseRequest::GetXattr {
                        ino: file.ino,
                        name: "user.tag".into(),
                    },
                )
                .unwrap(),
            FuseReply::XattrValue(b"v1".to_vec())
        );
        assert_eq!(
            dispatcher
                .dispatch_result(&hdr, FuseRequest::ListXattr { ino: file.ino })
                .unwrap(),
            FuseReply::XattrNames(vec!["user.tag".to_string()])
        );
        dispatcher
            .dispatch_result(
                &hdr,
                FuseRequest::RemoveXattr {
                    ino: file.ino,
                    name: "user.tag".into(),
                },
            )
            .unwrap();

        let sub = match dispatcher
            .dispatch_result(
                &hdr,
                FuseRequest::MkDir {
                    parent: ROOT_NODE_ID,
                    name: "sub".into(),
                    mode: 0o755,
                },
            )
            .unwrap()
        {
            FuseReply::Entry(a) => a,
            other => panic!("unexpected: {other:?}"),
        };
        let link_attr = match dispatcher
            .dispatch_result(
                &hdr,
                FuseRequest::Symlink {
                    parent: sub.ino,
                    name: "alias".into(),
                    target: "../notes.txt".into(),
                },
            )
            .unwrap()
        {
            FuseReply::Entry(a) => a,
            other => panic!("unexpected: {other:?}"),
        };
        assert_eq!(
            dispatcher
                .dispatch_result(&hdr, FuseRequest::ReadLink { ino: link_attr.ino })
                .unwrap(),
            FuseReply::ReadLink("../notes.txt".into())
        );
        dispatcher
            .dispatch_result(
                &hdr,
                FuseRequest::Link {
                    ino: file.ino,
                    new_parent: ROOT_NODE_ID,
                    new_name: "notes-hardlink.txt".into(),
                },
            )
            .unwrap();
        assert!(matches!(
            dispatcher
                .dispatch_result(&hdr, FuseRequest::StatFs { ino: ROOT_NODE_ID })
                .unwrap(),
            FuseReply::StatFs(_)
        ));
        dispatcher
            .dispatch_result(
                &hdr,
                FuseRequest::Unlink {
                    parent: sub.ino,
                    name: "alias".into(),
                },
            )
            .unwrap();
        dispatcher
            .dispatch_result(
                &hdr,
                FuseRequest::RmDir {
                    parent: ROOT_NODE_ID,
                    name: "sub".into(),
                },
            )
            .unwrap();
    }
}
