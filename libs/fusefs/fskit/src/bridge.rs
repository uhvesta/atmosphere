//! Framed Unix-domain App Group bridge server ([`FsKitBridgeServer`]) and
//! loopback client ([`FsKitBridgeClient`]).

use std::io;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};

use fusefs_vfs::Volume;

use crate::adapter::FsKitAdapter;
use crate::credentials::extract_peer_credentials;
use crate::protocol::{FsKitEnvelope, FsKitResponse, read_frame, write_frame};

/// Unix-domain socket bridge server connecting a macOS FSKit `.appex` extension
/// to an in-process [`FsKitAdapter<V>`].
pub struct FsKitBridgeServer<V: Volume + 'static> {
    socket_path: PathBuf,
    listener: UnixListener,
    adapter: Arc<FsKitAdapter<V>>,
}

impl<V: Volume + 'static> FsKitBridgeServer<V> {
    /// Binds a new bridge server at `socket_path` (cleaning up any stale socket file first).
    pub fn bind(
        socket_path: impl Into<PathBuf>,
        adapter: Arc<FsKitAdapter<V>>,
    ) -> io::Result<Self> {
        let socket_path = socket_path.into();
        if socket_path.exists() {
            let _ = std::fs::remove_file(&socket_path);
        }
        let listener = UnixListener::bind(&socket_path)?;
        Ok(Self {
            socket_path,
            listener,
            adapter,
        })
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Accepts one connection and serves framed requests until the peer closes.
    pub fn serve_one(&self) -> io::Result<()> {
        let (mut stream, _) = self.listener.accept()?;
        let peer = extract_peer_credentials(&stream)?;
        loop {
            let env: FsKitEnvelope = match read_frame(&mut stream) {
                Ok(env) => env,
                Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
                Err(err) => return Err(err),
            };
            let resp = self.adapter.dispatch(env, peer);
            write_frame(&mut stream, &resp)?;
        }
    }

    /// Spawns a background accept loop and returns a handle that stops cleanly on drop/shutdown.
    pub fn spawn(self) -> FsKitBridgeHandle {
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
                let adapter = Arc::clone(&self.adapter);
                thread::spawn(move || {
                    let Ok(peer) = extract_peer_credentials(&stream) else {
                        return;
                    };
                    loop {
                        let env: FsKitEnvelope = match read_frame(&mut stream) {
                            Ok(env) => env,
                            Err(_) => break,
                        };
                        let resp = adapter.dispatch(env, peer);
                        if write_frame(&mut stream, &resp).is_err() {
                            break;
                        }
                    }
                });
            }
        });

        FsKitBridgeHandle {
            socket_path,
            shutdown,
            join: Some(join),
        }
    }
}

