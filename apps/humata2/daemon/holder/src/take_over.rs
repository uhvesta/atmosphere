//! Moving a running holder onto another build in place (LIVEHOLDER, D801):
//! the process keeps its pid, its listening socket and its terminal, runs
//! the new image, and the new image rebuilds what the old one knew from a
//! handoff file written first. A part of `server`, so it sees its state.

use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::net::UnixStream as StdUnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use rustix::io::FdFlags;
use rustix::process::Pid;
use tokio::net::UnixListener;
use tokio::sync::{broadcast, watch};

use super::{
    KEYS_KEPT, Keys, LIVE_CHUNKS, Shared, State, pid_path, serve_clients, start_threads,
};
use crate::HolderError;
use crate::handoff::{Handoff, VERSIONS};
use crate::protocol::ServerFrame;
use crate::saved::SaveLog;
use crate::scrollback::Scrollback;

/// How long the new build gets to say which handoff versions it reads.
/// Nothing is held while it answers, so this can be long: the first run of
/// a freshly installed binary waits for the system to scan it (macOS
/// checks every new executable before it first runs).
const VERSIONS_LIMIT: Duration = Duration::from_secs(20);
/// How long the new build gets to read the handoff, while the terminal's
/// output is held (D801 A1). It has already run once above, so it starts
/// at once.
const CHECK_LIMIT: Duration = Duration::from_secs(2);

/// The handoff versions this build reads, to say to a holder that asks.
pub fn handoff_versions() -> &'static [u32] {
    VERSIONS
}

/// Reads the handoff at `path` as a holder taking over would, and touches
/// nothing.
pub fn check_take_over(path: &Path) -> Result<(), HolderError> {
    read_handoff(path).map(|_| ())
}

fn read_handoff(path: &Path) -> Result<Handoff, HolderError> {
    let bytes = fs::read(path).map_err(|source| HolderError::Saved {
        path: path.to_owned(),
        source,
    })?;
    Handoff::decode(&bytes).map_err(|source| HolderError::Handoff {
        path: path.to_owned(),
        source,
    })
}

fn handoff_path(socket: &Path) -> PathBuf {
    let mut name = socket.as_os_str().to_owned();
    name.push(".handoff");
    PathBuf::from(name)
}

fn plain<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Plain data, still consistent after a panic elsewhere.
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// What a check run of the new build said.
struct Ran {
    success: bool,
    stdout: String,
    stderr: String,
}

/// Runs `program` with `args`, giving it `limit` to finish, and ends it
/// if it doesn't. It's this holder's own child, so ending it is fine.
fn run_bounded(program: &Path, args: &[&OsStr], limit: Duration) -> Result<Ran, String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            format!(
                "couldn't run {}: {error}; give the path of an installed humata2_holder",
                program.display()
            )
        })?;
    let deadline = Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "{} didn't answer within {} seconds and was stopped; check that it is a humata2_holder and the machine isn't overloaded",
                    program.display(),
                    limit.as_secs()
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("couldn't wait for {}: {error}", program.display()));
            }
        }
    };
    let (mut stdout, mut stderr) = (String::new(), String::new());
    if let Some(mut out) = child.stdout.take() {
        let _ = out.read_to_string(&mut stdout);
    }
    if let Some(mut err) = child.stderr.take() {
        let _ = err.read_to_string(&mut stderr);
    }
    Ok(Ran {
        success: status.success(),
        stdout,
        stderr,
    })
}

/// The last lines of `text`, for a message.
fn tail(text: &str) -> String {
    let lines: Vec<&str> = text.trim().lines().collect();
    let from = lines.len().saturating_sub(5);
    lines[from..].join("; ")
}

/// A descriptor to be kept across the exec, or put back.
fn keep_across_exec(fd: RawFd, keep: bool) -> Result<(), std::io::Error> {
    // SAFETY: `fd` is the listener's or the terminal's, open for as long
    // as the holder serves, which this call is part of.
    let fd = unsafe { BorrowedFd::borrow_raw(fd) };
    let flags = if keep { FdFlags::empty() } else { FdFlags::CLOEXEC };
    rustix::io::fcntl_setfd(fd, flags).map_err(Into::into)
}

/// Moves this holder onto the build at `new`: returns only if it didn't,
/// with what to tell the client; on success the process runs `new` and
/// this never returns. `told` is the requesting connection, sent `Moving`
/// just before the exec.
pub(super) fn try_move(
    shared: &Shared,
    new: &Path,
    told: &StdUnixStream,
    runtime: &tokio::runtime::Handle,
) -> String {
    if shared.moving.swap(true, Ordering::SeqCst) {
        return "a move of this holder is already under way; wait for it to finish".to_owned();
    }
    let message = move_onto(shared, new, told, runtime);
    shared.moving.store(false, Ordering::SeqCst);
    message
}

