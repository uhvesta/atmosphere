//! `humata`: Humata's sessions from the command line, through the daemon.

#[cfg(feature = "admin")]
mod admin;
mod approved;
mod attach;
mod control;
mod error;
mod print;

use std::collections::HashSet;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use humata_wire::pb;

use crate::approved::{Client, Wait};
use tokio::net::UnixStream;

use crate::error::CliError;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// How long `humata mcp` waits for the person to approve an admin action
/// before telling the agent it still waits, so a turn isn't held up long.
const MCP_APPROVAL_WAIT: std::time::Duration = std::time::Duration::from_secs(20);

#[derive(Parser)]
#[command(
    name = "humata",
    version,
    about = "Direct Humata's coding agents from the command line."
)]
struct Arguments {
    /// The daemon's socket; defaults to $HUMATA_SOCKET, then the path humatad
    /// uses.
    #[arg(long, global = true)]
    socket: Option<PathBuf>,
    /// Don't wait for an action that needs your approval in Humata, such as
    /// adding a task: print its approval's id and stop, leaving it to run
    /// once approved.
    #[arg(long, global = true)]
    no_wait: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, ValueEnum)]
enum Backend {
    Scripted,
    Local,
    Copilot,
    Claude,
}

impl From<Backend> for pb::Backend {
    fn from(backend: Backend) -> Self {
        match backend {
            Backend::Scripted => Self::Scripted,
            Backend::Local => Self::Local,
            Backend::Copilot => Self::Copilot,
            Backend::Claude => Self::Claude,
        }
    }
}

/// Which agent's plugin.
#[derive(Clone, Copy, ValueEnum)]
enum PluginAgent {
    Claude,
    Copilot,
}

/// A project's choice for a plugin.
#[derive(Clone, Copy, ValueEnum)]
enum PluginChoice {
    /// On for the project's sessions.
    On,
    /// Off for the project's sessions.
    Off,
    /// However its CLI has it.
    Default,
}

/// What a session may do without asking.
#[derive(Clone, Copy, ValueEnum)]
enum Approval {
    /// Ask before every command, write and read.
    Ask,
    /// Reads and file writes go ahead; commands still ask.
    Edits,
    /// Everything goes ahead.
    Everything,
    /// The agent's own judge lets through what it finds safe; the rest asks.
    /// Claude Code and Copilot only.
    Auto,
}

impl From<Approval> for pb::ApprovalMode {
    fn from(approval: Approval) -> Self {
        match approval {
            Approval::Ask => Self::AskEveryTime,
            Approval::Edits => Self::AllowEdits,
            Approval::Everything => Self::AllowEverything,
            Approval::Auto => Self::Auto,
        }
    }
}

#[derive(Subcommand)]
enum ImportCommand {
    /// List them, newest first: backend, id, when, folder and first message.
    List {
        /// The host to look on; this machine if not given.
        #[arg(long, default_value = "")]
        host: String,
    },
    /// Import one, with its history, and print the new session's id.
    Session {
        #[arg(long, value_enum)]
        backend: Backend,
        /// Its id, as `humata import list` prints it.
        id: String,
        #[arg(long, default_value = "")]
        host: String,
        /// Named after its first message if not given.
        #[arg(long, default_value = "")]
        name: String,
        #[arg(long, default_value = "")]
        project: String,
    },
}

#[derive(Subcommand)]
enum ProjectCommand {
    /// Add a project of repositories, and print its id.
    Add {
        #[arg(long)]
        name: String,
        /// A repository: its folder on this Mac or its URL, which this Mac
        /// clones, as `PATH`, `URL`, `NAME=PATH` or `NAME=URL`; give it once
        /// for each. NAME is its folder in each session's copy, the folder's
        /// or URL's own name if not given.
        #[arg(long = "repository", required = true)]
        repositories: Vec<String>,
        /// Instructions for every session in it.
        #[arg(long, default_value = "")]
        instructions: String,
        /// A skill every session in it reads, as `NAME=FILE` of Markdown;
        /// give it once for each.
        #[arg(long = "skill")]
        skills: Vec<String>,
    },
    /// Change a project's name, instructions, skills or repositories.
    Update {
        project: String,
        #[arg(long)]
        name: Option<String>,
        /// Replaces every repository, for sessions started after, given as
        /// for `project add`; give it once for each.
        #[arg(long = "repository")]
        repositories: Vec<String>,
        #[arg(long)]
        instructions: Option<String>,
        /// Replaces every skill, as `NAME=FILE`; give it once for each.
        #[arg(long = "skill")]
        skills: Vec<String>,
        /// Removes every skill.
        #[arg(long, conflicts_with = "skills")]
        no_skills: bool,
    },
    /// List the projects.
    List,
    /// Delete a project and its board. Archive its sessions first; they are
    /// kept, in no project.
    Delete { project: String },
}

/// Where an item is on the board.
#[derive(Clone, Copy, ValueEnum)]
enum Status {
    Todo,
    InProgress,
    WaitingOnYou,
    InReview,
    Done,
    WontDo,
}

