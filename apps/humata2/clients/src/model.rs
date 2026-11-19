//! The domain values the fakes hand out: hosts, channels, sessions, Inbox
//! requests, diffs, board items, files, pull requests and specs.
//!
//! These stand in for `humata2_model` and the generated `api/proto`
//! messages until the real daemon is wired in (D50). Features read them
//! only through [`crate::Workspace`], so swapping the source changes
//! nothing above the clients.

pub use crate::workspace::Workspace;
use crate::machines::OperationId;
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
id!(
    /// A view an agent prepared (VW-1).
    ViewId
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
    /// Its sessions are stopped (D260–D262): the host's copy of the stop,
    /// so it shows while the link is down. `None` while they run. Not the
    /// provider VM's power, which is [`MachineHealth::stopped`].
    pub stop: Option<MachineStop>,
    /// The machine's Humata folder (PL-1): `~/Humata` unless changed.
    pub root: String,
}

/// What changing a machine's Humata folder would do (FC-10, PL-1, ST-5): what
/// the daemon found under the old root. Applying it needs this plan's token,
/// and nothing has moved when it is read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RootPlan {
    /// The machine.
    pub machine: MachineId,
    /// What a change with this plan is allowed by, as the daemon made it.
    pub token: String,
    /// The folder now.
    pub old_root: String,
    /// The folder asked for.
    pub new_root: String,
    /// The channels with a folder under the old root.
    pub channels: Vec<ChannelId>,
    /// The sessions running in those folders; moving the channels is refused
    /// while any is listed, using the new root for new channels only is not.
    pub running: Vec<SessionId>,
}

/// Who stopped a machine's sessions (D261, D263, D264).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoppedBy {
    /// The person: from the app, or a logged-in `humata stop`.
    You,
    /// Someone typing `humata stop` in a terminal on a machine, outside
    /// every Humata session.
    Terminal {
        /// The machine the terminal is on.
        machine: MachineId,
    },
    /// A session's agent, its rule allowing it.
    Agent {
        /// The agent's session.
        session: SessionId,
    },
    /// A script, its token listing the capability.
    Script,
}

impl StoppedBy {
    /// Who, as the banner and the Inbox card say it after "Stopped by"
    /// (D264): "you", "a terminal on devbox-1", "Login v2's agent". A
    /// machine or session `workspace` doesn't know is named by its id.
    #[must_use]
    pub fn phrase(&self, workspace: &Workspace) -> String {
        match self {
            Self::You => "you".to_owned(),
            Self::Script => "a script".to_owned(),
            Self::Terminal { machine } => format!(
                "a terminal on {}",
                workspace.machine(machine).map_or_else(|| machine.to_string(), |found| found.name.clone())
            ),
            Self::Agent { session } => format!(
                "{}'s agent",
                workspace.session(session).map_or_else(|| session.to_string(), |found| found.title.clone())
            ),
        }
    }
}

/// A stop of one machine's sessions, as the host keeps it (D261, D262).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MachineStop {
    /// The stop's id; a repeat of it acts once.
    pub stop_id: String,
    /// When it was made, as clock text such as "14:02".
    pub at: String,
    /// Who made it.
    pub by: StoppedBy,
    /// The machine took it. False: the host keeps it and sends it when
    /// the machine links, and Start only cancels it (frame 76).
    pub delivered: bool,
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
    /// An adopted agent whose conversation was never known (D784).
    ConversationUnknown,
}

