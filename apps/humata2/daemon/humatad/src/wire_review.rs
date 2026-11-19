//! Wiring adapter connecting `humata2_svc_review` to `humatad_core`'s concrete
//! `Guard`, `Feed`, `Operations`, `SessionSvc`, `Tell`, `Starter`, and
//! `terminal::Core` (Wiring Wave 1).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use humata2_access::Call;
use humata2_daemon_ports::{Caller, UserVia};
use humata2_grpc::humata::v1 as pb;
use humata2_sessions::Starter;
use humata2_store::{ChannelRecord, Store};
use humata2_svc_peer::ports::{PeerReviewPort, Replayed as PeerReplayed};
use humata2_svc_review::ports::{
    Checked, CommentsArrived, CommentsTarget, CommentsTyped, ReviewFeed, ReviewGuard,
    ReviewOperation, ReviewOperations, ReviewSessions, ReviewStarter, ReviewTell,
    ReviewTerminalCore, Scope,
};
use humata2_svc_review::{PullRequests, RecoveryState, Review, TerminalSink, UserWriteGuard};
use humatad_core::events::{Feed, Scope as EventScope};
use humatad_core::files::{UserWrite, UserWrites};
use humatad_core::guard::{Checked as GuardChecked, Guard};
use humatad_core::operations::{Operations, Progress, Started};
use humatad_core::plugins::{
    ReviewContext, ReviewDraftsPort, ReviewInstance, ReviewPlugin as ReviewPluginTrait,
};
use humatad_core::restart::OperationState;
use humatad_core::session_service::{SessionSvc, Tell};
use humatad_core::terminal::Core as TerminalCore;
use tonic::Status;
use tonic::service::RoutesBuilder;

fn to_local_caller(caller: &Caller) -> Caller {
    caller.clone()
}

/// Adapts `humata2_svc_review::Review` to `crate::files::UserWrites`.
pub(crate) struct UserWritesAdapter(pub(crate) Review);

struct UserWriteAdapter(UserWriteGuard);

#[tonic::async_trait]
impl UserWrite for UserWriteAdapter {
    async fn wrote(self: Box<Self>, repo: &pb::RepoRef, path: &str) {
        self.0.wrote(repo, path).await;
    }
}

impl UserWrites for UserWritesAdapter {
    fn writing(&self, session: &str) -> Option<Box<dyn UserWrite>> {
        self.0
            .begin_user_write(session)
            .map(|guard| Box::new(UserWriteAdapter(guard)) as Box<dyn UserWrite>)
    }
}

/// Converts `humata2_svc_review::RecoveryState` to `crate::restart::OperationState`.
pub(crate) fn recovery_state(state: RecoveryState) -> OperationState {
    match state {
        RecoveryState::Pending { waits_for } => OperationState::Pending { waits_for },
        RecoveryState::Finished => OperationState::Finished,
        RecoveryState::Stuck { reason } => OperationState::Stuck { reason },
    }
}

/// Adapts `crate::guard::Guard` to [`ReviewGuard`].
struct GuardAdapter(Arc<Guard>);

impl ReviewGuard for GuardAdapter {
    fn check(&self, caller: &Caller, call: &Call) -> Result<Checked, Status> {
        let local = to_local_caller(caller);
        match self.0.check(&local, call)? {
            GuardChecked::Allowed => Ok(Checked::Allowed),
            GuardChecked::Ask(asked) => Ok(Checked::Ask(Box::new(asked))),
        }
    }

    fn require(&self, caller: &Caller, call: &Call) -> Result<(), Status> {
        let local = to_local_caller(caller);
        self.0.require(&local, call)
    }
}

/// Adapts `crate::events::Feed` to [`ReviewFeed`].
struct FeedAdapter(Arc<Feed>);

impl ReviewFeed for FeedAdapter {
    fn send(
        &self,
        scope: Scope,
        change: pb::Change,
        payload: pb::event::Payload,
        actor: Option<pb::Actor>,
    ) {
        match scope {
            Scope::Global => self.0.publish(change, payload, actor),
            Scope::Channel(channel) => self.0.publish_in(
                change,
                payload,
                EventScope {
                    channel,
                    session: String::new(),
                },
                actor,
            ),
            Scope::Session(session) => self.0.publish_in(
                change,
                payload,
                EventScope {
                    channel: String::new(),
                    session,
                },
                actor,
            ),
        }
    }
}

