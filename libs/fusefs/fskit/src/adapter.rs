//! Auto-translating [`FsKitAdapter`] bridging macOS 26.7+ FSKit operations onto
//! any [`fusefs_vfs::Volume`] implementation.

use std::sync::Arc;

use fusefs_host::{CAPABILITY_PREFIX, CapabilityRouter};
use fusefs_vfs::{
    CallerContext, FileAttr, FileHandle, NodeId, OpenFlags, ROOT_NODE_ID, VfsError, VfsResult,
    VirtualPath, Volume,
};

use crate::credentials::PeerCredentials;
use crate::protocol::{
    FSKIT_PROTOCOL_VERSION, FsItemKind, FsKitEnvelope, FsKitRequest, FsKitResponse, ProbeResult,
};

/// Translates FSKit `FSUnaryFileSystem` / `FSVolume` / `FSItem` requests into
/// [`fusefs_vfs::Volume`] calls with caller credential extraction and optional
/// `cap_<id>` capability-root clamping.
pub struct FsKitAdapter<V: Volume> {
    volume: Arc<V>,
    router: Option<Arc<CapabilityRouter>>,
    url_scheme: String,
    volume_name: String,
}

impl<V: Volume> FsKitAdapter<V> {
    /// Creates an adapter over `volume` accepting `url_scheme://...` resources.
    pub fn new(
        volume: Arc<V>,
        url_scheme: impl Into<String>,
        volume_name: impl Into<String>,
    ) -> Self {
        Self {
            volume,
            router: None,
            url_scheme: url_scheme.into(),
            volume_name: volume_name.into(),
        }
    }

    /// Attaches a [`CapabilityRouter`] for `cap_<id>` root routing and clamping.
    pub fn with_capability_router(mut self, router: Arc<CapabilityRouter>) -> Self {
        self.router = Some(router);
        self
    }

    pub fn volume(&self) -> &Arc<V> {
        &self.volume
    }

    pub fn router(&self) -> Option<&Arc<CapabilityRouter>> {
        self.router.as_ref()
    }

    /// Dispatches a single [`FsKitEnvelope`] using socket `peer` credentials as
    /// fallback when the envelope does not carry an explicit `audit_token_t`.
    pub fn dispatch(&self, envelope: FsKitEnvelope, peer: PeerCredentials) -> FsKitResponse {
        let effective_peer = envelope
            .audit_token
            .map(|t| t.to_peer_credentials())
            .unwrap_or(peer);

        match self.dispatch_inner(envelope, effective_peer) {
            Ok(resp) => resp,
            Err(err) => FsKitResponse::Error {
                errno: err.errno(),
                message: err.to_string(),
            },
        }
    }

    fn resolve_caller(
        &self,
        cap_id: Option<&str>,
        peer: PeerCredentials,
    ) -> VfsResult<(CallerContext, bool)> {
        let base = peer.to_caller_context(None);
        match (cap_id, &self.router) {
            (Some(id), Some(router)) => {
                let (caller, _root, read_only) = router.resolve_cap_id(id, base)?;
                Ok((caller, read_only))
            }
            (Some(id), None) => Err(VfsError::Eacces(format!(
                "capability root '{id}' requested but no capability router is configured"
            ))),
            (None, _) => Ok((base, false)),
        }
    }

    fn resolve_virtual_path(&self, caller: &CallerContext, path: &VirtualPath) -> VfsResult<FileAttr> {
        let mut current = ROOT_NODE_ID;
        for seg in path.segments() {
            let attr = self.volume.lookup(caller, current, seg)?;
            current = attr.ino;
        }
        self.volume.getattr(caller, current)
    }

    fn require_writable(read_only: bool, op: &str) -> VfsResult<()> {
        if read_only {
            return Err(VfsError::Erofs(format!(
                "capability root is read-only; '{op}' denied"
            )));
        }
        Ok(())
    }