/// What Start fresh can summarize of a session whose resume failed (D143).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SummarySource {
    /// The old conversation file.
    Transcript,
    /// The tail of the terminal's saved output; the conversation file is gone.
    Scrollback,
    /// Nothing is left.
    Nothing,
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
    /// Its machine's sessions are stopped; Start sessions brings it back
    /// (D264, `SESSION_STATUS_MACHINE_STOPPED`).
    Stopped,
    /// The person stopped this session's agent; Resume brings it back
    /// (D784, `SESSION_STATUS_STOPPED`). Drawn as `Stopped` is, but its
    /// action is the session's Resume, never Start sessions.
    AgentStopped,
    /// Moving to another machine (MV-4); see `Session::moving`.
    Moving,
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
            Self::Stopped | Self::AgentStopped => "Stopped".to_owned(),
            Self::Moving => "Moving".to_owned(),
        }
    }

    /// Whether a person can type into it.
    #[must_use]
    pub const fn accepts_input(self) -> bool {
        !matches!(self, Self::Exited { .. } | Self::Offline | Self::Stopped | Self::AgentStopped | Self::Moving)
    }

    /// Whether it is stopped, by its machine's Stop sessions or by its own
    /// Stop: drawn as frame 73's grey square.
    #[must_use]
    pub const fn is_stopped(self) -> bool {
        matches!(self, Self::Stopped | Self::AgentStopped)
    }

    /// Whether its agent is not running and the session's own Resume brings
    /// it back: it exited, or the person stopped it.
    #[must_use]
    pub const fn agent_ended(self) -> bool {
        matches!(self, Self::Exited { .. } | Self::AgentStopped)
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

/// What archiving a session did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Archive {
    /// The disk its clones and folder free once the undo window closes, as
    /// people read it ("1.4 GB"). `None` when the machine hasn't said.
    pub frees: Option<String>,
}

/// A move to another machine in progress (MV-4, MV-17).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MoveProgress {
    /// The target; the UI shows its name.
    pub to: MachineId,
    /// The move's operation; the step card watches it.
    pub operation: OperationId,
    /// The running step's title; empty until the operation is read.
    pub step: String,
    /// That step's progress: bytes on `copy`; only rises.
    pub done_units: u64,
    /// That step's total; 0 when unknown.
    pub total_units: u64,
    /// Every step of the move in order, each with its state (frame 64).
    pub steps: Vec<MoveStep>,
}

/// One step of a move, as the daemon's operation reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MoveStep {
    /// As shown: "Copying the session".
    pub title: String,
    pub state: MoveStepState,
    /// Bytes on `copy`; only rises.
    pub done_units: u64,
    /// 0 when unknown.
    pub total_units: u64,
}

/// Where a move's step stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveStepState {
    /// Not started (also a state the daemon left unspecified).
    Pending,
    Running,
    Done,
    Failed,
    Skipped,
    /// Rolled back after the move failed.
    Undone,
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
    /// What its host left out of its agent's writes when the machine
    /// reconnected (AW-9, D267); `None` when nothing was.
    pub away_left_out: Option<AwayLeftOut>,
    /// What Start fresh could summarize, while its resume has failed and the
    /// daemon says; `None` otherwise.
    pub summary_source: Option<SummarySource>,
    /// The model its agent runs, such as "Opus", when the agent says.
    pub model: Option<String>,
    /// How it survives a restart, as its host reports it: who holds the
    /// terminal, how the agent comes back, what scrollback is kept.
    pub restart_detail: Option<String>,
    /// What its agent may do, one paragraph each, as the daemon's access
    /// rules give it; empty when not reported.
    pub permissions: Vec<String>,
    /// Whether it reviews pull requests (PR-10): its Diff starts against
    /// each pull request's base.
    pub pull_request_review: bool,
    /// Whether the person lets the agent push to the pull requests'
    /// branches (PR-8); false for any other session.
    pub push_allowed: bool,
    /// Set only while `status` is Moving (MV-4).
    pub moving: Option<MoveProgress>,
    /// The session's own folder on its machine, absolute; empty when not
    /// known (PL-10). Files sent from the session land in a folder of this
    /// name, so a renamed session keeps one.
    pub folder: String,
    /// The repositories cloned inside it, with their absolute paths.
    pub clones: Vec<SessionClone>,
    /// When its status became what it is now, unix ms (P8); the tab header
    /// shows how long it has been Working. `None` when not known.
    pub status_since_ms: Option<i64>,
}

