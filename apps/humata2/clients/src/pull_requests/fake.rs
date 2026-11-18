//! A scripted [`PullRequestsClient`]: it keeps reviews the way the daemon
//! does (PR-4 to PR-17, A9), with the GitHub it asks scripted by the test:
//! which pull requests exist, whether gh is signed in or rate limited, and
//! which posts fail. No timers and no real IO. Head changes are pushed by the
//! test with [`FakePullRequests::emit_head_change`], the way GitHub moves a
//! head under a poll.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tokio::sync::mpsc;

use super::{
    Commit, CommitState, Draft, HeadChange, HeadCheck, HeadEvents, HeadsChanged, HistoryEntry,
    HistoryKind, LeftOut, LeftOutReason, PlanPullRequest, PullRequestCommits, PullRequestError,
    PullRequestInfo, PullRequestRef, PullRequestReply, PullRequestsClient, ReviewStart,
    ReviewSubmission, ReviewedBy, SubmitPlan, SubmitResult,
};
use crate::daemon::DaemonError;
use crate::model::{MachineId, SessionId};

/// The login the fake's gh is signed in as.
const LOGIN: &str = "reviewer";

/// A call a test can read back, one per trait method, with what it was about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Call {
    /// `resolve`.
    Resolve(PullRequestRef),
    /// `start_review`, with its pull requests.
    StartReview(Vec<PullRequestRef>),
    /// `check_heads`.
    CheckHeads(SessionId),
    /// `update_clone`.
    UpdateClone(SessionId),
    /// `set_push_allowed`.
    SetPushAllowed(SessionId, bool),
    /// `plan_submit`.
    PlanSubmit(SessionId),
    /// `submit_review`, with the token presented.
    SubmitReview(SessionId, String),
    /// `draft_summary`.
    DraftSummary(SessionId, PullRequestRef),
    /// `list_commits`.
    ListCommits(SessionId, PullRequestRef),
    /// `mark_commits_reviewed`, with the commits and whether they were marked.
    MarkCommitsReviewed(SessionId, PullRequestRef, Vec<String>, bool),
    /// `watch_heads`.
    WatchHeads,
}

/// One review session's state.
#[derive(Default)]
struct Review {
    prs: Vec<PullRequestRef>,
    push_allowed: bool,
    /// The drafts, each on the pull request it will post to.
    drafts: Vec<(PullRequestRef, Draft)>,
    summaries: HashMap<PullRequestRef, String>,
    /// Head changes no check has reported yet.
    pending: Vec<HeadChange>,
    /// The token of the latest plan, until a submit spends it.
    token: Option<String>,
}

/// One pull request's commits and their marks, shared by every review of it.
#[derive(Clone)]
struct Board {
    head_sha: String,
    commits: Vec<Commit>,
    history: Vec<HistoryEntry>,
}

impl Board {
    fn snapshot(&self, pr: &PullRequestRef) -> PullRequestCommits {
        PullRequestCommits {
            pr: pr.clone(),
            head_sha: self.head_sha.clone(),
            commits: self.commits.clone(),
            history: self.history.clone(),
        }
    }
}

#[derive(Default)]
struct Inner {
    pull_requests: HashMap<PullRequestRef, PullRequestInfo>,
    boards: HashMap<PullRequestRef, Board>,
    reviews: HashMap<SessionId, Review>,
    signed_out: bool,
    rate_limit: Option<Duration>,
    failures: HashMap<&'static str, PullRequestError>,
    post_failures: HashMap<PullRequestRef, PullRequestError>,
    next_session: u32,
    next_token: u32,
    /// Advances by a second on every mark, so marks have distinct times.
    clock: i64,
    watchers: Vec<mpsc::UnboundedSender<HeadsChanged>>,
    posted: Vec<(PullRequestRef, ReviewSubmission)>,
    starts: Vec<ReviewStart>,
    calls: Vec<Call>,
}

fn this_mac() -> MachineId {
    MachineId::new("this-mac")
}

fn refused(what: &str, reason: &str) -> PullRequestError {
    PullRequestError::Daemon(DaemonError::Refused {
        what: what.to_owned(),
        reason: reason.to_owned(),
    })
}

fn no_review() -> PullRequestError {
    PullRequestError::Daemon(DaemonError::NotFound {
        what: "This review".to_owned(),
    })
}

