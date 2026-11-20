//! Generic [`Volume`] and [`AccessHook`] traits plus an [`InMemoryVolume`] & [`TreeBuilder`].
//!
//! The [`Volume`] trait explicitly separates:
//! 1. **Zero-data metadata & control RPC operations** (`lookup`, `getattr`, `setattr`,
//!    `readdir`, `readlink`, `mkdir`, `rmdir`, `unlink`, `rename`, `symlink`, `link`,
//!    `getxattr`, `listxattr`, `setxattr`, `removexattr`, `statfs`, `copy_file_range`,
//!    `open`, `release`, `fsync`, `flock`), which never require reading file payload chunks.
//! 2. **Chunk-data operations** (`read`, `write`), which transfer file content bytes.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, RwLock};

use crate::error::{VfsError, VfsResult};
use crate::lease::{LeaseTable, check_generation};
use crate::path::VirtualPath;
use crate::types::{
    CallerContext, DirEntry, FileAttr, FileHandle, FileKind, FlockOp, NodeId,
    OMITTED_STUB_FILENAME, OpenFlags, ROOT_NODE_ID, SetAttr, StatFs, SyntheticStub, Timestamp,
};

/// Metadata/control operation kind passed to [`AccessHook::before_metadata`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MetadataOp {
    Lookup,
    GetAttr,
    SetAttr,
    ReadDir,
    ReadLink,
    Mkdir,
    Rmdir,
    Unlink,
    Rename,
    Symlink,
    Link,
    GetXattr,
    ListXattr,
    SetXattr,
    RemoveXattr,
    StatFs,
    CopyFileRange,
    Open,
    Release,
    Fsync,
    Flock,
}

/// Extensible access-control / audit hook invoked before volume operations.
pub trait AccessHook: Send + Sync {
    fn before_metadata(
        &self,
        _caller: &CallerContext,
        _op: MetadataOp,
        _path: &VirtualPath,
    ) -> VfsResult<()> {
        Ok(())
    }

    fn before_read(
        &self,
        _caller: &CallerContext,
        _path: &VirtualPath,
        _offset: u64,
        _size: u32,
    ) -> VfsResult<()> {
        Ok(())
    }

    fn before_write(
        &self,
        _caller: &CallerContext,
        _path: &VirtualPath,
        _offset: u64,
        _size: usize,
    ) -> VfsResult<()> {
        Ok(())
    }
}

/// Default no-op [`AccessHook`].
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopAccessHook;

impl AccessHook for NoopAccessHook {}

/// Callback used by lazy/chunk-backed files in [`InMemoryVolume`] to materialize
/// byte ranges on demand during `read`, without ever being invoked by metadata ops.
pub type ChunkLoader = Arc<dyn Fn(u64, u32) -> VfsResult<Vec<u8>> + Send + Sync>;

/// Generic virtual filesystem volume trait.
pub trait Volume: Send + Sync {
    // -------------------------------------------------------------------------
    // Zero-data metadata & control RPC operations
    // -------------------------------------------------------------------------

    fn lookup(&self, caller: &CallerContext, parent: NodeId, name: &str) -> VfsResult<FileAttr>;

    fn getattr(&self, caller: &CallerContext, ino: NodeId) -> VfsResult<FileAttr>;

    fn setattr(&self, caller: &CallerContext, ino: NodeId, attr: SetAttr) -> VfsResult<FileAttr>;

    fn readdir(&self, caller: &CallerContext, ino: NodeId, offset: u64) -> VfsResult<Vec<DirEntry>>;

    fn readlink(&self, caller: &CallerContext, ino: NodeId) -> VfsResult<String>;

    fn mkdir(
        &self,
        caller: &CallerContext,
        parent: NodeId,
        name: &str,
        mode: u32,
    ) -> VfsResult<FileAttr>;

    fn rmdir(&self, caller: &CallerContext, parent: NodeId, name: &str) -> VfsResult<()>;

    fn unlink(&self, caller: &CallerContext, parent: NodeId, name: &str) -> VfsResult<()>;

    fn rename(
        &self,
        caller: &CallerContext,
        parent: NodeId,
        name: &str,
        new_parent: NodeId,
        new_name: &str,
    ) -> VfsResult<()>;

    fn symlink(
        &self,
        caller: &CallerContext,
        parent: NodeId,
        name: &str,
        target: &str,
    ) -> VfsResult<FileAttr>;

    fn link(
        &self,
        caller: &CallerContext,
        ino: NodeId,
        new_parent: NodeId,
        new_name: &str,
    ) -> VfsResult<FileAttr>;

    fn getxattr(&self, caller: &CallerContext, ino: NodeId, name: &str) -> VfsResult<Vec<u8>>;

    fn listxattr(&self, caller: &CallerContext, ino: NodeId) -> VfsResult<Vec<String>>;

    fn setxattr(
        &self,
        caller: &CallerContext,
        ino: NodeId,
        name: &str,
        value: &[u8],
    ) -> VfsResult<()>;

    fn removexattr(&self, caller: &CallerContext, ino: NodeId, name: &str) -> VfsResult<()>;

    fn statfs(&self, caller: &CallerContext, ino: NodeId) -> VfsResult<StatFs>;

    fn copy_file_range(
        &self,
        caller: &CallerContext,
        ino_in: NodeId,
        offset_in: u64,
        ino_out: NodeId,
        offset_out: u64,
        len: u64,
    ) -> VfsResult<u64>;

    fn open(&self, caller: &CallerContext, ino: NodeId, flags: OpenFlags) -> VfsResult<FileHandle>;

    fn release(&self, caller: &CallerContext, ino: NodeId, fh: FileHandle) -> VfsResult<()>;

    fn fsync(
        &self,
        caller: &CallerContext,
        ino: NodeId,
        fh: FileHandle,
        datasync: bool,
    ) -> VfsResult<()>;

    fn flock(
        &self,
        caller: &CallerContext,
        ino: NodeId,
        fh: FileHandle,
        op: FlockOp,
        non_blocking: bool,
    ) -> VfsResult<()>;

    // -------------------------------------------------------------------------
    // Chunk-data operations
    // -------------------------------------------------------------------------

    fn read(
        &self,
        caller: &CallerContext,
        ino: NodeId,
        fh: FileHandle,
        offset: u64,
        size: u32,
    ) -> VfsResult<Vec<u8>>;

    fn write(
        &self,
        caller: &CallerContext,
        ino: NodeId,
        fh: FileHandle,
        offset: u64,
        data: &[u8],
        expected_generation: Option<u64>,
    ) -> VfsResult<u32>;
}

// =============================================================================
// InMemoryVolume & TreeBuilder
// =============================================================================

#[derive(Clone)]
enum FileContent {
    Inline(Vec<u8>),
    Lazy {
        loader: ChunkLoader,
        /// Optional overlay patches applied after copy_file_range or writes.
        /// Kept separate so metadata-only `truncate` / `copy_file_range` never read chunks.
        overlay: Vec<OverlaySpan>,
    },
}

#[derive(Clone)]
struct OverlaySpan {
    dst_offset: u64,
    len: u64,
    source: OverlaySource,
}

#[derive(Clone)]
enum OverlaySource {
    Bytes(Vec<u8>),
    LazyRef { loader: ChunkLoader, src_offset: u64 },
}

impl FileContent {
    fn read_range(&self, file_size: u64, offset: u64, size: u32) -> VfsResult<Vec<u8>> {
        if offset >= file_size || size == 0 {
            return Ok(Vec::new());
        }
        let max_len = (file_size - offset).min(u64::from(size)) as usize;
        match self {
            FileContent::Inline(bytes) => {
                let start = offset as usize;
                let mut out = vec![0u8; max_len];
                if start < bytes.len() {
                    let avail = (bytes.len() - start).min(max_len);
                    out[..avail].copy_from_slice(&bytes[start..start + avail]);
                }
                Ok(out)
            }
            FileContent::Lazy { loader, overlay } => {
                let mut out = if overlay.is_empty() {
                    let mut raw = loader(offset, max_len as u32)?;
                    raw.resize(max_len, 0);
                    raw
                } else {
                    // Check if any overlay fully covers or partially covers the range.
                    let mut buf = vec![0u8; max_len];
                    let mut covered = vec![false; max_len];
                    // Apply overlays in reverse order (latest write wins).
                    for span in overlay.iter().rev() {
                        let span_end = span.dst_offset.saturating_add(span.len);
                        let req_end = offset.saturating_add(max_len as u64);
                        let ov_start = offset.max(span.dst_offset);
                        let ov_end = req_end.min(span_end);
                        if ov_start >= ov_end {
                            continue;
                        }
                        let rel_dst = (ov_start - offset) as usize;
                        let rel_len = (ov_end - ov_start) as usize;
                        let rel_src = ov_start - span.dst_offset;
                        match &span.source {
                            OverlaySource::Bytes(data) => {
                                let s = rel_src as usize;
                                for i in 0..rel_len {
                                    if !covered[rel_dst + i] {
                                        buf[rel_dst + i] = data.get(s + i).copied().unwrap_or(0);
                                        covered[rel_dst + i] = true;
                                    }
                                }
                            }
                            OverlaySource::LazyRef {
                                loader: src_loader,
                                src_offset,
                            } => {
                                let fetched = src_loader(
                                    src_offset.saturating_add(rel_src),
                                    rel_len as u32,
                                )?;
                                for i in 0..rel_len {
                                    if !covered[rel_dst + i] {
                                        buf[rel_dst + i] = fetched.get(i).copied().unwrap_or(0);
                                        covered[rel_dst + i] = true;
                                    }
                                }
                            }
                        }
                    }
                    if covered.iter().any(|c| !*c) {
                        let base = loader(offset, max_len as u32)?;
                        for i in 0..max_len {
                            if !covered[i] {
                                buf[i] = base.get(i).copied().unwrap_or(0);
                            }
                        }
                    }
                    buf
                };
                out.truncate(max_len);
                Ok(out)
            }
        }
    }

