//! PullRequestReviewService (PR-1..PR-17, D283-D288): paste a pull request
//! link and see what it is, start a review of one or more of them as a
//! session, follow their heads, post one review per pull request on Submit,
//! and let the review's agent push (PR-8).
//!
//! GitHub is reached only through the host's own `gh` (`github`), signed in
//! as the user; no token is read, kept or passed. A review session's
//! pull requests, the head each clone was made at and whether its agent may
//! push are the store's `review_sessions` record (migration 23).
//!
//! Each task of the PR milestone fills one submodule: `resolve` is A1, `start`
//! A2, `submit` A5 (with A4's drafts in `comments` and the MCP tools), `heads`
//! A6 and A8, `commits` A9. This file serves the service, holds the module's errors and does A3,
//! SetPushAllowed.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, OnceLock};

use humata2_access::{Call, Capability, Refusal, Target};
use humata2_grpc::humata::v1 as pb;
use humata2_sessions::Starter;
use humata2_store::{ReviewSessionRecord, Store};
use tonic::{Request, Response, Status};

use crate::channel_service::blocking;
use crate::events::Feed;
use crate::guard::Guard;
use crate::identity::{self, Caller};
use crate::session_service::{SessionSvc, Tell};
use crate::status;

mod clone_git;
mod commits;
mod github;
mod heads;
mod poll;
mod resolve;
mod start;
mod submit;

pub(crate) use start::PR_REVIEWS_FOLDER;

/// The service's shared state.
#[derive(Clone)]
pub(crate) struct PullRequests {
    inner: Arc<Inner>,
}

struct Inner {
    store: Arc<Store>,
    guard: Arc<Guard>,
    machine: String,
    /// The channels folder, the store and the remote template a review's
    /// channel and clones are made with.
    starter: Starter,
    /// Puts a session as the store holds it now on the event feed.
    tell: Tell,
    /// The event feed channels are told on.
    feed: Arc<Feed>,
    /// The agent the PR reviews channel is made with.
    default_agent: String,
    /// One review's channel setup at a time.
    channel: tokio::sync::Mutex<()>,
    /// The session service that starts a review's session; set once.
    sessions: OnceLock<SessionSvc>,
    /// The review chat's state: its comments and the terminal sink.
    review: crate::review::Review,
    operations: Arc<crate::operations::Operations>,
    /// Submit plans not yet taken.
    submits: submit::Plans,
    /// Each review's last check of its heads (A8, D298).
    checks: heads::Checks,
}

impl PullRequests {
    pub(crate) fn new(
        store: Arc<Store>,
        guard: Arc<Guard>,
        machine: String,
        starter: Starter,
        tell: Tell,
        feed: Arc<Feed>,
        default_agent: String,
        review: crate::review::Review,
        operations: Arc<crate::operations::Operations>,
    ) -> Self {
        let inner = Arc::new(Inner {
            store,
            guard,
            machine,
            starter,
            tell,
            feed,
            default_agent,
            channel: tokio::sync::Mutex::new(()),
            sessions: OnceLock::new(),
            review,
            operations,
            submits: submit::Plans::default(),
            checks: heads::Checks::default(),
        });
        // An archived or freed session is checked no more: its last check goes.
        let held = Arc::downgrade(&inner);
        inner.tell.on_ended(move |session| {
            if let Some(inner) = held.upgrade() {
                inner.checks.forget(session);
            }
        });
        Self { inner }
    }

    /// Gives the service the session service that starts a review's
    /// session. Once.
    pub(crate) fn use_sessions(&self, sessions: SessionSvc) {
        // PR-4, PR-21: a review on another machine is looked at there.
        if let Some(remote) = sessions.remote_work() {
            clone_git::install(self.inner.store.clone(), self.inner.machine.clone(), remote);
        }
        let _ = self.inner.sessions.set(sessions);
    }
}