impl Session {
    /// The title people see: an untitled shell reads as its agent, "Shell"
    /// (D784, Fable A2), never an empty row or header.
    #[must_use]
    pub fn shown_title(&self) -> String {
        if self.title.trim().is_empty() && self.agent == Agent::Shell {
            Agent::Shell.name().to_owned()
        } else {
            self.title.clone()
        }
    }
}

/// One repository cloned for a session (PL-10).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionClone {
    /// The repository, as `owner/name`.
    pub repo: String,
    /// The branch checked out in it.
    pub branch: String,
    /// Where it is on the session's machine, absolute.
    pub path: String,
}

/// Board writes a session's agent made while its host was away that the
/// host's rules no longer allowed when its machine reconnected (AW-9, D267).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AwayLeftOut {
    /// How many writes weren't applied.
    pub count: u32,
    /// The items whose history keeps one as lost (the first few).
    pub items: Vec<String>,
    /// The titles of the new items that weren't made (the first few).
    pub not_made: Vec<String>,
    /// Whether any is now refused; `false` when all now say Ask me.
    pub refused: bool,
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
    /// The agent a start that names none takes here, as the daemon says
    /// (D770): the channel's own default, Settings', or the one it was made
    /// with. `None` before a daemon has said.
    pub default_agent: Option<Agent>,
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
    /// Bring a machine's daemon, tools or holders to the host's release
    /// (UP-1). It comes from Humata, not a session.
    Migrate,
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
            Self::Migrate => "Migrate",
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
    /// What a Migrate item brings to the host's release.
    Migrate(MigrateCard),
    /// An agent's copy of files from one machine to another (IN-10, D467).
    CrossMachineCopy(CopyCard),
}

/// What an agent's cross-machine copy would do, measured at its source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CopyCard {
    /// The machine the files are on.
    pub from: MachineId,
    /// The machine they would land on.
    pub to: MachineId,
    /// Each source path as the agent named it, one per line. The card says
    /// what it is copying in words and keeps these behind Details.
    pub source_path: String,
    /// The folder they would land in, as the agent named it.
    pub destination_path: String,
    /// Where they land in words after the machine's name, such as `shared`;
    /// empty when the place has no short name.
    pub place: String,
    /// How many bytes the sources hold, measured when it was asked.
    pub bytes: u64,
    /// How many files the sources hold.
    pub files: u32,
}

impl CopyCard {
    /// What is copied, in words: the name of the one source, or how many
    /// there are.
    #[must_use]
    pub fn what(&self) -> String {
        let sources: Vec<&str> = self.source_path.lines().filter(|line| !line.is_empty()).collect();
        match sources.as_slice() {
            [one] => one
                .trim_end_matches('/')
                .rsplit('/')
                .next()
                .filter(|name| !name.is_empty())
                .map_or_else(|| (*one).to_owned(), str::to_owned),
            many => format!("{} items", many.len()),
        }
    }
}

/// What a Migrate item brings to the host's release (UP-1, D281).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MigrateThing {
    /// The machine's daemon.
    Daemon,
    /// The `humata` command line.
    Cli,
    /// The store's schema.
    Store,
    /// The processes that hold sessions' terminals.
    Holders,
    /// The tools new sessions start from.
    HostTools,
    /// The Desk folders made before documents had versions.
    DeskFolders,
}

impl MigrateThing {
    /// What people call it in a sentence.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Daemon => "daemon",
            Self::Cli => "command line",
            Self::Store => "store",
            Self::Holders => "session holders",
            Self::HostTools => "host tools",
            Self::DeskFolders => "Desk folders",
        }
    }
}

/// One build of Humata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    /// Its version, as the build reports it; empty when unknown.
    pub version: String,
    /// The full bundle id. The view shortens it.
    pub bundle: String,
}

/// Where a Migrate item is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MigratePhase {
    /// Waiting for the person to click Migrate.
    Waiting,
    /// The machine is migrating.
    Migrating,
    /// The last try failed.
    Failed(MigrateFailure),
}

/// Why a Migrate failed, in the daemon's words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigrateFailure {
    /// What failed.
    pub message: String,
    /// What the person can do.
    pub fix: String,
}

