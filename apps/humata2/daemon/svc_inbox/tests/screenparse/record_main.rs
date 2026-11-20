//! `screenparse_record`: runs an agent CLI under the real holder, follows
//! a step script, and writes labelled screen-text snapshots.
//!
//! Run with `bazel run //apps/humata2/daemon/svc_inbox:screenparse_record -- <flags>`.
//! Relative `--script`, `--out` and `--cwd` are taken from the workspace.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use humata2_screenparse::{Options, RecordError, record};

const USAGE: &str = "usage: screenparse_record --agent <claude|codex|copilot|agy> --cli-version <v> --program <path> [--arg <a>]... --cwd <dir> [--env K=V]... --script <file> --out <dir> [--cols 80] [--rows 40] [--widths 60,80,120,200] [--holder <path>]";

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("{reason}\n{USAGE}")]
    Args { reason: String },
    #[error("couldn't start the async runtime. Check the machine's thread limits and run again.")]
    Runtime(#[source] std::io::Error),
    #[error("couldn't find this program's own path, so the holder can't be located. Pass --holder <path>.")]
    Executable(#[source] std::io::Error),
    #[error(transparent)]
    Record(#[from] RecordError),
}

fn args_error(reason: impl Into<String>) -> Error {
    Error::Args {
        reason: reason.into(),
    }
}

fn parse() -> Result<Options, Error> {
    let workspace = std::env::var_os("BUILD_WORKSPACE_DIRECTORY").map(PathBuf::from);
    let resolve = |path: PathBuf| match &workspace {
        Some(root) if path.is_relative() => root.join(path),
        _ => path,
    };
    let mut agent = None;
    let mut cli = None;
    let mut program = None;
    let mut args = Vec::new();
    let mut cwd = None;
    let mut env = Vec::new();
    let mut script = None;
    let mut out = None;
    let mut cols = 80u16;
    let mut rows = 40u16;
    let mut widths = vec![60u16, 80, 120, 200];
    let mut holder = None;
    let mut input = std::env::args_os().skip(1);
    while let Some(flag) = input.next() {
        let flag = flag.to_string_lossy().into_owned();
        let value = input
            .next()
            .ok_or_else(|| args_error(format!("missing value for {flag}")))?;
        let text = || value.to_string_lossy().into_owned();
        let number = |what: &str| -> Result<u16, Error> {
            text()
                .parse::<u16>()
                .ok()
                .filter(|n| *n > 0)
                .ok_or_else(|| args_error(format!("{what} must be a positive whole number")))
        };
        match flag.as_str() {
            "--agent" => {
                let name = text();
                if !["claude", "codex", "copilot", "agy"].contains(&name.as_str()) {
                    return Err(args_error(format!(
                        "--agent {name} isn't one of claude, codex, copilot, agy"
                    )));
                }
                agent = Some(name);
            }
            "--cli-version" => cli = Some(text()),
            "--program" => program = Some(PathBuf::from(&value)),
            "--arg" => args.push(value),
            "--cwd" => cwd = Some(resolve(PathBuf::from(&value))),
            "--env" => {
                let pair = text();
                let (key, val) = pair
                    .split_once('=')
                    .ok_or_else(|| args_error(format!("--env {pair} must look like KEY=VALUE")))?;
                env.push((OsString::from(key), OsString::from(val)));
            }
            "--script" => script = Some(resolve(PathBuf::from(&value))),
            "--out" => out = Some(resolve(PathBuf::from(&value))),
            "--cols" => cols = number("--cols")?,
            "--rows" => rows = number("--rows")?,
            "--widths" => {
                widths = text()
                    .split(',')
                    .map(|w| {
                        w.trim()
                            .parse::<u16>()
                            .ok()
                            .filter(|n| *n > 0)
                            .ok_or_else(|| {
                                args_error("--widths must be comma-separated positive numbers")
                            })
                    })
                    .collect::<Result<_, _>>()?;
            }
            "--holder" => holder = Some(resolve(PathBuf::from(&value))),
            other => return Err(args_error(format!("unknown flag {other}"))),
        }
    }
    let holder = match holder {
        Some(holder) => holder,
        None => {
            // Next to this binary in its runfiles, as the terminal lab finds it.
            let executable = std::env::current_exe().map_err(Error::Executable)?;
            let mut runfiles = executable.into_os_string();
            runfiles.push(".runfiles");
            PathBuf::from(runfiles).join("_main/apps/humata2/daemon/holder/humata2_holder")
        }
    };
    Ok(Options {
        agent: need(agent, "--agent")?,
        cli_version: need(cli, "--cli-version")?,
        program: need(program, "--program")?,
        args,
        cwd: need(cwd, "--cwd")?,
        env,
        script: need(script, "--script")?,
        out: need(out, "--out")?,
        cols,
        rows,
        widths,
        holder,
    })
}

/// `value`, or an error naming the missing `flag`.
fn need<T>(value: Option<T>, flag: &str) -> Result<T, Error> {
    value.ok_or_else(|| args_error(format!("{flag} is required")))
}

fn run() -> Result<(), Error> {
    let options = parse()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(Error::Runtime)?;
    runtime.block_on(record(&options))?;
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("screenparse_record: {error}");
            let mut source = std::error::Error::source(&error);
            while let Some(cause) = source {
                eprintln!("  caused by: {cause}");
                source = cause.source();
            }
            ExitCode::FAILURE
        }
    }
}