fn move_onto(
    shared: &Shared,
    new: &Path,
    told: &StdUnixStream,
    runtime: &tokio::runtime::Handle,
) -> String {
    match fs::metadata(new) {
        Ok(meta) if meta.is_file() && meta.permissions().mode() & 0o111 != 0 => {}
        Ok(_) => {
            return format!(
                "{} isn't a program that can be run; give the path of an installed humata2_holder",
                new.display()
            );
        }
        Err(error) => {
            return format!(
                "can't read {}: {error}; give the path of an installed humata2_holder",
                new.display()
            );
        }
    }
    // The newest handoff version both builds read.
    let versions = match run_bounded(new, &[OsStr::new("--handoff-versions")], VERSIONS_LIMIT) {
        Ok(ran) if ran.success => ran.stdout,
        Ok(ran) => {
            return format!(
                "{} can't take a holder over: it doesn't answer --handoff-versions ({}); it is not a build that supports moving, so the holder stays as it is",
                new.display(),
                tail(&ran.stderr)
            );
        }
        Err(message) => return message,
    };
    let common = versions
        .split_whitespace()
        .filter_map(|word| word.parse::<u32>().ok())
        .filter(|version| VERSIONS.contains(version))
        .max();
    let Some(version) = common else {
        return format!(
            "{} reads handoff versions {:?} and this holder writes {:?}, with none in common; the holder stays as it is. Move it onto a build from the same generation",
            new.display(),
            versions.trim(),
            VERSIONS
        );
    };

    // Quiet: nothing reads the terminal, types into it or changes what is
    // kept from here until the exec, or until the move is given up.
    let _gate = plain(&shared.gate);
    let reaped = plain(&shared.reaped);
    let keys = plain(&shared.keys);
    let _writer = plain(&shared.writer);
    shared.sync_saved();
    let handoff_file = handoff_path(&shared.socket);
    {
        let state = shared.lock();
        let handoff = Handoff {
            listener: shared.listener,
            master: shared.master.as_raw_fd(),
            child: shared
                .group
                .and_then(|pid| u32::try_from(pid.as_raw_pid()).ok())
                .unwrap_or(0),
            exited: state.exited,
            reaped: *reaped,
            cols: state.cols,
            rows: state.rows,
            limit: state.kept.limit() as u64,
            end: state.kept.end(),
            runs: state.kept.since(0),
            save: state.saved.as_ref().map(SaveLog::parts),
            instance: shared.instance,
            keys: keys.0.iter().cloned().collect(),
            started_ms: u64::try_from(shared.started.elapsed().as_millis()).unwrap_or(u64::MAX),
        };
        if let Err(error) = write_handoff(&handoff_file, &handoff.encode(version)) {
            let _ = fs::remove_file(&handoff_file);
            return format!(
                "couldn't write {}: {error}; check that the folder belongs to you and the disk has room",
                handoff_file.display()
            );
        }
    }
    // The new build reads it as it will for real, and touches nothing.
    let check = run_bounded(
        new,
        &[OsStr::new("--check-take-over"), handoff_file.as_os_str()],
        CHECK_LIMIT,
    );
    match check {
        Ok(ran) if ran.success => {}
        Ok(ran) => {
            let _ = fs::remove_file(&handoff_file);
            return format!(
                "{} can't read this holder's handoff, so the holder stays as it is: {}",
                new.display(),
                tail(&ran.stderr)
            );
        }
        Err(message) => {
            let _ = fs::remove_file(&handoff_file);
            return message;
        }
    }

    let kept = keep_across_exec(shared.listener, true)
        .and_then(|()| keep_across_exec(shared.master.as_raw_fd(), true));
    if let Err(error) = kept {
        let _ = keep_across_exec(shared.listener, false);
        let _ = keep_across_exec(shared.master.as_raw_fd(), false);
        let _ = fs::remove_file(&handoff_file);
        return format!("couldn't keep the terminal open for the new build: {error}; the holder stays as it is");
    }
    // The client reads to the end of its connection after this: the exec
    // closes every connection.
    let mut told = told;
    let _ = told.write_all(&ServerFrame::Moving.encode());

    let error = Command::new(new)
        .arg("--socket")
        .arg(&shared.socket)
        .arg("--take-over")
        .arg(&handoff_file)
        .exec();

    // Only a failed exec comes back. It left SIGPIPE at its default, where
    // a write to a client that left would end this holder: a handler
    // makes it harmless again.
    {
        let _inside = runtime.enter();
        if let Ok(signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::from_raw(13))
        {
            // Dropping it leaves the handler installed.
            drop(signal);
        }
    }
    let _ = keep_across_exec(shared.listener, false);
    let _ = keep_across_exec(shared.master.as_raw_fd(), false);
    let _ = fs::remove_file(&handoff_file);
    format!(
        "couldn't start {}: {error}; the holder stays as it is. Check that the file is still there and runs on this machine",
        new.display()
    )
}

