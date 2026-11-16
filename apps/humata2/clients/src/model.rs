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
id!(
    /// A kind of action an agent may take through Humata, which an access
    /// rule governs (IN-17).
    Capability
);
id!(
    /// A scoped "Always allow" grant under an access rule (IN-19).
    GrantId
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
    /// While the app's link to it is lost: the clock time it was lost,
    /// such as "14:02". `None` while connected or when nobody knows.
    pub lost_at: Option<String>,
}

/// Health information reported by a host, never inferred from the UI.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MachineHealth {
    pub system: String,
    pub daemon: String,
    pub uptime: String,
    pub tools: Vec<String>,
    pub disk_used_percent: u8,
    pub disk_free_gb: u64,
    pub latency_ms: Option<u32>,
    pub stopped: bool,
}

/// Why an agent couldn't be resumed after a restart (PER-15, frame 46).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResumeFailure {
    /// Its conversation file is gone.
    ConversationMissing,
    /// The agent started a different conversation than the one pinned.
    OtherConversation,
    /// The agent exited while resuming, with this code.
    AgentExited(i32),
    /// The agent isn't installed on the host any more.
    AgentNotInstalled,
    /// The session's folder is gone.
    FolderMissing,
}

/// What a host found when it restarted (PER-29, frame 47).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Restart {
    /// The host.
    pub machine: MachineId,
    /// When its daemon started, in Unix ms.
    pub at_unix_ms: i64,
    /// The restart report's folder, if it was written.
    pub report: Option<String>,
    /// Sessions started again on their conversations.
    pub resumed: Vec<SessionId>,
    /// Each agent or shell not brought back.
    pub not_brought_back: Vec<NotBroughtBack>,
}

/// An agent or shell a restart didn't bring back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotBroughtBack {
    /// The session, for an agent.
    pub session: Option<SessionId>,
    /// The tab, for a shell.
    pub tab: Option<String>,
    /// What went wrong and what to do, as one line.
    pub problem: String,
}

/// A channel being set up: its folders and clones per host (frame 45).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelSetup {
    /// The setup's id (the daemon's operation id).
    pub id: SetupId,
    /// The channel's name.
    pub name: String,
    /// One step per host and repository, in the daemon's order.
    pub steps: Vec<SetupStep>,
    /// Whether Cancel is offered.
    pub cancellable: bool,
}

/// The id of a channel setup.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SetupId(pub String);

/// One step of a channel setup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetupStep {
    /// The host it runs on.
    pub machine: MachineId,
    /// The repository it clones, or `None` for "Folders made".
    pub repo: Option<String>,
    /// How far it got.
    pub state: SetupState,
}

/// How far a setup step got.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SetupState {
    /// Not started.
    Waiting,
    /// Running: percent done (0-100) and what it is doing, e.g. "Receiving objects".
    Running { percent: u8, detail: String },
    /// Done.
    Done,
    /// Failed, with the reason as one line.
    Failed { reason: String },
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

/// The explicit choices used when starting a session.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionOptions {
    pub model: Option<String>,
    pub repos: Vec<String>,
    pub first_message: String,
    pub item: Option<ItemId>,
    pub branch: String,
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
    /// Own code copies with a fresh conversation (legacy serialized name).
    Shared,
    /// Conversation context with read-only access to the parent clones.
    Conversation,
}

