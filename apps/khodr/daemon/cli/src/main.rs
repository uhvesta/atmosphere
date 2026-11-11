//! `khodr`: Khodr's sessions from the command line, through the daemon.

mod error;
mod print;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use khodr_wire::{Client, pb};
use tokio::net::UnixStream;

use crate::error::CliError;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser)]
#[command(
    name = "khodr",
    version,
    about = "Direct Khodr's coding agents from the command line."
)]
struct Arguments {
    /// The daemon's socket; defaults to $KHODR_SOCKET, then the path khodrd
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
    /// Add a project with one repository on this Mac, and print its id.
    Add {
        #[arg(long)]
        name: String,
        /// The repository's folder on this Mac.
        #[arg(long)]
        repository: PathBuf,
        /// Instructions for every session in it.
        #[arg(long, default_value = "")]
        instructions: String,
    },
    /// List the projects.
    List,
}

#[derive(Subcommand)]
enum Command {
    /// Add or list projects.
    #[command(subcommand)]
    Project(ProjectCommand),
    /// Start a session and print its id.
    Start {
        #[arg(long, value_enum)]
        backend: Backend,
        /// The project it belongs to. Its repository is the default
        /// directory and its instructions the default instructions.
        #[arg(long, default_value = "")]
        project: String,
        /// The folder the agent works in, on its host; required without
        /// --project.
        #[arg(long, required_unless_present = "project")]
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
    /// Fork a session and print the fork's id.
    Fork {
        session: String,
        /// Keep only the events before this one.
        #[arg(long, default_value = "")]
        before: String,
        #[arg(long, default_value = "")]
        name: String,
    },
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
            eprintln!("khodr: couldn't start: {error}");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(run(arguments)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("khodr: {}", khodr_wire::describe(&error));
            ExitCode::FAILURE
        }
    }
}

async fn connect(socket: Option<PathBuf>) -> Result<Client, CliError> {
    let path = socket
        .or_else(khodr_wire::default_socket)
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
            repository,
            instructions,
        }) => {
            let repository =
                std::path::absolute(&repository).map_err(|source| CliError::Resolve {
                    path: repository,
                    source,
                })?;
            let request = Request::CreateProject(pb::CreateProject {
                name,
                repository: repository.display().to_string(),
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
        Command::Start {
            backend,
            project,
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
        Command::Fork {
            session,
            before,
            name,
        } => {
            let request = Request::ForkSession(pb::ForkSession {
                session_id: session,
                before_event_id: before,
                name,
            });
            match client.call(request).await? {
                Response::SessionForked(pb::SessionForked {
                    session: Some(session),
                }) => println!("{}", session.id),
                other => return Err(unexpected("the forked session", &other)),
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
        Ok(other) => Err(CliError::Wire(khodr_wire::ClientError::Unexpected {
            expected: "a session event",
            got: format!("{other:?}"),
        })),
        Err(error) => Err(CliError::Wire(khodr_wire::ClientError::Refused {
            code: error.code(),
            message: error.message,
        })),
    }
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
    CliError::Wire(khodr_wire::ClientError::Unexpected {
        expected,
        got: format!("{got:?}"),
    })
}
