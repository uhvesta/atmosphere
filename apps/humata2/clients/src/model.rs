//! The domain values the fakes hand out: hosts, channels, sessions, Inbox
//! requests, diffs, board items, files, pull requests and specs.
//!
//! These stand in for `humata2_model` and the generated `api/proto`
//! messages until the real daemon is wired in (D50). Features read them
//! only through [`crate::Workspace`], so swapping the source changes
//! nothing above the clients.

use std::fmt;

use serde::{Deserialize, Serialize};

macro_rules! id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        pub struct $name(pub String);

        impl $name {
            /// The id written as `text`.
            #[must_use]
            pub fn new(text: impl Into<String>) -> Self {
                Self(text.into())
            }

            /// The id as text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

id!(
    /// A host a daemon runs on.
    MachineId
);
id!(
    /// A channel: a team of sessions around some repositories.
    ChannelId
);
id!(
    /// A session: one agent or shell in a terminal.
    SessionId
);
id!(
    /// A request in the Inbox.
    RequestId
);
id!(
    /// A milestone, task or step on a board.
    ItemId
);
id!(
    /// A file transfer between hosts.
    TransferId
);

/// How the app reaches a host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MachineKind {
    /// The Mac the app runs on.
    ThisMac,
    /// A host reached over SSH.
    Ssh {
        /// The SSH destination.
        address: String,
    },
    /// A host a provider made, such as a cloud VM.
    Provider {
        /// The provider's name.
        provider: String,
    },
}

/// A host and whether its daemon answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Machine {
    /// Its id.
    pub id: MachineId,
    /// The name people see.
    pub name: String,
    /// How it is reached.
    pub kind: MachineKind,
    /// Whether its daemon answers now.
    pub online: bool,
    /// One line about it: CPU, GPU, OS.
    pub detail: String,
    /// When it was last seen, as people read it.
    pub last_seen: String,
}

/// The agent a session runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Agent {
    /// Claude Code.
    Claude,
    /// Codex.
    Codex,
    /// GitHub Copilot.
    Copilot,
    /// OpenCode.
    OpenCode,
    /// A plain shell.
    Shell,
}

impl Agent {
    /// The program name used for its terminal tab.
    #[must_use]
    pub const fn command(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Copilot => "copilot",
            Self::OpenCode => "opencode",
            Self::Shell => "zsh",
        }
    }
    /// Every agent, in menu order.
    pub const ALL: [Self; 5] = [
        Self::Claude,
        Self::Codex,
        Self::Copilot,
        Self::OpenCode,
        Self::Shell,
    ];

    /// The name people see.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::Codex => "Codex",
            Self::Copilot => "Copilot",
            Self::OpenCode => "OpenCode",
            Self::Shell => "Shell",
        }
    }
}

/// What a session is doing, as the sidebar dot shows it (SE-2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Status {
    /// The agent is in a turn.
    Working,
    /// The agent waits on a request in the Inbox.
    NeedsYou,
    /// The turn ended.
    Idle,
    /// Output stopped a while ago, but the turn end wasn't reported.
    ProbablyIdle,
    /// The process ended.
    Exited {
        /// Its exit code.
        code: i32,
    },
    /// Its host doesn't answer.
    Offline,
}

impl Status {
    /// The words people see.
    #[must_use]
    pub fn label(self) -> String {
        match self {
            Self::Working => "Working".to_owned(),
            Self::NeedsYou => "Needs you".to_owned(),
            Self::Idle => "Idle".to_owned(),
            Self::ProbablyIdle => "Probably idle".to_owned(),
            Self::Exited { code } => format!("Exited ({code})"),
            Self::Offline => "Offline".to_owned(),
        }
    }

    /// Whether a person can type into it.
    #[must_use]
    pub const fn accepts_input(self) -> bool {
        !matches!(self, Self::Exited { .. } | Self::Offline)
    }
}

/// How a fork shares its parent's work (FK-1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ForkKind {
    /// Its own worktree and branch.
    Worktree,
    /// The parent's working tree.
    Shared,
    /// A copy of the conversation only.
    Conversation,
}

impl ForkKind {
    /// The name people see.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Worktree => "worktree fork",
            Self::Shared => "shared fork",
            Self::Conversation => "conversation fork",
        }
    }
}

/// Whether a session's turn has ended, so a queued message can go (SE-21–23).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Turn {
    /// In a turn: queued messages wait.
    Running,
    /// The turn ended: queued messages go.
    Ended,
    /// The agent can't tell us, so the app isn't sure.
    Uncertain,
}

/// A line the terminal shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    /// Who wrote it.
    pub from: Speaker,
    /// What it says.
    pub text: String,
}

/// Who wrote a terminal line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Speaker {
    /// The person.
    You,
    /// The agent.
    Agent,
    /// A tool the agent ran.
    Tool,
    /// The app, about the session.
    System,
}

