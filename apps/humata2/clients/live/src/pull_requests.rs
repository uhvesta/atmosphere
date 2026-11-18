//! [`PullRequestsClient`] for the real daemon, over PullRequestReviewService
//! (PR-1 to PR-17, A9). Head changes come on the event feed, narrowed to
//! sessions, as the daemon pushes them (PR-15, D286); nothing polls. Each user
//! action sends a fresh idempotency key.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use humata2_clients::comments::{Author, CommentId};
use humata2_clients::model::{MachineId, SessionId};
use humata2_clients::pull_requests::{
    Commit, CommitState, Draft, HeadChange, HeadCheck, HeadEvents, HeadsChanged, HistoryEntry,
    HistoryKind, LeftOut, LeftOutReason, PlanPullRequest, PullRequestCommits, PullRequestError,
    PullRequestInfo, PullRequestRef, PullRequestReply, PullRequestState, PullRequestsClient,
    ReviewStart, ReviewSubmission, ReviewedBy, StackPosition, SubmitPlan, SubmitResult, Verdict,
};
use humata2_clients::DaemonError;
use humata2_grpc::humata::v1 as pb;
use humata2_grpc::humata::v1::pull_request_review_service_client::PullRequestReviewServiceClient;
use tokio::sync::mpsc;
use tonic::{Code, Status, Streaming};

use crate::connection::{LiveDaemon, Shared, Wire, daemon_error, fresh_key};
use crate::{map, review_chat};

/// Pull requests on the real daemon. Cheap to clone.
#[derive(Clone)]
pub struct LivePullRequests {
    shared: Arc<Shared>,
}

impl LivePullRequests {
    /// Pull request reviews over `daemon`'s connection.
    #[must_use]
    pub fn new(daemon: &LiveDaemon) -> Self {
        Self {
            shared: daemon.shared.clone(),
        }
    }
}

fn client(shared: &Shared) -> PullRequestReviewServiceClient<Wire> {
    PullRequestReviewServiceClient::new(shared.wire.clone())
}

fn session_wire(session: &SessionId) -> Option<pb::SessionId> {
    Some(pb::SessionId {
        value: session.as_str().to_owned(),
    })
}

fn pr_wire(pr: &PullRequestRef) -> pb::PullRequestRef {
    pb::PullRequestRef {
        repo: map::parse_repo(&pr.repo),
        number: pr.number,
        note: String::new(),
    }
}

fn pr_of(wire: Option<&pb::PullRequestRef>) -> Option<PullRequestRef> {
    let wire = wire?;
    Some(PullRequestRef::new(wire.repo.as_ref().map(map::repo_text)?, wire.number))
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX))
}

/// The daemon's words for a failure: its message, then its fix.
fn words(error: &pb::Error) -> String {
    crate::connection::said(&error.message, &error.fix)
}

/// The failures the app says more plainly than the daemon's words: gh signed
/// out, GitHub's rate limit (with how long to wait), a repository the account
/// can't see. `None` for the rest.
fn classify(machine: &MachineId, kind: Option<&pb::error::Kind>) -> Option<PullRequestError> {
    match kind? {
        pb::error::Kind::GithubCli(cli)
            if cli.problem == pb::git_hub_cli_problem::Problem::NotSignedIn as i32 =>
        {
            Some(PullRequestError::SignedOut {
                machine: machine.clone(),
            })
        }
        pb::error::Kind::RateLimited(limited) => Some(PullRequestError::RateLimited {
            machine: machine.clone(),
            retry_after: limited.resets_at.as_ref().map(|at| {
                let left = at.seconds.saturating_sub(now_secs()).max(0);
                Duration::from_secs(u64::try_from(left).unwrap_or(0))
            }),
        }),
        pb::error::Kind::RepoNotVisible(named) => Some(PullRequestError::NotVisible {
            repo: named.repo.as_ref().map(map::repo_text).unwrap_or_default(),
        }),
        _ => None,
    }
}

