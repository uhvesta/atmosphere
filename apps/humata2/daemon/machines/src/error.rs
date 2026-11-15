//! What can go wrong, each error saying what failed and how to fix it.

use std::fmt;
use std::io;
use std::time::Duration;

use thiserror::Error;

use crate::install::{InstallProblem, InstalledFile};
use crate::name::MachineName;
use crate::platform::Platform;
use crate::provider::Action;
use crate::runner::Exit;

/// A machine name that can't be used.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum NameError {
    #[error("a machine needs a name; type one such as `ci-box-1`")]
    Empty,
    #[error(
        "the machine name {value:?} is {len} characters long; shorten it to at most 63"
    )]
    TooLong { value: String, len: usize },
    #[error(
        "the machine name {value:?} contains {character:?}; use only lowercase letters, digits and `-`"
    )]
    BadCharacter { value: String, character: char },
    #[error(
        "the machine name {value:?} starts or ends with `-`; start and end it with a letter or a digit"
    )]
    BadEdge { value: String },
}

/// A value no shell can carry.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{what} contains a NUL byte, which no shell can pass on; remove it")]
pub struct QuoteError {
    pub what: String,
}

/// The providers file can't be used as written.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error(
        "the providers file isn't valid TOML at line {line}: {message}. Fix that line in Settings › Machine providers"
    )]
    Syntax {
        line: usize,
        message: String,
        #[source]
        source: toml::de::Error,
    },
    #[error("{0}")]
    Problems(Problems),
}

/// Everything wrong with a providers file that parsed, so the user can fix
/// it in one pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problems(pub Vec<Problem>);

impl fmt::Display for Problems {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the providers file needs fixing in Settings › Machine providers:")?;
        for problem in &self.0 {
            write!(f, "\n- {problem}")?;
        }
        Ok(())
    }
}

/// One thing wrong with one provider, at the field (and line) to change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub provider: String,
    /// The field to change, such as `connect` or `params[1].name`.
    pub field: String,
    pub line: Option<usize>,
    pub kind: ProblemKind,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "providers.{}", self.provider)?;
        if !self.field.is_empty() {
            write!(f, ".{}", self.field)?;
        }
        if let Some(line) = self.line {
            write!(f, " (line {line})")?;
        }
        write!(f, ": {}", self.kind)
    }
}

/// What is wrong with a provider field.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProblemKind {
    #[error(
        "the provider name must be lowercase letters, digits and `-`, starting with a letter; rename the table"
    )]
    BadProviderName,
    #[error("is missing; add the command that makes a machine, such as `create = \"mycloud create {{name}}\"`")]
    MissingCreate,
    #[error(
        "is missing; add `connect` (a command that runs `{{cmd}}` on the machine) or `identity` (a command that prints its SSH details as JSON)"
    )]
    MissingLink,
    #[error("is set along with `identity`; keep only one of the two")]
    BothLinks,
    #[error("must contain `{{cmd}}`, where Humata puts the command it runs on the machine")]
    ConnectWithoutCmd,
    #[error("uses `{{cmd}}`, which only `connect` can use; remove it")]
    CmdOutsideConnect,
    #[error("is empty; write the command, or remove the field")]
    EmptyCommand,
    #[error("contains a NUL byte, which no shell can run; remove it")]
    Nul,
    #[error(
        "`{name}` isn't a param name; use lowercase letters, digits and `_`, starting with a letter"
    )]
    BadParamName { name: String },
    #[error("`{name}` is declared twice; remove one")]
    DuplicateParam { name: String },
    #[error("`name` is reserved for the machine's name (`{{name}}` and `HUMATA_NAME`); rename the param")]
    ReservedParam,
    #[error(
        "`{name}` isn't a variable name; use uppercase letters, digits and `_`, not starting with a digit"
    )]
    BadEnvName { name: String },
    #[error(
        "`{name}` is set by Humata itself, so a host variable can't replace it; list a different name"
    )]
    ReservedEnvName { name: String },
    #[error("`{name}` is listed twice; remove one")]
    DuplicateEnv { name: String },
}

/// The values typed into Add machine don't fit the provider's params.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ValuesError {
    #[error("the {provider} provider needs “{prompt}” ({param}) and it has no default; fill it in")]
    Missing { provider: String, param: String, prompt: String },
    #[error(
        "the {provider} provider has no param `{param}`; remove it, or declare it in the provider's `params`"
    )]
    Undeclared { provider: String, param: String },
    #[error("the value of `{param}` contains a NUL byte, which no shell can pass on; remove it")]
    Nul { param: String },
}