/// A session and where it sits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    /// Its id.
    pub id: SessionId,
    /// The channel it belongs to.
    pub channel: ChannelId,
    /// The session it was forked from, if any.
    pub parent: Option<SessionId>,
    /// How it was forked, if it was.
    pub fork: Option<ForkKind>,
    /// The name people see.
    pub title: String,
    /// The agent it runs.
    pub agent: Agent,
    /// The host it runs on.
    pub machine: MachineId,
    /// What it is doing.
    pub status: Status,
    /// Whether its turn has ended.
    pub turn: Turn,
    /// Pinned to the top of the sidebar (NV-20).
    pub pinned: bool,
    /// Marked very important (NV-23).
    pub vip: bool,
    /// Lines it wrote that weren't read yet.
    pub unread: u32,
    /// The repository it works in.
    pub repo: String,
    /// Its branch.
    pub branch: String,
    /// When it started, as people read it.
    pub started: String,
    /// When it last wrote, as people read it.
    pub active: String,
    /// The most recent useful activity, shown on Home and in session tables.
    pub activity: String,
    /// What its terminal shows.
    pub transcript: Vec<Line>,
    /// Its extra terminal tabs (SE-8), after the agent's.
    pub shells: Vec<String>,
    /// Extra terminals running agents, by tab index; absent entries are shells.
    pub terminal_agents: std::collections::BTreeMap<usize, Agent>,
    /// Whether it was archived.
    pub archived: bool,
    /// What happened to it when its host came back (PER-26), until the
    /// person opens it.
    pub recovery: Option<Recovery>,
}

/// What happened to a session when its host came back (PER-13–16, PER-26).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Recovery {
    /// Its process kept running; the terminal was attached again.
    Reattached,
    /// Its process had ended; the agent was started again on its
    /// conversation.
    Resumed,
    /// It couldn't be brought back by itself.
    Recoverable {
        /// What the person has to do, in their words.
        needs: String,
    },
}

impl Recovery {
    /// The words people see.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Reattached => "Reattached".to_owned(),
            Self::Resumed => "Resumed".to_owned(),
            Self::Recoverable { needs } => format!("Recoverable: {needs}"),
        }
    }
}

/// A channel: repositories, hosts and sessions working together.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Channel {
    /// Its id.
    pub id: ChannelId,
    /// Its name, without the `#`.
    pub name: String,
    /// The repositories it works in.
    pub repos: Vec<String>,
    /// The hosts its sessions run on.
    pub machines: Vec<MachineId>,
    /// One line about it.
    pub purpose: String,
}

/// What an Inbox request asks for (IN-6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestKind {
    /// Run a command.
    Command,
    /// Change files.
    FileEdit,
    /// Reach the network.
    Network,
    /// Answer a question.
    Question,
    /// Merge a pull request.
    Merge,
    /// Move or delete something that can't be undone.
    Destroy,
}

impl RequestKind {
    /// The name people see.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Command => "Run a command",
            Self::FileEdit => "Edit files",
            Self::Network => "Network access",
            Self::Question => "Question",
            Self::Merge => "Merge",
            Self::Destroy => "Delete",
        }
    }
}

/// What the request would do, shown before it is answered (IN-8).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Preview {
    /// A command line.
    Command(String),
    /// A diff.
    Diff(Vec<DiffLine>),
    /// Words.
    Text(String),
}

/// How a request was answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    /// Allowed once.
    Approved,
    /// Allowed, and a rule made for next time.
    Always {
        /// The rule's words.
        rule: String,
    },
    /// Refused.
    Denied {
        /// What the person wrote back, if anything.
        reply: Option<String>,
    },
    /// A question's answer.
    Replied {
        /// The answer.
        text: String,
    },
}

/// A request a session sent to the Inbox.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    /// Its id.
    pub id: RequestId,
    /// The session asking.
    pub session: SessionId,
    /// The session's channel.
    pub channel: ChannelId,
    /// What it asks for.
    pub kind: RequestKind,
    /// One line.
    pub title: String,
    /// Why the agent asks.
    pub reason: String,
    /// What would happen.
    pub preview: Preview,
    /// What the agent was doing when it asked.
    pub doing: String,
    /// The rule "Always let …" would make.
    pub rule: String,
    /// Whether what it does can be undone.
    pub undoable: bool,
    /// When it was asked, as people read it.
    pub asked: String,
    /// How it was answered, once it is.
    pub answer: Option<Answer>,
}

/// Which side of a diff a line is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    /// Unchanged.
    Context,
    /// Added.
    Added,
    /// Removed.
    Removed,
    /// A hunk header.
    Hunk,
}

/// One line of a diff.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLine {
    /// Its side.
    pub kind: LineKind,
    /// Its number in the old file.
    pub old: Option<u32>,
    /// Its number in the new file.
    pub new: Option<u32>,
    /// Its text.
    pub text: String,
}

