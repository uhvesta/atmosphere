//! The holders running on a machine (D784): every terminal of ours, those no
//! session names included, and the sessions of that machine that were
//! stopped and can be resumed.

use crate::model::{Agent, ChannelId, MachineId, SessionId};

/// One holder running on a machine, or one stopped session of it, as
/// [`super::Daemon::running_holders`] lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunningHolder {
    /// The tab to attach it by, as the daemon names it; `None` for a
    /// stopped session, which has no holder.
    pub tab: Option<String>,
    /// The session that names it; `None` for one no session names (an
    /// orphan, listed under "Not in a channel").
    pub session: Option<SessionId>,
    /// The session's title; empty for an orphan.
    pub title: String,
    /// The session's channel; `None` for an orphan and for a Desk session.
    pub channel: Option<ChannelId>,
    /// It is a Desk session.
    pub desk: bool,
    /// The program it runs; anything that isn't a known agent is a shell.
    pub agent: Agent,
    /// The holder's process; `None` for a stopped session.
    pub pid: Option<u32>,
    /// The folder it was started in.
    pub folder: String,
    /// When the holder started, in Unix milliseconds; `None` for a stopped
    /// session.
    pub started_unix_ms: Option<i64>,
    /// A client has it open in a tab now.
    pub attached: bool,
    /// The machine it runs on.
    pub machine: MachineId,
    /// A stopped session (D784 "Stop"): nothing runs, and Resume revives it.
    pub stopped: bool,
}

impl RunningHolder {
    /// Whether no session names it.
    #[must_use]
    pub const fn is_orphan(&self) -> bool {
        self.session.is_none()
    }
}

/// Which holder [`super::Daemon::end_running_holder`] ends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HolderTab {
    /// A tab as [`super::Daemon::running_holders`] listed it.
    Listed(String),
    /// Terminal tab `index` of `session`, as the session's page counts its
    /// tabs: 0 is the agent's.
    Session {
        /// The session.
        session: SessionId,
        /// Its tab.
        index: usize,
    },
}

/// The agent a daemon's agent kind names; anything else runs as a shell.
#[must_use]
pub fn agent_of_kind(kind: &str) -> Agent {
    match kind {
        "claude" => Agent::Claude,
        "codex" => Agent::Codex,
        "copilot" => Agent::Copilot,
        "opencode" => Agent::OpenCode,
        _ => Agent::Shell,
    }
}