impl From<Status> for pb::ItemStatus {
    fn from(status: Status) -> Self {
        match status {
            Status::Todo => Self::Todo,
            Status::InProgress => Self::InProgress,
            Status::WaitingOnYou => Self::WaitingOnYou,
            Status::InReview => Self::InReview,
            Status::Done => Self::Done,
            Status::WontDo => Self::WontDo,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum Priority {
    None,
    Low,
    Medium,
    High,
    Urgent,
}

impl From<Priority> for pb::Priority {
    fn from(priority: Priority) -> Self {
        match priority {
            Priority::None => Self::None,
            Priority::Low => Self::Low,
            Priority::Medium => Self::Medium,
            Priority::High => Self::High,
            Priority::Urgent => Self::Urgent,
        }
    }
}

#[derive(Subcommand)]
enum ItemCommand {
    /// Add a task, or a subtask with --parent, and print its id.
    Add {
        #[arg(long)]
        project: String,
        /// The task to add a subtask under.
        #[arg(long, default_value = "")]
        parent: String,
        title: String,
        /// A Markdown file of its spec; `-` reads it from stdin.
        #[arg(long)]
        spec: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = Status::Todo)]
        status: Status,
        #[arg(long, value_enum, default_value_t = Priority::None)]
        priority: Priority,
    },
    /// Change a task or subtask.
    Edit {
        item: String,
        #[arg(long)]
        title: Option<String>,
        /// A Markdown file of its spec; `-` reads it from stdin.
        #[arg(long)]
        spec: Option<PathBuf>,
        #[arg(long, value_enum)]
        status: Option<Status>,
        #[arg(long, value_enum)]
        priority: Option<Priority>,
    },
    /// Set a task's or subtask's status. While its sessions are busy, the
    /// board shows theirs instead, unless it is Done or Won't do.
    Move {
        item: String,
        #[arg(value_enum)]
        status: Status,
    },
    /// List a project's tasks, each followed by its subtasks.
    List {
        #[arg(long)]
        project: String,
    },
    /// Print an item's spec.
    Spec {
        item: String,
        #[arg(long)]
        project: String,
    },
}

/// A file of a session, by its repository's name there and its path in it.
#[derive(clap::Args)]
struct FileArgs {
    session: String,
    repository: String,
    path: String,
}

#[derive(clap::Args)]
struct EditArgs {
    #[command(flatten)]
    file: FileArgs,
    /// Only the hunk starting at these lines, before and after, as
    /// `OLD:NEW` from `humata changes`; the whole file if unset.
    #[arg(long, value_parser = hunk_at)]
    hunk: Option<(u32, u32)>,
    /// The context the hunk was read with.
    #[arg(long, default_value_t = 3)]
    context: u32,
}

fn hunk_at(text: &str) -> Result<(u32, u32), String> {
    let (old, new) = text
        .split_once(':')
        .ok_or_else(|| format!("{text:?} isn't OLD:NEW, such as 12:14"))?;
    let number = |part: &str| {
        part.trim()
            .parse::<u32>()
            .map_err(|_| format!("{part:?} isn't a line number"))
    };
    Ok((number(old)?, number(new)?))
}

#[derive(Subcommand)]
enum ReviewCommand {
    /// Comment on lines of a file as it is in the session's copy, and print
    /// the new thread's id. It is sent with the next review.
    Comment {
        #[command(flatten)]
        file: FileArgs,
        line: u32,
        /// The last line, if more than one.
        #[arg(long)]
        end: Option<u32>,
        /// The text picked in rendered Markdown.
        #[arg(long, default_value = "")]
        quote: String,
        text: String,
    },
    /// Add a comment to a thread, sent with the next review.
    Reply { thread: String, text: String },
    /// Resolve a thread.
    Resolve {
        thread: String,
        /// Open it again instead.
        #[arg(long)]
        undo: bool,
    },
    /// Delete a comment not sent yet.
    Delete { comment: String },
    /// Print a session's threads, with your comments and the agent's
    /// replies.
    Threads { session: String },
    /// Send your comments not sent yet to the agent, as one message.
    Send { session: String },
    /// Mark a file viewed as it is now; it stays viewed until it changes.
    Viewed {
        #[command(flatten)]
        file: FileArgs,
        /// Mark it not viewed instead.
        #[arg(long)]
        undo: bool,
    },
}

#[derive(Subcommand)]
enum BoardCommand {
    /// Print a project's board changes, newest first.
    Log {
        #[arg(long)]
        project: String,
    },
    /// Put an item back as it was before a change.
    Revert { change: u64 },
}

#[derive(Subcommand)]
enum HostCommand {
    /// Add a host by what `ssh` connects to; Humata installs humatad there
    /// and keeps it up to date.
    Add {
        name: String,
        /// A Host in your SSH config, or user@machine.
        destination: String,
        /// An SSH config file other than ~/.ssh/config.
        #[arg(long)]
        ssh_config: Option<PathBuf>,
        /// The folder on the host for humatad and its sessions, relative to
        /// the home folder there unless absolute.
        #[arg(long, default_value = "")]
        folder: String,
    },
    /// List the hosts.
    List,
    /// Remove a host. Its sessions stay on it, but Humata forgets them.
    Remove { name: String },
    /// Say whether a host is reachable, and how it is set up.
    Check { name: String },
    /// Copy humatad to a host: this one, or --binary for another platform.
    Install {
        name: String,
        #[arg(long)]
        binary: Option<PathBuf>,
    },
    /// Set or unset variables for a host's agents. Values are sent to the
    /// host when it connects and kept only in its memory there.
    Env {
        name: String,
        /// NAME=VALUE. It ends up in your shell's history; prefer --copy for
        /// secrets.
        #[arg(long = "set", value_name = "NAME=VALUE")]
        set: Vec<String>,
        /// Copies a variable from this shell, such as GH_TOKEN.
        #[arg(long = "copy", value_name = "NAME")]
        copy: Vec<String>,
        #[arg(long = "unset", value_name = "NAME")]
        unset: Vec<String>,
    },
    /// Set the script run on a host when it changes, or until it succeeds:
    /// a file, or - for stdin. An empty file removes it.
    Setup { name: String, script: PathBuf },
    /// List the hosts in ~/.ssh/config, to add.
    SshHosts,
}

#[derive(Subcommand)]
enum RepositoryCommand {
    /// List the saved repositories to pick from for a project, last used
    /// first.
    List,
    /// Rename a saved repository in the list; no project changes.
    Rename { id: i64, name: String },
    /// Take a saved repository out of the list; projects that have it
    /// keep it.
    Remove { id: i64 },
    /// The GitHub users and organizations whose repositories are saved.
    #[command(subcommand)]
    Owner(OwnerCommand),
}

#[derive(Subcommand)]
enum OwnerCommand {
    /// List the owners, with how many repositories each has saved and why
    /// the last listing failed, if it did.
    List,
    /// Save a user's or organization's repositories, as `octo-org`,
    /// `@octo-org` or `https://github.com/octo-org`; they are listed on
    /// GitHub in the background, with This Mac's token.
    Add { login: String },
    /// Remove an owner and its repositories from the list; those a project
    /// used stay.
    Remove { login: String },
    /// List an owner's repositories on GitHub again now, or every owner's.
    Refresh { login: Option<String> },
}

#[derive(Subcommand)]
enum RecoverCommand {
    /// List the Mac's recovery copies of sessions on hosts, and which ones
    /// their host lost.
    List,
    /// Set a session up again from its recovery copy: on its host, or with
    /// --host on another, which it runs on from then on.
    Session {
        session: String,
        /// The host to restore it on; "" for this Mac.
        #[arg(long)]
        host: Option<String>,
    },
    /// Set up again every session a host lost.
    Host { name: String },
}

/// Where to browse: a session's folder, or a folder on a host.
#[derive(clap::Args)]
struct FolderPlace {
    /// The session whose folder to browse, on its host.
    #[arg(long)]
    session: Option<String>,
    /// The host, when no session is given; this Mac when left out.
    #[arg(long, default_value = "")]
    host: String,
    /// An absolute folder on the host to browse from, instead of its home.
    #[arg(long)]
    root: Option<String>,
}

#[derive(Subcommand)]
enum ApprovalCommand {
    /// The approvals pending and recent, newest first, then the grants in
    /// force.
    List,
    /// How an approval stands.
    Status { id: String },
    /// Wait until the person decides an approval, then print what the
    /// action answered.
    Wait { id: String },
}

#[derive(Subcommand)]
enum ProposalCommand {
    /// A session's proposals, oldest first, or with --project what agents
    /// outside Humata proposed for its board through `humata mcp`.
    List {
        #[arg(required_unless_present = "project")]
        session: Option<String>,
        #[arg(long, conflicts_with = "session")]
        project: Option<String>,
    },
    /// Add the proposed task, or change the item's spec, and print the
    /// item's id.
    Apply {
        /// The session that proposed it, or "" for an outside agent.
        session: String,
        proposal: String,
        /// Use this title instead of the proposed one.
        #[arg(long)]
        title: Option<String>,
        /// Use the spec in this file instead of the proposed one.
        #[arg(long)]
        spec: Option<PathBuf>,
    },
    Decline {
        /// The session that proposed it, or "" for an outside agent.
        session: String,
        proposal: String,
    },
}

#[derive(Subcommand)]
enum PortCommand {
    /// Listen on 127.0.0.1 here for a port on a host, such as a dev
    /// server's, and print the forward with the port to open here.
    Forward {
        port: u32,
        /// The host the port is on; this Mac when left out.
        #[arg(long, default_value = "")]
        host: String,
        /// The session the server belongs to, shown with the forward.
        #[arg(long, default_value = "")]
        session: String,
    },
    /// List the ports being forwarded.
    List,
    /// Stop forwarding; connections already open run until they close.
    Stop { id: String },
}

#[derive(Subcommand)]
enum TerminalCommand {
    /// Open a shell in a session's folder, a folder on a host, or the
    /// host's home, and print the terminal.
    Open {
        /// Opens in this session's folder, on its host.
        #[arg(long)]
        session: Option<String>,
        /// The host, when no session is given; this Mac when left out.
        #[arg(long, default_value = "")]
        host: String,
        /// A folder on the host to open in, instead of its home.
        #[arg(long)]
        folder: Option<String>,
    },
    /// List a host's terminals, running and exited.
    List {
        #[arg(long, default_value = "")]
        host: String,
    },
    /// Type a line into a terminal, then Enter.
    Type {
        terminal: String,
        text: String,
        #[arg(long, default_value = "")]
        host: String,
        /// Type the text as given, without pressing Enter after it.
        #[arg(long)]
        raw: bool,
    },
    /// Print what a terminal shows: its recent output, then what it prints
    /// until --until's text appears, it exits, or --timeout passes.
    Read {
        terminal: String,
        #[arg(long, default_value = "")]
        host: String,
        #[arg(long)]
        until: Option<String>,
        /// Seconds to wait for --until's text.
        #[arg(long, default_value_t = 10)]
        timeout: u64,
    },
    /// Change a terminal's size.
    Resize {
        terminal: String,
        rows: u32,
        columns: u32,
        #[arg(long, default_value = "")]
        host: String,
    },
    /// End a terminal's shell and forget it.
    Close {
        terminal: String,
        #[arg(long, default_value = "")]
        host: String,
    },
    /// Be the terminal: what's typed here goes to its shell, and what it
    /// prints shows here, until it exits. For a terminal app to run; exits
    /// with the shell's exit code.
    Attach {
        terminal: String,
        #[arg(long, default_value = "")]
        host: String,
    },
}

#[derive(Subcommand)]
enum Command {
    /// Open, type into, read and close shells on a host.
    #[command(subcommand)]
    Terminal(TerminalCommand),
    /// Forward a port on a host to 127.0.0.1 here, list or stop forwards.
    #[command(subcommand)]
    Port(PortCommand),
    /// List, apply or decline the tasks and spec edits a session's agent
    /// proposed.
    #[command(subcommand)]
    Proposal(ProposalCommand),
    /// The admin actions asked for from here or by agents, which wait for
    /// your approval in Humata, and the grants that let them run.
    #[command(subcommand)]
    Approval(ApprovalCommand),
    /// List or import the Claude Code and Copilot sessions on a host that
    /// were started outside Humata.
    #[command(subcommand)]
    Import(ImportCommand),
    /// Add, check, set up or list the machines sessions run on.
    #[command(subcommand)]
    Host(HostCommand),
    /// List, rename or remove the saved repositories to pick from for a
    /// project, and the GitHub owners whose repositories are among them.
    #[command(subcommand)]
    Repository(RepositoryCommand),
    /// Restore sessions a host lost, or move them to another, from the
    /// copies the Mac keeps.
    #[command(subcommand)]
    Recover(RecoverCommand),
    /// Add or list projects.
    #[command(subcommand)]
    Project(ProjectCommand),
    /// Add, change or list a project's tasks and subtasks.
    #[command(subcommand)]
    Item(ItemCommand),
    /// Read or revert a project's board changes.
    #[command(subcommand)]
    Board(BoardCommand),
    /// List every session waiting on you, across projects, and what agents
    /// ask of you.
    Inbox,
    /// Search what every session said and did, and the review comments and
    /// specs: hits best first, each with its session, turn and event, to
    /// read with `humata history` or fork from. A time in the words, such as
    /// `last week` or `past 3 days`, narrows to then.
    Search {
        #[arg(required = true)]
        words: Vec<String>,
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        item: Option<String>,
        #[arg(long)]
        session: Option<String>,
        /// The most hits; 50 if not given.
        #[arg(long)]
        limit: Option<u32>,
    },
    /// Commit a session's work onto a branch in each repository it changed,
    /// and with --pull-requests, push them and open a pull request for each.
    Land {
        session: String,
        /// The branch to land on, new in each repository.
        #[arg(long)]
        branch: String,
        /// The commit message; its first line titles the pull requests.
        #[arg(long, short)]
        message: String,
        /// Push the branches to their origins.
        #[arg(long)]
        push: bool,
        /// Push and open or find a pull request for each branch on GitHub.
        #[arg(long)]
        pull_requests: bool,
    },
    /// List a session's or a task's pull requests, read again from GitHub
    /// with --refresh.
    Prs {
        #[arg(long, conflicts_with = "item", required_unless_present = "item")]
        session: Option<String>,
        #[arg(long)]
        item: Option<String>,
        #[arg(long)]
        refresh: bool,
    },
    /// Merge each repository's base branch into a session's worktrees; on
    /// conflicts, the agent is asked to resolve them.
    Update { session: String },
    /// Make a task of each open GitHub issue of a project's repositories
    /// that wasn't imported before.
    ImportIssues { project: String },
    /// Link this Mac's humata and humatad into ~/.local/bin, where a
    /// terminal finds them. A file there of either name that isn't such a
    /// link is left alone, and no shell profile is changed.
    Link {
        /// Only say how they're linked.
        #[arg(long)]
        check: bool,
    },
    /// Answer what an agent asked, by the request id `humata inbox` prints:
    /// reply, approve its move, move its task elsewhere, or dismiss it.
    Answer {
        request: String,
        /// Sent to the session as a message, which it carries on from.
        #[arg(long, conflicts_with_all = ["approve", "move_to", "dismiss"])]
        reply: Option<String>,
        #[arg(long, conflicts_with_all = ["move_to", "dismiss"])]
        approve: bool,
        #[arg(long, value_enum, conflicts_with = "dismiss")]
        move_to: Option<Status>,
        #[arg(long)]
        dismiss: bool,
    },
    /// Move a session to a task or subtask of its project, or with no
    /// --item to the project itself.
    MoveSession {
        session: String,
        #[arg(long, default_value = "")]
        item: String,
    },
    /// Start a session and print its id.
    Start {
        #[arg(long, value_enum)]
        backend: Backend,
        /// The project it belongs to. It works in its own copy of the
        /// project's repositories, with the project's instructions unless
        /// given others.
        #[arg(long, default_value = "")]
        project: String,
        /// The task or subtask it works on; the project is its. It reads the
        /// item's spec ahead of its instructions.
        #[arg(long, default_value = "")]
        item: String,
        /// Also read the spec of the task above its subtask.
        #[arg(long, requires = "item")]
        with_parents: bool,
        /// The folder the agent works in, on its host, as it is. Without
        /// it, --project or --item, it's a chat in an empty folder of its
        /// own on the host.
        #[arg(long)]
        directory: Option<PathBuf>,
        /// What it may do without asking.
        #[arg(long, value_enum, default_value_t = Approval::Ask)]
        approval: Approval,
        #[arg(long, default_value = "")]
        name: String,
        /// The host to run on, as named in the hosts file; this Mac if unset.
        #[arg(long, default_value = "")]
        host: String,
        #[arg(long, default_value = "")]
        model: String,
        /// How hard the model thinks, such as `low`, `high` or `max`; for
        /// Copilot Fusion, its tier: `standard`, `high` or `max`. The
        /// model's default if unset.
        #[arg(long, default_value = "")]
        effort: String,
        /// Copilot only: run with its experimental features on, such as
        /// local models in Fusion.
        #[arg(long)]
        experimental: bool,
        #[arg(long, default_value = "")]
        instructions: String,
        /// A file of settings only the backend reads, such as the scripted
        /// backend's script.
        #[arg(long)]
        backend_config: Option<PathBuf>,
        /// Plan in the project rather than change code: no copy of the
        /// repositories, so it starts at once.
        #[arg(long, requires = "project", conflicts_with_all = ["directory", "item"])]
        planning: bool,
    },
    /// Send a message to a session.
    Send {
        session: String,
        text: String,
        /// Steer the running turn instead of queueing after it.
        #[arg(long)]
        steer: bool,
        /// Print what happens until the session is idle again or waits for
        /// a permission answer.
        #[arg(long)]
        wait: bool,
    },
    /// Answer a permission request.
    Approve {
        session: String,
        request: String,
        /// Refuse it instead.
        #[arg(long)]
        deny: bool,
        /// Print what happens until the session is idle again or waits for
        /// another permission answer.
        #[arg(long)]
        wait: bool,
    },
    /// Set what a session may do without asking. Waiting requests the new
    /// mode allows go ahead.
    Mode {
        session: String,
        #[arg(value_enum)]
        approval: Approval,
    },
    /// Stop a session's running turn.
    Abort { session: String },
    /// Cut a session's chat back to before one of your messages, stopping
    /// its turn and dropping what's queued. Undo it with `undo-rewind`
    /// before sending another message.
    Rewind {
        session: String,
        /// The id of your message to cut from, as `events` prints it.
        before: String,
        /// Put the session's code back as it was then too.
        #[arg(long)]
        files: bool,
    },
    /// Undo a session's last rewind, if no message was sent since.
    UndoRewind { session: String },
    /// List the skills on this Mac and each host, then each clash: two or
    /// more files with one name that one session would get, with the
    /// hashes to pick between.
    Skills,
    /// Pick which skill of a clash wins, for every project and host, until
    /// either file changes.
    PickSkill {
        name: String,
        /// The hash of the file to keep, as `skills` prints it.
        winner: String,
        /// The hashes of every file of that name, as `skills` prints them.
        #[arg(required = true)]
        candidates: Vec<String>,
    },
    /// List the Claude Code and Copilot plugins installed on this Mac and
    /// each host, with what each adds and whether its CLI has it on.
    Plugins,
    /// Turn a plugin on or off for a project's sessions, over how its CLI
    /// has it, or back to how the CLI has it. Installing stays in each CLI.
    Plugin {
        project: String,
        agent: PluginAgent,
        /// Its name as `plugins` prints it, such as `review@market`.
        id: String,
        choice: PluginChoice,
    },
    /// Take a queued message out before its turn starts; its id is in the
    /// `queued` event.
    Unqueue { session: String, queue_id: String },
    /// Fork a session and print the fork's id.
    Fork {
        session: String,
        /// Keep only the events before this one.
        #[arg(long, default_value = "")]
        before: String,
        #[arg(long, default_value = "")]
        name: String,
        /// Share the session's copy of the code instead of getting one of
        /// the code as it was there.
        #[arg(long)]
        chat_only: bool,
        /// The task or subtask the fork works on, reading its specs; a fork
        /// of a planning session also gets a copy of the repositories.
        #[arg(long, default_value = "")]
        item: String,
    },
    /// Rename a session.
    Rename { session: String, name: String },
    /// Archive a session: its copy of the code is saved and removed, and it
    /// takes no messages until unarchived.
    Archive {
        session: String,
        /// Unarchive it instead; its code comes back before its next turn.
        #[arg(long)]
        undo: bool,
    },
    /// Mark the session whose changes are the ones to keep.
    Keeper {
        session: String,
        /// Unmark it instead.
        #[arg(long)]
        undo: bool,
    },
    /// Print what a session changed in each repository, as a diff:
    /// everything since its copy was made, unless asked for another range.
    Changes {
        session: String,
        /// Only what isn't staged yet.
        #[arg(long, conflicts_with_all = ["staged", "branch", "turn"])]
        unstaged: bool,
        /// Only what is staged.
        #[arg(long, conflicts_with_all = ["branch", "turn"])]
        staged: bool,
        /// Since the copy's work left this branch.
        #[arg(long, conflicts_with = "turn")]
        branch: Option<String>,
        /// What this turn changed; the first is 1.
        #[arg(long)]
        turn: Option<u32>,
        /// Unchanged lines around each change.
        #[arg(long)]
        context: Option<u32>,
        /// Only this file, as REPOSITORY/PATH with the repository's name in
        /// the session.
        #[arg(long)]
        file: Option<String>,
    },
    /// List the files in a session's folder, hidden ones left out.
    Files { session: String },
    /// Print a file in a session's folder, by its path from `files`.
    Cat { session: String, path: String },
    /// List one level of a folder: a session's, or any on a host.
    Ls {
        /// Under the session's folder or the root; the root itself when
        /// left out.
        directory: Option<String>,
        #[command(flatten)]
        place: FolderPlace,
    },
    /// Print a file under a session's folder or any folder on a host.
    Show {
        path: String,
        #[command(flatten)]
        place: FolderPlace,
    },
    /// Stage a file's changes in the session's copy, or one hunk's.
    Stage(EditArgs),
    /// Unstage a file's staged changes, or one hunk's.
    Unstage(EditArgs),
    /// Throw away a file's unstaged changes, or one hunk's. A file git
    /// doesn't know is removed. This can't be undone.
    Revert(EditArgs),
    /// Comment on a session's changes and send the comments to its agent.
    #[command(subcommand)]
    Review(ReviewCommand),
    /// Print a session's events; a subagent's are indented, and end with
    /// the call that started it.
    History { session: String },
    /// List the subagents a session's agent started: call id, status, type,
    /// tokens, tool calls, milliseconds, and what it was asked to do.
    Subagents { session: String },
    /// Print a session's events, then each new one, until interrupted.
    Watch { session: String },
    /// List every host's sessions.
    Sessions {
        /// Only this project's.
        #[arg(long, default_value = "")]
        project: String,
    },
    /// Serve Humata to an agent outside it, as MCP tools on stdin and
    /// stdout: `claude mcp add humata -- humata mcp`.
    Mcp,
    /// Point local-model sessions, here and on hosts, at a model server on
    /// this Mac's 127.0.0.1, as the Humata app does when it starts.
    ModelServer {
        /// The port it printed when it started.
        port: u32,
    },
}

fn main() -> ExitCode {
    let mut arguments = Arguments::parse();
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("humata: couldn't start: {error}");
            return ExitCode::FAILURE;
        }
    };
    if let Command::Terminal(TerminalCommand::Attach { terminal, host }) = &mut arguments.command {
        let target = attach::Target {
            host: std::mem::take(host),
            terminal: std::mem::take(terminal),
        };
        let code = runtime.block_on(attach_terminal(arguments.socket, target));
        // Standard input is read on a thread that may be blocked in a read
        // no one wants any more; don't wait for it.
        runtime.shutdown_background();
        return code;
    }
    match runtime.block_on(run(arguments)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("humata: {}", humata_wire::describe(&error));
            ExitCode::FAILURE
        }
    }
}

