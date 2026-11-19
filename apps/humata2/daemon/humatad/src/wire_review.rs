//! Wiring adapter connecting `humata2_svc_review` to `humatad`'s concrete
//! `Guard`, `Feed`, `Operations`, `SessionSvc`, `Tell`, `Starter`, and
//! `terminal::Core` (Wiring Wave 1).

#![cfg(feature = "svc_review")]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use humata2_access::Call;
use humata2_daemon_ports::{Caller, UserVia};
use humata2_grpc::humata::v1 as pb;
use humata2_sessions::Starter;
use humata2_store::{ChannelRecord, Store};
use humata2_svc_review::ports::{
    Checked, CommentsArrived, CommentsTarget, CommentsTyped, ReviewFeed, ReviewGuard,
    ReviewOperation, ReviewOperations, ReviewSessions, ReviewStarter, ReviewTell,
    ReviewTerminalCore, Scope,
};
use humata2_svc_review::{PullRequests, RecoveryState, Review, TerminalSink, UserWriteGuard};
use tonic::Status;

use crate::events::{Feed, Scope as EventScope};
use crate::files::{UserWrite, UserWrites};
use crate::guard::{Checked as GuardChecked, Guard};
use crate::identity::{
    AgentCaller as LocalAgentCaller, Caller as LocalCaller, UserVia as LocalUserVia,
};
use crate::operations::{Operations, Progress, Started};
use crate::restart::OperationState;
use crate::script_tokens::ScriptCaller as LocalScriptCaller;
use crate::session_service::{SessionSvc, Tell};
use crate::terminal::Core as TerminalCore;

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
                String::new(),
                String::new(),
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
            machines: Vec::new(),
            revision: 1,
            created_unix_ms: humata2_rpc_kit::now_ms(),
        };
        self.0
            .store
            .put_channel(&record)
            .map_err(|e| crate::status::store(e, machine, "create the PR reviews channel"))?;
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
        let caller = LocalCaller::User(LocalUserVia::App);
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
        let local_target = crate::terminal::CommentsTarget {
            tab: target.tab.clone(),
            holder: target.holder,
        };
        let mut watch = match self.0.watch_for_comments(&local_target).await {
            Ok(watch) => watch,
            Err(status) => return CommentsTyped::NotTyped(status),
        };
        let typed = self.0.type_comments(&local_target, key, paste).await;
        match typed {
            crate::terminal::CommentsTyped::Yes => {
                if let Some(watch) = watch.as_mut() {
                    let _ = watch.shown(paste, shown_wait).await;
                    if !return_pause.is_zero() {
                        tokio::time::sleep(return_pause).await;
                    }
                    let ret_key = format!("{key}:return");
                    let _ = self.0.type_comments(&local_target, &ret_key, b"\r").await;
                }
                CommentsTyped::Typed
            }
            crate::terminal::CommentsTyped::No(status) => CommentsTyped::NotTyped(status),
            crate::terminal::CommentsTyped::Unknown(status) => CommentsTyped::Unknown(status),
        }
    }

    async fn comments_arrived(&self, target: &CommentsTarget, key: &str) -> CommentsArrived {
        let local_target = crate::terminal::CommentsTarget {
            tab: target.tab.clone(),
            holder: target.holder,
        };
        match self.0.comments_arrived(&local_target, key).await {
            crate::terminal::CommentsArrived::Yes => CommentsArrived::Yes,
            crate::terminal::CommentsArrived::No => CommentsArrived::No,
            crate::terminal::CommentsArrived::Unknown => CommentsArrived::Unknown,
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
