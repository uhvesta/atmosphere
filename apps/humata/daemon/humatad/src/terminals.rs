//! Shells on this host, each on its own pseudo-terminal, kept running while
//! no one watches them and replayed to whoever attaches.

use std::collections::{BTreeMap, VecDeque};
use std::io::{Read, Write};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use humata_wire::pb;
use rustix::fs::{Mode, OFlags};
use rustix::pty::OpenptFlags;
use rustix::termios::Winsize;
use tokio::sync::broadcast;

/// How much of a terminal's latest output is kept, to replay on attach.
const KEPT_OUTPUT: usize = 256 * 1024;

/// How many chunks a watcher may fall behind before it's sent a replay.
const LIVE_CHUNKS: usize = 1024;

/// What a terminal can fail at.
#[derive(Debug, thiserror::Error)]
pub enum TerminalError {
    #[error(
        "no terminal {id} on this host; it was closed, or the host restarted since, which ends its terminals. Open a new one"
    )]
    Unknown { id: String },

    #[error(
        "couldn't open a pseudo-terminal for a shell; the host may have run out of them (check `ulimit -n` and the system's pty limit), or close some terminals and try again"
    )]
    Open {
        #[source]
        source: std::io::Error,
    },

    #[error(
        "couldn't start the shell {shell} in {folder}; check that the folder exists and that SHELL names a program on the host"
    )]
    Start {
        shell: PathBuf,
        folder: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("couldn't type into terminal {id}; its shell may have exited. Open a new one")]
    Write {
        id: String,
        #[source]
        source: std::io::Error,
    },

    #[error("couldn't resize terminal {id}; its shell may have exited. Open a new one")]
    Resize {
        id: String,
        #[source]
        source: std::io::Error,
    },
}

impl TerminalError {
    pub fn code(&self) -> pb::ErrorCode {
        match self {
            Self::Unknown { .. } => pb::ErrorCode::UnknownTerminal,
            Self::Open { .. } | Self::Start { .. } | Self::Write { .. } | Self::Resize { .. } => {
                pb::ErrorCode::BackendFailed
            }
        }
    }
}

/// What a terminal printed, or that its shell ended.
#[derive(Clone, Debug)]
pub enum Chunk {
    Output(Vec<u8>),
    Exited(i32),
}

/// A terminal's recent output and what follows it.
pub struct Attached {
    pub replay: Vec<u8>,
    /// Set when the shell has already exited.
    pub exited: Option<i32>,
    pub live: broadcast::Receiver<Chunk>,
    terminal: Arc<Terminal>,
}

impl Attached {
    /// The output from the start again, after falling behind.
    pub fn again(&mut self) -> (Vec<u8>, Option<i32>) {
        let (replay, exited, live) = self.terminal.watch();
        self.live = live;
        (replay, exited)
    }
}

struct Output {
    kept: VecDeque<u8>,
    exited: Option<i32>,
}

struct Terminal {
    info: pb::Terminal,
    /// The pseudo-terminal's controlling side: what's typed goes in, what
    /// the shell prints comes out.
    master: OwnedFd,
    writer: Mutex<std::fs::File>,
    output: Mutex<Output>,
    live: broadcast::Sender<Chunk>,
    /// The shell's process group, which it leads in a session of its own.
    group: Option<rustix::process::Pid>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Terminal {
    fn watch(&self) -> (Vec<u8>, Option<i32>, broadcast::Receiver<Chunk>) {
        // Held while subscribing, so nothing printed falls between the two.
        let output = lock(&self.output);
        let live = self.live.subscribe();
        (output.kept.iter().copied().collect(), output.exited, live)
    }

    fn printed(&self, bytes: &[u8]) {
        let mut output = lock(&self.output);
        output.kept.extend(bytes);
        let over = output.kept.len().saturating_sub(KEPT_OUTPUT);
        output.kept.drain(..over);
        let _ = self.live.send(Chunk::Output(bytes.to_vec()));
    }

    fn exited(&self, code: i32) {
        let mut output = lock(&self.output);
        output.exited = Some(code);
        let _ = self.live.send(Chunk::Exited(code));
    }

    fn info(&self) -> pb::Terminal {
        let mut info = self.info.clone();
        if let Some(code) = lock(&self.output).exited {
            info.exited = true;
            info.exit_code = code;
        }
        info
    }
}

/// The terminals open on this host.
#[derive(Default)]
pub struct Terminals {
    open: Mutex<BTreeMap<String, Arc<Terminal>>>,
}

/// Where and how big a new terminal is.
pub struct Opening<'a> {
    pub session_id: String,
    pub folder: &'a Path,
    pub rows: u16,
    pub columns: u16,
    pub env: Vec<(String, String)>,
}

