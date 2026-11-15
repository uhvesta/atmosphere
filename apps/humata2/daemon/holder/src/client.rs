//! Starting a holder and attaching to one.

use std::ffi::OsString;
use std::fs::OpenOptions;
use std::io::BufRead;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

use crate::HolderError;
use crate::protocol::{CHUNK, ClientFrame, FrameError, ServerFrame, VERSION, Welcome, read_frame};
use crate::server::check_socket_path;

const READY: &str = "ready";
const READY_TIMEOUT: Duration = Duration::from_secs(10);

/// A holder to start: the program it runs and where it listens.
#[derive(Clone, Debug)]
pub struct HolderSpec {
    /// The `humata2_holder` binary.
    pub holder: PathBuf,
    pub socket: PathBuf,
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub cwd: PathBuf,
    /// Set on the holder, which the program inherits. Kept off the command
    /// line, where other users can read it.
    pub env: Vec<(OsString, OsString)>,
    pub cols: u16,
    pub rows: u16,
    pub scrollback_bytes: usize,
    /// Where the holder writes what goes wrong.
    pub log: PathBuf,
}

/// A started holder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spawned {
    pub pid: u32,
}

/// Starts a holder in a session of its own, so it outlives the caller,
/// and returns once it's listening. Blocks for up to ten seconds.
pub fn spawn(spec: &HolderSpec) -> Result<Spawned, HolderError> {
    check_socket_path(&spec.socket)?;
    let launch_failed = |source| HolderError::Launch {
        holder: spec.holder.clone(),
        source,
    };
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&spec.log)
        .map_err(launch_failed)?;
    let mut child = Command::new(&spec.holder)
        .arg("--socket")
        .arg(&spec.socket)
        .arg("--cwd")
        .arg(&spec.cwd)
        .arg("--cols")
        .arg(spec.cols.to_string())
        .arg("--rows")
        .arg(spec.rows.to_string())
        .arg("--scrollback-bytes")
        .arg(spec.scrollback_bytes.to_string())
        .arg("--")
        .arg(&spec.program)
        .args(&spec.args)
        .envs(spec.env.iter().map(|(key, value)| (key, value)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(log)
        .spawn()
        .map_err(launch_failed)?;
    let pid = child.id();
    let start_failed = || HolderError::StartFailed {
        holder: spec.holder.clone(),
        log: spec.log.clone(),
    };
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(start_failed());
    };
    let (said, heard) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let read = std::io::BufReader::new(stdout).read_line(&mut line);
        let _ = said.send(read.is_ok() && line.trim_end() == READY);
    });
    match heard.recv_timeout(READY_TIMEOUT) {
        Ok(true) => {}
        Ok(false) | Err(RecvTimeoutError::Disconnected) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(start_failed());
        }
        Err(RecvTimeoutError::Timeout) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(HolderError::StartTimeout {
                holder: spec.holder.clone(),
                log: spec.log.clone(),
                seconds: READY_TIMEOUT.as_secs(),
            });
        }
    }
    // Reaps the holder when it ends, if the caller is still running.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(Spawned { pid })
}

/// What a holder sends after the welcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// Output that starts at `offset`. Each continues where the one before
    /// it ended, unless a `Trimmed` comes between.
    Output { offset: u64, data: Vec<u8> },
    /// What came before `resume_at` and after the client's cursor is no
    /// longer kept; output resumes there.
    Trimmed { resume_at: u64 },
    Exited { code: i32 },
    /// Every chunk of input `seq` reached the terminal.
    InputWritten { seq: u64 },
    /// A request was refused; the connection stays usable.
    Refused { message: String },
}

/// A connection to a holder.
pub struct Attachment {
    pub welcome: Welcome,
    pub events: Events,
    pub control: Control,
}

