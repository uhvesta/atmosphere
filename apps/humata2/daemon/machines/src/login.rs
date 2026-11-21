//! Interactive login (MP-12 to MP-18): a machine whose connect command
//! needs a person runs in a real terminal the Add machine dialog draws
//! (MP-14); once they say they're in, [`crate::Takeover`] types into the
//! shell, installs what is missing and starts the peer link (MP-15), and
//! the machine becomes a normal machine whose reconnects need the user
//! (MP-16). Afterwards the user may make it unattended (MP-17).
//!
//! What lives here, none of it async (the daemon runs a session on a thread
//! of its own): prompt detection (MP-13), the pseudo-terminal a command
//! runs on, the session that drives a takeover through it, registering the
//! machine, and the unattended step. Output is read by a thread that feeds
//! one channel, so nothing polls.

use std::collections::BTreeMap;
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use rustix::fs::{Mode, OFlags};
use rustix::process::{Pid, Signal};
use rustix::pty::OpenptFlags;
use rustix::termios::Winsize;
use thiserror::Error;

use crate::enroll::{self, Key};
use crate::error::{end_said, output_block, QuoteError, RunError, TakeoverError, ValuesError};
use crate::hosts::{EntryState, HostEntry, HostsError, ProviderMachine, StoredLink};
use crate::identity::{SshAccess, SshTarget};
use crate::install::{self, Builds};
use crate::known_hosts;
use crate::link::Link;
use crate::manage::{ManageError, Machines, DEFAULT_ROOT};
use crate::name::{MachineName, BASELINE};
use crate::platform::Platform;
use crate::provider::Provider;
use crate::quote;
use crate::runner::{EnvValue, Exit, Invocation, Purpose, Runner};
use crate::system::HostVariables;
use crate::takeover::{Takeover, TakeoverAction};

/// How long a step of the enrollment typed over the login may take.
const ENROLL_TIMEOUT: Duration = Duration::from_secs(20);
/// How long the terminal may print nothing while a file is being installed
/// before the takeover is given up. The takeover itself sets no deadline
/// there (a file takes as long as the link is slow), but a terminal that
/// has gone silent isn't receiving one.
const INSTALL_STALL: Duration = Duration::from_secs(60);
/// The longest command typed straight into the shell. A terminal in line
/// mode drops what is typed past its input limit (1024 bytes on macOS)
/// while the shell isn't reading, which mangled the install scripts; a
/// longer command is handed over with the terminal raw instead
/// ([`LoginSession::type_command`]).
const TYPED_DIRECT_MAX: usize = 600;
/// How long output may keep arriving after the terminal's process ended or
/// its output closed (a descendant may hold the terminal open).
const DRAIN: Duration = Duration::from_millis(500);
/// How much recent output prompt detection and error tails keep.
const KEEP_BYTES: usize = 4096;
const TAIL_LINES: usize = 20;
/// How long a non-interactive connect gets when making a machine unattended.
const VERIFY_TIMEOUT: Duration = Duration::from_secs(30);
const KEYGEN_TIMEOUT: Duration = Duration::from_secs(60);

// ---------------------------------------------------------------------------
// Errors

/// Why a login, or making a machine unattended, failed. Every message says
/// what to do next.
#[derive(Debug, Error)]
pub enum LoginError {
    #[error(
        "couldn't open a terminal for the login: {source}. Close some terminals or other programs that hold many of them, then try again"
    )]
    OpenPty {
        #[source]
        source: io::Error,
    },
    #[error("couldn't start `{program}` for the login: {source}. Check that it is installed and on PATH, or fix the provider's `connect`")]
    Spawn {
        program: String,
        #[source]
        source: io::Error,
    },
    #[error(
        "the login command needs host variable {variable}, which isn't set; set it in Settings › This Mac › Variables, then log in again"
    )]
    HostVariable { variable: String },
    #[error("couldn't read host variable {variable} for the login command, so the login didn't start")]
    UnreadableHostVariable {
        variable: String,
        #[source]
        source: crate::error::HostVariablesError,
    },
    #[error("couldn't type into the login terminal: {source}. Close the login and open it again")]
    Write {
        #[source]
        source: io::Error,
    },
    #[error("couldn't change the login terminal's mode or size: {source}. Close the login and open it again")]
    Terminal {
        #[source]
        source: io::Error,
    },
    #[error("the login to {machine} was cancelled; nothing was changed")]
    Cancelled { machine: MachineName },
    #[error(
        "the login to {machine} ended{} before it finished; {}. Open it interactively again and finish the login, then press Done only when you see a shell prompt",
        .exit.map(|how| format!(" ({how})")).unwrap_or_default(),
        end_said(.output_tail)
    )]
    Ended {
        machine: MachineName,
        exit: Option<Exit>,
        output_tail: String,
    },
    #[error(transparent)]
    Takeover(#[from] TakeoverError),
    #[error(
        "couldn't write the enrollment key on {machine} over the login; {}. Check that its home folder is writable, then open the login again",
        end_said(.output_tail)
    )]
    Enrollment {
        machine: MachineName,
        output_tail: String,
    },
    #[error(
        "the terminal to {machine} printed nothing for {} seconds while Humata was installing its tools there; {}. Check that the shell you logged in to is a POSIX shell that reads what is typed, then open the login again",
        .waited.as_secs(),
        end_said(.output_tail)
    )]
    Stalled {
        machine: MachineName,
        waited: Duration,
        output_tail: String,
    },
    #[error("couldn't save the enrollment key {path}: {source}. Free some disk space or fix the folder's permissions, then log in again")]
    SaveEnrollment {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(transparent)]
    Quote(#[from] QuoteError),
    #[error(transparent)]
    Hosts(#[from] HostsError),
    #[error(transparent)]
    Values(#[from] ValuesError),
    #[error(
        "the {provider} provider reaches machines through an `identity` command, not a `connect` command, so there is no login to open. Fix the identity command's SSH details in Settings › Machines › Providers, or give the provider a `connect` command"
    )]
    IdentityProvider { provider: String },
    #[error("{machine} can't be changed right now: {source}")]
    Manage {
        machine: String,
        #[source]
        source: ManageError,
    },
    #[error(transparent)]
    Run(#[from] RunError),
    #[error("{machine} has no provider record to register; start the login from Create machine, or log in to a machine that is already listed")]
    NotRegistrable { machine: String },
    #[error("{machine} doesn't need a login: it isn't marked interactive, so there is nothing to make unattended")]
    NotInteractive { machine: String },
    #[error("{machine} has no way to be reached recorded, so Humata can't test a connect. Create it again")]
    NoLink { machine: String },
    #[error(
        "Humata can't tell which SSH address {machine} has: its connect command is a provider tool. Give the provider an `identity` command that prints SSH details (with the host key), or add `host` and `user` params to the provider, then make it unattended again; or choose to keep the provider's cached token"
    )]
    NoSshAddress { machine: String },
    #[error(
        "{machine}'s SSH host key isn't pinned in Humata's own known_hosts, and a key added to its authorized_keys can't be trusted without it. Give the provider an `identity` command with `ssh.host_key`, or keep the provider's cached token instead"
    )]
    HostKeyNotPinned { machine: String },
    #[error("couldn't create a key pair for {machine}: `ssh-keygen` ended with {exit}. Check that ssh-keygen is installed; {}", output_block(.output_tail))]
    KeyGen {
        machine: String,
        exit: Exit,
        output_tail: String,
    },
    #[error("couldn't use the key pair {path}: {source}. Check the folder is yours and has space, then try again")]
    KeyFile {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(
        "couldn't add Humata's key to {machine}'s authorized_keys ({exit}). The machine must be reachable without a person now: log in again if the connect asks for one, then make it unattended. {}",
        output_block(.output_tail)
    )]
    Authorize {
        machine: String,
        exit: Exit,
        output_tail: String,
    },
    #[error(
        "{machine} still can't be reached without a person ({exit}), so it was left as it was. If Humata added a key to the machine's authorized_keys and couldn't remove it again, delete the line ending `humata2:{machine}` there yourself. {}",
        output_block(.output_tail)
    )]
    StillNeedsLogin {
        machine: String,
        exit: Exit,
        output_tail: String,
        undone: bool,
    },
}

// ---------------------------------------------------------------------------
// Prompt detection (MP-13)

