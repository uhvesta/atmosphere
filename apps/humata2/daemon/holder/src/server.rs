//! The holder process: one program on one pseudo-terminal, its output
//! kept with offsets, served to any number of clients on a Unix socket.

use std::collections::{BTreeSet, VecDeque};
use std::ffi::OsString;
use std::fs::{self, DirBuilder, File};
use std::future::Future;
use std::io::{ErrorKind, PipeReader, PipeWriter, Read, Write};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd, RawFd};
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::ffi::OsStringExt;
use std::os::unix::net::UnixStream as StdUnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use rustix::event::{PollFd, PollFlags};
use rustix::fs::{Mode, OFlags};
use rustix::process::{Pid, Signal, WaitId, WaitIdOptions, WaitOptions};
use rustix::pty::OpenptFlags;
use rustix::termios::Winsize;
use tokio::io::AsyncWriteExt;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Notify, broadcast, mpsc, watch};

use crate::HolderError;
use crate::protocol::{
    CHUNK, ClientFrame, MAX_KEYED, ServerFrame, VERSION, Welcome, output_frame, read_frame,
};
use crate::process::{Snapshot, own_group, send_group};
use crate::saved::{SYNC_EVERY, SaveLog};
use crate::scrollback::Scrollback;

#[path = "take_over.rs"]
mod take_over;

pub use take_over::{check_take_over, handoff_path, handoff_versions, take_over};

/// Live output a slow client can fall this many chunks behind before it
/// is caught up from the scrollback instead.
const LIVE_CHUNKS: usize = 32;
/// How long the program gets to end after a hang-up before it's killed.
const HANG_UP_GRACE: Duration = Duration::from_secs(2);
/// How many keyed inputs the holder remembers having typed.
const KEYS_KEPT: usize = 256;
/// How often an `AwaitKeys` looks at the terminal's modes again: nothing
/// tells the holder when the program changes them.
const KEYS_LOOK_EVERY: Duration = Duration::from_millis(20);

/// The keys of the keyed inputs typed, the most recent `KEYS_KEPT`. Kept
/// in memory only: a holder that starts again runs a new program on a new
/// terminal, which nothing sent before it could have reached.
struct Keys(VecDeque<String>);

impl Keys {
    fn has(&self, key: &str) -> bool {
        self.0.iter().any(|kept| kept == key)
    }

    fn remember(&mut self, key: String) {
        if self.0.len() == KEYS_KEPT {
            self.0.pop_front();
        }
        self.0.push_back(key);
    }
}

