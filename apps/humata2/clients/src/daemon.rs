//! The daemon client: everything the app asks of the daemons, and the
//! events they push. One trait; [`FakeDaemon`] is the only implementation
//! until the real one is wired in over `api/proto`.

use tca::BoxFuture;
use tokio::sync::mpsc;

use crate::model::{
    AccessRule, Agent, Answer, Archive, Capability, ChannelId, DiffBase, ForkKind, GrantId, Item,
    ItemDetails, ItemId, ItemStatus, Line, MachineId, PushChoice, PushPlan, Recovery, RepoDiff,
    RequestId, RuleSetting, Scope, Session, SessionId, SetupId, Transfer, TransferId,
};
use crate::workspace::Workspace;

/// How many events a subscriber may fall behind before it is told to
/// reload instead (TR-8: queues are bounded).
pub const EVENT_QUEUE: usize = 256;

/// A daemon reply.
pub type Reply<T> = BoxFuture<Result<T, DaemonError>>;

/// The events a subscription delivers, in order.
pub type Events = mpsc::Receiver<Result<Event, DaemonError>>;

/// Something changed on a daemon.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A session changed: its status, turn, title or flags.
    Session(Session),
    /// A session's terminal wrote a line.
    Output {
        /// The session.
        session: SessionId,
        /// The terminal tab: 0 is the agent's.
        tab: usize,
        /// The line.
        line: Line,
    },
    /// Bytes from the PTY, with their absolute offset for replay deduplication.
    TerminalData {
        session: SessionId,
        tab: usize,
        offset: usize,
        bytes: Vec<u8>,
    },
    /// A request was added or answered.
    Request(crate::model::Request),
    /// A request no one can answer any more: the agent stopped waiting
    /// (HOOKS, D191) or withdrew it.
    RequestGone(RequestId),
    /// A channel's access rules changed.
    Rules {
        /// The channel.
        channel: ChannelId,
        /// Its rules, in IN-17 order.
        rules: Vec<AccessRule>,
    },
    /// A host went online or offline.
    Machine(crate::model::Machine),
    /// A host's restart report is ready (frame 47).
    Restart(crate::model::Restart),
    /// A channel setup moved on; `Done`/`Failed` steps included (frame 45).
    ChannelSetup(crate::model::ChannelSetup),
    /// A transfer moved on.
    Transfer(Transfer),
    /// A board item was made or changed (BD-1).
    Item {
        /// Its card.
        item: Item,
        /// What its drawer shows.
        details: ItemDetails,
    },
    /// A board item was deleted.
    ItemGone(ItemId),
    /// Events were dropped because the subscriber fell behind: reload.
    Resync,
}

/// The operations a fake can be told to fail.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Operation {
    /// A [`crate::desk::DeskClient`] call.
    Desk(crate::desk::Operation),
    /// [`Daemon::load`].
    Load,
    /// [`Daemon::send`].
    Send,
    /// [`Daemon::answer`].
    Answer,
    /// [`Daemon::reconnect`].
    Reconnect,
    /// [`Daemon::fork`].
    Fork,
    /// [`Daemon::create_session`].
    CreateSession,
    /// [`Daemon::create_channel`].
    CreateChannel,
    /// [`Daemon::move_item`].
    MoveItem,
    /// [`Daemon::archive`].
    Archive,
    /// [`Daemon::move_session`].
    MoveSession,
    /// [`Daemon::resume`].
    Resume,
    /// [`Daemon::start_fresh`].
    StartFresh,
    /// [`Daemon::cancel_setup`].
    CancelSetup,
    /// [`Daemon::retry_setup`].
    RetrySetup,
    /// [`Daemon::set_rule`].
    SetRule,
    /// [`Daemon::remove_grant`].
    RemoveGrant,
    /// [`Daemon::diffs`].
    Diffs,
    /// [`Daemon::mark_reviewed`].
    MarkReviewed,
    /// [`Daemon::plan_push`].
    PlanPush,
    /// [`Daemon::push`].
    Push,
    /// [`Daemon::set_asks_in_inbox`].
    SetAsksInInbox,
}

/// Why a daemon call failed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DaemonError {
    /// The host doesn't answer.
    #[error(
        "{machine} doesn't answer, so this can't be done there now. Reconnect it from Hosts; its sessions keep running and nothing is lost"
    )]
    Offline {
        /// The host.
        machine: MachineId,
    },
    /// What was asked about is gone.
    #[error(
        "{what} no longer exists. It may have been archived or deleted from another window; reload to see what is there now"
    )]
    NotFound {
        /// What was asked about.
        what: String,
    },
    /// The daemon refused it.
    #[error("{what} was refused: {reason}")]
    Refused {
        /// What was asked.
        what: String,
        /// Why, and what to do instead.
        reason: String,
    },
    /// This host's daemon doesn't do this yet (D134): not a refusal, so the
    /// app can hide or disable the action instead of reporting a failure.
    #[error(
        "{what} isn't in this release of Humata yet; it comes in a later one. Nothing was changed, so update Humata when a new release is out"
    )]
    NotServedYet {
        /// What was asked.
        what: String,
    },
    /// A fake was told to fail this call.
    #[error("{operation:?} failed: {}. Try again; a test made this fail on purpose", .message.trim_end_matches('.'))]
    Scripted {
        /// The call.
        operation: Operation,
        /// What the test said.
        message: String,
    },
}

