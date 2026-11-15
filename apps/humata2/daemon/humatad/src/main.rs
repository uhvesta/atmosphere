//! `humatad2`: Humata 2's daemon. `serve` answers the gRPC API (GRPC.md) on
//! a Unix socket; `bridge` pipes stdin and stdout to that socket, so SSH or
//! a provider's connect command can carry it.

mod attach;
mod bridge;
mod daemon;
mod error;
mod not_built;
mod paste;
mod serve;
mod status;
mod terminal;

use std::process::ExitCode;

use clap::{Parser, Subcommand};

use crate::error::DaemonError;

#[derive(Parser)]
#[command(name = "humatad2", about = "Humata 2's daemon.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Serves Humata's gRPC API on a Unix socket until stopped.
    Serve(serve::ServeArgs),
    /// Pipes stdin and stdout to the daemon's socket.
    Bridge(bridge::BridgeArgs),
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|source| DaemonError::Runtime { source })
        .and_then(|runtime| {
            runtime.block_on(async move {
                match cli.command {
                    Command::Serve(args) => serve::run(args).await,
                    Command::Bridge(args) => bridge::run(args).await,
                }
            })
        });
    let code = match result {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("humatad2: {}", status::chain(&error));
            1
        }
    };
    // Leaves without waiting for a blocking read of stdin that will never
    // finish, which the runtime would otherwise wait for.
    std::process::exit(code)
}