/// What went wrong with a pull request lookup, before it is said as a status.
#[derive(Debug, thiserror::Error)]
enum PullRequestError {
    #[error("the request names no {field}")]
    Missing { field: &'static str },
    #[error("{given:?} is not a GitHub pull request link or an owner/repo#number")]
    Reference { given: String },
    #[error("couldn't ask GitHub about {target}")]
    Github {
        target: resolve::PrTarget,
        #[source]
        source: github::GithubError,
    },
    #[error("the GitHub CLI's answer for {target} has no usable {field}")]
    Answer {
        target: resolve::PrTarget,
        field: &'static str,
    },
}

impl PullRequestError {
    fn status(&self) -> Status {
        match self {
            Self::Missing { field } => status::missing(field),
            Self::Reference { .. } => status::invalid_field(
                "ref",
                pb::invalid_field::Problem::BadFormat,
                status::chain(self),
                "paste the pull request's link from the browser, https://github.com/<owner>/<repo>/pull/<number>, or write it as <owner>/<repo>#<number>",
            ),
            Self::Github { target, source } => {
                github::status_of(source, Some(&target.repo()), Some(target.number))
            }
            Self::Answer { .. } => crate::channel_service::failed(
                tonic::Code::FailedPrecondition,
                self,
                "update the GitHub CLI on this Mac (`brew upgrade gh`) and try again; if it still fails, GitHub's data for this pull request is unusual and a review can't be started from it",
                Some(pb::error::Kind::GithubCli(pb::GitHubCliProblem {
                    problem: pb::git_hub_cli_problem::Problem::ApiError as i32,
                    ..pb::GitHubCliProblem::default()
                })),
            ),
        }
    }
}

/// A pull request review session's binding, as the Session shows it.
pub(crate) fn binding(record: &ReviewSessionRecord) -> pb::PullRequestBinding {
    pb::PullRequestBinding {
        pull_requests: record
            .prs
            .iter()
            .map(|pr| pb::PullRequestRef {
                repo: Some(pb::RepoRef {
                    owner: pr.owner.clone(),
                    name: pr.name.clone(),
                }),
                number: pr.number,
                ..pb::PullRequestRef::default()
            })
            .collect(),
        push_allowed: record.push_allowed,
    }
}

/// PR-10: the ref each of `keys` (the Diff's repo keys, `owner/name` or a
/// clone's folder name) is compared with under PullRequestBase: its pull
/// request's base on the remote, fetched into the clone in `folder` if it
/// isn't there. A repo without a pull request is left out, and so is every
/// repo of a session that reviews none. Blocking.
pub(super) fn pull_request_bases_blocking(
    store: &Store,
    machine: &str,
    session: &str,
    folder: &Path,
    keys: &[String],
) -> Result<BTreeMap<String, String>, Status> {
    let Some(review) = store
        .review_session(session)
        .map_err(|error| status::store(error, machine, "read the review's pull requests"))?
    else {
        return Ok(BTreeMap::new());
    };
    let mut bases = BTreeMap::new();
    for key in keys {
        let (owner, name) = key
            .rsplit_once('/')
            .map_or((None, key.as_str()), |(owner, name)| (Some(owner), name));
        let Some(pr) = review.prs.iter().find(|pr| {
            pr.name == name
                && owner.is_none_or(|owner| pr.owner == owner)
                && !pr.base_branch.is_empty()
        }) else {
            continue;
        };
        let base = heads::base_ref_blocking(&folder.join(name), &pr.base_branch)
            .map_err(|error| error.status())?;
        bases.insert(key.clone(), base);
    }
    Ok(bases)
}

/// The machine a session on another machine runs on, with its link's
/// clients (or the offline status); `None` for a session on this machine.
pub(super) fn relayed_for(session: &str) -> Option<(String, Result<crate::host_relay::Relayed, Status>)> {
    clone_git::relayed_for(session)
}

/// The machine a session on another machine runs on; `None` for a session
/// on this machine.
pub(super) fn machine_of(session: &str) -> Option<String> {
    clone_git::machine_of(session)
}

/// A9, D299: RV-6 Mark reviewed in a pull request review also marks every
/// commit up to the head of each repo in `snapshots` (key, snapshot n) that
/// is one of its pull requests, as a whole-PR review by `actor`. Returns
/// those repos and snapshots, for the caller to pin. Blocking.
pub(super) fn mark_whole_pr_blocking(
    store: &Store,
    machine: &str,
    session: &str,
    folder: &Path,
    snapshots: &[(String, u32)],
    actor: &pb::Actor,
) -> Result<Vec<(String, u32)>, Status> {
    commits::mark_whole_blocking(store, machine, session, folder, snapshots, actor)
}

impl PullRequests {
    /// A3, PR-8: the user lets the review's agent push, or stops it. Only
    /// the user: the agent that would gain from it is refused as user-only.
    async fn set_push_allowed_of(
        &self,
        caller: &Caller,
        request: pb::SetPushAllowedRequest,
    ) -> Result<pb::SetPushAllowedResponse, Status> {
        let id = request
            .session
            .as_ref()
            .map(|session| session.value.clone())
            .filter(|session| !session.is_empty())
            .ok_or_else(|| status::missing("session"))?;
        let machine = self.inner.machine.clone();
        let (store, key) = (self.inner.store.clone(), id.clone());
        let record = blocking(move || store.session(&key))
            .await?
            .map_err(|error| status::store(error, &machine, "look up the session"))?
            .ok_or_else(|| {
                status::not_found(
                    pb::EntityKind::Session,
                    &id,
                    format!("no session {id} is on this machine"),
                    "list the sessions to see their ids; a review's session is on the machine it was started on",
                )
            })?;
        let session = identity::session_ref(&record, &self.inner.machine);
        self.inner.guard.require(
            caller,
            &Call::Use {
                capability: Capability::UserReviewState,
                target: Target::Session(session),
            },
        )?;
        let (store, key, allowed) = (self.inner.store.clone(), id.clone(), request.allowed);
        let machine = self.inner.machine.clone();
        let is_review = blocking(move || store.set_review_push_allowed(&key, allowed))
            .await?
            .map_err(|error| status::store(error, &machine, "change whether the review may push"))?;
        if !is_review {
            return Err(status::invalid_field(
                "session",
                pb::invalid_field::Problem::NotAllowedHere,
                format!("session {id} is not a pull request review, so there is no push to allow"),
                "turn this on in a review session's settings; a session that isn't a pull request review pushes under the channel's own rules",
            ));
        }
        // The binding is on the Session the feed shows.
        self.inner
            .tell
            .session(&id, pb::Change::Updated, Some(identity::actor(caller)))
            .await;
        Ok(pb::SetPushAllowedResponse {})
    }
}

impl PullRequests {
    /// D288: the user, or the agent of a session once the user has let it
    /// start a PR review (the guard asks in the Inbox when the switch is
    /// Ask). Resolving is a read of GitHub, so it rides the same approval.
    fn require_user_or_asked(&self, caller: &Caller) -> Result<(), Status> {
        match caller {
            Caller::Agent(agent) => self.inner.guard.require(
                caller,
                &Call::Use {
                    capability: Capability::PrReviewStart,
                    target: Target::Channel(agent.session.channel.clone()),
                },
            ),
            _ => require_user(caller),
        }
    }
}

/// Only the user may look a pull request up or start a review of it for
/// now: both run `gh` as the user.
fn require_user(caller: &Caller) -> Result<(), Status> {
    match caller {
        Caller::User(_) => Ok(()),
        _ => Err(status::refused(&Refusal::UserOnly {
            capability: Some(Capability::PrReviewStart),
        })),
    }
}

#[tonic::async_trait]
impl pb::pull_request_review_service_server::PullRequestReviewService for PullRequests {
    async fn resolve_pull_request(
        &self,
        request: Request<pb::ResolvePullRequestRequest>,
    ) -> Result<Response<pb::PullRequestInfo>, Status> {
        let caller = identity::caller(&request)?;
        self.require_user_or_asked(&caller)?;
        self.resolve(request.into_inner()).await.map(Response::new)
    }