/// Words a password-like prompt names.
const SECRET_WORDS: [&str; 10] = [
    "password",
    "passphrase",
    "passcode",
    "verification code",
    "authentication code",
    "security code",
    "login code",
    "one-time",
    "one time",
    "token code",
];

/// What a prompt asks for (MP-13, D167): the rule of [`detect_prompt`] that
/// matched. The word "password" is a password, "passphrase" a passphrase,
/// every other secret word (code, one-time, otp, 2fa, passcode) a code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    Password,
    Passphrase,
    Code,
    /// A `(yes/no)?` question.
    Confirm,
    /// A device login: a URL and a code to enter there.
    Device,
}

/// A prompt a command waits at: what kind, and the line that says so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    pub kind: PromptKind,
    pub line: String,
}

/// Whether `output`, what a command printed so far, ends waiting for a
/// person (MP-13): a password, passphrase or verification-code prompt, a
/// `(yes/no)?` question, or a device-login line (a URL and a code to
/// enter). Returns the line that says so and the kind of rule that matched
/// (D167). Escape sequences are ignored and case doesn't matter.
/// Conservative: a line that only mentions a password in a sentence, with
/// output after it, is not a prompt.
pub fn detect_prompt(output: &[u8]) -> Option<Prompt> {
    let text = strip(output);
    // The line still being written, if the output doesn't end in a
    // newline: a prompt waits there.
    if !text.ends_with('\n') {
        let last = text.rsplit('\n').next().unwrap_or("").trim();
        if !last.is_empty()
            && let Some(kind) = line_kind(last)
        {
            return Some(Prompt { kind, line: last.to_owned() });
        }
    }
    // A device login prints its instructions on whole lines and waits.
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|line| !line.is_empty()).collect();
    let window = &lines[lines.len().saturating_sub(12)..];
    let joined = window.join("\n").to_lowercase();
    let has_url = joined.contains("https://") || joined.contains("http://");
    let asks_for_code = (joined.contains("enter") && joined.contains("code"))
        || joined.contains("device code")
        || joined.contains("user code")
        || joined.contains("device login");
    if has_url && asks_for_code {
        return window
            .iter()
            .find(|line| {
                let lower = line.to_lowercase();
                lower.contains("code") || lower.contains("device")
            })
            .or_else(|| window.last())
            .map(|line| Prompt { kind: PromptKind::Device, line: (*line).to_owned() });
    }
    None
}

/// What the last line of output waits for, when it is a prompt.
fn line_kind(line: &str) -> Option<PromptKind> {
    let lower = line.to_lowercase();
    let lower = lower.trim_end();
    if lower.contains("(yes/no") && lower.ends_with('?') {
        return Some(PromptKind::Confirm);
    }
    if lower.ends_with(':') || lower.ends_with("]:") || lower.ends_with("):") {
        // Short names must be whole words: "footprint:" has an "otp".
        let whole = |word: &str| lower.split(|c: char| !c.is_alphanumeric()).any(|part| part == word);
        if SECRET_WORDS.iter().any(|word| lower.contains(word)) || whole("otp") || whole("2fa") {
            return Some(secret_kind(lower));
        }
    }
    None
}

/// The kind of a prompt line that is already known to be a prompt: the
/// non-interactive connect uses it for a line it found by looser words
/// (`setup::LOGIN_PROMPTS`), where [`detect_prompt`] saw none (D167).
pub(crate) fn kind_of_line(line: &str) -> PromptKind {
    let lower = line.to_lowercase();
    if lower.contains("(yes/no") {
        PromptKind::Confirm
    } else if ["device login", "enter the code", "enter code"].iter().any(|word| lower.contains(word)) {
        PromptKind::Device
    } else {
        secret_kind(&lower)
    }
}

/// A password is a password, a passphrase a passphrase, any other secret
/// word (code, one-time, otp, 2fa, passcode) a code.
fn secret_kind(lower: &str) -> PromptKind {
    if lower.contains("password") {
        PromptKind::Password
    } else if lower.contains("passphrase") {
        PromptKind::Passphrase
    } else {
        PromptKind::Code
    }
}

/// `bytes` as text without escape sequences, carriage returns and other
/// control characters (a newline and tab stay).
fn strip(bytes: &[u8]) -> String {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            0x1b => {
                i += 1;
                match bytes.get(i) {
                    Some(b'[') => {
                        i += 1;
                        while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                            i += 1;
                        }
                        i += 1;
                    }
                    Some(b']') => {
                        i += 1;
                        while i < bytes.len() {
                            if bytes[i] == 0x07 {
                                i += 1;
                                break;
                            }
                            if bytes[i] == 0x1b && bytes.get(i + 1) == Some(&b'\\') {
                                i += 2;
                                break;
                            }
                            i += 1;
                        }
                    }
                    Some(_) => i += 1,
                    None => {}
                }
            }
            b'\n' | b'\t' => {
                out.push(bytes[i]);
                i += 1;
            }
            b if b < 0x20 || b == 0x7f => i += 1,
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The last lines of `bytes` as text, for an error.
fn tail_of(bytes: &[u8]) -> String {
    let text = strip(bytes);
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(TAIL_LINES)..].join("\n")
}

// ---------------------------------------------------------------------------
// The terminal

/// What reaches a session: what the terminal did, and what the person did.
#[derive(Debug)]
pub enum LoginEvent {
    /// The terminal printed these bytes.
    Output(Vec<u8>),
    /// The terminal's output closed.
    Eof,
    /// The terminal's process ended.
    Exited(Exit),
    /// The person typed.
    Input(Vec<u8>),
    Resize { cols: u16, rows: u16 },
    /// "Done, I'm logged in" (MP-14).
    Done,
    Cancel,
}

/// The tells a [`LoginSession`] about the person; clone it freely.
#[derive(Debug, Clone)]
pub struct LoginHandle {
    events: Sender<LoginEvent>,
}

impl LoginHandle {
    /// Each returns false once the session is over.
    pub fn input(&self, bytes: Vec<u8>) -> bool {
        self.events.send(LoginEvent::Input(bytes)).is_ok()
    }

    pub fn resize(&self, cols: u16, rows: u16) -> bool {
        self.events.send(LoginEvent::Resize { cols, rows }).is_ok()
    }

    pub fn done(&self) -> bool {
        self.events.send(LoginEvent::Done).is_ok()
    }

    pub fn cancel(&self) -> bool {
        self.events.send(LoginEvent::Cancel).is_ok()
    }
}

/// A command running on a pseudo-terminal, as its own session with the
/// terminal as its controlling one (so ssh can ask for a password). It is
/// killed, with everything it started, when this is dropped.
#[derive(Debug)]
pub struct LoginTerminal {
    master: File,
    group: Option<Pid>,
    exited: Arc<AtomicBool>,
}

