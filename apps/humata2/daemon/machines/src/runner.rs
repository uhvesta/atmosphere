//! A command to run, what it printed, and the trait that runs it, so setup
//! can be driven by the real process runner or by a test's fake.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use crate::error::RunError;

/// Why a command runs, so logs and fakes can tell them apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Purpose {
    /// A provider command (MP-2).
    Create,
    Identity,
    Start,
    Stop,
    Delete,
    Status,
    /// `uname -sm` over the link: the connect step.
    Probe,
    /// The machine check: os, arch, git, disk, service manager.
    Check,
    /// Streaming one file in.
    Install,
    /// Moving a staged file into place once no holder runs.
    Promote,
    /// Writing and starting the user service.
    Service,
    /// Stopping humatad2 and removing its service and files (cancelling
    /// Add machine, or removing a machine).
    Uninstall,
    /// Connecting without offering any credential, only to learn the host
    /// key a machine presents (MP-9, MP-10).
    ScanHostKey,
    /// Adding a Humata key to `authorized_keys` (MP-17).
    AuthorizeKey,
    /// The bridge that carries the gRPC stream (TR-2).
    Bridge,
    /// The link in a real terminal for an interactive login (MP-14).
    Login,
}

/// The value of one variable a command gets.
#[derive(Clone, PartialEq, Eq)]
pub enum EnvValue {
    /// A value Humata made, such as `HUMATA_NAME`.
    Value(String),
    /// The host variable with this name. Only the runner looks its value up,
    /// and it never writes it anywhere or shows it (MP-4, HO-7).
    HostVariable(String),
}

impl fmt::Debug for EnvValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Value(value) => write!(f, "{value:?}"),
            Self::HostVariable(name) => write!(f, "host variable {name}"),
        }
    }
}

/// A program, its arguments and exactly the variables it gets.
#[derive(Clone, PartialEq, Eq)]
pub struct Invocation {
    pub program: String,
    pub args: Vec<String>,
    /// Set after the baseline (`HOME`, `PATH`, …), so these win.
    pub env: Vec<(String, EnvValue)>,
    /// Written to stdin, which is then closed; none means `/dev/null`.
    pub stdin: Option<Arc<[u8]>>,
    /// None for a command that runs until it is stopped, such as the bridge.
    pub timeout: Option<Duration>,
    pub purpose: Purpose,
}

impl fmt::Debug for Invocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Invocation")
            .field("program", &self.program)
            .field("args", &self.args)
            .field("env", &self.env)
            .field("stdin_bytes", &self.stdin.as_ref().map(|bytes| bytes.len()))
            .field("timeout", &self.timeout)
            .field("purpose", &self.purpose)
            .finish()
    }
}

/// How a command ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    Code(i32),
    Signal(i32),
    /// It ran past its timeout and was killed with its process group.
    TimedOut,
}

impl Exit {
    pub fn success(self) -> bool {
        self == Self::Code(0)
    }
}

impl fmt::Display for Exit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Code(code) => write!(f, "exit code {code}"),
            Self::Signal(signal) => write!(f, "killed by signal {signal}"),
            Self::TimedOut => f.write_str("timed out"),
        }
    }
}

/// What a command printed, with host variable values already replaced by
/// their names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub exit: Exit,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl Output {
    /// The last `lines` lines of stdout then stderr, for a failed step's
    /// output (MP-20).
    pub fn tail(&self, lines: usize) -> String {
        let mut text = String::from_utf8_lossy(&self.stdout).into_owned();
        if !text.is_empty() && !text.ends_with('\n') && !self.stderr.is_empty() {
            text.push('\n');
        }
        text.push_str(&String::from_utf8_lossy(&self.stderr));
        let all: Vec<&str> = text.lines().collect();
        all[all.len().saturating_sub(lines)..].join("\n")
    }

    /// stdout and stderr together, for spotting prompts and SSH errors.
    pub(crate) fn text(&self) -> String {
        let mut text = String::from_utf8_lossy(&self.stdout).into_owned();
        text.push('\n');
        text.push_str(&String::from_utf8_lossy(&self.stderr));
        text
    }
}

/// Runs invocations. The daemon uses [`crate::SystemRunner`] for commands on
/// the Mac and its own forwarding runner for a provider's `run_on` machine
/// (MP-5); tests use a fake.
pub trait Runner {
    fn run(&self, invocation: &Invocation) -> Result<Output, RunError>;
}