    fn write_at(&mut self, offset: u64, data: &[u8]) {
        match self {
            FileContent::Inline(bytes) => {
                let end = (offset as usize).saturating_add(data.len());
                if bytes.len() < end {
                    bytes.resize(end, 0);
                }
                bytes[offset as usize..end].copy_from_slice(data);
            }
            FileContent::Lazy { overlay, .. } => {
                overlay.push(OverlaySpan {
                    dst_offset: offset,
                    len: data.len() as u64,
                    source: OverlaySource::Bytes(data.to_vec()),
                });
            }
        }
    }

    fn truncate(&mut self, new_size: u64) {
        match self {
            FileContent::Inline(bytes) => {
                bytes.resize(new_size as usize, 0);
            }
            FileContent::Lazy { overlay, .. } => {
                // Trim overlays beyond new_size without touching the underlying loader.
                overlay.retain_mut(|span| {
                    if span.dst_offset >= new_size {
                        return false;
                    }
                    let max_len = new_size - span.dst_offset;
                    if span.len > max_len {
                        span.len = max_len;
                        if let OverlaySource::Bytes(b) = &mut span.source {
                            b.truncate(max_len as usize);
                        }
                    }
                    true
                });
            }
        }
    }
}

enum NodeData {
    Directory {
        parent: NodeId,
        children: BTreeMap<String, NodeId>,
    },
    Regular {
        content: FileContent,
    },
    Symlink {
        target: String,
    },
}

struct Node {
    attr: FileAttr,
    path: VirtualPath,
    xattrs: BTreeMap<String, Vec<u8>>,
    data: NodeData,
}

#[derive(Debug, Clone, Copy)]
struct OpenHandle {
    ino: NodeId,
    flags: OpenFlags,
}

#[derive(Debug, Default)]
struct LockState {
    shared: BTreeSet<FileHandle>,
    exclusive: Option<FileHandle>,
}

struct VolumeState {
    next_ino: u64,
    next_fh: u64,
    nodes: BTreeMap<NodeId, Node>,
    handles: BTreeMap<FileHandle, OpenHandle>,
    locks: BTreeMap<NodeId, LockState>,
    protected_roots: BTreeSet<VirtualPath>,
}

/// Thread-safe in-memory [`Volume`] supporting symlinks, lazy chunk-backed files,
/// synthetic `.omitted.json` stubs, protected root paths, [`AccessHook`], and
/// [`LeaseTable`] enforcement.
pub struct InMemoryVolume {
    state: RwLock<VolumeState>,
    leases: Option<Arc<LeaseTable>>,
    access_hook: Arc<dyn AccessHook>,
    clock: Arc<dyn Fn() -> u64 + Send + Sync>,
}

impl InMemoryVolume {
    /// Starts constructing an [`InMemoryVolume`] using [`TreeBuilder`].
    pub fn builder() -> TreeBuilder {
        TreeBuilder::new()
    }

    /// Resolves `raw_path` using `caller`'s capability-root clamping (if set) and returns
    /// the target node's attributes without reading any file chunk data.
    pub fn resolve_path(&self, caller: &CallerContext, raw_path: &str) -> VfsResult<FileAttr> {
        let cap = caller.capability_root_path()?;
        let normalized = match &cap {
            Some(root) => VirtualPath::parse_clamped(raw_path, root)?,
            None => VirtualPath::parse(raw_path)?,
        };
        let mut current = ROOT_NODE_ID;
        for seg in normalized.segments() {
            let attr = self.lookup(caller, current, seg)?;
            current = attr.ino;
        }
        self.getattr(caller, current)
    }

    /// Returns the current canonical [`VirtualPath`] of a node.
    pub fn node_path(&self, ino: NodeId) -> VfsResult<VirtualPath> {
        let state = self.state.read().expect("volume lock poisoned");
        state
            .nodes
            .get(&ino)
            .map(|n| n.path.clone())
            .ok_or_else(|| VfsError::Enoent(format!("unknown node {ino}")))
    }

    fn now_ms(&self) -> u64 {
        (self.clock)()
    }

    fn now_ts(&self) -> Timestamp {
        Timestamp::from_millis(self.now_ms())
    }

    fn authorize_write(&self, caller: &CallerContext, path: &VirtualPath) -> VfsResult<()> {
        if let Some(leases) = &self.leases {
            leases.authorize_write(caller, path, self.now_ms())?;
        }
        Ok(())
    }

    fn is_protected(state: &VolumeState, path: &VirtualPath) -> bool {
        path.is_root() || state.protected_roots.contains(path)
    }

    fn validate_name(name: &str) -> VfsResult<()> {
        if name.is_empty() || name.contains('/') || name.as_bytes().contains(&0) {
            return Err(VfsError::Einval(format!("invalid entry name '{name}'")));
        }
        if name == "." || name == ".." {
            return Err(VfsError::Einval(format!(
                "cannot create or modify special entry '{name}'"
            )));
        }
        Ok(())
    }

    fn update_subtree_paths(state: &mut VolumeState, ino: NodeId, new_path: VirtualPath) {
        let children: Vec<(String, NodeId)> = match state.nodes.get_mut(&ino) {
            Some(node) => {
                node.path = new_path.clone();
                match &node.data {
                    NodeData::Directory { children, .. } => {
                        children.iter().map(|(k, v)| (k.clone(), *v)).collect()
                    }
                    _ => Vec::new(),
                }
            }
            None => return,
        };
        for (name, child_ino) in children {
            if let Ok(child_path) = new_path.join(&name) {
                Self::update_subtree_paths(state, child_ino, child_path);
            }
        }
    }
}

impl Volume for InMemoryVolume {
    fn lookup(&self, caller: &CallerContext, parent: NodeId, name: &str) -> VfsResult<FileAttr> {
        if name.is_empty() || name.contains('/') || name.as_bytes().contains(&0) {
            return Err(VfsError::Einval(format!("invalid lookup name '{name}'")));
        }

        let (target_ino, target_path) = {
            let state = self.state.read().expect("volume lock poisoned");
            let parent_node = state
                .nodes
                .get(&parent)
                .ok_or_else(|| VfsError::Enoent(format!("parent {parent} not found")))?;
            let (parent_of_parent, children) = match &parent_node.data {
                NodeData::Directory { parent, children } => (*parent, children),
                _ => {
                    return Err(VfsError::Enotdir(format!(
                        "'{}' is not a directory",
                        parent_node.path
                    )));
                }
            };

            let target_ino = match name {
                "." => parent,
                ".." => {
                    // Enforce capability-root clamping on `..` traversal.
                    if let Some(cap) = caller.capability_root_path()?
                        && (!parent_node.path.is_within(&cap) || parent_node.path == cap)
                    {
                        parent
                    } else {
                        parent_of_parent
                    }
                }
                _ => *children.get(name).ok_or_else(|| {
                    VfsError::Enoent(format!("'{name}' not found in '{}'", parent_node.path))
                })?,
            };

            let target_node = state
                .nodes
                .get(&target_ino)
                .ok_or_else(|| VfsError::Enoent(format!("node {target_ino} not found")))?;
            (target_ino, target_node.path.clone())
        };

        self.access_hook
            .before_metadata(caller, MetadataOp::Lookup, &target_path)?;

        let state = self.state.read().expect("volume lock poisoned");
        state
            .nodes
            .get(&target_ino)
            .map(|n| n.attr.clone())
            .ok_or_else(|| VfsError::Enoent(format!("node {target_ino} not found")))
    }

    fn getattr(&self, caller: &CallerContext, ino: NodeId) -> VfsResult<FileAttr> {
        let (path, attr) = {
            let state = self.state.read().expect("volume lock poisoned");
            let node = state
                .nodes
                .get(&ino)
                .ok_or_else(|| VfsError::Enoent(format!("node {ino} not found")))?;
            (node.path.clone(), node.attr.clone())
        };
        self.access_hook
            .before_metadata(caller, MetadataOp::GetAttr, &path)?;
        Ok(attr)
    }

