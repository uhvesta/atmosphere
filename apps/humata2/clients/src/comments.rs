//! A session's comments for its agent, as the daemon keeps them (SM-6 to
//! SM-21, DK-9, RV-4 to RV-6, RV-27).
//!
//! One model serves every view that makes comments: the Diff panel, the
//! editor and Markdown source (SM-13). The daemon owns drafts, statuses,
//! threads and delivery; the app holds a copy that [`CommentsClient::watch`]
//! keeps current. [`FakeComments`] is the only implementation until the
//! live client serves CommentService.

use std::fmt;

use tokio::sync::mpsc;

use crate::daemon::Reply;
use crate::model::SessionId;

pub mod fake;

pub use fake::{Call, FakeComments, Typing};

/// A comment's id, `<session>/c-<n>`. Never reused, even after a drop.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CommentId(String);

impl CommentId {
    /// An id as the daemon gave it.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The id as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CommentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Which side of a comparison the lines are on. Comments outside a diff
/// (the editor, Markdown source) are on the new side: the file as it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    /// The base's lines.
    Old,
    /// The working tree's lines.
    New,
}

/// Where a comment points (SM-6, SM-13, RV-27).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Anchor {
    /// The repository, such as `api`. Every comment names one; the message
    /// shows it only when the session has more than one (SM-9).
    pub repo: String,
    /// The file inside the repository.
    pub path: String,
    /// The side of the comparison.
    pub side: Side,
    /// First line, 1-based.
    pub first: u32,
    /// Last line, inclusive; equal to `first` for one line.
    pub last: u32,
    /// The anchored lines as they were when the comment was made, one per
    /// line. Used to re-anchor when the file changes (SM-7).
    pub quoted: Vec<String>,
    /// For a comment made in a diff, the base as the user saw it named
    /// ("origin/main", "review 3"); `None` outside a diff.
    pub base: Option<String>,
    /// The passage is gone: the comment offers Re-attach or Drop (SM-7).
    pub detached: bool,
}

/// Where a comment is in its life (SM-11).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Status {
    /// Written, not sent. Survives restarts; never typed anywhere (SM-8).
    Draft,
    /// Humata is holding it because nothing could be typed: no terminal, or
    /// its shell exited (SM-10). See [`Comment::unconfirmed`].
    Waiting,
    /// The terminal took it (SM-10). Says nothing about what the agent did.
    Sent,
    /// A change after it was sent altered its lines (SM-16, SM-18).
    Addressed,
    /// Dealt with; hidden unless Show resolved is on (SM-12, SM-17).
    Resolved,
}

/// Who wrote a reply or resolved a comment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Author {
    /// The person using Humata.
    User,
    /// The session's agent, by name, such as `claude`.
    Agent(String),
}

/// One answer in a comment's thread, oldest first (DK-9, SM-19).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadReply {
    /// Unique within the comment.
    pub id: String,
    /// Who wrote it.
    pub author: Author,
    /// What it says.
    pub text: String,
}

/// A comment of the user's on the session's files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Comment {
    /// Its id.
    pub id: CommentId,
    /// The session it belongs to.
    pub session: SessionId,
    /// Where it points.
    pub anchor: Anchor,
    /// What it says.
    pub text: String,
    /// Where it is in its life.
    pub status: Status,
    /// Starts at 1 and goes up on every change. Edits name the revision
    /// they were made against; an older one is refused as stale.
    pub revision: u64,
    /// Waiting, and Humata can't tell whether an earlier try reached the
    /// terminal (it restarted mid-send). The user sends it again or takes it
    /// back; Humata never types it again on its own.
    pub unconfirmed: bool,
    /// Hidden everywhere and left out of every count, with its status kept
    /// for Unarchive (SM-20, SM-21).
    pub archived: bool,
    /// Its thread.
    pub replies: Vec<ThreadReply>,
    /// Who resolved it; set only while [`Status::Resolved`] (SM-19).
    pub resolved_by: Option<Author>,
    /// For [`Status::Addressed`], the anchored lines after the change; empty
    /// when they were deleted. `anchor.quoted` keeps them as they were
    /// (SM-18).
    pub addressed: Option<Vec<String>>,
}

