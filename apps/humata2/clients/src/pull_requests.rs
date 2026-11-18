//! Pull request reviews as the app sees them (PR-1 to PR-17, A9, D283 to
//! D300): a pasted pull request, a review session of one or more of them,
//! the heads GitHub has moved to, the drafts a review posts and the commits
//! marked reviewed.
//!
//! The daemon owns GitHub: it asks the host's own `gh`, keeps the review's
//! clones and records what was posted. [`PullRequestsClient`] is the app's
//! view of that. Head changes arrive on a feed
//! ([`PullRequestsClient::watch_heads`]) and are never polled for.
//! [`FakePullRequests`] is the scripted stand-in the features test against.

use std::fmt;
use std::time::Duration;

use tca::BoxFuture;
use tokio::sync::mpsc;

use crate::comments::{Anchor, Author, CommentId};
use crate::daemon::DaemonError;
use crate::model::{Agent, MachineId, SessionId};

pub mod fake;

pub use fake::{Call, FakePullRequests};

/// A pull request, by its repository (`owner/name`) and number.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PullRequestRef {
    /// The repository, as `owner/name`.
    pub repo: String,
    /// The pull request's number.
    pub number: u32,
}

impl PullRequestRef {
    /// A pull request of `repo` (`owner/name`) with `number`.
    #[must_use]
    pub fn new(repo: impl Into<String>, number: u32) -> Self {
        Self {
            repo: repo.into(),
            number,
        }
    }

    /// Reads a pasted link (`https://github.com/acme/api/pull/482`, with or
    /// without the scheme, `www.`, and anything after the number) or
    /// `acme/api#482`. `None` for anything else.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let url = text
            .strip_prefix("https://")
            .or_else(|| text.strip_prefix("http://"));
        let Some(rest) = url else {
            let (repo, number) = text.split_once('#')?;
            let (owner, name) = repo.split_once('/')?;
            return Self::from_parts(owner, name, number.parse().ok()?);
        };
        let rest = rest.strip_prefix("www.").unwrap_or(rest);
        let mut parts = rest.strip_prefix("github.com/")?.split('/');
        let (owner, name) = (parts.next()?, parts.next()?);
        if parts.next()? != "pull" {
            return None;
        }
        let digits: String = parts.next()?.chars().take_while(char::is_ascii_digit).collect();
        Self::from_parts(owner, name, digits.parse().ok()?)
    }

    fn from_parts(owner: &str, name: &str, number: u32) -> Option<Self> {
        let plain = |part: &str| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        };
        (number > 0 && plain(owner) && plain(name)).then(|| Self::new(format!("{owner}/{name}"), number))
    }
}

impl fmt::Display for PullRequestRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}#{}", self.repo, self.number)
    }
}

/// Where a pull request is in its life on GitHub.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PullRequestState {
    /// Open, and the review can post to it.
    Open,
    /// Merged; the review can be archived.
    Merged,
    /// Closed without merging.
    Closed,
}

/// A pull request as GitHub shows it now (PR-1, PR-3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullRequestInfo {
    /// Which pull request.
    pub pr: PullRequestRef,
    /// Its title.
    pub title: String,
    /// Who opened it.
    pub author: String,
    /// The branch it merges into.
    pub base_branch: String,
    /// The branch it comes from.
    pub head_branch: String,
    /// The commit its head is at.
    pub head_sha: String,
    /// Files changed.
    pub files: u32,
    /// Lines added.
    pub added: u32,
    /// Lines removed.
    pub removed: u32,
    /// Open, merged or closed.
    pub state: PullRequestState,
    /// The checks, as people read them ("3 passed, 1 failing"); empty when
    /// there are none.
    pub checks: String,
}

