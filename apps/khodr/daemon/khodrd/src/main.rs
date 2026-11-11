use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

/// Khodr's daemon, and the host role that runs sessions on any machine.
#[derive(Parser)]
#[command(name = "khodrd", version)]
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
        /// The socket to listen on; defaults to $KHODR_SOCKET, then a path in
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
    },
}

fn main() -> ExitCode {
    let arguments = Arguments::parse();
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("khodrd: couldn't start: {error}");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(run(arguments.role)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("khodrd: {}", khodr_wire::describe(&error));
            ExitCode::FAILURE
        }
    }
}

async fn run(role: Role) -> Result<(), khodrd::StartError> {
    match role {
        Role::Daemon {
            state,
            socket,
            hosts,
        } => {
            let state = match state {
                Some(state) => state,
                None => khodrd::default_state()?,
            };
            let socket = socket
                .or_else(khodr_wire::default_socket)
                .ok_or(khodrd::StartError::NoHome)?;
            let hosts = hosts.unwrap_or_else(|| state.join("hosts.json"));
            khodrd::daemon(&state, &socket, &hosts).await
        }
        Role::Host { stdio: _, state } => {
            let state = match state {
                Some(state) => state,
                None => khodrd::default_state()?.join("host"),
            };
            khodrd::host_stdio(&state).await
        }
    }
}