    fn setattr(&self, caller: &CallerContext, ino: NodeId, attr: SetAttr) -> VfsResult<FileAttr> {
        let path = self.node_path(ino)?;
        self.access_hook
            .before_metadata(caller, MetadataOp::SetAttr, &path)?;
        self.authorize_write(caller, &path)?;

        let now = self.now_ts();
        let mut state = self.state.write().expect("volume lock poisoned");
        let node = state
            .nodes
            .get_mut(&ino)
            .ok_or_else(|| VfsError::Enoent(format!("node {ino} not found")))?;

        check_generation(attr.expected_generation, node.attr.generation)?;

        if let Some(mode) = attr.mode {
            node.attr.mode = mode;
        }
        if let Some(uid) = attr.uid {
            node.attr.uid = uid;
        }
        if let Some(gid) = attr.gid {
            node.attr.gid = gid;
        }
        if let Some(new_size) = attr.size {
            match &mut node.data {
                NodeData::Regular { content } => {
                    content.truncate(new_size);
                    node.attr.size = new_size;
                    node.attr.blocks = FileAttr::blocks_for_size(new_size);
                    node.attr.mtime = now;
                }
                NodeData::Directory { .. } => {
                    return Err(VfsError::Eisdir(format!(
                        "cannot truncate directory '{}'",
                        node.path
                    )));
                }
                NodeData::Symlink { .. } => {
                    return Err(VfsError::Einval(format!(
                        "cannot truncate symlink '{}'",
                        node.path
                    )));
                }
            }
        }
        if let Some(atime) = attr.atime {
            node.attr.atime = atime;
        }
        if let Some(mtime) = attr.mtime {
            node.attr.mtime = mtime;
        }
        node.attr.ctime = now;
        node.attr.generation = node.attr.generation.saturating_add(1);
        Ok(node.attr.clone())
    }

    fn readdir(&self, caller: &CallerContext, ino: NodeId, offset: u64) -> VfsResult<Vec<DirEntry>> {
        let (path, entries) = {
            let state = self.state.read().expect("volume lock poisoned");
            let node = state
                .nodes
                .get(&ino)
                .ok_or_else(|| VfsError::Enoent(format!("node {ino} not found")))?;
            let (parent_ino, children) = match &node.data {
                NodeData::Directory { parent, children } => (*parent, children),
                _ => {
                    return Err(VfsError::Enotdir(format!(
                        "'{}' is not a directory",
                        node.path
                    )));
                }
            };

            let mut list = Vec::with_capacity(children.len() + 2);
            list.push(DirEntry {
                ino,
                kind: FileKind::Directory,
                name: ".".to_string(),
                offset: 1,
            });
            list.push(DirEntry {
                ino: parent_ino,
                kind: FileKind::Directory,
                name: "..".to_string(),
                offset: 2,
            });
            for (idx, (name, &child_ino)) in children.iter().enumerate() {
                if let Some(child) = state.nodes.get(&child_ino) {
                    list.push(DirEntry {
                        ino: child_ino,
                        kind: child.attr.kind,
                        name: name.clone(),
                        offset: (idx as u64) + 3,
                    });
                }
            }
            (node.path.clone(), list)
        };

        self.access_hook
            .before_metadata(caller, MetadataOp::ReadDir, &path)?;

        Ok(entries
            .into_iter()
            .skip(offset as usize)
            .collect())
    }

    fn readlink(&self, caller: &CallerContext, ino: NodeId) -> VfsResult<String> {
        let (path, target) = {
            let state = self.state.read().expect("volume lock poisoned");
            let node = state
                .nodes
                .get(&ino)
                .ok_or_else(|| VfsError::Enoent(format!("node {ino} not found")))?;
            match &node.data {
                NodeData::Symlink { target } => (node.path.clone(), target.clone()),
                _ => {
                    return Err(VfsError::Einval(format!(
                        "'{}' is not a symlink",
                        node.path
                    )));
                }
            }
        };
        self.access_hook
            .before_metadata(caller, MetadataOp::ReadLink, &path)?;
        Ok(target)
    }

    fn mkdir(
        &self,
        caller: &CallerContext,
        parent: NodeId,
        name: &str,
        mode: u32,
    ) -> VfsResult<FileAttr> {
        Self::validate_name(name)?;
        let child_path = self.node_path(parent)?.join(name)?;
        self.access_hook
            .before_metadata(caller, MetadataOp::Mkdir, &child_path)?;
        self.authorize_write(caller, &child_path)?;

        let now = self.now_ts();
        let mut state = self.state.write().expect("volume lock poisoned");
        {
            let parent_node = state
                .nodes
                .get(&parent)
                .ok_or_else(|| VfsError::Enoent(format!("parent {parent} not found")))?;
            match &parent_node.data {
                NodeData::Directory { children, .. } => {
                    if children.contains_key(name) {
                        return Err(VfsError::Eexist(format!("'{child_path}' already exists")));
                    }
                }
                _ => {
                    return Err(VfsError::Enotdir(format!(
                        "'{}' is not a directory",
                        parent_node.path
                    )));
                }
            }
        }

        let ino = NodeId(state.next_ino);
        state.next_ino = state.next_ino.saturating_add(1);
        let attr = FileAttr {
            ino,
            kind: FileKind::Directory,
            size: 0,
            blocks: 0,
            atime: now,
            mtime: now,
            ctime: now,
            mode,
            nlink: 2,
            uid: caller.uid,
            gid: caller.gid,
            generation: 1,
        };
        state.nodes.insert(
            ino,
            Node {
                attr: attr.clone(),
                path: child_path,
                xattrs: BTreeMap::new(),
                data: NodeData::Directory {
                    parent,
                    children: BTreeMap::new(),
                },
            },
        );
        if let Some(parent_node) = state.nodes.get_mut(&parent) {
            if let NodeData::Directory { children, .. } = &mut parent_node.data {
                children.insert(name.to_string(), ino);
            }
            parent_node.attr.nlink = parent_node.attr.nlink.saturating_add(1);
            parent_node.attr.mtime = now;
            parent_node.attr.ctime = now;
        }
        Ok(attr)
    }

    fn rmdir(&self, caller: &CallerContext, parent: NodeId, name: &str) -> VfsResult<()> {
        Self::validate_name(name)?;
        let child_path = self.node_path(parent)?.join(name)?;
        self.access_hook
            .before_metadata(caller, MetadataOp::Rmdir, &child_path)?;
        self.authorize_write(caller, &child_path)?;

        let now = self.now_ts();
        let mut state = self.state.write().expect("volume lock poisoned");
        let target_ino = {
            let parent_node = state
                .nodes
                .get(&parent)
                .ok_or_else(|| VfsError::Enoent(format!("parent {parent} not found")))?;
            match &parent_node.data {
                NodeData::Directory { children, .. } => *children
                    .get(name)
                    .ok_or_else(|| VfsError::Enoent(format!("'{child_path}' not found")))?,
                _ => {
                    return Err(VfsError::Enotdir(format!(
                        "'{}' is not a directory",
                        parent_node.path
                    )));
                }
            }
        };

        let target_node = state
            .nodes
            .get(&target_ino)
            .ok_or_else(|| VfsError::Enoent(format!("node {target_ino} not found")))?;
        if Self::is_protected(&state, &target_node.path) {
            return Err(VfsError::Eperm(format!(
                "cannot rmdir protected root '{}'",
                target_node.path
            )));
        }
        match &target_node.data {
            NodeData::Directory { children, .. } => {
                if !children.is_empty() {
                    return Err(VfsError::Enotempty(format!(
                        "directory '{}' is not empty",
                        target_node.path
                    )));
                }
            }
            _ => {
                return Err(VfsError::Enotdir(format!(
                    "'{}' is not a directory",
                    target_node.path
                )));
            }
        }

        if let Some(parent_node) = state.nodes.get_mut(&parent) {
            if let NodeData::Directory { children, .. } = &mut parent_node.data {
                children.remove(name);
            }
            parent_node.attr.nlink = parent_node.attr.nlink.saturating_sub(1);
            parent_node.attr.mtime = now;
            parent_node.attr.ctime = now;
        }
        state.nodes.remove(&target_ino);
        Ok(())
    }

    fn unlink(&self, caller: &CallerContext, parent: NodeId, name: &str) -> VfsResult<()> {
        Self::validate_name(name)?;
        let child_path = self.node_path(parent)?.join(name)?;
        self.access_hook
            .before_metadata(caller, MetadataOp::Unlink, &child_path)?;
        self.authorize_write(caller, &child_path)?;

        let now = self.now_ts();
        let mut state = self.state.write().expect("volume lock poisoned");
        let target_ino = {
            let parent_node = state
                .nodes
                .get(&parent)
                .ok_or_else(|| VfsError::Enoent(format!("parent {parent} not found")))?;
            match &parent_node.data {
                NodeData::Directory { children, .. } => *children
                    .get(name)
                    .ok_or_else(|| VfsError::Enoent(format!("'{child_path}' not found")))?,
                _ => {
                    return Err(VfsError::Enotdir(format!(
                        "'{}' is not a directory",
                        parent_node.path
                    )));
                }
            }
        };

        let target_node = state
            .nodes
            .get(&target_ino)
            .ok_or_else(|| VfsError::Enoent(format!("node {target_ino} not found")))?;
        if Self::is_protected(&state, &target_node.path) {
            return Err(VfsError::Eperm(format!(
                "cannot unlink protected root '{}'",
                target_node.path
            )));
        }
        if matches!(target_node.data, NodeData::Directory { .. }) {
            return Err(VfsError::Eisdir(format!(
                "cannot unlink directory '{}'",
                target_node.path
            )));
        }

        if let Some(parent_node) = state.nodes.get_mut(&parent) {
            if let NodeData::Directory { children, .. } = &mut parent_node.data {
                children.remove(name);
            }
            parent_node.attr.mtime = now;
            parent_node.attr.ctime = now;
        }
        let remove_node = if let Some(node) = state.nodes.get_mut(&target_ino) {
            node.attr.nlink = node.attr.nlink.saturating_sub(1);
            node.attr.ctime = now;
            node.attr.nlink == 0
        } else {
            false
        };
        if remove_node {
            state.nodes.remove(&target_ino);
        }
        Ok(())
    }

