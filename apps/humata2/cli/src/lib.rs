//! `humata`, Humata 2's command for people and their scripts (CLI.md).
//! Each command is one gRPC operation on the daemon, sent through the
//! [`Daemon`] trait so tests can stand in for humatad.

mod args;
mod call;
mod commands;
mod console;
mod daemon;
mod error;
pub mod exit;
mod as_json;
mod live;
mod location;
mod login;
mod output;
mod principal;
mod resolve;
mod socket;

use std::ffi::OsString;
use std::future::Future;

use clap::Parser;
use clap::error::ErrorKind;
use serde_json::json;

pub use crate::console::{Console, Keyboard};
pub use crate::daemon::{Daemon, DaemonError, Replies, ReportReply, ReportRequest, Rpc, rpc};
pub use crate::principal::{Environment, Principal, principal};
pub use crate::socket::{candidates as socket_candidates, find as find_socket};

use crate::args::Cli;
use crate::call::Ctx;
use crate::console::StdConsole;
use crate::error::CliError;
use crate::live::LiveDaemon;

/// The process: parse, find the daemon, run, and the exit code.
pub async fn main(args: Vec<OsString>) -> i32 {
    let console = StdConsole;
    let cli = match Cli::try_parse_from(&args) {
        Ok(cli) => cli,
        Err(error) => return usage(&console, &args, &error),
    };
    let environment = Environment::from_process();
    let principal = principal(&environment);
    let daemon = match connect(&environment, &principal).await {
        Ok(daemon) => daemon,
        Err(error) => {
            let operation = commands::operation(&cli.command);
            return output::finish(&console, &cli.global, operation, Err(error.into()));
        }
    };
    run(cli, &daemon, &console, &environment, ctrl_c()).await
}

/// One run against a daemon and console, as the process does it after
/// connecting: `args` start with the program name. The principal follows
/// from `environment` (CLI.md §2); `interrupt` resolving stops the run with
/// exit 130.
pub async fn run_args<D: Daemon, C: Console>(
    args: Vec<OsString>,
    environment: &Environment,
    daemon: &D,
    console: &C,
    interrupt: impl Future<Output = ()>,
) -> i32 {
    match Cli::try_parse_from(&args) {
        Ok(cli) => run(cli, daemon, console, environment, interrupt).await,
        Err(error) => usage(console, &args, &error),
    }
}

async fn run<D: Daemon, C: Console>(
    cli: Cli,
    daemon: &D,
    console: &C,
    environment: &Environment,
    interrupt: impl Future<Output = ()>,
) -> i32 {
    let operation = commands::operation(&cli.command);
    let principal = principal(environment);
    let ctx = Ctx::new(daemon, console, &cli.global, &principal, environment);
    let result = tokio::select! {
        result = commands::run(&ctx, cli.command) => result,
        () = interrupt => Err(CliError::Interrupted),
    };
    output::finish(console, &cli.global, operation, result)
}

/// A command line clap refused, or `--help` and `--version`. With `--json`
/// anywhere in it, the refusal is the usual error object on stdout.
fn usage(console: &impl Console, args: &[OsString], error: &clap::Error) -> i32 {
    let code = error.exit_code();
    match error.kind() {
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
            console.print(error.render().to_string().trim_end());
            return code;
        }
        _ => {}
    }
    let text = error.render().to_string();
    if args.iter().skip(1).any(|arg| arg == "--json") {
        let message = text
            .lines()
            .next()
            .unwrap_or_default()
            .trim_start_matches("error: ")
            .to_owned();
        console.print(
            &json!({
                "ok": false,
                "operation": null,
                "error": {
                    "kind": "invalid_field",
                    "message": message,
                    "fix": "Fix the command line; `humata --help` lists the commands and `humata <command> --help` what one takes.",
                    "details": {},
                },
            })
            .to_string(),
        );
    } else {
        console.eprint(text.trim_end());
    }
    exit::USAGE
}

async fn connect(environment: &Environment, principal: &Principal) -> Result<LiveDaemon, DaemonError> {
    let socket = socket::find(environment, |path| path.exists())?;
    LiveDaemon::connect(socket, principal).await
}

/// Ctrl-C; never, if the signal can't be watched.
async fn ctrl_c() {
    if tokio::signal::ctrl_c().await.is_err() {
        std::future::pending::<()>().await;
    }
}
