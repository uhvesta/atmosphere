use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

/// Humata's daemon, the host role that runs sessions on any machine, and the
/// MCP server agents use.
#[derive(Parser)]
#[command(name = "humatad", version)]
struct Arguments {
    #[command(subcommand)]
    role: Role,
}

#[derive(Subcommand)]
enum Role {
    /// Serve the app and the CLI on a local socket.
    Daemon {
        /// Where to keep sessions and settings.
        #[arg(long)]
        state: Option<PathBuf>,
        /// Where to keep sessions' copies of their code; defaults to
        /// $HUMATA_CACHE, then the system's cache folder.
        #[arg(long)]
        cache: Option<PathBuf>,
        /// The socket to listen on; defaults to $HUMATA_SOCKET, then a path in
        /// your home folder.
        #[arg(long)]
        socket: Option<PathBuf>,
        /// The hosts file; defaults to hosts.json in the state folder.
        #[arg(long)]
        hosts: Option<PathBuf>,
    },
    /// Run sessions on this machine for a daemon, over stdin and stdout.
    Host {
        /// Speak on stdin and stdout. Nothing else is supported yet.
        #[arg(long, required = true)]
        stdio: bool,
        /// Where to keep sessions.
        #[arg(long)]
        state: Option<PathBuf>,
        /// Where to keep sessions' copies of their code; defaults to
        /// $HUMATA_CACHE, then the system's cache folder.
        #[arg(long)]
        cache: Option<PathBuf>,
    },
    /// Serve the session in $HUMATA_SESSION's agent the board and review
    /// tools, over MCP on stdin and stdout. Humata starts it for the agent.
    Mcp {
        /// The daemon's socket; defaults to $HUMATA_SOCKET, then a path in
        /// your home folder.
        #[arg(long)]
        socket: Option<PathBuf>,
    },
}

fn main() -> ExitCode {
    let arguments = Arguments::parse();
    if matches!(arguments.role, Role::Daemon { .. } | Role::Host { .. }) {
        match humatad::login_environment() {
            Ok(variables) => {
                for (name, value) in variables {
                    // SAFETY: nothing else runs yet; the runtime and its
                    // threads start below.
                    unsafe { std::env::set_var(name, value) };
                }
            }
            Err(error) => eprintln!("humatad: {}", humata_wire::describe(&error)),
        }
    }
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("humatad: couldn't start: {error}");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(run(arguments.role)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("humatad: {}", humata_wire::describe(&error));
            ExitCode::FAILURE
        }
    }
}

async fn run(role: Role) -> Result<(), humatad::StartError> {
    match role {
        Role::Mcp { socket } => {
            let socket = socket
                .or_else(humata_wire::default_socket)
                .ok_or(humatad::StartError::NoHome)?;
            let session = std::env::var("HUMATA_SESSION").ok();
            humatad::mcp_stdio(&socket, session)
                .await
                .map_err(Into::into)
        }
        Role::Daemon {
            state,
            cache,
            socket,
            hosts,
        } => {
            let state = match state {
                Some(state) => state,
                None => humatad::default_state()?,
            };
            let socket = socket
                .or_else(humata_wire::default_socket)
                .ok_or(humatad::StartError::NoHome)?;
            let hosts = hosts.unwrap_or_else(|| state.join("hosts.json"));
            let cache = match cache {
                Some(cache) => cache,
                None => humatad::default_cache()?,
            };
            humatad::daemon(&state, &cache, &socket, &hosts).await
        }
        Role::Host {
            stdio: _,
            state,
            cache,
        } => {
            let state = match state {
                Some(state) => state,
                None => humatad::default_state()?.join("host"),
            };
            let cache = match cache {
                Some(cache) => cache,
                None => humatad::default_cache()?,
            };
            humatad::host_stdio(&state, &cache).await
        }
    }
}