    fn rename(
        &self,
        caller: &CallerContext,
        parent: NodeId,
        name: &str,
        new_parent: NodeId,
        new_name: &str,
    ) -> VfsResult<()> {
        Self::validate_name(name)?;
        Self::validate_name(new_name)?;
        let src_path = self.node_path(parent)?.join(name)?;
        let dst_path = self.node_path(new_parent)?.join(new_name)?;

        self.access_hook
            .before_metadata(caller, MetadataOp::Rename, &src_path)?;
        self.access_hook
            .before_metadata(caller, MetadataOp::Rename, &dst_path)?;
        self.authorize_write(caller, &src_path)?;
        self.authorize_write(caller, &dst_path)?;

        let now = self.now_ts();
        let mut state = self.state.write().expect("volume lock poisoned");

        let src_ino = {
            let p = state
                .nodes
                .get(&parent)
                .ok_or_else(|| VfsError::Enoent(format!("parent {parent} not found")))?;
            match &p.data {
                NodeData::Directory { children, .. } => *children
                    .get(name)
                    .ok_or_else(|| VfsError::Enoent(format!("'{src_path}' not found")))?,
                _ => {
                    return Err(VfsError::Enotdir(format!("'{}' is not a directory", p.path)));
                }
            }
        };

        let src_actual_path = state
            .nodes
            .get(&src_ino)
            .ok_or_else(|| VfsError::Enoent(format!("node {src_ino} not found")))?
            .path
            .clone();
        if Self::is_protected(&state, &src_actual_path) {
            return Err(VfsError::Eperm(format!(
                "cannot rename protected root '{src_actual_path}'"
            )));
        }

        let dst_parent_path = {
            let np = state
                .nodes
                .get(&new_parent)
                .ok_or_else(|| VfsError::Enoent(format!("new_parent {new_parent} not found")))?;
            if !matches!(np.data, NodeData::Directory { .. }) {
                return Err(VfsError::Enotdir(format!(
                    "'{}' is not a directory",
                    np.path
                )));
            }
            np.path.clone()
        };

        // Prevent moving a directory into itself or its own descendant.
        if dst_parent_path.is_within(&src_actual_path) {
            return Err(VfsError::Einval(format!(
                "cannot move '{src_actual_path}' into its own descendant '{dst_parent_path}'"
            )));
        }

        // Check if destination already exists.
        let existing_dst_ino = match &state.nodes.get(&new_parent).unwrap().data {
            NodeData::Directory { children, .. } => children.get(new_name).copied(),
            _ => None,
        };
        if let Some(dst_ino) = existing_dst_ino {
            if dst_ino == src_ino {
                return Ok(());
            }
            let dst_node = state.nodes.get(&dst_ino).unwrap();
            if Self::is_protected(&state, &dst_node.path) {
                return Err(VfsError::Eperm(format!(
                    "cannot overwrite protected root '{}'",
                    dst_node.path
                )));
            }
            if let NodeData::Directory { children, .. } = &dst_node.data
                && !children.is_empty()
            {
                return Err(VfsError::Enotempty(format!(
                    "destination directory '{}' is not empty",
                    dst_node.path
                )));
            }
        }

        // Detach from old parent.
        if let Some(p) = state.nodes.get_mut(&parent) {
            if let NodeData::Directory { children, .. } = &mut p.data {
                children.remove(name);
            }
            p.attr.mtime = now;
            p.attr.ctime = now;
        }

        // Attach to new parent.
        if let Some(np) = state.nodes.get_mut(&new_parent) {
            if let NodeData::Directory { children, .. } = &mut np.data {
                children.insert(new_name.to_string(), src_ino);
            }
            np.attr.mtime = now;
            np.attr.ctime = now;
        }

        if let Some(dst_ino) = existing_dst_ino {
            let remove = if let Some(d) = state.nodes.get_mut(&dst_ino) {
                d.attr.nlink = d.attr.nlink.saturating_sub(1);
                d.attr.nlink == 0
            } else {
                false
            };
            if remove {
                state.nodes.remove(&dst_ino);
            }
        }

        if let Some(src_node) = state.nodes.get_mut(&src_ino) {
            if let NodeData::Directory { parent: p, .. } = &mut src_node.data {
                *p = new_parent;
            }
            src_node.attr.ctime = now;
            src_node.attr.generation = src_node.attr.generation.saturating_add(1);
        }

        Self::update_subtree_paths(&mut state, src_ino, dst_path);
        Ok(())
    }

    fn symlink(
        &self,
        caller: &CallerContext,
        parent: NodeId,
        name: &str,
        target: &str,
    ) -> VfsResult<FileAttr> {
        Self::validate_name(name)?;
        let link_path = self.node_path(parent)?.join(name)?;
        self.access_hook
            .before_metadata(caller, MetadataOp::Symlink, &link_path)?;
        self.authorize_write(caller, &link_path)?;

        let now = self.now_ts();
        let mut state = self.state.write().expect("volume lock poisoned");
        {
            let p = state
                .nodes
                .get(&parent)
                .ok_or_else(|| VfsError::Enoent(format!("parent {parent} not found")))?;
            match &p.data {
                NodeData::Directory { children, .. } => {
                    if children.contains_key(name) {
                        return Err(VfsError::Eexist(format!("'{link_path}' already exists")));
                    }
                }
                _ => {
                    return Err(VfsError::Enotdir(format!("'{}' is not a directory", p.path)));
                }
            }
        }

        let ino = NodeId(state.next_ino);
        state.next_ino = state.next_ino.saturating_add(1);
        let size = target.len() as u64;
        let attr = FileAttr {
            ino,
            kind: FileKind::Symlink,
            size,
            blocks: FileAttr::blocks_for_size(size),
            atime: now,
            mtime: now,
            ctime: now,
            mode: 0o777,
            nlink: 1,
            uid: caller.uid,
            gid: caller.gid,
            generation: 1,
        };
        state.nodes.insert(
            ino,
            Node {
                attr: attr.clone(),
                path: link_path,
                xattrs: BTreeMap::new(),
                data: NodeData::Symlink {
                    target: target.to_string(),
                },
            },
        );
        if let Some(p) = state.nodes.get_mut(&parent) {
            if let NodeData::Directory { children, .. } = &mut p.data {
                children.insert(name.to_string(), ino);
            }
            p.attr.mtime = now;
            p.attr.ctime = now;
        }
        Ok(attr)
    }

    fn link(
        &self,
        caller: &CallerContext,
        ino: NodeId,
        new_parent: NodeId,
        new_name: &str,
    ) -> VfsResult<FileAttr> {
        Self::validate_name(new_name)?;
        let link_path = self.node_path(new_parent)?.join(new_name)?;
        self.access_hook
            .before_metadata(caller, MetadataOp::Link, &link_path)?;
        self.authorize_write(caller, &link_path)?;

        let now = self.now_ts();
        let mut state = self.state.write().expect("volume lock poisoned");
        {
            let target = state
                .nodes
                .get(&ino)
                .ok_or_else(|| VfsError::Enoent(format!("node {ino} not found")))?;
            if matches!(target.data, NodeData::Directory { .. }) {
                return Err(VfsError::Eperm(format!(
                    "hard links to directories are not permitted: '{}'",
                    target.path
                )));
            }
        }
        {
            let p = state
                .nodes
                .get(&new_parent)
                .ok_or_else(|| VfsError::Enoent(format!("parent {new_parent} not found")))?;
            match &p.data {
                NodeData::Directory { children, .. } => {
                    if children.contains_key(new_name) {
                        return Err(VfsError::Eexist(format!("'{link_path}' already exists")));
                    }
                }
                _ => {
                    return Err(VfsError::Enotdir(format!("'{}' is not a directory", p.path)));
                }
            }
        }

        if let Some(p) = state.nodes.get_mut(&new_parent) {
            if let NodeData::Directory { children, .. } = &mut p.data {
                children.insert(new_name.to_string(), ino);
            }
            p.attr.mtime = now;
            p.attr.ctime = now;
        }
        let target = state.nodes.get_mut(&ino).unwrap();
        target.attr.nlink = target.attr.nlink.saturating_add(1);
        target.attr.ctime = now;
        Ok(target.attr.clone())
    }