/// What starting a review asks for (PR-4 to PR-6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewStart {
    /// The pull requests the review holds; the first one is the review's
    /// main one.
    pub prs: Vec<PullRequestRef>,
    /// The machine the review's clones are made on.
    pub machine: MachineId,
    /// The agent the review runs.
    pub agent: Agent,
    /// The agent's model, or empty for its default.
    pub model: String,
    /// What the agent is told first, after its brief.
    pub first_message: String,
    /// Whether the agent may push (PR-8). Off until the person allows it.
    pub push_allowed: bool,
}

/// Where a pull request sits in a stack of pull requests (D300): the one its
/// branch is based on, and the ones based on its own. A stack may be a tree,
/// so a pull request can have several children.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StackPosition {
    /// The pull request this one is stacked on; `None` for the bottom, whose
    /// base is the trunk.
    pub parent: Option<PullRequestRef>,
    /// The pull requests stacked on this one.
    pub children: Vec<PullRequestRef>,
}

/// One pull request's head, as it moved since the last check (PR-15, D298).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeadChange {
    /// The pull request.
    pub pr: PullRequestRef,
    /// Commits its head gained.
    pub new_commits: u32,
    /// Whether its head was force-pushed (commits were replaced).
    pub force_pushed: bool,
    /// Its state now.
    pub state: PullRequestState,
    /// The new base, when it was retargeted since the last check.
    pub base_branch: Option<String>,
    /// Where it sits in a stack now, when it is in one (D300). A parent that
    /// merged retargets its child, which shows here as well as in
    /// `base_branch`.
    pub stack: Option<StackPosition>,
}

/// What a check of a review's heads found (PR-15, D298).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeadCheck {
    /// Each pull request that moved; empty when nothing did.
    pub changes: Vec<HeadChange>,
    /// When the check ran, in Unix milliseconds.
    pub checked_at_unix_ms: i64,
}

/// A review's head changes, pushed on the feed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeadsChanged {
    /// The review session the pull requests belong to.
    pub session: SessionId,
    /// What moved, one entry per pull request.
    pub changes: Vec<HeadChange>,
}

/// The stream [`PullRequestsClient::watch_heads`] returns.
pub type HeadEvents = mpsc::UnboundedReceiver<HeadsChanged>;

/// A review verdict (PR-12).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Verdict {
    /// Comments only.
    Comment,
    /// Approve the pull request.
    Approve,
    /// Ask for changes.
    RequestChanges,
}

/// A comment a review will post on a line of a pull request (PR-9, PR-16).
/// Its lines changing since it was written is [`Anchor::outdated`]; it can't
/// be posted then.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Draft {
    /// Its id in the comment ledger.
    pub id: CommentId,
    /// The line it is on.
    pub anchor: Anchor,
    /// What it says.
    pub text: String,
    /// Who wrote it: the person, or the agent by name.
    pub author: Author,
    /// The comment's revision; an edit names the one it was made against.
    pub revision: u64,
}

/// Why a draft won't be posted with a review (PR-12).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LeftOutReason {
    /// Its lines changed since it was written (PR-16).
    Outdated,
    /// Its line isn't in the pull request's diff, so GitHub would refuse it.
    OutOfDiff,
}

/// A draft that stays a draft, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeftOut {
    /// The draft.
    pub draft: Draft,
    /// Why it isn't posted.
    pub reason: LeftOutReason,
}

/// What one pull request's review will post (PR-12).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanPullRequest {
    /// The pull request.
    pub pr: PullRequestRef,
    /// The drafts it will post.
    pub drafts: Vec<Draft>,
    /// The agent's drafted summary, or empty.
    pub drafted_summary: String,
    /// Drafts that won't be posted with this review.
    pub left_out: Vec<LeftOut>,
}

/// Every pull request's review as it would post now, with the token a
/// submit must present (PR-12). The token is single-use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubmitPlan {
    /// The review session.
    pub session: SessionId,
    /// One entry per pull request of the review.
    pub pull_requests: Vec<PlanPullRequest>,
    /// What the submit presents to prove the plan was seen.
    pub token: String,
}

