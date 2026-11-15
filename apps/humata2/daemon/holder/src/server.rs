//! The holder process: one program on one pseudo-terminal, its output
//! kept with offsets, served to any number of clients on a Unix socket.

use std::ffi::OsString;
use std::fs::{self, DirBuilder, File};
use std::io::{ErrorKind, Read, Write};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, PermissionsExt};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use rustix::fs::{Mode, OFlags};
use rustix::process::{Pid, Signal};
use rustix::pty::OpenptFlags;
use rustix::termios::Winsize;
use tokio::io::AsyncWriteExt;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, mpsc, watch};

use crate::HolderError;
use crate::protocol::{
    CHUNK, ClientFrame, ServerFrame, VERSION, Welcome, output_frame, read_frame,
};
use crate::scrollback::Scrollback;

/// Live output a slow client can fall this many chunks behind before it
/// is caught up from the scrollback instead.
const LIVE_CHUNKS: usize = 256;
/// How long the program gets to end after a hang-up before it's killed.
const HANG_UP_GRACE: Duration = Duration::from_secs(2);

/// What a holder runs and where it listens.
#[derive(Clone, Debug)]
pub struct ServeOptions {
    pub socket: PathBuf,
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub cwd: PathBuf,
    pub cols: u16,
    pub rows: u16,
    pub scrollback_bytes: usize,
}

struct State {
    kept: Scrollback,
    exited: Option<i32>,
    cols: u16,
    rows: u16,
}

#[derive(Clone)]
enum Live {
    Output { offset: u64, data: Arc<Vec<u8>> },
    Exited(i32),
}

struct Shared {
    state: Mutex<State>,
    live: broadcast::Sender<Live>,
    master: OwnedFd,
    writer: Mutex<File>,
    group: Option<Pid>,
    stop: watch::Sender<bool>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        // Plain data, still consistent after a panic elsewhere.
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn printed(&self, data: &[u8]) {
        let mut state = self.lock();
        let offset = state.kept.end();
        state.kept.push(data);
        // Sent under the lock, so a client that subscribes while holding
        // it sees each byte once: in its backlog or live.
        let _ = self.live.send(Live::Output {
            offset,
            data: Arc::new(data.to_vec()),
        });
    }

    fn exited(&self, code: i32) {
        let mut state = self.lock();
        state.exited = Some(code);
        let _ = self.live.send(Live::Exited(code));
    }

    /// What a client attaching from `from` is sent first, and its feed of
    /// what comes after.
    fn backlog(&self, from: u64) -> (Welcome, u64, Vec<u8>, broadcast::Receiver<Live>) {
        let state = self.lock();
        let welcome = Welcome {
            earliest: state.kept.earliest(),
            end: state.kept.end(),
            cols: state.cols,
            rows: state.rows,
            exited: state.exited,
        };
        let (start, bytes) = state.kept.since(from);
        (welcome, start, bytes, self.live.subscribe())
    }

    fn resize(&self, cols: u16, rows: u16) -> Result<(), HolderError> {
        set_size(&self.master, cols, rows).map_err(|source| HolderError::Resize { source })?;
        let mut state = self.lock();
        state.cols = cols.max(1);
        state.rows = rows.max(1);
        Ok(())
    }

    fn write(&self, data: &[u8]) -> Result<(), HolderError> {
        self.writer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .write_all(data)
            .map_err(|source| HolderError::Write { source })
    }

    /// Hangs up on the program, as closing a terminal window does, and
    /// kills what's left of its session after a grace period.
    async fn end_program(&self) {
        let Some(group) = self.group else { return };
        if self.lock().exited.is_none() {
            let _ = rustix::process::kill_process_group(group, Signal::HUP);
            let deadline = tokio::time::Instant::now() + HANG_UP_GRACE;
            while self.lock().exited.is_none() && tokio::time::Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
        // Whatever it started in the background goes too. Gone already is
        // fine.
        let _ = rustix::process::kill_process_group(group, Signal::KILL);
    }
}

/// Runs a holder until a client asks it to shut down or it gets SIGTERM.
/// `ready` is called once the socket is listening and the program has
/// started.
pub async fn serve(options: ServeOptions, ready: impl FnOnce()) -> Result<(), HolderError> {
    check_socket_path(&options.socket)?;
    let listener = bind(&options.socket).await?;
    let shared = match start(&options) {
        Ok(shared) => shared,
        Err(error) => {
            let _ = fs::remove_file(&options.socket);
            return Err(error);
        }
    };
    ready();

    let mut stopping = shared.stop.subscribe();
    let mut terminate =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).ok();
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => {
                    tokio::spawn(connection(shared.clone(), stream));
                }
                Err(error) => {
                    eprintln!("humata2_holder: couldn't accept a client: {error}");
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            },
            _ = stopping.wait_for(|stop| *stop) => break,
            Some(()) = async {
                match terminate.as_mut() {
                    Some(signal) => signal.recv().await,
                    None => std::future::pending().await,
                }
            } => break,
        }
    }
    drop(listener);
    let _ = fs::remove_file(&options.socket);
    shared.end_program().await;
    Ok(())
}

