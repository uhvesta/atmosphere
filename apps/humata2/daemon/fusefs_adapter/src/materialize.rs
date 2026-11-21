//! T10: `fs status` summary and `fs materialize` byte-count estimator +
//! explicit confirmation planner.

use std::fmt;
use std::path::PathBuf;

use fusefs_host::MountState;
use fusefs_vfs::{CallerContext, FileKind, NodeId, ROOT_NODE_ID, VfsResult, Volume};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::layout::{BuildCacheStub, HUMATA_OMITTED_STUB_FILENAME, read_all_bytes};

/// Default byte threshold above which `fs materialize` requires explicit user
/// confirmation (50 MiB).
pub const DEFAULT_CONFIRMATION_THRESHOLD_BYTES: u64 = 50 * 1024 * 1024;

/// Active mount backend mode for `humata2_fusefs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FsMountMode {
    FsKit,
    Linux,
    Degraded,
}

#[cfg(target_os = "linux")]
impl From<fusefs_linux::MountMode> for FsMountMode {
    fn from(mode: fusefs_linux::MountMode) -> Self {
        match mode {
            fusefs_linux::MountMode::DevFuse => Self::Linux,
            fusefs_linux::MountMode::DegradedSymlinkSkeleton => Self::Degraded,
        }
    }
}

impl fmt::Display for FsMountMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FsKit => f.write_str("fskit"),
            Self::Linux => f.write_str("linux"),
            Self::Degraded => f.write_str("degraded"),
        }
    }
}

/// Human- and machine-readable `humata2 fs status` summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsStatusSummary {
    pub mode: FsMountMode,
    pub mount_state: MountState,
    pub mountpoint: PathBuf,
    pub nobrowse: bool,
    pub host_reachable: bool,
    pub daemon_connected: bool,
    pub active_sessions: usize,
    pub active_leases: usize,
    pub pending_requests: usize,
    pub cached_chunks: usize,
    pub cached_bytes: u64,
    pub cache_budget_bytes: u64,
    pub queued_invalidations: usize,
    pub away_journal_entries: usize,
}

impl FsStatusSummary {
    /// Formats a concise multi-line human-readable status report.
    pub fn format_human(&self) -> String {
        format!(
            "FUSEFS status:\n\
             \x20 mode:                 {}\n\
             \x20 mount_state:          {:?}\n\
             \x20 mountpoint:           {}\n\
             \x20 nobrowse:             {}\n\
             \x20 host_reachable:       {}\n\
             \x20 daemon_connected:     {}\n\
             \x20 active_sessions:      {}\n\
             \x20 active_leases:        {}\n\
             \x20 pending_requests:     {}\n\
             \x20 cache:                {} / {} ({} chunks)\n\
             \x20 queued_invalidations: {}\n\
             \x20 away_journal_entries: {}",
            self.mode,
            self.mount_state,
            self.mountpoint.display(),
            self.nobrowse,
            self.host_reachable,
            self.daemon_connected,
            self.active_sessions,
            self.active_leases,
            self.pending_requests,
            format_bytes(self.cached_bytes),
            format_bytes(self.cache_budget_bytes),
            self.cached_chunks,
            self.queued_invalidations,
            self.away_journal_entries,
        )
    }
}

/// Single omitted directory scheduled for explicit cache materialization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterializeTarget {
    pub virtual_path: String,
    pub estimated_bytes: u64,
    pub estimated_entries: u64,
    pub reason: String,
}

/// Byte-count and entry-count plan returned by [`MaterializePlanner`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterializePlan {
    pub targets: Vec<MaterializeTarget>,
    pub total_bytes: u64,
    pub total_entries: u64,
    pub confirmation_threshold_bytes: u64,
    pub requires_confirmation: bool,
}

impl MaterializePlan {
    /// Returns a clear confirmation prompt with exact byte and entry totals.
    pub fn confirmation_prompt(&self) -> String {
        format!(
            "Materialize {} omitted target(s) (~{}, {} entries)? Re-run with --yes to confirm.",
            self.targets.len(),
            format_bytes(self.total_bytes),
            self.total_entries,
        )
    }

    /// Verifies whether execution is permitted given `confirmed`.
    pub fn check_confirmed(&self, confirmed: bool) -> Result<(), MaterializeError> {
        if self.requires_confirmation && !confirmed {
            return Err(MaterializeError::ConfirmationRequired {
                total_bytes: self.total_bytes,
                total_entries: self.total_entries,
                prompt: self.confirmation_prompt(),
            });
        }
        Ok(())
    }
}

/// Execution report returned after materialization completes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterializeReport {
    pub materialized_targets: usize,
    pub materialized_bytes: u64,
}

/// Errors returned by [`MaterializePlanner`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum MaterializeError {
    #[error("explicit confirmation required before materializing {total_bytes} bytes ({total_entries} entries): {prompt}")]
    ConfirmationRequired {
        total_bytes: u64,
        total_entries: u64,
        prompt: String,
    },
    #[error("materialization failed for '{path}': {message}")]
    ExecutionFailed { path: String, message: String },
}

/// Estimator and explicit confirmation planner for `humata2 fs materialize`.
#[derive(Debug, Clone)]
pub struct MaterializePlanner {
    confirmation_threshold_bytes: u64,
    always_require_confirmation: bool,
}

