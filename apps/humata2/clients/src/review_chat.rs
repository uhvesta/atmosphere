//! The review chat's view of a review (PR-22 to PR-26, D59, D283, D288): what
//! the app reports on screen, the agent's changes to it with their undo, and
//! the notes the agent leaves for the person.
//!
//! The app reports the view as it is (debounced while the person scrolls) and
//! follows the agent's changes on a feed, applying each once the person is
//! quiet. The agent never reviews on its own: it answers, and it moves the
//! view and leaves notes. [`ReviewChatClient`] is the app's side of that.

use tokio::sync::mpsc;

use crate::comments::{Anchor, CommentId, Side};
use crate::daemon::Reply;
use crate::model::{RepoDiffSpec, SessionId};
use crate::views::{PlanGroup, PlanView, StackView};



/// Inclusive, 1-based lines of a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LineSpan {
    /// The first line.
    pub first: u32,
    /// The last line, inclusive.
    pub last: u32,
}

/// Where the person (or the agent) is in a review: the file open, the lines on
/// screen and any selection (PR-23).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    /// The file's repository, as `owner/name`.
    pub repo: String,
    /// The file, relative to the repository's root.
    pub path: String,
    /// The lines on screen, when known.
    pub visible: Option<LineSpan>,
    /// The lines selected, when any.
    pub selection: Option<LineSpan>,
    /// The side of the comparison the lines are on.
    pub side: Side,
}

/// A file folded as viewed at this content hash (RV-29).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewedFile {
    /// The file's repository, as `owner/name`.
    pub repo: String,
    /// The file.
    pub path: String,
    /// The blob it was folded at.
    pub blob_sha: String,
}

/// A labelled range the agent highlighted (PR-23).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Highlight {
    /// The file's repository, as `owner/name`.
    pub repo: String,
    /// The file.
    pub path: String,
    /// The lines, when the highlight names them.
    pub lines: Option<LineSpan>,
    /// The side of the comparison.
    pub side: Side,
    /// What the highlight says, in a few words.
    pub label: String,
}

/// What the person is looking at, as the app reports it (PR-22, RV-31 to
/// RV-34).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReviewView {
    /// Each repository's base, as its diff shows it.
    pub targets: Vec<RepoDiffSpec>,
    /// The file open, when one is.
    pub open: Option<Place>,
    /// The order to read the files in, as `(repo, path)`.
    pub order: Vec<(String, String)>,
    /// Titled sections of the order.
    pub groups: Vec<PlanGroup>,
    /// Files folded as viewed.
    pub viewed: Vec<ViewedFile>,
    /// What the agent highlighted.
    pub highlights: Vec<Highlight>,
    /// The stack the review shows (D800); `None` for a plain review.
    pub stack: Option<StackView>,
    /// The layer showing, by title; `None` for Whole stack.
    pub layer: Option<String>,
}

/// A note the agent left for the person, shown under its lines and never
/// posted (PR-23). Outdated when its lines changed since: its anchor says
/// so (PR-26).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    /// Its id in the comment ledger.
    pub id: CommentId,
    /// Its lines, as they were when written.
    pub anchor: Anchor,
    /// What it says.
    pub text: String,
}

/// What the agent may do to a review's view, each switch on or off by the
/// person (PR-25). All on by default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgentSwitches {
    /// The agent may move the view to a file and its lines (`review_focus`).
    pub focus: bool,
    /// The agent may leave, edit and drop notes (`review_note*`).
    pub notes: bool,
    /// The agent may reorder the files (`review_prepare`).
    pub order: bool,
}

impl Default for AgentSwitches {
    fn default() -> Self {
        Self {
            focus: true,
            notes: true,
            order: true,
        }
    }
}

/// What the agent reads when it asks what the person is looking at
/// (`review_context`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewContext {
    /// The view as the app last reported it.
    pub view: ReviewView,
    /// The quoted lines of the selection.
    pub selected_text: Vec<String>,
    /// The notes for the person, outdated ones included.
    pub notes: Vec<Note>,
    /// The switches as the person set them.
    pub switches: AgentSwitches,
}

/// What one agent change to the view is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReviewChangeKind {
    /// The view moved to a file and its lines.
    Focus(Place),
    /// A note was left.
    Note(Note),
    /// A note was dropped.
    NoteDropped(CommentId),
    /// The plan of how to review was prepared (`review_prepare`).
    Plan(PlanView),
    /// The view moved to a stack layer (D800); `None` is Whole stack.
    Layer(Option<String>),
}

/// One change to the review's view, as the app applies it and the chat shows
/// it ("● review_focus refund.rs:140 [Undo]", PR-24).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewChange {
    /// The id the chat's Undo names; empty when the change can't be undone (a
    /// change the code made, D88).
    pub undo_id: String,
    /// Increases per review; the feed resumes after it.
    pub seq: u64,
    /// The tool that made it: `review_focus`, `review_note`, `review_note_edit`,
    /// `review_note_drop`, `review_prepare`, `undo`, or `diff`.
    pub tool: String,
    /// What the chat line says, such as `refund.rs:140`.
    pub summary: String,
    /// What it changed, when it changed something the app shows.
    pub change: Option<ReviewChangeKind>,
    /// The highlights it set.
    pub highlights: Vec<Highlight>,
    /// For `undo`, the undo id of the change it took back; else empty.
    pub undid: String,
    /// When it was made, in Unix milliseconds.
    pub at_unix_ms: i64,
}

/// What the agent is asked to focus (`review_focus`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FocusRequest {
    /// The file's repository, as `owner/name`.
    pub repo: String,
    /// The file.
    pub path: String,
    /// The lines to show; `None` for the top of the file.
    pub lines: Option<LineSpan>,
    /// The side of the comparison.
    pub side: Side,
    /// Highlights to set; empty clears them.
    pub highlights: Vec<Highlight>,
    /// The stack layer to show, by title; `None` keeps the layer showing.
    pub layer: Option<String>,
    /// Show the Whole stack (D800); wins over `layer`.
    pub whole_stack: bool,
}

/// The stream [`ReviewChatClient::watch_changes`] returns.
pub type ChangeEvents = mpsc::UnboundedReceiver<ReviewChange>;

/// What the app asks of a review's chat.
pub trait ReviewChatClient: Send + Sync + 'static {
    /// What the person is looking at, the notes and the switches (PR-22). Never
    /// gated by the switches.
    fn context(&self, session: &SessionId) -> Reply<ReviewContext>;

    /// Reports the view as it is now (PR-22). Debounced by the caller while
    /// the person scrolls.
    fn report_view(&self, session: &SessionId, view: ReviewView) -> Reply<()>;

    /// Moves the view to a file and its lines, as the agent would. Refused,
    /// with a fix, when the Focus switch is off (PR-25).
    fn focus(&self, session: &SessionId, request: FocusRequest) -> Reply<ReviewChange>;

    /// Takes back the change `undo_id` made (PR-24). Refused when it was
    /// undone already or can't be undone.
    fn undo(&self, session: &SessionId, undo_id: &str) -> Reply<ReviewChange>;

    /// Follows the agent's changes after `after_seq`, in order, as they happen
    /// (PR-24). The stream is open before this answers.
    fn watch_changes(&self, session: &SessionId, after_seq: u64) -> Reply<ChangeEvents>;

    /// Sets what the agent may do (PR-25); answers the switches as they are.
    fn set_switches(&self, session: &SessionId, switches: AgentSwitches) -> Reply<AgentSwitches>;
}
