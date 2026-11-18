//! Submit review (PR-9, PR-12 to PR-14, PR-16, D287): the drafts of each pull
//! request, which the person can edit or drop, a verdict for each pull
//! request and a summary, and one post per pull request. A post that fails
//! leaves that pull request's drafts in place and names what failed; the
//! others still post. Drafts the plan left out (their lines changed or left
//! the diff) are listed with Re-attach or Drop.

use std::sync::Arc;

use humata2_clients::comments::{Anchor, Comment, CommentId, CommentsClient};
use humata2_clients::model::SessionId;
use humata2_clients::pull_requests::{
    Draft, LeftOut, PullRequestError, PullRequestRef, PullRequestsClient, ReviewSubmission, SubmitPlan,
    SubmitResult, Verdict,
};
use tca::{Effect, Reducer};

/// Where Submit review stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Step {
    /// Not showing.
    #[default]
    Closed,
    /// The daemon is working out what would post.
    Planning,
    /// The person is looking at what would post.
    Reviewing,
    /// The reviews are being posted.
    Posting,
}

/// What posting one pull request's review came to.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// GitHub took it, at this link.
    Posted(String),
    /// It didn't post. Its drafts are still drafts; the error says what to
    /// do.
    Failed(PullRequestError),
}

/// One pull request's part of Submit review.
#[derive(Clone, Debug, PartialEq)]
pub struct PullRequestReview {
    /// The pull request.
    pub pr: PullRequestRef,
    /// The drafts that will post.
    pub drafts: Vec<Draft>,
    /// Drafts the plan left out, with Re-attach or Drop.
    pub left_out: Vec<LeftOut>,
    /// The review's verdict.
    pub verdict: Verdict,
    /// The review's summary: the agent's draft, as the person edited it.
    pub summary: String,
    /// What the last post came to, once one was tried.
    pub outcome: Option<Outcome>,
}

impl PullRequestReview {
    /// Whether this review has something to post: drafts, a summary, or a
    /// verdict that says something. One that posted already has nothing
    /// more.
    #[must_use]
    pub fn has_something_to_post(&self) -> bool {
        !matches!(self.outcome, Some(Outcome::Posted(_)))
            && (!self.drafts.is_empty()
                || !self.summary.trim().is_empty()
                || self.verdict != Verdict::Comment)
    }
}

/// Submit review's state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SubmitState {
    /// Where it stands.
    pub step: Step,
    /// The review being submitted.
    pub session: Option<SessionId>,
    /// One entry per pull request of the review.
    pub reviews: Vec<PullRequestReview>,
    /// The last thing that failed that isn't one pull request's post, with
    /// what to do.
    pub error: Option<PullRequestError>,
    token: Option<String>,
}

impl SubmitState {
    /// Whether Submit would post something now.
    #[must_use]
    pub fn can_submit(&self) -> bool {
        self.step == Step::Reviewing
            && self.token.is_some()
            && self.reviews.iter().any(PullRequestReview::has_something_to_post)
    }

    fn review_mut(&mut self, pr: &PullRequestRef) -> Option<&mut PullRequestReview> {
        self.reviews.iter_mut().find(|review| &review.pr == pr)
    }

    /// The draft `id` where it stands, in a review's drafts or left out.
    fn holds(&self, id: &CommentId) -> bool {
        self.reviews.iter().any(|review| {
            review.drafts.iter().any(|draft| &draft.id == id)
                || review.left_out.iter().any(|left| &left.draft.id == id)
        })
    }

    fn revision_of(&self, id: &CommentId) -> Option<u64> {
        self.reviews.iter().find_map(|review| {
            review
                .drafts
                .iter()
                .chain(review.left_out.iter().map(|left| &left.draft))
                .find(|draft| &draft.id == id)
                .map(|draft| draft.revision)
        })
    }
}