async fn connect(
    socket: Option<PathBuf>,
    wait: Wait,
    say_waiting: bool,
) -> Result<Client, CliError> {
    let client = connect_wire(socket).await?;
    Ok(Client::new(client, wait, say_waiting))
}

async fn connect_wire(socket: Option<PathBuf>) -> Result<humata_wire::Client, CliError> {
    let path = socket
        .or_else(humata_wire::default_socket)
        .ok_or(CliError::NoSocket)?;
    let stream = UnixStream::connect(&path)
        .await
        .map_err(|source| CliError::Connect { path, source })?;
    let (reader, writer) = stream.into_split();
    Ok(humata_wire::Client::connect(reader, writer, pb::Role::Client, VERSION).await?)
}

/// How long `humata terminal attach` waits before its first try to
/// reattach after the connection drops.
const REATTACH_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

/// `humata terminal attach`: this process's terminal, raw, piped to the
/// host's until its shell exits, which gives the exit code.
async fn attach_terminal(socket: Option<PathBuf>, target: attach::Target) -> ExitCode {
    let sizes = match attach::pty::sizes() {
        Ok(sizes) => sizes,
        Err(error) => {
            eprintln!("humata: couldn't watch the terminal's size: {error}");
            return ExitCode::FAILURE;
        }
    };
    let raw = attach::pty::Raw::enter();
    let ended = attach::attach(
        &target,
        || connect_wire(socket.clone()),
        tokio::io::stdin(),
        tokio::io::stdout(),
        sizes,
        REATTACH_WAIT,
    )
    .await;
    drop(raw);
    match ended {
        Ok(attach::Ended::Exited(code)) => ExitCode::from(attach::exit_status(code)),
        Ok(attach::Ended::InputClosed) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("\r\nhumata: {}", humata_wire::describe(&error));
            ExitCode::FAILURE
        }
    }
}

