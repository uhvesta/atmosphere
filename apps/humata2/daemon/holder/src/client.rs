//! Starting a holder and attaching to one.

use std::ffi::{OsStr, OsString};
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
use crate::protocol::{
    CHUNK, ClientFrame, FrameError, MAX_KEY, MAX_KEYED, ServerFrame, VERSION, Welcome, read_frame,
};
use crate::server::check_socket_path;
use crate::shown::Shown;

/// The holder's first line, once the OS has it running: before that, the
/// wait is the OS's (exec, code-signature checks of a new binary), which a
/// loaded machine can stretch far past our own start-up.
const STARTED: &str = "started";
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(30);
/// Its second, once it listens: this part is ours.
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
pub fn spawn(spec: &HolderSpec) -> Result<Spawned, HolderError> {spawn_with_persistence(spec,false,None)}

/// Starts a holder with memory replay only. The explicit flag makes older
/// binaries fail closed instead of silently writing a raw output transcript.
pub fn spawn_memory_only(spec:&HolderSpec)->Result<Spawned,HolderError>{spawn_with_persistence(spec,true,None)}

/// Starts a holder as [`spawn`] does that shows `banner` before the
/// program's first output. A holder from an older build refuses the
/// option and doesn't start.
pub fn spawn_with_banner(spec: &HolderSpec, banner: &str) -> Result<Spawned, HolderError> {
    spawn_with_persistence(spec, false, Some(banner))
}

fn spawn_with_persistence(
    spec: &HolderSpec,
    memory_only: bool,
    banner: Option<&str>,
) -> Result<Spawned, HolderError> {
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
        .args(if memory_only {&["--memory-only"][..]}else{&[][..]})
        .args(
            banner
                .map(|banner| [OsStr::new("--banner"), OsStr::new(banner)])
                .into_iter()
                .flatten(),
        )
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
        let mut lines = std::io::BufReader::new(stdout).lines();
        for wanted in [STARTED, READY] {
            let heard = matches!(lines.next(), Some(Ok(line)) if line == wanted);
            if said.send(heard).is_err() || !heard {
                return;
            }
        }
    });
    for (timeout, timed_out) in [
        (
            LAUNCH_TIMEOUT,
            HolderError::LaunchTimeout {
                holder: spec.holder.clone(),
                log: spec.log.clone(),
                seconds: LAUNCH_TIMEOUT.as_secs(),
            },
        ),
        (
            READY_TIMEOUT,
            HolderError::StartTimeout {
                holder: spec.holder.clone(),
                log: spec.log.clone(),
                seconds: READY_TIMEOUT.as_secs(),
            },
        ),
    ] {
        let failed = match heard.recv_timeout(timeout) {
            Ok(true) => continue,
            Ok(false) | Err(RecvTimeoutError::Disconnected) => start_failed(),
            Err(RecvTimeoutError::Timeout) => timed_out,
        };
        let _ = child.kill();
        let _ = child.wait();
        return Err(failed);
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

/// How long a shutdown waits for the holder at its socket to answer, from
/// connecting to its welcome. A holder that's stopped or wedged doesn't
/// answer, and a shutdown must not wait on it for ever.
const SHUTDOWN_WAIT: Duration = Duration::from_secs(3);

/// Asks the holder at `socket` to end its program and stop. Gives up after
/// `SHUTDOWN_WAIT` with `HolderError::ShutdownTimeout`, and leaves the
/// holder running: its caller ends it by its pid.
pub async fn shutdown(socket: &Path) -> Result<(), HolderError> {
    let ask = async {
        let mut attachment = attach(socket, u64::MAX).await?;
        attachment.control.shutdown().await
    };
    tokio::time::timeout(SHUTDOWN_WAIT, ask)
        .await
        .map_err(|source| HolderError::ShutdownTimeout {
            socket: socket.to_owned(),
            seconds: SHUTDOWN_WAIT.as_secs(),
            source,
        })?
}

/// What typing a keyed input did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyedInput {
    /// All of it was typed just now.
    Written,
    /// Input with this key was typed before; nothing was typed now.
    Repeat,
}