/// Unix socket paths are short: `sun_path` holds this many bytes before
/// its terminating NUL.
pub(crate) fn check_socket_path(socket: &Path) -> Result<(), HolderError> {
    let max = if cfg!(target_os = "linux") { 107 } else { 103 };
    if socket.as_os_str().len() > max {
        return Err(HolderError::SocketPathTooLong {
            socket: socket.to_owned(),
            max,
        });
    }
    Ok(())
}

async fn bind(socket: &Path) -> Result<UnixListener, HolderError> {
    let failed = |source| HolderError::Bind {
        socket: socket.to_owned(),
        source,
    };
    if let Some(folder) = socket.parent() {
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(folder)
            .map_err(failed)?;
    }
    match fs::symlink_metadata(socket) {
        Ok(found) if found.file_type().is_socket() => {
            if UnixStream::connect(socket).await.is_ok() {
                return Err(HolderError::AlreadyRunning {
                    socket: socket.to_owned(),
                });
            }
            // Left by a holder that was killed: nothing answers on it.
            fs::remove_file(socket).map_err(failed)?;
        }
        Ok(_) => {
            return Err(failed(std::io::Error::new(
                ErrorKind::AlreadyExists,
                "something other than a socket is in the way",
            )));
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(failed(error)),
    }
    let listener = UnixListener::bind(socket).map_err(failed)?;
    fs::set_permissions(socket, fs::Permissions::from_mode(0o600)).map_err(failed)?;
    Ok(listener)
}

fn set_size(master: &impl AsFd, cols: u16, rows: u16) -> Result<(), std::io::Error> {
    let size = Winsize {
        ws_row: rows.max(1),
        ws_col: cols.max(1),
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    rustix::termios::tcsetwinsize(master, size).map_err(Into::into)
}

fn open_pty(cols: u16, rows: u16) -> Result<(OwnedFd, OwnedFd), std::io::Error> {
    let master = rustix::pty::openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY)?;
    rustix::io::fcntl_setfd(&master, rustix::io::FdFlags::CLOEXEC)?;
    rustix::pty::grantpt(&master)?;
    rustix::pty::unlockpt(&master)?;
    let name = rustix::pty::ptsname(&master, Vec::new())?;
    // Not the holder's controlling terminal: only the program's.
    let pty = rustix::fs::open(
        name.as_c_str(),
        OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    set_size(&master, cols, rows)?;
    Ok((master, pty))
}

fn start(options: &ServeOptions) -> Result<Arc<Shared>, HolderError> {
    let opened = |source| HolderError::OpenPty { source };
    let (master, pty) = open_pty(options.cols, options.rows).map_err(opened)?;
    let started = |source| HolderError::StartProgram {
        program: options.program.clone(),
        cwd: options.cwd.clone(),
        source,
    };

    let mut command = Command::new(&options.program);
    command
        .args(&options.args)
        .current_dir(&options.cwd)
        .env("TERM", "xterm-256color")
        .env("COLORTERM", "truecolor")
        .stdin(Stdio::from(pty.try_clone().map_err(started)?))
        .stdout(Stdio::from(pty.try_clone().map_err(started)?))
        .stderr(Stdio::from(pty));
    if std::env::var_os("LANG").is_none() {
        command.env("LANG", "en_US.UTF-8");
    }
    // SAFETY: between fork and exec this only makes system calls that are
    // safe there: it starts a session, leaving the holder's, and takes the
    // pseudo-terminal, already the program's stdin, as its controlling
    // terminal so job control and ^C work.
    unsafe {
        command.pre_exec(|| {
            rustix::process::setsid()?;
            rustix::process::ioctl_tiocsctty(BorrowedFd::borrow_raw(0))?;
            Ok(())
        });
    }
    let mut child = command.spawn().map_err(started)?;
    // The program holds the only copies of its side now, so reading ends
    // when it and what it started close the terminal.
    drop(command);
    let group = i32::try_from(child.id()).ok().and_then(Pid::from_raw);

    let reader = File::from(master.try_clone().map_err(opened)?);
    let writer = File::from(master.try_clone().map_err(opened)?);
    let shared = Arc::new(Shared {
        state: Mutex::new(State {
            kept: Scrollback::new(options.scrollback_bytes),
            exited: None,
            cols: options.cols.max(1),
            rows: options.rows.max(1),
        }),
        live: broadcast::channel(LIVE_CHUNKS).0,
        master,
        writer: Mutex::new(writer),
        group,
        stop: watch::channel(false).0,
    });

    let (read_all, all_read) = std::sync::mpsc::channel::<()>();
    let printing = shared.clone();
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut buffer = vec![0; CHUNK];
        // Ends at EOF, or EIO on macOS and Linux once the program's side
        // has closed.
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => printing.printed(&buffer[..read]),
                Err(error) if error.kind() == ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
        let _ = read_all.send(());
    });
    let waiting = shared.clone();
    std::thread::spawn(move || {
        let code = match child.wait() {
            Ok(status) => status
                .code()
                .or_else(|| status.signal().map(|signal| 128 + signal))
                .unwrap_or(-1),
            Err(_) => -1,
        };
        // What it printed last comes before the exit, unless something it
        // left running keeps the terminal open.
        let _ = all_read.recv_timeout(Duration::from_secs(1));
        waiting.exited(code);
    });
    Ok(shared)
}

