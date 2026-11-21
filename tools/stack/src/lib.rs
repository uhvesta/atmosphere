//! `stack`: turns the trailer-tagged commits on a branch into a linked stack
//! of GitHub pull requests, through the gh CLI. Only this package's binary
//! and tests use it.

mod body;
mod check;
mod cli;
mod clock;
mod commands;
mod config;
mod diff;
mod error;
mod gh;
mod git;
mod guide;
mod history;
mod import;
mod local;
mod parity;
mod pins;
mod plan;
mod restack;
mod state;
mod status;
mod trailers;
mod upload;

use std::error::Error as _;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser as _;

use crate::cli::{Cli, Command, ConfigCommand};
use crate::error::Error;

/// Parses the command line, runs the command and maps its error to an exit
/// code (listed in `stack --help`).
pub fn main() -> ExitCode {
    let cli = Cli::parse();
    match working_dir().and_then(|dir| run(&cli.command, &dir)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("stack: {error}");
            let mut source = error.source();
            while let Some(cause) = source {
                eprintln!("  caused by: {cause}");
                source = cause.source();
            }
            if let Some(next) = error.next_command() {
                eprintln!("Next: {next}");
            }
            ExitCode::from(error.exit_code())
        }
    }
}

fn run(command: &Command, dir: &std::path::Path) -> Result<(), Error> {
    match command {
        Command::Check(args) => check::run(args, dir),
        Command::Upload(args) => upload::run(args, dir),
        Command::Status(args) => status::run(args, dir),
        Command::Guide(args) => guide::run(args, dir),
        Command::Restack(command) => Ok(restack::run(command, dir)?),
        Command::Diff(args) => diff::run(args, dir),
        Command::Config(ConfigCommand::Show(args)) => commands::config_show(args),
        Command::Commands(args) => commands::run(args),
        Command::Import(args) => import::run(args, dir),
        Command::Init(args) => parity::init(args, dir),
        Command::Add(args) => parity::add(args, dir),
        Command::View(args) => status::run(args, dir),
        Command::Up => parity::navigate(parity::Nav::Up, dir),
        Command::Down => parity::navigate(parity::Nav::Down, dir),
        Command::Top => parity::navigate(parity::Nav::Top, dir),
        Command::Bottom => parity::navigate(parity::Nav::Bottom, dir),
        Command::Trunk(args) => parity::trunk(args, dir),
        Command::Submit(args) => upload::run_submit(args, dir),
        Command::Push(args) => upload::run_push(args, dir),
        Command::Sync(args) => upload::run_sync(args, dir),
        Command::Rebase(args) => parity::rebase(args, dir),
        Command::Link(args) => parity::link(args, dir),
    }
}

/// `bazel run` starts the binary in its runfiles; the user's checkout is in
/// BUILD_WORKING_DIRECTORY.
fn working_dir() -> Result<PathBuf, Error> {
    match std::env::var_os("BUILD_WORKING_DIRECTORY") {
        Some(dir) if !dir.is_empty() => Ok(PathBuf::from(dir)),
        _ => std::env::current_dir().map_err(|source| Error::WorkingDir { source }),
    }
}