fn write_handoff(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// Runs as the holder `handoff` describes, in the image a move started,
/// on the socket the old image listened on: same pid, same terminal, same
/// program. Returns once the holder is shut down.
pub async fn take_over(socket: PathBuf, handoff: &Path) -> Result<(), HolderError> {
    let state = read_handoff(handoff)?;
    // Test builds only: dies before serving, to test what a move that
    // loses its new image leaves behind. The handoff stays.
    #[cfg(feature = "faults")]
    if std::env::var_os("HUMATA2_CRASH_AFTER_TAKE_OVER").is_some_and(|value| value == "1") {
        std::process::abort();
    }

    let adopt = |what| move |source: std::io::Error| HolderError::Adopt { what, source };
    // SAFETY: the old image left these two descriptors open for this one
    // and nothing else in this process uses their numbers.
    let (listener, master) = unsafe {
        (
            OwnedFd::from_raw_fd(state.listener),
            OwnedFd::from_raw_fd(state.master),
        )
    };
    rustix::io::fcntl_setfd(&listener, FdFlags::CLOEXEC)
        .map_err(|error| adopt("its socket")(error.into()))?;
    rustix::io::fcntl_setfd(&master, FdFlags::CLOEXEC)
        .map_err(|error| adopt("its terminal")(error.into()))?;
    let listener = std::os::unix::net::UnixListener::from(listener);
    listener
        .set_nonblocking(true)
        .map_err(adopt("its socket"))?;
    let listener = UnixListener::from_std(listener).map_err(adopt("its socket"))?;

    let reader = File::from(master.try_clone().map_err(adopt("its terminal"))?);
    let writer = File::from(master.try_clone().map_err(adopt("its terminal"))?);
    let saved = state.save.map(SaveLog::reopen).transpose()?;
    let kept = Scrollback::restore(
        usize::try_from(state.limit).unwrap_or(usize::MAX),
        state.end,
        state.runs,
    );
    let now = tokio::time::Instant::now();
    let started = now
        .checked_sub(Duration::from_millis(state.started_ms))
        .unwrap_or(now);
    let group = i32::try_from(state.child).ok().and_then(Pid::from_raw);
    let known = state.exited.or(state.reaped);
    let mut remembered = Keys(Default::default());
    for key in state.keys.into_iter().rev().take(KEYS_KEPT).rev() {
        remembered.remember(key);
    }
    let listener_fd = listener.as_raw_fd();
    let shared = Arc::new(Shared {
        state: Mutex::new(State {
            kept,
            saved,
            exited: state.exited,
            cols: state.cols.max(1),
            rows: state.rows.max(1),
        }),
        live: broadcast::channel(LIVE_CHUNKS).0,
        master,
        writer: Mutex::new(writer),
        keys: Mutex::new(remembered),
        instance: state.instance,
        group,
        stop: watch::channel(false).0,
        wrote: tokio::sync::Notify::new(),
        exit: watch::channel(state.exited).0,
        started,
        #[cfg(feature = "faults")]
        synced_at: super::synced_path(&socket),
        socket: socket.clone(),
        listener: listener_fd,
        gate: Mutex::new(()),
        reaped: Mutex::new(state.reaped),
        moving: std::sync::atomic::AtomicBool::new(false),
    });
    let (exit_read, exit_write) = std::io::pipe().map_err(adopt("its terminal"))?;
    start_threads(&shared, reader, exit_read, exit_write, group, known);

    #[cfg(feature = "faults")]
    {
        super::power_cut_on_sigusr2();
        let _ = super::SERVING.set(socket.clone());
    }
    // Serving from here: what the old image left is no longer needed.
    let _ = fs::remove_file(handoff);
    serve_clients(shared, listener, &socket, &pid_path(&socket)).await;
    Ok(())
}