/// A Migrate item's card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigrateCard {
    /// What it brings.
    pub thing: MigrateThing,
    /// The machine it is on.
    pub machine: MachineId,
    /// What the machine runs now.
    pub from: Release,
    /// The host's release it would run.
    pub to: Release,
    /// What changes, one line each.
    pub changes: Vec<String>,
    /// What keeps running through it, one line each.
    pub keeps_running: Vec<String>,
    /// Where it is.
    pub phase: MigratePhase,
    /// Agent sessions that wait for it to restart.
    pub waiting_sessions: Vec<SessionId>,
    /// When it was migrated, as people read it ("2 min ago"). Set only once
    /// the item is answered Approved.
    pub migrated: Option<String>,
    /// Where each step of the migrate is, in order. Empty when the thing has
    /// no steps (Holders) or it has not started.
    pub steps: Vec<MigrateStep>,
    /// The sessions and shells whose holders a Holders item restarts.
    pub holders: Vec<HolderSession>,
    /// This round of Migrate's moves of a Holders item; `None` before one.
    pub moved: Option<HoldersMoved>,
}

/// How many terminals this round of a Holders Migrate has moved so far.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HoldersMoved {
    /// Terminals moved onto the current holder so far.
    pub done: u32,
    /// Terminals this round moves.
    pub total: u32,
    /// When the round started, unix ms.
    pub started_ms: i64,
}

impl HoldersMoved {
    /// The progress line for the round: measured when `total` is known.
    #[must_use]
    pub fn progress(&self) -> crate::progress::OpProgress {
        let fraction = (self.total > 0).then(|| self.done.min(self.total) as f32 / self.total as f32);
        let noun = if self.total == 1 { "terminal" } else { "terminals" };
        crate::progress::OpProgress {
            title: "Moving terminals".into(),
            fraction,
            label: format!("{} of {} {noun} moved", self.done.min(self.total), self.total),
            started_ms: self.started_ms,
        }
    }
}

/// One step of a running migrate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigrateStep {
    /// What the step does, as people read it.
    pub label: String,
    /// Where it is.
    pub state: MigrateStepState,
}

/// Where a migrate step is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MigrateStepState {
    /// Not reached yet.
    Pending,
    /// Running now.
    Doing,
    /// Finished.
    Done,
    /// It failed here.
    Failed,
}

/// A session or shell tab whose holder a Holders item restarts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HolderSession {
    /// The session; `None` for a shell tab.
    pub session: Option<SessionId>,
    /// Its title.
    pub title: String,
    /// What happens to it.
    pub note: HolderNote,
}

/// What a holder's restart does to its session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HolderNote {
    /// Restarts at the next idle moment.
    RestartsWhenIdle,
    /// Restarting now.
    Restarting,
    /// Stays on the old holder until it is closed.
    StaysUntilClosed,
    /// The agent has no turn tracking, so there is no idle moment.
    NoIdleMoment,
    /// Moves onto the current holder in place; keeps running.
    Moves,
    /// Moving now.
    Moving,
}

impl Request {
    /// The Migrate card, when this is a Migrate item.
    #[must_use]
    pub const fn migrate(&self) -> Option<&MigrateCard> {
        match &self.preview {
            Preview::Migrate(card) => Some(card),
            _ => None,
        }
    }

    /// The copy card, when this is an agent's cross-machine copy.
    #[must_use]
    pub const fn copy(&self) -> Option<&CopyCard> {
        match &self.preview {
            Preview::CrossMachineCopy(card) => Some(card),
            _ => None,
        }
    }

    /// Whether Approve all and Deny all answer it. A Migrate item is
    /// answered only with Migrate (D281), and an agent's provider import
    /// only one by one (MP-23, D792). A card that offers only Dismiss (D798)
    /// is not answered in bulk either.
    #[must_use]
    pub fn answered_in_bulk(&self) -> bool {
        self.kind != RequestKind::Migrate
            && !self.dismiss_only
            && self.operation != PROVIDER_IMPORT_OPERATION
    }
}

