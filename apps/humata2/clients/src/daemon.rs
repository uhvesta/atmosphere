//! The daemon client: everything the app asks of the daemons, and the
//! events they push. One trait; [`FakeDaemon`] is the only implementation
//! until the real one is wired in over `api/proto`.

use tca::BoxFuture;
use tokio::sync::mpsc;

use crate::model::{
    AccessRule, Agent, Answer, Capability, ChannelId, Comment, ForkKind, GrantId, ItemId, ItemStatus, Line, MachineId, Recovery,
    RequestId, RuleSetting, Scope, Session, SessionId, Transfer, TransferId,
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
    /// A channel's access rules changed.
    Rules {
        /// The channel.
        channel: ChannelId,
        /// Its rules, in IN-17 order.
        rules: Vec<AccessRule>,
    },
    /// A host went online or offline.
    Machine(crate::model::Machine),
    /// A transfer moved on.
    Transfer(Transfer),
    /// Events were dropped because the subscriber fell behind: reload.
    Resync,
}

/// The operations a fake can be told to fail.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Operation {
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
    /// [`Daemon::send_comments`].
    SendComments,
    /// [`Daemon::archive`].
    Archive,
    /// [`Daemon::resume`].
    Resume,
    /// [`Daemon::set_rule`].
    SetRule,
    /// [`Daemon::remove_grant`].
    RemoveGrant,
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
    /// A fake was told to fail this call.
    #[error("{operation:?} failed: {message}. Try again; a test made this fail on purpose")]
    Scripted {
        /// The call.
        operation: Operation,
        /// What the test said.
        message: String,
    },
}

/// What the app asks of the daemons.
pub trait Daemon: Send + Sync + 'static {
    /// Reads a text file in this session's own repository clone.
    fn read_file(&self, session: &SessionId, repo: &str, path: &str) -> Reply<String>;
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
    /// Forks a session.
    fn fork(&self, session: &SessionId, kind: ForkKind, title: &str) -> Reply<SessionId>;
    /// Archives a session; its forks keep running.
    fn archive(&self, session: &SessionId) -> Reply<()>;
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
    /// Moves a board item to a column.
    fn move_item(&self, item: &ItemId, status: ItemStatus) -> Reply<()>;
    /// Adds a board item.
    fn create_item(&self, scope: Scope, title: &str) -> Reply<ItemId>;
    /// Makes `item` wait for `on`; refused when that would make a cycle.
    fn add_dependency(&self, item: &ItemId, on: &ItemId) -> Reply<()>;
    /// Sends review comments to the session's agent.
    fn send_comments(&self, session: &SessionId, comments: Vec<Comment>) -> Reply<()>;
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
