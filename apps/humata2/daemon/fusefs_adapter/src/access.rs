//! T9 & T10: `HumataAccessHook` (subtree `WriteLease`s, `< 50 ms` `EACCES` +
//! `requests/<id>.json` ask descriptors, `ESTALE` generation checks, local
//! `away_journal` recording) and `MASSDELETE` `EPERM` destructive root safety.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use fusefs_vfs::{
    AccessHook, CallerContext, LeaseTable, MetadataOp, VfsError, VfsResult, VfsTarget, VirtualPath,
    WriteLease,
};
use serde::{Deserialize, Serialize};

/// Maximum duration for a turn-bounded subtree [`WriteLease`] (10 minutes).
pub const MAX_TURN_LEASE_DURATION_MS: u64 = 10 * 60 * 1000;

/// Ask descriptor written to `requests/<id>.json` whenever an unauthorized
/// cross-session or cross-machine write is denied with `EACCES`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteAskDescriptor {
    pub schema_version: u32,
    pub request_id: String,
    pub caller_pid: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caller_session_id: Option<String>,
    pub target_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_machine_id: Option<String>,
    pub operation: String,
    pub created_at_ms: u64,
    pub reason: String,
}

impl WriteAskDescriptor {
    pub fn filename(&self) -> String {
        format!("{}.json", self.request_id)
    }

    pub fn to_json_bytes(&self) -> VfsResult<Vec<u8>> {
        let mut out = serde_json::to_vec_pretty(self)
            .map_err(|source| VfsError::JsonIo {
                op: "serialize ask descriptor",
                target: VfsTarget::Record("write ask descriptor"),
                source,
            })?;
        out.push(b'\n');
        Ok(out)
    }
}

/// Entry appended to the local `away_journal` whenever `host_reachable == false`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AwayJournalEntry {
    pub seq: u64,
    pub timestamp_ms: u64,
    pub caller_pid: u32,
    pub path: String,
    pub operation: String,
    pub outcome: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

/// Local `AWAY` Inbox journal recording write attempts while `host_reachable == false`.
pub struct AwayJournal {
    entries: Mutex<Vec<AwayJournalEntry>>,
    next_seq: AtomicU64,
    disk_path: Option<PathBuf>,
}

impl AwayJournal {
    pub fn in_memory() -> Self {
        Self {
            entries: Mutex::new(Vec::new()),
            next_seq: AtomicU64::new(1),
            disk_path: None,
        }
    }

    pub fn with_file(path: impl Into<PathBuf>) -> Self {
        Self {
            entries: Mutex::new(Vec::new()),
            next_seq: AtomicU64::new(1),
            disk_path: Some(path.into()),
        }
    }

    pub fn record(
        &self,
        timestamp_ms: u64,
        caller_pid: u32,
        path: &str,
        operation: &str,
        outcome: &str,
        request_id: Option<String>,
    ) -> AwayJournalEntry {
        let seq = self.next_seq.fetch_add(1, Ordering::Relaxed);
        let entry = AwayJournalEntry {
            seq,
            timestamp_ms,
            caller_pid,
            path: path.to_string(),
            operation: operation.to_string(),
            outcome: outcome.to_string(),
            request_id,
        };
        {
            let mut guard = self.entries.lock().expect("away journal lock poisoned");
            guard.push(entry.clone());
        }
        if let Some(disk_path) = &self.disk_path {
            if let Some(parent) = disk_path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            if let Ok(mut line) = serde_json::to_string(&entry) {
                line.push('\n');
                if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(disk_path) {
                    let _ = file.write_all(line.as_bytes());
                }
            }
        }
        entry
    }

    pub fn entries(&self) -> Vec<AwayJournalEntry> {
        self.entries
            .lock()
            .expect("away journal lock poisoned")
            .clone()
    }