async fn run(arguments: Arguments) -> Result<(), CliError> {
    use pb::request::Kind as Request;
    use pb::response::Kind as Response;
    let mcp = matches!(arguments.command, Command::Mcp);
    let wait = if mcp {
        Wait::For(MCP_APPROVAL_WAIT)
    } else if arguments.no_wait {
        Wait::For(std::time::Duration::ZERO)
    } else {
        Wait::UntilDecided
    };
    let client = connect(arguments.socket, wait, !mcp).await?;
    match arguments.command {
        Command::Project(ProjectCommand::Add {
            name,
            repositories,
            instructions,
            skills,
        }) => {
            let repositories = repositories
                .iter()
                .map(|given| repository(given))
                .collect::<Result<_, _>>()?;
            let request = Request::CreateProject(pb::CreateProject {
                name,
                repositories,
                instructions,
                skills: read_skills(&skills)?,
            });
            match client.call(request).await? {
                Response::ProjectCreated(pb::ProjectCreated {
                    project: Some(project),
                }) => println!("{}", project.id),
                other => return Err(unexpected("the created project", &other)),
            }
        }
        Command::Project(ProjectCommand::Update {
            project,
            name,
            instructions,
            repositories,
            skills,
            no_skills,
        }) => {
            let repositories = if repositories.is_empty() {
                None
            } else {
                Some(pb::Repositories {
                    repositories: repositories
                        .iter()
                        .map(|given| repository(given))
                        .collect::<Result<_, _>>()?,
                })
            };
            let skills = if no_skills || !skills.is_empty() {
                Some(pb::Skills {
                    skills: read_skills(&skills)?,
                })
            } else {
                None
            };
            let request = Request::UpdateProject(pb::UpdateProject {
                project_id: project,
                name,
                instructions,
                skills,
                repositories,
            });
            match client.call(request).await? {
                Response::ProjectUpdated(_) => {}
                other => return Err(unexpected("the updated project", &other)),
            }
        }
        Command::Project(ProjectCommand::List) => {
            match client
                .call(Request::ListProjects(pb::ListProjects {}))
                .await?
            {
                Response::Projects(projects) => {
                    for project in &projects.projects {
                        println!("{}", print::project(project));
                    }
                }
                other => return Err(unexpected("the projects", &other)),
            }
        }
        Command::Project(ProjectCommand::Delete { project }) => {
            let request = Request::DeleteProject(pb::DeleteProject {
                project_id: project,
            });
            expect_ok(
                client.call(request).await?,
                |response| matches!(response, Response::ProjectDeleted(_)),
                "ProjectDeleted",
            )?;
        }
        Command::Item(command) => item(&client, command).await?,
        Command::Import(ImportCommand::List { host }) => {
            match client
                .call(Request::ListImportable(pb::ListImportable { host }))
                .await?
            {
                Response::Importable(importable) => {
                    for session in &importable.sessions {
                        println!("{}", print::importable(session));
                    }
                }
                other => return Err(unexpected("the importable sessions", &other)),
            }
        }
        Command::Import(ImportCommand::Session {
            backend,
            id,
            host,
            name,
            project,
        }) => {
            let request = Request::ImportSession(pb::ImportSession {
                backend: pb::Backend::from(backend).into(),
                external_id: id,
                host,
                name,
                project_id: project,
            });
            match client.call(request).await? {
                Response::SessionStarted(pb::SessionStarted {
                    session: Some(session),
                }) => println!("{}", session.id),
                other => return Err(unexpected("the imported session", &other)),
            }
        }
        Command::Board(BoardCommand::Log { project }) => {
            let request = Request::ReadBoardLog(pb::ReadBoardLog {
                project_id: project,
                ..Default::default()
            });
            match client.call(request).await? {
                Response::BoardLog(log) => {
                    for change in &log.changes {
                        println!("{}", print::change(change));
                    }
                }
                other => return Err(unexpected("the board log", &other)),
            }
        }
        Command::Board(BoardCommand::Revert { change }) => {
            let request = Request::RevertBoardChange(pb::RevertBoardChange {
                change_id: change,
                surface: pb::ChangeSurface::CommandLine.into(),
            });
            match client.call(request).await? {
                Response::BoardChangeReverted(reverted) => {
                    for field in reverted.kept() {
                        let name = match field {
                            pb::ItemField::Title => "title",
                            pb::ItemField::Spec => "spec",
                            pb::ItemField::Status => "status",
                            pb::ItemField::Priority => "priority",
                            pb::ItemField::Unspecified => continue,
                        };
                        eprintln!("Kept the {name}: it changed again since.");
                    }
                }
                other => return Err(unexpected("the reverted change", &other)),
            }
        }
        Command::Host(command) => host(&client, command).await?,
        Command::Repository(command) => saved_repository(&client, command).await?,
        Command::Recover(command) => recover(&client, command).await?,
        Command::Terminal(command) => terminal(&client, command).await?,
        Command::Port(command) => port(&client, command).await?,
        Command::Proposal(command) => proposal(&client, command).await?,
        Command::Approval(command) => approval(&client, command).await?,
        Command::Stage(edit) => edit_changes(&client, edit, pb::ChangeAction::Stage).await?,
        Command::Unstage(edit) => edit_changes(&client, edit, pb::ChangeAction::Unstage).await?,
        Command::Revert(edit) => edit_changes(&client, edit, pb::ChangeAction::Revert).await?,
        Command::Review(command) => review(&client, command).await?,
        Command::ModelServer { port } => {
            match client
                .call(Request::SetModelServer(pb::SetModelServer { port }))
                .await?
            {
                Response::ModelServerSet(_) => {}
                other => return Err(unexpected("the model server's port", &other)),
            }
        }
        Command::Inbox => match client.call(Request::ListInbox(pb::ListInbox {})).await? {
            Response::Inbox(inbox) => {
                for entry in &inbox.entries {
                    println!("{}", print::inbox_entry(entry));
                }
            }
            other => return Err(unexpected("the inbox", &other)),
        },
        Command::Search {
            words,
            project,
            item,
            session,
            limit,
        } => {
            let search = pb::Search {
                query: words.join(" "),
                project_id: project.unwrap_or_default(),
                item_id: item.unwrap_or_default(),
                session_id: session.unwrap_or_default(),
                limit: limit.unwrap_or_default(),
                ..pb::Search::default()
            };
            match client.call(Request::Search(search)).await? {
                Response::SearchResults(results) => {
                    for hit in &results.hits {
                        println!("{}", print::search_hit(hit));
                    }
                }
                other => return Err(unexpected("the search results", &other)),
            }
        }
        Command::Land {
            session,
            branch,
            message,
            push,
            pull_requests,
        } => {
            let land = pb::LandSession {
                session_id: session,
                branch,
                message,
                push,
                open_pull_requests: pull_requests,
            };
            match client.call(Request::LandSession(land)).await? {
                Response::SessionLanded(landed) => print!("{}", print::landed(&landed)),
                other => return Err(unexpected("the landed branches", &other)),
            }
        }
        Command::Prs {
            session,
            item,
            refresh,
        } => {
            let list = pb::ListPullRequests {
                session_id: session.unwrap_or_default(),
                item_id: item.unwrap_or_default(),
                refresh,
            };
            match client.call(Request::ListPullRequests(list)).await? {
                Response::PullRequests(pulls) => {
                    for pull in &pulls.pull_requests {
                        println!("{}", print::pull_request(pull));
                    }
                }
                other => return Err(unexpected("the pull requests", &other)),
            }
        }
        Command::Update { session } => {
            let update = pb::UpdateFromBase {
                session_id: session,
            };
            match client.call(Request::UpdateFromBase(update)).await? {
                Response::BaseUpdated(updated) => print!("{}", print::base_updated(&updated)),
                other => return Err(unexpected("the update", &other)),
            }
        }
        Command::ImportIssues { project } => {
            let import = pb::ImportIssues {
                project_id: project,
            };
            match client.call(Request::ImportIssues(import)).await? {
                Response::IssuesImported(imported) => {
                    for item in &imported.items {
                        println!("{}", print::item(item));
                    }
                    if imported.skipped > 0 {
                        println!("{} imported before, skipped", imported.skipped);
                    }
                }
                other => return Err(unexpected("the imported issues", &other)),
            }
        }
        Command::Link { check } => {
            match client
                .call(Request::LinkCommands(pb::LinkCommands { check }))
                .await?
            {
                Response::CommandsLinked(linked) => print!("{}", print::command_links(&linked)),
                other => return Err(unexpected("the links", &other)),
            }
        }
        Command::Answer {
            request,
            reply,
            approve,
            move_to,
            dismiss,
        } => {
            use pb::answer_request::Answer;
            let answer = match (reply, move_to) {
                (Some(text), _) => Answer::Reply(text),
                (None, Some(status)) => Answer::MoveTo(pb::ItemStatus::from(status).into()),
                (None, None) if approve => Answer::Approve(true),
                (None, None) if dismiss => Answer::Dismiss(true),
                (None, None) => return Err(CliError::NoAnswer),
            };
            let request = Request::AnswerRequest(pb::AnswerRequest {
                request_id: request,
                answer: Some(answer),
            });
            match client.call(request).await? {
                Response::RequestAnswered(_) => {}
                other => return Err(unexpected("RequestAnswered", &other)),
            }
        }
        Command::MoveSession { session, item } => {
            update(
                &client,
                pb::UpdateSession {
                    session_id: session,
                    item_id: Some(item),
                    ..Default::default()
                },
            )
            .await?;
        }
        Command::Start {
            backend,
            project,
            item,
            with_parents,
            directory,
            approval,
            name,
            host,
            model,
            effort,
            experimental,
            instructions,
            backend_config,
            planning,
        } => {
            let backend_config = match backend_config {
                Some(path) => std::fs::read_to_string(&path)
                    .map_err(|source| CliError::ReadBackendConfig { path, source })?,
                None => String::new(),
            };
            let request = Request::StartSession(pb::StartSession {
                backend: pb::Backend::from(backend).into(),
                working_directory: directory
                    .map(|directory| directory.display().to_string())
                    .unwrap_or_default(),
                model,
                instructions,
                name,
                host,
                backend_config,
                approval_mode: pb::ApprovalMode::from(approval).into(),
                project_id: project,
                repositories: Vec::new(),
                item_id: item,
                specs: if with_parents {
                    pb::SpecScope::WithParents
                } else {
                    pb::SpecScope::Item
                }
                .into(),
                planning,
                plugins: Vec::new(),
                // The daemon's to fill in.
                notices: Vec::new(),
                effort,
                experimental,
            });
            match client.call(request).await? {
                Response::SessionStarted(pb::SessionStarted {
                    session: Some(session),
                }) => println!("{}", session.id),
                other => return Err(unexpected("the started session", &other)),
            }
        }
        Command::Send {
            session,
            text,
            steer,
            wait,
        } => {
            let mode = if steer {
                pb::SendMode::Immediate
            } else {
                pb::SendMode::Enqueue
            };
            let request = Request::SendMessage(pb::SendMessage {
                session_id: session.clone(),
                text,
                mode: mode.into(),
                client_id: String::new(),
                images: Vec::new(),
            });
            let sent = |response: &Response| matches!(response, Response::MessageSent(_));
            if wait {
                call_and_wait(&client, &session, request, sent, "MessageSent", printed).await?;
            } else {
                expect_ok(client.call(request).await?, sent, "MessageSent")?;
            }
        }
        Command::Approve {
            session,
            request,
            deny,
            wait,
        } => {
            let request = Request::Approve(pb::Approve {
                session_id: session.clone(),
                permission_request_id: request,
                approved: !deny,
            });
            let approved = |response: &Response| matches!(response, Response::Approved(_));
            if wait {
                call_and_wait(&client, &session, request, approved, "Approved", printed).await?;
            } else {
                expect_ok(client.call(request).await?, approved, "Approved")?;
            }
        }
        Command::Mode { session, approval } => {
            let request = Request::SetApprovalMode(pb::SetApprovalMode {
                session_id: session,
                mode: pb::ApprovalMode::from(approval).into(),
            });
            expect_ok(
                client.call(request).await?,
                |response| matches!(response, Response::ApprovalModeSet(_)),
                "ApprovalModeSet",
            )?;
        }
        Command::Abort { session } => {
            let request = Request::AbortTurn(pb::AbortTurn {
                session_id: session,
                drop_queued: false,
            });
            expect_ok(
                client.call(request).await?,
                |response| matches!(response, Response::TurnAborted(_)),
                "TurnAborted",
            )?;
        }
        Command::Rewind {
            session,
            before,
            files,
        } => {
            let request = Request::RewindSession(pb::RewindSession {
                session_id: session,
                before_event_id: before,
                files,
            });
            expect_ok(
                client.call(request).await?,
                |response| matches!(response, Response::SessionRewound(_)),
                "SessionRewound",
            )?;
        }
        Command::Skills => match client.call(Request::ListSkills(pb::ListSkills {})).await? {
            Response::SkillList(list) => {
                for skill in &list.skills {
                    println!("{}", print::skill(skill));
                }
                for clash in &list.clashes {
                    println!("{}", print::skill_clash(clash));
                }
                for host in &list.unreachable {
                    println!("unreachable\t{host}");
                }
            }
            other => return Err(unexpected("the skills", &other)),
        },
        Command::PickSkill {
            name,
            winner,
            candidates,
        } => {
            let request = Request::PickSkill(pb::PickSkill {
                name,
                candidates,
                winner,
            });
            expect_ok(
                client.call(request).await?,
                |response| matches!(response, Response::SkillPicked(_)),
                "SkillPicked",
            )?;
        }
        Command::Plugins => match client
            .call(Request::ListPlugins(pb::ListPlugins {}))
            .await?
        {
            Response::PluginList(list) => {
                for plugin in &list.plugins {
                    println!("{}", print::plugin(plugin));
                }
                for host in &list.unreachable {
                    println!("unreachable\t{host}");
                }
            }
            other => return Err(unexpected("the plugins", &other)),
        },
        Command::Plugin {
            project,
            agent,
            id,
            choice,
        } => {
            let agent = match agent {
                PluginAgent::Claude => pb::PluginAgent::Claude,
                PluginAgent::Copilot => pb::PluginAgent::Copilot,
            };
            let enabled = match choice {
                PluginChoice::On => Some(true),
                PluginChoice::Off => Some(false),
                PluginChoice::Default => None,
            };
            let request = Request::SetProjectPlugin(pb::SetProjectPlugin {
                project_id: project,
                agent: agent as i32,
                id,
                enabled,
            });
            expect_ok(
                client.call(request).await?,
                |response| matches!(response, Response::ProjectUpdated(_)),
                "ProjectUpdated",
            )?;
        }
        Command::UndoRewind { session } => {
            let request = Request::UndoRewind(pb::UndoRewind {
                session_id: session,
            });
            expect_ok(
                client.call(request).await?,
                |response| matches!(response, Response::RewindUndone(_)),
                "RewindUndone",
            )?;
        }
        Command::Unqueue { session, queue_id } => {
            let request = Request::Unqueue(pb::Unqueue {
                session_id: session,
                queue_id,
            });
            expect_ok(
                client.call(request).await?,
                |response| matches!(response, Response::Unqueued(_)),
                "Unqueued",
            )?;
        }
        Command::Fork {
            session,
            before,
            name,
            chat_only,
            item,
        } => {
            let request = Request::ForkSession(pb::ForkSession {
                session_id: session,
                before_event_id: before,
                name,
                chat_only,
                item_id: item,
                ..Default::default()
            });
            match client.call(request).await? {
                Response::SessionForked(pb::SessionForked {
                    session: Some(session),
                }) => println!("{}", session.id),
                other => return Err(unexpected("the forked session", &other)),
            }
        }
        Command::Rename { session, name } => {
            update(
                &client,
                pb::UpdateSession {
                    session_id: session,
                    name: Some(name),
                    ..Default::default()
                },
            )
            .await?;
        }
        Command::Archive { session, undo } => {
            update(
                &client,
                pb::UpdateSession {
                    session_id: session,
                    archived: Some(!undo),
                    ..Default::default()
                },
            )
            .await?;
        }
        Command::Keeper { session, undo } => {
            update(
                &client,
                pb::UpdateSession {
                    session_id: session,
                    keeper: Some(!undo),
                    ..Default::default()
                },
            )
            .await?;
        }
        Command::Changes {
            session,
            unstaged,
            staged,
            branch,
            turn,
            context,
            file,
        } => {
            let kind = match (unstaged, staged, &branch, turn) {
                (true, ..) => pb::ChangeRangeKind::Unstaged,
                (_, true, ..) => pb::ChangeRangeKind::Staged,
                (_, _, Some(_), _) => pb::ChangeRangeKind::Branch,
                (_, _, _, Some(_)) => pb::ChangeRangeKind::Turn,
                _ => pb::ChangeRangeKind::Session,
            };
            let file = file.map(|file| {
                let (repository, path) = file.split_once('/').unwrap_or(("", &file));
                pb::FileRef {
                    repository: repository.to_owned(),
                    path: path.to_owned(),
                }
            });
            let request = Request::ReadChanges(pb::ReadChanges {
                session_id: session,
                range: Some(pb::ChangeRange {
                    kind: kind.into(),
                    branch: branch.unwrap_or_default(),
                    turn: turn.unwrap_or_default(),
                }),
                context,
                file,
                whole_file: false,
            });
            match client.call(request).await? {
                Response::Changes(changes) => print!("{}", print::changes(&changes)),
                other => return Err(unexpected("the changes", &other)),
            }
        }
        Command::Files { session } => {
            let request = Request::ListFiles(pb::ListFiles {
                session_id: session,
            });
            match client.call(request).await? {
                Response::Files(files) => print!("{}", print::files(&files)),
                other => return Err(unexpected("the files", &other)),
            }
        }
        Command::Cat { session, path } => {
            let request = Request::ReadFile(pb::ReadFile {
                session_id: session,
                path,
                ..pb::ReadFile::default()
            });
            match client.call(request).await? {
                Response::FileContents(contents) if contents.binary => {
                    println!("{} isn't text", contents.path);
                }
                Response::FileContents(contents) => print!("{}", contents.text),
                other => return Err(unexpected("the file", &other)),
            }
        }
        Command::Ls { directory, place } => {
            let request = Request::ListFolder(pb::ListFolder {
                host: place.host,
                session_id: place.session.unwrap_or_default(),
                root: place.root.unwrap_or_default(),
                directory: directory.unwrap_or_default(),
            });
            match client.call(request).await? {
                Response::Folder(folder) => print!("{}", print::folder(&folder)),
                other => return Err(unexpected("the folder", &other)),
            }
        }
        Command::Show { path, place } => {
            let request = Request::ReadFile(pb::ReadFile {
                session_id: place.session.unwrap_or_default(),
                path,
                host: place.host,
                root: place.root.unwrap_or_default(),
                turn: None,
            });
            match client.call(request).await? {
                Response::FileContents(contents) if contents.binary => {
                    println!("{} isn't text ({} bytes)", contents.path, contents.size);
                }
                Response::FileContents(contents) => {
                    print!("{}", contents.text);
                    if contents.truncated {
                        println!(
                            "\n({} is {} bytes; this is its start)",
                            contents.path, contents.size
                        );
                    }
                }
                other => return Err(unexpected("the file", &other)),
            }
        }
        Command::History { session } => {
            for event in history(&client, &session).await? {
                println!("{}", print::event(&event));
            }
        }
        Command::Subagents { session } => {
            for line in print::subagents(&history(&client, &session).await?) {
                println!("{line}");
            }
        }
        Command::Watch { session } => {
            let request = Request::WatchSession(pb::WatchSession {
                session_id: session,
                after_event_id: String::new(),
            });
            let (_, mut stream) = client.stream(request).await?;
            while let Some(item) = stream.next().await {
                if let Some(event) = session_event(item)? {
                    println!("{}", print::event(&event));
                }
            }
        }
        Command::Sessions { project } => match client
            .call(Request::ListSessions(pb::ListSessions {
                project_id: project,
            }))
            .await?
        {
            Response::Sessions(sessions) => {
                for session in &sessions.sessions {
                    println!("{}", print::session(session));
                }
            }
            other => return Err(unexpected("the sessions", &other)),
        },
        Command::Mcp => control::serve(&client, tokio::io::stdin(), tokio::io::stdout()).await?,
    }
    Ok(())
}

