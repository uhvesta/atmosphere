//! What the pane needs from outside it: the agent's terminal, to send
//! comments to, and a store, to save each session's review in.

use std::future::Future;
use std::pin::Pin;

use crate::error::{DeliveryError, HistoryError, StoreError};
use crate::model::{Base, Review, ReviewSnapshot, Reviewed, SessionId, Typed};

/// A message to type into a session's agent terminal, as one bracketed
/// paste (SM-10). The terminal types each `key` at most once: asked again
/// with a key it already typed, it answers [`Typed::AlreadyTyped`] and
/// types nothing. Keys must stay known across restarts of either end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeRequest {
    pub session: SessionId,
    pub key: String,
    pub text: String,
}

/// What a terminal answers a [`TypeRequest`] with.
pub type TypeFuture = Pin<Box<dyn Future<Output = Result<Typed, DeliveryError>> + Send>>;

/// The agent terminal comments and questions are sent to: the session's
/// agent, or for a PR review its review chat (PR-22). The daemon's
/// `TerminalService.Type` with `when = NOW`, or a fake. It's called off the
/// UI thread. The pane only calls it when the terminal's last event said
/// the agent can take input, or the user pressed Send now.
pub trait CommentSink: Send + Sync + 'static {
    fn type_text(&self, request: TypeRequest) -> TypeFuture;
}

/// Where each session's review is saved, so comments, drafts and the send
/// queue survive a restart at any instant. Called off the UI thread.
pub trait ReviewStore: Send + Sync + 'static {
    /// Saves `snapshot` durably (written and synced) before returning. A
    /// snapshot older than the newest saved for its session is ignored, so
    /// saves that finish out of order keep the newest.
    fn save(&self, snapshot: &ReviewSnapshot) -> Result<(), StoreError>;

    /// The newest snapshot saved for `session`, if any.
    fn load(&self, session: &SessionId) -> Result<Option<ReviewSnapshot>, StoreError>;
}

/// What a [`ReviewHistory`] answers with.
pub type HistoryFuture<T> = Pin<Box<dyn Future<Output = Result<T, HistoryError>> + Send>>;

/// The session's reviewed snapshots and the changes against any of them
/// (RV-28–RV-30): the daemon's review service, or a fake. Called off the
/// UI thread.
pub trait ReviewHistory: Send + Sync + 'static {
    /// The snapshots kept, newest first.
    fn reviewed(&self, session: &SessionId) -> HistoryFuture<Vec<Reviewed>>;

    /// Records exactly what the user sees now as a new reviewed snapshot.
    fn mark_reviewed(&self, session: &SessionId) -> HistoryFuture<Reviewed>;

    /// The session's changes against `base`.
    fn changes(&self, session: &SessionId, base: Base) -> HistoryFuture<Review>;
}