/// The identity command's output can't be used (MP-8, MP-11).
#[derive(Debug, Error)]
pub enum IdentityError {
    #[error(
        "the identity command printed key material. It must print paths only (`ssh.identity_file`, `ssh.ssh_config`), never a key; nothing it printed was kept or shown. Change the identity command in Settings › Machine providers"
    )]
    KeyMaterial,
    #[error(
        "the identity command didn't print a JSON object ({source}); make it print one, for example with `jq -n`, and print anything else to stderr"
    )]
    NotJson {
        #[source]
        source: serde_json::Error,
    },
    #[error(
        "identity field `{field}` {problem}; fix the identity command in Settings › Machine providers"
    )]
    Field { field: String, problem: FieldProblem },
}

/// What is wrong with one identity field.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FieldProblem {
    #[error("is missing")]
    Missing,
    #[error("must be a string")]
    NotString,
    #[error("must be an object")]
    NotObject,
    #[error("must be a port number from 1 to 65535")]
    NotPort,
    #[error("isn't a field Humata knows; remove it, or check its spelling")]
    Unknown,
    #[error("is {found:?}, but this machine is {expected:?}; print the machine's own name")]
    WrongName { expected: String, found: String },
    #[error("is empty")]
    Empty,
    #[error("must not contain spaces or control characters, or start with `-`")]
    BadText,
    #[error("can't be given with `ssh.ssh_config`; put it in that file, or drop `ssh_config`")]
    ConflictsWithConfig,
    #[error("must be a public host key such as `ssh-ed25519 AAAA…`")]
    BadHostKey,
    #[error("is {value:?}; Humata knows `linux` and `darwin`")]
    UnknownOs { value: String },
    #[error("is {value:?}; Humata knows `x86_64` and `aarch64` (or `amd64`, `arm64`)")]
    UnknownArch { value: String },
}

/// A command couldn't be run at all.
#[derive(Debug, Error)]
pub enum RunError {
    #[error("couldn't start `{program}`: {source}. Check that it is installed and on PATH")]
    Spawn {
        program: String,
        #[source]
        source: io::Error,
    },
    #[error("lost track of `{program}` while it ran: {source}. Try again")]
    Wait {
        program: String,
        #[source]
        source: io::Error,
    },
    #[error(
        "the provider lists host variable `{name}`, which isn't set; set it in Settings › This Mac › Variables, then try again"
    )]
    MissingHostVariable { name: String },
}