    pub fn len(&self) -> usize {
        self.entries
            .lock()
            .expect("away journal lock poisoned")
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// `MASSDELETE` destructive safety policy: returns `true` for any Humata
/// protected root (session, repo, config, channel, desk, machine, and alias
/// roots) so `rm -rf` or `mv` can never bypass `PlanDeleteSessions` or
/// `MoveSession`.
#[derive(Debug, Default, Clone)]
pub struct ProtectedRootPolicy {
    extra_roots: BTreeSet<VirtualPath>,
}

impl ProtectedRootPolicy {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_extra_root(mut self, path: VirtualPath) -> Self {
        self.extra_roots.insert(path);
        self
    }

    /// Returns `true` if `path` is a protected Humata root that MUST fail with
    /// `EPERM` on `unlink`, `rmdir`, or `rename`.
    pub fn is_protected(&self, path: &VirtualPath) -> bool {
        if path.is_root() || self.extra_roots.contains(path) {
            return true;
        }
        let segs: Vec<&str> = path.segments().collect();
        match segs.as_slice() {
            // Top-level virtual roots.
            ["s" | "m" | "channels" | "desks" | "machines" | "requests" | "control" | "context"] => {
                true
            }
            // Session roots: /s/<id>, /s/<id>/repo, /s/<id>/config, /s/<id>/config/<agent>, /s/<id>/context
            ["s", _] => true,
            ["s", _, "repo" | "config" | "context"] => true,
            ["s", _, "config", _] => true,
            // Machine roots & machine session aliases: /m/<m>, /m/<m>/s, /m/<m>/s/<id>
            ["m", _] => true,
            ["m", _, "s"] => true,
            ["m", _, "s", _] => true,
            // Machine info roots: /machines/<m>
            ["machines", _] => true,
            // Channel roots & aliases: /channels/<ch>, /channels/<ch>/<id>
            ["channels", _] => true,
            ["channels", _, _] => true,
            // Desk roots & aliases: /desks/<desk>, /desks/<desk>/<id>
            ["desks", _] => true,
            ["desks", _, _] => true,
            _ => false,
        }
    }
}

/// Humata [`AccessHook`] enforcing:
/// - `MASSDELETE` `EPERM` on all protected session/repo/config/channel/desk/machine/alias roots.
/// - Turn-bounded (up to 10m) subtree [`WriteLease`] checks via [`LeaseTable`].
/// - `< 50 ms` `EACCES` + `requests/<id>.json` ask descriptor emission on unauthorized writes.
/// - Local [`AwayJournal`] recording whenever `host_reachable == false`.
pub struct HumataAccessHook {
    leases: Arc<LeaseTable>,
    clock: Arc<dyn Fn() -> u64 + Send + Sync>,
    host_reachable: Arc<AtomicBool>,
    requests_dir: Option<PathBuf>,
    requests: RwLock<BTreeMap<String, WriteAskDescriptor>>,
    away_journal: Arc<AwayJournal>,
    caller_sessions: RwLock<BTreeMap<u32, String>>,
    protected: ProtectedRootPolicy,
    next_request_seq: AtomicU64,
}

impl HumataAccessHook {
    pub fn new(
        leases: Arc<LeaseTable>,
        clock: Arc<dyn Fn() -> u64 + Send + Sync>,
        host_reachable: Arc<AtomicBool>,
    ) -> Self {
        Self {
            leases,
            clock,
            host_reachable,
            requests_dir: None,
            requests: RwLock::new(BTreeMap::new()),
            away_journal: Arc::new(AwayJournal::in_memory()),
            caller_sessions: RwLock::new(BTreeMap::new()),
            protected: ProtectedRootPolicy::new(),
            next_request_seq: AtomicU64::new(1),
        }
    }

    pub fn with_requests_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.requests_dir = Some(dir.into());
        self
    }

    pub fn with_away_journal(mut self, journal: Arc<AwayJournal>) -> Self {
        self.away_journal = journal;
        self
    }

    pub fn with_protected_policy(mut self, policy: ProtectedRootPolicy) -> Self {
        self.protected = policy;
        self
    }

    pub fn leases(&self) -> &Arc<LeaseTable> {
        &self.leases
    }

    pub fn away_journal(&self) -> &Arc<AwayJournal> {
        &self.away_journal
    }

    pub fn register_caller_session(&self, caller_pid: u32, session_id: impl Into<String>) {
        self.caller_sessions
            .write()
            .expect("caller sessions lock poisoned")
            .insert(caller_pid, session_id.into());
    }

    /// Grants a turn-bounded subtree [`WriteLease`] clamped to [`MAX_TURN_LEASE_DURATION_MS`] (10m).
    pub fn grant_turn_lease(
        &self,
        caller_pid: u32,
        subtree: VirtualPath,
        duration_ms: u64,
        turn_id: impl Into<String>,
    ) -> WriteLease {
        let clamped = duration_ms.min(MAX_TURN_LEASE_DURATION_MS);
        let expires_at_ms = (self.clock)().saturating_add(clamped);
        self.leases
            .grant(caller_pid, subtree, expires_at_ms, Some(turn_id.into()))
    }

    /// Revokes all write leases associated with `turn_id` on turn-end.
    pub fn on_turn_end(&self, turn_id: &str) -> usize {
        self.leases.revoke_turn(turn_id)
    }

    pub fn list_requests(&self) -> Vec<WriteAskDescriptor> {
        self.requests
            .read()
            .expect("requests lock poisoned")
            .values()
            .cloned()
            .collect()
    }

    pub fn get_request(&self, request_id: &str) -> Option<WriteAskDescriptor> {
        self.requests
            .read()
            .expect("requests lock poisoned")
            .get(request_id)
            .cloned()
    }

    fn emit_ask_descriptor(
        &self,
        caller: &CallerContext,
        path: &VirtualPath,
        op_name: &str,
        reason: &str,
    ) -> WriteAskDescriptor {
        let seq = self.next_request_seq.fetch_add(1, Ordering::Relaxed);
        let request_id = format!("req-{seq:06}");
        let now_ms = (self.clock)();
        let caller_session_id = self
            .caller_sessions
            .read()
            .expect("caller sessions lock poisoned")
            .get(&caller.pid)
            .cloned();
        let (target_session_id, target_machine_id) = parse_target_identity(path);

        let desc = WriteAskDescriptor {
            schema_version: 1,
            request_id: request_id.clone(),
            caller_pid: caller.pid,
            caller_session_id,
            target_path: path.as_str().to_string(),
            target_session_id,
            target_machine_id,
            operation: op_name.to_string(),
            created_at_ms: now_ms,
            reason: reason.to_string(),
        };

        self.requests
            .write()
            .expect("requests lock poisoned")
            .insert(request_id.clone(), desc.clone());

        if let Some(dir) = &self.requests_dir {
            let _ = fs::create_dir_all(dir);
            let target_file = dir.join(desc.filename());
            let tmp_file = dir.join(format!(".{request_id}.tmp"));
            if let Ok(bytes) = desc.to_json_bytes()
                && fs::write(&tmp_file, bytes).is_ok()
            {
                let _ = fs::rename(&tmp_file, &target_file);
            }
        }

        desc
    }

    fn check_write_access(
        &self,
        caller: &CallerContext,
        path: &VirtualPath,
        op_name: &'static str,
    ) -> VfsResult<()> {
        let now_ms = (self.clock)();
        match self.leases.authorize_write(caller, path, now_ms) {
            Ok(_lease_id) => {
                if !self.host_reachable.load(Ordering::Acquire) {
                    self.away_journal.record(
                        now_ms,
                        caller.pid,
                        path.as_str(),
                        op_name,
                        "allowed_local_write",
                        None,
                    );
                }
                Ok(())
            }
            Err(err) => {
                let desc = self.emit_ask_descriptor(caller, path, op_name, &err.to_string());
                if !self.host_reachable.load(Ordering::Acquire) {
                    self.away_journal.record(
                        now_ms,
                        caller.pid,
                        path.as_str(),
                        op_name,
                        "denied_ask_created",
                        Some(desc.request_id.clone()),
                    );
                }
                Err(VfsError::Eacces {
                    op: op_name,
                    target: VfsTarget::Request {
                        id: desc.request_id.clone(),
                        path: path.to_string(),
                    },
                })
            }
        }
    }
}

impl AccessHook for HumataAccessHook {
    fn before_metadata(
        &self,
        caller: &CallerContext,
        op: MetadataOp,
        path: &VirtualPath,
    ) -> VfsResult<()> {
        // MASSDELETE: destructive ops on any protected root fail with EPERM immediately.
        if matches!(
            op,
            MetadataOp::Rmdir | MetadataOp::Unlink | MetadataOp::Rename
        ) && self.protected.is_protected(path)
        {
            return Err(VfsError::Eperm {
                op: metadata_op_name(op),
                target: VfsTarget::Path(path.to_string()),
            });
        }

        // Mutating metadata ops require an active write lease.
        if is_mutating_metadata_op(op) {
            self.check_write_access(caller, path, metadata_op_name(op))?;
        }

        Ok(())
    }