impl Comment {
    /// Whether it counts toward the session's draft badge (SM-15).
    #[must_use]
    pub const fn counts_as_draft(&self) -> bool {
        matches!(self.status, Status::Draft) && !self.archived
    }
}

/// Which of a session's comments to list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Which {
    /// Every status but resolved, archived ones left out (SM-12).
    Open,
    /// Every status, resolved ones included, archived ones left out (Show
    /// resolved).
    WithResolved,
    /// Only archived ones, of any status (SM-21).
    Archived,
}

/// A session's comments and its badge.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Listing {
    /// The comments, in the order they were made.
    pub comments: Vec<Comment>,
    /// Drafts that aren't archived (SM-15).
    pub draft_count: u32,
}

/// What Send did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sent {
    /// The comments sent, as they are now.
    pub comments: Vec<Comment>,
    /// True only when Humata is holding them because nothing could be typed
    /// (SM-10); they show as waiting to send.
    pub queued: bool,
}

/// A change to a session's comments, from anyone: this app, another
/// window, the agent's MCP tools, the daemon re-anchoring.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommentEvent {
    /// A comment was made or changed; this is it now.
    Changed(Comment),
    /// A comment was dropped (SM-7, SM-21).
    Dropped(CommentId),
    /// Events were missed: list again.
    Resync,
}

/// The stream [`CommentsClient::watch`] returns.
pub type CommentEvents = mpsc::UnboundedReceiver<CommentEvent>;

/// What the app asks a daemon to do with a session's comments.
pub trait CommentsClient: Send + Sync + 'static {
    /// The session's comments.
    fn list(&self, session: &SessionId, which: Which) -> Reply<Listing>;

    /// Follows changes to the session's comments from now on.
    fn watch(&self, session: &SessionId) -> Reply<CommentEvents>;

    /// Makes a draft. `key` makes a retry of the same call make nothing more.
    fn create(&self, session: &SessionId, anchor: Anchor, text: &str, key: &str) -> Reply<Comment>;

    /// Changes a draft's text. Refused when `revision` is not the comment's.
    fn edit(&self, id: &CommentId, text: &str, revision: u64) -> Reply<Comment>;

    /// Deletes a comment for good (SM-7). Archive keeps it instead.
    fn drop_comment(&self, id: &CommentId) -> Reply<()>;

    /// Points a detached draft at a new passage (SM-7).
    fn reattach(&self, id: &CommentId, anchor: Anchor, revision: u64) -> Reply<Comment>;

    /// Types these drafts, or an unconfirmed batch again, into the session's
    /// main agent as one message (SM-9, SE-23). Other drafts stay drafts.
    fn send(&self, session: &SessionId, ids: &[CommentId]) -> Reply<Sent>;

    /// Waiting comments go back to drafts; for an unconfirmed batch, the
    /// whole batch (SE-23). Answers every comment that went back.
    fn take_back(&self, session: &SessionId, ids: &[CommentId]) -> Reply<Vec<Comment>>;

    /// Resolves a comment as the user (SM-11).
    fn resolve(&self, id: &CommentId, revision: u64) -> Reply<Comment>;

    /// Resolves every addressed comment of one file, or of the whole
    /// session when `file` is `None` (SM-18). Answers the ones it resolved.
    fn resolve_addressed(
        &self,
        session: &SessionId,
        file: Option<(&str, &str)>,
    ) -> Reply<Vec<Comment>>;

    /// Sets a resolved comment back to draft, whoever resolved it (SM-11).
    fn reopen(&self, id: &CommentId, revision: u64) -> Reply<Comment>;

    /// Hides a comment everywhere, its status kept (SM-20).
    fn archive(&self, id: &CommentId) -> Reply<Comment>;

    /// Brings an archived comment back as it was (SM-21).
    fn unarchive(&self, id: &CommentId) -> Reply<Comment>;
}