impl ForkKind {
    /// The name people see.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Worktree => "Code + context",
            Self::Shared => "Code only",
            Self::Conversation => "Context only",
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
    /// When the person last used it (typed into it, opened it or answered it;
    /// SE-26), as people read it. Empty when not known yet.
    pub last_used: String,
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
    /// Why its resume failed, while it is failed (frame 46).
    pub resume_failed: Option<ResumeFailure>,
    /// When a host restart interrupted it, in Unix ms; `None` if none has.
    pub interrupted_unix_ms: Option<i64>,
    /// The model its agent runs, such as "Opus", when the agent says.
    pub model: Option<String>,
    /// How it survives a restart, as its host reports it: who holds the
    /// terminal, how the agent comes back, what scrollback is kept.
    pub restart_detail: Option<String>,
    /// What its agent may do, one paragraph each, as the daemon's access
    /// rules give it; empty when not reported.
    pub permissions: Vec<String>,
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
    /// The rule "Always allow …" would make.
    pub rule: String,
    /// Whether what it does can be undone.
    pub undoable: bool,
    /// When it was asked, as people read it.
    pub asked: String,
    /// How it was answered, once it is.
    pub answer: Option<Answer>,
    /// The call, as a dotted operation id such as `board.item.move` (TP-16).
    pub operation: String,
    /// The agent's last few steps, the pending call last.
    pub steps: Vec<String>,
    /// Why this needs the user and not the agent's own access.
    pub why_you: String,
    /// The access rule that sent it to the user, if one did.
    pub asked_by: Option<AskedBy>,
    /// Set only when it can't be undone or undoing costs something (IN-9.7).
    pub undo_note: Option<String>,
    /// The answers a question offers, at most four (IN-25).
    pub choices: Vec<String>,
}

/// The access rule that turned an agent's call into an Inbox request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AskedBy {
    /// The kind of action the rule governs.
    pub capability: Capability,
    /// The rule's row label, as the Agent access tab shows it.
    pub label: String,
    /// What the rule was set to.
    pub setting: RuleSetting,
}

/// What an access rule does with an agent's call (IN-17).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleSetting {
    /// Let it through.
    Allow,
    /// Put it in the Inbox.
    Ask,
    /// Deny it at once, naming the rule (IN-18).
    Never,
}

/// A scoped "Always allow" the user granted from a request (IN-19).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grant {
    /// Its id.
    pub id: GrantId,
    /// The session whose request it was granted for.
    pub session: SessionId,
    /// Where it applies, in words such as "only under ledger/docs/".
    pub scope: String,
    /// When it was granted, as people read it.
    pub granted: String,
}

/// One row of a channel's Agent access tab (IN-17).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessRule {
    /// The kind of action it governs.
    pub capability: Capability,
    /// The row's label.
    pub label: String,
    /// Its setting; `None` is "Always, no setting" (IN-16).
    pub setting: Option<RuleSetting>,
    /// The scoped grants listed under it.
    pub grants: Vec<Grant>,
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
    /// The comparison shown when this new-side comment was written.
    #[serde(default)]
    pub context: Option<CommentContext>,
}

/// Source context retained with a comment on the new side of a diff.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommentContext {
    /// The repository whose comparison was shown.
    pub repo: String,
    /// The base selected when the comment was written.
    pub base: String,
    /// The shown source line, when the comparison contains it.
    pub quoted: Option<String>,
}

impl Comment {
    /// Comments delivered together to the session's main agent (SM-9).
    #[must_use]
    pub fn message(comments: &[Self]) -> String {
        comments
            .iter()
            .map(|comment| {
                if let Some(context) = &comment.context {
                    let quote = context
                        .quoted
                        .as_ref()
                        .map(|line| format!("> {line}\n"))
                        .unwrap_or_default();
                    format!(
                        "[comment on {}/{}:{} · new · base {}]\n{quote}{}",
                        context.repo, comment.path, comment.line, context.base, comment.body
                    )
                } else {
                    format!(
                        "[comment on {}:{}]\n{}",
                        comment.path, comment.line, comment.body
                    )
                }
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }
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

/// Extra board details loaded alongside the compact card rows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ItemDetails {
    pub description: String,
    pub request_summary: String,
    pub session_label: Option<String>,
    pub history: Vec<String>,
    pub created_by: Option<SessionId>,
    pub sessions: Vec<SessionId>,
    pub request: Option<RequestId>,
    pub requested_status: Option<ItemStatus>,
    /// Day in the dashboard's fourteen-day reporting window, oldest first.
    pub done_day: Option<u8>,
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
    /// Its one-line summary, for the Journal.
    pub note: String,
    /// When it was prepared, as people read it.
    pub prepared: String,
}

/// A skill the `/` menu offers in the composer (SE-24).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skill {
    /// The command, without the `/`.
    pub name: String,
    /// What it does.
    pub summary: String,
}