/// What happens in Submit review.
#[derive(Clone, Debug, PartialEq)]
pub enum SubmitAction {
    /// Open Submit review for a review session: the daemon plans what posts.
    Open(SessionId),
    /// Close it without posting.
    Close,
    /// The daemon planned the submit, or couldn't.
    Planned(Result<SubmitPlan, PullRequestError>),
    /// Change a draft's text.
    EditDraft {
        /// The draft.
        id: CommentId,
        /// What it says now.
        text: String,
    },
    /// The daemon answered an edit.
    Edited(Result<Comment, PullRequestError>),
    /// Drop a draft, whether it will post or was left out.
    DropDraft(CommentId),
    /// The daemon answered a drop.
    Dropped {
        /// The draft.
        id: CommentId,
        /// Whether it was dropped, or why not.
        result: Result<(), PullRequestError>,
    },
    /// Point a left-out draft at lines that are in the diff.
    Reattach {
        /// The draft.
        id: CommentId,
        /// The lines it now points at.
        anchor: Anchor,
    },
    /// The daemon answered a re-attach.
    Reattached(Result<Comment, PullRequestError>),
    /// Choose a pull request's verdict.
    SetVerdict {
        /// The pull request.
        pr: PullRequestRef,
        /// The verdict.
        verdict: Verdict,
    },
    /// Edit a pull request's summary.
    SetSummary {
        /// The pull request.
        pr: PullRequestRef,
        /// The summary.
        summary: String,
    },
    /// Post one review per pull request that has something to post.
    Submit,
    /// The daemon answered Submit.
    Submitted(Result<Vec<SubmitResult>, PullRequestError>),
    /// For the parent.
    Delegate(Delegate),
}

/// What Submit review asks its parent to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delegate {
    /// Every review posted: the dialog is done.
    Posted(SessionId),
}

/// Submit review's reducer.
#[derive(Clone)]
pub struct Submit {
    pull_requests: Arc<dyn PullRequestsClient>,
    comments: Arc<dyn CommentsClient>,
}

impl Submit {
    /// Submit review that posts through `pull_requests` and changes drafts
    /// through `comments`.
    #[must_use]
    pub fn new(pull_requests: Arc<dyn PullRequestsClient>, comments: Arc<dyn CommentsClient>) -> Self {
        Self {
            pull_requests,
            comments,
        }
    }

    fn plan(&self, session: SessionId) -> Effect<SubmitAction> {
        let client = self.pull_requests.clone();
        Effect::future(async move { SubmitAction::Planned(client.plan_submit(&session).await) })
    }
}

fn comment_result<T>(result: Result<T, humata2_clients::DaemonError>) -> Result<T, PullRequestError> {
    result.map_err(PullRequestError::from)
}

/// Takes a plan into the reviews, keeping what the person chose and what
/// posting already did for pull requests the plan lists again.
fn take_plan(state: &mut SubmitState, plan: SubmitPlan) {
    let held = std::mem::take(&mut state.reviews);
    state.reviews = plan
        .pull_requests
        .into_iter()
        .map(|planned| {
            let before = held.iter().find(|review| review.pr == planned.pr);
            PullRequestReview {
                verdict: before.map_or(Verdict::Comment, |review| review.verdict),
                summary: before.map_or_else(|| planned.drafted_summary.clone(), |review| review.summary.clone()),
                outcome: before.and_then(|review| review.outcome.clone()),
                pr: planned.pr,
                drafts: planned.drafts,
                left_out: planned.left_out,
            }
        })
        .collect();
    state.token = Some(plan.token);
}

/// Puts `comment`'s text, revision and anchor into the draft it is.
fn refresh(draft: &mut Draft, comment: &Comment) {
    draft.text.clone_from(&comment.text);
    draft.revision = comment.revision;
    draft.anchor = comment.anchor.clone();
}

impl Reducer for Submit {
    type State = SubmitState;
    type Action = SubmitAction;

