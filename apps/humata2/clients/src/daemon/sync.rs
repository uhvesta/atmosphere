//! Settings › Sync (FUSEFS): which parts of the host's agent config every
//! session sees, per machine where it differs, and whether the cross-machine
//! view is mounted. Nothing here writes a user's real agent dotfiles; each
//! session gets its own composed config home.

use std::collections::BTreeMap;

/// How a host's skills meet a session's own copies.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SyncSkillsMode {
    /// The host's skill replaces a session's skill of the same name.
    #[default]
    HostWins,
    /// A session sees only the host's skills.
    HostOnly,
}

/// The Sync section as the daemon holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncSettings {
    /// The master switch. Off, no category syncs; the categories keep their
    /// values for when it is turned on again.
    pub sync_enabled: bool,
    /// Skills sync.
    pub skills: bool,
    /// Instruction files (CLAUDE.md and the like) sync.
    pub instructions: bool,
    /// MCP server definitions sync.
    pub mcp: bool,
    /// Agent settings files sync.
    pub settings: bool,
    /// How synced skills meet a session's own.
    pub skills_mode: SyncSkillsMode,
    /// Departures per machine, keyed by machine id; a machine without an
    /// entry follows the global values.
    pub machines: BTreeMap<String, MachineSyncSettings>,
}

impl Default for SyncSettings {
    fn default() -> Self {
        Self {
            sync_enabled: true,
            skills: true,
            instructions: true,
            mcp: false,
            settings: false,
            skills_mode: SyncSkillsMode::HostWins,
            machines: BTreeMap::new(),
        }
    }
}

/// One machine's departures from the global [`SyncSettings`]; a field left
/// `None` follows the global value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MachineSyncSettings {
    /// This machine's own master switch.
    pub enabled: Option<bool>,
    /// Skills sync on this machine.
    pub skills: Option<bool>,
    /// Instruction files sync on this machine.
    pub instructions: Option<bool>,
    /// MCP definitions sync on this machine.
    pub mcp: Option<bool>,
    /// Agent settings files sync on this machine.
    pub settings: Option<bool>,
    /// How synced skills meet a session's own on this machine.
    pub skills_mode: Option<SyncSkillsMode>,
}

/// What the daemon holds for Settings › Sync.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncSnapshot {
    /// The Sync section.
    pub sync: SyncSettings,
    /// Whether the daemon mounts the cross-machine view. Local session
    /// folders stay on native disk either way. Off until turned on.
    pub fusefs_enabled: bool,
    /// What the mount is doing now (D908). Output only: writes ignore it.
    pub fusefs_status: FuseMountStatus,
}

/// What the daemon's FUSE mount is doing, as Settings › Sync says it (D908).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum FuseMountStatus {
    /// The mount isn't wanted, or the daemon hasn't said.
    #[default]
    Off,
    /// The mount is being set up.
    Starting,
    /// The tree is mounted at `path` (the daemon's own spelling of it).
    Mounted {
        /// Where the tree is mounted.
        path: String,
    },
    /// The mount is wanted but can't run; `reason` is the daemon's sentence.
    CannotSetUp {
        /// What failed and how to fix it, shown as sent.
        reason: String,
    },
}

/// What a write changes; `None` leaves that part as it is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncChange {
    /// Replaces the whole Sync section.
    pub sync: Option<SyncSettings>,
    /// Turns the mount on or off.
    pub fusefs_enabled: Option<bool>,
}
