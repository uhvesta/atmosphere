//! The holder process: one program on one pseudo-terminal, its output
//! kept with offsets, served to any number of clients on a Unix socket.

use std::ffi::OsString;
use std::fs::{self, DirBuilder, File};
use std::future::Future;
use std::io::{ErrorKind, PipeReader, Read, Write};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use rustix::event::{PollFd, PollFlags};
use rustix::fs::{Mode, OFlags};
use rustix::process::{Pid, Signal};
use rustix::pty::OpenptFlags;
use rustix::termios::Winsize;
use tokio::io::AsyncWriteExt;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Notify, broadcast, mpsc, watch};

use crate::HolderError;
use crate::protocol::{
    CHUNK, ClientFrame, ServerFrame, VERSION, Welcome, output_frame, read_frame,
};
use crate::saved::{SYNC_EVERY, SaveLog};
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
    /// Where output is saved, so a holder started after this one is
    /// killed shows it again. Removed when a client shuts the holder
    /// down; kept when it's stopped by a signal.
    pub saved: Option<PathBuf>,
}

struct State {
    kept: Scrollback,
    saved: Option<SaveLog>,
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
    /// Output was saved and not yet synced.
    wrote: Notify,
    /// The program's exit code, once it has exited.
    exit: watch::Sender<Option<i32>>,
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
        let State { kept, saved, .. } = &mut *state;
        kept.push(data);
        // Saved before any client sees it.
        if let Some(saved) = saved {
            saved.output(offset, data, kept);
            self.wrote.notify_one();
        }
        // Sent under the lock, so a client that subscribes while holding
        // it sees each byte once: in its backlog or live.
        let _ = self.live.send(Live::Output {
            offset,
            data: Arc::new(data.to_vec()),
        });
    }

    /// Syncs output written since the last sync.
    fn sync_saved(&self) {
        let file = self.lock().saved.as_mut().and_then(SaveLog::take_unsynced);
        if let Some(file) = file {
            let _ = file.sync_data();
        }
    }

    /// Stops saving output and removes what was saved.
    fn forget_saved(&self) {
        if let Some(saved) = self.lock().saved.take() {
            let _ = humata2_durable::remove_file(saved.path());
        }
    }

    fn exited(&self, code: i32) {
        let mut state = self.lock();
        state.exited = Some(code);
        let _ = self.live.send(Live::Exited(code));
        self.exit.send_replace(Some(code));
    }

    /// What a client attaching from `from` is sent first, and its feed of
    /// what comes after.
    fn backlog(&self, from: u64) -> (Welcome, Vec<(u64, Vec<u8>)>, broadcast::Receiver<Live>) {
        let state = self.lock();
        let welcome = Welcome {
            earliest: state.kept.earliest(),
            end: state.kept.end(),
            cols: state.cols,
            rows: state.rows,
            exited: state.exited,
        };
        let runs = state.kept.since(from);
        (welcome, runs, self.live.subscribe())
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
        let mut exit = self.exit.subscribe();
        if exit.borrow().is_none() {
            let _ = rustix::process::kill_process_group(group, Signal::HUP);
            let _ = tokio::time::timeout(HANG_UP_GRACE, exit.wait_for(Option::is_some)).await;
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
    let pid_file = pid_path(&options.socket);
    if let Err(source) = write_pid(&pid_file) {
        let _ = fs::remove_file(&options.socket);
        return Err(HolderError::Bind {
            socket: options.socket.clone(),
            source,
        });
    }
    let started = match &options.saved {
        Some(path) => SaveLog::open(path, options.scrollback_bytes)
            .map(|(log, kept)| (Some(log), kept)),
        None => Ok((None, Scrollback::new(options.scrollback_bytes))),
    }
    .and_then(|(saved, kept)| start(&options, kept, saved));
    let shared = match started {
        Ok(shared) => shared,
        Err(error) => {
            let _ = fs::remove_file(&options.socket);
            let _ = fs::remove_file(&pid_file);
            return Err(error);
        }
    };
    // Before saying it's ready, so a SIGUSR2 sent once it is never takes
    // the signal's default.
    #[cfg(feature = "faults")]
    power_cut_on_sigusr2();
    ready();

    let syncing = shared.clone();
    let sync = tokio::spawn(async move {
        sync_after_writes(&syncing.wrote, || {
            let shared = syncing.clone();
            async move {
                let _ = tokio::task::spawn_blocking(move || shared.sync_saved()).await;
            }
        })
        .await;
    });

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
    sync.abort();
    let shut_down = *shared.stop.borrow();
    // Lets go of the files beside the socket before the socket itself, so
    // a holder started on it once it's gone finds them as left.
    if shut_down {
        // Shut down by a client: the terminal is closed for good.
        shared.forget_saved();
    } else {
        // Stopped by a signal, as at logout or a restart: what it printed
        // stays for the holder started after it.
        shared.sync_saved();
        shared.lock().saved.take();
    }
    let _ = fs::remove_file(&pid_file);
    drop(listener);
    let _ = fs::remove_file(&options.socket);
    shared.end_program().await;
    Ok(())
}

/// Test builds only: SIGUSR2 stops the holder as a power cut would, with
/// what it wrote and didn't sync undone (see humata2_durable).
#[cfg(feature = "faults")]
fn power_cut_on_sigusr2() {
    let signal = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::user_defined2());
    if let Ok(mut signal) = signal {
        tokio::spawn(async move {
            if signal.recv().await.is_some() {
                humata2_durable::cut("SIGUSR2");
            }
        });
    }
}

