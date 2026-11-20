//! T6: Humata virtual tree projector, stable-by-id `.sessions/<id>/` placement,
//! `.humata-omitted.json` `BuildCache` stubs, and daemon reconnect survival.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use fusefs_vfs::{
    AccessHook, CallerContext, ChunkLoader, DirEntry, FileAttr, FileHandle, FlockOp,
    InMemoryVolume, LeaseTable, NodeId, OpenFlags, SetAttr, StatFs, SyntheticStub, TreeBuilder,
    VfsError, VfsResult, VirtualPath, Volume,
};
use serde::{Deserialize, Serialize};

/// Filename used for Humata build-cache omission stubs inside projected repos.
pub const HUMATA_OMITTED_STUB_FILENAME: &str = ".humata-omitted.json";

/// Default build-cache / heavy-object directories omitted from remote mounts.
pub const DEFAULT_BUILD_CACHE_DIRS: &[&str] = &["target", "node_modules", ".git/objects"];

/// Computes the stable-by-id native session directory:
/// `<machine_root>/.sessions/<session_id>`.
///
/// Because placement is keyed strictly by `session_id` rather than channel or
/// desk name, same-machine channel renames and `humatad2` restarts never move
/// the underlying native directory or disturb live holder PIDs.
pub fn stable_session_native_path(machine_root: &Path, session_id: &str) -> PathBuf {
    machine_root.join(".sessions").join(session_id)
}

/// Returns `true` if `rel_path` matches one of the default omitted build-cache
/// prefixes (`target`, `node_modules`, `.git/objects`).
pub fn is_default_build_cache_dir(rel_path: &str) -> bool {
    let trimmed = rel_path.trim_matches('/');
    DEFAULT_BUILD_CACHE_DIRS
        .iter()
        .any(|&dir| trimmed == dir || trimmed.starts_with(&format!("{dir}/")))
}

/// Descriptor serialized into `.humata-omitted.json` for omitted build caches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildCacheStub {
    pub schema_version: u32,
    pub rel_path: String,
    pub reason: String,
    pub original_bytes: u64,
    pub original_entries: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl BuildCacheStub {
    pub fn build_cache(
        rel_path: impl Into<String>,
        original_bytes: u64,
        original_entries: u64,
    ) -> Self {
        let rel_path = rel_path.into();
        Self {
            schema_version: 1,
            hint: Some(format!(
                "Run `humata2_fusefs materialize {rel_path}` to hydrate on demand"
            )),
            rel_path,
            reason: "build_cache".to_string(),
            original_bytes,
            original_entries,
        }
    }

    pub fn to_synthetic_stub(&self, omitted_virtual_path: &str) -> SyntheticStub {
        let mut stub = SyntheticStub::new(omitted_virtual_path, &self.reason)
            .with_original_bytes(self.original_bytes)
            .with_original_entries(self.original_entries);
        if let Some(hint) = &self.hint {
            stub = stub.with_hint(hint.clone());
        }
        stub
    }

    pub fn to_json_bytes(&self) -> VfsResult<Vec<u8>> {
        let mut out = serde_json::to_vec_pretty(self)
            .map_err(|e| VfsError::Eio(format!("failed to serialize BuildCacheStub: {e}")))?;
        out.push(b'\n');
        Ok(out)
    }

    pub fn from_json_bytes(bytes: &[u8]) -> VfsResult<Self> {
        serde_json::from_slice(bytes)
            .map_err(|e| VfsError::Einval(format!("invalid BuildCacheStub JSON: {e}")))
    }
}

/// File entry projected inside a session's repository tree.
#[derive(Clone)]
pub enum SessionFileSpec {
    /// Small inline file (metadata, config, context, or local text).
    Inline { rel_path: String, bytes: Vec<u8> },
    /// Lazy chunk-backed file whose payload loader is NEVER called by metadata ops.
    Lazy {
        rel_path: String,
        size: u64,
        loader: ChunkLoader,
    },
}

