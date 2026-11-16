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
mod reconcile;
mod report;
mod watch;

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use humata2_store::TabRecord;

pub use agent::launch_agent;
pub use error::ReviveError;
pub use reconcile::{
    Reconciled, Revived, ShellRevived, reconcile, restart_agent, resume_agent,
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
}

/// The variables a tab's program gets, such as its session token; the
/// store keeps only their names.
pub type Env<'a> = &'a (dyn Fn(&TabRecord) -> Vec<(OsString, OsString)> + Sync);