/// What the Fork dialog sends (FK-1 to FK-5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForkOptions {
    /// How the fork shares the parent's work.
    pub kind: ForkKind,
    /// Its title.
    pub title: String,
    /// The turn to fork before; `None` forks from now.
    pub before_turn: Option<u32>,
    /// The agent the fork runs; `None` keeps the parent's.
    pub agent: Option<Agent>,
    /// Where the fork runs; `None` keeps the parent's machine.
    pub machine: Option<MachineId>,
    /// The branch the fork's clones are made on.
    pub branch: String,
}

/// One turn of a conversation a fork may start before.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PastTurn {
    /// Its number, counting from 1.
    pub index: u32,
    /// The first line of what was asked.
    pub text: String,
}

/// Work in one repository that archiving would delete.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnpushedWork {
    /// The repository, as the session names it.
    pub repo: String,
    /// What is unpushed there, in a sentence.
    pub what: String,
}

/// What archiving a session does, known before it happens.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ArchivePlan {
    /// How many forks stay running and move up the tree.
    pub forks_keep_running: u32,
    /// Work that exists only in this session's clones; empty when archiving
    /// loses nothing.
    pub unpushed: Vec<UnpushedWork>,
    /// The disk archiving frees, as people read it, when known.
    pub frees: Option<String>,
}