    fn reduce(&self, state: &mut SubmitState, action: SubmitAction) -> Effect<SubmitAction> {
        match action {
            SubmitAction::Open(session) => {
                *state = SubmitState {
                    step: Step::Planning,
                    session: Some(session.clone()),
                    ..SubmitState::default()
                };
                return self.plan(session);
            }
            SubmitAction::Close => *state = SubmitState::default(),
            // A plan that answers after Submit review was closed opens nothing.
            SubmitAction::Planned(_) if state.step == Step::Closed => {}
            SubmitAction::Planned(result) => match result {
                Ok(plan) => {
                    take_plan(state, plan);
                    state.step = Step::Reviewing;
                }
                Err(error) => {
                    state.token = None;
                    state.error = Some(error);
                    if state.step == Step::Posting || state.step == Step::Planning {
                        state.step = Step::Reviewing;
                    }
                }
            },
            SubmitAction::EditDraft { id, text } => {
                let Some(revision) = state.revision_of(&id) else {
                    return Effect::none();
                };
                let client = self.comments.clone();
                return Effect::future(async move {
                    SubmitAction::Edited(comment_result(client.edit(&id, &text, revision).await))
                });
            }
            SubmitAction::Edited(result) => match result {
                Ok(comment) => {
                    state.error = None;
                    for review in &mut state.reviews {
                        for draft in review
                            .drafts
                            .iter_mut()
                            .chain(review.left_out.iter_mut().map(|left| &mut left.draft))
                        {
                            if draft.id == comment.id {
                                refresh(draft, &comment);
                            }
                        }
                    }
                }
                Err(error) => state.error = Some(error),
            },
            SubmitAction::DropDraft(id) => {
                if !state.holds(&id) {
                    return Effect::none();
                }
                let client = self.comments.clone();
                return Effect::future(async move {
                    let result = comment_result(client.drop_comment(&id).await);
                    SubmitAction::Dropped { id, result }
                });
            }
            SubmitAction::Dropped { id, result } => match result {
                Ok(()) => {
                    state.error = None;
                    for review in &mut state.reviews {
                        review.drafts.retain(|draft| draft.id != id);
                        review.left_out.retain(|left| left.draft.id != id);
                    }
                }
                Err(error) => state.error = Some(error),
            },
            SubmitAction::Reattach { id, anchor } => {
                let Some(revision) = state.revision_of(&id) else {
                    return Effect::none();
                };
                let client = self.comments.clone();
                return Effect::future(async move {
                    SubmitAction::Reattached(comment_result(client.reattach(&id, anchor, revision).await))
                });
            }
            SubmitAction::Reattached(result) => match result {
                Ok(comment) => {
                    state.error = None;
                    for review in &mut state.reviews {
                        if let Some(index) = review.left_out.iter().position(|left| left.draft.id == comment.id) {
                            let mut draft = review.left_out.remove(index).draft;
                            refresh(&mut draft, &comment);
                            review.drafts.push(draft);
                        }
                    }
                }
                Err(error) => state.error = Some(error),
            },
            SubmitAction::SetVerdict { pr, verdict } => {
                if let Some(review) = state.review_mut(&pr) {
                    review.verdict = verdict;
                }
            }
            SubmitAction::SetSummary { pr, summary } => {
                if let Some(review) = state.review_mut(&pr) {
                    review.summary = summary;
                }
            }
            SubmitAction::Submit => {
                if !state.can_submit() {
                    return Effect::none();
                }
                let (Some(session), Some(token)) = (state.session.clone(), state.token.take()) else {
                    return Effect::none();
                };
                let reviews: Vec<ReviewSubmission> = state
                    .reviews
                    .iter()
                    .filter(|review| review.has_something_to_post())
                    .map(|review| ReviewSubmission {
                        pr: review.pr.clone(),
                        verdict: review.verdict,
                        summary: review.summary.clone(),
                        drafts: review.drafts.iter().map(|draft| draft.id.clone()).collect(),
                    })
                    .collect();
                state.step = Step::Posting;
                state.error = None;
                let client = self.pull_requests.clone();
                return Effect::future(async move {
                    SubmitAction::Submitted(client.submit_review(&session, &token, reviews).await)
                });
            }
            SubmitAction::Submitted(result) => {
                let Some(session) = state.session.clone() else {
                    return Effect::none();
                };
                match result {
                    Ok(results) => {
                        for result in results {
                            if let Some(review) = state.review_mut(&result.pr) {
                                review.outcome = Some(match result.outcome {
                                    Ok(url) => Outcome::Posted(url),
                                    Err(error) => Outcome::Failed(error),
                                });
                            }
                        }
                        let all_posted = state
                            .reviews
                            .iter()
                            .all(|review| !review.has_something_to_post());
                        if all_posted {
                            state.step = Step::Reviewing;
                            return Effect::send(SubmitAction::Delegate(Delegate::Posted(session)));
                        }
                    }
                    Err(error) => state.error = Some(error),
                }
                // The plan's token is spent whatever happened: plan again
                // so the drafts that didn't post show as they are now, and
                // Submit has a token to try again with.
                return self.plan(session);
            }
            SubmitAction::Delegate(_) => {}
        }
        Effect::none()
    }
}