/// The operation id of an agent's ask to import a provider file
/// (`ProviderService.RequestProviderImport`, D792).
pub const PROVIDER_IMPORT_OPERATION: &str = "provider.request_import";

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
    /// Set when it is a question on its agent's screen: the agent tab's
    /// index among the session's tabs. The Inbox shows that terminal under
    /// the card while it waits, and offers Dismiss, not Deny (D781).
    pub on_screen: Option<usize>,
    /// Set when the only answer is Dismiss: an agent waits in its own
    /// terminal, and nothing an answer does would type into it (D798).
    pub dismiss_only: bool,
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
    /// A note under a line, such as git's "No newline at end of file": not a
    /// code line, so it has no numbers and takes no comment.
    Note,
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

/// What a changed file did to its path.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FileChange {
    /// It is new.
    Added,
    /// Its contents changed.
    #[default]
    Modified,
    /// It is gone.
    Deleted,
    /// It moved, and may have changed too.
    Renamed,
}

/// A changed file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileDiff {
    /// Its path in the repository.
    pub path: String,
    /// Lines added.
    pub added: u32,
    /// Lines removed.
    pub removed: u32,
    /// Its lines.
    pub lines: Vec<DiffLine>,
    /// What happened to the file.
    pub change: FileChange,
    /// Where the file was before, set only for a rename.
    pub old_path: Option<String>,
    /// git's rename score, 0 to 100, for a rename; 0 otherwise.
    pub similarity: u32,
    /// Whether git sees the file as binary, so it has no lines.
    pub binary: bool,
    /// Why this file's lines could not be read; `None` when they could.
    pub error: Option<String>,
    /// git's mode for the file before the change, as its octal digits read as
    /// a number (100644); 0 when the old side is missing.
    pub old_mode: u32,
    /// git's mode for the file after the change, as its octal digits read as
    /// a number (100755); 0 when the new side is missing.
    pub new_mode: u32,
    /// How much the file matters, as the daemon served it (D802): the
    /// agent's tier, else the daemon's own (lockfiles and generated files
    /// skim).
    pub tier: FileTier,
}

/// How much a file matters to the reader (D802).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FileTier {
    /// Read this first.
    Key,
    /// Read it in order.
    #[default]
    Normal,
    /// Safe to skim.
    Skim,
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

/// What a session's changes are compared with (RV-1).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DiffBase {
    /// The repository's main branch on its remote.
    #[default]
    OriginMain,
    /// Only what isn't committed yet.
    UncommittedOnly,
    /// What changed after the last Mark reviewed (RV-6).
    SinceLastReviewed,
    /// What changed after the session was forked.
    SinceFork,
    /// Another session's branch.
    OtherSession(SessionId),
    /// Each repository against its pull request's base, as GitHub shows the
    /// pull request; for pull request reviews only (PR-10).
    PullRequestBase,
    /// Each repository against its own base (RV-31). A repository left out
    /// gets the daemon's default base.
    /// A spec with a stack base ([`RepoBase::Range`] or
    /// [`RepoBase::Overlay`]) is a stack layer or the whole stack (D800):
    /// exactly its repositories show, and no others.
    Spec(Vec<RepoDiffSpec>),
}

/// Which base one repository's changes are compared with.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RepoBase {
    /// The remote's default branch, as `origin/HEAD` names it.
    DefaultBranch,
    /// Another branch on the remote, compared as a pull request shows it: the
    /// merge base with HEAD. A name that already starts with `origin/` is
    /// used as it is.
    Branch(String),
    /// A branch, tag or commit as it is.
    Rev(String),
    /// What changed after the last Mark reviewed (RV-6).
    LastReviewed,
    /// What changed after the session was forked (forks only).
    ForkPoint,
    /// Only what isn't committed yet.
    Uncommitted,
    /// This repository's pull request's base (PR-10); pull request reviews only.
    PullRequestBase,
    /// `base..head` as they are, whatever is checked out (D800): a stack
    /// layer's own change.
    Range {
        /// The older rev.
        base: String,
        /// The newer rev.
        head: String,
    },
    /// `base` against the working tree, but only the uncommitted files under
    /// `paths` (D800): a stack's uncommitted layer, or the whole stack over
    /// one.
    Overlay {
        /// The rev the overlay sits on.
        base: String,
        /// The pathspecs whose uncommitted files show.
        paths: Vec<String>,
    },
}