impl LoginTerminal {
    /// Starts `invocation` with only the environment it names (plus
    /// `TERM`), sized `cols` by `rows`. Everything the terminal does is
    /// sent on `events`: output, then its end. `variables` gives host
    /// variables; none are supported without them.
    pub fn spawn(
        invocation: &Invocation,
        variables: &dyn HostVariables,
        cols: u16,
        rows: u16,
        events: Sender<LoginEvent>,
    ) -> Result<Self, LoginError> {
        let opened = |source: io::Error| LoginError::OpenPty { source };
        let (master, pty) = open_pty(cols, rows).map_err(opened)?;
        let spawn_failed = |source: io::Error| LoginError::Spawn {
            program: invocation.program.clone(),
            source,
        };

        let mut command = Command::new(&invocation.program);
        command.args(&invocation.args).env_clear();
        for name in BASELINE {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        command.env("TERM", "xterm-256color");
        for (name, value) in &invocation.env {
            match value {
                EnvValue::Value(value) => {
                    command.env(name, value);
                }
                EnvValue::HostVariable(variable) => {
                    let value = variables
                        .value(variable)
                        .map_err(|source| LoginError::UnreadableHostVariable {
                            variable: variable.clone(),
                            source,
                        })?
                        .ok_or_else(|| LoginError::HostVariable {
                            variable: variable.clone(),
                        })?;
                    command.env(name, value);
                }
            }
        }
        command
            .stdin(Stdio::from(pty.try_clone().map_err(spawn_failed)?))
            .stdout(Stdio::from(pty.try_clone().map_err(spawn_failed)?))
            .stderr(Stdio::from(pty));
        // SAFETY: between fork and exec this only makes system calls that
        // are safe there: it starts a session of its own and takes the
        // pseudo-terminal, already its stdin, as the controlling terminal,
        // so prompts read from it and job control works.
        unsafe {
            command.pre_exec(|| {
                rustix::process::setsid().map_err(io::Error::from)?;
                rustix::process::ioctl_tiocsctty(BorrowedFd::borrow_raw(0)).map_err(io::Error::from)?;
                Ok(())
            });
        }
        let mut child = command.spawn().map_err(spawn_failed)?;
        // The child holds the only other copies of its side, so reading
        // ends when it and what it started close the terminal.
        drop(command);
        let group = i32::try_from(child.id()).ok().and_then(Pid::from_raw);

        let mut reader = File::from(master.try_clone().map_err(opened)?);
        let output = events.clone();
        thread::spawn(move || {
            let mut buffer = [0_u8; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => {
                        if output.send(LoginEvent::Output(buffer[..n].to_vec())).is_err() {
                            return;
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                    // The terminal closing reads as an error on some systems.
                    Err(_) => break,
                }
            }
            let _ = output.send(LoginEvent::Eof);
        });
        let exited = Arc::new(AtomicBool::new(false));
        let done = exited.clone();
        thread::spawn(move || {
            let exit = match child.wait() {
                Ok(status) => match (status.code(), status.signal()) {
                    (Some(code), _) => Exit::Code(code),
                    (None, Some(signal)) => Exit::Signal(signal),
                    (None, None) => Exit::Code(-1),
                },
                Err(_) => Exit::Code(-1),
            };
            done.store(true, Ordering::SeqCst);
            let _ = events.send(LoginEvent::Exited(exit));
        });
        Ok(Self {
            master: File::from(master),
            group,
            exited,
        })
    }

    /// Types `bytes` into the terminal.
    pub fn write(&mut self, bytes: &[u8]) -> Result<(), LoginError> {
        self.master
            .write_all(bytes)
            .and_then(|()| self.master.flush())
            .map_err(|source| LoginError::Write { source })
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), LoginError> {
        set_size(&self.master, cols, rows).map_err(|source| LoginError::Terminal { source })
    }

    /// Puts the terminal in raw mode: no echo, no line editing, no
    /// translation, so bytes reach the far end unchanged.
    pub fn make_raw(&self) -> Result<(), LoginError> {
        let terminal = |error: rustix::io::Errno| LoginError::Terminal {
            source: io::Error::from(error),
        };
        let mut modes = rustix::termios::tcgetattr(&self.master).map_err(terminal)?;
        modes.make_raw();
        rustix::termios::tcsetattr(&self.master, rustix::termios::OptionalActions::Now, &modes).map_err(terminal)
    }

    /// Stops the command and everything it started.
    fn kill(&self) {
        if self.exited.load(Ordering::SeqCst) {
            return;
        }
        if let Some(group) = self.group {
            let _ = rustix::process::kill_process_group(group, Signal::HUP);
            let _ = rustix::process::kill_process_group(group, Signal::KILL);
        }
    }
}

impl Drop for LoginTerminal {
    fn drop(&mut self) {
        self.kill();
    }
}

fn set_size(master: &impl AsFd, cols: u16, rows: u16) -> io::Result<()> {
    let size = Winsize {
        ws_row: rows.max(1),
        ws_col: cols.max(1),
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    rustix::termios::tcsetwinsize(master, size).map_err(Into::into)
}

fn open_pty(cols: u16, rows: u16) -> io::Result<(OwnedFd, OwnedFd)> {
    let master = rustix::pty::openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY)?;
    rustix::io::fcntl_setfd(&master, rustix::io::FdFlags::CLOEXEC)?;
    rustix::pty::grantpt(&master)?;
    rustix::pty::unlockpt(&master)?;
    let name = rustix::pty::ptsname(&master, Vec::new())?;
    // Not this process's controlling terminal: only the command's.
    let pty = rustix::fs::open(
        name.as_c_str(),
        OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    set_size(&master, cols, rows)?;
    Ok((master, pty))
}

// ---------------------------------------------------------------------------
// The session

/// What the person's dialog is shown while they log in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observed {
    Output(Vec<u8>),
    /// The first time the output ends waiting for a person (MP-13), with
    /// the line that says so and what kind of prompt it is (D167).
    Prompt(Prompt),
}

/// What a takeover needs besides the terminal.
pub struct Plan<'a> {
    /// The files to install where they differ.
    pub builds: &'a dyn Builds,
    /// The folder of the host's enrollment keys when the machine has none
    /// yet: a key is made, written on the machine over the login and kept
    /// here. None for a machine that already has one.
    pub enroll_into: Option<&'a Path>,
    /// How long the shell gets to answer, and the link to say it is in
    /// sync, instead of the takeover's own.
    pub shell_timeout: Option<Duration>,
    pub sync_timeout: Option<Duration>,
}

/// One interactive login: the terminal, and the channel everything it and
/// the person do arrives on.
pub struct LoginSession {
    machine: MachineName,
    terminal: LoginTerminal,
    events: Receiver<LoginEvent>,
    exit: Option<Exit>,
    eof: bool,
    ended_at: Option<Instant>,
    recent: Vec<u8>,
}

/// What [`LoginSession::next`] found.
enum Next {
    Event(LoginEvent),
    Timeout,
    /// The terminal ended, with how its process did when known.
    Over(Option<Exit>),
}

impl LoginSession {
    /// Opens the terminal running `invocation` (a link's
    /// [`Link::interactive`]). Hand the [`LoginHandle`] to whatever relays
    /// the person; call [`LoginSession::run`] on a thread.
    pub fn open(
        machine: MachineName,
        invocation: &Invocation,
        variables: &dyn HostVariables,
        cols: u16,
        rows: u16,
    ) -> Result<(Self, LoginHandle), LoginError> {
        let (sender, events) = mpsc::channel();
        let terminal = LoginTerminal::spawn(invocation, variables, cols, rows, sender.clone())?;
        Ok((
            Self {
                machine,
                terminal,
                events,
                exit: None,
                eof: false,
                ended_at: None,
                recent: Vec::new(),
            },
            LoginHandle { events: sender },
        ))
    }

    /// Runs the login until it is over. While the person logs in, output
    /// goes to `observe` (which says false once nobody listens, which
    /// cancels), their input and size go to the terminal; at Done the
    /// takeover starts. On success the terminal carries the peer link:
    /// `humatad2 serve --stdio --sync` runs in it, and the result reads and
    /// writes its stream.
    pub fn run(
        mut self,
        plan: &Plan<'_>,
        observe: &mut dyn FnMut(Observed) -> bool,
    ) -> Result<LoginLink, LoginError> {
        let mut prompted = false;
        // Logging in: the terminal is the person's.
        loop {
            match self.next(None) {
                Next::Event(LoginEvent::Output(bytes)) => {
                    self.remember(&bytes);
                    if !prompted && let Some(prompt) = detect_prompt(&self.recent) {
                        prompted = true;
                        if !observe(Observed::Prompt(prompt)) {
                            return Err(self.cancelled());
                        }
                    }
                    if !observe(Observed::Output(bytes)) {
                        return Err(self.cancelled());
                    }
                }
                Next::Event(LoginEvent::Input(bytes)) => self.terminal.write(&bytes)?,
                Next::Event(LoginEvent::Resize { cols, rows }) => self.terminal.resize(cols, rows)?,
                Next::Event(LoginEvent::Done) => break,
                Next::Event(LoginEvent::Cancel) => return Err(self.cancelled()),
                Next::Event(_) | Next::Timeout => {}
                Next::Over(exit) => return Err(self.ended(exit)),
            }
        }
        self.take_over(plan, observe)
    }

    fn cancelled(&self) -> LoginError {
        LoginError::Cancelled {
            machine: self.machine.clone(),
        }
    }

    fn ended(&self, exit: Option<Exit>) -> LoginError {
        LoginError::Ended {
            machine: self.machine.clone(),
            exit,
            output_tail: tail_of(&self.recent),
        }
    }

    fn remember(&mut self, bytes: &[u8]) {
        self.recent.extend_from_slice(bytes);
        let excess = self.recent.len().saturating_sub(KEEP_BYTES);
        self.recent.drain(..excess);
    }

