//! The daemon client: everything the app asks of the daemons, and the
//! events they push. One trait; [`FakeDaemon`] is the only implementation
//! until the real one is wired in over `api/proto`.

use tca::BoxFuture;
use tokio::sync::mpsc;

use crate::model::{
    AccessRule, Agent, Answer, Archive, Capability, ChannelId, DiffBase, ForkKind, GrantId, Item,
    ItemDetails, ItemId, ItemStatus, Line, MachineId, MigrateThing, PushChoice, PushPlan, Recovery, RepoBases,
    RepoDiff, RequestId, RuleSetting, Scope, Session, SessionFile, SessionId, SetupId, Transfer,
    TransferId,
};
use crate::workspace::Workspace;

/// How many events a subscriber may fall behind before it is told to
/// reload instead (TR-8: queues are bounded).
pub const EVENT_QUEUE: usize = 256;

/// The most files listed for one session: a bigger clone shows its first
/// files in the palette and the rest are one search away.
pub const SESSION_FILES_CAP: usize = 500;

/// A daemon reply.
pub type Reply<T> = BoxFuture<Result<T, DaemonError>>;

/// The events a subscription delivers, in order.
pub type Events = mpsc::Receiver<Result<Event, DaemonError>>;

/// One change to a file in a session's clone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileChange {
    /// A file appeared.
    Created(SessionFile),
    /// A file went away.
    Deleted(SessionFile),
    /// A file moved.
    Renamed {
        /// Where it was.
        from: SessionFile,
        /// Where it is now.
        to: SessionFile,
    },
}

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
    /// A file in a session's clone changed. Only sessions whose file list
    /// was fetched act on it.
    Files {
        /// The session.
        session: SessionId,
        /// What happened.
        change: FileChange,
    },
    /// SEAM (D438): the host's history index added entries of this
    /// session's conversation; a search made before may have missed them.
    HistoryIndexed(SessionId),
    /// SEAM (DESKWATCH): a Desk session changed, or left the Desk. The
    /// app applies it to the workspace's Desk in place.
    Desk(crate::desk::DeskChange),
    /// SEAM (MACHINESCRIPTS, D686): a script run on this machine started, moved or ended;
    /// read its runs again.
    MachineScriptRun(MachineId),
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
    /// [`Daemon::github_repos`].
    GitHubRepos,
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
    /// [`Daemon::bases`].
    Bases,
    /// [`Daemon::find_default_branch`].
    FindDefaultBranch,
    /// [`Daemon::mark_reviewed`].
    MarkReviewed,
    /// [`Daemon::plan_push`].
    PlanPush,
    /// [`Daemon::push`].
    Push,
    /// [`Daemon::set_asks_in_inbox`].
    SetAsksInInbox,
    /// [`Daemon::set_agent_switch`].
    SetAgentSwitch,
    /// [`Daemon::rename_session`].
    RenameSession,
    /// [`Daemon::add_repo_to_session`].
    AddRepoToSession,
    /// [`Daemon::stop_agent`].
    StopSession,
    /// [`Daemon::continue_in_channel`].
    ContinueInChannel,
    /// [`Daemon::search_repos`].
    SearchRepos,
    /// [`Daemon::repo_library`].
    RepoLibrary,
    /// [`Daemon::remove_from_repo_library`].
    RemoveFromRepoLibrary,
    /// [`Daemon::forget_repo_library`].
    ForgetRepoLibrary,
    /// [`Daemon::repo_scope`].
    RepoScope,
    /// [`Daemon::save_repo_scope`].
    SaveRepoScope,
    /// [`Daemon::add_repo_to_channel`].
    AddRepoToChannel,
}