/// Adapts `crate::session_service::Tell` to [`ReviewTell`].
struct TellAdapter(Tell);

#[tonic::async_trait]
impl ReviewTell for TellAdapter {
    async fn session(&self, id: &str, change: pb::Change, actor: Option<pb::Actor>) {
        self.0.session(id, change, actor).await;
    }
}

/// Adapts `crate::session_service::SessionSvc` to [`ReviewSessions`].
pub(crate) struct SessionsAdapter(pub(crate) Arc<SessionSvc>);

#[tonic::async_trait]
impl ReviewSessions for SessionsAdapter {
    async fn begin_review(
        &self,
        caller: &Caller,
        channel: ChannelRecord,
        request: &pb::StartReviewRequest,
        prs: Vec<humata2_store::ReviewPr>,
        push_allowed: bool,
        title: String,
        first_message: String,
    ) -> Result<pb::StartSessionResponse, Status> {
        let local = to_local_caller(caller);
        let review = humata2_sessions::ReviewStart { prs, push_allowed };
        let operation = self
            .0
            .begin_review(
                &local,
                channel,
                request,
                review,
                title,
                first_message,
                None,
            )
            .await?;
        Ok(pb::StartSessionResponse {
            session: None,
            operation: Some(operation),
        })
    }
}

/// Adapts `humata2_sessions::Starter` to [`ReviewStarter`].
struct StarterAdapter(Starter);

impl ReviewStarter for StarterAdapter {
    fn channels_root(&self) -> PathBuf {
        self.0.layout.root().to_path_buf()
    }

    fn remote_template(&self) -> String {
        self.0.remote_template.clone()
    }

    fn create_pr_reviews_channel(
        &self,
        machine: &str,
        default_agent: &str,
    ) -> Result<ChannelRecord, Status> {
        let folder = humata2_svc_review::PR_REVIEWS_FOLDER;
        let base = self.0.layout.channel(folder);
        for sub in ["main", "sessions", "shared"] {
            std::fs::create_dir_all(base.join(sub))
                .map_err(|e| Status::internal(format!("couldn't create PR reviews folder: {e}")))?;
        }
        let record = ChannelRecord {
            id: format!("ch_{}", uuid::Uuid::new_v4().simple()),
            name: "PR Reviews".to_owned(),
            folder: folder.to_owned(),
            branch_prefix: None,
            default_agent: default_agent.to_owned(),
            repos: Vec::new(),
            // A channel's machines start with the host's own, as any new
            // channel's do; a review started on a box adds the box (PR-21).
            machines: vec![machine.to_owned()],
            revision: 1,
            created_unix_ms: humata2_rpc_kit::now_ms(),
        };
        self.0
            .store
            .put_channel(&record)
            .map_err(|e| humatad_core::status::store(e, machine, "create the PR reviews channel"))?;
        Ok(record)
    }

    fn clone_pr_reviews_mains(&self, channel_id: &str) -> Result<(), Status> {
        humata2_sessions::clone_mains_with(
            &self.0.store,
            &self.0.layout,
            &self.0.remote_template,
            channel_id,
            &mut |_: &str, _: Result<(), &humata2_sessions::GitError>| {},
            |_, _, _| {},
        )
        .map_err(|e| Status::internal(e.to_string()))?;
        Ok(())
    }
}

enum OpHandle {
    Live(Option<Progress>),
    Done(pb::Operation),
}

/// Adapts `crate::operations::Started` to [`ReviewOperation`].
struct OpAdapter(Mutex<OpHandle>);

impl ReviewOperation for OpAdapter {
    fn id(&self) -> String {
        let op = self.pb();
        op.id.map(|id| id.value).unwrap_or_default()
    }

    fn pb(&self) -> pb::Operation {
        match &*self.0.lock().unwrap_or_else(|e| e.into_inner()) {
            OpHandle::Live(Some(progress)) => progress.operation(),
            OpHandle::Live(None) => pb::Operation::default(),
            OpHandle::Done(op) => op.clone(),
        }
    }

    fn step_started(&self, _index: usize) {}

    fn step_done(&self, _index: usize) {}

    fn step_failed(&self, _index: usize, _err: pb::Error) {}