/// A status as the app's error, for a call about `pr` (when it names one).
fn failure(shared: &Shared, status: &Status, pr: Option<&PullRequestRef>, what: &str) -> PullRequestError {
    let kind = daemon_error(status).and_then(|error| error.kind);
    if let Some(known) = classify(&shared.machine, kind.as_ref()) {
        return known;
    }
    match pr {
        Some(pr) if status.code() == Code::NotFound => PullRequestError::NotFound { pr: pr.clone() },
        _ => PullRequestError::Daemon(shared.error(status, what)),
    }
}

/// A per-pull-request failure of a submit, as the daemon's error names it.
fn result_error(shared: &Shared, error: &pb::Error, pr: &PullRequestRef) -> PullRequestError {
    classify(&shared.machine, error.kind.as_ref()).unwrap_or_else(|| {
        PullRequestError::Daemon(DaemonError::Refused {
            what: format!("The review of {pr}"),
            reason: words(error),
        })
    })
}

/// An answer the app can't read: the app is older than the daemon.
fn unreadable(what: &str) -> PullRequestError {
    PullRequestError::Daemon(DaemonError::Refused {
        what: what.to_owned(),
        reason: "the daemon answered with data this app can't read; update Humata and try again".to_owned(),
    })
}

fn state_of(value: i32) -> Option<PullRequestState> {
    match pb::PullRequestState::try_from(value).ok()? {
        pb::PullRequestState::Open => Some(PullRequestState::Open),
        pb::PullRequestState::Merged => Some(PullRequestState::Merged),
        pb::PullRequestState::Closed => Some(PullRequestState::Closed),
        pb::PullRequestState::Unspecified => None,
    }
}

fn info(wire: &pb::PullRequestInfo) -> Option<PullRequestInfo> {
    Some(PullRequestInfo {
        pr: pr_of(wire.pr.as_ref())?,
        title: wire.title.clone(),
        author: wire.author.clone(),
        base_branch: wire.base_branch.clone(),
        head_branch: wire.head_branch.clone(),
        head_sha: wire.head_sha.clone(),
        files: wire.files,
        added: wire.added,
        removed: wire.removed,
        state: state_of(wire.state)?,
        checks: wire.checks.clone(),
    })
}

fn head_change(wire: &pb::HeadChange) -> Option<HeadChange> {
    Some(HeadChange {
        pr: pr_of(wire.pr.as_ref())?,
        new_commits: wire.new_commits,
        force_pushed: wire.force_pushed,
        state: state_of(wire.state)?,
        base_branch: Some(wire.base_branch.clone()).filter(|base| !base.is_empty()),
        stack: wire.stack.as_ref().map(stack_position),
    })
}

/// D300: a stack position; a child that doesn't map is left out.
fn stack_position(wire: &pb::StackPosition) -> StackPosition {
    StackPosition {
        parent: pr_of(wire.parent.as_ref()),
        children: wire.children.iter().filter_map(|child| pr_of(Some(child))).collect(),
    }
}

fn head_check(wire: &pb::CheckHeadsResponse) -> HeadCheck {
    HeadCheck {
        changes: wire.changes.iter().filter_map(head_change).collect(),
        checked_at_unix_ms: wire
            .checked_at
            .as_ref()
            .map_or(0, |at| map::unix_ms(at.seconds, at.nanos)),
    }
}

fn heads_changed(wire: &pb::PullRequestHeadsChanged) -> HeadsChanged {
    HeadsChanged {
        session: map::session_id(wire.session.as_ref()),
        changes: wire.changes.iter().filter_map(head_change).collect(),
    }
}

fn author_of(actor: Option<&pb::Actor>) -> Author {
    match actor.and_then(|actor| actor.actor.as_ref()) {
        Some(pb::actor::Actor::Agent(agent)) => {
            Author::Agent(agent.agent.as_ref().map(|id| id.value.clone()).unwrap_or_default())
        }
        _ => Author::User,
    }
}

