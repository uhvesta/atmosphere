//! `humatad2`: Humata 2's daemon. `serve` answers the gRPC API (GRPC.md) on
//! a Unix socket, or with `--stdio` the peer link the Mac starts over SSH;
//! `bridge` pipes stdin and stdout to that socket, so SSH or a provider's
//! connect command can carry a client's calls; `mcp` is the MCP server an
//! agent starts (MCP.md), a client of that socket.

mod access_service;
#[cfg(feature = "svc_catalog")]
pub(crate) use humata2_svc_catalog::{agent_gate, agent_service};
#[cfg(not(feature = "svc_catalog"))]
mod agent_gate;
pub(crate) use humata2_daemon_runtime::agent_switches;
mod attach;
mod wire_away;
pub(crate) use humata2_svc_away as away;
#[cfg(feature = "svc_board")]
mod wire_board;
#[cfg(feature = "svc_board")]
pub(crate) use humata2_svc_board as board_service;
mod bridge;
mod wire_transfers;
pub(crate) use humata2_svc_transfers::{carry, carry_service, transfer_end_service, transfer_service};
mod channel_service;
mod cli_login;
pub(crate) use humata2_rpc_kit::compression;
mod contained;
mod wire_stop_restart;
pub(crate) use humata2_svc_stop_restart::{daemon, doctor, restart, stop};
mod wire_desk;
pub(crate) use humata2_svc_desk::desk_service;
mod error;
mod fault;
mod events;
mod wire_files;
pub(crate) use humata2_svc_files as files;
mod grant_target;
mod guard;
mod history_service;
mod holder_end;
mod holders;
mod wire_peer;
pub(crate) use humata2_svc_peer::{
    host_relay, peer, peer_client, peer_link, peer_reports, peer_skills, relay, relay_auth,
    remote_end, remote_work,
};
mod host_vars;
mod identity;
mod wire_inbox;
pub(crate) use wire_inbox as inbox;
pub(crate) use humata2_svc_inbox::screen_ask;
mod wire_channels;
mod wire_terminal;
pub(crate) use humata2_svc_terminal::{remote_terminal, running_holders, terminal_session};
mod input_gate;
mod last_good;
pub(crate) use humata2_rpc_kit::link_proof;
pub(crate) use humata2_rpc_kit::log;
mod login_service;
mod wire_machines;
pub(crate) use humata2_svc_machines::{machine_scripts, machine_service};
mod mark_service;
#[cfg(feature = "svc_mcp")]
#[path = "wire_mcp.rs"]
mod mcp;
mod not_built;
mod operations;
pub(crate) use humata2_daemon_runtime::paste;
mod provider_service;
mod read_marks;
pub(crate) use humata2_daemon_runtime::ready;
mod settings_service;
// REPOS (RL-5): the repo library and the one repo search.
mod repo_service;
#[cfg(feature = "svc_review")]
mod wire_review;
#[cfg(feature = "svc_review")]
pub(crate) use humata2_svc_review as review;
mod rules_kept;
pub(crate) use humata2_daemon_runtime::screen_question;
mod script_tokens;
mod serve;
mod start_message;
mod start_ready;
mod session_service;
mod skills_service;
mod status;
mod storage;
pub(crate) use humata2_rpc_kit::sync;
mod terminal;
pub(crate) use humata2_daemon_runtime::terminal_uuid;
pub(crate) use humata2_daemon_runtime::tool_path;
mod trash_service;
mod wire_upgrade;
pub(crate) use humata2_svc_upgrade as upgrade;

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
    #[cfg(feature = "svc_mcp")]
    Mcp(mcp::McpArgs),
    /// Stops this machine's sessions, even when its daemon doesn't answer
    /// (D262).
    Stop(stop::cli::StopArgs),
}

fn main() -> ExitCode {
    if cfg!(feature = "admin") {
        humata2_rpc_kit::admin::set_admin_mode(true);
    }
    let cli = Cli::parse();
    // Before the runtime starts any thread: this sets PATH.
    if matches!(cli.command, Command::Serve(_)) && tool_path::asked() {
        tool_path::widen_from_login_shell();
    }
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
                    #[cfg(feature = "svc_mcp")]
                    Command::Mcp(args) => mcp::run(args).await,
                    Command::Stop(args) => stop::cli::run(args)
                        .await
                        .map_err(|source| DaemonError::StopCli { source }),
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
