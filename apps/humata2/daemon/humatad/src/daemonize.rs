//! Unix double-fork daemonization for `humatad2 serve --daemonize`.
//!
//! Must be called while the process is still single-threaded — BEFORE creating
//! the Tokio runtime or spawning any threads — because `fork(2)` in a
//! multi-threaded process only duplicates the calling thread and leaves locks
//! held by other threads permanently wedged.

use std::fs::{DirBuilder, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::Path;

use crate::error::DaemonError;

/// Detaches the current process into a background daemon:
///
/// 1. `fork()` — parent waits on a sync pipe for the grandchild to finish
///    detaching, reaps the intermediate child, and exits `0` cleanly.
/// 2. Intermediate child calls `setsid()` to start a new session with no
///    controlling terminal.
/// 3. Second `fork()` — intermediate child exits immediately so the grandchild
///    is not a session leader and can never re-acquire a controlling terminal.
/// 4. Grandchild redirects `stdin` to `/dev/null` and `stdout`/`stderr` to
///    `log_path` (creating its parent directory with mode `0700` when needed,
///    falling back to `/dev/null`), then signals the original parent over the
///    pipe so the parent (and any SSH session that invoked it) exits at once.
pub fn detach(log_path: Option<&Path>) -> Result<(), DaemonError> {
    let failed = |source| DaemonError::Daemonize { source };
    let (mut read_end, mut write_end) = std::io::pipe().map_err(failed)?;

    // SAFETY: called from `main` before the Tokio runtime or any worker thread
    // is created, so the process is strictly single-threaded.
    let first = unsafe { libc::fork() };
    if first < 0 {
        return Err(failed(io::Error::last_os_error()));
    }
    if first > 0 {
        // Original parent: close the write end, wait for grandchild's status,
        // reap the intermediate child, and exit cleanly.
        drop(write_end);
        let mut status = [0u8; 1];
        let read_ok = read_end.read_exact(&mut status).is_ok() && status[0] == 0;
        drop(read_end);
        let mut wstatus: libc::c_int = 0;
        // SAFETY: `first` is our direct child pid.
        unsafe {
            while libc::waitpid(first, &mut wstatus, 0) < 0 {
                if io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
                    break;
                }
            }
        }
        if read_ok {
            std::process::exit(0);
        }
        return Err(failed(io::Error::other(
            "the background daemon failed before finishing detach",
        )));
    }

    // Intermediate child.
    drop(read_end);
    if rustix::process::setsid().is_err() {
        let _ = write_end.write_all(&[1]);
        // SAFETY: child exiting immediately after failed setsid.
        unsafe { libc::_exit(1) };
    }

    // SAFETY: still single-threaded inside the intermediate child.
    let second = unsafe { libc::fork() };
    if second < 0 {
        let _ = write_end.write_all(&[1]);
        // SAFETY: child exiting immediately after failed second fork.
        unsafe { libc::_exit(1) };
    }
    if second > 0 {
        // Intermediate child exits immediately so the grandchild is reparented
        // and can never re-acquire a controlling terminal.
        // SAFETY: intermediate child exit without flushing parent buffers.
        unsafe { libc::_exit(0) };
    }

    // Grandchild (the actual daemon).
    if let Err(_) = redirect_stdio(log_path) {
        let _ = write_end.write_all(&[1]);
        // SAFETY: grandchild exiting after failing to redirect stdio.
        unsafe { libc::_exit(1) };
    }

    // Signal the original parent that we are detached and have released its
    // stdin/stdout/stderr descriptors, then close the sync pipe.
    let _ = write_end.write_all(&[0]);
    drop(write_end);
    Ok(())
}

fn redirect_stdio(log_path: Option<&Path>) -> io::Result<()> {
    let dev_null = OpenOptions::new().read(true).write(true).open("/dev/null")?;
    let out = log_path
        .and_then(|path| open_log_file(path).ok())
        .unwrap_or_else(|| {
            OpenOptions::new()
                .write(true)
                .open("/dev/null")
                .expect("/dev/null was already opened")
        });
    rustix::stdio::dup2_stdin(&dev_null).map_err(io::Error::from)?;
    rustix::stdio::dup2_stdout(&out).map_err(io::Error::from)?;
    rustix::stdio::dup2_stderr(&out).map_err(io::Error::from)?;
    Ok(())
}

fn open_log_file(path: &Path) -> io::Result<std::fs::File> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        DirBuilder::new().recursive(true).mode(0o700).create(parent)?;
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use clap::Parser;

    use crate::serve::ServeArgs;

    #[derive(Parser)]
    struct TestCli {
        #[command(flatten)]
        serve: ServeArgs,
    }

    #[test]
    fn daemonize_flag_parses_and_resolves_default_log_path() {
        let parsed = TestCli::try_parse_from([
            "serve",
            "--role",
            "machine",
            "--state",
            "/tmp/humata-test-state",
            "--daemonize",
        ])
        .expect("parse --daemonize");
        assert!(parsed.serve.daemonize);
        assert_eq!(
            parsed.serve.daemonize_log_path(),
            Some(PathBuf::from("/tmp/humata-test-state/logs/humatad2.daemon.log"))
        );
    }

    #[test]
    fn daemonize_accepts_explicit_log_file() {
        let parsed = TestCli::try_parse_from([
            "serve",
            "--daemonize",
            "--log-file",
            "/tmp/custom-humatad.log",
        ])
        .expect("parse --daemonize --log-file");
        assert!(parsed.serve.daemonize);
        assert_eq!(
            parsed.serve.daemonize_log_path(),
            Some(PathBuf::from("/tmp/custom-humatad.log"))
        );
    }

    #[test]
    fn daemonize_conflicts_with_stdio() {
        let err = TestCli::try_parse_from(["serve", "--stdio", "--daemonize"]);
        assert!(err.is_err(), "--daemonize must conflict with --stdio");
    }

    #[test]
    fn log_file_requires_daemonize() {
        let err = TestCli::try_parse_from(["serve", "--log-file", "/tmp/x.log"]);
        assert!(err.is_err(), "--log-file must require --daemonize");
    }
}