async fn item(client: &Client, command: ItemCommand) -> Result<(), CliError> {
    use pb::request::Kind as Request;
    use pb::response::Kind as Response;
    let updated = |response: &Response| matches!(response, Response::ItemUpdated(_));
    match command {
        ItemCommand::Add {
            project,
            parent,
            title,
            spec,
            status,
            priority,
        } => {
            let request = Request::CreateItem(pb::CreateItem {
                project_id: project,
                parent_id: parent,
                title,
                spec: spec
                    .map(|path| read_spec(&path))
                    .transpose()?
                    .unwrap_or_default(),
                status: pb::ItemStatus::from(status).into(),
                priority: pb::Priority::from(priority).into(),
                surface: pb::ChangeSurface::CommandLine.into(),
            });
            match client.call(request).await? {
                Response::ItemCreated(pb::ItemCreated { item: Some(item) }) => {
                    println!("{}", item.id);
                }
                other => return Err(unexpected("the created item", &other)),
            }
        }
        ItemCommand::Edit {
            item,
            title,
            spec,
            status,
            priority,
        } => {
            let request = Request::UpdateItem(pb::UpdateItem {
                item_id: item,
                title,
                spec: spec.map(|path| read_spec(&path)).transpose()?,
                status: status.map(|status| pb::ItemStatus::from(status).into()),
                priority: priority.map(|priority| pb::Priority::from(priority).into()),
                by_session_id: String::new(),
                surface: pb::ChangeSurface::CommandLine.into(),
            });
            expect_ok(client.call(request).await?, updated, "ItemUpdated")?;
        }
        ItemCommand::Move { item, status } => {
            let request = Request::UpdateItem(pb::UpdateItem {
                item_id: item,
                status: Some(pb::ItemStatus::from(status).into()),
                surface: pb::ChangeSurface::CommandLine.into(),
                ..Default::default()
            });
            expect_ok(client.call(request).await?, updated, "ItemUpdated")?;
        }
        ItemCommand::List { project } => {
            for item in items(client, project).await? {
                println!("{}", print::item(&item));
            }
        }
        ItemCommand::Spec { item, project } => {
            let found = items(client, project)
                .await?
                .into_iter()
                .find(|found| found.id == item)
                .ok_or(CliError::UnknownItem { item })?;
            print!("{}", found.spec);
        }
    }
    Ok(())
}