/// Handle to a running background [`FsKitBridgeServer`].
pub struct FsKitBridgeHandle {
    socket_path: PathBuf,
    shutdown: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl FsKitBridgeHandle {
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Gracefully stops the background bridge server and removes the socket file.
    pub fn shutdown(mut self) {
        self.stop_inner();
    }

    fn stop_inner(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        let _ = UnixStream::connect(&self.socket_path);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

impl Drop for FsKitBridgeHandle {
    fn drop(&mut self) {
        self.stop_inner();
    }
}

/// Persistent or single-shot Unix socket client for [`FsKitBridgeServer`].
pub struct FsKitBridgeClient {
    stream: UnixStream,
}

impl FsKitBridgeClient {
    /// Connects to a running [`FsKitBridgeServer`] at `socket_path`.
    pub fn connect(socket_path: impl AsRef<Path>) -> io::Result<Self> {
        let stream = UnixStream::connect(socket_path)?;
        Ok(Self { stream })
    }

    /// Sends one [`FsKitEnvelope`] and awaits its [`FsKitResponse`].
    pub fn call(&mut self, envelope: &FsKitEnvelope) -> io::Result<FsKitResponse> {
        write_frame(&mut self.stream, envelope)?;
        read_frame(&mut self.stream)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::AtomicUsize;

    use fusefs_host::CapabilityRouter;
    use fusefs_vfs::error::errno;
    use fusefs_vfs::{
        AccessHook, CallerContext, InMemoryVolume, MetadataOp, OpenFlags, ROOT_NODE_ID, SetAttr,
        VirtualPath,
    };

    use super::*;
    use crate::credentials::AuditToken;
    use crate::protocol::{FsItemKind, FsKitRequest, ProbeResult};

    struct RecordingHook {
        seen_callers: Mutex<Vec<CallerContext>>,
    }

    impl RecordingHook {
        fn new() -> Self {
            Self {
                seen_callers: Mutex::new(Vec::new()),
            }
        }
    }

    impl AccessHook for RecordingHook {
        fn before_metadata(
            &self,
            caller: &CallerContext,
            _op: MetadataOp,
            _path: &VirtualPath,
        ) -> fusefs_vfs::VfsResult<()> {
            self.seen_callers.lock().unwrap().push(caller.clone());
            Ok(())
        }
    }

    #[test]
    fn bridge_round_trips_all_vfs_ops_and_preserves_zero_data_metadata_isolation() {
        let chunk_reads = Arc::new(AtomicUsize::new(0));
        let chunk_reads_clone = Arc::clone(&chunk_reads);
        let payload = b"lazy-chunk-payload-0123456789".to_vec();
        let payload_clone = payload.clone();

        let volume = Arc::new(
            InMemoryVolume::builder()
                .dir("/work")
                .file("/work/notes.txt", b"initial".to_vec())
                .lazy_file(
                    "/work/big.bin",
                    payload.len() as u64,
                    Arc::new(move |offset, size| {
                        chunk_reads_clone.fetch_add(1, Ordering::SeqCst);
                        let start = (offset as usize).min(payload_clone.len());
                        let end = (start + size as usize).min(payload_clone.len());
                        Ok(payload_clone[start..end].to_vec())
                    }),
                )
                .symlink("/work/link", "notes.txt")
                .build()
                .unwrap(),
        );

        let adapter = Arc::new(FsKitAdapter::new(volume, "humatafs", "humata"));
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("fskit.sock");
        let server = FsKitBridgeServer::bind(&sock, adapter).unwrap().spawn();
        let mut client = FsKitBridgeClient::connect(server.socket_path()).unwrap();

        // Ping verifies socket peer credentials extraction.
        match client.call(&FsKitEnvelope::new(FsKitRequest::Ping)).unwrap() {
            FsKitResponse::Pong { peer, .. } => {
                assert_eq!(peer.pid, std::process::id());
            }
            other => panic!("expected Pong, got {other:?}"),
        }

        // ProbeResource recognizes humatafs:// and rejects unknown schemes.
        match client
            .call(&FsKitEnvelope::new(FsKitRequest::ProbeResource {
                resource_url: "humatafs://main".into(),
            }))
            .unwrap()
        {
            FsKitResponse::Probe {
                result: ProbeResult::Usable { name, .. },
            } => assert_eq!(name, "humata-main"),
            other => panic!("expected Usable probe, got {other:?}"),
        }

        // Lookup /work -> /work/big.bin + getattr + readdir + statfs + fsync: ZERO chunk reads!
        let work_ino = match client
            .call(&FsKitEnvelope::new(FsKitRequest::LookupItem {
                parent: ROOT_NODE_ID,
                name: "work".into(),
            }))
            .unwrap()
        {
            FsKitResponse::ItemAttributes { attr } => attr.ino,
            other => panic!("expected ItemAttributes, got {other:?}"),
        };

        let big_ino = match client
            .call(&FsKitEnvelope::new(FsKitRequest::LookupItem {
                parent: work_ino,
                name: "big.bin".into(),
            }))
            .unwrap()
        {
            FsKitResponse::ItemAttributes { attr } => {
                assert_eq!(attr.size, payload.len() as u64);
                attr.ino
            }
            other => panic!("expected ItemAttributes, got {other:?}"),
        };

        // getattr
        assert!(matches!(
            client
                .call(&FsKitEnvelope::new(FsKitRequest::GetAttributes {
                    item: big_ino,
                }))
                .unwrap(),
            FsKitResponse::ItemAttributes { .. }
        ));

        // readdir
        match client
            .call(&FsKitEnvelope::new(FsKitRequest::EnumerateDirectory {
                directory: work_ino,
                cookie: 0,
                max_entries: None,
            }))
            .unwrap()
        {
            FsKitResponse::DirectoryEntries { entries, eof, .. } => {
                assert!(eof);
                assert!(entries.iter().any(|e| e.name == "big.bin"));
                assert!(entries.iter().any(|e| e.name == "notes.txt"));
            }
            other => panic!("expected DirectoryEntries, got {other:?}"),
        }

        // readlink
        let link_ino = match client
            .call(&FsKitEnvelope::new(FsKitRequest::LookupItem {
                parent: work_ino,
                name: "link".into(),
            }))
            .unwrap()
        {
            FsKitResponse::ItemAttributes { attr } => attr.ino,
            other => panic!("expected ItemAttributes, got {other:?}"),
        };
        match client
            .call(&FsKitEnvelope::new(FsKitRequest::ReadSymbolicLink {
                item: link_ino,
            }))
            .unwrap()
        {
            FsKitResponse::SymbolicLink { target } => assert_eq!(target, "notes.txt"),
            other => panic!("expected SymbolicLink, got {other:?}"),
        }

        // statfs & fsync on lazy file
        assert!(matches!(
            client
                .call(&FsKitEnvelope::new(FsKitRequest::VolumeStatistics {
                    item: Some(work_ino),
                }))
                .unwrap(),
            FsKitResponse::Statistics { .. }
        ));
        assert!(matches!(
            client
                .call(&FsKitEnvelope::new(FsKitRequest::Synchronize {
                    item: big_ino,
                    handle: None,
                    datasync: true,
                }))
                .unwrap(),
            FsKitResponse::Ok
        ));

        // CRITICAL: none of the metadata/control operations triggered a chunk read!
        assert_eq!(chunk_reads.load(Ordering::SeqCst), 0);

        // Now perform an actual Read on big.bin -> triggers chunk loader.
        match client
            .call(&FsKitEnvelope::new(FsKitRequest::Read {
                item: big_ino,
                handle: None,
                offset: 5,
                length: 13,
            }))
            .unwrap()
        {
            FsKitResponse::ReadData { data } => assert_eq!(data, b"chunk-payload"),
            other => panic!("expected ReadData, got {other:?}"),
        }
        assert_eq!(chunk_reads.load(Ordering::SeqCst), 1);

        // Write + SetAttributes + Rename + Unlink round-trip on notes.txt.
        let notes_ino = match client
            .call(&FsKitEnvelope::new(FsKitRequest::LookupItem {
                parent: work_ino,
                name: "notes.txt".into(),
            }))
            .unwrap()
        {
            FsKitResponse::ItemAttributes { attr } => attr.ino,
            other => panic!("expected ItemAttributes, got {other:?}"),
        };
        let fh = match client
            .call(&FsKitEnvelope::new(FsKitRequest::OpenItem {
                item: notes_ino,
                flags: OpenFlags::read_write(),
            }))
            .unwrap()
        {
            FsKitResponse::Opened { handle } => handle,
            other => panic!("expected Opened, got {other:?}"),
        };
        match client
            .call(&FsKitEnvelope::new(FsKitRequest::Write {
                item: notes_ino,
                handle: Some(fh),
                offset: 0,
                data: b"updated-content".to_vec(),
                expected_generation: None,
            }))
            .unwrap()
        {
            FsKitResponse::Written { bytes_written } => assert_eq!(bytes_written, 15),
            other => panic!("expected Written, got {other:?}"),
        }
        assert!(matches!(
            client
                .call(&FsKitEnvelope::new(FsKitRequest::Synchronize {
                    item: notes_ino,
                    handle: Some(fh),
                    datasync: false,
                }))
                .unwrap(),
            FsKitResponse::Ok
        ));
        assert!(matches!(
            client
                .call(&FsKitEnvelope::new(FsKitRequest::CloseItem {
                    item: notes_ino,
                    handle: fh,
                }))
                .unwrap(),
            FsKitResponse::Ok
        ));

        // Truncate via SetAttributes
        match client
            .call(&FsKitEnvelope::new(FsKitRequest::SetAttributes {
                item: notes_ino,
                attr: SetAttr {
                    size: Some(7),
                    ..Default::default()
                },
            }))
            .unwrap()
        {
            FsKitResponse::ItemAttributes { attr } => assert_eq!(attr.size, 7),
            other => panic!("expected ItemAttributes, got {other:?}"),
        }
        match client
            .call(&FsKitEnvelope::new(FsKitRequest::Read {
                item: notes_ino,
                handle: None,
                offset: 0,
                length: 32,
            }))
            .unwrap()
        {
            FsKitResponse::ReadData { data } => assert_eq!(data, b"updated"),
            other => panic!("expected ReadData, got {other:?}"),
        }

        // CreateItem (mkdir), RenameItem, RemoveItem (unlink + rmdir).
        assert!(matches!(
            client
                .call(&FsKitEnvelope::new(FsKitRequest::CreateItem {
                    parent: work_ino,
                    name: "sub".into(),
                    kind: FsItemKind::Directory,
                    mode: 0o755,
                    symlink_target: None,
                }))
                .unwrap(),
            FsKitResponse::ItemAttributes { .. }
        ));
        assert!(matches!(
            client
                .call(&FsKitEnvelope::new(FsKitRequest::RenameItem {
                    source_parent: work_ino,
                    source_name: "notes.txt".into(),
                    dest_parent: work_ino,
                    dest_name: "notes_renamed.txt".into(),
                }))
                .unwrap(),
            FsKitResponse::Ok
        ));
        assert!(matches!(
            client
                .call(&FsKitEnvelope::new(FsKitRequest::RemoveItem {
                    parent: work_ino,
                    name: "notes_renamed.txt".into(),
                    is_directory: false,
                }))
                .unwrap(),
            FsKitResponse::Ok
        ));
        assert!(matches!(
            client
                .call(&FsKitEnvelope::new(FsKitRequest::RemoveItem {
                    parent: work_ino,
                    name: "sub".into(),
                    is_directory: true,
                }))
                .unwrap(),
            FsKitResponse::Ok
        ));

        server.shutdown();
    }

    #[test]
    fn audit_token_and_capability_root_routing_enforces_clamping_and_revocation() {
        let hook = Arc::new(RecordingHook::new());
        let volume = Arc::new(
            InMemoryVolume::builder()
                .with_access_hook(hook.clone())
                .file("/workspaces/turn1/secret.txt", b"scoped".to_vec())
                .file("/etc/passwd", b"root:x:0:0".to_vec())
                .build()
                .unwrap(),
        );

        let router = Arc::new(CapabilityRouter::new());
        router
            .register(
                "cap_turn1",
                VirtualPath::parse("/workspaces/turn1").unwrap(),
                Some("turn-1".into()),
                true,
            )
            .unwrap();

        let adapter = Arc::new(
            FsKitAdapter::new(volume, "humatafs", "humata").with_capability_router(router.clone()),
        );
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("cap.sock");
        let server = FsKitBridgeServer::bind(&sock, adapter).unwrap().spawn();
        let mut client = FsKitBridgeClient::connect(server.socket_path()).unwrap();

        let token = AuditToken::from_ids(7777, 502, 20);

        // Virtual `cap_turn1` lookup at root resolves `/workspaces/turn1`.
        let cap_root_ino = match client
            .call(
                &FsKitEnvelope::new(FsKitRequest::LookupItem {
                    parent: ROOT_NODE_ID,
                    name: "cap_turn1".into(),
                })
                .with_audit_token(token),
            )
            .unwrap()
        {
            FsKitResponse::ItemAttributes { attr } => attr.ino,
            other => panic!("expected cap_turn1 attr, got {other:?}"),
        };

        // `..` traversal from capability root stays clamped to `cap_root_ino`.
        match client
            .call(
                &FsKitEnvelope::new(FsKitRequest::LookupItem {
                    parent: cap_root_ino,
                    name: "..".into(),
                })
                .with_cap_id("cap_turn1")
                .with_audit_token(token),
            )
            .unwrap()
        {
            FsKitResponse::ItemAttributes { attr } => assert_eq!(attr.ino, cap_root_ino),
            other => panic!("expected clamped parent attr, got {other:?}"),
        }

        // Verify audit_token PID/UID/GID and capability_root reached the Volume AccessHook.
        let callers = hook.seen_callers.lock().unwrap();
        assert!(callers.iter().any(|c| {
            c.pid == 7777
                && c.uid == 502
                && c.gid == 20
                && c.capability_root.as_deref() == Some("/workspaces/turn1")
        }));
        drop(callers);

        // Read-only capability rejects write attempts with EROFS.
        let secret_ino = match client
            .call(
                &FsKitEnvelope::new(FsKitRequest::LookupItem {
                    parent: cap_root_ino,
                    name: "secret.txt".into(),
                })
                .with_cap_id("cap_turn1")
                .with_audit_token(token),
            )
            .unwrap()
        {
            FsKitResponse::ItemAttributes { attr } => attr.ino,
            other => panic!("expected secret.txt attr, got {other:?}"),
        };
        match client
            .call(
                &FsKitEnvelope::new(FsKitRequest::Write {
                    item: secret_ino,
                    handle: None,
                    offset: 0,
                    data: b"nope".to_vec(),
                    expected_generation: None,
                })
                .with_cap_id("cap_turn1")
                .with_audit_token(token),
            )
            .unwrap()
        {
            FsKitResponse::Error { errno: e, .. } => assert_eq!(e, errno::EROFS),
            other => panic!("expected EROFS, got {other:?}"),
        }

        // Unknown capability ID is rejected with EACCES.
        match client
            .call(
                &FsKitEnvelope::new(FsKitRequest::LookupItem {
                    parent: ROOT_NODE_ID,
                    name: "cap_unknown".into(),
                })
                .with_audit_token(token),
            )
            .unwrap()
        {
            FsKitResponse::Error { errno: e, .. } => assert_eq!(e, errno::EACCES),
            other => panic!("expected EACCES, got {other:?}"),
        }

        // Revoking turn-1 immediately rejects subsequent requests with EACCES.
        router.revoke_turn("turn-1");
        match client
            .call(
                &FsKitEnvelope::new(FsKitRequest::GetAttributes { item: secret_ino })
                    .with_cap_id("cap_turn1")
                    .with_audit_token(token),
            )
            .unwrap()
        {
            FsKitResponse::Error { errno: e, .. } => assert_eq!(e, errno::EACCES),
            other => panic!("expected EACCES after revocation, got {other:?}"),
        }

        server.shutdown();
    }
}
