//! Kernel PID-to-caller binding with process-tree, PGID, and SID lineage lookup.
//!
//! Every FUSE request carries a kernel-authenticated `(pid, uid, gid)` tuple.
//! [`PidRegistry`] binds registered PIDs, ancestor PIDs, process groups (PGID),
//! or sessions (SID) to a [`CallerContext`] and capability root. Any process
//! that cannot be matched fails closed with [`VfsError::Eacces`].

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, RwLock};

use fusefs_vfs::{CallerContext, VfsError, VfsResult, VirtualPath};
use serde::{Deserialize, Serialize};

/// Maximum ancestor hops walked when resolving a child process to a registered parent PID.
const MAX_LINEAGE_DEPTH: usize = 64;

/// Process lineage metadata resolved from the host (e.g., `/proc/<pid>/stat` on Linux).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PidLineage {
    pub pid: u32,
    pub ppid: Option<u32>,
    pub pgid: Option<u32>,
    pub sid: Option<u32>,
}

/// Strategy for resolving a kernel PID's parent/process-group/session lineage.
pub trait LineageResolver: Send + Sync {
    fn resolve_lineage(&self, pid: u32) -> Option<PidLineage>;
}

/// Default Linux `/proc/<pid>/stat` lineage resolver (no-op returning `None` on non-Linux hosts).
#[derive(Debug, Default, Clone, Copy)]
pub struct ProcfsLineageResolver;

impl LineageResolver for ProcfsLineageResolver {
    fn resolve_lineage(&self, pid: u32) -> Option<PidLineage> {
        #[cfg(target_os = "linux")]
        {
            let stat_path = format!("/proc/{pid}/stat");
            let contents = std::fs::read_to_string(stat_path).ok()?;
            parse_proc_stat_lineage(pid, &contents)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = pid;
            None
        }
    }
}

/// Parses Linux `/proc/<pid>/stat` contents into a [`PidLineage`].
///
/// Format per `proc(5)`:
/// `pid (comm) state ppid pgrp session ...`
/// Because `comm` may contain spaces or parentheses, parsing anchors at the last `')'`.
pub fn parse_proc_stat_lineage(pid: u32, stat_contents: &str) -> Option<PidLineage> {
    let close_paren = stat_contents.rfind(')')?;
    let rest = stat_contents.get(close_paren + 1..)?.trim();
    let mut fields = rest.split_whitespace();
    let _state = fields.next()?;
    let ppid = fields.next()?.parse::<u32>().ok().filter(|&v| v > 0);
    let pgid = fields.next()?.parse::<u32>().ok().filter(|&v| v > 0);
    let sid = fields.next()?.parse::<u32>().ok().filter(|&v| v > 0);
    Some(PidLineage {
        pid,
        ppid,
        pgid,
        sid,
    })
}

/// Deterministic in-memory [`LineageResolver`] for testing or explicit process-tree tracking.
#[derive(Debug, Default)]
pub struct StaticLineageResolver {
    entries: RwLock<BTreeMap<u32, PidLineage>>,
}

impl StaticLineageResolver {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, lineage: PidLineage) {
        self.entries
            .write()
            .expect("lineage lock poisoned")
            .insert(lineage.pid, lineage);
    }

    pub fn remove(&self, pid: u32) -> Option<PidLineage> {
        self.entries
            .write()
            .expect("lineage lock poisoned")
            .remove(&pid)
    }
}

impl LineageResolver for StaticLineageResolver {
    fn resolve_lineage(&self, pid: u32) -> Option<PidLineage> {
        self.entries
            .read()
            .expect("lineage lock poisoned")
            .get(&pid)
            .copied()
    }
}

/// Capability binding associated with a registered PID, PGID, or SID.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PidBinding {
    /// Normalized virtual capability root path (e.g., `"/workspace/repo"`).
    pub capability_root: String,
    /// Optional UID override; when `None`, preserves the kernel request's UID.
    pub uid_override: Option<u32>,
    /// Optional GID override; when `None`, preserves the kernel request's GID.
    pub gid_override: Option<u32>,
    /// Optional opaque label for diagnostics/auditing.
    pub label: Option<String>,
}

impl PidBinding {
    /// Creates a new binding after validating and normalizing `capability_root`.
    pub fn new(capability_root: impl AsRef<str>) -> VfsResult<Self> {
        let normalized = VirtualPath::parse(capability_root.as_ref())?;
        Ok(Self {
            capability_root: normalized.as_str().to_string(),
            uid_override: None,
            gid_override: None,
            label: None,
        })
    }