/// Calls `sync` half a second after output is saved, so a burst of
/// output is synced once, and never while nothing is written: an idle
/// holder has no timer running.
async fn sync_after_writes<F: Future<Output = ()>>(wrote: &Notify, mut sync: impl FnMut() -> F) {
    loop {
        wrote.notified().await;
        tokio::time::sleep(SYNC_EVERY).await;
        sync().await;
    }
}

fn pid_path(socket: &Path) -> PathBuf {
    socket.with_extension("pid")
}

fn write_pid(path: &Path) -> std::io::Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    write!(file, "{}", std::process::id())
}

/// The process id of the holder answering on `socket`, as it wrote it
/// beside the socket when it started. Only trust it while a holder
/// answers there: one that was killed leaves its file behind.
pub fn holder_pid(socket: &Path) -> Option<u32> {
    fs::read_to_string(pid_path(socket)).ok()?.trim().parse().ok()
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

fn start(
    options: &ServeOptions,
    kept: Scrollback,
    saved: Option<SaveLog>,
) -> Result<Arc<Shared>, HolderError> {
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
            kept,
            saved,
            exited: None,
            cols: options.cols.max(1),
            rows: options.rows.max(1),
        }),
        live: broadcast::channel(LIVE_CHUNKS).0,
        master,
        writer: Mutex::new(writer),
        group,
        stop: watch::channel(false).0,
        wrote: Notify::new(),
        exit: watch::channel(None).0,
    });

    let (exit_read, mut exit_write) = std::io::pipe().map_err(started)?;
    let printing = shared.clone();
    std::thread::spawn(move || read_output(&reader, exit_read, &printing));
    std::thread::spawn(move || {
        let code = match child.wait() {
            Ok(status) => status
                .code()
                .or_else(|| status.signal().map(|signal| 128 + signal))
                .unwrap_or(-1),
            Err(_) => -1,
        };
        let _ = exit_write.write_all(&code.to_le_bytes());
    });
    Ok(shared)
}