/// A keyed input being received: held until whole, then typed at once.
struct Pending {
    key: String,
    data: Vec<u8>,
    /// Went over `MAX_KEYED`; the rest is dropped.
    overflowed: bool,
}

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
    /// A line shown before the program's first output, such as why the
    /// terminal was started again. Plain text; written as its own lines.
    pub banner: Option<String>,
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
    keys: Mutex<Keys>,
    /// Names this run of the holder; see `new_instance`.
    instance: u64,
    group: Option<Pid>,
    stop: watch::Sender<bool>,
    /// Output was saved and not yet synced.
    wrote: Notify,
    /// The program's exit code, once it has exited.
    exit: watch::Sender<Option<i32>>,
    /// When the program started.
    started: tokio::time::Instant,
    /// Test builds only: where each sync says how far it got.
    #[cfg(feature = "faults")]
    synced_at: PathBuf,
    /// The socket this holder serves.
    socket: PathBuf,
    /// The listening socket's descriptor, handed on when moving.
    listener: RawFd,
    /// Held by the reader while it reads the terminal and keeps what it
    /// read, so a move can stop it between two reads.
    gate: Mutex<()>,
    /// Held while the program is reaped, and by a move: the exit code
    /// once the program was reaped, so no move loses it.
    reaped: Mutex<Option<i32>>,
    /// A move is under way.
    moving: AtomicBool,
    /// The process groups stopped by a `Freeze`, while the program is
    /// frozen for a cross-machine move (D801 J). Typing is refused while
    /// it is set. Kept in memory: a move onto a new build is refused
    /// while it is set, so nothing hands it over.
    frozen: Mutex<Option<BTreeSet<u32>>>,
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
        // Read first: every byte before it is in the file the take below
        // syncs.
        #[cfg(feature = "faults")]
        let end = {
            let state = self.lock();
            state.saved.is_some().then(|| state.kept.end())
        };
        let file = self.lock().saved.as_mut().and_then(SaveLog::take_unsynced);
        #[cfg_attr(not(feature = "faults"), allow(unused_variables))]
        let synced = match file {
            Some(file) => file.sync_data().is_ok(),
            None => true,
        };
        // Nothing left to sync counts as synced through the end. A save log
        // that failed does too, and a test then finds its output missing.
        #[cfg(feature = "faults")]
        if synced && let Some(end) = end {
            let _ = fs::write(&self.synced_at, end.to_string());
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

    fn is_frozen(&self) -> bool {
        self.frozen
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_some()
    }

    /// Stops the program: SIGSTOP to the process groups `end_by_pid`
    /// targets, those under this holder. The holder goes on serving the
    /// screen as it is. Freezing a frozen program, or one that has ended,
    /// does nothing.
    async fn freeze(&self) -> Result<(), HolderError> {
        if self.exit.borrow().is_some() || self.is_frozen() {
            return Ok(());
        }
        let own = own_group();
        let pid = std::process::id();
        let listing = |source| HolderError::Freeze {
            socket: self.socket.clone(),
            source,
        };
        let mut groups = Snapshot::take()
            .await
            .map_err(listing)?
            .groups_under(pid, own);
        if let Some(group) = self.group.and_then(|group| u32::try_from(group.as_raw_pid()).ok()) {
            groups.insert(group);
        }
        // Set before the signals, so nothing is typed into a program that
        // is about to stop.
        {
            let mut frozen = self
                .frozen
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if frozen.is_some() {
                return Ok(());
            }
            *frozen = Some(groups.clone());
        }
        for group in &groups {
            send_group(*group, Signal::STOP);
        }
        // What started in the meantime is stopped too.
        if let Ok(snapshot) = Snapshot::take().await {
            let late: Vec<u32> = snapshot
                .groups_under(pid, own)
                .into_iter()
                .filter(|group| !groups.contains(group))
                .collect();
            for group in &late {
                send_group(*group, Signal::STOP);
            }
            if let Some(frozen) = self
                .frozen
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .as_mut()
            {
                frozen.extend(late);
            }
        }
        Ok(())
    }

    /// Lets a frozen program run again: SIGCONT to the groups stopped.
    /// Thawing one that isn't frozen does nothing.
    fn thaw(&self) {
        let groups = self
            .frozen
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        for group in groups.into_iter().flatten() {
            send_group(group, Signal::CONT);
        }
    }

    fn write(&self, data: &[u8]) -> Result<(), HolderError> {
        if self.is_frozen() {
            return Err(HolderError::Frozen {
                socket: self.socket.clone(),
            });
        }
        self.writer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .write_all(data)
            .map_err(|source| HolderError::Write { source })
    }

    /// Types `data` unless `key` was typed already, and remembers `key`
    /// once all of it is written. Returns whether it was a repeat. Keyed
    /// inputs are typed one at a time, so two connections sending the same
    /// key can't both type it.
    fn write_keyed(&self, key: &str, data: &[u8]) -> Result<bool, HolderError> {
        let mut keys = self
            .keys
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if keys.has(key) {
            return Ok(true);
        }
        self.write(data)?;
        keys.remember(key.to_owned());
        Ok(false)
    }

    /// Whether the program reads keys itself: it turned the terminal's line
    /// editing off, as a full-screen program does once it's up. Until then
    /// the terminal echoes and holds what's typed itself.
    fn reads_keys(&self) -> bool {
        rustix::termios::tcgetattr(&self.master).is_ok_and(|modes| {
            !modes
                .local_modes
                .contains(rustix::termios::LocalModes::ICANON)
        })
    }

    /// Waits until the program reads keys itself, it ends, or it has run
    /// for `starting`: one that still edits lines by then is taken to keep
    /// doing so. Returns whether it reads keys.
    async fn await_keys(&self, starting: Duration) -> bool {
        let mut exit = self.exit.subscribe();
        let deadline = self.started + starting;
        loop {
            if self.reads_keys() {
                return true;
            }
            if exit.borrow_and_update().is_some() || tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::select! {
                () = tokio::time::sleep(KEYS_LOOK_EVERY) => {}
                _ = exit.changed() => {}
            }
        }
    }

    fn has_key(&self, key: &str) -> bool {
        self.keys
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .has(key)
    }

    /// Hangs up on the program, as closing a terminal window does, and
    /// kills what's left of its session after a grace period.
    async fn end_program(&self) {
        let Some(group) = self.group else { return };
        let mut exit = self.exit.subscribe();
        if exit.borrow().is_none() {
            let _ = rustix::process::kill_process_group(group, Signal::HUP);
            // A stopped program takes the hangup only once it runs again.
            self.thaw();
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
    // A marker a holder cut on this socket left says nothing of this one.
    #[cfg(feature = "faults")]
    let _ = fs::remove_file(synced_path(&options.socket));
    let started = match &options.saved {
        Some(path) => SaveLog::open(path, options.scrollback_bytes)
            .map(|(log, kept)| (Some(log), kept)),
        None => Ok((None, Scrollback::new(options.scrollback_bytes))),
    }
    .and_then(|(saved, kept)| start(&options, kept, saved, listener.as_raw_fd()));
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
    #[cfg(feature = "faults")]
    let _ = SERVING.set(options.socket.clone());
    ready();
    serve_clients(shared, listener, &options.socket, &pid_file).await;
    Ok(())
}

/// Serves clients on `listener` until one shuts the holder down or it gets
/// SIGTERM, then cleans up beside the socket.
async fn serve_clients(
    shared: Arc<Shared>,
    listener: UnixListener,
    socket: &Path,
    pid_file: &Path,
) {
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
    #[cfg(feature = "faults")]
    let _ = fs::remove_file(&shared.synced_at);
    let _ = fs::remove_file(pid_file);
    drop(listener);
    let _ = fs::remove_file(socket);
    shared.end_program().await;
}

/// Test builds only: the socket this holder serves, so a fault can be
/// switched on beside it.
#[cfg(feature = "faults")]
static SERVING: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Test builds only: whether the holder ignores its shutdown while still
/// answering, as a wedged one would. A file `<socket stem>.ignore-shutdown`
/// beside its socket switches it on.
#[cfg(feature = "faults")]
fn ignores_shutdown() -> bool {
    SERVING
        .get()
        .is_some_and(|socket| socket.with_extension("ignore-shutdown").exists())
}

/// Test builds only: the file beside `socket` where its holder writes the
/// offset its saved output is synced through, as decimal text, after each
/// sync. A power-cut test waits for it, not for a guessed time.
#[cfg(feature = "faults")]
fn synced_path(socket: &Path) -> PathBuf {
    socket.with_extension("synced")
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
        // What was written during the wait is covered by this sync, so its
        // notice is taken back: a burst is synced once. Output that lands
        // while syncing leaves a new notice and gets a sync of its own.
        tokio::select! {
            biased;
            () = wrote.notified() => {}
            () = std::future::ready(()) => {}
        }
        sync().await;
    }
}

/// An id that differs between runs of a holder, even for the same tab: the
/// start time in nanoseconds mixed with the process id. Never 0, which
/// clients use for "not known".
fn new_instance() -> u64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| u64::try_from(since.as_nanos()).unwrap_or(0));
    let id = nanos ^ u64::from(std::process::id()).rotate_left(40);
    id.max(1)
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