/// Whether a keyed input was typed, as the holder remembers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyState {
    /// It was typed, in full, by the holder process `holder`.
    Arrived { holder: u64 },
    /// The holder process `holder` did not type it. That is certain only
    /// if `holder` is the process that held the terminal when the key was
    /// first sent: a holder started again for the same tab has a new
    /// terminal and an empty memory, and has a different `holder`. It
    /// remembers the last 256 keys: ask about recent ones.
    NotArrived { holder: u64 },
}

/// Types `data` into the holder at `socket` once for `key`, on a
/// connection of its own: the holder holds all of `data` until it is
/// whole and types it in one go, then remembers `key`. Sending a key it
/// has already typed types nothing and answers `Repeat`. If the
/// connection breaks, nothing may have been typed or all of it may: ask
/// `key_arrived`. A holder from a build before keyed input is
/// [`HolderError::Outdated`]: nothing was typed, it keeps running, and its
/// session needs migrating before it takes typing.
pub async fn keyed_input(
    socket: &Path,
    key: &str,
    data: &[u8],
) -> Result<KeyedInput, HolderError> {
    check_key(key)?;
    if data.len() > MAX_KEYED {
        return Err(HolderError::KeyedInputTooLarge {
            len: data.len(),
            max: MAX_KEYED,
        });
    }
    let Attachment {
        mut events,
        mut control,
        ..
    } = attach(socket, u64::MAX).await?;
    // Asking first tells a holder that can't do this, which closes the
    // connection on a frame it doesn't know, from one that can.
    match ask(&mut events, &mut control, key).await? {
        None => {
            older_holder(socket).await?;
            return Err(HolderError::Outdated {
                socket: socket.to_owned(),
            });
        }
        Some(Answer { arrived: true, .. }) => return Ok(KeyedInput::Repeat),
        Some(Answer { arrived: false, .. }) => {}
    }
    let mut chunks = data.chunks(CHUNK).peekable();
    if chunks.peek().is_none() {
        control.keyed_chunk(key, true, &[]).await?;
    }
    while let Some(chunk) = chunks.next() {
        control.keyed_chunk(key, chunks.peek().is_none(), chunk).await?;
    }
    match events.reply().await? {
        Some(ServerFrame::KeyedWritten { repeat }) => Ok(if repeat {
            KeyedInput::Repeat
        } else {
            KeyedInput::Written
        }),
        Some(ServerFrame::Refused { message }) => Err(HolderError::Refused {
            socket: socket.to_owned(),
            message,
        }),
        Some(_) => Err(events.protocol("it answered keyed input with something else")),
        None => Err(events.lost(std::io::ErrorKind::UnexpectedEof.into())),
    }
}

/// Asks the holder at `socket` whether the keyed input `key` was typed. A
/// holder from a build before keyed input is [`HolderError::Outdated`].
pub async fn key_arrived(socket: &Path, key: &str) -> Result<KeyState, HolderError> {
    check_key(key)?;
    let mut attachment = attach(socket, u64::MAX).await?;
    Ok(match ask(&mut attachment.events, &mut attachment.control, key).await? {
        Some(Answer {
            arrived: true,
            instance,
        }) => KeyState::Arrived { holder: instance },
        Some(Answer {
            arrived: false,
            instance,
        }) => KeyState::NotArrived { holder: instance },
        None => {
            older_holder(socket).await?;
            return Err(HolderError::Outdated {
                socket: socket.to_owned(),
            });
        }
    })
}

/// How long after it started `type_then_return` waits for the program to
/// read keys itself: an agent's CLI starting cold on a loaded machine
/// takes seconds. One that keeps line editing on (a shell's `read`) holds
/// up only text typed while it's starting.
const KEYS_LIMIT: Duration = Duration::from_secs(5);
/// How long `type_then_return` waits for the program to show the text
/// before it holds the return back.
const SHOWN_LIMIT: Duration = Duration::from_secs(15);

/// Whether the program in a holder reads keys itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadsKeys {
    Yes,
    /// The terminal still edits lines itself: the program had run for the
    /// time given, or ended, before it turned that off.
    NotYet,
}

