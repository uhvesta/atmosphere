//! The views an agent offers, as the app sees them (VW-1 to VW-10, P7).
//!
//! Views are keyed by [`ViewId`]. Offers, Later and Journal entries reach the
//! app on the daemon's event feed ([`ViewsClient::watch`]): nothing polls.
//! Every answer the person gives is one `set_state`, which sends the daemon
//! a fresh idempotency key, so a retried call cannot answer twice.

use tokio::sync::mpsc;

use crate::daemon::Reply;
use crate::model::{FileTier, PreparedView, RepoDiffSpec, Scope, SessionId, ViewId, ViewState};

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
    /// A pull request draft, one entry per repository (VW-13).
    PullRequest(Vec<PullRequestDraftView>),
    /// How to review the session's changes.
    ReviewPlan(PlanView),
    /// Nothing the app knows beyond the summary.
    Summary,
}

/// One repository's pull request draft in a prepared pull request view (VW-13).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PullRequestDraftView {
    /// The repository, as `owner/name`.
    pub repo: String,
    /// The pull request's title.
    pub title: String,
    /// The pull request's body.
    pub body: String,
    /// The branch the pull request targets; empty for the repository's default branch.
    pub base_branch: String,
    /// Whether the pull request is opened as a draft.
    pub draft: bool,
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
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlanView {
    /// Each repository's base.
    pub targets: Vec<RepoDiffSpec>,
    /// Files as `(repo, path)`, in the order to read them.
    pub order: Vec<(String, String)>,
    /// Titled sections of the order.
    pub groups: Vec<PlanGroup>,
    /// Notes on files.
    pub notes: Vec<PlanNote>,
    /// The layers the change is told as (D800); `None` for a plain review.
    pub stack: Option<StackView>,
    /// The tiers the agent gave files (D802); a file it leaves out takes
    /// the tier its diff carries. For a stack, the layers' tiers joined.
    pub importance: Vec<PlanTier>,
}

/// One file's tier as the agent gave it (D802).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanTier {
    /// The repository, as `owner/name`.
    pub repo: String,
    /// The file's path in it.
    pub path: String,
    pub tier: FileTier,
}

/// A stack of layers, bottom first (D800).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StackView {
    /// The layers, bottom of the stack first.
    pub layers: Vec<LayerView>,
    /// Each repository's Whole stack diff: the union of the layers.
    pub whole: Vec<RepoDiffSpec>,
}

/// One layer of a stack.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LayerView {
    /// Its title, which names it everywhere.
    pub title: String,
    /// One line on what it is for; empty for none.
    pub purpose: String,
    /// The stack topic it was read from (D803); empty when the agent sent
    /// the layer itself.
    pub topic: String,
    /// The repositories it touches, in the agent's order.
    pub repos: Vec<LayerRepoView>,
    /// Files as `(repo, path)`, in the order to read them in this layer.
    pub order: Vec<(String, String)>,
    /// Titled sections of this layer's order.
    pub groups: Vec<PlanGroup>,
    /// The tiers the agent gave files in this layer (D802).
    pub importance: Vec<PlanTier>,
}

/// A layer's part in one repository.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayerRepoView {
    /// The repository, as `owner/name`.
    pub repo: String,
    /// What this layer shows of it.
    pub spec: RepoDiffSpec,
    /// Its pull request as `owner/repo#n`; empty for none.
    pub pull_request: String,
    /// How many files this layer changes in it; 0 when drifted.
    pub files: u32,
    /// Whether it moved since the agent sent it.
    pub drift: Option<LayerDrift>,
}

/// How a layer's part stopped matching what the agent sent (D800).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerDrift {
    /// A rev moved and the chain or ancestry broke.
    Moved,
    /// A rev is gone from the clone.
    Missing,
    /// HEAD left the top of the stack under an uncommitted layer.
    HeadMoved,
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
