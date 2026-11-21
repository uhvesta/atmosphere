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
use humata2_daemon_ports::Caller;
use crate::pb;
use humata2_rpc_kit::{blocking, chain, failed, internal, invalid_field, missing, not_found, now_ms};
use humata2_store::{ReviewSessionRecord, Store};
use tonic::{Request, Response, Status};

use crate::ports::{
    ReviewFeed, ReviewGuard, ReviewOperations, ReviewSessions, ReviewStarter, ReviewTell, actor,
    caller_of, refused, session_ref, store_error,
};

mod clone_git;
mod commits;
pub mod feedback;
mod github;
mod heads;
pub mod inbox_poll;
mod poll;
mod resolve;
mod start;
mod submit;

pub use clone_git::ReviewRemote;
pub(super) use clone_git::remote_for as relayed_for;
#[allow(unused_imports)]
pub use feedback::{
    CollapsedBots, FeedbackError, FeedbackThread, PrFeedback, PrLifecycleState, ThreadComment,
    TopLevelReview, deduplicate_threads, fetch_pr_feedback, parse_pr_feedback,
};
#[allow(unused_imports)]
pub use inbox_poll::{
    InboxNotification, InboxNotificationKind, InboxPollError, InboxPollState, PollFetchResult,
    PollTrigger, PrPollCursor, ReviewInboxPoller, ReviewRequestEntry, WatchedPr,
    DEFAULT_POLL_INTERVAL,
};
pub use start::PR_REVIEWS_FOLDER;

/// The service's shared state.
#[derive(Clone)]
pub struct PullRequests {
    inner: Arc<Inner>,
}

struct Inner {
    store: Arc<Store>,
    guard: Arc<dyn ReviewGuard>,
    machine: String,
    /// The channels folder, the store and the remote template a review's
    /// channel and clones are made with.
    starter: Arc<dyn ReviewStarter>,
    /// Puts a session as the store holds it now on the event feed.
    tell: Arc<dyn ReviewTell>,
    /// The event feed channels are told on.
    feed: Arc<dyn ReviewFeed>,
    /// The agent the PR reviews channel is made with.
    default_agent: String,
    /// One review's channel setup at a time.
    channel: tokio::sync::Mutex<()>,
    /// The session service that starts a review's session; set once.
    sessions: OnceLock<Arc<dyn ReviewSessions>>,
    /// The review chat's state: its comments and the terminal sink.
    review: crate::Review,
    operations: Arc<dyn ReviewOperations>,
    /// Submit plans not yet taken.
    submits: submit::Plans,
    /// Each review's last check of its heads (A8, D298).
    checks: heads::Checks,
    /// Serializes read-modify-write updates to `review.watched_prs`.
    watched_prs: tokio::sync::Mutex<()>,
}

const WATCHED_PRS_SETTING: &str = "review.watched_prs";

