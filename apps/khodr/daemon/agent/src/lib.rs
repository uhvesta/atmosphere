//! Khodr's agent interface.
//!
//! Every backend (Copilot, a local model, the scripted one tests use)
//! implements [`Agent`], so nothing above it knows which one runs. The
//! interface is deliberately narrow: start, send, approve, abort, fork, read
//! and watch history.
//!
//! [`LocalAgent`] implements it in process for any [`Runner`], which only has
//! to play one turn. It keeps each session's history as JSON lines on disk,
//! queues and steers turns, holds approvals and forks.

mod copilot;
mod error;
mod event;
mod local_model;
mod log;
mod paths;
mod runtime;
mod scripted;

use std::future::Future;

use tokio::sync::broadcast;

pub use copilot::{COPILOT_VERSION, CopilotError, CopilotOptions, CopilotProvider, CopilotRunner};
pub use error::AgentError;
pub use event::{
    ApprovalMode, Event, EventId, EventKind, ForkOptions, Parent, PermissionKind, SendMode,
    SessionId, SessionRecord, SessionState, StartOptions,
};
pub use local_model::{DEFAULT_BASE_URL, LocalModelConfig, LocalModelError, LocalModelRunner};
pub use runtime::{LocalAgent, Runner, Turn};
pub use scripted::{Rule, Script, ScriptedError, ScriptedRunner, Step};

/// A session's history so far, and every event after it.
#[derive(Debug)]
pub struct Watch {
    pub history: Vec<Event>,
    /// Lags, and then reports [`broadcast::error::RecvError::Lagged`], if the
    /// reader falls far behind. Watch again to catch up.
    pub live: broadcast::Receiver<Event>,
}

/// A coding agent that runs sessions.
pub trait Agent: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Starts a session, idle until the first message.
    fn start(
        &self,
        options: StartOptions,
    ) -> impl Future<Output = Result<SessionRecord, Self::Error>> + Send;

    /// Starts a turn with `text`, queues it after the running one, or steers
    /// the running one, as `mode` says.
    fn send(
        &self,
        session: &SessionId,
        text: String,
        mode: SendMode,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// Answers the permission request `request_id`.
    fn approve(
        &self,
        session: &SessionId,
        request_id: &str,
        approved: bool,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// Sets what the session may do without asking, from now on. Requests
    /// waiting for an answer that the new mode allows go ahead.
    fn set_approval_mode(
        &self,
        session: &SessionId,
        mode: ApprovalMode,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// Stops the running turn, if any. Queued messages still run.
    fn abort(&self, session: &SessionId) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// A new session with the history of `session` before `options.before`.
    fn fork(
        &self,
        session: &SessionId,
        options: ForkOptions,
    ) -> impl Future<Output = Result<SessionRecord, Self::Error>> + Send;

    fn history(
        &self,
        session: &SessionId,
    ) -> impl Future<Output = Result<Vec<Event>, Self::Error>> + Send;

    fn watch(&self, session: &SessionId)
    -> impl Future<Output = Result<Watch, Self::Error>> + Send;

    fn sessions(&self) -> impl Future<Output = Result<Vec<SessionRecord>, Self::Error>> + Send;
}