    /// The next event, waiting at most until `deadline`. The terminal's
    /// end is folded in: once its process ended and its output closed (or
    /// a short while after the first of the two), it is over.
    fn next(&mut self, deadline: Option<Instant>) -> Next {
        loop {
            if self.exit.is_some() && self.eof {
                return Next::Over(self.exit);
            }
            let drained_by = self.ended_at.map(|at| at + DRAIN);
            let limit = match (deadline, drained_by) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
            let received = match limit {
                Some(at) => self.events.recv_timeout(at.saturating_duration_since(Instant::now())),
                None => self.events.recv().map_err(|_| RecvTimeoutError::Disconnected),
            };
            match received {
                Ok(LoginEvent::Exited(exit)) => {
                    self.exit = Some(exit);
                    self.ended_at.get_or_insert_with(Instant::now);
                }
                Ok(LoginEvent::Eof) => {
                    self.eof = true;
                    self.ended_at.get_or_insert_with(Instant::now);
                }
                Ok(event) => return Next::Event(event),
                Err(RecvTimeoutError::Disconnected) => return Next::Over(self.exit),
                Err(RecvTimeoutError::Timeout) => {
                    if drained_by.is_some_and(|at| Instant::now() >= at) {
                        return Next::Over(self.exit);
                    }
                    return Next::Timeout;
                }
            }
        }
    }

    /// The person is in: drives the takeover until its bridge says it is in
    /// sync, the takeover fails, or the person cancels.
    ///
    /// Until the bridge line is typed, what the terminal prints still goes
    /// to `observe`: the person watches the takeover in the same terminal
    /// they logged in with (MP-14). After it, the stream is the link's.
    fn take_over(
        mut self,
        plan: &Plan<'_>,
        observe: &mut dyn FnMut(Observed) -> bool,
    ) -> Result<LoginLink, LoginError> {
        let mut takeover = Takeover::new(self.machine.clone());
        if let Some(timeout) = plan.shell_timeout {
            takeover = takeover.with_shell_timeout(timeout);
        }
        if let Some(timeout) = plan.sync_timeout {
            takeover = takeover.with_sync_timeout(timeout);
        }
        // Output printed while the person logged in isn't the shell's
        // answer to anything Humata typed.
        let mut enrolled: Option<Key> = None;
        let mut bridging = false;
        let mut quiet_since = Instant::now();
        let mut actions = takeover.logged_in();
        loop {
            for action in std::mem::take(&mut actions) {
                quiet_since = Instant::now();
                match action {
                    TakeoverAction::Write(bytes) => {
                        if is_bridge_line(&bytes) {
                            bridging = true;
                            if plan.enroll_into.is_some() {
                                enrolled = Some(self.enroll()?);
                            }
                            self.type_command(&as_peer_link(&bytes))?;
                        } else {
                            self.type_command(&bytes)?;
                        }
                    }
                    TakeoverAction::MakeLocalRaw => self.terminal.make_raw()?,
                    TakeoverAction::SendFile(payload) => self.terminal.write(payload.bytes())?,
                    TakeoverAction::SendBundle(bundle) => self.terminal.write(&bundle)?,
                    TakeoverAction::Ready { platform, leftover } => {
                        let mut wrote = false;
                        if let (Some(dir), Some(key)) = (plan.enroll_into, enrolled.take()) {
                            enroll::write_host(dir, &self.machine, &key).map_err(|source| {
                                LoginError::SaveEnrollment {
                                    path: enroll::host_path(dir, self.machine.as_str()),
                                    source,
                                }
                            })?;
                            wrote = true;
                        }
                        return Ok(LoginLink {
                            machine: self.machine,
                            platform,
                            enrolled: wrote,
                            reader: LinkReader {
                                events: self.events,
                                pending: leftover,
                                exited: self.exit.is_some(),
                            },
                            writer: LinkWriter {
                                master: self.terminal.master.try_clone().map_err(|source| LoginError::Write { source })?,
                                keeper: Arc::new(self.terminal),
                            },
                        });
                    }
                    TakeoverAction::Failed(error) => return Err(error.into()),
                }
            }
            // Where the takeover sets no deadline of its own (installing a
            // file), a terminal that stays silent is given up on.
            let own = takeover.deadline();
            let stalls_at = quiet_since + INSTALL_STALL;
            actions = match self.next(Some(own.unwrap_or(stalls_at))) {
                Next::Event(LoginEvent::Output(bytes)) => {
                    quiet_since = Instant::now();
                    self.remember(&bytes);
                    if !bridging && !observe(Observed::Output(bytes.clone())) {
                        return Err(self.cancelled());
                    }
                    takeover.output(&bytes, plan.builds)
                }
                Next::Event(LoginEvent::Resize { cols, rows }) => {
                    self.terminal.resize(cols, rows)?;
                    Vec::new()
                }
                Next::Event(LoginEvent::Cancel) => return Err(self.cancelled()),
                // The person's typing would corrupt what Humata types.
                Next::Event(_) => Vec::new(),
                Next::Timeout if own.is_none() => {
                    if Instant::now() >= stalls_at {
                        return Err(LoginError::Stalled {
                            machine: self.machine.clone(),
                            waited: INSTALL_STALL,
                            output_tail: tail_of(&self.recent),
                        });
                    }
                    Vec::new()
                }
                Next::Timeout => takeover.time_passed(Instant::now()),
                Next::Over(exit) => takeover.ended(exit),
            };
        }
    }

    /// Types one command line (as the takeover wrote it: Ctrl-U, the
    /// command, a newline). A short one is typed as it is. A long one would
    /// overflow the terminal's line buffer and arrive mangled, so a short
    /// line is typed first that turns the terminal raw, says it is ready,
    /// reads exactly the command's bytes, puts the terminal back and runs
    /// what it read; the command is sent once that line says it is ready.
    /// Raw mode holds the writer back when the buffer is full instead of
    /// dropping bytes, as it does for the files sent the same way.
    fn type_command(&mut self, bytes: &[u8]) -> Result<(), LoginError> {
        if bytes.len() <= TYPED_DIRECT_MAX {
            return self.terminal.write(bytes);
        }
        let command = bytes.strip_prefix(&[0x15_u8][..]).unwrap_or(bytes);
        let command = command.strip_suffix(&b"\n"[..]).unwrap_or(command);
        let n = install::nonce();
        let reader = format!(
            "\u{15}sh -c 'o=$(stty -g 2>/dev/null); stty raw -echo 2>/dev/null; printf \"%s_%s_%s\\n\" HUMATA TYPED {n}; s=$(head -c {len}); stty \"$o\" 2>/dev/null; eval \"$s\"'\n",
            len = command.len()
        );
        self.terminal.write(reader.as_bytes())?;
        self.wait_for(&format!("HUMATA_TYPED_{n}")).map_err(|error| match error {
            LoginError::Enrollment { machine, output_tail } => LoginError::Stalled {
                machine,
                waited: ENROLL_TIMEOUT,
                output_tail,
            },
            other => other,
        })?;
        self.terminal.write(command)
    }

    /// Writes a fresh enrollment key on the machine through the shell, with
    /// the terminal's echo off first so the key isn't printed (GRPC.md §8).
    /// The host's copy is saved only once the takeover is ready.
    fn enroll(&mut self) -> Result<Key, LoginError> {
        let key = Key::new();
        let n = install::nonce();
        self.terminal.write(
            format!("stty -echo 2>/dev/null; printf '%s_%s_%s\\n' HUMATA ENR {n}\n").as_bytes(),
        )?;
        self.wait_for(&format!("HUMATA_ENR_{n}"))?;
        // As `enroll::machine_script`, but the key is one line the script
        // asks for once it is running, not typed ahead: a shell may have
        // changed the terminal's mode between the two.
        let script = quote::sh_command(&format!(
            "umask 077\nd=\"$HOME/.humata2/state\"\nmkdir -p \"$d\" || exit 1\nt=\"$d/.enrollment-key.$$.new\"\n\
             printf '%s_%s_%s\\n' HUMATA ENRREADY {n}\n\
             IFS= read -r k || {{ rm -f \"$t\"; exit 1; }}\n\
             if printf '%s' \"$k\" > \"$t\" && mv -f \"$t\" \"$d/enrollment-key\"; then exit 0; fi\nrm -f \"$t\"\nexit 1"
        ))?;
        let mut typed = script.into_bytes();
        typed.extend_from_slice(format!("; printf '%s_%s_%s %s\\n' HUMATA ENRDONE {n} \"$?\"\n").as_bytes());
        self.terminal.write(&typed)?;
        self.wait_for(&format!("HUMATA_ENRREADY_{n}"))?;
        let mut line = key.bytes().to_vec();
        line.push(b'\n');
        self.terminal.write(&line)?;
        let answer = self.wait_for(&format!("HUMATA_ENRDONE_{n} "))?;
        if answer.trim() != "0" {
            return Err(LoginError::Enrollment {
                machine: self.machine.clone(),
                output_tail: tail_of(&self.recent),
            });
        }
        Ok(key)
    }