/// Why a daemon call failed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DaemonError {
    /// The host doesn't answer.
    #[error(
        "{} doesn't answer, so this can't be done there now. Reconnect it from Hosts; its sessions keep running and nothing is lost",
        crate::machine_names::name_of(.machine)
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
    /// Someone else changed it after the app last read it, so the change
    /// was not made: the daemon holds the newer version.
    #[error(
        "{what} was not changed: {reason}"
    )]
    Conflict {
        /// What was asked.
        what: String,
        /// What changed and what to do, in the daemon's words.
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
    /// The call would run across releases (UP-1, D281): the machine's
    /// `thing` is behind this Humata, and its Migrate item in the Inbox brings
    /// it up to date. Nothing was changed.
    #[error(
        "{}'s {} is at another release of Humata than this one, so this wasn't done. Open the Migrate item in the Inbox and click Migrate; nothing was changed",
        crate::machine_names::name_of(.machine),
        .thing.label()
    )]
    MigrationNeeded {
        /// What is behind.
        thing: MigrateThing,
        /// The machine it is on.
        machine: MachineId,
        /// The Migrate item that fixes it, when the daemon named one.
        request: Option<RequestId>,
    },
    /// A repository's clone doesn't know its remote's default branch, so
    /// its changes can't be read against it (frame 98).
    /// [`Daemon::find_default_branch`] fixes it; a base that names a branch
    /// works around it.
    #[error(
        "{repo} has no default branch to compare with, so its changes weren't read. Use Find the default branch to have Humata ask its remote, or pick a branch to compare against"
    )]
    NoDefaultBranch {
        /// The repository, as `owner/name`.
        repo: String,
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
    /// PR-17: a pull request review's drafts never submitted. Archiving
    /// keeps them with the archived review (D513).
    pub unsubmitted_drafts: u32,
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
    /// The branches one repository of the session can be compared with, the
    /// default branch first (VIEWS C3), as its clone knows them at the last
    /// fetch. Nothing is fetched.
    fn bases(&self, session: &SessionId, repo: &str) -> Reply<RepoBases> {
        let what = format!("The branches of {repo} in {session}");
        Box::pin(async move { Err(DaemonError::NotServedYet { what }) })
    }
    /// Has Humata ask `repo`'s remote for its default branch in the
    /// session's clone, on the session's machine (frame 98), after a read
    /// answered [`DaemonError::NoDefaultBranch`]. The branch it names now.
    fn find_default_branch(&self, session: &SessionId, repo: &str) -> Reply<String> {
        let what = format!("Finding the default branch of {repo} in {session}");
        Box::pin(async move { Err(DaemonError::NotServedYet { what }) })
    }

    /// The files in each of the session's clones, at most
    /// [`SESSION_FILES_CAP`], for the palette's Files group. Asked once per
    /// session; [`Event::Files`] keeps the list current after that.
    fn find_files(&self, session: &SessionId) -> Reply<Vec<SessionFile>> {
        let what = format!("The files of {session}");
        Box::pin(async move { Err(DaemonError::NotServedYet { what }) })
    }

    /// What pushing each of the session's repositories would do (RV-2).
    fn plan_push(&self, session: &SessionId) -> Reply<PushPlan> {
        let what = format!("Pushing {session}");
        Box::pin(async move { Err(DaemonError::NotServedYet { what }) })
    }
    /// Commits, pushes and opens pull requests for the chosen repositories
    /// of a plan, spending its `token`. Returns the pull request the daemon
    /// opened or found for each repository that has one, with its note.
    fn push(
        &self,
        session: &SessionId,
        _token: &str,
        _choices: Vec<PushChoice>,
    ) -> Reply<Vec<crate::model::PushedPullRequest>> {
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
    /// Like [`Daemon::send`], but `key` names this one message: sending the
    /// same key again, as an app does after it quit mid-send, types nothing
    /// a second time.
    fn send_keyed(&self, session: &SessionId, text: &str, key: &str) -> Reply<()> {
        let _ = key;
        self.send(session, text)
    }
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
    /// Starts a new top-level session in `channel`, set up as that
    /// channel's New session, whose agent resumes a copy of `session`'s
    /// conversation: all of it, or what came before turn `before_turn`
    /// (CV-8, CV-2). `session` is left as it is. Returns the new session.
    fn continue_in_channel(
        &self,
        session: &SessionId,
        channel: &ChannelId,
        _before_turn: Option<u32>,
    ) -> Reply<SessionId> {
        let what = format!("Continuing {session} in {channel}");
        Box::pin(async move { Err(DaemonError::NotServedYet { what }) })
    }
    /// Renames a session, and with `rename_branch` its branch from the new
    /// title (FC-17). The daemon refuses to rename a branch already pushed.
    fn rename_session(&self, session: &SessionId, title: &str, rename_branch: bool) -> Reply<()> {
        let _ = (title, rename_branch);
        let what = format!("Renaming {session}");
        Box::pin(async move { Err(DaemonError::NotServedYet { what }) })
    }
    // REPOS (CH-21): clones `repo` (`owner/name`) in a running session on its
    // branch, adds it to the channel when the channel lacks it, and tells the
    // agent. Waits for the clone.
    fn add_repo_to_session(&self, session: &SessionId, repo: &str) -> Reply<()> {
        let _ = repo;
        let what = format!("Adding a repo to {session}");
        Box::pin(async move { Err(DaemonError::NotServedYet { what }) })
    }
    /// Stops a session's agent; the session, its terminals and clones stay
    /// (SE-7).
    fn stop_agent(&self, session: &SessionId) -> Reply<()> {
        let what = format!("Stopping the agent of {session}");
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
    /// Marks a session read as it opens (CH-17): clears its unread mark and
    /// a reattached or resumed note, and views it as [`Self::view`] does. A
    /// session that couldn't come back keeps its recovery until it is
    /// resumed or started fresh. A session the daemon doesn't know has
    /// nothing to clear.
    fn mark_read(&self, session: &SessionId) -> Reply<()>;
    /// The session the user is looking at now, or none (CH-16, CH-17;
    /// D392): its new output marks nothing unread, and viewing it clears
    /// its mark. Each call replaces the last.
    fn view(&self, session: Option<&SessionId>) -> Reply<()>;
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
    /// The GitHub repositories `gh` on the host sees, as `owner/name`, that
    /// match `query` (all of them for an empty one), the most recently
    /// pushed first (CH-3).
    fn github_repos(&self, query: &str) -> Reply<Vec<String>> {
        let what = format!("Listing the GitHub repositories gh sees matching {query:?}");
        Box::pin(async move { Err(DaemonError::NotServedYet { what }) })
    }
    // REPOS (RL-5): the one repo search. `scope` None is the saved default.
    fn search_repos(&self, query: &str, scope: Option<crate::repos::RepoScope>) -> Reply<crate::repos::RepoSearch> {
        let what = format!("Searching repos for {query:?}");
        Box::pin(async move { Err(DaemonError::NotServedYet { what }) })
    }
    // REPOS (RL-4): the repo library, most recently used first.
    fn repo_library(&self) -> Reply<Vec<crate::repos::LibraryRepo>> {
        let what = "Listing the repo library".to_owned();
        Box::pin(async move { Err(DaemonError::NotServedYet { what }) })
    }
    // REPOS (RL-4): drops one repo (`owner/name`) from the library; the library left.
    fn remove_from_repo_library(&self, repo: &str) -> Reply<Vec<crate::repos::LibraryRepo>> {
        let what = format!("Removing {repo} from the repo library");
        Box::pin(async move { Err(DaemonError::NotServedYet { what }) })
    }
    // REPOS (RL-4): empties the library; how many repos it held.
    fn forget_repo_library(&self) -> Reply<u32> {
        let what = "Forgetting the repo library".to_owned();
        Box::pin(async move { Err(DaemonError::NotServedYet { what }) })
    }
    // REPOS (RL-7): the saved default scope.
    fn repo_scope(&self) -> Reply<crate::repos::RepoScope> {
        let what = "Reading the repo search scope".to_owned();
        Box::pin(async move { Err(DaemonError::NotServedYet { what }) })
    }
    // REPOS (RL-8): saves the default scope; All of GitHub is refused.
    fn save_repo_scope(&self, scope: crate::repos::RepoScope) -> Reply<crate::repos::RepoScope> {
        let _ = scope;
        let what = "Saving the repo search scope".to_owned();
        Box::pin(async move { Err(DaemonError::NotServedYet { what }) })
    }
    // REPOS (CH-6, CH-18): adds a GitHub repo (`owner/name`) to a channel and clones
    // it there by the channel's root rules; the host records it in the repo library.
    // Answers once the clone is made, or refuses with why.
    fn add_repo_to_channel(&self, channel: &ChannelId, repo: &str) -> Reply<()> {
        let _ = channel;
        let what = format!("Adding {repo} to a channel");
        Box::pin(async move { Err(DaemonError::NotServedYet { what }) })
    }
    // REPOS (RL-8): the orgs the user's GitHub login belongs to, for + Org.
    fn repo_owners(&self) -> Reply<Vec<String>> {
        let what = "Listing the orgs your GitHub login belongs to".to_owned();
        Box::pin(async move { Err(DaemonError::NotServedYet { what }) })
    }
    // REPOS (RL-6): the time in milliseconds since the epoch, to work "used 2 days
    // ago" out from. The reducers ask here, never the screen, so a test fixes it.
    fn now_ms(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .and_then(|age| i64::try_from(age.as_millis()).ok())
            .unwrap_or_default()
    }
    /// Stops a channel setup; the folders it made are moved to the trash.
    fn cancel_setup(&self, setup: &SetupId) -> Reply<()>;
    /// Runs a failed channel setup's failed steps again; the channel, once done.
    fn retry_setup(&self, setup: &SetupId) -> Reply<ChannelId>;
    /// Moves a board item to a column. `seen` is the item's revision as the
    /// caller last read it: when the item's column changed since, the move is
    /// refused as [`DaemonError::Conflict`] instead of overwriting that
    /// change. `None` skips the check, for an item just made.
    fn move_item(&self, item: &ItemId, status: ItemStatus, seen: Option<u64>) -> Reply<()>;
    /// Adds a board item in `status`'s column, in one call: the column comes
    /// with the create, so a refused move can't leave it somewhere else.
    fn create_item_in(&self, scope: Scope, title: &str, status: ItemStatus) -> Reply<ItemId>;
    /// Adds a board item in To do, the column the app's New item uses.
    fn create_item(&self, scope: Scope, title: &str) -> Reply<ItemId> {
        self.create_item_in(scope, title, ItemStatus::Todo)
    }
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
    /// Makes `agent` Settings' default agent, the one a start takes when
    /// neither it nor the channel names one (ST-15, D770). The daemon then
    /// tells each channel's default again.
    fn set_default_agent(&self, agent: Agent) -> Reply<()>;
    /// Whether Codex's hooks still need trusting in Codex's /hooks (D202).
    fn codex_needs_trust(&self) -> Reply<bool>;
    /// Every built-in agent except the shell with its switches, enabled or
    /// not, in the daemon's order (AGENTSET).
    fn agent_switches(&self) -> Reply<Vec<AgentSwitch>>;
    /// Changes one agent's switches; gives what the daemon holds after.
    fn set_agent_switch(&self, agent: Agent, change: SwitchChange) -> Reply<AgentSwitch>;
}

/// The terminals of sessions. What a tab draws, and what is typed into it,
/// go over its [`Terminals::link`]; the store holds no terminal bytes.
pub trait Terminals: Send + Sync + 'static {
    /// Starts a shell, or attaches the one terminal already running this agent.
    fn open(&self, session: &SessionId, agent: Agent) -> Reply<(usize, Session)>;

    /// Starts a new tab of `session`'s machine running `agent`, started in
    /// `folder`, an absolute path on that machine (PL-10, Open terminal
    /// here). The tab is always new: nothing is attached.
    fn open_in(&self, session: &SessionId, _agent: Agent, folder: &str) -> Reply<(usize, Session)> {
        let what = format!("Starting a terminal for {session} in {folder}");
        Box::pin(async move { Err(DaemonError::NotServedYet { what }) })
    }

    /// Types `text` and Enter into tab `tab` of `session`.
    fn write(&self, session: &SessionId, tab: usize, text: &str) -> Reply<()>;

    /// Opens an ordered link to tab `tab` of `session`: the terminal
    /// component's frames in, its frames out (D271). The first frame is the
    /// tab's screen. Input sent while the link reconnects, or after it
    /// closed, fails with [`SendError::Closed`]; it is never replayed.
    fn link(&self, session: &SessionId, tab: usize) -> Reply<TerminalLink>;

    /// Saves an image pasted into one of `session`'s tabs on the session's
    /// machine and returns its absolute path there, for the tab to type
    /// (D30). `bytes` must be an image of `kind`, at most 32 MiB.
    fn paste_image(&self, session: &SessionId, kind: ImageKind, bytes: Vec<u8>) -> Reply<String>;
}

/// Why a frame could not be sent on a [`TerminalLink`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SendError {
    /// The link is closed or reconnecting.
    #[error(
        "The terminal is closed or reconnecting, so what you typed was not sent. Wait for the terminal to show its screen again, then type it again"
    )]
    Closed,
    /// The link is not keeping up.
    #[error(
        "The terminal is not keeping up, so this was not sent. Wait a moment, then try again"
    )]
    Full,
}