    fn getxattr(&self, caller: &CallerContext, ino: NodeId, name: &str) -> VfsResult<Vec<u8>> {
        let path = self.node_path(ino)?;
        self.access_hook
            .before_metadata(caller, MetadataOp::GetXattr, &path)?;
        let state = self.state.read().expect("volume lock poisoned");
        let node = state
            .nodes
            .get(&ino)
            .ok_or_else(|| VfsError::Enoent(format!("node {ino} not found")))?;
        node.xattrs
            .get(name)
            .cloned()
            .ok_or_else(|| VfsError::Enodata(format!("xattr '{name}' not found on '{path}'")))
    }

    fn listxattr(&self, caller: &CallerContext, ino: NodeId) -> VfsResult<Vec<String>> {
        let path = self.node_path(ino)?;
        self.access_hook
            .before_metadata(caller, MetadataOp::ListXattr, &path)?;
        let state = self.state.read().expect("volume lock poisoned");
        let node = state
            .nodes
            .get(&ino)
            .ok_or_else(|| VfsError::Enoent(format!("node {ino} not found")))?;
        Ok(node.xattrs.keys().cloned().collect())
    }

    fn setxattr(
        &self,
        caller: &CallerContext,
        ino: NodeId,
        name: &str,
        value: &[u8],
    ) -> VfsResult<()> {
        let path = self.node_path(ino)?;
        self.access_hook
            .before_metadata(caller, MetadataOp::SetXattr, &path)?;
        self.authorize_write(caller, &path)?;

        let now = self.now_ts();
        let mut state = self.state.write().expect("volume lock poisoned");
        let node = state
            .nodes
            .get_mut(&ino)
            .ok_or_else(|| VfsError::Enoent(format!("node {ino} not found")))?;
        node.xattrs.insert(name.to_string(), value.to_vec());
        node.attr.ctime = now;
        Ok(())
    }

    fn removexattr(&self, caller: &CallerContext, ino: NodeId, name: &str) -> VfsResult<()> {
        let path = self.node_path(ino)?;
        self.access_hook
            .before_metadata(caller, MetadataOp::RemoveXattr, &path)?;
        self.authorize_write(caller, &path)?;

        let now = self.now_ts();
        let mut state = self.state.write().expect("volume lock poisoned");
        let node = state
            .nodes
            .get_mut(&ino)
            .ok_or_else(|| VfsError::Enoent(format!("node {ino} not found")))?;
        if node.xattrs.remove(name).is_none() {
            return Err(VfsError::Enodata(format!(
                "xattr '{name}' not found on '{path}'"
            )));
        }
        node.attr.ctime = now;
        Ok(())
    }

    fn statfs(&self, caller: &CallerContext, ino: NodeId) -> VfsResult<StatFs> {
        let path = self.node_path(ino)?;
        self.access_hook
            .before_metadata(caller, MetadataOp::StatFs, &path)?;
        let state = self.state.read().expect("volume lock poisoned");
        let mut stats = StatFs::default();
        stats.files = state.nodes.len() as u64;
        stats.ffree = 1_048_576u64.saturating_sub(stats.files);
        Ok(stats)
    }

    fn copy_file_range(
        &self,
        caller: &CallerContext,
        ino_in: NodeId,
        offset_in: u64,
        ino_out: NodeId,
        offset_out: u64,
        len: u64,
    ) -> VfsResult<u64> {
        let src_path = self.node_path(ino_in)?;
        let dst_path = self.node_path(ino_out)?;
        self.access_hook
            .before_metadata(caller, MetadataOp::CopyFileRange, &src_path)?;
        self.access_hook
            .before_metadata(caller, MetadataOp::CopyFileRange, &dst_path)?;
        self.authorize_write(caller, &dst_path)?;

        let now = self.now_ts();
        let mut state = self.state.write().expect("volume lock poisoned");

        // Snapshot source metadata/reference without reading lazy chunk bytes.
        let (actual_len, overlay_source) = {
            let src = state
                .nodes
                .get(&ino_in)
                .ok_or_else(|| VfsError::Enoent(format!("source node {ino_in} not found")))?;
            let content = match &src.data {
                NodeData::Regular { content } => content,
                NodeData::Directory { .. } => {
                    return Err(VfsError::Eisdir(format!(
                        "copy_file_range source '{}' is a directory",
                        src.path
                    )));
                }
                NodeData::Symlink { .. } => {
                    return Err(VfsError::Einval(format!(
                        "copy_file_range source '{}' is a symlink",
                        src.path
                    )));
                }
            };
            if offset_in >= src.attr.size || len == 0 {
                return Ok(0);
            }
            let avail = (src.attr.size - offset_in).min(len);
            let source = match content {
                FileContent::Inline(bytes) => {
                    let start = offset_in as usize;
                    let end = (start + avail as usize).min(bytes.len());
                    let mut slice = vec![0u8; avail as usize];
                    if start < end {
                        slice[..end - start].copy_from_slice(&bytes[start..end]);
                    }
                    OverlaySource::Bytes(slice)
                }
                FileContent::Lazy { loader, .. } => OverlaySource::LazyRef {
                    loader: Arc::clone(loader),
                    src_offset: offset_in,
                },
            };
            (avail, source)
        };

        let dst = state
            .nodes
            .get_mut(&ino_out)
            .ok_or_else(|| VfsError::Enoent(format!("dest node {ino_out} not found")))?;
        match &mut dst.data {
            NodeData::Regular { content } => {
                match (content, overlay_source) {
                    (FileContent::Inline(bytes), OverlaySource::Bytes(slice)) => {
                        let end = (offset_out as usize).saturating_add(slice.len());
                        if bytes.len() < end {
                            bytes.resize(end, 0);
                        }
                        bytes[offset_out as usize..end].copy_from_slice(&slice);
                    }
                    (c @ FileContent::Inline(_), OverlaySource::LazyRef { loader, src_offset }) => {
                        // Promote destination to lazy-overlay without reading source chunks!
                        let existing = match std::mem::replace(
                            c,
                            FileContent::Lazy {
                                loader: Arc::new(|_, _| Ok(Vec::new())),
                                overlay: Vec::new(),
                            },
                        ) {
                            FileContent::Inline(b) => b,
                            _ => Vec::new(),
                        };
                        let mut overlay = Vec::new();
                        if !existing.is_empty() {
                            overlay.push(OverlaySpan {
                                dst_offset: 0,
                                len: existing.len() as u64,
                                source: OverlaySource::Bytes(existing),
                            });
                        }
                        overlay.push(OverlaySpan {
                            dst_offset: offset_out,
                            len: actual_len,
                            source: OverlaySource::LazyRef { loader, src_offset },
                        });
                        *c = FileContent::Lazy {
                            loader: Arc::new(|_, _| Ok(Vec::new())),
                            overlay,
                        };
                    }
                    (FileContent::Lazy { overlay, .. }, source) => {
                        overlay.push(OverlaySpan {
                            dst_offset: offset_out,
                            len: actual_len,
                            source,
                        });
                    }
                }
                let new_end = offset_out.saturating_add(actual_len);
                if new_end > dst.attr.size {
                    dst.attr.size = new_end;
                    dst.attr.blocks = FileAttr::blocks_for_size(new_end);
                }
                dst.attr.mtime = now;
                dst.attr.ctime = now;
                dst.attr.generation = dst.attr.generation.saturating_add(1);
                Ok(actual_len)
            }
            NodeData::Directory { .. } => Err(VfsError::Eisdir(format!(
                "copy_file_range dest '{}' is a directory",
                dst.path
            ))),
            NodeData::Symlink { .. } => Err(VfsError::Einval(format!(
                "copy_file_range dest '{}' is a symlink",
                dst.path
            ))),
        }
    }

    fn open(&self, caller: &CallerContext, ino: NodeId, flags: OpenFlags) -> VfsResult<FileHandle> {
        let path = self.node_path(ino)?;
        self.access_hook
            .before_metadata(caller, MetadataOp::Open, &path)?;
        if flags.requires_write() {
            self.authorize_write(caller, &path)?;
        }

        let now = self.now_ts();
        let mut state = self.state.write().expect("volume lock poisoned");
        {
            let node = state
                .nodes
                .get_mut(&ino)
                .ok_or_else(|| VfsError::Enoent(format!("node {ino} not found")))?;
            if matches!(node.data, NodeData::Directory { .. }) && flags.requires_write() {
                return Err(VfsError::Eisdir(format!(
                    "cannot open directory '{}' for writing",
                    node.path
                )));
            }
            if flags.truncate
                && let NodeData::Regular { content } = &mut node.data
            {
                content.truncate(0);
                node.attr.size = 0;
                node.attr.blocks = 0;
                node.attr.mtime = now;
                node.attr.ctime = now;
                node.attr.generation = node.attr.generation.saturating_add(1);
            }
        }

        let fh = FileHandle(state.next_fh);
        state.next_fh = state.next_fh.saturating_add(1);
        state.handles.insert(fh, OpenHandle { ino, flags });
        Ok(fh)
    }