    /// Waits for a line holding `marker` and returns the rest of it. The
    /// echo of what was typed never holds it: markers are put together by
    /// `printf`.
    fn wait_for(&mut self, marker: &str) -> Result<String, LoginError> {
        let deadline = Instant::now() + ENROLL_TIMEOUT;
        let mut seen = Vec::new();
        loop {
            match self.next(Some(deadline)) {
                Next::Event(LoginEvent::Output(bytes)) => {
                    self.remember(&bytes);
                    seen.extend_from_slice(&bytes);
                    let text = strip(&seen);
                    for line in text.lines() {
                        if let Some(index) = line.find(marker) {
                            return Ok(line[index + marker.len()..].to_owned());
                        }
                    }
                }
                Next::Event(LoginEvent::Cancel) => return Err(self.cancelled()),
                Next::Event(LoginEvent::Resize { cols, rows }) => self.terminal.resize(cols, rows)?,
                Next::Event(_) => {}
                Next::Timeout => {
                    return Err(LoginError::Enrollment {
                        machine: self.machine.clone(),
                        output_tail: tail_of(&self.recent),
                    });
                }
                Next::Over(exit) => return Err(self.ended(exit)),
            }
        }
    }
}

/// The line that starts the bridge, the takeover's last.
fn is_bridge_line(bytes: &[u8]) -> bool {
    String::from_utf8_lossy(bytes).contains("bridge --sync ")
}

/// The takeover's bridge line asking for the peer link instead: the bridge
/// pipes to the machine's socket, while the host's link speaks to
/// `serve --stdio` (D43). One word changes, so the sync marker and the
/// takeover's waiting are the same.
///
/// The line also prints a newline before anything else. The takeover only
/// takes a sync marker that starts a line, and an interactive shell's
/// prompt can arrive just before it with no newline after (the shell
/// echoes nothing once the enrollment turned echo off), which left the
/// marker in the middle of a line and the takeover waiting.
fn as_peer_link(bytes: &[u8]) -> Vec<u8> {
    String::from_utf8_lossy(bytes)
        .replacen("bridge --sync ", "serve --stdio --sync ", 1)
        .replacen("stty raw ", "echo; stty raw ", 1)
        .into_bytes()
}

// ---------------------------------------------------------------------------
// The link a login ends with

/// A login that ended in the peer link: the terminal now carries gRPC.
pub struct LoginLink {
    machine: MachineName,
    platform: Platform,
    enrolled: bool,
    reader: LinkReader,
    writer: LinkWriter,
}

impl LoginLink {
    pub fn machine(&self) -> &MachineName {
        &self.machine
    }

    pub fn platform(&self) -> Platform {
        self.platform
    }

    /// Whether this login wrote a new enrollment key (and the host kept its
    /// copy), so undoing the login must forget it.
    pub fn enrolled(&self) -> bool {
        self.enrolled
    }

    /// The two halves of the byte stream, to use on threads of their own.
    /// The terminal and what it started are stopped when both are dropped.
    pub fn split(self) -> (LinkReader, LinkWriter) {
        (self.reader, self.writer)
    }
}

/// What the machine sends: the bytes the bridge printed after its marker,
/// then the terminal's output, until it closes.
pub struct LinkReader {
    events: Receiver<LoginEvent>,
    pending: Vec<u8>,
    exited: bool,
}

impl Read for LinkReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            if !self.pending.is_empty() {
                let n = buf.len().min(self.pending.len());
                buf[..n].copy_from_slice(&self.pending[..n]);
                self.pending.drain(..n);
                return Ok(n);
            }
            let received = if self.exited {
                self.events.recv_timeout(DRAIN)
            } else {
                self.events.recv().map_err(|_| RecvTimeoutError::Disconnected)
            };
            match received {
                Ok(LoginEvent::Output(bytes)) => self.pending = bytes,
                Ok(LoginEvent::Eof) => return Ok(0),
                Ok(LoginEvent::Exited(_)) => self.exited = true,
                Ok(_) => {}
                Err(_) => return Ok(0),
            }
        }
    }
}

/// What the host sends the machine.
pub struct LinkWriter {
    master: File,
    keeper: Arc<LoginTerminal>,
}

impl Write for LinkWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // Keeps the terminal alive for as long as the link is written to.
        let _ = &self.keeper;
        self.master.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.master.flush()
    }
}

impl LinkWriter {
    /// Stops the command and everything it started: the link is over.
    pub fn close(&self) {
        self.keeper.kill();
    }
}

// ---------------------------------------------------------------------------
// Where a login goes

/// The link an existing machine is reached by, to open a login to it
/// (MP-16 Log in).
pub fn existing_link(machines: &Machines, machine: &str) -> Result<Link, LoginError> {
    let manage = |source| LoginError::Manage {
        machine: machine.to_owned(),
        source,
    };
    let entry = machines.get(machine).map_err(manage)?;
    machines.link_if_any(&entry).map_err(manage)?.ok_or_else(|| LoginError::NoLink {
        machine: machine.to_owned(),
    })
}

/// The link `provider`'s `connect` makes to a machine not yet listed, for
/// the login a failed create asked for (MP-13).
pub fn provider_link(
    provider: &Provider,
    machine: &MachineName,
    values: &BTreeMap<String, String>,
) -> Result<Link, LoginError> {
    provider.link(machine, values)?.ok_or_else(|| LoginError::IdentityProvider {
        provider: provider.id().to_owned(),
    })
}

// ---------------------------------------------------------------------------
// Registering the machine

/// Marks `machine` as one whose reconnects need a person (MP-16), or not.
/// The mark is a file beside `hosts.json`, so hosts.json's format didn't
/// change: `<state>/interactive/<machine>`.
pub fn mark_interactive(hosts_file: &Path, machine: &str, interactive: bool) -> io::Result<()> {
    let dir = interactive_dir(hosts_file);
    let file = dir.join(machine);
    if interactive {
        DirBuilder::new().recursive(true).mode(0o700).create(&dir)?;
        OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&file)?;
        Ok(())
    } else {
        match fs::remove_file(&file) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }
}

/// Whether `machine`'s reconnects need a person.
pub fn is_interactive(hosts_file: &Path, machine: &str) -> bool {
    interactive_dir(hosts_file).join(machine).is_file()
}

fn interactive_dir(hosts_file: &Path) -> PathBuf {
    hosts_file.parent().unwrap_or(Path::new(".")).join("interactive")
}

/// Registers the machine a login ended in (MP-19): an existing entry is
/// updated; otherwise `made` (the provider that connects to it, and the
/// params it was made with) makes a new one. The machine is marked
/// interactive (MP-16). Hosts.json keeps no secrets.
pub fn register(
    machines: &Machines,
    machine: &MachineName,
    made: Option<(&Provider, &BTreeMap<String, String>)>,
    link: &Link,
    platform: Platform,
) -> Result<HostEntry, LoginError> {
    let hosts = machines.hosts();
    let name = machine.as_str();
    let entry = match hosts.get(name)? {
        Some(mut entry) => {
            entry.state = EntryState::Ready;
            entry.undo_pending = false;
            entry.os = platform.os.to_string();
            entry.arch = platform.arch.to_string();
            if let Some(provider) = &mut entry.provider
                && provider.link.is_none()
            {
                provider.link = Some(link.stored());
            }
            hosts.put(entry.clone())?;
            entry
        }
        None => {
            let (provider, values) = made.ok_or_else(|| LoginError::NotRegistrable {
                machine: name.to_owned(),
            })?;
            let entry = HostEntry {
                name: name.to_owned(),
                ssh_host: format!("via {}", provider.id()),
                root: DEFAULT_ROOT.to_owned(),
                state: EntryState::Ready,
                os: platform.os.to_string(),
                arch: platform.arch.to_string(),
                undo_pending: false,
                provider: Some(ProviderMachine {
                    id: provider.id().to_owned(),
                    params: values.clone(),
                    link: Some(link.stored()),
                }),
            };
            if !hosts.insert_new(entry.clone())? {
                return Err(LoginError::Manage {
                    machine: name.to_owned(),
                    source: ManageError::NameTaken {
                        machine: name.to_owned(),
                    },
                });
            }
            entry
        }
    };
    mark_interactive(machines.hosts_file(), name, true).map_err(|source| LoginError::KeyFile {
        path: interactive_dir(machines.hosts_file()).join(name),
        source,
    })?;
    Ok(entry)
}