/// A step of setting a machine up failed (MP-19, MP-20).
#[derive(Debug, Error)]
pub enum SetupError {
    #[error(
        "the provider's `{action}` command failed ({exit}); its output is below. Fix what it reports and try again, or change the command in Settings › Machine providers"
    )]
    CommandFailed { action: Action, exit: Exit },
    #[error(transparent)]
    Values(#[from] ValuesError),
    #[error(transparent)]
    Identity(#[from] IdentityError),
    #[error(transparent)]
    Quote(#[from] QuoteError),
    #[error(transparent)]
    Run(#[from] RunError),
    #[error(
        "couldn't reach {machine} after {attempts} attempts (last: {exit}); the output is below. If it is still booting, try again; if it asks for a login, open it interactively"
    )]
    Unreachable { machine: MachineName, attempts: u32, exit: Exit },
    #[error(
        "a command that reaching {machine} needs wasn't found (exit 127); the output names it. Install it on this Mac (or on the machine), or fix the provider's `connect`"
    )]
    ToolMissing { machine: MachineName },
    #[error(
        "{machine}'s SSH host key isn't one Humata knows. Check its fingerprint and confirm it, or add `ssh.host_key` to the identity output so Humata can pin it"
    )]
    HostKeyUnknown { machine: MachineName },
    #[error(
        "{machine}'s SSH host key changed since Humata pinned it. That happens when the machine was rebuilt, and also when something intercepts the connection; confirm the new key only if you know it was rebuilt"
    )]
    HostKeyChanged { machine: MachineName },
    #[error(
        "{machine} answered, but `uname -sm` printed nothing Humata can read; make sure `connect` runs `{{cmd}}` unchanged and that nothing else prints to stdout"
    )]
    BadProbe { machine: MachineName },
    #[error(
        "the machine check on {machine} failed ({exit}) or printed nothing Humata can read; the output is below"
    )]
    CheckFailed { machine: MachineName, exit: Exit },
    #[error(
        "Humata 2 sets up Linux machines only (x86_64 or aarch64); {machine} is {platform}. Use a Linux machine"
    )]
    Unsupported { machine: MachineName, platform: Platform },
    #[error(
        "the identity output says {machine} is {said}, but it reports {found}; fix `os` and `arch` in the identity command"
    )]
    IdentityMismatch { machine: MachineName, said: Platform, found: Platform },
    #[error(
        "git isn't installed on {machine}; install it there (for example `sudo apt-get install git`), then try again"
    )]
    GitMissing { machine: MachineName },
    #[error(
        "{machine} has {free_mib} MiB free in its home folder and Humata needs {need_mib} MiB; free some space or grow the disk, then try again"
    )]
    DiskLow { machine: MachineName, free_mib: u64, need_mib: u64 },
    #[error(
        "{machine} has neither systemd user services nor `setsid` and `crontab`, so humatad2 can't keep running there; install systemd or cron there, or use another machine"
    )]
    NoServiceManager { machine: MachineName },
    #[error("this Humata has no humatad2 build for {platform}; use a machine of a supported platform")]
    NoBuild { platform: Platform },
    #[error("installing {file} on {machine} failed: {problem}")]
    Install { machine: MachineName, file: InstalledFile, problem: InstallProblem },
    #[error(
        "{machine} won't let {user} keep services running after logout (linger). An admin must run this once on {machine}: `{admin_command}`, then try again. Or start humatad2 without systemd, which won't restart it if it crashes"
    )]
    LingerUnavailable { machine: MachineName, user: String, admin_command: String },
    #[error(
        "setting up the humatad2 service on {machine} failed ({exit}); the output is below. Fix what it reports and try again"
    )]
    ServiceFailed { machine: MachineName, exit: Exit },
    #[error(
        "humatad2 didn't stay running on {machine}; the output below has its last log lines. Fix what it reports and try again"
    )]
    ServiceDidNotStart { machine: MachineName },
}

/// Taking over an interactive login failed (MP-14). The terminal stays the
/// person's: they can keep using it, or close it and try again.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TakeoverError {
    #[error(
        "after the login, the machine's shell answered {line:?}, which Humata can't read; make sure the login ends at a POSIX shell with nothing rewriting its output, then open it interactively again"
    )]
    BadAnswer { line: String },
    #[error("this Humata has no humatad2 build for {platform}; use a machine of a supported platform")]
    NoBuild { platform: Platform },
    #[error("installing {file} over the login failed: {problem}")]
    Install { file: InstalledFile, problem: InstallProblem },
    #[error(
        "humatad2 on {machine} didn't say it was ready within {}s; the end of the terminal's output is below. Check that humatad2 is installed at ~/.humata2/bin/humatad2 on {machine} and runs: log in over ssh and run the line Humata typed by hand to see its error. If the machine was only slow, for example starting its daemon, open it interactively again",
        .waited.as_secs_f32()
    )]
    SyncTimedOut { machine: MachineName, waited: Duration, output_tail: String },
    #[error(
        "the terminal logged in to {machine} ended{} before the takeover finished; the end of its output is below. If humatad2 wasn't found or failed, check ~/.humata2/bin/humatad2 on {machine}: log in over ssh and run the line Humata typed by hand to see its error. Then open it interactively again",
        .exit.map(|how| format!(" ({how})")).unwrap_or_default()
    )]
    TerminalEnded { machine: MachineName, exit: Option<Exit>, output_tail: String },
    #[error(transparent)]
    Quote(#[from] QuoteError),
}

/// What the person can do about a failure (MP-20). The daemon turns each
/// into a button.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Remedy {
    /// Run the failed step again.
    TryAgain,
    /// Change the provider in Settings › Machine providers.
    FixProvider,
    /// Log in through a real terminal (MP-12).
    OpenInteractively,
    /// Show the host key's fingerprint and let the user trust it.
    ConfirmHostKey,
    /// Show an admin the exact command to run once on the machine.
    RunAsAdmin { command: String },
    /// Run humatad2 with `setsid` and a crontab entry instead (PER-19).
    StartWithoutSystemd,
    /// Run the provider's `delete` for what `create` made.
    DeleteWhatWasMade,
}
