//! Shutting a holder down, and ending it by its pid when it won't take that
//! (D292). A holder that is stopped, wedged or refuses doesn't answer its
//! shutdown in time, so whatever waits on it would hang. Such a holder is
//! ended by its pid instead: no action waits on a holder for ever, and none
//! leaves running a holder whose pid is known.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rustix::process::Signal;
use tokio::process::Command;

use crate::process::{Snapshot, holder_names, send, settle_until};
use crate::{HolderError, shutdown};

/// How long a holder gets to end once sent SIGTERM, before it is SIGKILLed:
/// the 2 s a holder gives its program itself.
const GRACE: Duration = Duration::from_secs(2);

/// How long a SIGKILLed holder gets to leave the process list. It can't run
/// any more, but on a busy machine its exit can take longer than the sweep's
/// [`KILL_WAIT`](crate::process::KILL_WAIT) to finish, and a close must not fail on that.
const KILLED_WAIT: Duration = Duration::from_secs(5);

/// What [`shutdown_or_end`] did to a holder.
#[derive(Debug)]
pub enum Stopped {
    /// The holder took its shutdown.
    Shut,
    /// The holder didn't take its shutdown, for `reason`, so its process
    /// was ended by its pid, or was gone already. A socket that is gone is
    /// how a holder that has stopped looks.
    Ended { reason: HolderError },
}

/// A holder whose process couldn't be ended by its pid.
#[derive(Debug, thiserror::Error)]
pub enum EndError {
    #[error(
        "couldn't list the running processes with `ps` to end the terminal's holder at {} by its pid {pid}: {source}; check that `ps` runs for this user, then end process {pid} by hand only if `ps -p {pid}` still lists it as a humata2 holder",
        socket.display()
    )]
    ListProcesses {
        socket: PathBuf,
        pid: u32,
        #[source]
        source: io::Error,
    },

    #[error(
        "the terminal's holder at {} didn't take its shutdown and is still running as process {pid} after SIGKILL; check with `ps -p {pid}` that it is a humata2 holder, then end it by hand",
        socket.display()
    )]
    Survived { socket: PathBuf, pid: u32 },

    #[error(
        "couldn't list the running processes with `ps` to find the terminal's holder at {}, whose pid file is gone: {source}; check that `ps` runs for this user. `ps -A -ww -o pid=,args= | grep -- '--socket {}'` shows the holder if one still runs; end it by hand before starting another there",
        socket.display(),
        socket.display()
    )]
    FindHolder {
        socket: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// Shuts down the holder at `socket`, whose process is `pid`. When it
/// doesn't take the shutdown, or its socket is gone, its process is ended by
/// its pid instead: SIGTERM, then SIGKILL if it is still running once the
/// grace is out. Only a live process named as a holder is signalled, so a
/// pid that has ended, or been reused by another program, is left alone.
pub async fn shutdown_or_end(socket: &Path, pid: u32) -> Result<Stopped, EndError> {
    let Err(reason) = shutdown(socket).await else {
        return Ok(Stopped::Shut);
    };
    end_by_pid(socket, pid, GRACE).await?;
    Ok(Stopped::Ended { reason })
}

/// Ends the holder at `socket`, whose process is `pid`, by its pid without
/// asking it to shut down: SIGTERM, which keeps what it printed saved for
/// the holder started after it, then SIGKILL if it is still running once
/// `grace` is out. Waits until it is gone. Only a live holder is signalled,
/// one named as a holder build or one serving `socket` under a name of its
/// own, so a pid that has ended, or been reused by another program, is left
/// alone.
pub async fn end_by_pid(socket: &Path, pid: u32, grace: Duration) -> Result<(), EndError> {
    let names: Vec<OsString> = holder_names(Path::new(""));
    let listing = |source| EndError::ListProcesses {
        socket: socket.to_owned(),
        pid,
        source,
    };
    let snapshot = Snapshot::take().await.map_err(listing)?;
    if !is_holder(&snapshot, socket, pid, &names).await.map_err(listing)? {
        return Ok(());
    }
    send(pid, Signal::TERM);
    let snapshot = settle_until(grace, |snapshot| snapshot.is_alive(pid))
        .await
        .map_err(listing)?;
    // Asked again: a pid freed by the holder's exit may have been reused.
    if !is_holder(&snapshot, socket, pid, &names).await.map_err(listing)? {
        return Ok(());
    }
    send(pid, Signal::KILL);
    let snapshot = settle_until(KILLED_WAIT, |snapshot| snapshot.is_alive(pid))
        .await
        .map_err(listing)?;
    if is_holder(&snapshot, socket, pid, &names).await.map_err(listing)? {
        return Err(EndError::Survived {
            socket: socket.to_owned(),
            pid,
        });
    }
    Ok(())
}

/// The pid of the holder serving `socket`: the one its pid file names, or,
/// with that file gone, the process started to serve `socket`. `None` when
/// neither is there.
pub async fn find_holder(socket: &Path) -> Result<Option<u32>, EndError> {
    if let Some(pid) = crate::holder_pid(socket) {
        return Ok(Some(pid));
    }
    let output = Command::new("ps")
        .args(["-A", "-ww", "-o", "pid=,args="])
        .output()
        .await
        .map_err(|source| EndError::FindHolder {
            socket: socket.to_owned(),
            source,
        })?;
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(text.lines().find_map(|line| {
        let (pid, args) = line.trim_start().split_once(char::is_whitespace)?;
        if !serves(args, socket) {
            return None;
        }
        pid.parse().ok()
    }))
}

/// Whether `pid` is a live holder: named as a holder build, or serving
/// `socket` under a name of its own, as an installed holder can be.
async fn is_holder(
    snapshot: &Snapshot,
    socket: &Path,
    pid: u32,
    names: &[OsString],
) -> io::Result<bool> {
    if snapshot.is_holder(pid, names) {
        return Ok(true);
    }
    if !snapshot.is_alive(pid) {
        return Ok(false);
    }
    // `ps` prints nothing, and fails, for a process that is gone.
    let output = Command::new("ps")
        .args(["-ww", "-o", "args=", "-p", &pid.to_string()])
        .output()
        .await?;
    Ok(serves(&String::from_utf8_lossy(&output.stdout), socket))
}

/// Whether the command line `args` is a holder's serving `socket`: started
/// as [`crate::spawn`] starts one, its socket followed by its other flags.
fn serves(args: &str, socket: &Path) -> bool {
    args.contains(&format!("--socket {} ", socket.display()))
}
