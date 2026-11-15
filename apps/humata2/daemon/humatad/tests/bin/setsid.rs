//! `setsid <program> <args>…`: runs the program in a terminal session of
//! its own, with no controlling terminal, as a process that escaped its
//! terminal would. macOS has no setsid(1); the CLI login tests use this.

use std::os::unix::process::CommandExt;
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let Some(program) = args.next() else {
        eprintln!("usage: setsid <program> [args…]");
        return ExitCode::from(2);
    };
    if let Err(error) = rustix::process::setsid() {
        eprintln!("setsid: couldn't start a terminal session: {error}");
        return ExitCode::from(1);
    }
    let error = Command::new(&program).args(args).exec();
    eprintln!("setsid: couldn't run {}: {error}", program.to_string_lossy());
    ExitCode::from(127)
}