async fn connection(shared: Arc<Shared>, stream: UnixStream) {
    let (mut read, mut write) = stream.into_split();
    let from = match read_frame(&mut read).await {
        Ok(Some(frame)) => match ClientFrame::decode(&frame) {
            Ok(ClientFrame::Hello { version, from }) if version == VERSION => from,
            Ok(ClientFrame::Hello { version, .. }) => {
                let message = format!(
                    "it speaks version {VERSION} and the client version {version}; humatad2 and humata2_holder come from different builds. Reinstall Humata on this machine"
                );
                let _ = write
                    .write_all(&ServerFrame::Refused { message }.encode())
                    .await;
                return;
            }
            _ => return,
        },
        _ => return,
    };
    let _ = feed(&shared, from, read, &mut write).await;
}

/// Sends the backlog from `from`, then live output, until the client goes.
async fn feed(
    shared: &Arc<Shared>,
    from: u64,
    read: OwnedReadHalf,
    write: &mut OwnedWriteHalf,
) -> Result<(), std::io::Error> {
    let (welcome, start, backlog, mut live) = shared.backlog(from);
    write.write_all(&ServerFrame::Welcome(welcome).encode()).await?;
    let mut sent_exit = welcome.exited.is_some();
    let mut cursor = send_backlog(write, from, start, &backlog).await?;

    let (replies, mut replying) = mpsc::channel(16);
    let commands = tokio::spawn(commands(shared.clone(), read, replies));
    let fed = loop {
        tokio::select! {
            reply = replying.recv() => match reply {
                Some(frame) => write.write_all(&frame).await?,
                // The client hung up.
                None => break Ok(()),
            },
            update = live.recv() => match update {
                Ok(Live::Output { offset, data }) => {
                    let end = offset + data.len() as u64;
                    if end <= cursor {
                        continue;
                    }
                    let skip = usize::try_from(cursor.saturating_sub(offset)).unwrap_or(0);
                    write.write_all(&output_frame(offset.max(cursor), &data[skip..])).await?;
                    cursor = end;
                }
                Ok(Live::Exited(code)) => {
                    if !sent_exit {
                        write.write_all(&ServerFrame::Exited { code }.encode()).await?;
                        sent_exit = true;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    // Fell behind: catch up from the scrollback.
                    let (welcome, start, backlog, fresh) = shared.backlog(cursor);
                    live = fresh;
                    cursor = send_backlog(write, cursor, start, &backlog).await?;
                    if let (Some(code), false) = (welcome.exited, sent_exit) {
                        write.write_all(&ServerFrame::Exited { code }.encode()).await?;
                        sent_exit = true;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break Ok(()),
            },
        }
    };
    commands.abort();
    fed
}

/// Sends `bytes`, which start at `start`, saying first if what the client
/// asked for from `from` was already trimmed. Returns the new cursor.
async fn send_backlog(
    write: &mut OwnedWriteHalf,
    from: u64,
    start: u64,
    bytes: &[u8],
) -> Result<u64, std::io::Error> {
    if from < start {
        write
            .write_all(&ServerFrame::Trimmed { resume_at: start }.encode())
            .await?;
    }
    let mut offset = start;
    for chunk in bytes.chunks(CHUNK) {
        write.write_all(&output_frame(offset, chunk)).await?;
        offset += chunk.len() as u64;
    }
    Ok(offset)
}

/// Carries out what the client asks, answering through `replies`.
async fn commands(shared: Arc<Shared>, mut read: OwnedReadHalf, replies: mpsc::Sender<Vec<u8>>) {
    while let Ok(Some(frame)) = read_frame(&mut read).await {
        let reply = match ClientFrame::decode(&frame) {
            Ok(ClientFrame::Input { seq, last, data }) => {
                let typing = shared.clone();
                let written = tokio::task::spawn_blocking(move || typing.write(&data)).await;
                match written {
                    Ok(Ok(())) if last => Some(ServerFrame::InputWritten { seq }),
                    Ok(Ok(())) => None,
                    Ok(Err(error)) => Some(refused(&error)),
                    Err(_) => Some(ServerFrame::Refused {
                        message: "typing was interrupted; type again".to_owned(),
                    }),
                }
            }
            Ok(ClientFrame::Resize { cols, rows }) => {
                shared.resize(cols, rows).err().map(|error| refused(&error))
            }
            Ok(ClientFrame::Shutdown) => {
                shared.stop.send_replace(true);
                None
            }
            Ok(ClientFrame::Hello { .. }) | Err(_) => break,
        };
        if let Some(reply) = reply
            && replies.send(reply.encode()).await.is_err()
        {
            break;
        }
    }
}

fn refused(error: &HolderError) -> ServerFrame {
    ServerFrame::Refused {
        message: error.to_string(),
    }
}
