//! New commits (PR-15 to PR-17, D286, D298, D300): the daemon checks each
//! review's pull requests and pushes what moved on the feed, so nothing here
//! polls and no timer runs in the app. A moved head shows as a notice with
//! Update the clone; a merged or closed pull request offers Archive.

use std::collections::BTreeMap;
use std::sync::Arc;

use humata2_clients::model::SessionId;
use humata2_clients::pull_requests::{
    HeadChange, HeadsChanged, PullRequestError, PullRequestRef, PullRequestState, PullRequestsClient,
    StackPosition,
};
use tca::{Effect, Reducer};

const WATCH: &str = "pr-review-heads";

/// What changed on one pull request since the clone was last updated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Moved {
    /// Commits its head gained, added up over every change since the clone
    /// was updated.
    pub new_commits: u32,
    /// Whether it was force-pushed at any point since.
    pub force_pushed: bool,
    /// Its state now.
    pub state: PullRequestState,
    /// The base it was retargeted to, when it was.
    pub base_branch: Option<String>,
    /// Where it sits in a stack now (D300), when it is in one.
    pub stack: Option<StackPosition>,
}

impl Moved {
    fn first(change: &HeadChange) -> Self {
        Self {
            new_commits: 0,
            force_pushed: false,
            state: change.state,
            base_branch: None,
            stack: None,
        }
        .after(change)
    }

    fn after(mut self, change: &HeadChange) -> Self {
        self.new_commits = self.new_commits.saturating_add(change.new_commits);
        self.force_pushed |= change.force_pushed;
        self.state = change.state;
        if change.base_branch.is_some() {
            self.base_branch.clone_from(&change.base_branch);
        }
        if change.stack.is_some() {
            self.stack.clone_from(&change.stack);
        }
        self
    }
}

/// The notices' state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HeadsState {
    /// Whether the feed is followed or being opened.
    pub watching: bool,
    /// What moved, per review and pull request, until its clone is updated.
    pub moved: BTreeMap<SessionId, BTreeMap<PullRequestRef, Moved>>,
    /// Reviews whose clones are being updated.
    pub updating: Vec<SessionId>,
    /// The last thing that failed, with what to do.
    pub error: Option<PullRequestError>,
}

impl HeadsState {
    /// What moved in `session`'s review; empty when nothing did.
    #[must_use]
    pub fn notice(&self, session: &SessionId) -> Vec<(&PullRequestRef, &Moved)> {
        self.moved
            .get(session)
            .map(|prs| prs.iter().collect())
            .unwrap_or_default()
    }

    /// The pull requests of `session`'s review that are merged or closed,
    /// which the review can archive.
    #[must_use]
    pub fn archivable(&self, session: &SessionId) -> Vec<&PullRequestRef> {
        self.notice(session)
            .into_iter()
            .filter(|(_, moved)| moved.state != PullRequestState::Open)
            .map(|(pr, _)| pr)
            .collect()
    }
}

/// What happens to the notices.
#[derive(Clone, Debug, PartialEq)]
pub enum HeadsAction {
    /// Follow head changes from now on.
    Start,
    /// The daemon pushed a review's head changes.
    Pushed(HeadsChanged),
    /// The feed ended: it is followed again.
    WatchEnded,
    /// The feed couldn't be opened.
    WatchFailed(PullRequestError),
    /// Update the clone: fetch the new commits and tell the agent.
    UpdateClone(SessionId),
    /// The daemon answered Update the clone.
    CloneUpdated {
        /// The review.
        session: SessionId,
        /// Whether it worked, or why not.
        result: Result<(), PullRequestError>,
    },
    /// Archive the review whose pull request merged or closed. The parent
    /// opens the archive dialog, which names the unsubmitted drafts.
    Archive(SessionId),
    /// Hide the error.
    DismissError,
    /// For the parent.
    Delegate(Delegate),
}

/// What the notices ask their parent to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delegate {
    /// Open the archive dialog for this review.
    Archiving(SessionId),
}

/// The notices' reducer.
#[derive(Clone)]
pub struct Heads {
    client: Arc<dyn PullRequestsClient>,
}

impl Heads {
    /// Notices that follow `client`'s feed.
    #[must_use]
    pub fn new(client: Arc<dyn PullRequestsClient>) -> Self {
        Self { client }
    }

    fn watch(&self) -> Effect<HeadsAction> {
        let client = self.client.clone();
        Effect::run(move |send| async move {
            match client.watch_heads().await {
                Err(error) => send.send(HeadsAction::WatchFailed(error)),
                Ok(mut events) => {
                    while let Some(pushed) = events.recv().await {
                        send.ready().await;
                        send.send(HeadsAction::Pushed(pushed));
                    }
                    send.send(HeadsAction::WatchEnded);
                }
            }
        })
        .cancellable(WATCH, true)
    }
}

impl Reducer for Heads {
    type State = HeadsState;
    type Action = HeadsAction;

    fn reduce(&self, state: &mut HeadsState, action: HeadsAction) -> Effect<HeadsAction> {
        match action {
            // A feed that ends is followed again: push, never poll.
            HeadsAction::Start | HeadsAction::WatchEnded => {
                state.watching = true;
                return self.watch();
            }
            HeadsAction::WatchFailed(error) => {
                state.watching = false;
                state.error = Some(error);
            }
            HeadsAction::Pushed(pushed) => {
                for change in &pushed.changes {
                    // A head that only reports it is the same as before is
                    // not news.
                    let news = change.new_commits > 0
                        || change.force_pushed
                        || change.base_branch.is_some()
                        || change.state != PullRequestState::Open;
                    if !news {
                        continue;
                    }
                    let prs = state.moved.entry(pushed.session.clone()).or_default();
                    match prs.remove(&change.pr) {
                        Some(held) => prs.insert(change.pr.clone(), held.after(change)),
                        None => prs.insert(change.pr.clone(), Moved::first(change)),
                    };
                }
            }
            HeadsAction::UpdateClone(session) => {
                if state.updating.contains(&session) {
                    return Effect::none();
                }
                state.updating.push(session.clone());
                state.error = None;
                let client = self.client.clone();
                return Effect::future(async move {
                    let result = client.update_clone(&session).await;
                    HeadsAction::CloneUpdated { session, result }
                });
            }
            HeadsAction::CloneUpdated { session, result } => {
                state.updating.retain(|held| held != &session);
                match result {
                    // What moved is now in the clone. A pull request that
                    // merged or closed keeps its notice: the clone being
                    // current doesn't make it open again.
                    Ok(()) => {
                        if let Some(prs) = state.moved.get_mut(&session) {
                            prs.retain(|_, moved| moved.state != PullRequestState::Open);
                            for moved in prs.values_mut() {
                                moved.new_commits = 0;
                                moved.force_pushed = false;
                                moved.base_branch = None;
                            }
                        }
                    }
                    Err(error) => state.error = Some(error),
                }
            }
            HeadsAction::Archive(session) => {
                if !state.archivable(&session).is_empty() {
                    return Effect::send(HeadsAction::Delegate(Delegate::Archiving(session)));
                }
            }
            HeadsAction::DismissError => state.error = None,
            HeadsAction::Delegate(_) => {}
        }
        Effect::none()
    }
}