    async fn start_review(
        &self,
        request: Request<pb::StartReviewRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        let caller = identity::caller(&request)?;
        self.require_user_or_asked(&caller)?;
        self.start_review_of(&caller, request.into_inner())
            .await
            .map(Response::new)
    }

    async fn check_heads(
        &self,
        request: Request<pb::CheckHeadsRequest>,
    ) -> Result<Response<pb::CheckHeadsResponse>, Status> {
        let caller = identity::caller(&request)?;
        self.check_heads_of(&caller, request.into_inner())
            .await
            .map(Response::new)
    }

    async fn update_clone(
        &self,
        request: Request<pb::UpdateCloneRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        let caller = identity::caller(&request)?;
        self.update_clone_of(&caller, request.into_inner())
            .await
            .map(Response::new)
    }

    async fn set_push_allowed(
        &self,
        request: Request<pb::SetPushAllowedRequest>,
    ) -> Result<Response<pb::SetPushAllowedResponse>, Status> {
        let caller = identity::caller(&request)?;
        self.set_push_allowed_of(&caller, request.into_inner())
            .await
            .map(Response::new)
    }

    async fn plan_submit(
        &self,
        request: Request<pb::PlanSubmitRequest>,
    ) -> Result<Response<pb::SubmitPlan>, Status> {
        let caller = identity::caller(&request)?;
        self.plan_submit_of(&caller, request.into_inner())
            .await
            .map(Response::new)
    }

    async fn submit_review(
        &self,
        request: Request<pb::SubmitReviewRequest>,
    ) -> Result<Response<pb::SubmitReviewResponse>, Status> {
        let caller = identity::caller(&request)?;
        self.submit_review_of(&caller, request.into_inner())
            .await
            .map(Response::new)
    }

    async fn draft_summary(
        &self,
        request: Request<pb::DraftSummaryRequest>,
    ) -> Result<Response<pb::DraftSummaryResponse>, Status> {
        let caller = identity::caller(&request)?;
        self.draft_summary_of(&caller, request.into_inner())
            .await
            .map(Response::new)
    }

    async fn list_pull_request_commits(
        &self,
        request: Request<pb::ListPullRequestCommitsRequest>,
    ) -> Result<Response<pb::PullRequestCommits>, Status> {
        let caller = identity::caller(&request)?;
        self.list_commits_of(&caller, request.into_inner())
            .await
            .map(Response::new)
    }

    async fn mark_commits_reviewed(
        &self,
        request: Request<pb::MarkCommitsReviewedRequest>,
    ) -> Result<Response<pb::PullRequestCommits>, Status> {
        let caller = identity::caller(&request)?;
        self.mark_commits_of(&caller, request.into_inner())
            .await
            .map(Response::new)
    }
}
