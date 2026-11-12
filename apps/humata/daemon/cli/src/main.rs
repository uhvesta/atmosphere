//! `humata`: Humata's sessions from the command line, through the daemon.

mod error;
mod print;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use humata_wire::{Client, pb};
use tokio::net::UnixStream;

use crate::error::CliError;

const VERSION: &str = env!("CARGO_PKG_VERSION");

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

/// What a session may do without asking.
#[derive(Clone, Copy, ValueEnum)]
enum Approval {
    /// Ask before every command, write and read.
    Ask,
    /// Reads and file writes go ahead; commands still ask.
    Edits,
    /// Everything goes ahead.
    Everything,
}

impl From<Approval> for pb::ApprovalMode {
    fn from(approval: Approval) -> Self {
        match approval {
            Approval::Ask => Self::AskEveryTime,
            Approval::Edits => Self::AllowEdits,
            Approval::Everything => Self::AllowEverything,
        }
    }
}

#[derive(Subcommand)]
enum ProjectCommand {
    /// Add a project of repositories on this Mac, and print its id.
    Add {
        #[arg(long)]
        name: String,
        /// A repository's folder on this Mac, as `PATH` or `NAME=PATH`; give
        /// it once for each. NAME is its folder in each session's copy, the
        /// folder's own name if not given.
        #[arg(long = "repository", required = true)]
        repositories: Vec<String>,
        /// Instructions for every session in it.
        #[arg(long, default_value = "")]
        instructions: String,
    },
    /// List the projects.
    List,
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
enum Command {
    /// Add, check, set up or list the machines sessions run on.
    #[command(subcommand)]
    Host(HostCommand),
    /// Add or list projects.
    #[command(subcommand)]
    Project(ProjectCommand),
    /// Add, change or list a project's tasks and subtasks.
    #[command(subcommand)]
    Item(ItemCommand),
    /// Read or revert a project's board changes.
    #[command(subcommand)]
    Board(BoardCommand),
    /// List every session waiting on you, across projects.
    Inbox,
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
        #[arg(long, default_value = "")]
        instructions: String,
        /// A file of settings only the backend reads, such as the scripted
        /// backend's script.
        #[arg(long)]
        backend_config: Option<PathBuf>,
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
    /// Print a session's events.
    History { session: String },
    /// Print a session's events, then each new one, until interrupted.
    Watch { session: String },
    /// List every host's sessions.
    Sessions {
        /// Only this project's.
        #[arg(long, default_value = "")]
        project: String,
    },
}

fn main() -> ExitCode {
    let arguments = Arguments::parse();
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
    match runtime.block_on(run(arguments)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("humata: {}", humata_wire::describe(&error));
            ExitCode::FAILURE
        }
    }
}

async fn connect(socket: Option<PathBuf>) -> Result<Client, CliError> {
    let path = socket
        .or_else(humata_wire::default_socket)
        .ok_or(CliError::NoSocket)?;
    let stream = UnixStream::connect(&path)
        .await
        .map_err(|source| CliError::Connect { path, source })?;
    let (reader, writer) = stream.into_split();
    Ok(Client::connect(reader, writer, pb::Role::Client, VERSION).await?)
}