    fn release(&self, caller: &CallerContext, ino: NodeId, fh: FileHandle) -> VfsResult<()> {
        let path = self.node_path(ino)?;
        self.access_hook
            .before_metadata(caller, MetadataOp::Release, &path)?;
        let mut state = self.state.write().expect("volume lock poisoned");
        let handle = state
            .handles
            .remove(&fh)
            .ok_or_else(|| VfsError::Ebadf(format!("unknown handle {fh}")))?;
        if handle.ino != ino {
            return Err(VfsError::Ebadf(format!(
                "handle {fh} does not belong to {ino}"
            )));
        }
        if let Some(lock_state) = state.locks.get_mut(&ino) {
            lock_state.shared.remove(&fh);
            if lock_state.exclusive == Some(fh) {
                lock_state.exclusive = None;
            }
        }
        Ok(())
    }

    fn fsync(
        &self,
        caller: &CallerContext,
        ino: NodeId,
        fh: FileHandle,
        _datasync: bool,
    ) -> VfsResult<()> {
        let path = self.node_path(ino)?;
        self.access_hook
            .before_metadata(caller, MetadataOp::Fsync, &path)?;
        let state = self.state.read().expect("volume lock poisoned");
        match state.handles.get(&fh) {
            Some(h) if h.ino == ino => Ok(()),
            _ => Err(VfsError::Ebadf(format!("invalid handle {fh} for {ino}"))),
        }
    }

    fn flock(
        &self,
        caller: &CallerContext,
        ino: NodeId,
        fh: FileHandle,
        op: FlockOp,
        _non_blocking: bool,
    ) -> VfsResult<()> {
        let path = self.node_path(ino)?;
        self.access_hook
            .before_metadata(caller, MetadataOp::Flock, &path)?;
        let mut state = self.state.write().expect("volume lock poisoned");
        match state.handles.get(&fh) {
            Some(h) if h.ino == ino => {}
            _ => return Err(VfsError::Ebadf(format!("invalid handle {fh} for {ino}"))),
        }
        let lock = state.locks.entry(ino).or_default();
        match op {
            FlockOp::Unlock => {
                lock.shared.remove(&fh);
                if lock.exclusive == Some(fh) {
                    lock.exclusive = None;
                }
                Ok(())
            }
            FlockOp::Shared => {
                if let Some(holder) = lock.exclusive
                    && holder != fh
                {
                    return Err(VfsError::Ewouldblock(format!(
                        "exclusive lock held on '{path}'"
                    )));
                }
                lock.exclusive = None;
                lock.shared.insert(fh);
                Ok(())
            }
            FlockOp::Exclusive => {
                let other_shared = lock.shared.iter().any(|h| *h != fh);
                let other_excl = lock.exclusive.is_some_and(|h| h != fh);
                if other_shared || other_excl {
                    return Err(VfsError::Ewouldblock(format!("lock conflict on '{path}'")));
                }
                lock.shared.remove(&fh);
                lock.exclusive = Some(fh);
                Ok(())
            }
        }
    }

    // -------------------------------------------------------------------------
    // Chunk-data operations
    // -------------------------------------------------------------------------

    fn read(
        &self,
        caller: &CallerContext,
        ino: NodeId,
        fh: FileHandle,
        offset: u64,
        size: u32,
    ) -> VfsResult<Vec<u8>> {
        let (path, content, file_size) = {
            let state = self.state.read().expect("volume lock poisoned");
            let handle = state
                .handles
                .get(&fh)
                .ok_or_else(|| VfsError::Ebadf(format!("unknown handle {fh}")))?;
            if handle.ino != ino {
                return Err(VfsError::Ebadf(format!(
                    "handle {fh} does not belong to {ino}"
                )));
            }
            if !handle.flags.read {
                return Err(VfsError::Eacces(format!(
                    "handle {fh} was not opened for reading"
                )));
            }
            let node = state
                .nodes
                .get(&ino)
                .ok_or_else(|| VfsError::Enoent(format!("node {ino} not found")))?;
            let content = match &node.data {
                NodeData::Regular { content } => content.clone(),
                NodeData::Directory { .. } => {
                    return Err(VfsError::Eisdir(format!(
                        "cannot read directory '{}'",
                        node.path
                    )));
                }
                NodeData::Symlink { .. } => {
                    return Err(VfsError::Einval(format!(
                        "cannot read symlink '{}'",
                        node.path
                    )));
                }
            };
            (node.path.clone(), content, node.attr.size)
        };

        self.access_hook.before_read(caller, &path, offset, size)?;
        content.read_range(file_size, offset, size)
    }

    fn write(
        &self,
        caller: &CallerContext,
        ino: NodeId,
        fh: FileHandle,
        offset: u64,
        data: &[u8],
        expected_generation: Option<u64>,
    ) -> VfsResult<u32> {
        let path = self.node_path(ino)?;
        self.access_hook
            .before_write(caller, &path, offset, data.len())?;
        self.authorize_write(caller, &path)?;

        let now = self.now_ts();
        let mut state = self.state.write().expect("volume lock poisoned");
        let handle = *state
            .handles
            .get(&fh)
            .ok_or_else(|| VfsError::Ebadf(format!("unknown handle {fh}")))?;
        if handle.ino != ino {
            return Err(VfsError::Ebadf(format!(
                "handle {fh} does not belong to {ino}"
            )));
        }
        if !handle.flags.write && !handle.flags.append {
            return Err(VfsError::Eacces(format!(
                "handle {fh} was not opened for writing"
            )));
        }

        let node = state
            .nodes
            .get_mut(&ino)
            .ok_or_else(|| VfsError::Enoent(format!("node {ino} not found")))?;
        check_generation(expected_generation, node.attr.generation)?;

        let write_offset = if handle.flags.append {
            node.attr.size
        } else {
            offset
        };

        match &mut node.data {
            NodeData::Regular { content } => {
                content.write_at(write_offset, data);
                let new_end = write_offset.saturating_add(data.len() as u64);
                if new_end > node.attr.size {
                    node.attr.size = new_end;
                    node.attr.blocks = FileAttr::blocks_for_size(new_end);
                }
                node.attr.mtime = now;
                node.attr.ctime = now;
                node.attr.generation = node.attr.generation.saturating_add(1);
                Ok(data.len() as u32)
            }
            NodeData::Directory { .. } => Err(VfsError::Eisdir(format!(
                "cannot write directory '{}'",
                node.path
            ))),
            NodeData::Symlink { .. } => Err(VfsError::Einval(format!(
                "cannot write symlink '{}'",
                node.path
            ))),
        }
    }
}

/// Declarative builder for constructing an [`InMemoryVolume`].
pub struct TreeBuilder {
    state: VolumeState,
    leases: Option<Arc<LeaseTable>>,
    access_hook: Arc<dyn AccessHook>,
    clock: Arc<dyn Fn() -> u64 + Send + Sync>,
    pending_err: Option<VfsError>,
}

impl Default for TreeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl TreeBuilder {
    pub fn new() -> Self {
        let mut nodes = BTreeMap::new();
        nodes.insert(
            ROOT_NODE_ID,
            Node {
                attr: FileAttr {
                    ino: ROOT_NODE_ID,
                    kind: FileKind::Directory,
                    size: 0,
                    blocks: 0,
                    atime: Timestamp::ZERO,
                    mtime: Timestamp::ZERO,
                    ctime: Timestamp::ZERO,
                    mode: 0o755,
                    nlink: 2,
                    uid: 0,
                    gid: 0,
                    generation: 1,
                },
                path: VirtualPath::root(),
                xattrs: BTreeMap::new(),
                data: NodeData::Directory {
                    parent: ROOT_NODE_ID,
                    children: BTreeMap::new(),
                },
            },
        );
        Self {
            state: VolumeState {
                next_ino: ROOT_NODE_ID.0 + 1,
                next_fh: 1,
                nodes,
                handles: BTreeMap::new(),
                locks: BTreeMap::new(),
                protected_roots: BTreeSet::new(),
            },
            leases: None,
            access_hook: Arc::new(NoopAccessHook),
            clock: Arc::new(|| 0),
            pending_err: None,
        }
    }

    /// Attaches a [`LeaseTable`] for write authorization enforcement.
    pub fn with_lease_table(mut self, leases: Arc<LeaseTable>) -> Self {
        self.leases = Some(leases);
        self
    }

    /// Attaches an [`AccessHook`] invoked before metadata, read, and write operations.
    pub fn with_access_hook(mut self, hook: Arc<dyn AccessHook>) -> Self {
        self.access_hook = hook;
        self
    }

    /// Attaches a clock closure returning current wall time in milliseconds.
    pub fn with_clock(mut self, clock: Arc<dyn Fn() -> u64 + Send + Sync>) -> Self {
        self.clock = clock;
        self
    }

    /// Marks `path` as a protected root (`EPERM` on `unlink`, `rmdir`, or `rename`).
    pub fn protect_root(mut self, path: &str) -> Self {
        match VirtualPath::parse(path) {
            Ok(p) => {
                self.state.protected_roots.insert(p);
            }
            Err(e) => self.record_err(e),
        }
        self
    }

    /// Ensures a directory (and any missing parent directories) exists at `path`.
    pub fn dir(mut self, path: &str) -> Self {
        match VirtualPath::parse(path) {
            Ok(p) => {
                if let Err(e) = self.ensure_dir(&p) {
                    self.record_err(e);
                }
            }
            Err(e) => self.record_err(e),
        }
        self
    }

