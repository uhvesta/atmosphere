//! `humatad2`: Humata 2's daemon. `serve` answers the gRPC API (GRPC.md) on
//! a Unix socket, or with `--stdio` the peer link the Mac starts over SSH;
//! `bridge` pipes stdin and stdout to that socket, so SSH or a provider's
//! connect command can carry a client's calls; `mcp` is the MCP server an
//! agent starts (MCP.md), a client of that socket.

mod access_service;
mod agent_gate;
mod agent_service;
mod agent_switches;
mod attach;
mod away;
mod board_service;
mod bridge;
mod carry;
mod carry_service;
mod channel_service;
mod cli_login;
mod compression;
mod contained;
mod daemon;
mod desk_service;
mod doctor;
mod error;
mod fault;
mod events;
mod files;
mod grant_target;
mod guard;
mod history_service;
mod holder_end;
mod holders;
mod host_relay;
mod host_vars;
mod identity;
mod inbox;
mod input_gate;
mod last_good;
mod link_proof;
mod log;
mod login_service;
mod machine_scripts;
mod machine_service;
mod mark_service;
mod mcp;
mod not_built;
mod operations;
mod paste;
mod peer;
mod peer_client;
mod peer_link;
mod peer_reports;
mod peer_skills;
mod provider_service;
mod read_marks;
mod ready;
mod settings_service;
mod relay;
mod relay_auth;
mod remote_end;
mod remote_terminal;
mod remote_work;
// REPOS (RL-5): the repo library and the one repo search.
mod repo_service;
mod restart;
mod review;
mod rules_kept;
mod screen_ask;
mod screen_question;
mod script_tokens;
mod serve;
mod start_message;
mod session_service;
mod skills_service;
mod status;
mod stop;
mod storage;
mod sync;
mod terminal;
mod terminal_session;
mod terminal_uuid;
mod transfer_end_service;
mod transfer_service;
mod trash_service;
mod upgrade;

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
    /// Stops this machine's sessions, even when its daemon doesn't answer
    /// (D262).
    Stop(stop::cli::StopArgs),
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|source| DaemonError::Runtime { source })
        .and_then(|runtime| {
            let result = runtime.block_on(async move {
                // Test builds with `faults`: SIGUSR2 is a power cut.
                fault::power_cut_on_sigusr2();
                match cli.command {
                    Command::Serve(args) => serve::run(args).await,
                    Command::Bridge(args) => bridge::run(args).await,
                    Command::Mcp(args) => mcp::run(args).await,
                    Command::Stop(args) => stop::cli::run(args).await,
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