impl Inner {
    /// What GitHub or the daemon answers before the call does its work.
    fn gate(&self, method: &'static str) -> Result<(), PullRequestError> {
        if self.signed_out {
            return Err(PullRequestError::SignedOut { machine: this_mac() });
        }
        if let Some(retry_after) = self.rate_limit {
            return Err(PullRequestError::RateLimited {
                machine: this_mac(),
                retry_after: Some(retry_after),
            });
        }
        match self.failures.get(method) {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }

    fn review_mut(&mut self, session: &SessionId) -> Result<&mut Review, PullRequestError> {
        self.reviews.get_mut(session).ok_or_else(no_review)
    }

    /// Checks the session is a review that holds `pr`.
    fn holds(&self, session: &SessionId, pr: &PullRequestRef) -> Result<(), PullRequestError> {
        let review = self.reviews.get(session).ok_or_else(no_review)?;
        if review.prs.contains(pr) {
            Ok(())
        } else {
            Err(PullRequestError::NotFound { pr: pr.clone() })
        }
    }

    fn board_of(&self, pr: &PullRequestRef) -> PullRequestCommits {
        match self.boards.get(pr) {
            Some(board) => board.snapshot(pr),
            None => PullRequestCommits {
                pr: pr.clone(),
                head_sha: String::new(),
                commits: Vec::new(),
                history: Vec::new(),
            },
        }
    }

    /// Marks `shas` reviewed (or unmarks them) on the pull request's board and
    /// records it in the history.
    fn mark(
        &mut self,
        pr: &PullRequestRef,
        shas: &[String],
        reviewed: bool,
    ) -> Result<PullRequestCommits, PullRequestError> {
        let known = self
            .boards
            .get(pr)
            .ok_or_else(|| PullRequestError::NotFound { pr: pr.clone() })?;
        if let Some(unknown) = shas
            .iter()
            .find(|sha| !known.commits.iter().any(|commit| &commit.sha == *sha))
        {
            return Err(refused(
                "These commits",
                &format!("{unknown} isn't a commit of {pr}'s head; choose commits the list shows"),
            ));
        }
        self.clock += 1000;
        let at = self.clock;
        let by = ReviewedBy {
            login: LOGIN.to_owned(),
        };
        let Some(board) = self.boards.get_mut(pr) else {
            return Err(PullRequestError::NotFound { pr: pr.clone() });
        };
        for commit in board.commits.iter_mut().filter(|commit| shas.contains(&commit.sha)) {
            if reviewed {
                commit.state = CommitState::Reviewed;
                commit.reviewed_at_unix_ms = Some(at);
                commit.reviewed_head = Some(board.head_sha.clone());
                commit.by = Some(by.clone());
            } else {
                commit.state = CommitState::Unreviewed;
                commit.reviewed_at_unix_ms = None;
                commit.reviewed_head = None;
                commit.by = None;
            }
        }
        let seq = u32::try_from(board.history.len() + 1).unwrap_or(u32::MAX);
        board.history.push(HistoryEntry {
            seq,
            at_unix_ms: at,
            kind: if reviewed {
                HistoryKind::Marked
            } else {
                HistoryKind::Unmarked
            },
            commits: shas.to_vec(),
            head_sha: board.head_sha.clone(),
            snapshot: None,
            by: Some(by),
        });
        Ok(board.snapshot(pr))
    }
}

/// Posts each review in `reviews` that the token allows; a post that the
/// test scripted to fail is answered with its error and keeps its drafts.
fn submit(
    inner: &mut Inner,
    session: &SessionId,
    token: &str,
    reviews: Vec<ReviewSubmission>,
) -> Result<Vec<SubmitResult>, PullRequestError> {
    let failures = inner.post_failures.clone();
    let base = inner.posted.len();
    let review = inner.review_mut(session)?;
    if review.token.as_deref() != Some(token) {
        return Err(refused(
            "This review",
            "the plan it was made from is out of date; plan the review again, check what it posts, then submit",
        ));
    }
    review.token = None;
    let mut results = Vec::new();
    let mut posted = Vec::new();
    for submission in reviews {
        let outcome = match failures.get(&submission.pr) {
            Some(error) => Err(error.clone()),
            None => {
                review.drafts.retain(|(pr, draft)| {
                    !(pr == &submission.pr && submission.drafts.contains(&draft.id))
                });
                let number = base + posted.len() + 1;
                let url = format!(
                    "https://github.com/{}/pull/{}#pullrequestreview-{number}",
                    submission.pr.repo, submission.pr.number
                );
                posted.push((submission.pr.clone(), submission.clone()));
                Ok(url)
            }
        };
        results.push(SubmitResult {
            pr: submission.pr,
            outcome,
        });
    }
    inner.posted.extend(posted);
    Ok(results)
}

/// A pull request review client whose GitHub is scripted by the test.
#[derive(Clone, Default)]
pub struct FakePullRequests {
    shared: Arc<Mutex<Inner>>,
}

impl FakePullRequests {
    /// A fake with no pull requests, gh signed in and no failures.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.shared.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn call<T: Send + 'static>(
        &self,
        method: &'static str,
        call: Call,
        run: impl FnOnce(&mut Inner) -> Result<T, PullRequestError>,
    ) -> PullRequestReply<T> {
        let mut inner = self.inner();
        inner.calls.push(call);
        let result = inner.gate(method).and_then(|()| run(&mut inner));
        Box::pin(std::future::ready(result))
    }