    fn finish(&self, _result: Vec<u8>, _result_type: String) {
        if let OpHandle::Live(slot) = &mut *self.0.lock().unwrap_or_else(|e| e.into_inner())
            && let Some(progress) = slot.take()
        {
            progress.succeed(None);
        }
    }

    fn fail(&self, err: pb::Error) {
        if let OpHandle::Live(slot) = &mut *self.0.lock().unwrap_or_else(|e| e.into_inner())
            && let Some(progress) = slot.take()
        {
            progress.fail(err);
        }
    }
}

/// Adapts `crate::operations::Operations` to [`ReviewOperations`].
struct OperationsAdapter(Arc<Operations>);

impl ReviewOperations for OperationsAdapter {
    fn start(
        &self,
        kind: pb::OperationKind,
        _title: String,
        _steps: Vec<String>,
        _actor: Option<pb::Actor>,
    ) -> Box<dyn ReviewOperation> {
        let caller = Caller::User(UserVia::App);
        let handle = match self
            .0
            .start(&caller, kind, pb::EntityRef::default(), &[], false, None)
        {
            Ok(Started::New(progress)) => OpHandle::Live(Some(progress)),
            Ok(Started::Existing(op)) => OpHandle::Done(op),
            Err(_) => OpHandle::Done(pb::Operation::default()),
        };
        Box::new(OpAdapter(Mutex::new(handle)))
    }
}

/// Adapts `crate::terminal::Core` to [`ReviewTerminalCore`].
struct TerminalCoreAdapter(Arc<TerminalCore>);

#[tonic::async_trait]
impl ReviewTerminalCore for TerminalCoreAdapter {
    async fn comments_target(&self, session: &str) -> Result<CommentsTarget, Status> {
        let target = self.0.comments_target(session).await?;
        Ok(CommentsTarget {
            tab: target.tab,
            holder: target.holder,
        })
    }

    async fn type_comments(
        &self,
        target: &CommentsTarget,
        key: &str,
        paste: &[u8],
        shown_wait: Duration,
        return_pause: Duration,
    ) -> CommentsTyped {
        let local_target = humatad_core::terminal::CommentsTarget {
            tab: target.tab.clone(),
            holder: target.holder,
        };
        let mut watch = match self.0.watch_for_comments(&local_target).await {
            Ok(watch) => watch,
            Err(status) => return CommentsTyped::NotTyped(status),
        };
        let typed = self.0.type_comments(&local_target, key, paste).await;
        match typed {
            humatad_core::terminal::CommentsTyped::Yes => {
                // The return that sends the batch goes in whether or not the
                // paste can be watched: a session on another machine has no
                // local screen to watch, and its paste is still only a draft
                // until the return.
                if let Some(watch) = watch.as_mut() {
                    let _ = watch.shown(paste, shown_wait).await;
                }
                if !return_pause.is_zero() {
                    tokio::time::sleep(return_pause).await;
                }
                let ret_key = format!("{key}:return");
                let _ = self.0.type_comments(&local_target, &ret_key, b"\r").await;
                CommentsTyped::Typed
            }
            humatad_core::terminal::CommentsTyped::No(status) => CommentsTyped::NotTyped(status),
            humatad_core::terminal::CommentsTyped::Unknown(status) => CommentsTyped::Unknown(status),
        }
    }

    async fn comments_arrived(&self, target: &CommentsTarget, key: &str) -> CommentsArrived {
        let local_target = humatad_core::terminal::CommentsTarget {
            tab: target.tab.clone(),
            holder: target.holder,
        };
        match self.0.comments_arrived(&local_target, key).await {
            humatad_core::terminal::CommentsArrived::Yes => CommentsArrived::Yes,
            humatad_core::terminal::CommentsArrived::No => CommentsArrived::No,
            humatad_core::terminal::CommentsArrived::Unknown => CommentsArrived::Unknown,
        }
    }
}

/// Constructs [`humata2_svc_review::Review`] wired to `humatad`'s concrete
/// services.
pub(crate) fn build_review(
    store: Arc<Store>,
    guard: Arc<Guard>,
    machine: String,
    feed: Arc<Feed>,
    terminal_core: Arc<TerminalCore>,
) -> Review {
    let sink = Arc::new(TerminalSink::new(Arc::new(TerminalCoreAdapter(
        terminal_core,
    ))));
    Review::new(
        store,
        Arc::new(GuardAdapter(guard)),
        machine,
        Arc::new(FeedAdapter(feed)),
        sink,
    )
}