/// One pull request's review to post (PR-12).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewSubmission {
    /// The pull request.
    pub pr: PullRequestRef,
    /// The verdict.
    pub verdict: Verdict,
    /// The review's summary, or empty.
    pub summary: String,
    /// The drafts to post; the rest stay drafts.
    pub drafts: Vec<CommentId>,
}

/// What one pull request's post did (PR-13, PR-14).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubmitResult {
    /// The pull request.
    pub pr: PullRequestRef,
    /// The review's link when it posted; why it didn't otherwise. Its drafts
    /// stay drafts when it didn't.
    pub outcome: Result<String, PullRequestError>,
}

/// Whether a commit is reviewed (A9, D299).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CommitState {
    /// Not marked.
    Unreviewed,
    /// Marked by the person.
    Reviewed,
    /// An earlier commit with the same patch was marked (after a force push).
    Carried,
}

/// Who marked commits reviewed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewedBy {
    /// The GitHub login gh was signed in as.
    pub login: String,
}

/// One commit of a pull request, base to head, oldest first (A9).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commit {
    /// Its sha.
    pub sha: String,
    /// Its first line.
    pub subject: String,
    /// Its `git patch-id --stable`, which says whether its change is the same.
    pub patch_id: String,
    /// Whether it is reviewed.
    pub state: CommitState,
    /// For a carried commit, the earlier sha that was marked.
    pub reviewed_as: Option<String>,
    /// When it was marked, in Unix milliseconds.
    pub reviewed_at_unix_ms: Option<i64>,
    /// The head it was marked at.
    pub reviewed_head: Option<String>,
    /// Who marked it.
    pub by: Option<ReviewedBy>,
}

/// What a history entry recorded (A9).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HistoryKind {
    /// Commits were marked reviewed.
    Marked,
    /// Commits were unmarked.
    Unmarked,
    /// Commits were carried over by a force push.
    Carried,
    /// The whole pull request was marked reviewed.
    WholePr,
}

/// One entry of a pull request's review history, append-only (A9).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryEntry {
    /// Its place in the history, from 1.
    pub seq: u32,
    /// When it happened, in Unix milliseconds.
    pub at_unix_ms: i64,
    /// What happened.
    pub kind: HistoryKind,
    /// The commits it is about.
    pub commits: Vec<String>,
    /// The head at the time.
    pub head_sha: String,
    /// For a whole-PR mark, the review snapshot it pinned.
    pub snapshot: Option<u32>,
    /// Who did it.
    pub by: Option<ReviewedBy>,
}

/// A pull request's commits and their marks, which every review of it shares
/// across restarts and force pushes (A9, D299).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullRequestCommits {
    /// The pull request.
    pub pr: PullRequestRef,
    /// The head the review's clone is at.
    pub head_sha: String,
    /// The commits, oldest first.
    pub commits: Vec<Commit>,
    /// The history, oldest first.
    pub history: Vec<HistoryEntry>,
}

/// Why a pull request call failed, with what the person can do about it.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PullRequestError {
    /// gh on the machine isn't signed in to GitHub.
    #[error(
        "gh on {machine} isn't signed in to GitHub, so this pull request can't be read there. Run `gh auth login` on {machine}, then try again; Humata uses gh's own login and never asks for a token"
    )]
    SignedOut {
        /// The machine whose gh is signed out.
        machine: MachineId,
    },
    /// GitHub has no such pull request.
    #[error("{pr} isn't a pull request GitHub can find. Check the link and its repository, then try again")]
    NotFound {
        /// The pull request asked for.
        pr: PullRequestRef,
    },
    /// The account gh is signed in as can't read the repository.
    #[error(
        "The GitHub account gh is signed in as can't see {repo}. Sign in to gh as an account that can read it (`gh auth status` shows which one is signed in), then try again"
    )]
    NotVisible {
        /// The repository, as `owner/name`.
        repo: String,
    },
    /// GitHub is limiting requests for the account.
    #[error(
        "GitHub is limiting requests from {machine}'s gh right now. Wait {}, then try again; a different login doesn't help, since the limit belongs to the account",
        wait_phrase(.retry_after.as_ref())
    )]
    RateLimited {
        /// The machine whose gh was limited.
        machine: MachineId,
        /// How long until the limit resets, when GitHub said.
        retry_after: Option<Duration>,
    },
    /// Anything else the daemon refused or couldn't do (offline, not served
    /// yet, a stale plan, and so on), as the daemon words it.
    #[error(transparent)]
    Daemon(#[from] DaemonError),
}

