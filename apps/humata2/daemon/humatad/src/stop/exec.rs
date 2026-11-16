//! The daemon starting itself again, in the same process, after a stop or a
//! start (D261, D265): its next run reads the stop record and sweeps, or
//! brings everything back.
//!
//! The state lock and the listening socket are close-on-exec, so the new
//! image takes both again; a socket file left behind is a dead one, which a
//! boot removes as it does after a crash.

use std::convert::Infallible;
use std::ffi::{OsStr, OsString};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub(crate) enum ExecError {
    #[error(
        "couldn't find this program's own file to start it again: {source}; the change is saved, so start humatad2 by hand (`humatad2 serve`) or let its service manager do it"
    )]
    OwnPath {
        #[source]
        source: std::io::Error,
    },

    #[error(
        "couldn't start {} again: {source}; the change is saved, so start humatad2 by hand (`humatad2 serve`) or let its service manager do it; check that the file still exists and can be run",
        path.display()
    )]
    Exec {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// The arguments the new run gets: the ones this run had, less
/// `--ready-fd` and its value. That descriptor was closed once the socket
/// listened, and the bridge that asked for it has heard already.
pub(crate) fn again_args(args: impl IntoIterator<Item = OsString>) -> Vec<OsString> {
    let mut kept = Vec::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg == OsStr::new("--ready-fd") {
            args.next();
        } else if !arg.as_encoded_bytes().starts_with(b"--ready-fd=") {
            kept.push(arg);
        }
    }
    kept
}

/// Replaces this process with a new run of the same program, the same
/// arguments (less `--ready-fd`) and the same environment. Returns only
/// when that failed. `point` is the fault point reached just before.
pub(crate) fn exec_again(point: &str) -> Result<Infallible, ExecError> {
    crate::fault::point(point);
    let path = std::env::current_exe().map_err(|source| ExecError::OwnPath { source })?;
    let args = again_args(std::env::args_os().skip(1));
    let source = std::process::Command::new(&path).args(args).exec();
    Err(ExecError::Exec { path, source })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn the_ready_descriptor_and_its_value_are_dropped() {
        assert_eq!(
            again_args(args(&["serve", "--ready-fd", "7", "--state", "/s"])),
            args(&["serve", "--state", "/s"])
        );
    }

    #[test]
    fn the_ready_descriptor_given_with_an_equals_sign_is_dropped() {
        assert_eq!(
            again_args(args(&["serve", "--ready-fd=7", "--role", "host"])),
            args(&["serve", "--role", "host"])
        );
    }

    #[test]
    fn a_ready_descriptor_with_no_value_is_dropped() {
        assert_eq!(again_args(args(&["serve", "--ready-fd"])), args(&["serve"]));
    }

    #[test]
    fn arguments_without_a_ready_descriptor_are_kept_as_they_are() {
        let all = args(&["serve", "--socket", "/a b/s.sock", "--shell-arg", "--ready"]);
        assert_eq!(again_args(all.clone()), all);
    }
}