impl Terminals {
    /// Starts the login shell in `opening.folder` on a new pseudo-terminal.
    pub fn open(&self, opening: Opening<'_>) -> Result<pb::Terminal, TerminalError> {
        let opened = |source| TerminalError::Open { source };
        let master = rustix::pty::openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY)
            .map_err(|error| opened(error.into()))?;
        rustix::io::fcntl_setfd(&master, rustix::io::FdFlags::CLOEXEC)
            .map_err(|error| opened(error.into()))?;
        rustix::pty::grantpt(&master).map_err(|error| opened(error.into()))?;
        rustix::pty::unlockpt(&master).map_err(|error| opened(error.into()))?;
        let name =
            rustix::pty::ptsname(&master, Vec::new()).map_err(|error| opened(error.into()))?;
        // Not this process's controlling terminal: only the shell's.
        let pty = rustix::fs::open(
            name.as_c_str(),
            OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|error| opened(error.into()))?;
        resize(&master, opening.rows, opening.columns).map_err(opened)?;

        let shell = std::env::var_os("SHELL")
            .map(PathBuf::from)
            .filter(|shell| shell.is_absolute())
            .unwrap_or_else(|| PathBuf::from("/bin/sh"));
        let started = |source| TerminalError::Start {
            shell: shell.clone(),
            folder: opening.folder.to_owned(),
            source,
        };
        let mut command = tokio::process::Command::new(&shell);
        command
            .arg("-l")
            .current_dir(opening.folder)
            .envs(opening.env)
            .env("TERM", "xterm-256color")
            .env("COLORTERM", "truecolor");
        if std::env::var_os("LANG").is_none() {
            command.env("LANG", "en_US.UTF-8");
        }
        for stdio in 0..3 {
            let copy = pty.try_clone().map_err(started)?;
            match stdio {
                0 => command.stdin(copy),
                1 => command.stdout(copy),
                _ => command.stderr(copy),
            };
        }
        // SAFETY: between fork and exec this only makes system calls that
        // are safe there: it starts a session, which leaves the host's, and
        // takes the pseudo-terminal, already the shell's stdin, as its
        // controlling terminal so job control and ^C work.
        unsafe {
            command.pre_exec(|| {
                rustix::process::setsid()?;
                rustix::process::ioctl_tiocsctty(BorrowedFd::borrow_raw(0))?;
                Ok(())
            });
        }
        let child = command.spawn().map_err(started)?;
        let group = child
            .id()
            .and_then(|id| i32::try_from(id).ok())
            .and_then(rustix::process::Pid::from_raw);
        // The shell holds the only copies now, so reading ends when it and
        // what it started close the terminal.
        drop(command);
        drop(pty);