/// A draft: a comment the review will post on a line (PR-9).
fn draft(wire: &pb::Comment) -> Option<Draft> {
    let id = wire.id.as_ref().map(|id| id.value.clone()).filter(|id| !id.is_empty())?;
    let anchor = review_chat::anchor(wire.anchor.as_ref()?)?;
    Some(Draft {
        id: CommentId::new(id),
        anchor,
        text: wire.text.clone(),
        author: author_of(wire.author.as_ref()),
        revision: wire.revision.as_ref().map_or(0, |revision| revision.value),
    })
}

fn left_out(wire: &pb::submit_plan::LeftOut) -> Option<LeftOut> {
    use pb::submit_plan::left_out::Reason;
    let reason = match Reason::try_from(wire.reason).ok()? {
        Reason::Outdated => LeftOutReason::Outdated,
        Reason::OutOfDiff => LeftOutReason::OutOfDiff,
        Reason::Unspecified => return None,
    };
    Some(LeftOut {
        draft: draft(wire.draft.as_ref()?)?,
        reason,
    })
}

fn plan(wire: &pb::SubmitPlan) -> Option<SubmitPlan> {
    let pull_requests = wire
        .pull_requests
        .iter()
        .map(|each| {
            Some(PlanPullRequest {
                pr: pr_of(each.pr.as_ref())?,
                drafts: each.drafts.iter().filter_map(draft).collect(),
                drafted_summary: each.drafted_summary.clone(),
                left_out: each.left_out.iter().filter_map(left_out).collect(),
            })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(SubmitPlan {
        session: map::session_id(wire.session.as_ref()),
        pull_requests,
        token: wire.token.as_ref().map(|token| token.value.clone()).unwrap_or_default(),
    })
}

fn verdict_wire(verdict: Verdict) -> pb::Verdict {
    match verdict {
        Verdict::Comment => pb::Verdict::Comment,
        Verdict::Approve => pb::Verdict::Approve,
        Verdict::RequestChanges => pb::Verdict::RequestChanges,
    }
}

fn review_wire(review: &ReviewSubmission) -> pb::ReviewSubmission {
    pb::ReviewSubmission {
        pr: Some(pr_wire(&review.pr)),
        verdict: verdict_wire(review.verdict) as i32,
        summary: review.summary.clone(),
        drafts: review
            .drafts
            .iter()
            .map(|id| pb::CommentId {
                value: id.as_str().to_owned(),
            })
            .collect(),
    }
}

fn submit_result(shared: &Shared, wire: &pb::submit_review_response::Result) -> Option<SubmitResult> {
    let pr = pr_of(wire.pr.as_ref())?;
    let outcome = match wire.error.as_ref() {
        Some(error) => Err(result_error(shared, error, &pr)),
        None => Ok(wire.review_url.clone()),
    };
    Some(SubmitResult { pr, outcome })
}

fn reviewed_by(wire: Option<&pb::ReviewedBy>) -> Option<ReviewedBy> {
    wire.map(|by| ReviewedBy {
        login: by.gh_login.clone(),
    })
}

fn commit_state(value: i32) -> CommitState {
    match pb::pr_commit::State::try_from(value) {
        Ok(pb::pr_commit::State::Reviewed) => CommitState::Reviewed,
        Ok(pb::pr_commit::State::Carried) => CommitState::Carried,
        _ => CommitState::Unreviewed,
    }
}

fn commit(wire: &pb::PrCommit) -> Commit {
    Commit {
        sha: wire.sha.clone(),
        subject: wire.subject.clone(),
        patch_id: wire.patch_id.clone(),
        state: commit_state(wire.state),
        reviewed_as: Some(wire.reviewed_as_sha.clone()).filter(|sha| !sha.is_empty()),
        reviewed_at_unix_ms: wire.reviewed_at.as_ref().map(|at| map::unix_ms(at.seconds, at.nanos)),
        reviewed_head: Some(wire.reviewed_head_sha.clone()).filter(|sha| !sha.is_empty()),
        by: reviewed_by(wire.by.as_ref()),
    }
}

fn history_entry(wire: &pb::CommitReviewEntry) -> Option<HistoryEntry> {
    use pb::commit_review_entry::Kind;
    let kind = match Kind::try_from(wire.kind).ok()? {
        Kind::Marked => HistoryKind::Marked,
        Kind::Unmarked => HistoryKind::Unmarked,
        Kind::Carried => HistoryKind::Carried,
        Kind::WholePr => HistoryKind::WholePr,
        Kind::Unspecified => return None,
    };
    Some(HistoryEntry {
        seq: wire.seq,
        at_unix_ms: wire.at.as_ref().map_or(0, |at| map::unix_ms(at.seconds, at.nanos)),
        kind,
        commits: wire.commits.clone(),
        head_sha: wire.head_sha.clone(),
        snapshot: wire.snapshot_n,
        by: reviewed_by(wire.by.as_ref()),
    })
}

fn commits_of(wire: &pb::PullRequestCommits) -> Option<PullRequestCommits> {
    Some(PullRequestCommits {
        pr: pr_of(wire.pr.as_ref())?,
        head_sha: wire.head_sha.clone(),
        commits: wire.commits.iter().map(commit).collect(),
        history: wire.history.iter().filter_map(history_entry).collect(),
    })
}

/// Follows the feed's head changes until it ends or nobody listens.
async fn forward_heads(mut stream: Streaming<pb::Event>, sender: mpsc::UnboundedSender<HeadsChanged>) {
    loop {
        let next = tokio::select! {
            () = sender.closed() => return,
            next = stream.message() => next,
        };
        match next {
            Ok(Some(event)) => {
                if let Some(pb::event::Payload::PullRequestHeads(wire)) = event.payload {
                    if sender.send(heads_changed(&wire)).is_err() {
                        return;
                    }
                }
            }
            Ok(None) | Err(_) => return,
        }
    }
}

impl PullRequestsClient for LivePullRequests {
    fn resolve(&self, pr: &PullRequestRef) -> PullRequestReply<PullRequestInfo> {
        let shared = self.shared.clone();
        let pr = pr.clone();
        Box::pin(async move {
            let answer = client(&shared)
                .resolve_pull_request(pb::ResolvePullRequestRequest {
                    r#ref: Some(pb::resolve_pull_request_request::Ref::Pr(pr_wire(&pr))),
                })
                .await
                .map_err(|status| failure(&shared, &status, Some(&pr), "This pull request"))?
                .into_inner();
            info(&answer).ok_or_else(|| unreadable("This pull request"))
        })
    }

    fn start_review(&self, start: ReviewStart) -> PullRequestReply<SessionId> {
        let shared = self.shared.clone();
        Box::pin(async move {
            let answer = client(&shared)
                .start_review(pb::StartReviewRequest {
                    idempotency_key: fresh_key(),
                    prs: start.prs.iter().map(pr_wire).collect(),
                    machine: Some(pb::MachineId {
                        value: start.machine.as_str().to_owned(),
                    }),
                    agent: Some(pb::AgentId {
                        value: shared.agents.name(start.agent),
                    }),
                    model: start.model,
                    first_message: start.first_message,
                    push_allowed: start.push_allowed,
                    presence: None,
                })
                .await
                .map_err(|status| failure(&shared, &status, None, "The review"))?
                .into_inner();
            let operation = shared.finish(answer, "The review").await?;
            match operation.result {
                Some(pb::operation::Result::Session(session)) => Ok(map::session_id(Some(&session))),
                _ => Err(unreadable("The review")),
            }
        })
    }

    fn check_heads(&self, session: &SessionId) -> PullRequestReply<HeadCheck> {
        let shared = self.shared.clone();
        let session = session.clone();
        Box::pin(async move {
            let answer = client(&shared)
                .check_heads(pb::CheckHeadsRequest {
                    session: session_wire(&session),
                })
                .await
                .map_err(|status| failure(&shared, &status, None, "The review"))?
                .into_inner();
            Ok(head_check(&answer))
        })
    }

    fn update_clone(&self, session: &SessionId) -> PullRequestReply<()> {
        let shared = self.shared.clone();
        let session = session.clone();
        Box::pin(async move {
            let operation = client(&shared)
                .update_clone(pb::UpdateCloneRequest {
                    idempotency_key: fresh_key(),
                    session: session_wire(&session),
                })
                .await
                .map_err(|status| failure(&shared, &status, None, "The clone"))?
                .into_inner();
            shared.finish(operation, "The clone").await?;
            Ok(())
        })
    }

    fn set_push_allowed(&self, session: &SessionId, allowed: bool) -> PullRequestReply<()> {
        let shared = self.shared.clone();
        let session = session.clone();
        Box::pin(async move {
            client(&shared)
                .set_push_allowed(pb::SetPushAllowedRequest {
                    idempotency_key: fresh_key(),
                    session: session_wire(&session),
                    allowed,
                    presence: None,
                })
                .await
                .map_err(|status| failure(&shared, &status, None, "The review"))?;
            Ok(())
        })
    }

    fn plan_submit(&self, session: &SessionId) -> PullRequestReply<SubmitPlan> {
        let shared = self.shared.clone();
        let session = session.clone();
        Box::pin(async move {
            let answer = client(&shared)
                .plan_submit(pb::PlanSubmitRequest {
                    session: session_wire(&session),
                })
                .await
                .map_err(|status| failure(&shared, &status, None, "The review"))?
                .into_inner();
            plan(&answer).ok_or_else(|| unreadable("The review"))
        })
    }

    fn submit_review(
        &self,
        session: &SessionId,
        token: &str,
        reviews: Vec<ReviewSubmission>,
    ) -> PullRequestReply<Vec<SubmitResult>> {
        let shared = self.shared.clone();
        let session = session.clone();
        let token = token.to_owned();
        Box::pin(async move {
            let answer = client(&shared)
                .submit_review(pb::SubmitReviewRequest {
                    idempotency_key: fresh_key(),
                    session: session_wire(&session),
                    reviews: reviews.iter().map(review_wire).collect(),
                    plan: Some(pb::PlanToken { value: token }),
                    presence: None,
                })
                .await
                .map_err(|status| failure(&shared, &status, None, "The review"))?
                .into_inner();
            answer
                .results
                .iter()
                .map(|result| submit_result(&shared, result))
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| unreadable("The review"))
        })
    }

    fn draft_summary(
        &self,
        session: &SessionId,
        pr: &PullRequestRef,
        summary: &str,
    ) -> PullRequestReply<()> {
        let shared = self.shared.clone();
        let session = session.clone();
        let pr = pr.clone();
        let summary = summary.to_owned();
        Box::pin(async move {
            client(&shared)
                .draft_summary(pb::DraftSummaryRequest {
                    idempotency_key: fresh_key(),
                    session: session_wire(&session),
                    pr: Some(pr_wire(&pr)),
                    summary,
                })
                .await
                .map_err(|status| failure(&shared, &status, Some(&pr), "The review"))?;
            Ok(())
        })
    }

    fn list_commits(
        &self,
        session: &SessionId,
        pr: &PullRequestRef,
    ) -> PullRequestReply<PullRequestCommits> {
        let shared = self.shared.clone();
        let session = session.clone();
        let pr = pr.clone();
        Box::pin(async move {
            let answer = client(&shared)
                .list_pull_request_commits(pb::ListPullRequestCommitsRequest {
                    session: session_wire(&session),
                    pr: Some(pr_wire(&pr)),
                })
                .await
                .map_err(|status| failure(&shared, &status, Some(&pr), "These commits"))?
                .into_inner();
            commits_of(&answer).ok_or_else(|| unreadable("These commits"))
        })
    }

    fn mark_commits_reviewed(
        &self,
        session: &SessionId,
        pr: &PullRequestRef,
        commits: Vec<String>,
        reviewed: bool,
    ) -> PullRequestReply<PullRequestCommits> {
        let shared = self.shared.clone();
        let session = session.clone();
        let pr = pr.clone();
        Box::pin(async move {
            let answer = client(&shared)
                .mark_commits_reviewed(pb::MarkCommitsReviewedRequest {
                    idempotency_key: fresh_key(),
                    session: session_wire(&session),
                    pr: Some(pr_wire(&pr)),
                    commits,
                    reviewed,
                    presence: None,
                })
                .await
                .map_err(|status| failure(&shared, &status, Some(&pr), "These commits"))?
                .into_inner();
            commits_of(&answer).ok_or_else(|| unreadable("These commits"))
        })
    }

    fn watch_heads(&self) -> PullRequestReply<HeadEvents> {
        let shared = self.shared.clone();
        Box::pin(async move {
            let stream = shared
                .event_stream()
                .subscribe(pb::SubscribeRequest {
                    after: None,
                    channels: Vec::new(),
                    sessions: Vec::new(),
                    topics: vec![pb::Topic::Sessions as i32],
                })
                .await
                .map_err(|status| PullRequestError::Daemon(shared.error(&status, "Following pull requests")))?
                .into_inner();
            let (sender, receiver) = mpsc::unbounded_channel();
            shared.spawn(forward_heads(stream, sender));
            Ok(receiver)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `acme/api#<number>`, as the wire carries it.
    fn wire_pr(number: u32) -> pb::PullRequestRef {
        pb::PullRequestRef {
            repo: Some(pb::RepoRef {
                owner: "acme".to_owned(),
                name: "api".to_owned(),
            }),
            number,
            note: String::new(),
        }
    }

    fn wire_change(stack: Option<pb::StackPosition>) -> pb::HeadChange {
        pb::HeadChange {
            pr: Some(wire_pr(2)),
            new_commits: 1,
            force_pushed: false,
            state: pb::PullRequestState::Open as i32,
            base_branch: String::new(),
            stack,
        }
    }

    fn numbers(prs: &[PullRequestRef]) -> Vec<u32> {
        prs.iter().map(|pr| pr.number).collect()
    }

    #[test]
    fn a_stack_position_keeps_its_parent_and_children() {
        let wire = wire_change(Some(pb::StackPosition {
            parent: Some(wire_pr(1)),
            children: vec![wire_pr(3), wire_pr(4)],
        }));

        let stack = head_change(&wire)
            .and_then(|change| change.stack)
            .expect("the change is in a stack");

        assert_eq!(stack.parent.as_ref().map(|parent| parent.number), Some(1));
        assert_eq!(numbers(&stack.children), vec![3, 4]);
    }

    #[test]
    fn a_change_with_no_stack_position_has_none() {
        let change = head_change(&wire_change(None)).expect("the change maps");

        assert_eq!(change.stack, None);
    }

    #[test]
    fn a_child_without_a_repo_is_skipped() {
        let unmappable = pb::PullRequestRef {
            repo: None,
            number: 5,
            note: String::new(),
        };
        let wire = wire_change(Some(pb::StackPosition {
            parent: None,
            children: vec![unmappable, wire_pr(3)],
        }));

        let stack = head_change(&wire)
            .and_then(|change| change.stack)
            .expect("the change is in a stack");

        assert_eq!(stack.parent, None);
        assert_eq!(numbers(&stack.children), vec![3]);
    }
}
