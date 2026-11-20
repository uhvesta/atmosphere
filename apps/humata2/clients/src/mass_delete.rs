//! MASSDELETE: deleting many sessions in one go. A plan says what a delete
//! would end, what it leaves alone and what keeps running; the delete names
//! the plan's token, and a delete whose sessions changed since the plan
//! deletes nothing and answers the new plan (NV-27).

use crate::daemon::UnpushedWork;
use crate::model::{ChannelId, MachineId, SessionId};

/// Which sessions a bulk delete covers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeleteScope {
    /// These sessions, as picked.
    Picked(Vec<SessionId>),
    /// Every session in a channel, live and archived.
    Channel(ChannelId),
    /// Every session on a machine.
    Machine(MachineId),
    /// Every channel session, live and archived. Desk sessions are not in
    /// it: the Desk's Clean up deletes them.
    All,
}

/// Where a terminal stands when its session ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndingState {
    /// The agent is in a turn.
    Working,
    /// The agent waits on the person.
    WaitingForYou,
    /// Nothing is running in it.
    Idle,
}

/// One agent or shell terminal a delete ends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalEnding {
    /// Its name, such as "claude" or "zsh".
    pub name: String,
    /// Its state when the delete ends it.
    pub state: EndingState,
}

/// A session a delete would delete.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionToDelete {
    /// Its id.
    pub session: SessionId,
    /// The name people see.
    pub title: String,
    /// The channel it belongs to.
    pub channel: ChannelId,
    /// The host it runs on.
    pub machine: MachineId,
    /// Whether it is archived already.
    pub archived: bool,
    /// Work that exists only in its clones; deleting it deletes this too.
    pub unpushed: Vec<UnpushedWork>,
    /// Agent and shell terminals the delete ends.
    pub running_terminals: u32,
    /// Set on a context-only fork pulled in because it works in this
    /// parent's folder.
    pub goes_with: Option<SessionId>,
    /// Each terminal the delete ends, named with its state.
    pub terminals: Vec<TerminalEnding>,
    /// A live session whose agent has exited: nothing running, not archived.
    pub exited: bool,
}

/// A session's work on a machine that a plan reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionWork {
    /// The session.
    pub session: SessionId,
    /// The name people see.
    pub title: String,
    /// Files with uncommitted changes.
    pub dirty_files: u32,
    /// Commits not yet on the remote.
    pub unpushed_commits: u32,
}

/// Sessions on a machine that can't be reached: a delete leaves them alone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeftAlone {
    /// The machine.
    pub machine: MachineId,
    /// Its sessions, from the host's last report.
    pub sessions: Vec<SessionWork>,
}

/// What one bulk delete would do, known before it happens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeleteSessionsPlan {
    /// The sessions the delete deletes.
    pub sessions: Vec<SessionToDelete>,
    /// The sessions on machines that can't be reached.
    pub left_alone: Vec<LeftAlone>,
    /// Code forks of a deleted session that keep their folder and run on.
    pub forks_staying: Vec<SessionWork>,
    /// Desk sessions the scope matched and the delete leaves out.
    pub desk_sessions_left_out: u32,
    /// What the delete names to act on this plan.
    pub token: String,
    /// Whether the delete needs the person to type the number of sessions.
    pub needs_count: bool,
}

/// What a bulk delete answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeleteSessionsOutcome {
    /// The sessions were deleted; their folders and records went to trash.
    Deleted {
        /// How many sessions were deleted.
        sessions: u32,
    },
    /// The sessions changed since the plan, so nothing was deleted. The new
    /// plan is here to review and delete again.
    Changed {
        /// The plan for the sessions as they are now.
        plan: DeleteSessionsPlan,
        /// What changed, in words.
        changes: Vec<String>,
    },
}