impl PullRequests {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        store: Arc<Store>,
        guard: Arc<dyn ReviewGuard>,
        machine: String,
        starter: Arc<dyn ReviewStarter>,
        tell: Arc<dyn ReviewTell>,
        feed: Arc<dyn ReviewFeed>,
        default_agent: String,
        review: crate::Review,
        operations: Arc<dyn ReviewOperations>,
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
            watched_prs: tokio::sync::Mutex::new(()),
        });
        Self { inner }
    }

    /// Gives the service the session service that starts a review's
    /// session. Once.
    pub fn use_sessions(&self, sessions: Arc<dyn ReviewSessions>) {
        let _ = self.inner.sessions.set(sessions);
    }

    /// Installs the remote link used to reach review clones on other machines.
    pub fn install_remote(&self, remote: Arc<dyn ReviewRemote>) {
        clone_git::install(self.inner.store.clone(), self.inner.machine.clone(), remote);
    }

    /// Drops the session's cached head check (called when a session is archived
    /// or freed).
    pub fn forget_checks(&self, session: &str) {
        self.inner.checks.forget(session);
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
            Self::Missing { field } => missing(field),
            Self::Reference { .. } => invalid_field(
                "ref",
                pb::invalid_field::Problem::BadFormat,
                chain(self),
                "paste the pull request's link from the browser, https://github.com/<owner>/<repo>/pull/<number>, or write it as <owner>/<repo>#<number>",
            ),
            Self::Github { target, source } => {
                github::status_of(source, Some(&target.repo()), Some(target.number))
            }
            Self::Answer { .. } => failed(
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
pub fn binding(record: &ReviewSessionRecord) -> pb::PullRequestBinding {
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
pub(crate) fn pull_request_bases_blocking(
    store: &Store,
    machine: &str,
    session: &str,
    folder: &Path,
    keys: &[String],
) -> Result<BTreeMap<String, String>, Status> {
    let Some(review) = store
        .review_session(session)
        .map_err(|error| store_error(error, machine, "read the review's pull requests"))?
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

/// The machine a session on another machine runs on; `None` for a session
/// on this machine.
pub(crate) fn machine_of(session: &str) -> Option<String> {
    clone_git::machine_of(session)
}

/// A9, D299: RV-6 Mark reviewed in a pull request review also marks every
/// commit up to the head of each repo in `snapshots` (key, snapshot n) that
/// is one of its pull requests, as a whole-PR review by `actor`. Returns
/// those repos and snapshots, for the caller to pin. Blocking.
pub(crate) fn mark_whole_pr_blocking(
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
            .ok_or_else(|| missing("session"))?;
        let machine = self.inner.machine.clone();
        let (store, key) = (self.inner.store.clone(), id.clone());
        let record = blocking(move || store.session(&key))
            .await?
            .map_err(|error| store_error(error, &machine, "look up the session"))?
            .ok_or_else(|| {
                not_found(
                    pb::EntityKind::Session,
                    &id,
                    format!("no session {id} is on this machine"),
                    "list the sessions to see their ids; a review's session is on the machine it was started on",
                )
            })?;
        let session = session_ref(&record, &self.inner.machine);
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
            .map_err(|error| store_error(error, &machine, "change whether the review may push"))?;
        if !is_review {
            return Err(invalid_field(
                "session",
                pb::invalid_field::Problem::NotAllowedHere,
                format!("session {id} is not a pull request review, so there is no push to allow"),
                "turn this on in a review session's settings; a session that isn't a pull request review pushes under the channel's own rules",
            ));
        }
        // The binding is on the Session the feed shows.
        self.inner
            .tell
            .session(&id, pb::Change::Updated, Some(actor(caller)))
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
        _ => Err(refused(&Refusal::UserOnly {
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
        let caller = caller_of(&request)?;
        self.require_user_or_asked(&caller)?;
        self.resolve(request.into_inner()).await.map(Response::new)
    }

    async fn start_review(
        &self,
        request: Request<pb::StartReviewRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        let caller = caller_of(&request)?;
        self.require_user_or_asked(&caller)?;
        self.start_review_of(&caller, request.into_inner())
            .await
            .map(Response::new)
    }

    async fn check_heads(
        &self,
        request: Request<pb::CheckHeadsRequest>,
    ) -> Result<Response<pb::CheckHeadsResponse>, Status> {
        let caller = caller_of(&request)?;
        self.check_heads_of(&caller, request.into_inner())
            .await
            .map(Response::new)
    }

    async fn update_clone(
        &self,
        request: Request<pb::UpdateCloneRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        let caller = caller_of(&request)?;
        self.update_clone_of(&caller, request.into_inner())
            .await
            .map(Response::new)
    }

    async fn set_push_allowed(
        &self,
        request: Request<pb::SetPushAllowedRequest>,
    ) -> Result<Response<pb::SetPushAllowedResponse>, Status> {
        let caller = caller_of(&request)?;
        self.set_push_allowed_of(&caller, request.into_inner())
            .await
            .map(Response::new)
    }

    async fn plan_submit(
        &self,
        request: Request<pb::PlanSubmitRequest>,
    ) -> Result<Response<pb::SubmitPlan>, Status> {
        let caller = caller_of(&request)?;
        self.plan_submit_of(&caller, request.into_inner())
            .await
            .map(Response::new)
    }

    async fn submit_review(
        &self,
        request: Request<pb::SubmitReviewRequest>,
    ) -> Result<Response<pb::SubmitReviewResponse>, Status> {
        let caller = caller_of(&request)?;
        self.submit_review_of(&caller, request.into_inner())
            .await
            .map(Response::new)
    }

    async fn draft_summary(
        &self,
        request: Request<pb::DraftSummaryRequest>,
    ) -> Result<Response<pb::DraftSummaryResponse>, Status> {
        let caller = caller_of(&request)?;
        self.draft_summary_of(&caller, request.into_inner())
            .await
            .map(Response::new)
    }

    async fn list_pull_request_commits(
        &self,
        request: Request<pb::ListPullRequestCommitsRequest>,
    ) -> Result<Response<pb::PullRequestCommits>, Status> {
        let caller = caller_of(&request)?;
        self.list_commits_of(&caller, request.into_inner())
            .await
            .map(Response::new)
    }

    async fn mark_commits_reviewed(
        &self,
        request: Request<pb::MarkCommitsReviewedRequest>,
    ) -> Result<Response<pb::PullRequestCommits>, Status> {
        let caller = caller_of(&request)?;
        self.mark_commits_of(&caller, request.into_inner())
            .await
            .map(Response::new)
    }

    async fn watch_pull_request(
        &self,
        request: Request<pb::WatchPullRequestRequest>,
    ) -> Result<Response<pb::WatchPullRequestResponse>, Status> {
        let _caller = caller_of(&request)?;
        self.watch_pull_request_of(request.into_inner())
            .await
            .map(Response::new)
    }

    async fn unwatch_pull_request(
        &self,
        request: Request<pb::UnwatchPullRequestRequest>,
    ) -> Result<Response<pb::UnwatchPullRequestResponse>, Status> {
        let _caller = caller_of(&request)?;
        self.unwatch_pull_request_of(request.into_inner())
            .await
            .map(Response::new)
    }

    async fn list_watched_pull_requests(
        &self,
        request: Request<pb::ListWatchedPullRequestsRequest>,
    ) -> Result<Response<pb::ListWatchedPullRequestsResponse>, Status> {
        let _caller = caller_of(&request)?;
        self.list_watched_pull_requests_of()
            .await
            .map(Response::new)
    }
}

fn load_watched_prs(
    store: &Store,
    machine: &str,
) -> Result<BTreeMap<String, inbox_poll::WatchedPr>, Status> {
    let raw = store
        .setting(WATCHED_PRS_SETTING)
        .map_err(|error| store_error(error, machine, "read watched pull requests"))?;
    match raw {
        None => Ok(BTreeMap::new()),
        Some(saved) if saved.value.trim().is_empty() => Ok(BTreeMap::new()),
        Some(saved) => serde_json::from_str(&saved.value).map_err(|error| {
            internal(
                format!("couldn't decode saved watched pull requests: {error}"),
                "",
            )
        }),
    }
}

fn save_watched_prs(
    store: &Store,
    machine: &str,
    watched: &BTreeMap<String, inbox_poll::WatchedPr>,
) -> Result<(), Status> {
    let json = serde_json::to_string(watched)
        .map_err(|error| internal(format!("couldn't encode watched pull requests: {error}"), ""))?;
    store
        .put_setting(
            WATCHED_PRS_SETTING,
            &json,
            u64::try_from(now_ms()).unwrap_or_default(),
        )
        .map_err(|error| store_error(error, machine, "save watched pull requests"))
}

fn watched_to_pb(pr: &inbox_poll::WatchedPr) -> pb::PullRequestRef {
    pb::PullRequestRef {
        repo: Some(pb::RepoRef {
            owner: pr.owner.clone(),
            name: pr.name.clone(),
        }),
        number: pr.number,
        ..pb::PullRequestRef::default()
    }
}

impl PullRequests {
    async fn watch_pull_request_of(
        &self,
        request: pb::WatchPullRequestRequest,
    ) -> Result<pb::WatchPullRequestResponse, Status> {
        let watched = inbox_poll::parse_pr_url(&request.url).map_err(|error| {
            invalid_field(
                "url",
                pb::invalid_field::Problem::BadFormat,
                chain(&error),
                "pass a GitHub pull request URL (https://github.com/<owner>/<repo>/pull/<number>) or <owner>/<repo>#<number>",
            )
        })?;
        let _guard = self.inner.watched_prs.lock().await;
        let (store, machine, entry) = (
            self.inner.store.clone(),
            self.inner.machine.clone(),
            watched.clone(),
        );
        blocking(move || {
            let mut map = load_watched_prs(&store, &machine)?;
            map.insert(entry.key(), entry);
            save_watched_prs(&store, &machine, &map)
        })
        .await??;
        Ok(pb::WatchPullRequestResponse {
            pr: Some(watched_to_pb(&watched)),
            url: watched.url,
        })
    }

    async fn unwatch_pull_request_of(
        &self,
        request: pb::UnwatchPullRequestRequest,
    ) -> Result<pb::UnwatchPullRequestResponse, Status> {
        let pr = request.pr.ok_or_else(|| missing("pr"))?;
        let repo = pr.repo.unwrap_or_default();
        if repo.owner.is_empty() || repo.name.is_empty() || pr.number == 0 {
            return Err(invalid_field(
                "pr",
                pb::invalid_field::Problem::BadFormat,
                format!(
                    "{}/{}#{} is not a valid pull request reference",
                    repo.owner, repo.name, pr.number
                ),
                "provide a pull request with non-empty repo owner, repo name, and positive number",
            ));
        }
        let key = format!("{}/{}#{}", repo.owner, repo.name, pr.number);
        let _guard = self.inner.watched_prs.lock().await;
        let (store, machine) = (self.inner.store.clone(), self.inner.machine.clone());
        let removed = blocking(move || {
            let mut map = load_watched_prs(&store, &machine)?;
            let removed = map.remove(&key).is_some();
            if removed {
                save_watched_prs(&store, &machine, &map)?;
            }
            Ok::<_, Status>(removed)
        })
        .await??;
        Ok(pb::UnwatchPullRequestResponse { removed })
    }

    async fn list_watched_pull_requests_of(
        &self,
    ) -> Result<pb::ListWatchedPullRequestsResponse, Status> {
        let _guard = self.inner.watched_prs.lock().await;
        let (store, machine) = (self.inner.store.clone(), self.inner.machine.clone());
        let prs = blocking(move || {
            let map = load_watched_prs(&store, &machine)?;
            Ok::<_, Status>(map.values().map(watched_to_pb).collect())
        })
        .await??;
        Ok(pb::ListWatchedPullRequestsResponse { prs })
    }
}

#[cfg(test)]
mod watched_pr_tests {
    use std::path::PathBuf;

    use super::*;
    use crate::pb::pull_request_review_service_server::PullRequestReviewService;
    use crate::ports::{Checked, ReviewOperation, Scope};
    use humata2_store::ChannelRecord;

    struct DummyGuard;
    impl ReviewGuard for DummyGuard {
        fn check(&self, _caller: &Caller, _call: &Call) -> Result<Checked, Status> {
            Ok(Checked::Allowed)
        }

        fn require(&self, _caller: &Caller, _call: &Call) -> Result<(), Status> {
            Ok(())
        }
    }

    struct DummyStarter;
    impl ReviewStarter for DummyStarter {
        fn channels_root(&self) -> PathBuf {
            PathBuf::from("/tmp/humata-dummy")
        }

        fn remote_template(&self) -> String {
            String::new()
        }

        fn create_pr_reviews_channel(
            &self,
            _machine: &str,
            _default_agent: &str,
        ) -> Result<ChannelRecord, Status> {
            Err(Status::unimplemented("unused"))
        }
    }

    struct DummyTell;
    #[tonic::async_trait]
    impl ReviewTell for DummyTell {
        async fn session(&self, _id: &str, _change: pb::Change, _by: Option<pb::Actor>) {}
    }

    struct DummyFeed;
    impl ReviewFeed for DummyFeed {
        fn send(
            &self,
            _scope: Scope,
            _change: pb::Change,
            _payload: pb::event::Payload,
            _by: Option<pb::Actor>,
        ) {
        }
    }

    struct DummySink;
    #[tonic::async_trait]
    impl crate::CommentSink for DummySink {
        async fn target(&self, _session: &str) -> Result<crate::sink::Target, Status> {
            Err(Status::unimplemented("unused"))
        }

        async fn paste(
            &self,
            _target: &crate::sink::Target,
            _key: &str,
            _paste: &[u8],
        ) -> crate::sink::Pasted {
            crate::sink::Pasted::NotTyped(Status::unimplemented("unused"))
        }

        async fn arrived(
            &self,
            _target: &crate::sink::Target,
            _key: &str,
        ) -> crate::sink::Arrived {
            crate::sink::Arrived::Unknown
        }
    }

    struct DummyOperations;
    impl ReviewOperations for DummyOperations {
        fn start(
            &self,
            _kind: pb::OperationKind,
            _title: String,
            _steps: Vec<String>,
            _actor: Option<pb::Actor>,
        ) -> Box<dyn ReviewOperation> {
            unreachable!("unused")
        }
    }

    fn test_service(store: Arc<Store>) -> PullRequests {
        let guard: Arc<dyn ReviewGuard> = Arc::new(DummyGuard);
        let feed: Arc<dyn ReviewFeed> = Arc::new(DummyFeed);
        let review = crate::Review::new(
            store.clone(),
            guard.clone(),
            "this-mac".to_owned(),
            feed.clone(),
            Arc::new(DummySink),
        );
        PullRequests::new(
            store,
            guard,
            "this-mac".to_owned(),
            Arc::new(DummyStarter),
            Arc::new(DummyTell),
            feed,
            "claude".to_owned(),
            review,
            Arc::new(DummyOperations),
        )
    }

    fn user_req<T>(body: T) -> Request<T> {
        let mut req = Request::new(body);
        req.extensions_mut()
            .insert(Caller::User(humata2_daemon_ports::UserVia::App));
        req
    }

    #[tokio::test]
    async fn watch_list_unwatch_pull_request_roundtrip_and_invalid_url() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(&dir.path().join("humata.db")).unwrap());
        let svc = test_service(store);

        // Invalid URL is rejected with InvalidArgument.
        let err = svc
            .watch_pull_request(user_req(pb::WatchPullRequestRequest {
                url: "not-a-pr-url".to_owned(),
            }))
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);

        // Watch two PRs (full URL + short ref).
        let w1 = svc
            .watch_pull_request(user_req(pb::WatchPullRequestRequest {
                url: "https://github.com/acme/api/pull/42/files".to_owned(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(w1.url, "https://github.com/acme/api/pull/42");
        assert_eq!(w1.pr.as_ref().map(|p| p.number), Some(42));

        let w2 = svc
            .watch_pull_request(user_req(pb::WatchPullRequestRequest {
                url: "acme/web#7".to_owned(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(w2.url, "https://github.com/acme/web/pull/7");

        // List returns both PRs.
        let listed = svc
            .list_watched_pull_requests(user_req(pb::ListWatchedPullRequestsRequest {}))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(listed.prs.len(), 2);

        // Unwatch one PR -> removed: true, second unwatch -> removed: false.
        let unwatched = svc
            .unwatch_pull_request(user_req(pb::UnwatchPullRequestRequest { pr: w1.pr.clone() }))
            .await
            .unwrap()
            .into_inner();
        assert!(unwatched.removed);

        let unwatched_again = svc
            .unwatch_pull_request(user_req(pb::UnwatchPullRequestRequest { pr: w1.pr }))
            .await
            .unwrap()
            .into_inner();
        assert!(!unwatched_again.removed);

        // List now returns only the remaining PR.
        let remaining = svc
            .list_watched_pull_requests(user_req(pb::ListWatchedPullRequestsRequest {}))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(remaining.prs.len(), 1);
        assert_eq!(remaining.prs[0].number, 7);
    }
}