/// Constructs [`humata2_svc_review::PullRequests`] wired to `humatad`'s
/// concrete services.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_pull_requests(
    store: Arc<Store>,
    guard: Arc<Guard>,
    machine: String,
    starter: Starter,
    tell: Tell,
    feed: Arc<Feed>,
    default_agent: String,
    review: Review,
    operations: Arc<Operations>,
) -> PullRequests {
    PullRequests::new(
        store,
        Arc::new(GuardAdapter(guard)),
        machine,
        Arc::new(StarterAdapter(starter)),
        Arc::new(TellAdapter(tell)),
        Arc::new(FeedAdapter(feed)),
        default_agent,
        review,
        Arc::new(OperationsAdapter(operations)),
    )
}

/// The host's link to its machines, for reviews whose clones and terminal
/// live on one of them (PR-4, PR-21).
struct ReviewRemoteAdapter(humatad_core::remote_work::RemoteWork);

impl ReviewRemoteAdapter {
    fn relayed(&self, machine: &str) -> Result<humatad_core::host_relay::Relayed, Status> {
        self.0.relayed(machine).map_err(Status::from)
    }
}

#[tonic::async_trait]
impl humata2_svc_review::ReviewRemote for ReviewRemoteAdapter {
    async fn run_review_git(
        &self,
        machine: &str,
        request: pb::RunReviewGitRequest,
    ) -> Result<pb::RunReviewGitResponse, Status> {
        match self.relayed(machine)?.carry().run_review_git(request).await {
            Ok(answer) => Ok(answer.into_inner()),
            Err(refused) => Err(self.0.peer_refused(machine, refused).await),
        }
    }

    async fn type_terminal(&self, machine: &str, request: pb::TypeRequest) -> Result<(), Status> {
        match self.relayed(machine)?.terminal().r#type(request).await {
            Ok(_) => Ok(()),
            Err(refused) => Err(self.0.peer_refused(machine, refused).await),
        }
    }

    async fn get_diff(&self, machine: &str, request: pb::GetDiffRequest) -> Result<pb::GetDiffResponse, Status> {
        match self.relayed(machine)?.diff().get_diff(request).await {
            Ok(answer) => Ok(answer.into_inner()),
            Err(refused) => Err(self.0.peer_refused(machine, refused).await),
        }
    }

    async fn get_file_diff(&self, machine: &str, request: pb::GetFileDiffRequest) -> Result<pb::FileDiff, Status> {
        match self.relayed(machine)?.diff().get_file_diff(request).await {
            Ok(answer) => Ok(answer.into_inner()),
            Err(refused) => Err(self.0.peer_refused(machine, refused).await),
        }
    }
}

/// VW-12, D225: a machine's daemon journals each view its agents offer, so
/// the host applies it from the journal; the host's own daemon keeps none.
pub(crate) struct ViewJournalAdapter {
    pub(crate) store: Arc<Store>,
    pub(crate) link: Arc<humata2_svc_away::HostLink>,
}

#[tonic::async_trait]
impl humata2_svc_review::ports::ViewJournal for ViewJournalAdapter {
    async fn offered(&self, session: &str, view: &pb::PreparedView) -> Result<(), Status> {
        if self.link.on_host() {
            return Ok(());
        }
        match humata2_svc_away::record_view_journal(&self.store, &self.link, session, view).await {
            Ok(_) => Ok(()),
            Err(error) => Err(humata2_rpc_kit::failure(
                tonic::Code::Unavailable,
                pb::Error {
                    message: format!(
                        "the view wasn't offered: it couldn't be kept for the host, which is the only way it reaches the app: {}",
                        humata2_rpc_kit::chain(&error)
                    ),
                    fix: "fix what the message names on this machine, then mark the draft ready again; the draft is kept".to_owned(),
                    ..pb::Error::default()
                },
            )),
        }
    }
}

fn map_review_replayed(outcome: humata2_svc_review::Replayed) -> PeerReplayed {
    match outcome {
        humata2_svc_review::Replayed::Held => PeerReplayed::Held,
        humata2_svc_review::Replayed::Applied => PeerReplayed::Applied,
        humata2_svc_review::Replayed::Rejected => PeerReplayed::Rejected,
    }
}