async fn items(client: &Client, project: String) -> Result<Vec<pb::Item>, CliError> {
    let request = pb::request::Kind::ListItems(pb::ListItems {
        project_id: project,
    });
    match client.call(request).await? {
        pb::response::Kind::Items(items) => Ok(items.items),
        other => Err(unexpected("the items", &other)),
    }
}

/// A spec from `path`, or from stdin for `-`.
async fn recover(client: &Client, command: RecoverCommand) -> Result<(), CliError> {
    use pb::request::Kind as Request;
    use pb::response::Kind as Response;
    match command {
        RecoverCommand::List => {
            match client
                .call(Request::ListRecoveries(pb::ListRecoveries {}))
                .await?
            {
                Response::Recoveries(recoveries) => {
                    for recovery in &recoveries.recoveries {
                        println!("{}", print::recovery(recovery));
                    }
                }
                other => return Err(unexpected("the recovery copies", &other)),
            }
        }
        RecoverCommand::Session { session, host } => {
            let host = match host {
                Some(host) => host,
                None => recovery_host(client, &session).await?,
            };
            let request = Request::RestoreSession(pb::RestoreSession {
                session_id: session,
                host,
            });
            match client.call(request).await? {
                Response::SessionRestored(pb::SessionRestored {
                    session: Some(session),
                }) => println!("{}", print::session(&session)),
                other => return Err(unexpected("the session restored", &other)),
            }
        }
        RecoverCommand::Host { name } => {
            let request = Request::RestoreHost(pb::RestoreHost { host: name });
            match client.call(request).await? {
                Response::HostRestored(restored) => {
                    for session in &restored.restored {
                        println!("{}", print::session(session));
                    }
                    for failure in &restored.failed {
                        eprintln!("{}\t{}", failure.session_id, failure.message);
                    }
                    if !restored.failed.is_empty() {
                        return Err(CliError::NotAllRestored {
                            failed: restored.failed.len(),
                        });
                    }
                }
                other => return Err(unexpected("the sessions restored", &other)),
            }
        }
    }
    Ok(())
}

async fn port(client: &Client, command: PortCommand) -> Result<(), CliError> {
    use pb::request::Kind as Request;
    use pb::response::Kind as Response;
    match command {
        PortCommand::Forward {
            port,
            host,
            session,
        } => {
            let request = Request::ForwardPort(pb::ForwardPort {
                host,
                port,
                session_id: session,
            });
            match client.call(request).await? {
                Response::PortForwarded(pb::PortForwarded {
                    forward: Some(forward),
                }) => println!("{}", print::forward(&forward)),
                other => return Err(unexpected("the port forwarded", &other)),
            }
        }
        PortCommand::List => match client
            .call(Request::ListForwards(pb::ListForwards {}))
            .await?
        {
            Response::Forwards(forwards) => {
                for forward in &forwards.forwards {
                    println!("{}", print::forward(forward));
                }
            }
            other => return Err(unexpected("the forwards", &other)),
        },
        PortCommand::Stop { id } => {
            match client
                .call(Request::StopForward(pb::StopForward { id }))
                .await?
            {
                Response::ForwardStopped(_) => {}
                other => return Err(unexpected("the forward stopped", &other)),
            }
        }
    }
    Ok(())
}

async fn terminal(client: &Client, command: TerminalCommand) -> Result<(), CliError> {
    use pb::request::Kind as Request;
    use pb::response::Kind as Response;
    match command {
        // Run by main before it connects, since it reconnects on its own.
        TerminalCommand::Attach { .. } => {}
        TerminalCommand::Open {
            session,
            host,
            folder,
        } => {
            let request = Request::OpenTerminal(pb::OpenTerminal {
                host,
                session_id: session.unwrap_or_default(),
                folder: folder.unwrap_or_default(),
                rows: 24,
                columns: 80,
            });
            match client.call(request).await? {
                Response::TerminalOpened(pb::TerminalOpened {
                    terminal: Some(terminal),
                }) => println!("{}", print::terminal(&terminal)),
                other => return Err(unexpected("the terminal opened", &other)),
            }
        }
        TerminalCommand::List { host } => {
            match client
                .call(Request::ListTerminals(pb::ListTerminals { host }))
                .await?
            {
                Response::Terminals(terminals) => {
                    for terminal in &terminals.terminals {
                        println!("{}", print::terminal(terminal));
                    }
                }
                other => return Err(unexpected("the terminals", &other)),
            }
        }
        TerminalCommand::Type {
            terminal,
            text,
            host,
            raw,
        } => {
            let mut data = text.into_bytes();
            if !raw {
                data.push(b'\r');
            }
            let request = Request::TerminalInput(pb::TerminalInput {
                host,
                terminal_id: terminal,
                data,
            });
            match client.call(request).await? {
                Response::TerminalInputSent(_) => {}
                other => return Err(unexpected("the text typed", &other)),
            }
        }
        TerminalCommand::Read {
            terminal,
            host,
            until,
            timeout,
        } => read_terminal(client, terminal, host, until, timeout).await?,
        TerminalCommand::Resize {
            terminal,
            rows,
            columns,
            host,
        } => {
            let request = Request::ResizeTerminal(pb::ResizeTerminal {
                host,
                terminal_id: terminal,
                rows,
                columns,
            });
            match client.call(request).await? {
                Response::TerminalResized(_) => {}
                other => return Err(unexpected("the terminal resized", &other)),
            }
        }
        TerminalCommand::Close { terminal, host } => {
            let request = Request::CloseTerminal(pb::CloseTerminal {
                host,
                terminal_id: terminal,
            });
            match client.call(request).await? {
                Response::TerminalClosed(_) => {}
                other => return Err(unexpected("the terminal closed", &other)),
            }
        }
    }
    Ok(())
}