/// Undoes [`register`] for a machine the login created: its entry, the
/// mark, and (when the login wrote one) the host's enrollment key. What was
/// put on the machine itself (humatad2) is the machine's to keep: a
/// retried login finds it in place.
pub fn unregister(machines: &Machines, machine: &MachineName, enrolled: bool) -> Result<(), LoginError> {
    let name = machine.as_str();
    machines.hosts().remove(name)?;
    let _ = mark_interactive(machines.hosts_file(), name, false);
    let key_file = machines.peer_key(name);
    if enrolled && let Some(dir) = key_file.parent() {
        let _ = enroll::remove_host(dir, name);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Making it unattended (MP-17)

/// How an interactive machine stops needing a person.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// Add a key Humata made for this machine to its `authorized_keys`,
    /// and connect with it.
    AuthorizedKey,
    /// The provider's tool keeps a login (a cached token): its connect
    /// already works without a person.
    KeepCachedToken,
}

/// Makes `machine` unattended (MP-17) by `method` and returns its entry.
///
/// `AuthorizedKey` makes a key pair under the host's state folder (`login-keys`,
/// 0600, never `~/.ssh`), appends the public key to the machine's
/// `authorized_keys` through its current link, checks that a connect with
/// the key now works without a person, and switches the machine's stored link to
/// SSH with that key. When the check fails the appended line is removed
/// and the key pair deleted (PER-27), and the machine stays as it was.
/// It needs the machine's SSH address (the stored link's, or provider
/// params `host`/`ssh_host`, `user` and `port`) and its host key pinned in
/// Humata's own known_hosts: a key is never trusted over a connection
/// nobody checked.
///
/// `KeepCachedToken` checks the provider's own connect works without a
/// person and clears the mark.
pub fn make_unattended(
    machines: &Machines,
    machine: &str,
    method: Method,
    runner: &dyn Runner,
) -> Result<HostEntry, LoginError> {
    let manage = |source| LoginError::Manage {
        machine: machine.to_owned(),
        source,
    };
    let _busy = machines.claim(machine).map_err(manage)?;
    let entry = machines.get(machine).map_err(manage)?;
    if !is_interactive(machines.hosts_file(), machine) {
        return Err(LoginError::NotInteractive {
            machine: machine.to_owned(),
        });
    }
    let link = machines.link_if_any(&entry).map_err(manage)?.ok_or_else(|| LoginError::NoLink {
        machine: machine.to_owned(),
    })?;
    let cleared = |source| LoginError::KeyFile {
        path: interactive_dir(machines.hosts_file()).join(machine),
        source,
    };
    match method {
        Method::KeepCachedToken => {
            let probe = link.script("true", Purpose::Probe, Some(VERIFY_TIMEOUT), None)?;
            let output = runner.run(&probe)?;
            if !output.exit.success() {
                return Err(LoginError::StillNeedsLogin {
                    machine: machine.to_owned(),
                    exit: output.exit,
                    output_tail: output.tail(TAIL_LINES),
                    undone: true,
                });
            }
            mark_interactive(machines.hosts_file(), machine, false).map_err(cleared)?;
            Ok(entry)
        }
        Method::AuthorizedKey => {
            let entry = authorize(machines, entry, &link, runner)?;
            mark_interactive(machines.hosts_file(), machine, false).map_err(cleared)?;
            Ok(entry)
        }
    }
}

fn authorize(machines: &Machines, mut entry: HostEntry, link: &Link, runner: &dyn Runner) -> Result<HostEntry, LoginError> {
    let machine = entry.name.clone();
    let name = MachineName::new(&machine).map_err(|source| LoginError::Manage {
        machine: machine.clone(),
        source: ManageError::BadName {
            value: machine.clone(),
            source,
        },
    })?;
    let (host, port, user) = ssh_address(&entry, link).ok_or_else(|| LoginError::NoSshAddress {
        machine: machine.clone(),
    })?;
    let pins = machines.known_hosts_file();
    if known_hosts::pinned(pins, &name)
        .ok()
        .flatten()
        .is_none()
    {
        return Err(LoginError::HostKeyNotPinned { machine });
    }

    // A key pair of Humata's own for this machine, never in ~/.ssh.
    let keys = machines
        .hosts_file()
        .parent()
        .unwrap_or(Path::new("."))
        .join("login-keys");
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&keys)
        .map_err(|source| LoginError::KeyFile { path: keys.clone(), source })?;
    let private = keys.join(&machine);
    let public_file = keys.join(format!("{machine}.pub"));
    for stale in [&private, &public_file] {
        match fs::remove_file(stale) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => {
                return Err(LoginError::KeyFile {
                    path: stale.clone(),
                    source: error,
                });
            }
            _ => {}
        }
    }
    let generate = Invocation {
        program: "ssh-keygen".to_owned(),
        args: vec![
            "-q".to_owned(),
            "-t".to_owned(),
            "ed25519".to_owned(),
            "-N".to_owned(),
            String::new(),
            "-C".to_owned(),
            format!("humata2:{machine}"),
            "-f".to_owned(),
            private.display().to_string(),
        ],
        env: Vec::new(),
        stdin: None,
        timeout: Some(KEYGEN_TIMEOUT),
        purpose: Purpose::AuthorizeKey,
    };
    let delete_pair = || {
        let _ = fs::remove_file(&private);
        let _ = fs::remove_file(&public_file);
    };
    let made = runner.run(&generate)?;
    if !made.exit.success() {
        delete_pair();
        return Err(LoginError::KeyGen {
            machine,
            exit: made.exit,
            output_tail: made.tail(TAIL_LINES),
        });
    }
    let prepared = fs::set_permissions(&private, fs::Permissions::from_mode(0o600))
        .map_err(|source| LoginError::KeyFile { path: private.clone(), source })
        .and_then(|()| {
            fs::read_to_string(&public_file).map_err(|source| LoginError::KeyFile {
                path: public_file.clone(),
                source,
            })
        });
    let public = match prepared {
        Ok(text) => text.trim().to_owned(),
        Err(error) => {
            delete_pair();
            return Err(error);
        }
    };

    let appended = runner.run(&link.authorize_key(&public)?);
    let appended = match appended {
        Ok(output) => output,
        Err(error) => {
            delete_pair();
            return Err(error.into());
        }
    };
    if !appended.exit.success() {
        delete_pair();
        return Err(LoginError::Authorize {
            machine,
            exit: appended.exit,
            output_tail: appended.tail(TAIL_LINES),
        });
    }

    // Undoes the append, and the key pair; true when the machine is clean.
    let undo = || {
        let removed = remove_key_script(&public)
            .and_then(|script| link.script(&script, Purpose::AuthorizeKey, Some(VERIFY_TIMEOUT), None))
            .ok()
            .and_then(|invocation| runner.run(&invocation).ok())
            .is_some_and(|output| output.exit.success());
        delete_pair();
        removed
    };

    let ssh = Link::ssh(
        name,
        SshTarget {
            host: host.clone(),
            access: SshAccess::Direct {
                port,
                user: user.clone(),
                identity_file: Some(private.display().to_string()),
            },
            host_key: None,
        },
        pins,
    );
    let probe = match ssh.script("true", Purpose::Probe, Some(VERIFY_TIMEOUT), None) {
        Ok(probe) => probe,
        Err(error) => {
            undo();
            return Err(error.into());
        }
    };
    let verified = match runner.run(&probe) {
        Ok(output) => output,
        Err(error) => {
            undo();
            return Err(error.into());
        }
    };
    if !verified.exit.success() {
        let undone = undo();
        return Err(LoginError::StillNeedsLogin {
            machine,
            exit: verified.exit,
            output_tail: verified.tail(TAIL_LINES),
            undone,
        });
    }

    // Switch the stored link to the non-interactive form.
    match &mut entry.provider {
        Some(provider) => provider.link = Some(ssh.stored()),
        None => {
            undo();
            return Err(LoginError::NotRegistrable { machine });
        }
    }
    if let Err(error) = machines.hosts().put(entry.clone()) {
        undo();
        return Err(error.into());
    }
    Ok(entry)
}