/// Passes on what the program prints until its terminal closes, and its
/// exit code, sent on `exit`, once what it printed before it exited is
/// passed on. Something it left running may keep the terminal open after
/// it exits; its output goes on being passed on after the exit.
fn read_output(reader: &File, mut exit: PipeReader, shared: &Shared) {
    let mut buffer = vec![0; CHUNK];
    let mut code = None;
    let mut reported = false;
    let now = rustix::event::Timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    loop {
        // Once the program has exited, whatever it printed is already
        // waiting on the terminal: a check that finds nothing there means
        // it has all been passed on.
        let checking = code.is_some() && !reported;
        let mut fds = vec![PollFd::new(reader, PollFlags::IN)];
        if code.is_none() {
            fds.push(PollFd::new(&exit, PollFlags::IN));
        }
        match rustix::event::poll(&mut fds, checking.then_some(&now)) {
            Ok(_) => {}
            Err(rustix::io::Errno::INTR) => continue,
            Err(_) => break,
        }
        let printed = !fds[0].revents().is_empty();
        let exited = fds.get(1).is_some_and(|fd| !fd.revents().is_empty());
        drop(fds);
        if printed {
            // Ends at EOF, or EIO on macOS and Linux once the program's
            // side has closed.
            match (&*reader).read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => shared.printed(&buffer[..read]),
                Err(error) if error.kind() == ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        } else if checking && let Some(code) = code {
            shared.exited(code);
            reported = true;
        }
        if exited {
            code = Some(exit_code(&mut exit));
        }
    }
    if !reported {
        let code = code.unwrap_or_else(|| exit_code(&mut exit));
        shared.exited(code);
    }
}

/// The exit code the thread waiting on the program sends, or -1 if it
/// couldn't.
fn exit_code(exit: &mut PipeReader) -> i32 {
    let mut bytes = [0; 4];
    match exit.read_exact(&mut bytes) {
        Ok(()) => i32::from_le_bytes(bytes),
        Err(_) => -1,
    }
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
    let (welcome, backlog, mut live) = shared.backlog(from);
    write.write_all(&ServerFrame::Welcome(welcome).encode()).await?;
    let mut sent_exit = welcome.exited.is_some();
    let mut cursor = send_backlog(write, from, welcome.end, &backlog).await?;

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
                    if offset > cursor {
                        write.write_all(&ServerFrame::Trimmed { resume_at: offset }.encode()).await?;
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
                    let (welcome, backlog, fresh) = shared.backlog(cursor);
                    live = fresh;
                    cursor = send_backlog(write, cursor, welcome.end, &backlog).await?;
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

/// Sends the kept `runs` from `from` up to `end`, saying before each gap
/// in them that what the client asked for there isn't kept. Returns the
/// new cursor.
async fn send_backlog(
    write: &mut OwnedWriteHalf,
    from: u64,
    end: u64,
    runs: &[(u64, Vec<u8>)],
) -> Result<u64, std::io::Error> {
    let mut cursor = from.min(end);
    for (start, bytes) in runs {
        if *start > cursor {
            write
                .write_all(&ServerFrame::Trimmed { resume_at: *start }.encode())
                .await?;
        }
        let mut offset = *start;
        for chunk in bytes.chunks(CHUNK) {
            write.write_all(&output_frame(offset, chunk)).await?;
            offset += chunk.len() as u64;
        }
        cursor = offset;
    }
    if cursor < end {
        write
            .write_all(&ServerFrame::Trimmed { resume_at: end }.encode())
            .await?;
    }
    Ok(end)
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use tokio::sync::Notify;

    use super::sync_after_writes;

    async fn syncing(wrote: &Notify, syncs: &AtomicUsize) {
        sync_after_writes(wrote, || async {
            syncs.fetch_add(1, Ordering::SeqCst);
        })
        .await;
    }

    #[tokio::test(start_paused = true)]
    async fn an_idle_holder_never_syncs() {
        let (wrote, syncs) = (Notify::new(), AtomicUsize::new(0));
        tokio::select! {
            () = syncing(&wrote, &syncs) => unreachable!(),
            () = tokio::time::sleep(Duration::from_secs(24 * 3600)) => {}
        }
        assert_eq!(syncs.load(Ordering::SeqCst), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn a_burst_of_output_is_synced_once_half_a_second_after() {
        let (wrote, syncs) = (Notify::new(), AtomicUsize::new(0));
        let check = async {
            for _ in 0..50 {
                wrote.notify_one();
            }
            tokio::time::sleep(Duration::from_millis(490)).await;
            let early = syncs.load(Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(20)).await;
            let after = syncs.load(Ordering::SeqCst);
            tokio::time::sleep(Duration::from_secs(3600)).await;
            (early, after, syncs.load(Ordering::SeqCst))
        };
        let counts = tokio::select! {
            () = syncing(&wrote, &syncs) => unreachable!(),
            counts = check => counts,
        };
        assert_eq!(counts, (0, 1, 1));
    }
}