    fn before_write(
        &self,
        caller: &CallerContext,
        path: &VirtualPath,
        _offset: u64,
        _size: usize,
    ) -> VfsResult<()> {
        self.check_write_access(caller, path, "write")
    }
}

fn is_mutating_metadata_op(op: MetadataOp) -> bool {
    matches!(
        op,
        MetadataOp::SetAttr
            | MetadataOp::Mkdir
            | MetadataOp::Rmdir
            | MetadataOp::Unlink
            | MetadataOp::Rename
            | MetadataOp::Symlink
            | MetadataOp::Link
            | MetadataOp::SetXattr
            | MetadataOp::RemoveXattr
            | MetadataOp::CopyFileRange
    )
}

fn metadata_op_name(op: MetadataOp) -> &'static str {
    match op {
        MetadataOp::SetAttr => "setattr",
        MetadataOp::Mkdir => "mkdir",
        MetadataOp::Rmdir => "rmdir",
        MetadataOp::Unlink => "unlink",
        MetadataOp::Rename => "rename",
        MetadataOp::Symlink => "symlink",
        MetadataOp::Link => "link",
        MetadataOp::SetXattr => "setxattr",
        MetadataOp::RemoveXattr => "removexattr",
        MetadataOp::CopyFileRange => "copy_file_range",
        _ => "metadata",
    }
}

fn parse_target_identity(path: &VirtualPath) -> (Option<String>, Option<String>) {
    let segs: Vec<&str> = path.segments().collect();
    match segs.as_slice() {
        ["s", sid, ..] => (Some((*sid).to_string()), None),
        ["m", mid, "s", sid, ..] => (Some((*sid).to_string()), Some((*mid).to_string())),
        ["m", mid, ..] => (None, Some((*mid).to_string())),
        ["channels", _, sid, ..] => (Some((*sid).to_string()), None),
        ["desks", _, sid, ..] => (Some((*sid).to_string()), None),
        _ => (None, None),
    }
}
