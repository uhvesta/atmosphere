//! The views an agent offers, as the app sees them (VW-1 to VW-10, P7).
//!
//! Views are keyed by [`ViewId`]. Offers, Later and Journal entries reach the
//! app on the daemon's event feed ([`ViewsClient::watch`]): nothing polls.
//! Every answer the person gives is one `set_state`, which sends the daemon
//! a fresh idempotency key, so a retried call cannot answer twice.

use tokio::sync::mpsc;

use crate::daemon::Reply;
use crate::model::{PreparedView, RepoDiffSpec, Scope, SessionId, ViewId, ViewState};

pub use crate::daemon::FakeViews;

/// A prepared view with its id and where it stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewRecord {
    /// Its id.
    pub id: ViewId,
    /// What the offer shows.
    pub view: PreparedView,
    /// Where the person has left it.
    pub state: ViewState,
    /// Goes up with every change to the view, from any client.
    pub revision: u64,
    /// The agent's name as shown ("Claude Code"); empty when unknown.
    pub prepared_by: String,
    /// What the view holds.
    pub content: ViewContent,
}

/// What a prepared view holds, as far as the app knows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewContent {
    /// Files to walk through, in order.
    Walkthrough(Vec<WalkthroughStep>),
    /// A board to show.
    Board(Scope),
    /// A session's board, with its forks' items when `with_forks`. The app
    /// finds the session's channel; a session it does not know shows as a
    /// summary.
    SessionBoard {
        /// The session.
        session: SessionId,
        /// Whether its forks' items show too.
        with_forks: bool,
    },
    /// A pull request draft; the app shows the summary only.
    PullRequest,
    /// How to review the session's changes.
    ReviewPlan(PlanView),
    /// Nothing the app knows beyond the summary.
    Summary,
}

/// One stop of a walkthrough.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WalkthroughStep {
    /// The file's repository, as `owner/name`.
    pub repo: String,
    /// The file, relative to the repository's root.
    pub path: String,
    /// What the agent says about it.
    pub note: String,
}

/// A review plan: what to compare, in what order, with what said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanView {
    /// Each repository's base.
    pub targets: Vec<RepoDiffSpec>,
    /// Files as `(repo, path)`, in the order to read them.
    pub order: Vec<(String, String)>,
    /// Titled sections of the order.
    pub groups: Vec<PlanGroup>,
    /// Notes on files.
    pub notes: Vec<PlanNote>,
}

/// A titled section of a plan's order, from `first` until the next group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanGroup {
    /// Its title.
    pub title: String,
    /// What the agent says about it.
    pub note: String,
    /// Its first file as `(repo, path)`.
    pub first: (String, String),
}

/// A note on one file of a plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanNote {
    /// The file's repository, as `owner/name`.
    pub repo: String,
    /// The file.
    pub path: String,
    /// The note.
    pub text: String,
    /// The lines it is on, first and last, where they are now; `None` for a
    /// note on the whole file. A hunk's note carries the hunk's line.
    pub lines: Option<(u32, u32)>,
    /// Its lines changed since the agent wrote it: the note is kept, shown
    /// at the lines it had.
    pub outdated: bool,
}

/// The stream [`ViewsClient::watch`] returns: a view as it is after each
/// change, from any client, this one included.
pub type ViewEvents = mpsc::Receiver<ViewRecord>;

/// What the app asks the daemon about views.
pub trait ViewsClient: Send + Sync + 'static {
    /// The views the session's agent has offered, in any state.
    fn list(&self, session: &SessionId) -> Reply<Vec<ViewRecord>>;

    /// Every session's views the person can see (anything but a draft),
    /// newest first.
    fn list_offered(&self) -> Reply<Vec<ViewRecord>>;

    /// Answers a view: moves it to `state`. Each call carries a fresh
    /// idempotency key, one per user action. Answers with the view after the
    /// change.
    fn set_state(&self, id: &ViewId, state: ViewState) -> Reply<ViewRecord>;

    /// Follows views from now on: each offer and each change, as it happens.
    /// The stream is open before this answers, so a change made after it is
    /// not missed.
    fn watch(&self) -> Reply<ViewEvents>;
}