    fn dispatch_inner(
        &self,
        envelope: FsKitEnvelope,
        peer: PeerCredentials,
    ) -> VfsResult<FsKitResponse> {
        match envelope.request {
            FsKitRequest::Ping => Ok(FsKitResponse::Pong {
                protocol_version: FSKIT_PROTOCOL_VERSION,
                peer,
                server_pid: std::process::id(),
            }),
            FsKitRequest::ProbeResource { resource_url } => {
                let prefix = format!("{}://", self.url_scheme);
                if let Some(rest) = resource_url.strip_prefix(&prefix)
                    && !rest.is_empty()
                {
                    let authority = rest.split('/').next().unwrap_or(&self.volume_name);
                    Ok(FsKitResponse::Probe {
                        result: ProbeResult::Usable {
                            name: format!("{}-{authority}", self.volume_name),
                            container_id: format!("{}://{authority}", self.url_scheme),
                        },
                    })
                } else {
                    Ok(FsKitResponse::Probe {
                        result: ProbeResult::NotRecognized,
                    })
                }
            }
            FsKitRequest::LookupItem { parent, name } => {
                // Support virtual `cap_<id>` root lookup directly at volume root.
                if parent == ROOT_NODE_ID
                    && name.starts_with(CAPABILITY_PREFIX)
                    && let Some(router) = &self.router
                {
                    let base = peer.to_caller_context(None);
                    let (caller, root_path, _ro) = router.resolve_cap_id(&name, base)?;
                    let attr = self.resolve_virtual_path(&caller, &root_path)?;
                    return Ok(FsKitResponse::ItemAttributes { attr });
                }

                let (caller, _ro) = self.resolve_caller(envelope.cap_id.as_deref(), peer)?;
                let attr = self.volume.lookup(&caller, parent, &name)?;
                Ok(FsKitResponse::ItemAttributes { attr })
            }
            FsKitRequest::GetAttributes { item } => {
                let (caller, _ro) = self.resolve_caller(envelope.cap_id.as_deref(), peer)?;
                let attr = self.volume.getattr(&caller, item)?;
                Ok(FsKitResponse::ItemAttributes { attr })
            }
            FsKitRequest::SetAttributes { item, attr } => {
                let (caller, ro) = self.resolve_caller(envelope.cap_id.as_deref(), peer)?;
                Self::require_writable(ro, "setAttributes")?;
                let updated = self.volume.setattr(&caller, item, attr)?;
                Ok(FsKitResponse::ItemAttributes { attr: updated })
            }
            FsKitRequest::EnumerateDirectory {
                directory,
                cookie,
                max_entries,
            } => {
                let (caller, _ro) = self.resolve_caller(envelope.cap_id.as_deref(), peer)?;
                let mut entries = self.volume.readdir(&caller, directory, cookie)?;
                let total = entries.len();
                let eof = match max_entries {
                    Some(limit) if limit < total => {
                        entries.truncate(limit);
                        false
                    }
                    _ => true,
                };
                let next_cookie = entries
                    .last()
                    .map(|e| e.offset)
                    .unwrap_or(cookie);
                Ok(FsKitResponse::DirectoryEntries {
                    entries,
                    next_cookie,
                    eof,
                })
            }
            FsKitRequest::ReadSymbolicLink { item } => {
                let (caller, _ro) = self.resolve_caller(envelope.cap_id.as_deref(), peer)?;
                let target = self.volume.readlink(&caller, item)?;
                Ok(FsKitResponse::SymbolicLink { target })
            }
            FsKitRequest::CreateItem {
                parent,
                name,
                kind,
                mode,
                symlink_target,
            } => {
                let (caller, ro) = self.resolve_caller(envelope.cap_id.as_deref(), peer)?;
                Self::require_writable(ro, "createItem")?;
                let attr = match kind {
                    FsItemKind::Directory => self.volume.mkdir(&caller, parent, &name, mode)?,
                    FsItemKind::Symlink => {
                        let target = symlink_target.ok_or_else(|| {
                            VfsError::Einval("symlink creation requires symlink_target".into())
                        })?;
                        self.volume.symlink(&caller, parent, &name, &target)?
                    }
                    FsItemKind::Regular => {
                        return Err(VfsError::Enosys(
                            "regular file creation is not exposed on generic Volume trait".into(),
                        ));
                    }
                };
                Ok(FsKitResponse::ItemAttributes { attr })
            }
            FsKitRequest::RemoveItem {
                parent,
                name,
                is_directory,
            } => {
                let (caller, ro) = self.resolve_caller(envelope.cap_id.as_deref(), peer)?;
                Self::require_writable(ro, "removeItem")?;
                if is_directory {
                    self.volume.rmdir(&caller, parent, &name)?;
                } else {
                    self.volume.unlink(&caller, parent, &name)?;
                }
                Ok(FsKitResponse::Ok)
            }
            FsKitRequest::RenameItem {
                source_parent,
                source_name,
                dest_parent,
                dest_name,
            } => {
                let (caller, ro) = self.resolve_caller(envelope.cap_id.as_deref(), peer)?;
                Self::require_writable(ro, "renameItem")?;
                self.volume
                    .rename(&caller, source_parent, &source_name, dest_parent, &dest_name)?;
                Ok(FsKitResponse::Ok)
            }
            FsKitRequest::OpenItem { item, flags } => {
                let (caller, ro) = self.resolve_caller(envelope.cap_id.as_deref(), peer)?;
                if flags.requires_write() {
                    Self::require_writable(ro, "openItem(write)")?;
                }
                let handle = self.volume.open(&caller, item, flags)?;
                Ok(FsKitResponse::Opened { handle })
            }
            FsKitRequest::CloseItem { item, handle } => {
                let (caller, _ro) = self.resolve_caller(envelope.cap_id.as_deref(), peer)?;
                self.volume.release(&caller, item, handle)?;
                Ok(FsKitResponse::Ok)
            }
            FsKitRequest::Read {
                item,
                handle,
                offset,
                length,
            } => {
                let (caller, _ro) = self.resolve_caller(envelope.cap_id.as_deref(), peer)?;
                let data = self.with_handle(&caller, item, handle, OpenFlags::read_only(), |fh| {
                    self.volume.read(&caller, item, fh, offset, length)
                })?;
                Ok(FsKitResponse::ReadData { data })
            }
            FsKitRequest::Write {
                item,
                handle,
                offset,
                data,
                expected_generation,
            } => {
                let (caller, ro) = self.resolve_caller(envelope.cap_id.as_deref(), peer)?;
                Self::require_writable(ro, "write")?;
                let bytes_written =
                    self.with_handle(&caller, item, handle, OpenFlags::read_write(), |fh| {
                        self.volume
                            .write(&caller, item, fh, offset, &data, expected_generation)
                    })?;
                Ok(FsKitResponse::Written { bytes_written })
            }
            FsKitRequest::Synchronize {
                item,
                handle,
                datasync,
            } => {
                let (caller, _ro) = self.resolve_caller(envelope.cap_id.as_deref(), peer)?;
                self.with_handle(&caller, item, handle, OpenFlags::read_only(), |fh| {
                    self.volume.fsync(&caller, item, fh, datasync)
                })?;
                Ok(FsKitResponse::Ok)
            }
            FsKitRequest::VolumeStatistics { item } => {
                let (caller, _ro) = self.resolve_caller(envelope.cap_id.as_deref(), peer)?;
                let target = item.unwrap_or(ROOT_NODE_ID);
                let stats = self.volume.statfs(&caller, target)?;
                Ok(FsKitResponse::Statistics { stats })
            }
        }
    }

    fn with_handle<T, F>(
        &self,
        caller: &CallerContext,
        item: NodeId,
        existing: Option<FileHandle>,
        fallback_flags: OpenFlags,
        f: F,
    ) -> VfsResult<T>
    where
        F: FnOnce(FileHandle) -> VfsResult<T>,
    {
        if let Some(fh) = existing {
            return f(fh);
        }
        let fh = self.volume.open(caller, item, fallback_flags)?;
        let res = f(fh);
        let _ = self.volume.release(caller, item, fh);
        res
    }
}