/// `text` as the terminal shows it: on lines of its own (after what's kept,
/// if anything is), a blank line after it. It doesn't pass through the
/// terminal, so its new lines are written as the terminal writes them, and
/// other control characters are dropped.
fn banner_bytes(text: &str, after_output: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() + 6);
    if after_output {
        out.extend_from_slice(b"\r\n");
    }
    for c in text.chars() {
        match c {
            '\n' => out.extend_from_slice(b"\r\n"),
            '\t' => out.push(b'\t'),
            c if c.is_control() => {}
            c => out.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes()),
        }
    }
    out.extend_from_slice(b"\r\n\r\n");
    out
}

fn start(
    options: &ServeOptions,
    mut kept: Scrollback,
    mut saved: Option<SaveLog>,
    listener: RawFd,
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
    let child = command.spawn().map_err(started)?;
    // The program holds the only copies of its side now, so reading ends
    // when it and what it started close the terminal.
    drop(command);
    // After the program started, so a failed start leaves no banner.
    if let Some(text) = &options.banner {
        let line = banner_bytes(text, kept.end() > 0);
        let offset = kept.end();
        kept.push(&line);
        if let Some(saved) = saved.as_mut() {
            saved.output(offset, &line, &kept);
        }
    }
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
        keys: Mutex::new(Keys(VecDeque::new())),
        instance: new_instance(),
        group,
        stop: watch::channel(false).0,
        wrote: Notify::new(),
        exit: watch::channel(None).0,
        started: tokio::time::Instant::now(),
        #[cfg(feature = "faults")]
        synced_at: synced_path(&options.socket),
        socket: options.socket.clone(),
        listener,
        gate: Mutex::new(()),
        reaped: Mutex::new(None),
        moving: AtomicBool::new(false),
        frozen: Mutex::new(None),
    });

    let (exit_read, exit_write) = std::io::pipe().map_err(started)?;
    start_threads(&shared, reader, exit_read, exit_write, group, None);
    Ok(shared)
}