    /// Creates a regular file with inline bytes at `path` (creating parent dirs as needed).
    pub fn file(mut self, path: &str, bytes: impl Into<Vec<u8>>) -> Self {
        let data = bytes.into();
        let size = data.len() as u64;
        self.insert_regular(path, size, FileContent::Inline(data));
        self
    }

    /// Creates a lazy chunk-backed regular file at `path`.
    ///
    /// Metadata operations (`stat`, `readdir`, `rename`, `truncate`, `copy_file_range`)
    /// will never invoke `loader`; only `read` invokes `loader`.
    pub fn lazy_file(mut self, path: &str, size: u64, loader: ChunkLoader) -> Self {
        self.insert_regular(
            path,
            size,
            FileContent::Lazy {
                loader,
                overlay: Vec::new(),
            },
        );
        self
    }

    /// Creates a symbolic link at `path` pointing to `target`.
    pub fn symlink(mut self, path: &str, target: impl Into<String>) -> Self {
        let target = target.into();
        if self.pending_err.is_some() {
            return self;
        }
        let parsed = match VirtualPath::parse(path) {
            Ok(p) => p,
            Err(e) => {
                self.record_err(e);
                return self;
            }
        };
        let Some(parent_path) = parsed.parent() else {
            self.record_err(VfsError::Einval("cannot create symlink at root".into()));
            return self;
        };
        let Some(name) = parsed.file_name().map(str::to_string) else {
            self.record_err(VfsError::Einval("missing symlink name".into()));
            return self;
        };
        let parent_ino = match self.ensure_dir(&parent_path) {
            Ok(ino) => ino,
            Err(e) => {
                self.record_err(e);
                return self;
            }
        };
        let ino = NodeId(self.state.next_ino);
        self.state.next_ino = self.state.next_ino.saturating_add(1);
        let size = target.len() as u64;
        self.state.nodes.insert(
            ino,
            Node {
                attr: FileAttr {
                    ino,
                    kind: FileKind::Symlink,
                    size,
                    blocks: FileAttr::blocks_for_size(size),
                    atime: Timestamp::ZERO,
                    mtime: Timestamp::ZERO,
                    ctime: Timestamp::ZERO,
                    mode: 0o777,
                    nlink: 1,
                    uid: 0,
                    gid: 0,
                    generation: 1,
                },
                path: parsed,
                xattrs: BTreeMap::new(),
                data: NodeData::Symlink { target },
            },
        );
        if let Some(p) = self.state.nodes.get_mut(&parent_ino)
            && let NodeData::Directory { children, .. } = &mut p.data
        {
            children.insert(name, ino);
        }
        self
    }

    /// Inserts a synthetic `.omitted.json` stub file inside `dir_path`.
    pub fn omitted_stub(mut self, dir_path: &str, stub: SyntheticStub) -> Self {
        if self.pending_err.is_some() {
            return self;
        }
        let stub_path = match VirtualPath::parse(dir_path).and_then(|p| p.join(OMITTED_STUB_FILENAME))
        {
            Ok(p) => p,
            Err(e) => {
                self.record_err(e);
                return self;
            }
        };
        match stub.to_json_bytes() {
            Ok(bytes) => self.file(stub_path.as_str(), bytes),
            Err(e) => {
                self.record_err(e);
                self
            }
        }
    }

    /// Finalizes the [`InMemoryVolume`].
    pub fn build(self) -> VfsResult<InMemoryVolume> {
        if let Some(err) = self.pending_err {
            return Err(err);
        }
        Ok(InMemoryVolume {
            state: RwLock::new(self.state),
            leases: self.leases,
            access_hook: self.access_hook,
            clock: self.clock,
        })
    }

    fn record_err(&mut self, err: VfsError) {
        if self.pending_err.is_none() {
            self.pending_err = Some(err);
        }
    }

    fn insert_regular(&mut self, path: &str, size: u64, content: FileContent) {
        if self.pending_err.is_some() {
            return;
        }
        let parsed = match VirtualPath::parse(path) {
            Ok(p) => p,
            Err(e) => {
                self.record_err(e);
                return;
            }
        };
        let Some(parent_path) = parsed.parent() else {
            self.record_err(VfsError::Einval("cannot create file at root".into()));
            return;
        };
        let Some(name) = parsed.file_name().map(str::to_string) else {
            self.record_err(VfsError::Einval("missing file name".into()));
            return;
        };
        let parent_ino = match self.ensure_dir(&parent_path) {
            Ok(ino) => ino,
            Err(e) => {
                self.record_err(e);
                return;
            }
        };
        let ino = NodeId(self.state.next_ino);
        self.state.next_ino = self.state.next_ino.saturating_add(1);
        self.state.nodes.insert(
            ino,
            Node {
                attr: FileAttr {
                    ino,
                    kind: FileKind::Regular,
                    size,
                    blocks: FileAttr::blocks_for_size(size),
                    atime: Timestamp::ZERO,
                    mtime: Timestamp::ZERO,
                    ctime: Timestamp::ZERO,
                    mode: 0o644,
                    nlink: 1,
                    uid: 0,
                    gid: 0,
                    generation: 1,
                },
                path: parsed,
                xattrs: BTreeMap::new(),
                data: NodeData::Regular { content },
            },
        );
        if let Some(p) = self.state.nodes.get_mut(&parent_ino)
            && let NodeData::Directory { children, .. } = &mut p.data
        {
            children.insert(name, ino);
        }
    }