/// A changed file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileDiff {
    /// Its path in the repository.
    pub path: String,
    /// Lines added.
    pub added: u32,
    /// Lines removed.
    pub removed: u32,
    /// Its lines.
    pub lines: Vec<DiffLine>,
}

/// A review comment on a diff line (RV-4).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comment {
    /// The file.
    pub path: String,
    /// The line in the new file.
    pub line: u32,
    /// What it says.
    pub body: String,
    /// Whether it was sent to the agent.
    pub sent: bool,
}

/// A session's changes in one repository.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoDiff {
    /// The session.
    pub session: SessionId,
    /// The repository.
    pub repo: String,
    /// Its base, such as `main`.
    pub base: String,
    /// The changed files.
    pub files: Vec<FileDiff>,
    /// Comments on it.
    pub comments: Vec<Comment>,
}

/// Where a board item lives.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Scope {
    /// The global board.
    Global,
    /// A channel's board.
    Channel(ChannelId),
}

/// A board item's size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    /// A milestone.
    Milestone,
    /// A task.
    Task,
    /// A step of a task.
    Step,
}

/// A board column.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ItemStatus {
    /// Planned, before it is ready to start.
    Backlog,
    /// Not started.
    Todo,
    /// Being worked on.
    InProgress,
    /// Ready for a person to review.
    InReview,
    /// Waiting on something.
    Blocked,
    /// Finished.
    Done,
    /// Dropped.
    WontDo,
}

impl ItemStatus {
    /// Every column, in board order.
    pub const ALL: [Self; 7] = [
        Self::Backlog,
        Self::Todo,
        Self::InProgress,
        Self::InReview,
        Self::Blocked,
        Self::Done,
        Self::WontDo,
    ];

    /// The column's title.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Backlog => "Backlog",
            Self::Todo => "To do",
            Self::InProgress => "In progress",
            Self::InReview => "In review",
            Self::Blocked => "Blocked",
            Self::Done => "Done",
            Self::WontDo => "Won't do",
        }
    }
}

/// A milestone, task or step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// Its id.
    pub id: ItemId,
    /// Its board.
    pub scope: Scope,
    /// Its size.
    pub kind: ItemKind,
    /// Its title.
    pub title: String,
    /// Its column.
    pub status: ItemStatus,
    /// Items that must be done first.
    pub depends_on: Vec<ItemId>,
    /// The session working on it.
    pub session: Option<SessionId>,
    /// Its parent milestone or task.
    pub parent: Option<ItemId>,
}

/// A path inside one repository clone belonging to a session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionFile {
    /// The repository, such as `acme/api`.
    pub repo: String,
    /// The path relative to that clone's root.
    pub path: String,
    /// Whether the path is a directory.
    pub folder: bool,
}

/// A file on a host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    /// Its host.
    pub machine: MachineId,
    /// Its path.
    pub path: String,
    /// Its size, as people read it.
    pub size: String,
    /// Whether it is a folder.
    pub folder: bool,
}

/// How far a transfer got.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferState {
    /// Copying, with percent done.
    Copying(u8),
    /// Finished.
    Done,
    /// Failed.
    Failed,
}

/// A file copied between hosts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transfer {
    /// Its id.
    pub id: TransferId,
    /// The file.
    pub path: String,
    /// From.
    pub from: MachineId,
    /// To.
    pub to: MachineId,
    /// How far it got.
    pub state: TransferState,
}

/// Something deleted that can still be restored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trashed {
    /// What it was.
    pub what: String,
    /// When it goes for good, as people read it.
    pub expires: String,
}

/// A pull request a session opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullRequest {
    /// Its number.
    pub number: u32,
    /// Its title.
    pub title: String,
    /// Its repository.
    pub repo: String,
    /// The session that opened it.
    pub session: Option<SessionId>,
    /// Its checks, as people read them.
    pub checks: String,
}

/// A spec a channel works from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spec {
    /// Its title.
    pub title: String,
    /// Its channel.
    pub channel: ChannelId,
    /// Its text.
    pub body: String,
}

/// One journal line (JR-1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalEntry {
    /// When, as people read it.
    pub at: String,
    /// The session it is about.
    pub session: Option<SessionId>,
    /// What happened.
    pub text: String,
}

/// What happened on a channel's Activity feed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivityKind {
    Inbox,
    Fork,
    Turn,
    Item,
    Offline,
    PullRequest,
}

/// One event on a channel's Activity feed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Activity {
    pub channel: ChannelId,
    pub session: Option<SessionId>,
    pub kind: ActivityKind,
    pub text: String,
    pub by: String,
    pub at: String,
}

/// A prepared view an agent offered (VW-1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedView {
    /// Its title.
    pub title: String,
    /// The session that offered it.
    pub session: SessionId,
    /// What it shows.
    pub body: String,
    /// Whether the person opted in to it.
    pub accepted: bool,
}

/// A skill the `/` menu offers in the composer (SE-24).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skill {
    /// The command, without the `/`.
    pub name: String,
    /// What it does.
    pub summary: String,
}