    pub fn with_uid_gid(mut self, uid: u32, gid: u32) -> Self {
        self.uid_override = Some(uid);
        self.gid_override = Some(gid);
        self
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    fn into_caller(&self, pid: u32, uid: u32, gid: u32) -> CallerContext {
        CallerContext::new(
            pid,
            self.uid_override.unwrap_or(uid),
            self.gid_override.unwrap_or(gid),
        )
        .with_capability_root(self.capability_root.clone())
    }
}

#[derive(Debug, Default)]
struct RegistryState {
    by_pid: BTreeMap<u32, PidBinding>,
    by_pgid: BTreeMap<u32, PidBinding>,
    by_sid: BTreeMap<u32, PidBinding>,
}

/// Thread-safe registry mapping kernel PIDs, process groups, and sessions to [`CallerContext`].
///
/// Unknown PIDs always fail closed with [`VfsError::Eacces`].
pub struct PidRegistry {
    state: RwLock<RegistryState>,
    lineage: Arc<dyn LineageResolver>,
}

impl Default for PidRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl PidRegistry {
    /// Creates a [`PidRegistry`] backed by [`ProcfsLineageResolver`].
    pub fn new() -> Self {
        Self::with_lineage_resolver(Arc::new(ProcfsLineageResolver))
    }

    /// Creates a [`PidRegistry`] with a custom [`LineageResolver`].
    pub fn with_lineage_resolver(lineage: Arc<dyn LineageResolver>) -> Self {
        Self {
            state: RwLock::new(RegistryState::default()),
            lineage,
        }
    }

    /// Registers a direct PID binding to `capability_root`.
    pub fn register_pid(&self, pid: u32, capability_root: impl AsRef<str>) -> VfsResult<()> {
        let binding = PidBinding::new(capability_root)?;
        self.register_pid_binding(pid, binding);
        Ok(())
    }

    /// Registers an explicit [`PidBinding`] for `pid`.
    pub fn register_pid_binding(&self, pid: u32, binding: PidBinding) {
        self.state
            .write()
            .expect("pid registry lock poisoned")
            .by_pid
            .insert(pid, binding);
    }

    /// Registers a process-group (PGID) binding to `capability_root`.
    pub fn register_pgid(&self, pgid: u32, capability_root: impl AsRef<str>) -> VfsResult<()> {
        let binding = PidBinding::new(capability_root)?;
        self.state
            .write()
            .expect("pid registry lock poisoned")
            .by_pgid
            .insert(pgid, binding);
        Ok(())
    }

    /// Registers a session (SID) binding to `capability_root`.
    pub fn register_sid(&self, sid: u32, capability_root: impl AsRef<str>) -> VfsResult<()> {
        let binding = PidBinding::new(capability_root)?;
        self.state
            .write()
            .expect("pid registry lock poisoned")
            .by_sid
            .insert(sid, binding);
        Ok(())
    }

    pub fn unregister_pid(&self, pid: u32) -> bool {
        self.state
            .write()
            .expect("pid registry lock poisoned")
            .by_pid
            .remove(&pid)
            .is_some()
    }

    pub fn unregister_pgid(&self, pgid: u32) -> bool {
        self.state
            .write()
            .expect("pid registry lock poisoned")
            .by_pgid
            .remove(&pgid)
            .is_some()
    }

    pub fn unregister_sid(&self, sid: u32) -> bool {
        self.state
            .write()
            .expect("pid registry lock poisoned")
            .by_sid
            .remove(&sid)
            .is_some()
    }