/// Prints the terminal's output as it comes, until `until` shows up in it,
/// the shell exits, or `timeout` seconds pass; with no `until`, just what
/// it has printed so far.
async fn read_terminal(
    client: &Client,
    terminal: String,
    host: String,
    until: Option<String>,
    timeout: u64,
) -> Result<(), CliError> {
    use std::io::Write;
    let request = pb::request::Kind::AttachTerminal(pb::AttachTerminal {
        host,
        terminal_id: terminal.clone(),
    });
    let (_, mut stream) = client.stream(request).await?;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout);
    let mut shown = Vec::new();
    let mut stdout = std::io::stdout();
    loop {
        let next = tokio::time::timeout_at(deadline, stream.next()).await;
        let Ok(next) = next else {
            return Err(CliError::TerminalTimedOut {
                terminal,
                until: until.unwrap_or_default(),
                seconds: timeout,
            });
        };
        let output = match next {
            Some(Ok(pb::StreamItem {
                kind: Some(pb::stream_item::Kind::TerminalOutput(output)),
            })) => output,
            Some(Ok(_)) => continue,
            Some(Err(error)) => {
                return Err(CliError::Wire(humata_wire::ClientError::Refused {
                    code: error.code(),
                    message: error.message,
                }));
            }
            None => return Ok(()),
        };
        if output.replay {
            shown.clear();
        }
        shown.extend_from_slice(&output.data);
        let _ = stdout.write_all(&output.data);
        let _ = stdout.flush();
        let found = until
            .as_deref()
            .is_none_or(|until| String::from_utf8_lossy(&shown).contains(until));
        if found || output.exited {
            return Ok(());
        }
    }
}

/// The host `session`'s recovery copy is of, to restore it there.
async fn recovery_host(client: &Client, session: &str) -> Result<String, CliError> {
    use pb::request::Kind as Request;
    use pb::response::Kind as Response;
    match client
        .call(Request::ListRecoveries(pb::ListRecoveries {}))
        .await?
    {
        Response::Recoveries(recoveries) => Ok(recoveries
            .recoveries
            .into_iter()
            .find(|recovery| recovery.session_id == session)
            .map(|recovery| recovery.host)
            .unwrap_or_default()),
        other => Err(unexpected("the recovery copies", &other)),
    }
}

async fn saved_repository(client: &Client, command: RepositoryCommand) -> Result<(), CliError> {
    use pb::request::Kind as Request;
    use pb::response::Kind as Response;
    let request = match command {
        RepositoryCommand::List => Request::ListSavedRepositories(pb::ListSavedRepositories {}),
        RepositoryCommand::Rename { id, name } => {
            Request::RenameSavedRepository(pb::RenameSavedRepository { id, name })
        }
        RepositoryCommand::Remove { id } => {
            Request::RemoveSavedRepository(pb::RemoveSavedRepository { id })
        }
        RepositoryCommand::Owner(command) => return repository_owner(client, command).await,
    };
    match client.call(request).await? {
        Response::SavedRepositories(saved) => {
            for repository in &saved.repositories {
                println!("{}", print::saved_repository(repository));
            }
            Ok(())
        }
        other => Err(unexpected("the saved repositories", &other)),
    }
}

async fn repository_owner(client: &Client, command: OwnerCommand) -> Result<(), CliError> {
    use pb::request::Kind as Request;
    use pb::response::Kind as Response;
    let request = match command {
        OwnerCommand::List => Request::ListRepositoryOwners(pb::ListRepositoryOwners {}),
        OwnerCommand::Add { login } => {
            Request::AddRepositoryOwner(pb::AddRepositoryOwner { login })
        }
        OwnerCommand::Remove { login } => {
            Request::RemoveRepositoryOwner(pb::RemoveRepositoryOwner { login })
        }
        OwnerCommand::Refresh { login } => {
            Request::RefreshRepositoryOwners(pb::RefreshRepositoryOwners {
                login: login.unwrap_or_default(),
            })
        }
    };
    match client.call(request).await? {
        Response::RepositoryOwners(owners) => {
            for owner in &owners.owners {
                println!("{}", print::repository_owner(owner));
            }
            Ok(())
        }
        other => Err(unexpected("the repository owners", &other)),
    }
}

async fn host(client: &Client, command: HostCommand) -> Result<(), CliError> {
    use pb::request::Kind as Request;
    use pb::response::Kind as Response;
    let saved = |response| match response {
        Response::HostSaved(pb::HostSaved { host: Some(host) }) => {
            println!("{}", print::host(&host));
            Ok(())
        }
        other => Err(unexpected("the host", &other)),
    };
    match command {
        HostCommand::Add {
            name,
            destination,
            ssh_config,
            folder,
        } => {
            let ssh_config = match ssh_config {
                Some(path) => std::path::absolute(&path)
                    .map_err(|source| CliError::Resolve { path, source })?
                    .display()
                    .to_string(),
                None => String::new(),
            };
            let request = Request::AddHost(pb::AddHost {
                name,
                ssh_destination: destination,
                ssh_config,
                folder,
            });
            saved(client.call(request).await?)?;
        }
        HostCommand::List => match client.call(Request::ListHosts(pb::ListHosts {})).await? {
            Response::Hosts(hosts) => {
                for host in &hosts.hosts {
                    println!("{}", print::host(host));
                }
            }
            other => return Err(unexpected("the hosts", &other)),
        },
        HostCommand::Remove { name } => {
            expect_ok(
                client
                    .call(Request::RemoveHost(pb::RemoveHost { name }))
                    .await?,
                |response| matches!(response, Response::HostRemoved(_)),
                "HostRemoved",
            )?;
        }
        HostCommand::Check { name } => {
            match client
                .call(Request::CheckHost(pb::CheckHost { name }))
                .await?
            {
                Response::HostHealth(health) => {
                    println!("{}", print::health(&health));
                    if !health.reachable {
                        return Err(CliError::Unreachable { host: health.name });
                    }
                }
                other => return Err(unexpected("the host's health", &other)),
            }
        }
        HostCommand::Install { name, binary } => {
            let binary = match binary {
                Some(path) => std::path::absolute(&path)
                    .map_err(|source| CliError::Resolve { path, source })?
                    .display()
                    .to_string(),
                None => String::new(),
            };
            let request = Request::InstallHost(pb::InstallHost { name, binary });
            match client.call(request).await? {
                Response::HostInstalled(installed) => println!("{}", installed.program_version),
                other => return Err(unexpected("the installed version", &other)),
            }
        }
        HostCommand::Env {
            name,
            set,
            copy,
            unset,
        } => {
            let mut set_env = Vec::new();
            for pair in set {
                let Some((var, value)) = pair.split_once('=') else {
                    return Err(CliError::NotAVariable { given: pair });
                };
                set_env.push(pb::EnvVar {
                    name: var.to_owned(),
                    value: value.to_owned(),
                });
            }
            for var in copy {
                let value =
                    std::env::var(&var).map_err(|_| CliError::NoVariable { name: var.clone() })?;
                set_env.push(pb::EnvVar { name: var, value });
            }
            let request = Request::UpdateHost(pb::UpdateHost {
                name,
                set_env,
                unset_env: unset,
                setup_script: None,
            });
            saved(client.call(request).await?)?;
        }
        HostCommand::Setup { name, script } => {
            let script = read_spec(&script)?;
            let request = Request::UpdateHost(pb::UpdateHost {
                name,
                setup_script: Some(script),
                ..Default::default()
            });
            saved(client.call(request).await?)?;
        }
        HostCommand::SshHosts => {
            match client
                .call(Request::ListSshHosts(pb::ListSshHosts {}))
                .await?
            {
                Response::SshHosts(hosts) => {
                    for name in &hosts.names {
                        println!("{name}");
                    }
                }
                other => return Err(unexpected("the SSH hosts", &other)),
            }
        }
    }
    Ok(())
}

fn read_spec(path: &std::path::Path) -> Result<String, CliError> {
    let read = if path == std::path::Path::new("-") {
        std::io::read_to_string(std::io::stdin())
    } else {
        std::fs::read_to_string(path)
    };
    read.map_err(|source| CliError::ReadSpec {
        path: path.to_owned(),
        source,
    })
}

/// A `--repository` value: `PATH` or `NAME=PATH`, made absolute.
/// Skills given as `NAME=FILE`, read from their files.
fn read_skills(given: &[String]) -> Result<Vec<pb::Skill>, CliError> {
    given
        .iter()
        .map(|given| {
            let (name, file) = given
                .split_once('=')
                .filter(|(name, file)| !name.trim().is_empty() && !file.is_empty())
                .ok_or_else(|| CliError::BadSkill {
                    given: given.clone(),
                })?;
            let path = PathBuf::from(file);
            let text = std::fs::read_to_string(&path)
                .map_err(|source| CliError::ReadFile { path, source })?;
            Ok(pb::Skill {
                name: name.trim().to_owned(),
                text,
            })
        })
        .collect()
}

async fn approval(client: &Client, command: ApprovalCommand) -> Result<(), CliError> {
    match command {
        ApprovalCommand::List => {
            match client
                .call(pb::request::Kind::ListApprovals(pb::ListApprovals {}))
                .await?
            {
                pb::response::Kind::Approvals(listed) => {
                    for approval in &listed.approvals {
                        println!("{}", print::approval(approval));
                    }
                    for grant in &listed.grants {
                        println!("grant\t{}", print::grant(grant));
                    }
                }
                other => return Err(unexpected("the approvals", &other)),
            }
        }
        ApprovalCommand::Status { id } => {
            let approval = client
                .wait_for(&id, Wait::For(std::time::Duration::ZERO))
                .await?;
            println!("{}", print::approval(&approval));
        }
        ApprovalCommand::Wait { id } => {
            let approval = client.wait_for(&id, Wait::UntilDecided).await?;
            println!("{}", print::approval(&approval));
            approved::outcome(*approval)?;
        }
    }
    Ok(())
}