/// Waits until the program in the holder at `socket` reads keys itself:
/// it has turned the terminal's line editing off, as a full-screen program
/// does once it's up. Text typed before then is echoed and held by the
/// terminal, not by the program. A program still editing lines once it
/// has run for `starting` is taken to keep doing so: one that has run that
/// long gets no wait at all.
pub async fn await_keys(socket: &Path, starting: Duration) -> Result<ReadsKeys, HolderError> {
    let starting_ms = u32::try_from(starting.as_millis()).unwrap_or(u32::MAX);
    let Attachment {
        mut events,
        mut control,
        ..
    } = attach(socket, u64::MAX).await?;
    let asked = control.send(ClientFrame::AwaitKeys { starting_ms }).await;
    let answer = match asked {
        Ok(()) => events.reply().await,
        Err(_) => Ok(None),
    };
    match answer {
        Ok(Some(ServerFrame::KeysRead { ready: true })) => Ok(ReadsKeys::Yes),
        Ok(Some(ServerFrame::KeysRead { ready: false })) => Ok(ReadsKeys::NotYet),
        Ok(Some(ServerFrame::Refused { message })) => Err(HolderError::Refused {
            socket: socket.to_owned(),
            message,
        }),
        Ok(Some(_)) => Err(events.protocol("it answered a question with something else")),
        // An older holder closes the connection on a frame it doesn't know.
        // Nothing is typed into it: the upgrade's "Migrate the holders"
        // item moves its session onto the current holder (D373).
        Ok(None) | Err(HolderError::Lost { .. }) => {
            older_holder(socket).await?;
            Err(HolderError::Outdated {
                socket: socket.to_owned(),
            })
        }
        Err(error) => Err(error),
    }
}

/// The output a holder prints from when this was made, to tell when the
/// program shows text typed after it.
pub struct OutputWatch {
    events: Events,
    /// Kept open: the holder stops sending output once a client's side
    /// closes.
    control: Control,
    /// A wait was cut off part way through a frame: what follows can't be
    /// read.
    torn: bool,
}

/// Starts watching the output of the holder at `socket` from now on.
pub async fn watch_output(socket: &Path) -> Result<OutputWatch, HolderError> {
    let Attachment {
        events, control, ..
    } = attach(socket, u64::MAX).await?;
    Ok(OutputWatch {
        events,
        control,
        torn: false,
    })
}

impl OutputWatch {
    /// Waits, for up to `limit`, until the output since the watch began
    /// shows the end of `text` as a person would read it on the screen.
    /// Returns whether it did: `false` when the limit passed or the
    /// program ended first. Text with nothing visible counts as shown.
    pub async fn shown(&mut self, text: &[u8], limit: Duration) -> Result<bool, HolderError> {
        let Some(mut seen) = Shown::new(text) else {
            return Ok(true);
        };
        if self.torn {
            return Ok(false);
        }
        match tokio::time::timeout(limit, look_for(&mut self.events, &mut seen)).await {
            Ok(shown) => shown,
            Err(_) => {
                self.torn = true;
                Ok(false)
            }
        }
    }
}

/// Reads output until `seen` sees its text (`true`) or the program ends.
async fn look_for(events: &mut Events, seen: &mut Shown) -> Result<bool, HolderError> {
    loop {
        match events.next().await? {
            Some(Event::Output { data, .. }) => {
                if seen.push(&data) {
                    return Ok(true);
                }
            }
            Some(Event::Exited { .. }) | None => return Ok(false),
            Some(_) => {}
        }
    }
}

/// What became of the return [`type_then_return`] was to press.
#[derive(Debug)]
pub enum Typed {
    /// The program showed the text, and return was pressed after it.
    Submitted,
    /// The program didn't show the text in time, as one that first asks a
    /// question of its own (Copilot's folder trust) doesn't: return wasn't
    /// pressed, since it would answer that question for the person
    /// (BUG-T1b). [`HeldReturn::once_shown`] presses it once the program
    /// shows the text.
    Held(HeldReturn),
}

/// A return held back until the program shows the text typed before it.
pub struct HeldReturn {
    /// Reads the program's output until it shows the text (`true`) or
    /// ends; `None` once the program has ended.
    shown: Option<tokio::task::JoinHandle<Result<bool, HolderError>>>,
    control: Control,
    pause: Duration,
}

