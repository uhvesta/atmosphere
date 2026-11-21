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
    /// The lines changed since the comment was written (PR-16, PR-26): a
    /// note stays at its old lines, and a pull request draft can't be posted
    /// until it is re-attached or dropped.
    pub outdated: bool,
}

impl Anchor {
    /// Normalizes a 1-based inclusive line range `(first, last)`.
    ///
    /// - `(0, 0)` stays `(0, 0)` (whole-file comment).
    /// - Single-line backward compatibility: if `last == 0` and `first > 0`,
    ///   treats it as `(first, first)` (and symmetrically for `first == 0`).
    /// - Reversed ranges (`first > last`) are swapped to `(last, first)`.
    #[must_use]
    pub const fn normalized_range(first: u32, last: u32) -> (u32, u32) {
        if first == 0 && last == 0 {
            (0, 0)
        } else if last == 0 {
            (first, first)
        } else if first == 0 {
            (last, last)
        } else if first <= last {
            (first, last)
        } else {
            (last, first)
        }
    }

    /// Returns this anchor with `first` and `last` normalized.
    #[must_use]
    pub fn normalized(mut self) -> Self {
        let (first, last) = Self::normalized_range(self.first, self.last);
        self.first = first;
        self.last = last;
        self
    }

    /// The normalized `(first, last)` inclusive line range of this anchor.
    #[must_use]
    pub const fn line_range(&self) -> (u32, u32) {
        Self::normalized_range(self.first, self.last)
    }

    /// Whether this anchor spans more than one line (`first > 0 && last > first`).
    #[must_use]
    pub const fn is_multi_line(&self) -> bool {
        let (first, last) = self.line_range();
        first > 0 && last > first
    }

    /// Whether `line` on `side` falls inside this anchor's inclusive line range.
    #[must_use]
    pub const fn contains_line(&self, side: Side, line: u32) -> bool {
        let (first, last) = self.line_range();
        matches!((self.side, side), (Side::Old, Side::Old) | (Side::New, Side::New))
            && first > 0
            && line >= first
            && line <= last
    }

    /// Extends an existing range `(first, last)` to include `line`, returning a
    /// normalized `(start, end)` inclusive range.
    #[must_use]
    pub const fn extend_range(first: u32, last: u32, line: u32) -> (u32, u32) {
        let (lo, hi) = Self::normalized_range(first, last);
        if lo == 0 {
            return (line, line);
        }
        let start = if line < lo { line } else { lo };
        let end = if line > hi { line } else { hi };
        (start, end)
    }
}

/// Whether lines in repository `anchor` can carry a draft for a pull
/// request of `pull` (`owner/name`). Repositories are keyed `owner/name`,
/// or the bare name when the session's clone has no owner (D624, D732):
/// the same key matches, and so does a bare name equal to the pull
/// request's name part. Another owner's repository of that name does not.
#[must_use]
pub fn repo_holds_pull(anchor: &str, pull: &str) -> bool {
    anchor == pull || (!anchor.contains('/') && pull.rsplit_once('/').is_some_and(|(_, name)| name == anchor))
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
    /// A pull request draft that GitHub took (PR-14). Its link is
    /// [`Comment::posted_url`]; it is never edited or deleted here.
    Posted,
}

/// Where a comment goes (SM-9, PR-9, PR-23, D284).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Destination {
    /// Typed into the session's main agent (SM-9).
    #[default]
    Terminal,
    /// A note from the review chat, shown under its lines and never posted
    /// (PR-23). "Make it a draft" turns it into a pull request draft.
    Note,
    /// A draft for a pull request, posted to GitHub from Submit review
    /// (PR-9). Never typed into a terminal.
    PullRequest {
        /// The repository, `owner/name`.
        repo: String,
        /// The pull request's number in that repository.
        number: u32,
    },
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
    /// they were made against; an older one answers `DaemonError::Conflict`.
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
    /// Where it goes: the terminal, a note, or a pull request draft.
    pub destination: Destination,
    /// Who wrote it. The user's, unless the agent or the review chat wrote it.
    pub author: Author,
    /// For [`Status::Posted`], the link to the posted comment (PR-14).
    pub posted_url: Option<String>,
    /// A pull request draft whose submit was cut short after GitHub may have
    /// taken it. Still a draft; the user decides whether to post again
    /// (D287). Never set with [`Status::Posted`].
    pub post_unconfirmed: bool,
}