        let reader = master.try_clone().map_err(opened)?;
        let writer = master.try_clone().map_err(opened)?;
        let info = pb::Terminal {
            id: uuid::Uuid::new_v4().to_string(),
            host: String::new(),
            session_id: opening.session_id,
            folder: opening.folder.display().to_string(),
            shell: shell.display().to_string(),
            opened_unix_ms: now_unix_ms(),
            exited: false,
            exit_code: 0,
        };
        let terminal = Arc::new(Terminal {
            info: info.clone(),
            master,
            writer: Mutex::new(std::fs::File::from(writer)),
            output: Mutex::new(Output {
                kept: VecDeque::new(),
                exited: None,
            }),
            live: broadcast::channel(LIVE_CHUNKS).0,
            group,
        });
        lock(&self.open).insert(info.id.clone(), terminal.clone());
        spawn_reader(&terminal, std::fs::File::from(reader), child);
        Ok(info)
    }

    fn get(&self, id: &str) -> Result<Arc<Terminal>, TerminalError> {
        lock(&self.open)
            .get(id)
            .cloned()
            .ok_or_else(|| TerminalError::Unknown { id: id.to_owned() })
    }

    pub fn attach(&self, id: &str) -> Result<(pb::Terminal, Attached), TerminalError> {
        let terminal = self.get(id)?;
        let (replay, exited, live) = terminal.watch();
        Ok((
            terminal.info(),
            Attached {
                replay,
                exited,
                live,
                terminal,
            },
        ))
    }

    /// Types `data`, off the async threads: a shell that isn't reading
    /// leaves the write waiting.
    pub async fn input(&self, id: &str, data: Vec<u8>) -> Result<(), TerminalError> {
        let terminal = self.get(id)?;
        let failed = |source| TerminalError::Write {
            id: id.to_owned(),
            source,
        };
        tokio::task::spawn_blocking(move || lock(&terminal.writer).write_all(&data))
            .await
            .map_err(|joined| failed(std::io::Error::other(joined)))?
            .map_err(failed)
    }

    pub fn resize(&self, id: &str, rows: u16, columns: u16) -> Result<(), TerminalError> {
        let terminal = self.get(id)?;
        resize(&terminal.master, rows, columns).map_err(|source| TerminalError::Resize {
            id: id.to_owned(),
            source,
        })
    }

    pub fn list(&self) -> Vec<pb::Terminal> {
        let mut all: Vec<pb::Terminal> = lock(&self.open)
            .values()
            .map(|terminal| terminal.info())
            .collect();
        all.sort_by_key(|terminal| terminal.opened_unix_ms);
        all
    }

    /// Ends the shell, if it still runs, and forgets the terminal.
    pub fn close(&self, id: &str) -> Result<(), TerminalError> {
        let terminal = lock(&self.open)
            .remove(id)
            .ok_or_else(|| TerminalError::Unknown { id: id.to_owned() })?;
        // Hangs up on the shell's session, as closing a terminal window
        // does, so what it started ends with it. Already gone is fine.
        if lock(&terminal.output).exited.is_none()
            && let Some(group) = terminal.group
        {
            let _ = rustix::process::kill_process_group(group, rustix::process::Signal::HUP);
        }
        Ok(())
    }
}

fn resize(master: &impl AsFd, rows: u16, columns: u16) -> Result<(), std::io::Error> {
    let size = Winsize {
        ws_row: rows.max(1),
        ws_col: columns.max(1),
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    rustix::termios::tcsetwinsize(master, size).map_err(Into::into)
}

/// Reads what the shell prints until it's done, then waits for its exit.
fn spawn_reader(
    terminal: &Arc<Terminal>,
    mut reader: std::fs::File,
    mut shell: tokio::process::Child,
) {
    let printing = terminal.clone();
    let done = tokio::task::spawn_blocking(move || {
        let mut buffer = vec![0; 16 * 1024];
        // Ends at EOF, or EIO on macOS and Linux once the shell's side
        // has closed.
        while let Ok(read) = reader.read(&mut buffer) {
            if read == 0 {
                break;
            }
            printing.printed(&buffer[..read]);
        }
    });
    let waiting = terminal.clone();
    tokio::spawn(async move {
        let code = shell
            .wait()
            .await
            .ok()
            .and_then(|status| status.code())
            .unwrap_or(-1);
        // Whatever it printed last comes before the exit.
        let _ = tokio::time::timeout(std::time::Duration::from_secs(1), done).await;
        waiting.exited(code);
    });
}

fn now_unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
        })
}