impl<T> From<tokio::sync::mpsc::error::TrySendError<T>> for SendError {
    fn from(error: tokio::sync::mpsc::error::TrySendError<T>) -> Self {
        match error {
            tokio::sync::mpsc::error::TrySendError::Full(_) => SendError::Full,
            tokio::sync::mpsc::error::TrySendError::Closed(_) => SendError::Closed,
        }
    }
}

/// Where a view's frames go: refuses them once the link is closed, and
/// while it reconnects, so nothing typed is replayed into a later attach.
#[derive(Clone)]
pub struct LinkSink {
    send: std::sync::Arc<dyn Fn(terminal_wire::ClientFrame) -> Result<(), SendError> + Send + Sync>,
}

impl LinkSink {
    /// A sink that hands each frame to `send`.
    pub fn new(
        send: impl Fn(terminal_wire::ClientFrame) -> Result<(), SendError> + Send + Sync + 'static,
    ) -> Self {
        LinkSink {
            send: std::sync::Arc::new(send),
        }
    }

    /// Sends `frame`, in order after the ones sent before it.
    pub fn send(&self, frame: terminal_wire::ClientFrame) -> Result<(), SendError> {
        (self.send)(frame)
    }
}

/// One tab's link: the frames from the terminal, in order, and where the
/// view's frames go. Dropping it closes the link; the program keeps running.
pub struct TerminalLink {
    /// The terminal's frames, the screen first.
    pub frames: tokio::sync::mpsc::Receiver<terminal_wire::ServerFrame>,
    /// Humata's own lines about the tab, drawn beside its terminal.
    pub notices: tokio::sync::mpsc::Receiver<TerminalNotice>,
    /// The view's frames.
    pub sink: LinkSink,
}

/// One of Humata's own lines about a tab, drawn beside its terminal rather
/// than in it. A lost connection and an exit are drawn from the session's
/// and the screen's state, so they are not here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalNotice {
    /// The tab's machine is back (SE-16) and the screen shows what it
    /// printed meanwhile: `caught_up_lines` lines, 0 when nobody counted.
    Reconnected { caught_up_lines: u32 },
    /// How many others can type into the tab now (SE-24), such as another
    /// window or a `humata attach`; read-only attaches are not counted.
    /// The tab bar says it, not a line.
    Writers { others: u32 },
    /// The daemon refused to let this view type until something is
    /// migrated (D377): always a [`DaemonError::MigrationNeeded`]. The
    /// attach ended; nothing typed is sent.
    Refused(DaemonError),
}

/// The kinds of image a terminal paste saves on the session's machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageKind {
    Png,
    Jpeg,
    Gif,
    Webp,
    Tiff,
    Heic,
}

mod agent_switches;
mod fake;

pub use agent_switches::{AgentSwitch, SwitchChange};
pub use fake::{FakeDaemon, FakeViews};