    /// Makes `pr` exist on GitHub, as its info reads now.
    pub fn add_pull_request(&self, info: PullRequestInfo) {
        self.inner().pull_requests.insert(info.pr.clone(), info);
    }

    /// Gives `pr` its commits, base to head, oldest first, as `(sha, subject)`.
    /// The last sha is its head. Nothing is marked yet.
    pub fn seed_commits(&self, pr: &PullRequestRef, commits: &[(&str, &str)]) {
        let commits: Vec<Commit> = commits
            .iter()
            .map(|(sha, subject)| Commit {
                sha: (*sha).to_owned(),
                subject: (*subject).to_owned(),
                patch_id: format!("patch-{sha}"),
                state: CommitState::Unreviewed,
                reviewed_as: None,
                reviewed_at_unix_ms: None,
                reviewed_head: None,
                by: None,
            })
            .collect();
        let head_sha = commits.last().map(|commit| commit.sha.clone()).unwrap_or_default();
        self.inner().boards.insert(
            pr.clone(),
            Board {
                head_sha,
                commits,
                history: Vec::new(),
            },
        );
    }

    /// Makes gh report that it is signed out, for every call, or not.
    pub fn sign_out(&self, signed_out: bool) {
        self.inner().signed_out = signed_out;
    }

    /// Makes every call fail as rate limited, with `retry_after` to wait, or
    /// stops limiting with `None`.
    pub fn set_rate_limit(&self, retry_after: Option<Duration>) {
        self.inner().rate_limit = retry_after;
    }

    /// Makes calls of `method` (the trait's method name, such as `"resolve"`)
    /// fail with `error` until cleared.
    pub fn fail(&self, method: &'static str, error: PullRequestError) {
        self.inner().failures.insert(method, error);
    }

    /// Stops failing calls of `method`.
    pub fn clear_failure(&self, method: &'static str) {
        self.inner().failures.remove(method);
    }

    /// Makes posting `pr`'s review fail with `error`, and the rest post.
    pub fn fail_post(&self, pr: &PullRequestRef, error: PullRequestError) {
        self.inner().post_failures.insert(pr.clone(), error);
    }

    /// Puts a draft on `pr` in the review `session`, as the comment ledger
    /// holds it.
    pub fn add_draft(&self, session: &SessionId, pr: &PullRequestRef, draft: Draft) {
        if let Some(review) = self.inner().reviews.get_mut(session) {
            review.drafts.push((pr.clone(), draft));
        }
    }

    /// GitHub moved a review's head: the change is pushed on the feed and
    /// waits for the next check to list it. Nothing happens for a session that
    /// holds no review.
    pub fn emit_head_change(&self, session: &SessionId, change: HeadChange) {
        let mut inner = self.inner();
        let Some(review) = inner.reviews.get_mut(session) else {
            return;
        };
        review.pending.push(change.clone());
        let event = HeadsChanged {
            session: session.clone(),
            changes: vec![change],
        };
        inner.watchers.retain(|watcher| watcher.send(event.clone()).is_ok());
    }

    /// Every review posted, in the order it was posted.
    #[must_use]
    pub fn posted(&self) -> Vec<(PullRequestRef, ReviewSubmission)> {
        self.inner().posted.clone()
    }

    /// Every review started, as it was asked for, oldest first.
    #[must_use]
    pub fn starts(&self) -> Vec<ReviewStart> {
        self.inner().starts.clone()
    }

    /// The calls made so far, oldest first.
    #[must_use]
    pub fn calls(&self) -> Vec<Call> {
        self.inner().calls.clone()
    }
}

impl PullRequestsClient for FakePullRequests {
    fn resolve(&self, pr: &PullRequestRef) -> PullRequestReply<PullRequestInfo> {
        self.call("resolve", Call::Resolve(pr.clone()), |inner| {
            inner
                .pull_requests
                .get(pr)
                .cloned()
                .ok_or_else(|| PullRequestError::NotFound { pr: pr.clone() })
        })
    }

