//! Thin binary entry point for `humatad2` and all its scoped test profiles.
//! The daemon core lives in `humatad_core` (compiled once + `_faults`); this
//! file only parses the profile's CLI flags (keeping `--admin*` and
//! `set_admin_mode(true)` strictly out of release binaries per D41) and injects
//! whichever optional service plugins this profile links.

#[cfg(feature = "svc_mcp")]
#[path = "wire_mcp.rs"]
mod mcp;

use std::process::ExitCode;

use clap::{Parser, Subcommand};
use humatad_core::error::DaemonError;
use humatad_core::plugins::Plugins;
use humatad_core::serve::{AdminPolicy, ServeArgs};
use humatad_core::{bridge, browser_host, fault, log, serve, status, stop, tool_path};

#[cfg(feature = "admin")]
#[derive(clap::Args)]
struct ProfileServeArgs {
    #[command(flatten)]
    base: ServeArgs,
    /// Test daemons only: a call with no credential acts as the user.
    /// Builds without the `admin` feature, as releases are, lack this flag.
    /// Never with --stdio: the peer link is the Mac's, not the user's.
    #[arg(long, conflicts_with = "stdio")]
    admin: bool,
    /// Test daemons only: as --admin, but only for calls from this
    /// process, so a test can play the app while every other process is
    /// judged as a release daemon judges it.
    #[arg(long, conflicts_with_all = ["stdio", "admin"])]
    admin_pid: Option<i32>,
    /// Test daemons only: every CLI login lasts this many seconds, not the
    /// hours it asked for, so tests can see one expire.
    #[arg(long, conflicts_with = "stdio")]
    cli_login_seconds: Option<u64>,
    /// How long every script token lasts instead of the days it asked for, so tests can see one expire.
    #[arg(long, conflicts_with = "stdio")]
    script_token_seconds: Option<u64>,
}

#[cfg(feature = "admin")]
impl ProfileServeArgs {
    fn into_parts(self) -> (ServeArgs, AdminPolicy) {
        let admin = match (self.admin, self.admin_pid) {
            (true, _) => Some(humatad_core::identity::Admin::Anyone),
            (false, Some(pid)) => Some(humatad_core::identity::Admin::Process(pid)),
            (false, None) => None,
        };
        (
            self.base,
            AdminPolicy {
                admin,
                cli_login_seconds: self.cli_login_seconds,
                script_token_seconds: self.script_token_seconds,
            },
        )
    }
}

#[cfg(not(feature = "admin"))]
#[derive(clap::Args)]
struct ProfileServeArgs {
    #[command(flatten)]
    base: ServeArgs,
}

#[cfg(not(feature = "admin"))]
impl ProfileServeArgs {
    fn into_parts(self) -> (ServeArgs, AdminPolicy) {
        (self.base, AdminPolicy::disabled())
    }
}

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
    Serve(ProfileServeArgs),
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
    /// Runs the WebExtension Native Messaging host bridging stdio to the daemon.
    BrowserHost(browser_host::BrowserHostArgs),
}

fn build_plugins() -> Plugins {
    Plugins {
        #[cfg(feature = "svc_catalog")]
        catalog: Some(Box::new(humatad_plugin_catalog::CatalogPlugin)),
        #[cfg(not(feature = "svc_catalog"))]
        catalog: None,
        #[cfg(feature = "svc_board")]
        board: Some(Box::new(humatad_plugin_board::BoardPlugin)),
        #[cfg(not(feature = "svc_board"))]
        board: None,
        #[cfg(feature = "svc_review")]
        review: Some(Box::new(humatad_plugin_review::ReviewPlugin)),
        #[cfg(not(feature = "svc_review"))]
        review: None,
    }
}

fn main() -> ExitCode {
    #[cfg(feature = "admin")]
    humata2_rpc_kit::admin::set_admin_mode(true);
    let command = if browser_host::invoked_as_browser_host() {
        Command::BrowserHost(browser_host::args_from_argv0())
    } else {
        Cli::parse().command
    };
    // Before the runtime starts any thread: this sets PATH.
    if matches!(command, Command::Serve(_)) && tool_path::asked() {
        tool_path::widen_from_login_shell();
    }
    // Before the runtime starts any thread: double-fork and detach when asked.
    if let Command::Serve(ref args) = command
        && args.base.daemonize
        && let Err(error) = humatad_core::daemonize::detach(args.base.daemonize_log_path().as_deref())
    {
        log::say!("humatad2: {}", status::chain(&error));
        std::process::exit(1);
    }
    let mut builder = if matches!(command, Command::Serve(_)) {
        let mut builder = tokio::runtime::Builder::new_multi_thread();
        builder.worker_threads(4);
        builder
    } else {
        tokio::runtime::Builder::new_current_thread()
    };
    let result = builder
        .enable_all()
        .build()
        .map_err(|source| DaemonError::Runtime { source })
        .and_then(|runtime| {
            let result = runtime.block_on(async move {
                // Test builds with `faults`: SIGUSR2 is a power cut.
                fault::power_cut_on_sigusr2();
                match command {
                    Command::Serve(args) => {
                        let (base, admin_policy) = args.into_parts();
                        serve::run(base, admin_policy, build_plugins()).await
                    }
                    Command::Bridge(args) => bridge::run(args).await,
                    #[cfg(feature = "svc_mcp")]
                    Command::Mcp(args) => mcp::run(args).await,
                    Command::Stop(args) => stop::cli::run(args)
                        .await
                        .map_err(|source| DaemonError::StopCli { source }),
                    Command::BrowserHost(args) => browser_host::run(args).await,
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
            log::say!("humatad2: {}", status::chain(&error));
            1
        }
    };
    std::process::exit(code)
}