/// One repository's base in a [`DiffBase::Spec`].
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RepoDiffSpec {
    /// The repository, as `owner/name`.
    pub repo: String,
    /// What its changes are compared with.
    pub base: RepoBase,
}

/// The branches one repository can be compared with, as its clone knew them
/// at the last fetch (VIEWS C3). The default branch comes first when the clone
/// knows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoBases {
    /// The branch names without `origin/`.
    pub branches: Vec<String>,
    /// origin/HEAD's branch; empty when the clone doesn't know it.
    pub default_branch: String,
}

/// The base each of a session's repositories is compared with, as the Diff
/// panel picks it: one preset for every repository, plus overrides for the
/// repositories picked on their own.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DiffTargets {
    /// The base every repository uses unless it has an override.
    pub preset: DiffBase,
    /// Per-repository overrides, by `owner/name`.
    pub repos: std::collections::BTreeMap<String, RepoBase>,
}

impl DiffTargets {
    /// The base to ask for. With no overrides it is the preset as it is.
    /// Otherwise it is a [`DiffBase::Spec`] with one entry for each name in
    /// `repos`, in that order, each one as [`DiffTargets::of`] gives it.
    #[must_use]
    pub fn base(&self, repos: &[String]) -> DiffBase {
        if self.repos.is_empty() {
            return self.preset.clone();
        }
        DiffBase::Spec(
            repos
                .iter()
                .map(|repo| RepoDiffSpec {
                    repo: repo.clone(),
                    base: self.of(repo),
                })
                .collect(),
        )
    }

    /// The base one repository is compared with: its override, or else the
    /// preset as that repository's base.
    #[must_use]
    pub fn of(&self, repo: &str) -> RepoBase {
        self.repos
            .get(repo)
            .cloned()
            .unwrap_or_else(|| preset_for_repo(&self.preset))
    }

    /// Whether any repository has a base of its own.
    #[must_use]
    pub fn per_repo(&self) -> bool {
        !self.repos.is_empty()
    }

    /// Sets the preset and drops every override: the header picker resets
    /// every repository.
    pub fn pick_all(&mut self, preset: DiffBase) {
        self.preset = preset;
        self.repos.clear();
    }

    /// Gives one repository its own base. Another session's branch has no
    /// per-repository form, so while it is the preset this resets the preset
    /// to `origin/main`, and the other repositories keep that. The pull
    /// requests' bases do have one, so the other repositories keep theirs.
    pub fn pick(&mut self, repo: &str, own: RepoBase) {
        if matches!(self.preset, DiffBase::OtherSession(_)) {
            self.preset = DiffBase::OriginMain;
        }
        self.repos.insert(repo.to_owned(), own);
    }
}

/// A session-wide preset as one repository's base.
fn preset_for_repo(preset: &DiffBase) -> RepoBase {
    match preset {
        DiffBase::OriginMain => RepoBase::DefaultBranch,
        DiffBase::UncommittedOnly => RepoBase::Uncommitted,
        DiffBase::SinceLastReviewed => RepoBase::LastReviewed,
        DiffBase::SinceFork => RepoBase::ForkPoint,
        DiffBase::PullRequestBase => RepoBase::PullRequestBase,
        // `pick` never leaves another session as the preset with overrides,
        // and a spec is never a preset, so these read as the default branch.
        DiffBase::OtherSession(_) | DiffBase::Spec(_) => RepoBase::DefaultBranch,
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

/// What pushing a session would do, one repository at a time (RV-2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushPlan {
    /// Proves the plan is current; a push spends it, so it works once.
    pub token: String,
    /// The repositories the session has changes in.
    pub repos: Vec<PushRepo>,
}

/// One repository's branch and its changes, as a push would find them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushRepo {
    /// The repository, as `owner/name`.
    pub repo: String,
    /// The branch the session's work is on.
    pub branch: String,
    /// Files with uncommitted changes.
    pub dirty_files: u32,
    /// Commits not yet on the remote branch.
    pub unpushed_commits: u32,
    /// Lines added against the base.
    pub added: u32,
    /// Lines removed against the base.
    pub removed: u32,
    /// The open pull request for this branch, when there is one.
    pub open_pull_request: Option<u32>,
}