struct PeerReviewAdapter(Arc<Review>);

#[tonic::async_trait]
impl PeerReviewPort for PeerReviewAdapter {
    async fn replay_offered(&self, machine: &str, entry: &pb::JournalEntry) -> PeerReplayed {
        map_review_replayed(self.0.replay_offered(machine, entry).await)
    }
}

struct DraftsAdapter(Review);

impl ReviewDraftsPort for DraftsAdapter {
    fn unsubmitted_pr_drafts(&self, session: &str) -> Result<u32, Status> {
        self.0.unsubmitted_pr_drafts(session)
    }
}

struct LiveReview {
    review: Review,
    pull_requests: PullRequests,
}

impl ReviewInstance for LiveReview {
    fn drafts_port(&self) -> Arc<dyn ReviewDraftsPort> {
        Arc::new(DraftsAdapter(self.review.clone()))
    }

    fn use_sessions(&self, sessions: Arc<SessionSvc>) {
        self.pull_requests
            .use_sessions(Arc::new(SessionsAdapter(sessions)));
    }

    fn spawn_reconcile_submits(&self) {
        let pull_requests = self.pull_requests.clone();
        tokio::spawn(async move { pull_requests.reconcile_submits().await });
    }

    fn peer_review(&self) -> Arc<dyn PeerReviewPort> {
        Arc::new(PeerReviewAdapter(Arc::new(self.review.clone())))
    }

    fn register(self: Box<Self>, routes: &mut RoutesBuilder) {
        routes
            .add_service(pb::diff_service_server::DiffServiceServer::new(
                self.review.clone(),
            ))
            .add_service(
                pb::review_chat_service_server::ReviewChatServiceServer::new(self.review.clone()),
            )
            .add_service(
                pb::pull_request_review_service_server::PullRequestReviewServiceServer::new(
                    self.pull_requests,
                ),
            )
            .add_service(pb::comment_service_server::CommentServiceServer::new(
                self.review.clone(),
            ))
            .add_service(pb::view_service_server::ViewServiceServer::new(self.review));
    }
}

pub struct ReviewPlugin;

impl ReviewPluginTrait for ReviewPlugin {
    fn build(&self, ctx: ReviewContext) -> Box<dyn ReviewInstance> {
        let review = build_review(
            ctx.store.clone(),
            ctx.guard.clone(),
            ctx.machine.clone(),
            ctx.feed.clone(),
            ctx.terminal_core,
        );
        // VW-12: a view offered on a machine reaches the host through the journal.
        review.use_view_journal(Arc::new(ViewJournalAdapter {
            store: ctx.store.clone(),
            link: ctx.guard.host_link(),
        }));
        // BUG 42: the user's own write of a file is not the agent's change.
        ctx.files
            .use_user_writes(Arc::new(UserWritesAdapter(review.clone())));
        // D138: a batch of comments the last run was still typing is settled
        // before anyone asks.
        // A stopped machine types nothing, so it settles them at its Start.
        if !ctx.stopped_now {
            let review = review.clone();
            let operations = ctx.last_start.operations().clone();
            // D241: each batch in doubt is named pending before the report is written.
            let doubtful = review.note_comments(|id, kind, step, state| {
                operations.record(id, kind, step, recovery_state(state));
            });
            tokio::spawn(async move {
                review
                    .recover_comments(
                        |id, kind, step, state| {
                            operations.record(id, kind, step, recovery_state(state));
                        },
                        doubtful,
                    )
                    .await;
            });
        }
        let pull_requests = build_pull_requests(
            ctx.store,
            ctx.guard,
            ctx.machine,
            ctx.starter,
            ctx.session_tell,
            ctx.feed,
            ctx.default_agent,
            review.clone(),
            ctx.operations,
        );
        // D286: a stopped machine asks GitHub nothing on its own; its Start
        // (the daemon starting itself again, running) follows the reviews again.
        // PR-4, PR-21: a review on a box runs git and types there, through the link.
        if let Some(remote) = ctx.remote_work {
            pull_requests.install_remote(Arc::new(ReviewRemoteAdapter(remote)));
        }
        if !ctx.stopped_now {
            pull_requests.spawn_poll();
        }
        Box::new(LiveReview {
            review,
            pull_requests,
        })
    }
}
