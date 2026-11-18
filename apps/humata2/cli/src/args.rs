//! The command line (CLI.md §4 and §5).

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(
    name = "humata",
    version,
    about = "Humata from a terminal: each command is one operation on the daemon",
    propagate_version = true
)]
pub struct Cli {
    #[command(flatten)]
    pub global: Global,
    #[command(subcommand)]
    pub command: Command,
}

/// The flags every command takes (CLI.md §4).
#[derive(Clone, Debug, Default, Args)]
pub struct Global {
    /// Print one JSON object on stdout and nothing else there.
    #[arg(long, global = true)]
    pub json: bool,
    /// The channel, by name or id.
    #[arg(long, global = true, value_name = "NAME|ID")]
    pub channel: Option<String>,
    /// A machine, by name or id. Some commands take several.
    #[arg(long, global = true, value_name = "NAME|ID")]
    pub machine: Vec<String>,
    /// How long to wait for an answer. Default: until done in a terminal,
    /// 0 with --json.
    #[arg(long, global = true, value_name = "SECONDS")]
    pub wait: Option<u32>,
    /// Fail (exit 20) when the change is only journaled.
    #[arg(long, global = true)]
    pub strict: bool,
    /// Reuse an idempotency key to retry the same intent safely.
    #[arg(long, global = true, value_name = "KEY")]
    pub key: Option<String>,
    /// Do what a plan printed by --dry-run said.
    #[arg(long, global = true, value_name = "TOKEN", conflicts_with = "dry_run")]
    pub plan: Option<String>,
    /// Print the plan and its token; change nothing.
    #[arg(long, global = true)]
    pub dry_run: bool,
    /// Skip "Continue?" after a plan. Never skips presence.
    #[arg(short = 'y', long, global = true)]
    pub yes: bool,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Which daemon answers, and who you are to it.
    Status,
    /// Who the daemon takes you for.
    Whoami,
    /// Stop every session on this machine, and keep them stopped until
    /// `humata start`. A session is stopped with `humata session stop`.
    Stop {
        /// Why, shown wherever the stop is.
        #[arg(long)]
        reason: Option<String>,
    },
    /// Start this machine's sessions again after a stop.
    Start,
    /// Sessions.
    Ls {
        /// Archived sessions too.
        #[arg(long)]
        all: bool,
    },
    /// Channels.
    Channels,
    #[command(subcommand)]
    Channel(ChannelCommand),
    /// The channel's sessions and their forks.
    Topology,
    /// The channel's activity.
    Activity,
    /// Live events, one per line, until stopped.
    Events {
        #[arg(long, value_name = "SEQ")]
        after: Option<u64>,
        #[arg(long, value_enum)]
        topic: Vec<Topic>,
    },
    /// The audit log.
    Audit,
    /// Long-running operations.
    #[command(args_conflicts_with_subcommands = true)]
    Ops {
        id: Option<String>,
        #[command(subcommand)]
        command: Option<OpsCommand>,
    },
    /// A session's terminal, here.
    Attach {
        /// Optional with `--tab`, which names a terminal no session holds.
        #[arg(required_unless_present = "tab")]
        session: Option<String>,
        #[arg(long)]
        tab: Option<String>,
        /// Watch without typing; not counted as a writer.
        #[arg(long)]
        read_only: bool,
    },
    #[command(subcommand)]
    Session(SessionCommand),
    /// A shell on a machine, or with `--session` a new shell tab in that
    /// session.
    Shell {
        #[arg(required_unless_present = "session", conflicts_with = "session")]
        machine: Option<String>,
        #[arg(long)]
        session: Option<String>,
    },
    /// A session's conversation.
    Conversation { session: String },
    /// Board items. `--global` and `--session` pick the board for every
    /// subcommand; `--column` filters the list.
    Board {
        #[command(flatten)]
        which: BoardWhich,
        #[arg(long, value_enum)]
        column: Vec<Column>,
        #[command(subcommand)]
        command: Option<BoardCommand>,
    },
    /// Views agents prepared.
    Views {
        #[arg(long)]
        session: Option<String>,
    },
    /// Open, defer or close a prepared view.
    View {
        #[arg(value_enum)]
        action: ViewAction,
        view: String,
    },
    /// A session's changes.
    #[command(args_conflicts_with_subcommands = true, subcommand_negates_reqs = true)]
    Diff {
        #[arg(required = true)]
        session: Option<String>,
        #[arg(long)]
        repo: Vec<String>,
        /// main, uncommitted, last-reviewed, fork, pr, or another session.
        #[arg(long, default_value = "main")]
        against: String,
        #[command(subcommand)]
        command: Option<DiffCommand>,
    },
    #[command(subcommand)]
    Comment(CommentCommand),
    #[command(subcommand)]
    Review(ReviewCommand),
    /// Requests waiting for you. `--session` and `--channel` scope the
    /// list and approve-all/deny-all.
    Inbox {
        /// Answered requests too.
        #[arg(long)]
        all: bool,
        #[arg(long)]
        session: Option<String>,
        #[arg(long, requires = "session")]
        with_forks: bool,
        #[command(subcommand)]
        command: Option<InboxCommand>,
    },
    /// What agents may do.
    #[command(args_conflicts_with_subcommands = true)]
    Access {
        #[command(subcommand)]
        command: Option<AccessCommand>,
    },
    /// Prove you're here once for this terminal session.
    Login {
        #[arg(long, default_value_t = 12)]
        hours: u32,
    },
    Logout,
    /// Terminal sessions that are logged in.
    Logins,
    #[command(subcommand)]
    Token(TokenCommand),
    #[command(subcommand)]
    Files(FilesCommand),
    /// Copy files between machines.
    Cp(TransferArgs),
    /// Move files between machines.
    Mv(TransferArgs),
    /// Copies and moves in progress.
    Transfers {
        /// Running and paused ones only.
        #[arg(long)]
        active: bool,
    },
    #[command(subcommand)]
    Transfer(TransferCommand),
    /// What's in a machine's trash.
    #[command(args_conflicts_with_subcommands = true)]
    Trash {
        #[command(subcommand)]
        command: Option<TrashCommand>,
    },
    #[command(subcommand, alias = "machine")]
    Host(HostCommand),
    #[command(subcommand)]
    Provider(ProviderCommand),
    #[command(subcommand)]
    Skills(SkillsCommand),
    #[command(subcommand)]
    Bridge(BridgeCommand),
    /// Settings.
    #[command(args_conflicts_with_subcommands = true)]
    Settings {
        #[command(subcommand)]
        command: Option<SettingsCommand>,
    },
    #[command(subcommand)]
    Agents(AgentsCommand),
    /// Pin a channel, session, machine, item or request.
    Pin { thing: String },
    Unpin { thing: String },
    /// Mark something VIP.
    Vip { thing: String },
    Unvip { thing: String },
    /// What's pinned, in order.
    #[command(args_conflicts_with_subcommands = true)]
    Pinned {
        #[command(subcommand)]
        command: Option<PinnedCommand>,
    },
    /// Find what sessions said.
    #[command(subcommand)]
    Sessions(SessionsCommand),
    /// Desk sessions: the ones that belong to no channel.
    #[command(subcommand)]
    Desk(DeskCommand),
    // REPOS
    /// The repo library and the one repo search.
    #[command(subcommand)]
    Repos(ReposCommand),
    #[command(subcommand)]
    Report(ReportCommand),
    /// Find out why a restart didn't bring everything back.
    #[command(subcommand)]
    Doctor(DoctorCommand),
    /// Say what turn this session's agent is in. For the agent's hooks:
    /// prints nothing, and works only from the session's own terminal.
    Turn {
        #[arg(value_enum)]
        state: TurnArg,
    },
    /// Pass an agent's hook to the Inbox and print its answer. For the
    /// agent's hooks: reads the payload on stdin, always exits 0.
    Hook {
        /// claude, codex or copilot
        agent: String,
        #[arg(value_enum)]
        event: HookEventArg,
    },
    /// Run OpenCode for a Humata session: its TUI on a server of its own,
    /// with turns and permission asks going to Humata. The built-in
    /// OpenCode agent runs this; exits as OpenCode's TUI does.
    Opencode {
        /// The session's conversation id; OpenCode's session follows from it.
        #[arg(long)]
        conversation: String,
        /// Where the server's address and password are kept while it runs.
        #[arg(long)]
        state: std::path::PathBuf,
        /// The OpenCode to run.
        #[arg(long, default_value = "opencode")]
        program: std::path::PathBuf,
        /// The model the session was started on (`provider/model`); none
        /// runs OpenCode's own default.
        #[arg(long)]
        model: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum ChannelCommand {
    Create {
        name: String,
        /// owner/name
        #[arg(long)]
        repo: Vec<String>,
    },
    Rename {
        channel: String,
        name: String,
        /// Rename even while a session of the channel is in a turn: its
        /// agent is stopped and resumed in the channel's new folder.
        #[arg(long)]
        confirm_running: bool,
    },
    #[command(subcommand)]
    Repo(ChannelRepoCommand),
    #[command(subcommand)]
    Machine(ChannelMachineCommand),
    /// Which repos are cloned on which machines.
    Grid { channel: String },
    Settings {
        channel: String,
        #[command(subcommand)]
        command: Option<KeyCommand>,
    },
    Delete {
        channel: String,
        /// Delete the sessions' unpushed work too (CH-24).
        #[arg(long)]
        delete_work: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum ChannelRepoCommand {
    Add { channel: String, repo: String },
    Rm { channel: String, repo: String },
}

#[derive(Debug, Subcommand)]
pub enum ChannelMachineCommand {
    Add { channel: String, machine: String },
    Rm { channel: String, machine: String },
}

#[derive(Debug, Subcommand)]
pub enum KeyCommand {
    Get { key: Option<String> },
    Set { key: String, value: String },
}

#[derive(Debug, Subcommand)]
pub enum OpsCommand {
    Cancel { id: String },
    Retry { id: String },
}

#[derive(Debug, Subcommand)]
pub enum DeskCommand {
    /// Keep a Desk session's scratch folder after it closes, so it is
    /// never deleted after its 7 days. Inside a Desk session, leave out
    /// the session to keep this one's own folder.
    Keep {
        /// The session's id or title; this session's own when left out.
        session: Option<String>,
    },
}

// REPOS
#[derive(Debug, Subcommand)]
pub enum ReposCommand {
    /// Search the repo library, then GitHub. With no flag it uses the saved
    /// scope; a flag changes the scope for this one search and is never
    /// saved.
    Search {
        /// The words to look for.
        query: Vec<String>,
        /// Search this GitHub owner (an organization or a user); repeat it
        /// for more.
        #[arg(
            long = "owner",
            value_name = "OWNER",
            conflicts_with_all = ["all_of_github", "your_repos"]
        )]
        owners: Vec<String>,
        /// Search all of GitHub.
        #[arg(long, conflicts_with = "your_repos")]
        all_of_github: bool,
        /// Search the repos your own gh login can see.
        #[arg(long)]
        your_repos: bool,
    },
    /// The library, most recently used first, with where each was last used.
    List,
    /// Take one repo out of the library. Channels, sessions and clones that
    /// use it are untouched. For the user only.
    Remove {
        /// The repo as owner/name.
        repo: String,
    },
    /// Forget the whole library. Asks first (or pass --yes); channels,
    /// sessions, clones and GitHub are untouched. For the user only.
    ForgetAll,
    /// The saved scope every search uses unless one search widens it.
    #[command(subcommand)]
    Scope(RepoScopeCommand),
}

// REPOS
#[derive(Debug, Subcommand)]
pub enum RepoScopeCommand {
    /// Print the saved scope.
    Show,
    /// Save the scope: these owners plus your own repos, or with no --owner
    /// just your own repos. For the user only.
    Set {
        /// A GitHub user or organization to search; repeat it for more.
        #[arg(long = "owner", value_name = "OWNER")]
        owners: Vec<String>,
        /// Leave out the repos your own gh login can see.
        #[arg(long)]
        no_include_me: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum SessionsCommand {
    /// Search every session you may see for words.
    Search {
        /// Every word must match.
        words: Vec<String>,
        /// Only this agent kind, such as claude.
        #[arg(long)]
        agent: Option<String>,
        /// Only after this time (RFC 3339).
        #[arg(long)]
        since: Option<String>,
        /// Only before this time (RFC 3339).
        #[arg(long)]
        until: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum SessionCommand {
    /// Typing waiting for the turn to end.
    Queue { session: String },
    /// A session's terminal tabs: its agent's and any shells, with their ids
    /// for `attach --tab`.
    Tabs { session: String },
    /// Type queued text now.
    SendNow { session: String, ids: Vec<String> },
    /// Start a session on --machine (default: this machine, D672 (Provisional)).
    Start {
        #[arg(long)]
        agent: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        repo: Vec<String>,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        message: Option<String>,
    },
    Fork {
        session: String,
        /// What the fork copies: code-and-context, code (only), or
        /// context (only).
        #[arg(long, alias = "copy", value_enum)]
        kind: ForkKind,
        /// The fork's agent (default: the parent's).
        #[arg(long)]
        agent: Option<String>,
        /// Fork from just before this turn (1 is the first prompt, so
        /// nothing comes before it). Default: from now.
        #[arg(long, value_parser = clap::value_parser!(u32).range(2..))]
        before_turn: Option<u32>,
        #[arg(long)]
        title: Option<String>,
        /// The branch the fork's clones are made on (default: one named
        /// from the title and the branch prefix).
        #[arg(long)]
        branch: Option<String>,
    },
    /// Print a session's turns.
    Read {
        session: String,
        /// First turn to read (default: the first).
        #[arg(long = "from")]
        from: Option<u32>,
        /// Last turn to read (default: the last).
        #[arg(long = "to")]
        to: Option<u32>,
    },
    Rename { session: String, title: String },
    /// Stop a session's agent, or with `--tab` a shell tab (a session's,
    /// a machine's own, or one no channel has) on `--machine`; either can
    /// be resumed (D784).
    Stop {
        #[arg(required_unless_present = "tab")]
        session: Option<String>,
        /// A shell's tab, as `session running` lists it.
        #[arg(long)]
        tab: Option<String>,
    },
    /// Resume a stopped session, or with `--tab` a stopped shell tab.
    Resume {
        #[arg(required_unless_present = "tab")]
        session: Option<String>,
        /// A stopped shell's tab, as `session running` lists it.
        #[arg(long, conflicts_with = "fresh")]
        tab: Option<String>,
        /// Start over with a summary instead of resuming the conversation.
        #[arg(long)]
        fresh: bool,
    },
    /// End a session for good: it is archived, by the plan. With `--tab`, a
    /// shell tab ends instead; nothing is archived. Neither can be resumed.
    End {
        #[arg(required_unless_present = "tab")]
        session: Option<String>,
        /// A shell's tab, as `session running` lists it.
        #[arg(long)]
        tab: Option<String>,
    },
    /// Every terminal of ours on `--machine` (default: this one): running
    /// sessions, stopped ones, and those no channel has.
    Running,
    /// Put a terminal no channel holds (see `session running`) into
    /// `--channel`, or the Desk without it, on `--machine`. It keeps running.
    Adopt {
        /// The terminal's tab, as `session running` lists it.
        tab: String,
    },
    Archive {
        session: String,
        /// Commit, push and open PRs first.
        #[arg(long, conflicts_with = "delete_work")]
        push_first: bool,
        #[arg(long)]
        delete_work: bool,
    },
    Delete { session: String },
    Restore {
        session: String,
        /// Bring it back stopped, as End for good's undo does.
        #[arg(long)]
        stopped: bool,
    },
    Rebuild { session: String },
    Move {
        session: String,
        #[arg(long)]
        to: String,
    },
    #[command(subcommand)]
    Repo(SessionRepoCommand),
    /// Give a context-only fork its own copy of its parent's clones (D185).
    OwnCopy { session: String },
    /// Start a new session in the channel given by --channel, whose agent
    /// goes on from a copy of this session's conversation (CV-8). The
    /// session itself is untouched.
    Continue {
        session: String,
        /// Go on from just before this turn (1 is the first prompt, so
        /// nothing comes before it). Default: the whole conversation.
        #[arg(long, value_parser = clap::value_parser!(u32).range(2..))]
        before_turn: Option<u32>,
    },
    Push {
        session: String,
        #[arg(long)]
        repo: Vec<String>,
        /// Open a pull request for each repo pushed.
        #[arg(long)]
        pr: bool,
        #[arg(long)]
        from_view: Option<String>,
        #[arg(long)]
        message: Option<String>,
    },
    Note { session: String, text: String },
    Ask { session: String, text: String },
    /// Answer a question another session typed into yours (its `qu_` id);
    /// the answer is what its ask returns. --decline says no instead.
    Answer {
        question: String,
        #[arg(required_unless_present = "decline")]
        answer: Option<String>,
        #[arg(long, conflicts_with = "answer")]
        decline: bool,
    },
    /// Type text into a session and press Enter. Use --no-submit to leave it unsent.
    Type {
        session: String,
        text: String,
        /// Type the text but do not press Enter; the agent does not receive it until Enter is pressed.
        #[arg(long)]
        no_submit: bool,
        #[arg(long)]
        at_turn_end: bool,
        #[arg(long)]
        tab: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum SessionRepoCommand {
    Add { session: String, repo: String },
    Drop { session: String, repo: String },
}

#[derive(Clone, Debug, Default, Args)]
#[group(multiple = false)]
pub struct BoardWhich {
    /// The global board.
    #[arg(long)]
    pub global: bool,
    /// A session's board.
    #[arg(long)]
    pub session: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum BoardCommand {
    Add {
        title: String,
        #[arg(long, value_enum)]
        column: Option<Column>,
        #[arg(long)]
        description: Option<String>,
        /// A session working on it; repeat for more.
        #[arg(long, value_name = "SESSION")]
        assign: Vec<String>,
        /// An item it depends on, on any board; repeat for more.
        #[arg(long, value_name = "ITEM")]
        depends_on: Vec<String>,
    },
    Edit {
        item: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        description: Option<String>,
        /// Add a session; repeat for more.
        #[arg(long, value_name = "SESSION")]
        assign: Vec<String>,
        /// Remove a session; repeat for more.
        #[arg(long, value_name = "SESSION")]
        unassign: Vec<String>,
        /// Add an item it depends on; repeat for more.
        #[arg(long, value_name = "ITEM")]
        depends_on: Vec<String>,
        /// Remove an item it depends on; repeat for more.
        #[arg(long, value_name = "ITEM")]
        drop_depends_on: Vec<String>,
        /// The item's revision as you read it (`humata board`); the edit is
        /// refused if a field it sets changed since.
        #[arg(long, value_name = "N")]
        revision: Option<u64>,
    },
    Move {
        item: String,
        #[arg(value_enum)]
        column: Column,
        #[arg(long, value_name = "ITEM")]
        after: Option<String>,
        /// The item's revision as you read it (`humata board`); the move is
        /// refused if the item's column changed since.
        #[arg(long, value_name = "N")]
        revision: Option<u64>,
    },
    Rm {
        item: String,
        /// The item's revision as you read it (`humata board`); the delete
        /// is refused if a field it would remove changed since.
        #[arg(long, value_name = "N")]
        revision: Option<u64>,
    },
    History { item: String },
}

#[derive(Debug, Subcommand)]
pub enum DiffCommand {
    MarkReviewed {
        session: String,
        paths: Vec<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum CommentCommand {
    /// Comment on lines: `<owner/repo>:<path>:<first>-<last>`.
    Add {
        session: String,
        at: String,
        text: String,
    },
    /// Send draft comments to the session's agent.
    Send { session: String, ids: Vec<String> },
    Ls { session: String },
    Rm { comment: String },
}

#[derive(Debug, Subcommand)]
pub enum ReviewCommand {
    /// Review pull requests (URLs or owner/repo#n) in a session on --machine.
    Start {
        #[arg(required = true)]
        prs: Vec<String>,
        #[arg(long)]
        agent: Option<String>,
        #[arg(long)]
        message: Option<String>,
        #[arg(long)]
        allow_push: bool,
    },
    Drafts { session: String },
    Submit {
        session: String,
        #[arg(long = "as", value_enum)]
        verdict: Verdict,
        #[arg(long)]
        summary: Option<String>,
    },
    AllowPush {
        session: String,
        #[arg(value_enum)]
        state: OnOff,
    },
}

#[derive(Debug, Subcommand)]
pub enum InboxCommand {
    Show { request: String },
    Approve {
        request: String,
        #[arg(long)]
        note: Option<String>,
        /// Allow it from now on, as the request offers.
        #[arg(long)]
        always: bool,
    },
    Deny {
        request: String,
        #[arg(long)]
        note: Option<String>,
    },
    Reply { request: String, text: String },
    /// Approve every pending request listed.
    ApproveAll,
    DenyAll,
}

#[derive(Debug, Subcommand)]
pub enum AccessCommand {
    Set {
        capability: String,
        #[arg(value_enum)]
        decision: Decision,
    },
    /// What agents may do while the Mac is away.
    Away {
        #[arg(value_enum)]
        mode: Away,
        /// Capabilities counted as low risk.
        #[arg(long, num_args = 1..)]
        kinds: Vec<String>,
    },
    Grants,
    Revoke { grant: String },
    /// Why a session may or may not do something.
    Why {
        session: String,
        capability: String,
        target: Option<String>,
    },
    /// Set the approval passphrase, typed or generated.
    Passphrase {
        #[arg(long)]
        generate: bool,
    },
    /// Ask now for access needed later (agents).
    AskAhead {
        capability: String,
        target: String,
        #[arg(long)]
        reason: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum TokenCommand {
    Create {
        #[arg(long)]
        name: String,
        #[arg(long = "capability", required = true)]
        capabilities: Vec<String>,
        #[arg(long, default_value_t = 30)]
        days: u32,
    },
    Ls,
    Revoke { token: String },
}

#[derive(Debug, Subcommand)]
pub enum FilesCommand {
    Ls { location: String },
    Cat { location: String },
    /// Files in a session's clones whose names match.
    Find { session: String, query: String },
    Put {
        local: std::path::PathBuf,
        location: String,
        #[arg(long)]
        replace: bool,
    },
    Mkdir { location: String },
    /// Rename in place.
    Mv { location: String, name: String },
    Rm {
        #[arg(required = true)]
        locations: Vec<String>,
    },
}

#[derive(Debug, Args)]
pub struct TransferArgs {
    /// Sources, then the destination: a location, or only a machine, where
    /// the files land by rule (PL-3, PL-12).
    #[arg(required = true, num_args = 2..)]
    pub locations: Vec<String>,
    /// Unset: fail for a location, keep both for a machine only (PL-13).
    #[arg(long, value_enum)]
    pub on_existing: Option<OnExisting>,
}

#[derive(Debug, Subcommand)]
pub enum TransferCommand {
    Pause { id: String },
    Resume { id: String },
    Cancel { id: String },
}

#[derive(Debug, Subcommand)]
pub enum TrashCommand {
    Restore { item: String },
    Empty,
}

#[derive(Debug, Subcommand)]
pub enum HostCommand {
    Ls,
    Show { machine: String },
    Add {
        ssh_host: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        root: Option<String>,
        /// The host's key fingerprint, as the first add printed it, once
        /// you have checked it is the host's.
        #[arg(long, value_name = "FINGERPRINT")]
        trust_host_key: Option<String>,
    },
    Reconnect { machine: String },
    Remove { machine: String },
    Forget { machine: String },
    /// Print where a machine keeps Humata's folders; with a path (or
    /// `--default`) change it.
    Root {
        machine: String,
        /// The new root. Leave it out to print the current one.
        path: Option<String>,
        /// Reset the root to ~/Humata.
        #[arg(long = "default", conflicts_with_all = ["path", "move_channels"])]
        default_root: bool,
        #[arg(long)]
        move_channels: bool,
    },
    VmTier {
        machine: String,
        #[arg(value_enum)]
        state: OnOff,
    },
    Storage { machine: String },
    /// Clear a build cache folder in a session's clone.
    ClearBuildCache {
        session: String,
        path: String,
        /// Even while the session runs.
        #[arg(long)]
        confirm_running: bool,
    },
    TrimLogs { machine: String },
    /// Stop every session on a machine, as a last resort: agents and shells
    /// end gracefully and everything a restart needs is saved. They stay
    /// stopped, across reboots, until `humata host start`.
    Stop {
        machine: String,
        /// Why, shown on the machine's page.
        #[arg(long)]
        reason: Option<String>,
    },
    /// Start a machine's sessions again, or cancel a stop it never got.
    Start { machine: String },
    /// The names of a machine's variables (never their values).
    Vars { machine: String },
    #[command(subcommand)]
    Var(VarCommand),
    TrustKey {
        machine: String,
        #[arg(long)]
        fingerprint: String,
    },
    /// Scripts that run on machines when they link up or a session moves
    /// (the rules are in ~/.humata2/config/machine-scripts.toml).
    #[command(subcommand)]
    Script(ScriptCommand),
}

/// SEAM (MACHINESCRIPTS, D687): nothing here writes rules or scripts; they
/// change only in the owner's files.
#[derive(Debug, Subcommand)]
pub enum ScriptCommand {
    /// The rules, their triggers, and each machine's last outcome.
    List {
        /// Only this machine's last runs.
        machine: Option<String>,
    },
    /// Check the rules file and the script names; exits 2 when invalid.
    Check,
    /// Run a rule on a machine now. `--wait` follows it to the end.
    Run {
        rule: String,
        machine: String,
        /// Run as this session's trigger (a session-arrived or
        /// session-leaving rule).
        #[arg(long)]
        session: Option<String>,
    },
    /// The host's run log for a machine, newest first.
    Log {
        machine: String,
        #[arg(long)]
        rule: Option<String>,
        #[arg(long)]
        limit: Option<u32>,
    },
}

#[derive(Debug, Subcommand)]
pub enum VarCommand {
    /// Set a variable; the value is read with echo off, or from stdin.
    Set {
        machine: String,
        name: String,
        /// Not taken: a value on the command line would stay in the
        /// shell's history and show in the process list (HO-13). Anything
        /// given here is refused without being repeated.
        #[arg(hide = true, num_args = 0.., value_name = "VALUE")]
        value_on_command_line: Vec<String>,
    },
    Rm { machine: String, name: String },
}

/// The provider file's format, after `provider import`'s long help.
const PROVIDER_FILE_HELP: &str = concat!(
    "The provider file\n",
    "\n",
    "Each provider is a `[providers.<id>]` table; the id is how commands name it\n",
    "(`humata provider create <id> <name>`). Its keys:\n",
    "\n",
    "  label      The name shown in Add machine. Defaults to the id.\n",
    "  params     The fields Add machine asks for: a list of\n",
    "             { name = \"...\", prompt = \"...\", default = \"...\" }. prompt and\n",
    "             default are optional; a param with no default must be given.\n",
    "  create     Required. Makes the machine.\n",
    "  connect    Runs a command on the machine: it must contain {cmd}, which\n",
    "             Humata replaces with the command, quoted for `sh -c`.\n",
    "  identity   Instead of connect: prints the machine's SSH details as JSON.\n",
    "             A provider has exactly one of connect and identity.\n",
    "  start      Optional. Starts a stopped machine.\n",
    "  stop       Optional. Stops it.\n",
    "  delete     Optional. Deletes what create made.\n",
    "  status     Optional. Prints the machine's state (Running, Stopped, ...).\n",
    "  env        Optional. Names of this Mac's host variables the commands get,\n",
    "             by the same name (set them in Settings > This Mac > Variables).\n",
    "\n",
    "Each command is run with `sh -c` on this Mac. {name} in any command is the\n",
    "machine's name; {cmd} may appear only in connect. Every command also gets\n",
    "HUMATA_NAME, and each param as $HUMATA_<NAME> (the param's name in upper\n",
    "case): `cpus` arrives as $HUMATA_CPUS. Use the variables in scripts rather\n",
    "than pasting values into the command text.\n",
    "\n",
    "Example: the Lima provider Humata ships\n",
    "\n",
    include_str!(env!("HUMATA2_LIMA_PROVIDER")),
);

#[derive(Debug, Subcommand)]
pub enum ProviderCommand {
    Ls,
    #[command(subcommand)]
    Config(ProviderConfigCommand),
    /// Add the providers in a TOML file.
    ///
    /// The user sees the whole file and every command it will run before
    /// anything is added. An agent's import is a request that waits for the
    /// user in the Inbox.
    #[command(after_long_help = PROVIDER_FILE_HELP)]
    Import {
        file: std::path::PathBuf,
        /// Why the task needs this provider. An agent must give one: the
        /// user reads it on the Inbox card.
        #[arg(long, default_value = "")]
        reason: String,
    },
    /// Remove an imported provider by its id.
    ///
    /// A built-in provider can't be removed, and neither can one that
    /// machines still use: delete those machines first. Only the user can
    /// remove a provider; an agent is told to ask them.
    Rm { provider: String },
    /// Make a machine with a provider.
    Create {
        provider: String,
        name: String,
        #[arg(long = "param", value_name = "KEY=VALUE")]
        params: Vec<String>,
    },
    Start { machine: String },
    Stop { machine: String },
    Status { machine: String },
    Delete { machine: String },
    /// Log in to a provider's tool interactively.
    Login { machine: String },
}

#[derive(Debug, Subcommand)]
pub enum ProviderConfigCommand {
    Show,
    /// Replace the providers' TOML with a file's, or stdin's.
    Edit {
        #[arg(long)]
        file: Option<std::path::PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
pub enum SkillsCommand {
    /// Copy skills to machines (all of them when none is named).
    Sync { machines: Vec<String> },
}

#[derive(Debug, Subcommand)]
pub enum BridgeCommand {
    Ls {
        #[arg(long)]
        session: Option<String>,
    },
    Close { forward: String },
}

#[derive(Debug, Subcommand)]
pub enum SettingsCommand {
    Get { key: Option<String> },
    Set { key: String, value: String },
    /// Contain agents in new and resumed sessions.
    Contain {
        #[arg(value_enum)]
        state: OnOff,
        /// With --channel: use the global setting again.
        #[arg(long, requires = "channel")]
        inherit: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum AgentsCommand {
    Ls,
    Add {
        name: String,
        #[command(flatten)]
        fields: AgentFields,
    },
    Edit {
        agent: String,
        #[command(flatten)]
        fields: AgentFields,
        /// Whether this agent's own questions go to the Inbox.
        #[arg(long, value_name = "true|false")]
        asks_in_inbox: Option<bool>,
    },
}

#[derive(Debug, Default, Args)]
pub struct AgentFields {
    #[arg(long)]
    pub launch: Option<String>,
    #[arg(long)]
    pub resume_args: Option<String>,
    #[arg(long)]
    pub fork_args: Option<String>,
    #[arg(long = "model")]
    pub models: Vec<String>,
    #[arg(long = "variable")]
    pub variables: Vec<String>,
}

impl AgentFields {
    /// Whether any field was given.
    pub fn any(&self) -> bool {
        self.launch.is_some()
            || self.resume_args.is_some()
            || self.fork_args.is_some()
            || !self.models.is_empty()
            || !self.variables.is_empty()
    }
}

#[derive(Debug, Subcommand)]
pub enum PinnedCommand {
    Reorder {
        #[arg(required = true)]
        things: Vec<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum DoctorCommand {
    /// Read a restart report, say what wasn't brought back and what is
    /// stuck, repair what can't lose anything and write diagnosis.md beside
    /// the report.
    Restarts(RestartsArgs),
}

#[derive(Debug, Args)]
pub struct RestartsArgs {
    /// A report's folder, or its name in restarts/. Default: the newest.
    pub report: Option<String>,
    /// Only read and say: change nothing, and work with the daemon down.
    #[arg(long)]
    pub explain: bool,
    /// Also run this repair, which can lose something (an id from
    /// --explain, such as start-fresh:<session>). Repeat for more.
    #[arg(long, value_name = "ID")]
    pub allow: Vec<String>,
    /// The diagnosis you wrote, as a file or `-` for stdin, put at the top
    /// of diagnosis.md.
    #[arg(long, value_name = "FILE")]
    pub diagnosis: Option<String>,
    /// The state folder humatad2 serves from. Default: the one beside the
    /// daemon's socket.
    #[arg(long, value_name = "DIR")]
    pub state: Option<std::path::PathBuf>,
}

#[derive(Debug, Subcommand)]
pub enum ReportCommand {
    /// Start recording what you do, for a bug report.
    Record {
        #[arg(long)]
        title: Option<String>,
    },
    /// Stop recording and save the report.
    Stop,
    /// Save what's on screen now as a report.
    Snapshot {
        #[arg(long)]
        title: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Column {
    Backlog,
    #[value(alias = "todo")]
    ToDo,
    InProgress,
    InReview,
    Done,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum ForkKind {
    CodeAndContext,
    #[value(alias = "code")]
    CodeOnly,
    #[value(alias = "context")]
    ContextOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum OnOff {
    On,
    Off,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum ViewAction {
    Open,
    Later,
    NotNow,
    Revert,
    Keep,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum TurnArg {
    Working,
    Idle,
    NeedsInput,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum HookEventArg {
    Permission,
    /// The agent's own ask-user form (D761).
    Question,
    /// Which conversation the agent's CLI is in now, so a `/clear` that
    /// starts another one is the one a resume takes up.
    Conversation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Verdict {
    Comment,
    Approve,
    RequestChanges,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Decision {
    Allow,
    Ask,
    Never,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Away {
    Wait,
    Skip,
    LowRisk,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum OnExisting {
    Fail,
    Replace,
    KeepBoth,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Topic {
    Sessions,
    Channels,
    Machines,
    Boards,
    Inbox,
    Access,
    Comments,
    Views,
    Operations,
    Transfers,
    Activity,
    Marks,
    Settings,
    Journal,
}