/// Starts the threads that pass on the program's output and wait for its
/// exit. `known` is its exit code if it was reaped already, as by the
/// image a holder moved from.
fn start_threads(
    shared: &Arc<Shared>,
    reader: File,
    exit_read: PipeReader,
    mut exit_write: PipeWriter,
    child: Option<Pid>,
    known: Option<i32>,
) {
    if let Some(code) = known {
        let _ = exit_write.write_all(&code.to_le_bytes());
    }
    let printing = shared.clone();
    std::thread::spawn(move || read_output(&reader, exit_read, &printing));
    if known.is_none()
        && let Some(pid) = child
    {
        let waiting = shared.clone();
        std::thread::spawn(move || wait_for_exit(pid, &waiting, exit_write));
    }
}

/// Waits for the program to exit, reaps it and sends its exit code on
/// `exit`. It waits without reaping first, so a move can happen while it
/// waits and the image it moves onto still finds the program to reap; the
/// reaping is under the lock a move takes.
fn wait_for_exit(pid: Pid, shared: &Shared, mut exit: PipeWriter) {
    loop {
        match rustix::process::waitid(
            WaitId::Pid(pid),
            WaitIdOptions::EXITED | WaitIdOptions::NOWAIT,
        ) {
            Err(rustix::io::Errno::INTR) => {}
            _ => break,
        }
    }
    let mut reaped = shared
        .reaped
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let code = match rustix::process::waitpid(Some(pid), WaitOptions::empty()) {
        Ok(Some((_, status))) => status
            .exit_status()
            .or_else(|| status.terminating_signal().map(|signal| 128 + signal))
            .unwrap_or(-1),
        _ => -1,
    };
    *reaped = Some(code);
    let _ = exit.write_all(&code.to_le_bytes());
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
            // A move waits here, between two reads.
            let _gate = shared
                .gate
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
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
    // A copy of the socket for the one thing written outside `feed`: the
    // word that a move is under way, which must be sent before the move.
    let Ok(copy) = stream.as_fd().try_clone_to_owned() else {
        return;
    };
    let told = Arc::new(StdUnixStream::from(copy));
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
    let _ = feed(&shared, from, read, &mut write, told).await;
}