/// A file the person looked at (D852): the repository (`owner/name`), the
/// file's path, and the fingerprint of the diff that was shown then.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LookedFile {
    pub repo: String,
    pub path: String,
    pub fingerprint: u64,
}

/// A pull request a push opened or found for one repository.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushedPullRequest {
    /// The repository, as `owner/name`.
    pub repo: String,
    /// The pull request's number.
    pub number: u32,
    /// What the push found about an open pull request it left as it was,
    /// such as "PR #3 targets main"; empty otherwise.
    pub note: String,
}

/// What to do with one repository of a push.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushChoice {
    /// The repository, as `owner/name`.
    pub repo: String,
    /// Uncommitted files are committed with this message first; empty
    /// means don't commit.
    pub commit_message: String,
    /// Whether to open a pull request after the push.
    pub open_pull_request: bool,
    /// The pull request's title.
    pub title: String,
    /// The pull request's body.
    pub body: String,
    /// The branch the pull request targets, such as `release/2.4`. Empty
    /// means the repository's default branch.
    pub base_branch: String,
}

/// Where a board item lives.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Scope {
    /// The global board.
    Global,
    /// A channel's board.
    Channel(ChannelId),
    /// A session's board, in its channel (BD-1, BD-8).
    Session {
        /// The session's channel.
        channel: ChannelId,
        /// The session.
        session: SessionId,
    },
}

impl Scope {
    /// The channel the board belongs to; `None` for the Global board.
    #[must_use]
    pub fn channel(&self) -> Option<&ChannelId> {
        match self {
            Self::Global => None,
            Self::Channel(channel) | Self::Session { channel, .. } => Some(channel),
        }
    }
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

    /// The order of a board stacked in a side panel: active lanes first.
    pub const COMPACT: [Self; 7] = [
        Self::InProgress,
        Self::InReview,
        Self::Todo,
        Self::Backlog,
        Self::Done,
        Self::Blocked,
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
    /// The daemon's revision of the item when it was read. A move names it,
    /// so a change made since by an agent or another window is refused
    /// instead of silently overwritten; 0 for an item no daemon has numbered.
    pub revision: u64,
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
    /// A requested edit or delete (BD-15); a requested move is
    /// `requested_status`.
    pub requested_change: Option<ItemChange>,
    /// Day in the dashboard's fourteen-day reporting window, oldest first.
    pub done_day: Option<u8>,
}

/// An agent's requested change to an item other than a move (BD-15).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ItemChange {
    /// Fields change, each from before to after.
    Edit(Vec<FieldChange>),
    /// The item goes.
    Delete,
}

/// One field of a requested edit, as the item stands now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldChange {
    /// The field's name, such as `Title`.
    pub field: String,
    /// Its value now.
    pub before: String,
    /// Its value after the edit.
    pub after: String,
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

/// Where a prepared view stands with the person (VW-2 to VW-9).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewState {
    /// Prepared by the agent, not offered yet.
    Draft,
    /// Offered, waiting for the person.
    Waiting,
    /// Put off until later.
    Later,
    /// Turned down for now.
    NotNow,
    /// Accepted and open on screen.
    Opened,
    /// Opened, then put back as it was.
    Reverted,
    /// Opened, then closed.
    OpenedAndClosed,
    /// Opened and kept as it is.
    Kept,
}

/// A skill the `/` menu offers in the composer (SE-24).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skill {
    /// The command, without the `/`.
    pub name: String,
    /// What it does.
    pub summary: String,
}