impl HeldReturn {
    /// Waits up to `limit` for the program to show the text, as it does
    /// once the person has answered its question, then presses return after
    /// the pause. Returns whether it pressed it: not when the program ended
    /// or the limit passed first.
    pub async fn once_shown(&mut self, limit: Duration) -> Result<bool, HolderError> {
        let Some(shown) = self.shown.as_mut() else {
            return Ok(false);
        };
        let Ok(looked) = tokio::time::timeout(limit, shown).await else {
            return Ok(false);
        };
        // Done looking either way: a finished task isn't polled again.
        self.shown = None;
        match looked {
            Ok(Ok(true)) => {}
            Ok(Err(error)) => return Err(error),
            Ok(Ok(false)) | Err(_) => return Ok(false),
        }
        tokio::time::sleep(self.pause).await;
        self.control.input(2, b"\r").await?;
        Ok(true)
    }

    /// Whether waiting is over: the text was shown and its return pressed,
    /// or the program ended.
    pub fn done(&self) -> bool {
        self.shown.is_none()
    }
}

/// Whether a program's input is clear to type into now, as the daemon's
/// screen check says; `false` while it asks a question of its own or its
/// screen can't be read.
pub type Ready = std::sync::Arc<
    dyn Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send>> + Send + Sync,
>;

/// How often [`deliver`] asks whether the input is clear again.
const READY_EVERY: Duration = Duration::from_millis(500);

/// Sees `text` submitted into the holder at `socket` within `limit`, after
/// [`type_then_return`] held its return (`held`) or never typed it (`None`,
/// as when the input wasn't clear). With `ready`, it types only once the
/// input is clear, and types the text again, once, when the input is clear
/// but the text still isn't shown after a bounded wait: the program dropped
/// what was typed while it asked. Without `ready` nothing tells a dropped
/// text from one still held behind a question, so it only presses return
/// once the text shows. Returns whether the text was submitted.
pub async fn deliver(
    socket: &Path,
    text: &[u8],
    pause: Duration,
    mut held: Option<HeldReturn>,
    ready: Option<Ready>,
    limit: Duration,
) -> Result<bool, HolderError> {
    let deadline = tokio::time::Instant::now() + limit;
    let left = || deadline.saturating_duration_since(tokio::time::Instant::now());
    let mut typings = u8::from(held.is_some());
    loop {
        let Some(waiting) = held.as_mut() else {
            if let Some(ready) = &ready {
                while !ready().await {
                    if left().is_zero() {
                        return Ok(false);
                    }
                    tokio::time::sleep(READY_EVERY.min(left())).await;
                }
            }
            typings += 1;
            match type_then_return(socket, text, pause).await? {
                Typed::Submitted => return Ok(true),
                Typed::Held(next) => held = Some(next),
            }
            continue;
        };
        let Some(ready) = &ready else {
            return waiting.once_shown(left()).await;
        };
        // Watch for the text while the program asks.
        loop {
            if waiting.once_shown(READY_EVERY.min(left())).await? {
                return Ok(true);
            }
            if waiting.done() || left().is_zero() {
                return Ok(false);
            }
            if ready().await {
                break;
            }
        }
        // The input is clear: the text shows soon if the program kept it.
        if waiting.once_shown(SHOWN_LIMIT.min(left())).await? {
            return Ok(true);
        }
        if waiting.done() || left().is_zero() {
            return Ok(false);
        }
        if typings >= 2 {
            return waiting.once_shown(left()).await;
        }
        held = None;
    }
}

impl std::fmt::Debug for HeldReturn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HeldReturn")
            .field("socket", &self.control.socket)
            .field("pause", &self.pause)
            .finish_non_exhaustive()
    }
}

impl Drop for HeldReturn {
    fn drop(&mut self) {
        if let Some(shown) = &self.shown {
            shown.abort();
        }
    }
}