/// Metadata written to `/s/<id>/meta.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionMetaJson {
    pub session_id: String,
    pub machine_id: String,
    pub holder_pid: u32,
    pub native_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desk: Option<String>,
}

/// Session input for the Humata virtual tree projector.
#[derive(Clone)]
pub struct SessionProjection {
    pub session_id: String,
    pub machine_id: String,
    pub channel: Option<String>,
    pub desk: Option<String>,
    pub holder_pid: u32,
    pub machine_root: PathBuf,
    pub config_agent: Option<String>,
    pub files: Vec<SessionFileSpec>,
    pub build_cache_stubs: Vec<BuildCacheStub>,
    pub context_files: Vec<(String, Vec<u8>)>,
}

impl SessionProjection {
    pub fn new(
        session_id: impl Into<String>,
        machine_id: impl Into<String>,
        machine_root: impl Into<PathBuf>,
        holder_pid: u32,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            machine_id: machine_id.into(),
            channel: None,
            desk: None,
            holder_pid,
            machine_root: machine_root.into(),
            config_agent: Some("claude".to_string()),
            files: Vec::new(),
            build_cache_stubs: Vec::new(),
            context_files: Vec::new(),
        }
    }

    pub fn with_channel(mut self, channel: impl Into<String>) -> Self {
        self.channel = Some(channel.into());
        self
    }

    pub fn with_desk(mut self, desk: impl Into<String>) -> Self {
        self.desk = Some(desk.into());
        self
    }

    pub fn with_config_agent(mut self, agent: impl Into<String>) -> Self {
        self.config_agent = Some(agent.into());
        self
    }

    pub fn with_inline_file(
        mut self,
        rel_path: impl Into<String>,
        bytes: impl Into<Vec<u8>>,
    ) -> Self {
        self.files.push(SessionFileSpec::Inline {
            rel_path: rel_path.into(),
            bytes: bytes.into(),
        });
        self
    }

    pub fn with_lazy_file(
        mut self,
        rel_path: impl Into<String>,
        size: u64,
        loader: ChunkLoader,
    ) -> Self {
        self.files.push(SessionFileSpec::Lazy {
            rel_path: rel_path.into(),
            size,
            loader,
        });
        self
    }

    pub fn with_build_cache_stub(mut self, stub: BuildCacheStub) -> Self {
        self.build_cache_stubs.push(stub);
        self
    }

    pub fn with_context_file(
        mut self,
        rel_path: impl Into<String>,
        bytes: impl Into<Vec<u8>>,
    ) -> Self {
        self.context_files.push((rel_path.into(), bytes.into()));
        self
    }

    /// Stable native path `<machine_root>/.sessions/<session_id>`.
    pub fn native_path(&self) -> PathBuf {
        stable_session_native_path(&self.machine_root, &self.session_id)
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

/// Complete snapshot of Humata topology projected into FUSEFS.
#[derive(Clone, Default)]
pub struct HumataTopology {
    pub local_machine_id: String,
    pub local_machine_root: PathBuf,
    pub machines: Vec<MachineProjection>,
    pub sessions: Vec<SessionProjection>,
    pub desks: Vec<DeskProjection>,
    pub local_context_files: Vec<(String, Vec<u8>)>,
    pub control_status_json: Option<Vec<u8>>,
}

/// Projects a [`HumataTopology`] into a [`fusefs_vfs::InMemoryVolume`] with all
/// canonical roots, aliases, local-authority `context/`, `.humata-omitted.json`
/// `BuildCache` stubs, and `MASSDELETE` root protections.
#[derive(Debug, Default, Clone, Copy)]
pub struct HumataTreeProjector;

impl HumataTreeProjector {
    pub fn project(
        topology: &HumataTopology,
        leases: Option<Arc<LeaseTable>>,
        access_hook: Option<Arc<dyn AccessHook>>,
        clock: Option<Arc<dyn Fn() -> u64 + Send + Sync>>,
    ) -> VfsResult<InMemoryVolume> {
        let mut builder = TreeBuilder::new();
        if let Some(leases) = leases {
            builder = builder.with_lease_table(leases);
        }
        if let Some(hook) = access_hook {
            builder = builder.with_access_hook(hook);
        }
        if let Some(clock) = clock {
            builder = builder.with_clock(clock);
        }

        // Top-level Humata virtual namespace directories (all protected roots).
        for top in [
            "/s",
            "/m",
            "/channels",
            "/desks",
            "/machines",
            "/requests",
            "/control",
            "/context",
        ] {
            builder = builder.dir(top).protect_root(top);
        }

        // Local-authority context/ files.
        for (rel, bytes) in &topology.local_context_files {
            let clean = rel.trim_start_matches('/');
            let full = format!("/context/{clean}");
            builder = builder.file(&full, bytes.clone());
        }

        // Control directory status snapshot.
        let control_bytes = topology.control_status_json.clone().unwrap_or_else(|| {
            serde_json::to_vec_pretty(&serde_json::json!({
                "local_machine_id": topology.local_machine_id,
                "sessions": topology.sessions.len(),
                "machines": topology.machines.len(),
            }))
            .unwrap_or_default()
        });
        builder = builder.file("/control/status.json", control_bytes);

        // Machines under /m/<machine_id> and /machines/<machine_id>.
        for machine in &topology.machines {
            let m_root = format!("/m/{}", machine.machine_id);
            let m_sessions = format!("{m_root}/s");
            let info_dir = format!("/machines/{}", machine.machine_id);
            let info_file = format!("{info_dir}/status.json");
            let info_bytes = serde_json::to_vec_pretty(machine)
                .map_err(|e| VfsError::Eio(format!("machine status json: {e}")))?;

            builder = builder
                .dir(&m_root)
                .protect_root(&m_root)
                .dir(&m_sessions)
                .protect_root(&m_sessions)
                .dir(&info_dir)
                .protect_root(&info_dir)
                .file(&info_file, info_bytes);
        }

        // Desks under /desks/<desk_id> (deduplicated with per-session desk fields).
        let mut desk_links = std::collections::BTreeSet::new();
        for desk in &topology.desks {
            let desk_root = format!("/desks/{}", desk.desk_id);
            builder = builder.dir(&desk_root).protect_root(&desk_root);
            for sid in &desk.session_ids {
                if desk_links.insert((desk.desk_id.clone(), sid.clone())) {
                    let alias = format!("{desk_root}/{sid}");
                    let target = format!("/s/{sid}");
                    builder = builder.symlink(&alias, target).protect_root(&alias);
                }
            }
        }

        // Sessions under /s/<session_id>, plus machine/channel/desk aliases.
        for sess in &topology.sessions {
            let s_root = format!("/s/{}", sess.session_id);
            let repo_root = format!("{s_root}/repo");
            let config_root = format!("{s_root}/config");
            let context_root = format!("{s_root}/context");

            builder = builder
                .dir(&s_root)
                .protect_root(&s_root)
                .dir(&repo_root)
                .protect_root(&repo_root)
                .dir(&config_root)
                .protect_root(&config_root)
                .dir(&context_root)
                .protect_root(&context_root);

            if let Some(agent) = &sess.config_agent {
                let agent_cfg = format!("{config_root}/{agent}");
                builder = builder.dir(&agent_cfg).protect_root(&agent_cfg);
            }

            let meta = SessionMetaJson {
                session_id: sess.session_id.clone(),
                machine_id: sess.machine_id.clone(),
                holder_pid: sess.holder_pid,
                native_path: sess.native_path().display().to_string(),
                channel: sess.channel.clone(),
                desk: sess.desk.clone(),
            };
            let meta_bytes = serde_json::to_vec_pretty(&meta)
                .map_err(|e| VfsError::Eio(format!("session meta json: {e}")))?;
            builder = builder.file(&format!("{s_root}/meta.json"), meta_bytes);

            // Local-authority session context files.
            for (rel, bytes) in &sess.context_files {
                let clean = rel.trim_start_matches('/');
                builder = builder.file(&format!("{context_root}/{clean}"), bytes.clone());
            }

            // Projected repository files (inline or lazy chunk-backed).
            for f in &sess.files {
                match f {
                    SessionFileSpec::Inline { rel_path, bytes } => {
                        let clean = rel_path.trim_start_matches('/');
                        builder = builder.file(&format!("{repo_root}/{clean}"), bytes.clone());
                    }
                    SessionFileSpec::Lazy {
                        rel_path,
                        size,
                        loader,
                    } => {
                        let clean = rel_path.trim_start_matches('/');
                        builder = builder.lazy_file(
                            &format!("{repo_root}/{clean}"),
                            *size,
                            Arc::clone(loader),
                        );
                    }
                }
            }

            // BuildCache omission stubs (`.humata-omitted.json`).
            for stub in &sess.build_cache_stubs {
                let clean = stub.rel_path.trim_matches('/');
                let dir_path = format!("{repo_root}/{clean}");
                let stub_file = format!("{dir_path}/{HUMATA_OMITTED_STUB_FILENAME}");
                let stub_bytes = stub.to_json_bytes()?;
                builder = builder.dir(&dir_path).file(&stub_file, stub_bytes);
            }

            // Machine alias: /m/<machine_id>/s/<session_id> -> /s/<session_id>
            let m_root = format!("/m/{}", sess.machine_id);
            let m_s_dir = format!("{m_root}/s");
            let m_alias = format!("{m_s_dir}/{}", sess.session_id);
            builder = builder
                .dir(&m_root)
                .protect_root(&m_root)
                .dir(&m_s_dir)
                .protect_root(&m_s_dir)
                .symlink(&m_alias, &s_root)
                .protect_root(&m_alias);

            // Channel alias: /channels/<channel>/<session_id> -> /s/<session_id>
            if let Some(ch) = &sess.channel {
                let ch_dir = format!("/channels/{ch}");
                let ch_alias = format!("{ch_dir}/{}", sess.session_id);
                builder = builder
                    .dir(&ch_dir)
                    .protect_root(&ch_dir)
                    .symlink(&ch_alias, &s_root)
                    .protect_root(&ch_alias);
            }

            // Desk alias: /desks/<desk>/<session_id> -> /s/<session_id>
            if let Some(desk) = &sess.desk
                && desk_links.insert((desk.clone(), sess.session_id.clone()))
            {
                let desk_dir = format!("/desks/{desk}");
                let desk_alias = format!("{desk_dir}/{}", sess.session_id);
                builder = builder
                    .dir(&desk_dir)
                    .protect_root(&desk_dir)
                    .symlink(&desk_alias, &s_root)
                    .protect_root(&desk_alias);
            }
        }

        builder.build()
    }
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

/// Stable runtime record for a live session holder, preserved across `humatad2`
/// kill/restart cycles and same-machine channel renames.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionHolderRecord {
    pub session_id: String,
    pub holder_pid: u32,
    pub native_path: PathBuf,
    pub channel: Option<String>,
}

/// Long-lived Humata volume host that stays mounted and serving across
/// `humatad2` disconnects/restarts and same-machine channel renames.
pub struct HumataVolumeHost {
    volume: RwLock<Arc<InMemoryVolume>>,
    topology: RwLock<HumataTopology>,
    daemon_link: RwLock<DaemonLinkState>,
    holders: RwLock<BTreeMap<String, SessionHolderRecord>>,
    reconnect_count: AtomicU64,
    leases: Option<Arc<LeaseTable>>,
    access_hook: Option<Arc<dyn AccessHook>>,
    clock: Option<Arc<dyn Fn() -> u64 + Send + Sync>>,
}

impl HumataVolumeHost {
    /// Creates a new volume host from an initial topology and daemon PID.
    pub fn new(
        daemon_pid: u32,
        topology: HumataTopology,
        leases: Option<Arc<LeaseTable>>,
        access_hook: Option<Arc<dyn AccessHook>>,
        clock: Option<Arc<dyn Fn() -> u64 + Send + Sync>>,
    ) -> VfsResult<Self> {
        let vol = HumataTreeProjector::project(
            &topology,
            leases.clone(),
            access_hook.clone(),
            clock.clone(),
        )?;
        let mut holders = BTreeMap::new();
        for sess in &topology.sessions {
            holders.insert(
                sess.session_id.clone(),
                SessionHolderRecord {
                    session_id: sess.session_id.clone(),
                    holder_pid: sess.holder_pid,
                    native_path: sess.native_path(),
                    channel: sess.channel.clone(),
                },
            );
        }

        Ok(Self {
            volume: RwLock::new(Arc::new(vol)),
            topology: RwLock::new(topology),
            daemon_link: RwLock::new(DaemonLinkState::Connected {
                daemon_pid,
                epoch: 1,
            }),
            holders: RwLock::new(holders),
            reconnect_count: AtomicU64::new(0),
            leases,
            access_hook,
            clock,
        })
    }

    /// Returns the current projected [`InMemoryVolume`] handle.
    pub fn current_volume(&self) -> Arc<InMemoryVolume> {
        Arc::clone(&self.volume.read().expect("volume lock poisoned"))
    }

    /// Returns the current daemon link state.
    pub fn daemon_link_state(&self) -> DaemonLinkState {
        self.daemon_link.read().expect("daemon link lock poisoned").clone()
    }

    /// Returns how many times `humatad2` has reconnected since mount startup.
    pub fn reconnect_count(&self) -> u64 {
        self.reconnect_count.load(Ordering::Relaxed)
    }

    /// Returns the live holder record for `session_id` (if tracked).
    pub fn holder_record(&self, session_id: &str) -> Option<SessionHolderRecord> {
        self.holders
            .read()
            .expect("holders lock poisoned")
            .get(session_id)
            .cloned()
    }

    /// Called when `humatad2` is killed or disconnects.
    ///
    /// The mounted volume remains live and continues serving all projected paths
    /// and open handles without disturbing any session holder PID.
    pub fn on_daemon_disconnect(&self) -> DaemonLinkState {
        let mut link = self.daemon_link.write().expect("daemon link lock poisoned");
        let next = match *link {
            DaemonLinkState::Connected { daemon_pid, epoch } => DaemonLinkState::Disconnected {
                last_daemon_pid: Some(daemon_pid),
                epoch,
            },
            DaemonLinkState::Disconnected {
                last_daemon_pid,
                epoch,
            } => DaemonLinkState::Disconnected {
                last_daemon_pid,
                epoch,
            },
        };
        *link = next.clone();
        next
    }

    /// Called when a restarted `humatad2` reconnects with a refreshed topology.
    ///
    /// Preserves existing session `holder_pid` and stable `.sessions/<id>/`
    /// native paths while atomically swapping in the refreshed virtual view.
    pub fn on_daemon_reconnect(
        &self,
        new_daemon_pid: u32,
        mut refreshed_topology: HumataTopology,
    ) -> VfsResult<DaemonLinkState> {
        // Preserve live holder PIDs for sessions that were already running.
        {
            let mut holders = self.holders.write().expect("holders lock poisoned");
            for sess in &mut refreshed_topology.sessions {
                if let Some(existing) = holders.get_mut(&sess.session_id) {
                    sess.holder_pid = existing.holder_pid;
                    existing.channel = sess.channel.clone();
                } else {
                    holders.insert(
                        sess.session_id.clone(),
                        SessionHolderRecord {
                            session_id: sess.session_id.clone(),
                            holder_pid: sess.holder_pid,
                            native_path: sess.native_path(),
                            channel: sess.channel.clone(),
                        },
                    );
                }
            }
        }

        let new_vol = HumataTreeProjector::project(
            &refreshed_topology,
            self.leases.clone(),
            self.access_hook.clone(),
            self.clock.clone(),
        )?;

        *self.volume.write().expect("volume lock poisoned") = Arc::new(new_vol);
        *self.topology.write().expect("topology lock poisoned") = refreshed_topology;

        self.reconnect_count.fetch_add(1, Ordering::Relaxed);
        let mut link = self.daemon_link.write().expect("daemon link lock poisoned");
        let next_epoch = link.epoch().saturating_add(1);
        let next = DaemonLinkState::Connected {
            daemon_pid: new_daemon_pid,
            epoch: next_epoch,
        };
        *link = next.clone();
        Ok(next)
    }

    /// Performs a same-machine channel rename (`old_channel` -> `new_channel`).
    ///
    /// Only updates the `/channels/<name>/<session_id>` symlink alias; canonical
    /// `/s/<id>` paths, native `.sessions/<id>/` placement, and live `holder_pid`
    /// values remain completely unchanged.
    pub fn rename_channel_same_machine(
        &self,
        old_channel: &str,
        new_channel: &str,
    ) -> VfsResult<usize> {
        let mut topo = self.topology.write().expect("topology lock poisoned");
        let mut updated = 0usize;
        for sess in &mut topo.sessions {
            if sess.channel.as_deref() == Some(old_channel) {
                sess.channel = Some(new_channel.to_string());
                updated += 1;
            }
        }
        {
            let mut holders = self.holders.write().expect("holders lock poisoned");
            for rec in holders.values_mut() {
                if rec.channel.as_deref() == Some(old_channel) {
                    rec.channel = Some(new_channel.to_string());
                }
            }
        }
        let new_vol = HumataTreeProjector::project(
            &topo,
            self.leases.clone(),
            self.access_hook.clone(),
            self.clock.clone(),
        )?;
        *self.volume.write().expect("volume lock poisoned") = Arc::new(new_vol);
        Ok(updated)
    }

    /// Resolves `raw_path` through the currently active volume.
    pub fn resolve_path(&self, caller: &CallerContext, raw_path: &str) -> VfsResult<FileAttr> {
        self.current_volume().resolve_path(caller, raw_path)
    }
}

impl Volume for HumataVolumeHost {
    fn lookup(&self, caller: &CallerContext, parent: NodeId, name: &str) -> VfsResult<FileAttr> {
        self.current_volume().lookup(caller, parent, name)
    }

    fn getattr(&self, caller: &CallerContext, ino: NodeId) -> VfsResult<FileAttr> {
        self.current_volume().getattr(caller, ino)
    }

    fn setattr(&self, caller: &CallerContext, ino: NodeId, attr: SetAttr) -> VfsResult<FileAttr> {
        self.current_volume().setattr(caller, ino, attr)
    }

    fn readdir(&self, caller: &CallerContext, ino: NodeId, offset: u64) -> VfsResult<Vec<DirEntry>> {
        self.current_volume().readdir(caller, ino, offset)
    }

    fn readlink(&self, caller: &CallerContext, ino: NodeId) -> VfsResult<String> {
        self.current_volume().readlink(caller, ino)
    }

    fn mkdir(
        &self,
        caller: &CallerContext,
        parent: NodeId,
        name: &str,
        mode: u32,
    ) -> VfsResult<FileAttr> {
        self.current_volume().mkdir(caller, parent, name, mode)
    }

    fn rmdir(&self, caller: &CallerContext, parent: NodeId, name: &str) -> VfsResult<()> {
        self.current_volume().rmdir(caller, parent, name)
    }

    fn unlink(&self, caller: &CallerContext, parent: NodeId, name: &str) -> VfsResult<()> {
        self.current_volume().unlink(caller, parent, name)
    }

    fn rename(
        &self,
        caller: &CallerContext,
        parent: NodeId,
        name: &str,
        new_parent: NodeId,
        new_name: &str,
    ) -> VfsResult<()> {
        self.current_volume()
            .rename(caller, parent, name, new_parent, new_name)
    }

    fn symlink(
        &self,
        caller: &CallerContext,
        parent: NodeId,
        name: &str,
        target: &str,
    ) -> VfsResult<FileAttr> {
        self.current_volume().symlink(caller, parent, name, target)
    }

    fn link(
        &self,
        caller: &CallerContext,
        ino: NodeId,
        new_parent: NodeId,
        new_name: &str,
    ) -> VfsResult<FileAttr> {
        self.current_volume().link(caller, ino, new_parent, new_name)
    }

    fn getxattr(&self, caller: &CallerContext, ino: NodeId, name: &str) -> VfsResult<Vec<u8>> {
        self.current_volume().getxattr(caller, ino, name)
    }

    fn listxattr(&self, caller: &CallerContext, ino: NodeId) -> VfsResult<Vec<String>> {
        self.current_volume().listxattr(caller, ino)
    }

    fn setxattr(
        &self,
        caller: &CallerContext,
        ino: NodeId,
        name: &str,
        value: &[u8],
    ) -> VfsResult<()> {
        self.current_volume().setxattr(caller, ino, name, value)
    }

    fn removexattr(&self, caller: &CallerContext, ino: NodeId, name: &str) -> VfsResult<()> {
        self.current_volume().removexattr(caller, ino, name)
    }

    fn statfs(&self, caller: &CallerContext, ino: NodeId) -> VfsResult<StatFs> {
        self.current_volume().statfs(caller, ino)
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
        self.current_volume()
            .copy_file_range(caller, ino_in, offset_in, ino_out, offset_out, len)
    }

    fn open(&self, caller: &CallerContext, ino: NodeId, flags: OpenFlags) -> VfsResult<FileHandle> {
        self.current_volume().open(caller, ino, flags)
    }

    fn release(&self, caller: &CallerContext, ino: NodeId, fh: FileHandle) -> VfsResult<()> {
        self.current_volume().release(caller, ino, fh)
    }

    fn fsync(
        &self,
        caller: &CallerContext,
        ino: NodeId,
        fh: FileHandle,
        datasync: bool,
    ) -> VfsResult<()> {
        self.current_volume().fsync(caller, ino, fh, datasync)
    }

    fn flock(
        &self,
        caller: &CallerContext,
        ino: NodeId,
        fh: FileHandle,
        op: FlockOp,
        non_blocking: bool,
    ) -> VfsResult<()> {
        self.current_volume()
            .flock(caller, ino, fh, op, non_blocking)
    }

    fn read(
        &self,
        caller: &CallerContext,
        ino: NodeId,
        fh: FileHandle,
        offset: u64,
        size: u32,
    ) -> VfsResult<Vec<u8>> {
        self.current_volume().read(caller, ino, fh, offset, size)
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
        self.current_volume()
            .write(caller, ino, fh, offset, data, expected_generation)
    }
}

/// Helper to read a regular file's full bytes via [`Volume`].
pub fn read_all_bytes(
    volume: &dyn Volume,
    caller: &CallerContext,
    attr: &FileAttr,
) -> VfsResult<Vec<u8>> {
    let fh = volume.open(caller, attr.ino, OpenFlags::read_only())?;
    let bytes = volume.read(caller, attr.ino, fh, 0, attr.size as u32);
    let _ = volume.release(caller, attr.ino, fh);
    bytes
}

/// Helper to validate that a path is within a session's canonical root.
pub fn session_canonical_root(session_id: &str) -> VfsResult<VirtualPath> {
    VirtualPath::parse(&format!("/s/{session_id}"))
}