impl Default for MaterializePlanner {
    fn default() -> Self {
        Self::new(DEFAULT_CONFIRMATION_THRESHOLD_BYTES)
    }
}

impl MaterializePlanner {
    pub fn new(confirmation_threshold_bytes: u64) -> Self {
        Self {
            confirmation_threshold_bytes,
            always_require_confirmation: false,
        }
    }

    pub fn with_always_confirm(mut self, always: bool) -> Self {
        self.always_require_confirmation = always;
        self
    }

    /// Builds a [`MaterializePlan`] from an explicit list of `(virtual_path, BuildCacheStub)` pairs.
    pub fn plan_from_stubs(&self, stubs: &[(String, BuildCacheStub)]) -> MaterializePlan {
        let mut targets = Vec::with_capacity(stubs.len());
        let mut total_bytes = 0u64;
        let mut total_entries = 0u64;

        for (vpath, stub) in stubs {
            total_bytes = total_bytes.saturating_add(stub.original_bytes);
            total_entries = total_entries.saturating_add(stub.original_entries);
            targets.push(MaterializeTarget {
                virtual_path: vpath.clone(),
                estimated_bytes: stub.original_bytes,
                estimated_entries: stub.original_entries,
                reason: stub.reason.clone(),
            });
        }

        let requires_confirmation = !targets.is_empty()
            && (self.always_require_confirmation
                || total_bytes >= self.confirmation_threshold_bytes);

        MaterializePlan {
            targets,
            total_bytes,
            total_entries,
            confirmation_threshold_bytes: self.confirmation_threshold_bytes,
            requires_confirmation,
        }
    }

    /// Walks `root_path` inside `volume` using zero-data directory traversal,
    /// reads only `.humata-omitted.json` stubs, and builds a [`MaterializePlan`].
    pub fn plan_from_volume(
        &self,
        volume: &dyn Volume,
        caller: &CallerContext,
        root_path: &str,
    ) -> VfsResult<MaterializePlan> {
        let start_ino = resolve_ino(volume, caller, root_path)?;
        let mut discovered = Vec::new();
        walk_omitted_stubs(volume, caller, start_ino, root_path, &mut discovered)?;
        Ok(self.plan_from_stubs(&discovered))
    }

    /// Executes `plan` only if confirmation requirements are satisfied.
    pub fn execute_plan<F>(
        &self,
        plan: &MaterializePlan,
        confirmed: bool,
        mut materialize_fn: F,
    ) -> Result<MaterializeReport, MaterializeError>
    where
        F: FnMut(&MaterializeTarget) -> Result<u64, String>,
    {
        plan.check_confirmed(confirmed)?;
        let mut materialized_bytes = 0u64;
        for target in &plan.targets {
            let bytes = materialize_fn(target).map_err(|message| {
                MaterializeError::ExecutionFailed {
                    path: target.virtual_path.clone(),
                    message,
                }
            })?;
            materialized_bytes = materialized_bytes.saturating_add(bytes);
        }
        Ok(MaterializeReport {
            materialized_targets: plan.targets.len(),
            materialized_bytes,
        })
    }
}

fn resolve_ino(volume: &dyn Volume, caller: &CallerContext, raw_path: &str) -> VfsResult<NodeId> {
    let mut current = ROOT_NODE_ID;
    for seg in raw_path.split('/').filter(|s| !s.is_empty()) {
        let attr = volume.lookup(caller, current, seg)?;
        current = attr.ino;
    }
    Ok(current)
}

fn walk_omitted_stubs(
    volume: &dyn Volume,
    caller: &CallerContext,
    dir_ino: NodeId,
    dir_path: &str,
    out: &mut Vec<(String, BuildCacheStub)>,
) -> VfsResult<()> {
    let entries = volume.readdir(caller, dir_ino, 0)?;
    for entry in entries {
        if entry.name == "." || entry.name == ".." {
            continue;
        }
        let child_path = if dir_path == "/" {
            format!("/{}", entry.name)
        } else {
            format!("{}/{}", dir_path.trim_end_matches('/'), entry.name)
        };
        match entry.kind {
            FileKind::Directory => {
                walk_omitted_stubs(volume, caller, entry.ino, &child_path, out)?;
            }
            FileKind::Regular if entry.name == HUMATA_OMITTED_STUB_FILENAME => {
                let attr = volume.getattr(caller, entry.ino)?;
                let raw = read_all_bytes(volume, caller, &attr)?;
                let stub = BuildCacheStub::from_json_bytes(&raw)?;
                out.push((dir_path.to_string(), stub));
            }
            FileKind::Regular | FileKind::Symlink => {}
        }
    }
    Ok(())
}

/// Formats a byte count into a compact human-readable string (`B`, `KiB`, `MiB`, `GiB`).
pub fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

    let b = bytes as f64;
    if b >= GIB {
        format!("{:.2} GiB", b / GIB)
    } else if b >= MIB {
        format!("{:.2} MiB", b / MIB)
    } else if b >= KIB {
        format!("{:.2} KiB", b / KIB)
    } else {
        format!("{bytes} B")
    }
}
