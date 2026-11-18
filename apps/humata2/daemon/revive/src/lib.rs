//! Brings a machine's sessions back after their terminals' holders are
//! gone: after a reboot or power cut, or a holder killed on its own
//! (PER-11 to PER-17, PER-24).
//!
//! [`launch_agent`] saves how an agent is launched before it starts, so
//! there's always a record to resume from. [`reconcile`], run when the
//! daemon starts, goes through every saved agent and shell: a holder
//! still answering is reattached; an agent whose holder is gone is resumed
//! with its pinned conversation and the resume is checked, never passed
//! off as a fresh start; a shell comes back as a new shell in its folder.
//! Each restarted terminal starts with the output its old holder saved,
//! then a line saying what happened. Each reconcile writes a restart
//! report saying what it found and what became of it (PER-29).

mod agent;
mod error;
mod gate;
mod reconcile;
#[cfg(test)]
mod relaunch_tests;
#[cfg(test)]
mod resume_version_tests;
mod report;
mod shell_start;
mod watch;

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use humata2_store::TabRecord;

pub use agent::{launch_agent, launch_in_flight};
pub use error::{ContainRefused, ReviveError};
pub use gate::{GateRefusal, WriteGate};
pub use reconcile::{
    Reconciled, Revived, ShellRevived, reconcile, reconcile_then, restart_agent, restart_shell, restart_shells,
    resume_agent, resume_agent_asked, resume_moved_agent,
};
pub use report::RestartReports;

/// What reviving needs to know about this machine's daemon.
#[derive(Clone, Debug)]
pub struct ReviveConfig {
    /// The `humata2_holder` binary.
    pub holder: PathBuf,
    /// Where each holder writes what goes wrong, as `<tab>.log`.
    pub logs: PathBuf,
    pub scrollback_bytes: usize,
    /// The shell a restarted shell tab runs, and its arguments.
    pub shell: PathBuf,
    pub shell_args: Vec<OsString>,
    /// Where a machine shell, or one whose folder is gone, starts.
    pub home: PathBuf,
    /// How long a resumed agent has to show it took up its conversation.
    pub resume_timeout: Duration,
    /// How long a resumed agent's output stays quiet before it's told it
    /// was interrupted.
    pub settle: Duration,
    /// Where each reconcile writes its restart report (PER-29).
    pub reports: RestartReports,
    /// SEC-6: runs agents contained when their session's setting says so.
    /// `None` runs every agent as launched.
    pub contain: Option<std::sync::Arc<dyn Contain>>,
    /// D373: told of each holder this starts, so the daemon can say which
    /// holder file each one was spawned from. `None` records nothing.
    pub builds: Option<std::sync::Arc<dyn HolderBuilds>>,
    /// CV-3: asked for each agent's CLI version when the agent is
    /// resumed, which is kept in its launch record. `None` keeps the
    /// version the launch recorded.
    pub versions: Option<std::sync::Arc<dyn CliVersions>>,
    /// D763: asked before a resumed agent is told it was interrupted. The
    /// note waits while the agent asks a question of its own, and the
    /// resume's timeout waits with it. `None` tells it once it settles.
    pub gate: Option<std::sync::Arc<dyn WriteGate>>,
}

/// D373: what the daemon keeps of the holder file each holder was spawned
/// from. Called just before a holder is started, never when one is only
/// reattached, so what it kept says what is running. It can't fail the
/// start: an implementation that can't keep the record says so itself.
pub trait HolderBuilds: std::fmt::Debug + Send + Sync {
    /// A holder is about to start for `session`'s agent.
    fn agent_spawned(&self, session: &str);
    /// A holder is about to start for the shell tab `tab`.
    fn shell_spawned(&self, tab: &str);
}

/// What a [`CliVersions::current`] answer is, as a future.
pub type VersionAsked<'a> = std::pin::Pin<Box<dyn std::future::Future<Output = Option<String>> + Send + 'a>>;

/// CV-3: what a resume asks for the version of an agent's CLI, which may
/// have been upgraded since the agent was launched.
pub trait CliVersions: std::fmt::Debug + Send + Sync {
    /// What `agent`'s CLI says its version is now, as its version command
    /// prints it; None when the agent names no version command, isn't
    /// known, or its CLI doesn't answer in time.
    fn current<'a>(&'a self, agent: &'a str) -> VersionAsked<'a>;
}

/// SEC-6 (D217): what every agent launch and resume asks for the argv to
/// run, so a change to Contain agents applies at the next one. A session
/// that must run contained and can't is refused: it never runs uncontained
/// instead.
pub trait Contain: std::fmt::Debug + Send + Sync {
    /// The argv that runs `argv` for `record`'s session: contained when its
    /// channel or Settings says so, `argv` itself when not.
    fn wrap(
        &self,
        record: &humata2_store::AgentRecord,
        argv: &[OsString],
    ) -> Result<Vec<OsString>, ContainRefused>;
}

/// The variables a tab's program gets, such as its session token; the
/// store keeps only their names.
pub type Env<'a> = &'a (dyn Fn(&TabRecord) -> Vec<(OsString, OsString)> + Sync);
