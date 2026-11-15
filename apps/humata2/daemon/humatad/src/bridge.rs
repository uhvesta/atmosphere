//! `humatad2 bridge`: the daemon's socket on stdin and stdout, for a link
//! that only carries a process's stdio: SSH or a provider's connect command
//! (TR-2, D15). When no daemon answers it starts one, detached, so the
//! daemon outlives the link, and waits on a pipe the daemon writes a byte
//! to once it listens, rather than trying to connect again and again.
//!
//! It only carries bytes. It adds no credential and checks none: the daemon
//! on the other end checks every call (D41).

use std::io::{ErrorKind, PipeReader, PipeWriter};
use std::os::fd::{AsRawFd, BorrowedFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
use tokio::net::UnixStream;
use tokio::net::unix::pipe;

use crate::error::DaemonError;
use crate::{serve, sync};

/// How long a daemon that is restarting (a service manager's restart, say)
/// gets to come back before the bridge starts one of its own. The bridge
/// tries once more after it, not in a loop.
const GRACE: Duration = Duration::from_millis(500);
/// How long a daemon the bridge started gets to listen.
const START_WAIT: Duration = Duration::from_secs(10);

#[derive(clap::Args)]
pub(crate) struct BridgeArgs {
    /// The daemon's socket. Default: ~/.humata2/humatad.sock.
    #[arg(long)]
    socket: Option<PathBuf>,
    /// Once connected, print `HUMATA_SYNC_<nonce>` on a line of its own
    /// before the stream, so a terminal Humata took over can tell where the
    /// stream starts (MP-15). Letters and digits only.
    #[arg(long, value_parser = sync::parse_nonce)]
    sync: Option<String>,
}

pub(crate) async fn run(args: BridgeArgs) -> Result<(), DaemonError> {
    let socket = match args.socket {
        Some(socket) => socket,
        None => serve::default_socket()?,
    };
    let daemon = connect_or_start(&socket).await?;
    let mut stdout = tokio::io::stdout();
    if let Some(nonce) = &args.sync
        && let Err(error) = sync::write_marker(&mut stdout, nonce).await
    {
        return closed_or_broken(error);
    }
    carry(daemon, tokio::io::stdin(), stdout).await
}

/// Copies both ways until either side closes. The far end going away is
/// how a bridge normally ends, so it isn't an error.
async fn carry(
    daemon: UnixStream,
    mut input: impl AsyncRead + Unpin,
    mut output: impl AsyncWrite + Unpin,
) -> Result<(), DaemonError> {
    let (mut from_daemon, mut to_daemon) = daemon.into_split();
    let copied = tokio::select! {
        copied = tokio::io::copy(&mut input, &mut to_daemon) => copied,
        copied = tokio::io::copy(&mut from_daemon, &mut output) => copied,
    };
    match copied {
        Ok(_) => Ok(()),
        Err(error) => closed_or_broken(error),
    }
}

fn closed_or_broken(error: std::io::Error) -> Result<(), DaemonError> {
    match error.kind() {
        ErrorKind::BrokenPipe | ErrorKind::ConnectionReset | ErrorKind::UnexpectedEof => Ok(()),
        _ => Err(DaemonError::BridgeCopy { source: error }),
    }
}

/// Whether a failed connect means no daemon is listening, rather than one
/// the bridge isn't allowed to reach.
fn nobody_listening(error: &std::io::Error) -> bool {
    matches!(error.kind(), ErrorKind::NotFound | ErrorKind::ConnectionRefused)
}

/// Connects to the daemon on `socket`, or starts one there and waits for it.
async fn connect_or_start(socket: &Path) -> Result<UnixStream, DaemonError> {
    if let Some(stream) = try_connect(socket).await? {
        return Ok(stream);
    }
    tokio::time::sleep(GRACE).await;
    if let Some(stream) = try_connect(socket).await? {
        return Ok(stream);
    }

    let (ready, mut child) = start_daemon(socket)?;
    let silent = || DaemonError::BridgeDaemonSilent {
        socket: socket.to_owned(),
        waited: START_WAIT,
    };
    let started = async {
        if heard_ready(ready).await {
            return try_connect(socket).await;
        }
        // It closed the pipe without a word, so it ended. Another bridge
        // may have started one first, which this one then finds running.
        if let Some(stream) = try_connect(socket).await? {
            return Ok(Some(stream));
        }
        let status = tokio::task::spawn_blocking(move || child.wait())
            .await
            .ok()
            .and_then(Result::ok);
        Err(match status {
            Some(status) => DaemonError::BridgeDaemonExited {
                socket: socket.to_owned(),
                status,
            },
            None => silent(),
        })
    };
    match tokio::time::timeout(START_WAIT, started).await {
        Ok(Ok(Some(stream))) => Ok(stream),
        Ok(Ok(None)) | Err(_) => Err(silent()),
        Ok(Err(error)) => Err(error),
    }
}

/// The daemon on `socket`, or `None` when nobody listens there.
async fn try_connect(socket: &Path) -> Result<Option<UnixStream>, DaemonError> {
    match UnixStream::connect(socket).await {
        Ok(stream) => Ok(Some(stream)),
        Err(error) if nobody_listening(&error) => Ok(None),
        Err(source) => Err(DaemonError::BridgeConnect {
            socket: socket.to_owned(),
            source,
        }),
    }
}

/// Whether the daemon wrote its byte, which it does once it listens. A
/// pipe that closes first, or can't be read, means it didn't.
async fn heard_ready(ready: PipeReader) -> bool {
    let Ok(mut ready) = pipe::Receiver::from_owned_fd(OwnedFd::from(ready)) else {
        return false;
    };
    let mut byte = [0; 1];
    matches!(ready.read(&mut byte).await, Ok(1))
}

/// Starts `humatad2 serve --role machine` on `socket` in a session of its
/// own, with no stdio, so it outlives the bridge and the link's hangup and
/// writes nothing into the stream. Returns the end of the pipe it says it
/// listens on.
fn start_daemon(socket: &Path) -> Result<(PipeReader, Child), DaemonError> {
    let program = std::env::current_exe().map_err(|source| DaemonError::BridgeOwnBinary {
        socket: socket.to_owned(),
        source,
    })?;
    let start_failed = |source| DaemonError::BridgeStart {
        program: program.clone(),
        socket: socket.to_owned(),
        source,
    };
    let (ready, ready_writer): (PipeReader, PipeWriter) = std::io::pipe().map_err(start_failed)?;
    let ready_fd = ready_writer.as_raw_fd();
    let mut command = Command::new(&program);
    command
        .args(["serve", "--role", "machine", "--socket"])
        .arg(socket)
        .arg("--ready-fd")
        .arg(ready_fd.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: setsid and fcntl are async-signal-safe and touch no memory of
    // this process, so they may run between fork and exec. The pipe's end
    // is opened close-on-exec; only the daemon's copy is kept open.
    unsafe {
        command.pre_exec(move || {
            rustix::process::setsid().map_err(std::io::Error::from)?;
            let fd = BorrowedFd::borrow_raw(ready_fd);
            rustix::io::fcntl_setfd(fd, rustix::io::FdFlags::empty()).map_err(std::io::Error::from)
        });
    }
    let child = command.spawn().map_err(start_failed)?;
    // Only the daemon writes: with this end closed here, the pipe closes
    // when the daemon closes or exits.
    drop(ready_writer);
    Ok((ready, child))
}
