//! Machine scripts (MACHINESCRIPTS, D680-D689): the run engine. A trigger
//! (a machine linked up, a session arrived or is leaving, or the owner's
//! "run now") runs the rules of `~/.humata2/config/machine-scripts.toml`
//! that apply, step by step, on the machine; each run is written to a
//! per-machine log, and a failure is reported to the Inbox and the feed.
//!
//! The engine does the ordering, queueing, timeouts and records. What a
//! step does on the machine is behind [`Steps`], and where a result goes
//! is behind [`Report`], so both are faked in the tests.
#![allow(dead_code)]

use std::future::Future;
use std::io;
use std::path::PathBuf;
use std::pin::Pin;
use std::time::Duration;

use humata2_machines::{ScriptBody, Trigger};
use serde::{Deserialize, Serialize};

mod engine;
mod live;
mod log;

pub use engine::{MachineScripts, RunNowError};
pub use live::{Parts, build, leaving};
pub use log::LogError;

/// A boxed future, as the crate's other object-safe async traits return.
pub type Fut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What fired the run: the machine, the trigger, and the session a session
/// trigger is about.
#[derive(Clone, Debug)]
pub struct Context {
    pub machine: String,
    pub trigger: Trigger,
    pub session: Option<SessionContext>,
}

/// The moved session a `session-arrived` or `session-leaving` run is about.
#[derive(Clone, Debug)]
pub struct SessionContext {
    pub session: String,
    pub channel: String,
    pub folder: String,
    pub from: String,
    pub to: String,
}

/// How a step, or a whole run, ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    Passed,
    Failed,
    TimedOut,
    /// Not run, because an earlier step of its rule failed.
    Skipped,
}

/// One step of a run, as recorded.
#[derive(Clone, Debug)]
pub struct StepRun {
    pub index: u32,
    /// "run", "humata", "start" or "send".
    pub kind: &'static str,
    pub label: String,
    pub outcome: Outcome,
    pub exit: Option<i32>,
    /// The last lines of output.
    pub tail: Vec<String>,
    pub error: Option<String>,
    pub took_ms: u64,
}

/// One run of one rule on one machine, as logged.
#[derive(Clone, Debug)]
pub struct RunRecord {
    pub id: String,
    pub machine: String,
    pub rule: String,
    pub trigger: Trigger,
    pub session: Option<String>,
    pub started_at_ms: i64,
    pub took_ms: u64,
    pub outcome: Outcome,
    pub steps: Vec<StepRun>,
}

/// What a script or CLI step produced on the machine.
pub struct ShellOutput {
    /// `None` when a signal ended it.
    pub exit: Option<i32>,
    pub timed_out: bool,
    pub tail: Vec<String>,
}

/// Why a step could not be carried out (as opposed to the script exiting
/// nonzero, which is a [`ShellOutput`]).
#[derive(Debug, thiserror::Error)]
pub enum StepError {
    #[error("can't read the script {path}: {source}. Check that the file is still in ~/.humata2/scripts and readable, then run the rule again")]
    ReadScript {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("there is no session to type into: put a `start` step before the `send`, or use `send` only on a session trigger")]
    NoSession,
    #[error("couldn't run on {machine}: {source}. Check that the machine is connected, then run the rule again")]
    Shell {
        machine: String,
        #[source]
        source: humata2_machines::ManageError,
    },
    #[error("couldn't start a {agent} session on {machine}: {message}. Check the agent kind and channel named in the rule")]
    Start {
        machine: String,
        agent: String,
        message: String,
    },
    #[error("session {session} did not become idle: {message}. If this agent kind can't be told apart from busy, set `wait = \"none\"` on the step")]
    WaitIdle { session: String, message: String },
    #[error("couldn't type into session {session}: {message}. Check that the session is still running, then run the rule again")]
    Send { session: String, message: String },
    #[error("{what} stopped before it finished: {message}. Run the rule again; if it keeps happening, restart Humata")]
    Interrupted { what: String, message: String },
}

/// The things a step does. The live implementation drives the machines and
/// the session service; tests use a fake.
pub trait Steps: Send + Sync {
    /// Runs a script (streamed to `bash -s`) or the machine's `humata` CLI.
    fn shell<'a>(
        &'a self,
        machine: &'a str,
        body: ScriptBody,
        args: Vec<String>,
        env: Vec<(String, String)>,
        timeout: Duration,
    ) -> Fut<'a, Result<ShellOutput, StepError>>;

    /// Starts a session of `agent` in `channel` on `machine`, once for
    /// `key`, and returns its id.
    fn start<'a>(
        &'a self,
        machine: &'a str,
        agent: &'a str,
        channel: &'a str,
        title: &'a str,
        key: &'a str,
    ) -> Fut<'a, Result<String, StepError>>;

    /// Waits for the session's turn to end.
    fn wait_idle<'a>(&'a self, session: &'a str, timeout: Duration) -> Fut<'a, Result<(), StepError>>;

    /// Types `text` into the session, once for `key`.
    fn send<'a>(&'a self, session: &'a str, text: &'a str, enter: bool, key: &'a str) -> Fut<'a, Result<(), StepError>>;
}

/// Where a run's result goes: the Inbox (one host ask per machine and rule)
/// and the feed.
pub trait Report: Send + Sync {
    /// Posts, or replaces, the host ask for this (machine, rule).
    fn failed<'a>(&'a self, run: &'a RunRecord) -> Fut<'a, ()>;
    /// Withdraws that ask, if there is one.
    fn passed<'a>(&'a self, run: &'a RunRecord) -> Fut<'a, ()>;
    /// The machine's run log changed.
    fn changed(&self, machine: &str);
}