    /// Resolves `(pid, uid, gid)` into a capability-bound [`CallerContext`].
    ///
    /// Resolution order:
    /// 1. Direct `pid` match.
    /// 2. Ancestor `ppid` chain walk via [`LineageResolver`] (bounded depth, cycle-safe).
    /// 3. Process group (`pgid`) match.
    /// 4. Session (`sid`) match.
    /// 5. **Fail closed**: returns [`VfsError::Eacces`].
    pub fn resolve(&self, pid: u32, uid: u32, gid: u32) -> VfsResult<CallerContext> {
        if pid == 0 {
            return Err(VfsError::Eacces(
                "kernel pid 0 is not bound to any capability root".to_string(),
            ));
        }

        let state = self.state.read().expect("pid registry lock poisoned");

        // 1. Direct PID match.
        if let Some(binding) = state.by_pid.get(&pid) {
            return Ok(binding.into_caller(pid, uid, gid));
        }

        // 2. Lineage lookup: walk parent chain, then check PGID and SID.
        if let Some(initial) = self.lineage.resolve_lineage(pid) {
            let mut visited = BTreeSet::new();
            visited.insert(pid);
            let mut current_ppid = initial.ppid;

            for _ in 0..MAX_LINEAGE_DEPTH {
                let Some(ancestor) = current_ppid else {
                    break;
                };
                if !visited.insert(ancestor) {
                    break;
                }
                if let Some(binding) = state.by_pid.get(&ancestor) {
                    return Ok(binding.into_caller(pid, uid, gid));
                }
                current_ppid = self.lineage.resolve_lineage(ancestor).and_then(|l| l.ppid);
            }

            if let Some(pgid) = initial.pgid
                && let Some(binding) = state.by_pgid.get(&pgid)
            {
                return Ok(binding.into_caller(pid, uid, gid));
            }

            if let Some(sid) = initial.sid
                && let Some(binding) = state.by_sid.get(&sid)
            {
                return Ok(binding.into_caller(pid, uid, gid));
            }
        }

        // 3. Fail closed for unknown PIDs.
        Err(VfsError::Eacces(format!(
            "unregistered kernel pid {pid}: access denied (fail-closed)"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_pid_binding_returns_capability_root_and_unknown_fails_closed() {
        let registry = PidRegistry::new();
        registry.register_pid(4242, "/workspace/project-a").unwrap();

        let caller = registry.resolve(4242, 1000, 1000).unwrap();
        assert_eq!(caller.pid, 4242);
        assert_eq!(caller.uid, 1000);
        assert_eq!(caller.gid, 1000);
        assert_eq!(
            caller.capability_root.as_deref(),
            Some("/workspace/project-a")
        );

        let err = registry.resolve(9999, 1000, 1000).unwrap_err();
        assert!(matches!(err, VfsError::Eacces(_)));

        let zero_err = registry.resolve(0, 0, 0).unwrap_err();
        assert!(matches!(zero_err, VfsError::Eacces(_)));
    }

    #[test]
    fn child_process_inherits_registered_ancestor_pid_binding() {
        let lineage = Arc::new(StaticLineageResolver::new());
        lineage.insert(PidLineage {
            pid: 100,
            ppid: Some(1),
            pgid: Some(100),
            sid: Some(100),
        });
        lineage.insert(PidLineage {
            pid: 101,
            ppid: Some(100),
            pgid: Some(100),
            sid: Some(100),
        });
        lineage.insert(PidLineage {
            pid: 102,
            ppid: Some(101),
            pgid: Some(100),
            sid: Some(100),
        });

        let registry = PidRegistry::with_lineage_resolver(lineage);
        registry.register_pid(100, "/tenant/alpha").unwrap();

        let grandchild = registry.resolve(102, 501, 20).unwrap();
        assert_eq!(grandchild.pid, 102);
        assert_eq!(grandchild.capability_root.as_deref(), Some("/tenant/alpha"));
    }

    #[test]
    fn pgid_and_sid_bindings_resolve_when_pid_not_directly_registered() {
        let lineage = Arc::new(StaticLineageResolver::new());
        lineage.insert(PidLineage {
            pid: 201,
            ppid: Some(1),
            pgid: Some(200),
            sid: Some(50),
        });
        lineage.insert(PidLineage {
            pid: 301,
            ppid: Some(1),
            pgid: Some(300),
            sid: Some(60),
        });

        let registry = PidRegistry::with_lineage_resolver(lineage);
        registry.register_pgid(200, "/pgid-root").unwrap();
        registry.register_sid(60, "/sid-root").unwrap();

        let by_pgid = registry.resolve(201, 1000, 1000).unwrap();
        assert_eq!(by_pgid.capability_root.as_deref(), Some("/pgid-root"));

        let by_sid = registry.resolve(301, 1000, 1000).unwrap();
        assert_eq!(by_sid.capability_root.as_deref(), Some("/sid-root"));
    }

    #[test]
    fn parse_proc_stat_handles_comm_with_spaces_and_parentheses() {
        let stat = "1234 (worker (sub) thread) S 1000 1000 500 0 -1 4194304";
        let lineage = parse_proc_stat_lineage(1234, stat).unwrap();
        assert_eq!(
            lineage,
            PidLineage {
                pid: 1234,
                ppid: Some(1000),
                pgid: Some(1000),
                sid: Some(500),
            }
        );
    }

    #[test]
    fn cyclic_lineage_terminates_and_fails_closed() {
        let lineage = Arc::new(StaticLineageResolver::new());
        lineage.insert(PidLineage {
            pid: 10,
            ppid: Some(11),
            pgid: None,
            sid: None,
        });
        lineage.insert(PidLineage {
            pid: 11,
            ppid: Some(10),
            pgid: None,
            sid: None,
        });

        let registry = PidRegistry::with_lineage_resolver(lineage);
        let err = registry.resolve(10, 1000, 1000).unwrap_err();
        assert!(matches!(err, VfsError::Eacces(_)));
    }
}