/// How long to wait out a rate limit, as people read it.
fn wait_phrase(retry_after: Option<&Duration>) -> String {
    match retry_after {
        Some(wait) if wait.as_secs() >= 120 => {
            format!("about {} minutes (until the limit resets)", wait.as_secs().div_ceil(60))
        }
        Some(wait) => format!("about {} seconds (until the limit resets)", wait.as_secs().max(1)),
        None => "until the limit resets".to_owned(),
    }
}

/// A pull request call's answer.
pub type PullRequestReply<T> = BoxFuture<Result<T, PullRequestError>>;

/// What the app asks of pull request reviews.
pub trait PullRequestsClient: Send + Sync + 'static {
    /// A pasted pull request as GitHub shows it now (PR-1, PR-3).
    fn resolve(&self, pr: &PullRequestRef) -> PullRequestReply<PullRequestInfo>;

    /// Starts a review of `start.prs` as one session on `start.machine`
    /// (PR-4 to PR-6). Answers once the session exists.
    fn start_review(&self, start: ReviewStart) -> PullRequestReply<SessionId>;

    /// Checks the review's heads now (PR-15). Changes come back here and on
    /// the feed; a check never moves the clones (that is
    /// [`PullRequestsClient::update_clone`]).
    fn check_heads(&self, session: &SessionId) -> PullRequestReply<HeadCheck>;

    /// Fetches each pull request's new commits into the review's clones and
    /// tells the agent what changed (PR-16).
    fn update_clone(&self, session: &SessionId) -> PullRequestReply<()>;

    /// Lets the review's agent push, or stops it (PR-8).
    fn set_push_allowed(&self, session: &SessionId, allowed: bool) -> PullRequestReply<()>;

    /// The review as it would post now, with the token a submit presents
    /// (PR-12).
    fn plan_submit(&self, session: &SessionId) -> PullRequestReply<SubmitPlan>;

    /// Posts one review per pull request with `token` from the plan. A post
    /// that fails is answered in its [`SubmitResult`]; the others still post
    /// (PR-13, PR-14).
    fn submit_review(
        &self,
        session: &SessionId,
        token: &str,
        reviews: Vec<ReviewSubmission>,
    ) -> PullRequestReply<Vec<SubmitResult>>;

    /// Saves the agent's drafted summary for one pull request (PR-12).
    fn draft_summary(
        &self,
        session: &SessionId,
        pr: &PullRequestRef,
        summary: &str,
    ) -> PullRequestReply<()>;

    /// A pull request's commits and their marks (A9).
    fn list_commits(
        &self,
        session: &SessionId,
        pr: &PullRequestRef,
    ) -> PullRequestReply<PullRequestCommits>;

    /// Marks commits reviewed, or unmarks them, and answers the commits as
    /// they are now (A9, D299).
    fn mark_commits_reviewed(
        &self,
        session: &SessionId,
        pr: &PullRequestRef,
        commits: Vec<String>,
        reviewed: bool,
    ) -> PullRequestReply<PullRequestCommits>;

    /// Follows head changes from now on, as they are pushed (PR-15, D286).
    fn watch_heads(&self) -> PullRequestReply<HeadEvents>;
}