/// What the app asks of the daemons.
pub trait Daemon: Send + Sync + 'static {
    /// Reads a text file in this session's own repository clone.
    fn read_file(&self, session: &SessionId, repo: &str, path: &str) -> Reply<String>;
    /// The session's changes in each of its repositories against `base`
    /// (RV-1), read from its clones now.
    fn diffs(&self, session: &SessionId, base: &DiffBase) -> Reply<Vec<RepoDiff>> {
        let what = format!("The changes of {session} against {base:?}");
        Box::pin(async move { Err(DaemonError::NotServedYet { what }) })
    }
    /// What pushing each of the session's repositories would do (RV-2).
    fn plan_push(&self, session: &SessionId) -> Reply<PushPlan> {
        let what = format!("Pushing {session}");
        Box::pin(async move { Err(DaemonError::NotServedYet { what }) })
    }
    /// Commits, pushes and opens pull requests for the chosen repositories
    /// of a plan, spending its `token`. Returns the pull request number the
    /// daemon opened or found, if any.
    fn push(
        &self,
        session: &SessionId,
        token: &str,
        choices: Vec<PushChoice>,
    ) -> Reply<Option<u32>> {
        let what = format!("Pushing {session}");
        Box::pin(async move { Err(DaemonError::NotServedYet { what }) })
    }
    /// Remembers the session's changes as reviewed, so "Since I last
    /// reviewed" starts from here (RV-6).
    fn mark_reviewed(&self, session: &SessionId) -> Reply<()> {
        let what = format!("Mark reviewed for {session}");
        Box::pin(async move { Err(DaemonError::NotServedYet { what }) })
    }
    /// Everything there is, merged across hosts.
    fn load(&self) -> Reply<Workspace>;
    /// Follows what changes, from now on.
    fn events(&self) -> Reply<Events>;
    /// Types `text` into the session's agent and presses Enter.
    fn send(&self, session: &SessionId, text: &str) -> Reply<()>;
    /// Answers an Inbox request.
    fn answer(&self, request: &RequestId, answer: Answer) -> Reply<()>;
    /// Connects to a host again, and says what happened to each of its
    /// sessions (PER-26).
    fn reconnect(&self, machine: &MachineId) -> Reply<Vec<(SessionId, Recovery)>>;
    /// Starts an exited session's agent again on its conversation.
    fn resume(&self, session: &SessionId) -> Reply<()>;
    /// Starts a session whose resume failed on a new conversation, seeded
    /// with a summary of the old one (PER-15, D143).
    fn start_fresh(&self, session: &SessionId) -> Reply<()>;
    /// Forks a session as the dialog set it up (FK-1 to FK-5).
    fn fork(&self, session: &SessionId, options: ForkOptions) -> Reply<SessionId>;
    /// The turns a fork may start before, oldest first (FK-3).
    fn list_turns(&self, session: &SessionId) -> Reply<Vec<PastTurn>> {
        let what = format!("The turns of {session}");
        Box::pin(async move { Err(DaemonError::NotServedYet { what }) })
    }
    /// Moves a session to another machine (MV-1 to MV-6).
    fn move_session(&self, session: &SessionId, to: &MachineId) -> Reply<()> {
        let what = format!("Moving {session} to {to}");
        Box::pin(async move { Err(DaemonError::NotServedYet { what }) })
    }
    /// What archiving a session would lose or leave running (MV-8, MV-9),
    /// read before it is archived.
    fn plan_archive(&self, session: &SessionId) -> Reply<ArchivePlan> {
        let what = format!("What archiving {session} would lose");
        Box::pin(async move { Err(DaemonError::NotServedYet { what }) })
    }
    /// Archives a session; its forks keep running. With `push_first` its
    /// unpushed work is committed, pushed and opened as pull requests first;
    /// without it, that work is deleted with the session.
    fn archive(&self, session: &SessionId, push_first: bool) -> Reply<Archive>;
    /// Adds a plain SSH destination to the fake workspace.
    fn add_machine(&self, address: &str) -> Reply<MachineId>;
    /// Restores a session archived by this client, including its tree and pin.
    fn restore_session(&self, session: &SessionId) -> Reply<()>;
    /// Pins or unpins a session.
    fn set_pinned(&self, session: &SessionId, pinned: bool) -> Reply<()>;
    /// Marks a session very important, or not.
    fn set_vip(&self, session: &SessionId, vip: bool) -> Reply<()>;
    /// Clears a session's unread mark and its recovery note.
    fn mark_read(&self, session: &SessionId) -> Reply<()>;
    /// Starts a session.
    fn create_session(
        &self,
        channel: &ChannelId,
        agent: Agent,
        machine: &MachineId,
        title: &str,
        options: crate::model::SessionOptions,
    ) -> Reply<SessionId>;
    /// Makes a channel.
    fn create_channel(
        &self,
        name: &str,
        repos: Vec<String>,
        machines: Vec<MachineId>,
    ) -> Reply<ChannelId>;
    /// Stops a channel setup; the folders it made are moved to the trash.
    fn cancel_setup(&self, setup: &SetupId) -> Reply<()>;
    /// Runs a failed channel setup's failed steps again; the channel, once done.
    fn retry_setup(&self, setup: &SetupId) -> Reply<ChannelId>;
    /// Moves a board item to a column.
    fn move_item(&self, item: &ItemId, status: ItemStatus) -> Reply<()>;
    /// Adds a board item.
    fn create_item(&self, scope: Scope, title: &str) -> Reply<ItemId>;
    /// Makes `item` wait for `on`; refused when that would make a cycle.
    fn add_dependency(&self, item: &ItemId, on: &ItemId) -> Reply<()>;
    /// Stops `item` waiting for `on` (BD-14).
    fn remove_dependency(&self, item: &ItemId, on: &ItemId) -> Reply<()>;
    /// Adds `session` to the sessions working on `item`, or removes it
    /// (BD-16).
    fn set_item_session(&self, item: &ItemId, session: &SessionId, working: bool) -> Reply<()>;
    /// An item's history as the drawer words it, newest first (BD-3).
    fn item_history(&self, item: &ItemId) -> Reply<Vec<String>>;
    /// Retries a failed transfer.
    fn retry_transfer(&self, transfer: &TransferId) -> Reply<()>;
    /// Sets what a channel's rule does; refused for a rule with no setting
    /// (IN-16).
    fn set_rule(
        &self,
        channel: &ChannelId,
        capability: &Capability,
        setting: RuleSetting,
    ) -> Reply<()>;
    /// Removes a scoped "Always allow" grant (IN-19).
    fn remove_grant(&self, grant: &GrantId) -> Reply<()>;
    /// Whether agents' permission asks wait in the Inbox (D199); false: the
    /// agent's own prompt asks.
    fn asks_in_inbox(&self) -> Reply<bool>;
    /// Sets it; gives what the daemon holds after.
    fn set_asks_in_inbox(&self, inbox: bool) -> Reply<bool>;
    /// Whether Codex's hooks still need trusting in Codex's /hooks (D202).
    fn codex_needs_trust(&self) -> Reply<bool>;
}

/// The terminals of sessions: typing into tabs other than the agent's.
/// Their output arrives as [`Event::Output`].
pub trait Terminals: Send + Sync + 'static {
    /// Starts a shell, or attaches the one terminal already running this agent.
    fn open(&self, session: &SessionId, agent: Agent) -> Reply<(usize, Session)>;

    /// Types `text` and Enter into tab `tab` of `session`.
    fn write(&self, session: &SessionId, tab: usize, text: &str) -> Reply<()>;

    /// Writes encoded terminal input without adding a newline.
    fn input(&self, session: &SessionId, tab: usize, bytes: Vec<u8>) -> Reply<()>;

    /// Reports the view's cell dimensions to the running program.
    fn resize(&self, session: &SessionId, tab: usize, columns: u16, rows: u16) -> Reply<()>;
}

mod fake;

pub use fake::FakeDaemon;