    fn start_review(&self, start: ReviewStart) -> PullRequestReply<SessionId> {
        let call = Call::StartReview(start.prs.clone());
        self.call("start_review", call, move |inner| {
            if start.prs.is_empty() {
                return Err(refused("This review", "name at least one pull request to review"));
            }
            if let Some(missing) = start.prs.iter().find(|pr| !inner.pull_requests.contains_key(*pr)) {
                return Err(PullRequestError::NotFound { pr: missing.clone() });
            }
            inner.next_session += 1;
            let session = SessionId::new(format!("se_pr_{}", inner.next_session));
            let review = Review {
                prs: start.prs.clone(),
                push_allowed: start.push_allowed,
                ..Review::default()
            };
            inner.starts.push(start);
            inner.reviews.insert(session.clone(), review);
            Ok(session)
        })
    }

    fn check_heads(&self, session: &SessionId) -> PullRequestReply<HeadCheck> {
        self.call("check_heads", Call::CheckHeads(session.clone()), |inner| {
            let review = inner.review_mut(session)?;
            Ok(HeadCheck {
                changes: std::mem::take(&mut review.pending),
                checked_at_unix_ms: 0,
            })
        })
    }

    fn update_clone(&self, session: &SessionId) -> PullRequestReply<()> {
        self.call("update_clone", Call::UpdateClone(session.clone()), |inner| {
            inner.review_mut(session).map(|_| ())
        })
    }

    fn set_push_allowed(&self, session: &SessionId, allowed: bool) -> PullRequestReply<()> {
        self.call(
            "set_push_allowed",
            Call::SetPushAllowed(session.clone(), allowed),
            |inner| {
                inner.review_mut(session)?.push_allowed = allowed;
                Ok(())
            },
        )
    }

    fn plan_submit(&self, session: &SessionId) -> PullRequestReply<SubmitPlan> {
        self.call("plan_submit", Call::PlanSubmit(session.clone()), |inner| {
            inner.next_token += 1;
            let token = format!("plan-{}", inner.next_token);
            let review = inner.review_mut(session)?;
            review.token = Some(token.clone());
            let pull_requests = review
                .prs
                .iter()
                .map(|pr| {
                    let (outdated, drafts): (Vec<&Draft>, Vec<&Draft>) = review
                        .drafts
                        .iter()
                        .filter(|(owner, _)| owner == pr)
                        .map(|(_, draft)| draft)
                        .partition(|draft| draft.anchor.outdated);
                    PlanPullRequest {
                        pr: pr.clone(),
                        drafts: drafts.into_iter().cloned().collect(),
                        drafted_summary: review.summaries.get(pr).cloned().unwrap_or_default(),
                        left_out: outdated
                            .into_iter()
                            .map(|draft| LeftOut {
                                draft: draft.clone(),
                                reason: LeftOutReason::Outdated,
                            })
                            .collect(),
                    }
                })
                .collect();
            Ok(SubmitPlan {
                session: session.clone(),
                pull_requests,
                token,
            })
        })
    }

    fn submit_review(
        &self,
        session: &SessionId,
        token: &str,
        reviews: Vec<ReviewSubmission>,
    ) -> PullRequestReply<Vec<SubmitResult>> {
        let call = Call::SubmitReview(session.clone(), token.to_owned());
        self.call("submit_review", call, |inner| submit(inner, session, token, reviews))
    }

    fn draft_summary(
        &self,
        session: &SessionId,
        pr: &PullRequestRef,
        summary: &str,
    ) -> PullRequestReply<()> {
        self.call(
            "draft_summary",
            Call::DraftSummary(session.clone(), pr.clone()),
            |inner| {
                inner.holds(session, pr)?;
                let review = inner.review_mut(session)?;
                review.summaries.insert(pr.clone(), summary.to_owned());
                Ok(())
            },
        )
    }

    fn list_commits(
        &self,
        session: &SessionId,
        pr: &PullRequestRef,
    ) -> PullRequestReply<PullRequestCommits> {
        self.call(
            "list_commits",
            Call::ListCommits(session.clone(), pr.clone()),
            |inner| {
                inner.holds(session, pr)?;
                Ok(inner.board_of(pr))
            },
        )
    }

    fn mark_commits_reviewed(
        &self,
        session: &SessionId,
        pr: &PullRequestRef,
        commits: Vec<String>,
        reviewed: bool,
    ) -> PullRequestReply<PullRequestCommits> {
        let call = Call::MarkCommitsReviewed(session.clone(), pr.clone(), commits.clone(), reviewed);
        self.call("mark_commits_reviewed", call, |inner| {
            inner.holds(session, pr)?;
            inner.mark(pr, &commits, reviewed)
        })
    }

    fn watch_heads(&self) -> PullRequestReply<HeadEvents> {
        self.call("watch_heads", Call::WatchHeads, |inner| {
            let (sender, receiver) = mpsc::unbounded_channel();
            inner.watchers.push(sender);
            Ok(receiver)
        })
    }
}