impl Comment {
    /// Whether it counts toward the session's draft badge (SM-15): a draft
    /// for the terminal that isn't archived. Notes and pull request drafts
    /// are never typed, so they are not counted.
    #[must_use]
    pub const fn counts_as_draft(&self) -> bool {
        matches!(self.status, Status::Draft)
            && !self.archived
            && matches!(self.destination, Destination::Terminal)
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

    /// Makes a draft for a pull request the session's review holds, as the
    /// user (PR-9). Kept apart from [`Self::create`] so the many callers that
    /// comment for the terminal stay as they are. `repo` is `owner/name`;
    /// the anchor names the same repository. Refused for a pull request the
    /// review doesn't hold. `key` makes a retry make nothing more.
    fn create_for_pull_request(
        &self,
        session: &SessionId,
        anchor: Anchor,
        text: &str,
        repo: &str,
        number: u32,
        key: &str,
    ) -> Reply<Comment>;

    /// Changes a draft's text. A `revision` that is not the comment's answers
    /// `DaemonError::Conflict`; a comment that is no longer a draft is refused.
    fn edit(&self, id: &CommentId, text: &str, revision: u64) -> Reply<Comment>;

    /// Deletes a comment for good (SM-7). Archive keeps it instead.
    fn drop_comment(&self, id: &CommentId) -> Reply<()>;

    /// Points a detached draft at a new passage (SM-7).
    fn reattach(&self, id: &CommentId, anchor: Anchor, revision: u64) -> Reply<Comment>;

    /// Turns a note into a draft for a pull request, in place: the same
    /// comment, its id, author and anchor kept (D284). `pr` names one of the
    /// review's pull requests. Without it the daemon takes the review's only
    /// pull request whose `owner/name` is the note's repository or whose name
    /// is its folder; with none, or several, it refuses and `pr` is needed.
    fn make_note_a_draft(&self, id: &CommentId, pr: Option<(&str, u32)>) -> Reply<Comment>;

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

    /// Sets a done comment (resolved, whoever resolved it, or addressed) back to draft (SM-11, D851).
    fn reopen(&self, id: &CommentId, revision: u64) -> Reply<Comment>;

    /// Hides a comment everywhere, its status kept (SM-20).
    fn archive(&self, id: &CommentId) -> Reply<Comment>;

    /// Brings an archived comment back as it was (SM-21).
    fn unarchive(&self, id: &CommentId) -> Reply<Comment>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchor_range_normalization_and_extension() {
        assert_eq!(Anchor::normalized_range(0, 0), (0, 0));
        assert_eq!(Anchor::normalized_range(10, 10), (10, 10));
        // Backward compatibility: single-line with last == 0.
        assert_eq!(Anchor::normalized_range(10, 0), (10, 10));
        assert_eq!(Anchor::normalized_range(0, 10), (10, 10));
        // Multi-line forward and reverse.
        assert_eq!(Anchor::normalized_range(10, 18), (10, 18));
        assert_eq!(Anchor::normalized_range(18, 10), (10, 18));

        // Extending a range from line 18 to line 10 normalizes to 10..=18.
        assert_eq!(Anchor::extend_range(18, 18, 10), (10, 18));
        assert_eq!(Anchor::extend_range(10, 18, 22), (10, 22));
        assert_eq!(Anchor::extend_range(10, 18, 5), (5, 18));

        let mut a = Anchor {
            repo: "api".into(),
            path: "src/lib.rs".into(),
            side: Side::New,
            first: 18,
            last: 10,
            quoted: Vec::new(),
            base: None,
            detached: false,
            outdated: false,
        };
        assert!(a.is_multi_line());
        assert!(a.contains_line(Side::New, 10));
        assert!(a.contains_line(Side::New, 14));
        assert!(a.contains_line(Side::New, 18));
        assert!(!a.contains_line(Side::New, 9));
        assert!(!a.contains_line(Side::Old, 14));

        a = a.normalized();
        assert_eq!((a.first, a.last), (10, 18));
    }
}
