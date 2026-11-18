//! The daemon starting itself again, in the same process, after a stop or a
//! start (D261, D265): its next run reads the stop record and sweeps, or
//! brings everything back.
//!
//! The state lock and the listening socket are close-on-exec, so the new
//! image takes both again; a socket file left behind is a dead one, which a
//! boot removes as it does after a crash.

use std::convert::Infallible;
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::os::unix::ffi::OsStrExt;
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

/// Whether this run has told its starter it listens (D398, D439).
pub(crate) enum Ready {
    /// It has, or nobody asked: the new run is given no `--ready-fd`.
    Told,
    /// It hasn't: the pipe is handed on, and the new run tells.
    Owed(File),
}

/// The arguments the new run gets: the ones this run had, less
/// `--ready-fd` and its value unless `keep_ready`. A run that listened has
/// closed that descriptor, and the bridge that asked for it has heard
/// already; a run that hands its pipe on keeps the flag, whose descriptor
/// the exec keeps under the same number (D439).
pub(crate) fn again_args(args: impl IntoIterator<Item = OsString>, keep_ready: bool) -> Vec<OsString> {
    if keep_ready {
        return args.into_iter().collect();
    }
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

/// Where this program is installed: its own path, less the " (deleted)"
/// mark Linux puts on it once an install has replaced the file (D281). The
/// new run starts from the file that is there now.
pub(crate) fn installed_path() -> Result<PathBuf, ExecError> {
    std::env::current_exe()
        .map(without_deleted_mark)
        .map_err(|source| ExecError::OwnPath { source })
}

/// `path` without a trailing " (deleted)", which `/proc/self/exe` reads
/// once the running file was renamed over or removed.
fn without_deleted_mark(path: PathBuf) -> PathBuf {
    const MARK: &[u8] = b" (deleted)";
    match path.as_os_str().as_bytes().strip_suffix(MARK) {
        Some(kept) if !kept.is_empty() => PathBuf::from(OsStr::from_bytes(kept)),
        _ => path,
    }
}

/// Replaces this process with a new run of the program at `path`, with the
/// same arguments and the same environment. `--ready-fd` goes with it only
/// when `ready` is owed and its pipe could be handed on; otherwise the new
/// run is given none, and a starter still waiting sees the pipe close.
/// Returns only when that failed. `point` is the fault point reached just
/// before.
pub(crate) fn exec_at(path: PathBuf, point: &str, ready: Ready) -> Result<Infallible, ExecError> {
    crate::fault::point(point);
    // Held to the exec: dropped, it would close the descriptor handed on.
    let pipe = match ready {
        Ready::Told => None,
        Ready::Owed(pipe) => match crate::ready::hand_on(&pipe) {
            Ok(()) => Some(pipe),
            Err(error) => {
                crate::log::say!("humatad2: warning: {}", crate::status::chain(&error));
                None
            }
        },
    };
    let args = again_args(std::env::args_os().skip(1), pipe.is_some());
    let source = std::process::Command::new(&path).args(args).exec();
    drop(pipe);
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
            again_args(args(&["serve", "--ready-fd", "7", "--state", "/s"]), false),
            args(&["serve", "--state", "/s"])
        );
    }

    #[test]
    fn the_ready_descriptor_given_with_an_equals_sign_is_dropped() {
        assert_eq!(
            again_args(args(&["serve", "--ready-fd=7", "--role", "host"]), false),
            args(&["serve", "--role", "host"])
        );
    }

    #[test]
    fn a_ready_descriptor_with_no_value_is_dropped() {
        assert_eq!(again_args(args(&["serve", "--ready-fd"]), false), args(&["serve"]));
    }

    #[test]
    fn arguments_without_a_ready_descriptor_are_kept_as_they_are() {
        let all = args(&["serve", "--socket", "/a b/s.sock", "--shell-arg", "--ready"]);
        assert_eq!(again_args(all.clone(), false), all);
    }

    #[test]
    fn a_handed_on_ready_descriptor_is_kept_with_its_value() {
        let all = args(&["serve", "--ready-fd", "7", "--state", "/s", "--ready-fd=9"]);
        assert_eq!(again_args(all.clone(), true), all);
    }

    #[test]
    fn a_replaced_programs_path_is_the_installed_one() {
        assert_eq!(
            without_deleted_mark(PathBuf::from("/usr/local/bin/humatad2 (deleted)")),
            PathBuf::from("/usr/local/bin/humatad2")
        );
    }

    #[test]
    fn a_path_without_the_mark_is_kept_as_it_is() {
        let path = PathBuf::from("/opt/humata/humatad2");
        assert_eq!(without_deleted_mark(path.clone()), path);
    }

    #[test]
    fn a_path_that_is_only_the_mark_is_kept_as_it_is() {
        let path = PathBuf::from(" (deleted)");
        assert_eq!(without_deleted_mark(path.clone()), path);
    }
}
