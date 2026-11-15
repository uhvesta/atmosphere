//! `humatad2`: Humata 2's daemon. `serve` answers the gRPC API (GRPC.md) on
//! a Unix socket, or with `--stdio` the peer link the Mac starts over SSH;
//! `bridge` pipes stdin and stdout to that socket, so SSH or a provider's
//! connect command can carry a client's calls; `mcp` is the MCP server an
//! agent starts (MCP.md), a client of that socket.

mod access_service;
mod attach;
mod bridge;
mod cli_login;
mod compression;
mod daemon;
mod error;
mod files;
mod grant_target;
mod guard;
mod holders;
mod identity;
mod inbox;
mod last_good;
mod log;
mod link_proof;
mod machine_service;
mod mcp;
mod not_built;
mod operations;
mod paste;
mod peer;
mod peer_link;
mod restart;
mod review;
mod serve;
mod status;
mod sync;
mod terminal;
mod terminal_session;

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
    /// Serves Humata's gRPC API on a Unix socket until stopped, or with
    /// --stdio the peer link on stdin and stdout until it closes.
    Serve(serve::ServeArgs),
    /// Pipes stdin and stdout to the daemon's socket, starting the daemon
    /// if none answers.
    Bridge(bridge::BridgeArgs),
    /// Serves MCP on stdin and stdout for the agent of the session whose
    /// token is in HUMATA_SESSION_TOKEN.
    Mcp(mcp::McpArgs),
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|source| DaemonError::Runtime { source })
        .and_then(|runtime| {
            let result = runtime.block_on(async move {
                match cli.command {
                    Command::Serve(args) => serve::run(args).await,
                    Command::Bridge(args) => bridge::run(args).await,
                    Command::Mcp(args) => mcp::run(args).await,
                }
            });
            // Dropping the runtime would wait for a blocking read of stdin
            // that never finishes (the bridge after its daemon goes away).
            runtime.shutdown_background();
            result
        });
    let code = match result {
        Ok(()) => 0,
        Err(error) => {
            crate::log::say!("humatad2: {}", status::chain(&error));
            1
        }
    };
    std::process::exit(code)
}
