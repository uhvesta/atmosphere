//! `humatad2 bridge`: the daemon's socket on stdin and stdout, for a link
//! that only carries a process's stdio: SSH or a provider's connect command
//! (TR-2, D15). When no daemon answers it starts one, detached, so the
//! daemon outlives the link.
//!
//! It only carries bytes. It adds no credential and checks none: the daemon
//! on the other end checks every call (D41).

use std::io::ErrorKind;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::UnixStream;

use crate::error::DaemonError;
use crate::{serve, sync};

/// How long a daemon that is restarting (a service manager's restart, say)
/// gets to come back before the bridge starts one of its own.
const GRACE: Duration = Duration::from_millis(500);
/// How long a daemon the bridge started gets to listen.
const START_WAIT: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(25);

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
    let connect_failed = |source| DaemonError::BridgeConnect {
        socket: socket.to_owned(),
        source,
    };
    let grace = Instant::now() + GRACE;
    loop {
        match UnixStream::connect(socket).await {
            Ok(stream) => return Ok(stream),
            Err(error) if nobody_listening(&error) => {}
            Err(error) => return Err(connect_failed(error)),
        }
        if Instant::now() >= grace {
            break;
        }
        tokio::time::sleep(POLL).await;
    }

    let mut child = start_daemon(socket)?;
    let deadline = Instant::now() + START_WAIT;
    loop {
        match UnixStream::connect(socket).await {
            Ok(stream) => return Ok(stream),
            Err(error) if nobody_listening(&error) => {}
            Err(error) => return Err(connect_failed(error)),
        }
        if let Ok(Some(status)) = child.try_wait() {
            // Another bridge may have started one first, which this one
            // then found already running.
            return UnixStream::connect(socket)
                .await
                .map_err(|_| DaemonError::BridgeDaemonExited {
                    socket: socket.to_owned(),
                    status,
                });
        }
        if Instant::now() >= deadline {
            return Err(DaemonError::BridgeDaemonSilent {
                socket: socket.to_owned(),
                waited: START_WAIT,
            });
        }
        tokio::time::sleep(POLL).await;
    }
}

/// Starts `humatad2 serve --role machine` on `socket` in a session of its
/// own, with no stdio, so it outlives the bridge and the link's hangup and
/// writes nothing into the stream.
fn start_daemon(socket: &Path) -> Result<Child, DaemonError> {
    let program = std::env::current_exe().map_err(|source| DaemonError::BridgeOwnBinary {
        socket: socket.to_owned(),
        source,
    })?;
    let mut command = Command::new(&program);
    command
        .args(["serve", "--role", "machine", "--socket"])
        .arg(socket)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: setsid is async-signal-safe and touches no memory of this
    // process, so it may run between fork and exec.
    unsafe {
        command.pre_exec(|| {
            rustix::process::setsid()
                .map(drop)
                .map_err(std::io::Error::from)
        });
    }
    command.spawn().map_err(|source| DaemonError::BridgeStart {
        program,
        socket: socket.to_owned(),
        source,
    })
}