async fn proposal(client: &Client, command: ProposalCommand) -> Result<(), CliError> {
    use pb::request::Kind as Request;
    use pb::response::Kind as Response;
    match command {
        ProposalCommand::List { session, project } => {
            let request = Request::ListProposals(pb::ListProposals {
                session_id: session.unwrap_or_default(),
                project_id: project.unwrap_or_default(),
            });
            match client.call(request).await? {
                Response::Proposals(proposals) => {
                    for proposal in &proposals.proposals {
                        println!("{}", print::proposal(proposal));
                    }
                }
                other => return Err(unexpected("the proposals", &other)),
            }
        }
        ProposalCommand::Apply {
            session,
            proposal,
            title,
            spec,
        } => {
            let spec = spec
                .map(|path| {
                    std::fs::read_to_string(&path)
                        .map_err(|source| CliError::ReadFile { path, source })
                })
                .transpose()?;
            let request = Request::ApplyProposal(pb::ApplyProposal {
                session_id: session,
                proposal_id: proposal,
                title,
                spec,
            });
            match client.call(request).await? {
                Response::ProposalApplied(pb::ProposalApplied {
                    item: Some(item), ..
                }) => println!("{}", item.id),
                other => return Err(unexpected("the applied proposal", &other)),
            }
        }
        ProposalCommand::Decline { session, proposal } => {
            let request = Request::DeclineProposal(pb::DeclineProposal {
                session_id: session,
                proposal_id: proposal,
            });
            match client.call(request).await? {
                Response::ProposalDeclined(_) => {}
                other => return Err(unexpected("the declined proposal", &other)),
            }
        }
    }
    Ok(())
}

fn repository(given: &str) -> Result<pb::Repository, CliError> {
    let (name, path) = match given.split_once('=') {
        Some((name, path)) if !name.contains('/') => (name, path),
        _ => ("", given),
    };
    if is_url(path) {
        return Ok(pb::Repository {
            name: name.to_owned(),
            path: String::new(),
            url: path.to_owned(),
        });
    }
    let path = PathBuf::from(path);
    let path = std::path::absolute(&path).map_err(|source| CliError::Resolve { path, source })?;
    Ok(pb::Repository {
        name: name.to_owned(),
        path: path.display().to_string(),
        url: String::new(),
    })
}

/// Whether git takes `given` as a URL rather than a folder: `scheme://…`,
/// or scp's `user@host:path`.
fn is_url(given: &str) -> bool {
    if given.contains("://") {
        return true;
    }
    match given.split_once(':') {
        Some((before, _)) => before.contains('@') && !before.contains('/'),
        None => false,
    }
}

fn printed(event: &pb::SessionEvent) {
    println!("{}", print::event(event));
}

async fn update(client: &Client, update: pb::UpdateSession) -> Result<(), CliError> {
    match client
        .call(pb::request::Kind::UpdateSession(update))
        .await?
    {
        pb::response::Kind::SessionUpdated(_) => Ok(()),
        other => Err(unexpected("the updated session", &other)),
    }
}

async fn history(client: &Client, session: &str) -> Result<Vec<pb::SessionEvent>, CliError> {
    let request = pb::request::Kind::ReadHistory(pb::ReadHistory {
        session_id: session.to_owned(),
    });
    match client.call(request).await? {
        pb::response::Kind::History(history) => Ok(history.events),
        other => Err(unexpected("the history", &other)),
    }
}

/// Makes `request`, then hands each of the session's new events to
/// `on_event` until it is idle again or waits for a person to answer a
/// permission request.
async fn call_and_wait(
    client: &Client,
    session: &str,
    request: pb::request::Kind,
    answered: impl Fn(&pb::response::Kind) -> bool,
    expected: &'static str,
    mut on_event: impl FnMut(&pb::SessionEvent),
) -> Result<(), CliError> {
    // What is already there isn't new: the watch starts after it. A history
    // since cut back starts over, and its events already seen are skipped.
    let history = history(client, session).await?;
    let after_event_id = history
        .last()
        .map(|event| event.id.clone())
        .unwrap_or_default();
    let seen: HashSet<String> = history.into_iter().map(|event| event.id).collect();
    let watch = pb::request::Kind::WatchSession(pb::WatchSession {
        session_id: session.to_owned(),
        after_event_id,
    });
    let (_, mut stream) = client.stream(watch).await?;
    expect_ok(client.call(request).await?, answered, expected)?;
    while let Some(item) = stream.next().await {
        let Some(event) = session_event(item)? else {
            continue;
        };
        if seen.contains(&event.id) {
            continue;
        }
        on_event(&event);
        match &event.kind {
            Some(pb::session_event::Kind::Idle(_)) => return Ok(()),
            Some(pb::session_event::Kind::PermissionRequested(asked))
                if waits_for_a_person(client, session, &asked.request_id).await? =>
            {
                return Ok(());
            }
            _ => {}
        }
    }
    Err(CliError::WatchEnded)
}

/// Whether `request` is still unanswered. A request the approval mode
/// allows is answered in the same write that records it, so the history
/// already holds its answer.
async fn waits_for_a_person(
    client: &Client,
    session: &str,
    request: &str,
) -> Result<bool, CliError> {
    let answered = history(client, session).await?.iter().any(|event| {
        matches!(
            &event.kind,
            Some(pb::session_event::Kind::PermissionCompleted(completed))
                if completed.request_id == request
        )
    });
    Ok(!answered)
}

/// The event a watch's item carries; None for its notes on how far the
/// history is read.
fn session_event(
    item: Result<pb::StreamItem, pb::Error>,
) -> Result<Option<pb::SessionEvent>, CliError> {
    match item {
        Ok(pb::StreamItem {
            kind: Some(pb::stream_item::Kind::SessionEvent(event)),
        }) => Ok(Some(event)),
        Ok(pb::StreamItem {
            kind:
                Some(
                    pb::stream_item::Kind::HistoryLoading(_)
                    | pb::stream_item::Kind::HistoryLoaded(_),
                ),
        }) => Ok(None),
        Ok(other) => Err(CliError::Wire(humata_wire::ClientError::Unexpected {
            expected: "a session event",
            got: format!("{other:?}"),
        })),
        Err(error) => Err(CliError::Wire(humata_wire::ClientError::Refused {
            code: error.code(),
            message: error.message,
        })),
    }
}

fn file_ref(file: &FileArgs) -> pb::FileRef {
    pb::FileRef {
        repository: file.repository.clone(),
        path: file.path.clone(),
    }
}

async fn edit_changes(
    client: &Client,
    edit: EditArgs,
    action: pb::ChangeAction,
) -> Result<(), CliError> {
    let request = pb::request::Kind::EditChanges(pb::EditChanges {
        session_id: edit.file.session.clone(),
        file: Some(file_ref(&edit.file)),
        action: action.into(),
        hunk: edit.hunk.map(|(old_start, new_start)| pb::HunkAt {
            old_start,
            new_start,
            context: edit.context,
        }),
    });
    expect_ok(
        client.call(request).await?,
        |response| matches!(response, pb::response::Kind::ChangesEdited(_)),
        "ChangesEdited",
    )
}

async fn review(client: &Client, command: ReviewCommand) -> Result<(), CliError> {
    use pb::request::Kind as Request;
    use pb::response::Kind as Response;
    let saved = |response: Response| match response {
        Response::ThreadSaved(pb::ThreadSaved {
            thread: Some(thread),
        }) => Ok(thread),
        other => Err(unexpected("the thread", &other)),
    };
    match command {
        ReviewCommand::Comment {
            file,
            line,
            end,
            quote,
            text,
        } => {
            let request = Request::CreateThread(pb::CreateThread {
                session_id: file.session.clone(),
                file: Some(file_ref(&file)),
                start_line: line,
                end_line: end.unwrap_or(line),
                quote,
                text,
            });
            println!("{}", saved(client.call(request).await?)?.id);
        }
        ReviewCommand::Reply { thread, text } => {
            let request = Request::ReplyToThread(pb::ReplyToThread {
                thread_id: thread,
                text,
            });
            saved(client.call(request).await?)?;
        }
        ReviewCommand::Resolve { thread, undo } => {
            let request = Request::ResolveThread(pb::ResolveThread {
                thread_id: thread,
                resolved: !undo,
            });
            saved(client.call(request).await?)?;
        }
        ReviewCommand::Delete { comment } => {
            let request = Request::DeleteComment(pb::DeleteComment {
                comment_id: comment,
            });
            expect_ok(
                client.call(request).await?,
                |response| matches!(response, Response::CommentDeleted(_)),
                "CommentDeleted",
            )?;
        }
        ReviewCommand::Threads { session } => {
            let request = Request::ListThreads(pb::ListThreads {
                session_id: session,
            });
            match client.call(request).await? {
                Response::Threads(threads) => {
                    for thread in &threads.threads {
                        print!("{}", print::thread(thread));
                    }
                }
                other => return Err(unexpected("the threads", &other)),
            }
        }
        ReviewCommand::Send { session } => {
            let request = Request::SendReview(pb::SendReview {
                session_id: session,
            });
            match client.call(request).await? {
                Response::ReviewSent(sent) => println!("{}", sent.comments),
                other => return Err(unexpected("the review sent", &other)),
            }
        }
        ReviewCommand::Viewed { file, undo } => {
            // Viewed as it is now: with the contents the diff shows.
            let request = Request::ReadChanges(pb::ReadChanges {
                session_id: file.session.clone(),
                file: Some(file_ref(&file)),
                ..Default::default()
            });
            let blob = match client.call(request).await? {
                Response::Changes(changes) => changes
                    .repositories
                    .iter()
                    .flat_map(|repository| &repository.files)
                    .map(|diff| diff.blob.clone())
                    .next()
                    .unwrap_or_default(),
                other => return Err(unexpected("the changes", &other)),
            };
            let request = Request::MarkViewed(pb::MarkViewed {
                session_id: file.session.clone(),
                file: Some(file_ref(&file)),
                blob,
                viewed: !undo,
            });
            expect_ok(
                client.call(request).await?,
                |response| matches!(response, Response::FileViewed(_)),
                "FileViewed",
            )?;
        }
    }
    Ok(())
}

fn expect_ok(
    response: pb::response::Kind,
    ok: impl Fn(&pb::response::Kind) -> bool,
    expected: &'static str,
) -> Result<(), CliError> {
    if ok(&response) {
        Ok(())
    } else {
        Err(unexpected(expected, &response))
    }
}

fn unexpected(expected: &'static str, got: &pb::response::Kind) -> CliError {
    CliError::Wire(humata_wire::ClientError::Unexpected {
        expected,
        got: format!("{got:?}"),
    })
}