/// Sends the backlog from `from`, then live output, until the client goes.
async fn feed(
    shared: &Arc<Shared>,
    from: u64,
    read: OwnedReadHalf,
    write: &mut OwnedWriteHalf,
    told: Arc<StdUnixStream>,
) -> Result<(), std::io::Error> {
    let (welcome, backlog, mut live) = shared.backlog(from);
    write.write_all(&ServerFrame::Welcome(welcome).encode()).await?;
    let mut sent_exit = welcome.exited.is_some();
    let mut cursor = send_backlog(write, from, welcome.end, &backlog).await?;

    let (replies, mut replying) = mpsc::channel(16);
    let commands = tokio::spawn(commands(shared.clone(), read, replies, told));
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
async fn commands(
    shared: Arc<Shared>,
    mut read: OwnedReadHalf,
    replies: mpsc::Sender<Vec<u8>>,
    told: Arc<StdUnixStream>,
) {
    // Dropped with the connection: a keyed input cut off before its last
    // chunk types nothing.
    let mut pending: Option<Pending> = None;
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
            Ok(ClientFrame::KeyedInput { key, last, data }) => {
                if pending.as_ref().is_none_or(|held| held.key != key) {
                    pending = Some(Pending {
                        key,
                        data: Vec::new(),
                        overflowed: false,
                    });
                }
                let Some(held) = pending.as_mut() else { continue };
                let mut reply = None;
                if !held.overflowed {
                    if held.data.len() + data.len() > MAX_KEYED {
                        held.overflowed = true;
                        held.data = Vec::new();
                        reply = Some(ServerFrame::Refused {
                            message: format!(
                                "keyed input over {MAX_KEYED} bytes; nothing was typed. Send it in smaller parts"
                            ),
                        });
                    } else {
                        held.data.extend_from_slice(&data);
                    }
                }
                if last
                    && let Some(done) = pending.take()
                    && !done.overflowed
                {
                    reply = Some(type_keyed(&shared, done).await);
                }
                reply
            }
            Ok(ClientFrame::KeyQuery { key }) => Some(ServerFrame::KeyAnswer {
                arrived: shared.has_key(&key),
                instance: shared.instance,
            }),
            Ok(ClientFrame::AwaitKeys { starting_ms }) => Some(ServerFrame::KeysRead {
                ready: shared
                    .await_keys(Duration::from_millis(u64::from(starting_ms)))
                    .await,
            }),
            Ok(ClientFrame::MoveTo { path }) => {
                let moving = shared.clone();
                let told = told.clone();
                let handle = tokio::runtime::Handle::current();
                let path = PathBuf::from(OsString::from_vec(path));
                // Returns only if the move failed.
                let failed = tokio::task::spawn_blocking(move || {
                    take_over::try_move(&moving, &path, &told, &handle)
                })
                .await;
                Some(ServerFrame::Refused {
                    message: failed.unwrap_or_else(|_| {
                        "the move was interrupted; the holder is unchanged, try again".to_owned()
                    }),
                })
            }
            Ok(ClientFrame::Freeze) => Some(match shared.freeze().await {
                Ok(()) => ServerFrame::Frozen {
                    frozen: shared.is_frozen(),
                },
                Err(error) => refused(&error),
            }),
            Ok(ClientFrame::Thaw) => {
                shared.thaw();
                Some(ServerFrame::Frozen { frozen: false })
            }
            Ok(ClientFrame::FrozenQuery) => Some(ServerFrame::Frozen {
                frozen: shared.is_frozen(),
            }),
            Ok(ClientFrame::Resize { cols, rows }) => {
                shared.resize(cols, rows).err().map(|error| refused(&error))
            }
            Ok(ClientFrame::Shutdown) => {
                #[cfg(feature = "faults")]
                let ignored = ignores_shutdown();
                #[cfg(not(feature = "faults"))]
                let ignored = false;
                if !ignored {
                    shared.stop.send_replace(true);
                }
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

/// Types a whole keyed input in one go.
async fn type_keyed(shared: &Arc<Shared>, input: Pending) -> ServerFrame {
    let typing = shared.clone();
    let written =
        tokio::task::spawn_blocking(move || typing.write_keyed(&input.key, &input.data)).await;
    match written {
        Ok(Ok(repeat)) => ServerFrame::KeyedWritten { repeat },
        Ok(Err(error)) => refused(&error),
        Err(_) => ServerFrame::Refused {
            message: "typing was interrupted; ask whether it arrived".to_owned(),
        },
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

    use super::{KEYS_KEPT, Keys, banner_bytes, sync_after_writes};

    #[test]
    fn a_banner_is_its_own_lines_as_plain_text() {
        assert_eq!(banner_bytes("one\ntwo", false), b"one\r\ntwo\r\n\r\n");
        assert_eq!(banner_bytes("x\x1b[2Jy", false), b"x[2Jy\r\n\r\n");
        assert!(banner_bytes("after", true).starts_with(b"\r\n"));
    }

    #[test]
    fn the_oldest_key_is_forgotten_after_256() {
        let mut keys = Keys(Default::default());
        for n in 0..=KEYS_KEPT {
            keys.remember(format!("k{n}"));
        }
        assert!(!keys.has("k0"));
        assert!(keys.has("k1"));
        assert!(keys.has(&format!("k{KEYS_KEPT}")));
    }

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
        // The syncing side is polled first, as in the holder, where it is
        // already waiting when output arrives.
        let counts = tokio::select! {
            biased;
            () = syncing(&wrote, &syncs) => unreachable!(),
            counts = check => counts,
        };
        assert_eq!(counts, (0, 1, 1));
    }
}