/// Types `text` into the holder at `socket`, then presses return as a key
/// of its own, the way a person does: once the program reads keys itself,
/// has shown the text (or the summary it shows a long paste as), and
/// `pause` has passed after that. An agent's CLI takes a return that
/// reaches it with the text, as one starting up and reading both at once
/// does, as part of a paste and doesn't submit it. A program that hasn't
/// shown the text after a bounded wait gets no return: it may be asking
/// something of its own, which a return would answer (BUG-T1b).
pub async fn type_then_return(
    socket: &Path,
    text: &[u8],
    pause: Duration,
) -> Result<Typed, HolderError> {
    await_keys(socket, KEYS_LIMIT).await?;
    let OutputWatch {
        mut events,
        mut control,
        ..
    } = watch_output(socket).await?;
    let looking = Shown::new(text);
    // Read in a task of its own, as reading isn't cancel-safe: a held
    // return goes on reading where this wait stopped.
    let mut shown = tokio::spawn(async move {
        match looking {
            Some(mut looking) => look_for(&mut events, &mut looking).await,
            None => Ok(true),
        }
    });
    control.input(1, text).await?;
    let waited = tokio::time::timeout(SHOWN_LIMIT, &mut shown).await;
    let shown = match waited {
        Ok(Ok(Ok(true))) => {
            tokio::time::sleep(pause).await;
            control.input(2, b"\r").await?;
            return Ok(Typed::Submitted);
        }
        Ok(Ok(Err(error))) => return Err(error),
        // The program ended: nothing will show the text.
        Ok(Ok(Ok(false)) | Err(_)) => None,
        Err(_) => Some(shown),
    };
    Ok(Typed::Held(HeldReturn {
        shown,
        control,
        pause,
    }))
}

/// A holder's answer about a key.
struct Answer {
    arrived: bool,
    instance: u64,
}

/// Asks whether `key` was typed. `None` if the holder closed the
/// connection instead of answering.
async fn ask(
    events: &mut Events,
    control: &mut Control,
    key: &str,
) -> Result<Option<Answer>, HolderError> {
    if control
        .send(ClientFrame::KeyQuery { key: key.to_owned() })
        .await
        .is_err()
    {
        return Ok(None);
    }
    match events.reply().await {
        Ok(Some(ServerFrame::KeyAnswer { arrived, instance })) => {
            Ok(Some(Answer { arrived, instance }))
        }
        Ok(Some(ServerFrame::Refused { message })) => Err(HolderError::Refused {
            socket: events.socket.clone(),
            message,
        }),
        Ok(Some(_)) => Err(events.protocol("it answered a question with something else")),
        Ok(None) | Err(HolderError::Lost { .. }) => Ok(None),
        Err(error) => Err(error),
    }
}

fn check_key(key: &str) -> Result<(), HolderError> {
    if key.len() > MAX_KEY {
        return Err(HolderError::KeyTooLong {
            key: key.to_owned(),
            len: key.len(),
            max: MAX_KEY,
        });
    }
    Ok(())
}

/// An asked-about connection was closed: an older holder does that on a
/// frame it doesn't know. Confirms the holder is still there, so a holder
/// that died isn't taken for an old one.
async fn older_holder(socket: &Path) -> Result<(), HolderError> {
    attach(socket, u64::MAX).await.map(|_| ())
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
            Some(
                ServerFrame::KeyedWritten { .. }
                | ServerFrame::KeyAnswer { .. }
                | ServerFrame::KeysRead { .. },
            ) => {
                return Err(self.protocol("it answered keyed input on the events connection"));
            }
        })
    }

    /// The next answer to keyed input, skipping terminal output, or `None`
    /// once the holder has closed the connection.
    async fn reply(&mut self) -> Result<Option<ServerFrame>, HolderError> {
        loop {
            match self.frame().await? {
                Some(
                    ServerFrame::Output { .. }
                    | ServerFrame::Trimmed { .. }
                    | ServerFrame::Exited { .. }
                    | ServerFrame::InputWritten { .. },
                ) => {}
                other => return Ok(other),
            }
        }
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

    async fn keyed_chunk(&mut self, key: &str, last: bool, data: &[u8]) -> Result<(), HolderError> {
        self.send(ClientFrame::KeyedInput {
            key: key.to_owned(),
            last,
            data: data.to_vec(),
        })
        .await
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