async fn run(arguments: Arguments) -> Result<(), CliError> {
    use pb::request::Kind as Request;
    use pb::response::Kind as Response;
    let client = connect(arguments.socket).await?;
    match arguments.command {
        Command::Project(ProjectCommand::Add {
            name,
            repositories,
            instructions,
        }) => {
            let repositories = repositories
                .iter()
                .map(|given| repository(given))
                .collect::<Result<_, _>>()?;
            let request = Request::CreateProject(pb::CreateProject {
                name,
                repositories,
                instructions,
            });
            match client.call(request).await? {
                Response::ProjectCreated(pb::ProjectCreated {
                    project: Some(project),
                }) => println!("{}", project.id),
                other => return Err(unexpected("the created project", &other)),
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
        Command::Item(command) => item(&client, command).await?,
        Command::Board(BoardCommand::Log { project }) => {
            let request = Request::ReadBoardLog(pb::ReadBoardLog {
                project_id: project,
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
            let request = Request::RevertBoardChange(pb::RevertBoardChange { change_id: change });
            expect_ok(
                client.call(request).await?,
                |response| matches!(response, Response::BoardChangeReverted(_)),
                "BoardChangeReverted",
            )?;
        }
        Command::Host(command) => host(&client, command).await?,
        Command::Stage(edit) => edit_changes(&client, edit, pb::ChangeAction::Stage).await?,
        Command::Unstage(edit) => edit_changes(&client, edit, pb::ChangeAction::Unstage).await?,
        Command::Revert(edit) => edit_changes(&client, edit, pb::ChangeAction::Revert).await?,
        Command::Review(command) => review(&client, command).await?,
        Command::Inbox => match client.call(Request::ListInbox(pb::ListInbox {})).await? {
            Response::Inbox(inbox) => {
                for entry in &inbox.entries {
                    println!("{}", print::inbox_entry(entry));
                }
            }
            other => return Err(unexpected("the inbox", &other)),
        },
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
            instructions,
            backend_config,
        } => {
            let backend_config = match backend_config {
                Some(path) => std::fs::read_to_string(&path)
                    .map_err(|source| CliError::ReadBackendConfig { path, source })?,
                None => String::new(),
            };
            let backend = match backend {
                Backend::Scripted => pb::Backend::Scripted,
                Backend::Local => pb::Backend::Local,
                Backend::Copilot => pb::Backend::Copilot,
                Backend::Claude => pb::Backend::Claude,
            };
            let request = Request::StartSession(pb::StartSession {
                backend: backend.into(),
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
            });
            let sent = |response: &Response| matches!(response, Response::MessageSent(_));
            if wait {
                call_and_wait(&client, &session, request, sent, "MessageSent").await?;
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
                call_and_wait(&client, &session, request, approved, "Approved").await?;
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
            });
            expect_ok(
                client.call(request).await?,
                |response| matches!(response, Response::TurnAborted(_)),
                "TurnAborted",
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
        } => {
            let request = Request::ForkSession(pb::ForkSession {
                session_id: session,
                before_event_id: before,
                name,
                chat_only,
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
            });
            match client.call(request).await? {
                Response::FileContents(contents) if contents.binary => {
                    println!("{} isn't text", contents.path);
                }
                Response::FileContents(contents) => print!("{}", contents.text),
                other => return Err(unexpected("the file", &other)),
            }
        }
        Command::History { session } => {
            for event in history(&client, &session).await? {
                println!("{}", print::event(&event));
            }
        }
        Command::Watch { session } => {
            let request = Request::WatchSession(pb::WatchSession {
                session_id: session,
            });
            let (_, mut stream) = client.stream(request).await?;
            while let Some(item) = stream.next().await {
                println!("{}", print::event(&session_event(item)?));
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
            });
            expect_ok(client.call(request).await?, updated, "ItemUpdated")?;
        }
        ItemCommand::Move { item, status } => {
            let request = Request::UpdateItem(pb::UpdateItem {
                item_id: item,
                status: Some(pb::ItemStatus::from(status).into()),
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
fn repository(given: &str) -> Result<pb::Repository, CliError> {
    let (name, path) = match given.split_once('=') {
        Some((name, path)) if !name.contains('/') => (name, path),
        _ => ("", given),
    };
    let path = PathBuf::from(path);
    let path = std::path::absolute(&path).map_err(|source| CliError::Resolve { path, source })?;
    Ok(pb::Repository {
        name: name.to_owned(),
        path: path.display().to_string(),
    })
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

/// Makes `request`, then prints the session's new events until it is idle
/// again or waits for a person to answer a permission request.
async fn call_and_wait(
    client: &Client,
    session: &str,
    request: pb::request::Kind,
    answered: impl Fn(&pb::response::Kind) -> bool,
    expected: &'static str,
) -> Result<(), CliError> {
    // The watch replays the history first; what is already there isn't new.
    let seen = history(client, session).await?.len();
    let watch = pb::request::Kind::WatchSession(pb::WatchSession {
        session_id: session.to_owned(),
    });
    let (_, mut stream) = client.stream(watch).await?;
    expect_ok(client.call(request).await?, answered, expected)?;
    let mut index = 0;
    while let Some(item) = stream.next().await {
        let event = session_event(item)?;
        index += 1;
        if index <= seen {
            continue;
        }
        println!("{}", print::event(&event));
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

fn session_event(item: Result<pb::StreamItem, pb::Error>) -> Result<pb::SessionEvent, CliError> {
    match item {
        Ok(pb::StreamItem {
            kind: Some(pb::stream_item::Kind::SessionEvent(event)),
        }) => Ok(event),
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