    fn ensure_dir(&mut self, dir: &VirtualPath) -> VfsResult<NodeId> {
        let mut current = ROOT_NODE_ID;
        let mut cur_path = VirtualPath::root();
        for seg in dir.segments() {
            cur_path = cur_path.join(seg)?;
            let existing = match &self.state.nodes.get(&current).unwrap().data {
                NodeData::Directory { children, .. } => children.get(seg).copied(),
                _ => {
                    return Err(VfsError::Enotdir(format!(
                        "'{cur_path}' parent is not a directory"
                    )));
                }
            };
            if let Some(ino) = existing {
                if !matches!(
                    self.state.nodes.get(&ino).unwrap().data,
                    NodeData::Directory { .. }
                ) {
                    return Err(VfsError::Enotdir(format!(
                        "'{cur_path}' already exists as non-directory"
                    )));
                }
                current = ino;
            } else {
                let ino = NodeId(self.state.next_ino);
                self.state.next_ino = self.state.next_ino.saturating_add(1);
                self.state.nodes.insert(
                    ino,
                    Node {
                        attr: FileAttr {
                            ino,
                            kind: FileKind::Directory,
                            size: 0,
                            blocks: 0,
                            atime: Timestamp::ZERO,
                            mtime: Timestamp::ZERO,
                            ctime: Timestamp::ZERO,
                            mode: 0o755,
                            nlink: 2,
                            uid: 0,
                            gid: 0,
                            generation: 1,
                        },
                        path: cur_path.clone(),
                        xattrs: BTreeMap::new(),
                        data: NodeData::Directory {
                            parent: current,
                            children: BTreeMap::new(),
                        },
                    },
                );
                if let Some(p) = self.state.nodes.get_mut(&current) {
                    if let NodeData::Directory { children, .. } = &mut p.data {
                        children.insert(seg.to_string(), ino);
                    }
                    p.attr.nlink = p.attr.nlink.saturating_add(1);
                }
                current = ino;
            }
        }
        Ok(current)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

    use super::*;

    #[test]
    fn synthetic_omitted_stub_generation_and_read() {
        let stub = SyntheticStub::new("/repo/target", "build_cache")
            .with_original_bytes(512_000_000)
            .with_original_entries(3_200)
            .with_hint("Omitted from remote mount");

        let vol = InMemoryVolume::builder()
            .omitted_stub("/repo/target", stub.clone())
            .build()
            .unwrap();

        let caller = CallerContext::new(100, 1000, 1000);
        let attr = vol
            .resolve_path(&caller, "/repo/target/.omitted.json")
            .unwrap();
        assert_eq!(attr.kind, FileKind::Regular);
        assert!(attr.size > 0);

        let fh = vol.open(&caller, attr.ino, OpenFlags::read_only()).unwrap();
        let bytes = vol.read(&caller, attr.ino, fh, 0, 4096).unwrap();
        vol.release(&caller, attr.ino, fh).unwrap();

        let decoded = SyntheticStub::from_json_bytes(&bytes).unwrap();
        assert_eq!(decoded, stub);
    }

    #[test]
    fn protected_roots_return_eperm_on_unlink_rename_rmdir() {
        let vol = InMemoryVolume::builder()
            .dir("/mount/protected_dir")
            .file("/mount/protected_file", b"keep".to_vec())
            .dir("/mount/other")
            .protect_root("/mount/protected_dir")
            .protect_root("/mount/protected_file")
            .build()
            .unwrap();

        let caller = CallerContext::new(1, 0, 0);
        let mount_ino = vol.resolve_path(&caller, "/mount").unwrap().ino;
        let other_ino = vol.resolve_path(&caller, "/mount/other").unwrap().ino;

        assert!(matches!(
            vol.rmdir(&caller, mount_ino, "protected_dir"),
            Err(VfsError::Eperm(_))
        ));
        assert!(matches!(
            vol.unlink(&caller, mount_ino, "protected_file"),
            Err(VfsError::Eperm(_))
        ));
        assert!(matches!(
            vol.rename(&caller, mount_ino, "protected_dir", other_ino, "moved"),
            Err(VfsError::Eperm(_))
        ));
        assert!(matches!(
            vol.rename(&caller, mount_ino, "protected_file", other_ino, "moved"),
            Err(VfsError::Eperm(_))
        ));
    }

    #[test]
    fn write_lease_enforcement_and_estale_generation_mismatch() {
        let leases = Arc::new(LeaseTable::new());
        let now_ms = Arc::new(AtomicU64::new(1_000));
        let clock_ref = Arc::clone(&now_ms);

        let vol = InMemoryVolume::builder()
            .file("/work/allowed/a.txt", b"hello".to_vec())
            .file("/work/denied/b.txt", b"secret".to_vec())
            .with_lease_table(Arc::clone(&leases))
            .with_clock(Arc::new(move || clock_ref.load(Ordering::SeqCst)))
            .build()
            .unwrap();

        let caller = CallerContext::new(55, 1000, 1000);
        leases.grant(
            55,
            VirtualPath::parse("/work/allowed").unwrap(),
            5_000,
            Some("turn-42".into()),
        );

        let allowed_attr = vol.resolve_path(&caller, "/work/allowed/a.txt").unwrap();
        let denied_attr = vol.resolve_path(&caller, "/work/denied/b.txt").unwrap();

        // Opening denied path for write fails with EACCES.
        assert!(matches!(
            vol.open(&caller, denied_attr.ino, OpenFlags::read_write()),
            Err(VfsError::Eacces(_))
        ));

        // Allowed path succeeds with matching generation, fails with ESTALE on stale generation.
        let fh = vol
            .open(&caller, allowed_attr.ino, OpenFlags::read_write())
            .unwrap();
        let stale_err = vol
            .write(
                &caller,
                allowed_attr.ino,
                fh,
                0,
                b"world",
                Some(allowed_attr.generation + 99),
            )
            .unwrap_err();
        assert!(matches!(stale_err, VfsError::Estale(_)));

        let written = vol
            .write(
                &caller,
                allowed_attr.ino,
                fh,
                0,
                b"world",
                Some(allowed_attr.generation),
            )
            .unwrap();
        assert_eq!(written, 5);

        // Turn revocation blocks subsequent writes with EACCES.
        assert_eq!(leases.revoke_turn("turn-42"), 1);
        let revoked_err = vol
            .write(&caller, allowed_attr.ino, fh, 0, b"again", None)
            .unwrap_err();
        assert!(matches!(revoked_err, VfsError::Eacces(_)));

        // Re-grant and verify expiry.
        leases.grant(55, VirtualPath::parse("/work/allowed").unwrap(), 6_000, None);
        now_ms.store(6_000, Ordering::SeqCst);
        let expired_err = vol
            .write(&caller, allowed_attr.ino, fh, 0, b"late", None)
            .unwrap_err();
        assert!(matches!(expired_err, VfsError::Eacces(_)));

        vol.release(&caller, allowed_attr.ino, fh).unwrap();
    }

    #[test]
    fn zero_data_metadata_ops_never_trigger_chunk_reads() {
        let chunk_reads = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&chunk_reads);
        let payload = Arc::new(b"0123456789abcdef".to_vec());
        let payload_ref = Arc::clone(&payload);

        let loader: ChunkLoader = Arc::new(move |offset, size| {
            counter.fetch_add(1, Ordering::SeqCst);
            let start = offset as usize;
            let end = (start + size as usize).min(payload_ref.len());
            if start >= payload_ref.len() {
                Ok(Vec::new())
            } else {
                Ok(payload_ref[start..end].to_vec())
            }
        });

        let vol = InMemoryVolume::builder()
            .lazy_file("/data/src.bin", payload.len() as u64, loader)
            .file("/data/dst.bin", Vec::new())
            .build()
            .unwrap();

        let caller = CallerContext::new(10, 1000, 1000);
        let data_dir = vol.resolve_path(&caller, "/data").unwrap().ino;

        // 1. stat (getattr)
        let src_attr = vol.resolve_path(&caller, "/data/src.bin").unwrap();
        assert_eq!(src_attr.size, 16);

        // 2. readdir
        let entries = vol.readdir(&caller, data_dir, 0).unwrap();
        assert!(entries.iter().any(|e| e.name == "src.bin"));

        // 3. rename
        vol.rename(&caller, data_dir, "src.bin", data_dir, "moved.bin")
            .unwrap();
        let moved_attr = vol.lookup(&caller, data_dir, "moved.bin").unwrap();
        assert_eq!(moved_attr.ino, src_attr.ino);

        // 4. copy_file_range (zero-data server-side extent copy)
        let dst_attr = vol.lookup(&caller, data_dir, "dst.bin").unwrap();
        let copied = vol
            .copy_file_range(&caller, moved_attr.ino, 4, dst_attr.ino, 0, 8)
            .unwrap();
        assert_eq!(copied, 8);

        // 5. truncate (setattr)
        let truncated = vol
            .setattr(
                &caller,
                moved_attr.ino,
                SetAttr {
                    size: Some(10),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(truncated.size, 10);

        // All 5 zero-data metadata operations must have performed ZERO chunk reads!
        assert_eq!(chunk_reads.load(Ordering::SeqCst), 0);

        // Now perform an actual data read on dst.bin and verify the copied bytes + loader call.
        let fh = vol
            .open(&caller, dst_attr.ino, OpenFlags::read_only())
            .unwrap();
        let read_bytes = vol.read(&caller, dst_attr.ino, fh, 0, 8).unwrap();
        vol.release(&caller, dst_attr.ino, fh).unwrap();
        assert_eq!(read_bytes, b"456789ab");
        assert_eq!(chunk_reads.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn capability_root_clamps_dotdot_lookup_and_resolve_path() {
        let vol = InMemoryVolume::builder()
            .file("/etc/passwd", b"root:x:0:0".to_vec())
            .file("/sandbox/etc/passwd", b"sandbox-decoy".to_vec())
            .file("/sandbox/app/main.rs", b"fn main() {}".to_vec())
            .build()
            .unwrap();

        let caller = CallerContext::new(20, 1000, 1000).with_capability_root("/sandbox");

        // Adversarial path traversal resolves inside /sandbox, never /etc/passwd.
        let attr = vol
            .resolve_path(&caller, "/sandbox/app/../../../../etc/passwd")
            .unwrap();
        let fh = vol.open(&caller, attr.ino, OpenFlags::read_only()).unwrap();
        let bytes = vol.read(&caller, attr.ino, fh, 0, 64).unwrap();
        vol.release(&caller, attr.ino, fh).unwrap();
        assert_eq!(bytes, b"sandbox-decoy");

        // Direct `..` lookup from capability root stays at capability root.
        let sandbox_ino = vol.resolve_path(&caller, "/sandbox").unwrap().ino;
        let parent_attr = vol.lookup(&caller, sandbox_ino, "..").unwrap();
        assert_eq!(parent_attr.ino, sandbox_ino);
    }

    #[test]
    fn symlinks_xattrs_and_flock_work_cleanly() {
        let vol = InMemoryVolume::builder()
            .dir("/work")
            .file("/work/file.txt", b"abc".to_vec())
            .symlink("/work/link.txt", "file.txt")
            .build()
            .unwrap();

        let caller = CallerContext::new(30, 1000, 1000);
        let link_attr = vol.resolve_path(&caller, "/work/link.txt").unwrap();
        assert_eq!(link_attr.kind, FileKind::Symlink);
        assert_eq!(vol.readlink(&caller, link_attr.ino).unwrap(), "file.txt");

        let file_ino = vol.resolve_path(&caller, "/work/file.txt").unwrap().ino;
        vol.setxattr(&caller, file_ino, "user.tag", b"v1").unwrap();
        assert_eq!(vol.getxattr(&caller, file_ino, "user.tag").unwrap(), b"v1");
        assert_eq!(vol.listxattr(&caller, file_ino).unwrap(), vec!["user.tag"]);
        vol.removexattr(&caller, file_ino, "user.tag").unwrap();
        assert!(matches!(
            vol.getxattr(&caller, file_ino, "user.tag"),
            Err(VfsError::Enodata(_))
        ));

        let fh1 = vol.open(&caller, file_ino, OpenFlags::read_only()).unwrap();
        let fh2 = vol.open(&caller, file_ino, OpenFlags::read_only()).unwrap();
        vol.flock(&caller, file_ino, fh1, FlockOp::Shared, true)
            .unwrap();
        vol.flock(&caller, file_ino, fh2, FlockOp::Shared, true)
            .unwrap();
        assert!(matches!(
            vol.flock(&caller, file_ino, fh1, FlockOp::Exclusive, true),
            Err(VfsError::Ewouldblock(_))
        ));
        vol.release(&caller, file_ino, fh2).unwrap();
        vol.flock(&caller, file_ino, fh1, FlockOp::Exclusive, true)
            .unwrap();
        vol.release(&caller, file_ino, fh1).unwrap();
    }
}