/// Connects to the holder at `socket` and asks for output from `from`:
/// what's kept from there, then live output.
pub async fn attach(socket: &Path, from: u64) -> Result<Attachment, HolderError> {
    let stream = UnixStream::connect(socket)
        .await
        .map_err(|source| HolderError::Connect {
            socket: socket.to_owned(),
            source,
        })?;
    let (read, write) = stream.into_split();
    let mut events = Events {
        socket: socket.to_owned(),
        read,
    };
    let mut control = Control {
        socket: socket.to_owned(),
        write,
    };
    control
        .send(ClientFrame::Hello {
            version: VERSION,
            from,
        })
        .await?;
    let welcome = match events.frame().await? {
        Some(ServerFrame::Welcome(welcome)) => welcome,
        Some(ServerFrame::Refused { message }) => {
            return Err(HolderError::Refused {
                socket: socket.to_owned(),
                message,
            });
        }
        Some(_) => return Err(events.protocol("it spoke before its welcome")),
        None => return Err(events.lost(std::io::ErrorKind::UnexpectedEof.into())),
    };
    Ok(Attachment {
        welcome,
        events,
        control,
    })
}

/// Asks the holder at `socket` to end its program and stop.
pub async fn shutdown(socket: &Path) -> Result<(), HolderError> {
    let mut attachment = attach(socket, u64::MAX).await?;
    attachment.control.shutdown().await
}

/// The holder's side of a connection.
pub struct Events {
    socket: PathBuf,
    read: OwnedReadHalf,
}

impl Events {
    /// The next event, or `None` once the holder has closed the
    /// connection. Not cancel-safe: dropping a pending call can lose part
    /// of a frame, so read from one task.
    pub async fn next(&mut self) -> Result<Option<Event>, HolderError> {
        Ok(match self.frame().await? {
            None => None,
            Some(ServerFrame::Output { offset, data }) => Some(Event::Output { offset, data }),
            Some(ServerFrame::Trimmed { resume_at }) => Some(Event::Trimmed { resume_at }),
            Some(ServerFrame::Exited { code }) => Some(Event::Exited { code }),
            Some(ServerFrame::InputWritten { seq }) => Some(Event::InputWritten { seq }),
            Some(ServerFrame::Refused { message }) => Some(Event::Refused { message }),
            Some(ServerFrame::Welcome(_)) => return Err(self.protocol("it welcomed twice")),
        })
    }

    async fn frame(&mut self) -> Result<Option<ServerFrame>, HolderError> {
        match read_frame(&mut self.read).await {
            Ok(None) => Ok(None),
            Ok(Some(frame)) => ServerFrame::decode(&frame)
                .map(Some)
                .map_err(|error| self.frame_error(error)),
            Err(error) => Err(self.frame_error(error)),
        }
    }

    fn frame_error(&self, error: FrameError) -> HolderError {
        match error {
            FrameError::Io(source) => self.lost(source),
            FrameError::Malformed(reason) => self.protocol(reason),
        }
    }

    fn lost(&self, source: std::io::Error) -> HolderError {
        HolderError::Lost {
            socket: self.socket.clone(),
            source,
        }
    }

    fn protocol(&self, reason: &'static str) -> HolderError {
        HolderError::Protocol {
            socket: self.socket.clone(),
            reason,
        }
    }
}

/// The client's side of a connection.
pub struct Control {
    socket: PathBuf,
    write: OwnedWriteHalf,
}

impl Control {
    /// Types `data`. The holder answers `InputWritten { seq }` once all of
    /// it reached the terminal.
    pub async fn input(&mut self, seq: u64, data: &[u8]) -> Result<(), HolderError> {
        let mut chunks = data.chunks(CHUNK).peekable();
        if chunks.peek().is_none() {
            return self
                .send(ClientFrame::Input {
                    seq,
                    last: true,
                    data: Vec::new(),
                })
                .await;
        }
        while let Some(chunk) = chunks.next() {
            let last = chunks.peek().is_none();
            self.send(ClientFrame::Input {
                seq,
                last,
                data: chunk.to_vec(),
            })
            .await?;
        }
        Ok(())
    }

    pub async fn resize(&mut self, cols: u16, rows: u16) -> Result<(), HolderError> {
        self.send(ClientFrame::Resize { cols, rows }).await
    }

    /// Ends the program and the holder.
    pub async fn shutdown(&mut self) -> Result<(), HolderError> {
        self.send(ClientFrame::Shutdown).await
    }

    async fn send(&mut self, frame: ClientFrame) -> Result<(), HolderError> {
        self.write
            .write_all(&frame.encode())
            .await
            .map_err(|source| HolderError::Lost {
                socket: self.socket.clone(),
                source,
            })
    }
}