/// Where the machine can be reached over SSH: the stored link's, or the
/// provider params `host` (or `ssh_host`), `user` and `port`.
fn ssh_address(entry: &HostEntry, link: &Link) -> Option<(String, u16, String)> {
    let record = entry.provider.as_ref()?;
    if let Some(StoredLink::Ssh {
        host,
        port,
        user: Some(user),
        ..
    }) = &record.link
    {
        return Some((host.clone(), port.unwrap_or(22), user.clone()));
    }
    // Only a provider's own connect is without an address of its own.
    link.is_connect().then_some(())?;
    let host = record.params.get("host").or_else(|| record.params.get("ssh_host"))?;
    let user = record.params.get("user")?;
    let port = match record.params.get("port") {
        Some(port) => port.parse().ok()?,
        None => 22,
    };
    Some((host.clone(), port, user.clone()))
}

/// Removes exactly the line `public_key` from `authorized_keys`, keeping
/// the rest and the file's mode.
fn remove_key_script(public_key: &str) -> Result<String, QuoteError> {
    let key = quote::quote("the public key", public_key)?;
    Ok(format!(
        "f=\"$HOME/.ssh/authorized_keys\"; [ -f \"$f\" ] || exit 0; t=\"$f.humata2.$$\"; \
         umask 077; grep -vxF -- {key} \"$f\" > \"$t\"; mv -f \"$t\" \"$f\"; chmod 600 \"$f\"; exit 0"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompts_that_wait_for_a_person_are_found() {
        let waiting = [
            "Verification code: ",
            "Password: ",
            "me@box's password: ",
            "[sudo] password for me: ",
            "Enter passphrase for key '/home/me/.ssh/id_ed25519': ",
            "Are you sure you want to continue connecting (yes/no)? ",
            "Are you sure you want to continue connecting (yes/no/[fingerprint])? ",
            "\x1b[1mOne-time passcode:\x1b[0m ",
            "Enter your OTP: ",
        ];
        for text in waiting {
            assert!(detect_prompt(text.as_bytes()).is_some(), "{text:?}");
        }
        let device = "To sign in, open https://example.test/device and enter the code ABCD-1234\nWaiting for you...\n";
        assert!(detect_prompt(device.as_bytes()).is_some());
    }

    #[test]
    fn the_matched_line_is_what_is_returned() {
        let found = detect_prompt(b"connecting...\r\n\x1b[2mVerification code: \x1b[0m").unwrap();
        assert_eq!(found.line, "Verification code:");
        assert_eq!(found.kind, PromptKind::Code);
    }

    #[test]
    fn each_prompt_comes_with_the_kind_of_rule_that_matched() {
        let cases = [
            ("Password: ", PromptKind::Password),
            ("me@box's password: ", PromptKind::Password),
            ("[sudo] password for me: ", PromptKind::Password),
            ("Enter your one-time password: ", PromptKind::Password),
            ("Enter passphrase for key '/home/me/.ssh/id_ed25519': ", PromptKind::Passphrase),
            ("Verification code: ", PromptKind::Code),
            ("\x1b[1mOne-time passcode:\x1b[0m ", PromptKind::Code),
            ("Enter your OTP: ", PromptKind::Code),
            ("Your 2FA: ", PromptKind::Code),
            ("Are you sure you want to continue connecting (yes/no)? ", PromptKind::Confirm),
            (
                "Are you sure you want to continue connecting (yes/no/[fingerprint])? ",
                PromptKind::Confirm,
            ),
            (
                "To sign in, open https://example.test/device and enter the code ABCD-1234\nWaiting for you...\n",
                PromptKind::Device,
            ),
        ];
        for (text, kind) in cases {
            let found = detect_prompt(text.as_bytes()).unwrap_or_else(|| panic!("{text:?} is a prompt"));
            assert_eq!(found.kind, kind, "{text:?}");
        }
    }

    #[test]
    fn a_line_found_by_looser_words_gets_its_kind_from_them() {
        assert_eq!(kind_of_line("dev@10.0.0.5's Password:"), PromptKind::Password);
        assert_eq!(kind_of_line("Enter passphrase for key 'id':"), PromptKind::Passphrase);
        assert_eq!(kind_of_line("Verification code:"), PromptKind::Code);
        assert_eq!(kind_of_line("Continue connecting (yes/no)?"), PromptKind::Confirm);
        assert_eq!(kind_of_line("Visit https://example.test and enter the code 1234"), PromptKind::Device);
    }

    #[test]
    fn look_alikes_and_plain_output_are_not_prompts() {
        let quiet = [
            "",
            "Welcome to box-1\n",
            "Your password was changed 3 days ago.\n",
            "Set a new password: see https://example.test/docs for the rules\n",
            "password manager started\n$ ",
            "Verification code: 123456\nlogged in\n",
            "Visit https://example.test for more information\n",
            "Do you want to continue (y/n)? ",
            "Authenticating with public key \"id\"\n",
        ];
        for text in quiet {
            assert!(detect_prompt(text.as_bytes()).is_none(), "{text:?}");
        }
    }

    #[test]
    fn removing_a_key_names_only_that_line() {
        let script = remove_key_script("ssh-ed25519 AAAA humata2:box").unwrap();
        assert!(script.contains("grep -vxF"));
        assert!(script.contains("'ssh-ed25519 AAAA humata2:box'"));
    }

    #[test]
    fn the_interactive_mark_is_a_private_file_beside_hosts_json() {
        let dir = tempfile::tempdir().unwrap();
        let hosts = dir.path().join("hosts.json");
        assert!(!is_interactive(&hosts, "box-1"));
        mark_interactive(&hosts, "box-1", true).unwrap();
        assert!(is_interactive(&hosts, "box-1"));
        assert!(!is_interactive(&hosts, "box-2"));
        mark_interactive(&hosts, "box-1", false).unwrap();
        mark_interactive(&hosts, "box-1", false).unwrap();
        assert!(!is_interactive(&hosts, "box-1"));
    }
}

/// Making a machine unattended against a machine home in a temp folder: a
/// fake `ssh-keygen` and a connect that runs the machine's script locally
/// with `HOME` set to that folder. Never the real `~/.ssh`.
#[cfg(test)]
mod unattended_tests {
    use std::process::Command as Process;
    use std::sync::Mutex;

    use super::*;
    use crate::manage::MachinesConfig;
    use crate::runner::Output;
    use crate::setup::RetryPolicy;

    const HOST_KEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl";
    const PUBLIC: &str = "ssh-ed25519 AAAAFAKEKEY humata2:box-1";
    const OTHER: &str = "ssh-ed25519 AAAAOTHER me@laptop";

    struct Fake {
        home: PathBuf,
        /// Whether a connect with the new key (or the cached token) works.
        probe_ok: bool,
        ran: Mutex<Vec<Purpose>>,
    }

    impl Runner for Fake {
        fn run(&self, invocation: &Invocation) -> Result<crate::runner::Output, RunError> {
            self.ran.lock().unwrap().push(invocation.purpose);
            let done = |code: i32| Output {
                exit: Exit::Code(code),
                stdout: Vec::new(),
                stderr: b"fake output".to_vec(),
            };
            if invocation.program == "ssh-keygen" {
                let path = invocation.args.last().unwrap();
                fs::write(path, "PRIVATE").unwrap();
                fs::write(format!("{path}.pub"), format!("{PUBLIC}\n")).unwrap();
                return Ok(done(0));
            }
            if invocation.purpose == Purpose::Probe {
                return Ok(done(if self.probe_ok { 0 } else { 255 }));
            }
            // The machine's own script, run here with the machine's home.
            let status = Process::new(&invocation.program)
                .args(&invocation.args)
                .env_clear()
                .env("HOME", &self.home)
                .env("PATH", "/usr/bin:/bin")
                .status()
                .unwrap();
            Ok(done(status.code().unwrap_or(-1)))
        }
    }

    struct World {
        dir: tempfile::TempDir,
        machines: Machines,
        fake: Fake,
    }

    impl World {
        fn new(probe_ok: bool) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let state = dir.path().join("state");
            fs::create_dir_all(&state).unwrap();
            let home = dir.path().join("machine-home");
            fs::create_dir_all(home.join(".ssh")).unwrap();
            fs::write(home.join(".ssh/authorized_keys"), format!("{OTHER}\n")).unwrap();
            fs::write(state.join("known_hosts"), format!("humata2.box-1 {HOST_KEY}\n")).unwrap();
            let machines = Machines::open(MachinesConfig {
                hosts_file: state.join("hosts.json"),
                known_hosts: state.join("known_hosts"),
                ssh_config: state.join("ssh_config"),
                peer_keys: state.join("peer-keys"),
                retry: RetryPolicy {
                    attempts: 2,
                    first_delay: std::time::Duration::ZERO,
                    max_delay: std::time::Duration::ZERO,
                },
            });
            let entry = HostEntry {
                name: "box-1".to_owned(),
                ssh_host: "via corp".to_owned(),
                root: DEFAULT_ROOT.to_owned(),
                state: EntryState::Ready,
                os: "linux".to_owned(),
                arch: "x86_64".to_owned(),
                undo_pending: false,
                provider: Some(ProviderMachine {
                    id: "corp".to_owned(),
                    params: BTreeMap::from([
                        ("host".to_owned(), "box-1.internal".to_owned()),
                        ("user".to_owned(), "me".to_owned()),
                    ]),
                    // A connect that runs the machine's script right here.
                    link: Some(StoredLink::Connect {
                        template: "sh -c {cmd}".to_owned(),
                        env: Vec::new(),
                    }),
                }),
            };
            machines.hosts().put(entry).unwrap();
            mark_interactive(machines.hosts_file(), "box-1", true).unwrap();
            Self {
                fake: Fake {
                    home,
                    probe_ok,
                    ran: Mutex::new(Vec::new()),
                },
                dir,
                machines,
            }
        }

        fn authorized(&self) -> Vec<String> {
            fs::read_to_string(self.fake.home.join(".ssh/authorized_keys"))
                .unwrap()
                .lines()
                .map(str::to_owned)
                .collect()
        }

        fn keys(&self) -> PathBuf {
            self.dir.path().join("state/login-keys")
        }
    }

    #[test]
    fn an_authorized_key_is_appended_once_and_the_link_switches_to_ssh() {
        let world = World::new(true);

        let entry = make_unattended(&world.machines, "box-1", Method::AuthorizedKey, &world.fake).unwrap();

        assert_eq!(world.authorized(), [OTHER, PUBLIC]);
        let link = entry.provider.as_ref().unwrap().link.clone().unwrap();
        let StoredLink::Ssh {
            host,
            user,
            identity_file,
            ..
        } = link
        else {
            panic!("the link didn't switch to ssh");
        };
        assert_eq!((host.as_str(), user.as_deref()), ("box-1.internal", Some("me")));
        let key = world.keys().join("box-1");
        assert_eq!(identity_file.as_deref(), Some(key.to_str().unwrap()));
        assert_eq!(fs::metadata(&key).unwrap().permissions().mode() & 0o777, 0o600);
        assert!(!is_interactive(world.machines.hosts_file(), "box-1"));
        assert_eq!(world.machines.get("box-1").unwrap(), entry);
        // Asking again has nothing to do: the machine isn't interactive.
        assert!(matches!(
            make_unattended(&world.machines, "box-1", Method::AuthorizedKey, &world.fake),
            Err(LoginError::NotInteractive { .. })
        ));
        assert_eq!(world.authorized(), [OTHER, PUBLIC]);
    }

    #[test]
    fn a_connect_that_still_needs_a_person_removes_the_key_again() {
        let world = World::new(false);
        let before = world.machines.get("box-1").unwrap();

        let error = make_unattended(&world.machines, "box-1", Method::AuthorizedKey, &world.fake).unwrap_err();

        assert!(matches!(error, LoginError::StillNeedsLogin { undone: true, .. }), "{error}");
        assert_eq!(world.authorized(), [OTHER]);
        assert!(!world.keys().join("box-1").exists());
        assert!(!world.keys().join("box-1.pub").exists());
        assert_eq!(world.machines.get("box-1").unwrap(), before);
        assert!(is_interactive(world.machines.hosts_file(), "box-1"));
    }

    #[test]
    fn a_host_key_nobody_pinned_is_not_trusted_with_a_new_key() {
        let world = World::new(true);
        fs::write(world.dir.path().join("state/known_hosts"), "").unwrap();

        let error = make_unattended(&world.machines, "box-1", Method::AuthorizedKey, &world.fake).unwrap_err();

        assert!(matches!(error, LoginError::HostKeyNotPinned { .. }), "{error}");
        assert_eq!(world.authorized(), [OTHER]);
        assert!(world.fake.ran.lock().unwrap().is_empty(), "nothing ran");
    }

    #[test]
    fn a_cached_token_is_kept_only_when_connect_works_without_a_person() {
        let works = World::new(true);
        make_unattended(&works.machines, "box-1", Method::KeepCachedToken, &works.fake).unwrap();
        assert!(!is_interactive(works.machines.hosts_file(), "box-1"));
        assert!(matches!(
            works.machines.get("box-1").unwrap().provider.unwrap().link,
            Some(StoredLink::Connect { .. })
        ));

        let broken = World::new(false);
        let error = make_unattended(&broken.machines, "box-1", Method::KeepCachedToken, &broken.fake).unwrap_err();
        assert!(matches!(error, LoginError::StillNeedsLogin { .. }), "{error}");
        assert!(is_interactive(broken.machines.hosts_file(), "box-1"));
    }

    #[test]
    fn registering_a_login_creates_the_entry_and_unregistering_removes_it() {
        let world = World::new(true);
        let provider = Provider::parse("[providers.corp]\ncreate = \"true\"\nconnect = \"sh -c {cmd}\"\n")
            .unwrap()
            .remove(0);
        let name = MachineName::new("box-2").unwrap();
        let link = provider.link(&name, &BTreeMap::new()).unwrap().unwrap();
        let platform = Platform {
            os: crate::platform::Os::Linux,
            arch: crate::platform::Arch::X86_64,
        };

        let entry = register(&world.machines, &name, Some((&provider, &BTreeMap::new())), &link, platform).unwrap();

        assert_eq!(entry.state, EntryState::Ready);
        assert_eq!(entry.os, "linux");
        assert!(is_interactive(world.machines.hosts_file(), "box-2"));
        assert!(world.machines.get("box-2").is_ok());
        // A new entry can't be made without the provider that reaches it.
        assert!(matches!(
            register(&world.machines, &MachineName::new("box-3").unwrap(), None, &link, platform),
            Err(LoginError::NotRegistrable { .. })
        ));
        unregister(&world.machines, &name, false).unwrap();
        assert!(world.machines.get("box-2").is_err());
        assert!(!is_interactive(world.machines.hosts_file(), "box-2"));
    }
}

#[cfg(test)]
mod quiet_failure_tests {
    use super::*;

    fn machine() -> MachineName {
        MachineName::new("box-1").unwrap()
    }

    #[test]
    fn a_login_failure_with_no_output_promises_none() {
        let quiet = [
            LoginError::Ended { machine: machine(), exit: Some(Exit::Code(1)), output_tail: String::new() },
            LoginError::Enrollment { machine: machine(), output_tail: String::new() },
            LoginError::Stalled { machine: machine(), waited: Duration::from_secs(90), output_tail: String::new() },
            LoginError::KeyGen { machine: "box-1".to_owned(), exit: Exit::Code(1), output_tail: String::new() },
            LoginError::Authorize { machine: "box-1".to_owned(), exit: Exit::Code(1), output_tail: String::new() },
            LoginError::StillNeedsLogin {
                machine: "box-1".to_owned(),
                exit: Exit::Code(255),
                output_tail: String::new(),
                undone: false,
            },
        ];
        for error in quiet {
            assert!(!error.to_string().contains("below"), "{error}");
        }
    }

    #[test]
    fn a_login_failure_with_output_still_says_where_the_output_is() {
        let error = LoginError::Ended {
            machine: machine(),
            exit: Some(Exit::Code(1)),
            output_tail: "login: permission denied\n".to_owned(),
        };
        assert!(error.to_string().contains("below"), "{error}");
    }
}
