//! Wiring adapter connecting `humata2_svc_board::BoardSvc` to `humatad_core`'s
//! concrete services (`Guard`, `Feed`, `Inbox`, `Forwarder`, `Tell`, `RepoSvc`,
//! `MachineSvc`, `Files`, `TransferSvc`).

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use humata2_access::{AccessRequest, Call};
use humata2_daemon_ports::{BoardAsk, Caller, Run, UserVia};
use humata2_grpc::humata::v1 as pb;
use humata2_grpc::humata::v1::browser_bridge_service_server::BrowserBridgeService;
use humata2_grpc::humata::v1::file_service_server::FileService;
use humata2_grpc::humata::v1::machine_service_server::MachineService;
use humata2_grpc::humata::v1::provider_service_server::ProviderService;
use humata2_grpc::humata::v1::repo_service_server::RepoService;
use humata2_grpc::humata::v1::session_service_server::SessionService;
use humata2_grpc::humata::v1::transfer_service_server::TransferService;
use humata2_store::Store;
use humata2_svc_board::ports::{
    AwayCall, BoardFeed, BoardForwarder, BoardGuard, BoardInbox, Checked, HostBrowser, HostCalls,
    HostFiles, HostMachines, HostProviders, HostRepos, HostTransfers, Method, OwnedItem, Row,
    Scope, SessionTell, SyncMode, TodoAutomation, TodoAutomationSource,
};
use humata2_svc_peer::ports::{PeerBoardPort, Replayed as PeerReplayed};
use humatad_core::away::{self, Forwarder, HostLink};
use humatad_core::browser_service::BrowserBridgeSvc;
use humatad_core::events::Feed;
use humatad_core::files::Files;
use humatad_core::guard::{self, Guard};
use humatad_core::inbox::Inbox;
use humatad_core::machine_service::MachineSvc;
use humatad_core::plugins::{BoardContext, BoardInstance, BoardPlugin as BoardPluginTrait};
use humatad_core::provider_service::ProviderSvc;
use humatad_core::repo_service::RepoSvc;
use humatad_core::session_service::{SessionSvc, Tell};
use humatad_core::status;
use humatad_core::transfer_service::TransferSvc;
use humata2_rpc_kit::{now_ms, say};
use humata2_svc_board::{ArchiveOutcome, GhPrStatus, SeenStatus, SessionControl, SessionSeen};
use tokio::sync::{Mutex as AsyncMutex, Notify, broadcast};
use tonic::service::RoutesBuilder;
use tonic::{Request, Response, Status};

// ---------------------------------------------------------------------------
// Caller conversion (`humata2_daemon_ports::Caller` -> `crate::identity::Caller`)
// ---------------------------------------------------------------------------

fn to_local_caller(caller: &Caller) -> Caller {
    caller.clone()
}

fn adapt_request<T>(request: Request<T>) -> Request<T> {
    request
}

// ---------------------------------------------------------------------------
// Guard adapter
// ---------------------------------------------------------------------------

struct GuardAdapter(Arc<Guard>);

impl BoardGuard for GuardAdapter {
    fn check(&self, caller: &Caller, call: &Call) -> Result<Checked, Status> {
        let local = to_local_caller(caller);
        match self.0.check(&local, call)? {
            guard::Checked::Allowed => Ok(Checked::Allowed),
            guard::Checked::Ask(asked) => Ok(Checked::Ask(Box::new(asked))),
        }
    }

    fn check_away(&self, caller: &Caller, call: &Call) -> Result<AwayCall, Status> {
        let local = to_local_caller(caller);
        match self.0.check_away(&local, call)? {
            guard::AwayCall::Journal => Ok(AwayCall::Journal),
            guard::AwayCall::Asks(asked) => Ok(AwayCall::Asks(asked)),
            guard::AwayCall::LastCopy => Ok(AwayCall::LastCopy),
            guard::AwayCall::Reachable => Ok(AwayCall::Reachable),
        }
    }

    fn lists_low_risk(&self, asked: &AccessRequest) -> bool {
        self.0.lists_low_risk(asked)
    }

    fn policy_changed(&self) {
        self.0.policy_changed();
    }

    fn host_link(&self) -> String {
        self.0.machine().to_owned()
    }
}

// ---------------------------------------------------------------------------
// Event feed adapter
// ---------------------------------------------------------------------------

struct FeedAdapter(Arc<Feed>);

impl BoardFeed for FeedAdapter {
    fn publish_in(
        &self,
        change: pb::Change,
        payload: pb::event::Payload,
        scope: Scope,
        by: Option<pb::Actor>,
    ) {
        self.0.publish_in(
            change,
            payload,
            humatad_core::events::Scope {
                channel: scope.channel,
                session: scope.session,
            },
            by,
        );
    }
}

// ---------------------------------------------------------------------------
// Inbox adapter
// ---------------------------------------------------------------------------

struct InboxAdapter(Arc<Inbox>);

#[tonic::async_trait]
impl BoardInbox for InboxAdapter {
    async fn withdraw_about_item(&self, key: &str) -> Result<(), Status> {
        self.0.withdraw_about_item(key).await
    }

    async fn defer_board(
        &self,
        caller: &Caller,
        asked: AccessRequest,
        idempotency: Option<String>,
        board: BoardAsk,
        run: Run,
    ) -> Status {
        let local = to_local_caller(caller);
        self.0
            .defer_board(&local, asked, idempotency, board, run)
            .await
    }

    async fn pending_board_asks(&self) -> Vec<pb::InboxRequest> {
        self.0.pending_board_asks().await
    }

    async fn wait_final(
        &self,
        caller: Caller,
        request: pb::WaitForAnswerRequest,
    ) -> Result<pb::WaitResponse, Status> {
        let local = to_local_caller(&caller);
        self.0.wait_final(local, request).await
    }

    async fn withdraw_as(
        &self,
        caller: Caller,
        request: pb::WithdrawRequestRequest,
    ) -> Result<pb::InboxRequest, Status> {
        let local = to_local_caller(&caller);
        self.0.withdraw_as(local, request).await
    }

    async fn ask_why_as(
        &self,
        caller: Caller,
        request: pb::AskWhyRequest,
    ) -> Result<pb::InboxRequest, Status> {
        let local = to_local_caller(&caller);
        self.0.ask_why_as(local, request).await
    }

    async fn explain_as(
        &self,
        caller: Caller,
        request: pb::ExplainAskRequest,
    ) -> Result<pb::InboxRequest, Status> {
        let local = to_local_caller(&caller);
        self.0.explain_as(local, request).await
    }

    async fn approved_on(
        &self,
        machine: &str,
        id: &str,
        session: &str,
    ) -> Option<pb::InboxRequest> {
        self.0.approved_on(machine, id, session).await
    }
}

// ---------------------------------------------------------------------------
// Away / forwarder adapter
// ---------------------------------------------------------------------------

fn away_method(method: Method) -> away::Method {
    match method {
        Method::ListItems => away::Method::ListItems,
        Method::GetItem => away::Method::GetItem,
        Method::GetItemHistory => away::Method::GetItemHistory,
        Method::CreateItem => away::Method::CreateItem,
        Method::UpdateItem => away::Method::UpdateItem,
        Method::MoveItem => away::Method::MoveItem,
        Method::DeleteItem => away::Method::DeleteItem,
        Method::WaitForAnswer => away::Method::WaitForAnswer,
        Method::WithdrawRequest => away::Method::WithdrawRequest,
        Method::AskWhy => away::Method::AskWhy,
        Method::ExplainAsk => away::Method::ExplainAsk,
        Method::SearchRepos => away::Method::SearchRepos,
        Method::ListRepoLibrary => away::Method::ListRepoLibrary,
        Method::ListMachines => away::Method::ListMachines,
        Method::GetMachine => away::Method::GetMachine,
        Method::ListDirectory => away::Method::ListDirectory,
        Method::Stat => away::Method::Stat,
        Method::StartTransfer => away::Method::StartTransfer,
        Method::GetBrowserBridgeStatus => away::Method::GetBrowserBridgeStatus,
        Method::ExecuteBrowserAction => away::Method::ExecuteBrowserAction,
        Method::ListProviders => away::Method::ListProviders,
        Method::GetProvidersConfig => away::Method::GetProvidersConfig,
        Method::RequestProviderImport => away::Method::RequestProviderImport,
    }
}

struct ForwarderAdapter {
    forwarder: Arc<Forwarder>,
    link: Arc<HostLink>,
    store: Arc<Store>,
}

#[tonic::async_trait]
impl BoardForwarder for ForwarderAdapter {
    fn on_machine(&self) -> bool {
        self.forwarder.on_machine()
    }

    fn forwards(&self) -> bool {
        self.forwarder.forwards()
    }

    async fn call(
        &self,
        session: &str,
        row: &'static Row,
        request: Vec<u8>,
        key: Option<pb::IdempotencyKey>,
    ) -> Result<Vec<u8>, Status> {
        self.forwarder
            .call(session, away_method(row.method).row(), request, key)
            .await
    }

    async fn record_board_journal(
        &self,
        _store: &Store,
        _host_link: &str,
        session: &str,
        operation: &'static str,
        bytes: Vec<u8>,
        base: Option<u64>,
        approved: Option<String>,
    ) -> Result<u64, Status> {
        away::record_board_journal(&self.store, &self.link, session, operation, bytes, base, approved)
            .await
            .map_err(|err| status::internal(status::chain(&err), ""))
    }

    fn owned_item(
        &self,
        store: &Store,
        owner: &str,
        item: &str,
    ) -> Result<Option<OwnedItem>, Status> {
        let found = away::owned_item(store, owner, item)
            .map_err(|err| status::internal(status::chain(&err), ""))?;
        Ok(found.map(|owned| OwnedItem { board: owned.board }))
    }
}

// ---------------------------------------------------------------------------
// Sibling host service adapters
// ---------------------------------------------------------------------------

struct TellAdapter(Tell);

#[tonic::async_trait]
impl SessionTell for TellAdapter {
    async fn session(&self, id: &str, change: pb::Change, by: Option<pb::Actor>) {
        self.0.session(id, change, by).await;
    }
}

struct ReposAdapter(RepoSvc);

#[tonic::async_trait]
impl HostRepos for ReposAdapter {
    async fn search_repos(
        &self,
        request: Request<pb::SearchReposRequest>,
    ) -> Result<Response<pb::SearchReposResponse>, Status> {
        RepoService::search_repos(&self.0, adapt_request(request)).await
    }

    async fn list_repo_library(
        &self,
        request: Request<pb::ListRepoLibraryRequest>,
    ) -> Result<Response<pb::ListRepoLibraryResponse>, Status> {
        RepoService::list_repo_library(&self.0, adapt_request(request)).await
    }
}

struct MachinesAdapter(MachineSvc);

#[tonic::async_trait]
impl HostMachines for MachinesAdapter {
    async fn list_for_machine_agent(
        &self,
        request: Request<pb::ListMachinesRequest>,
    ) -> Result<Response<pb::ListMachinesResponse>, Status> {
        self.0.list_for_machine_agent(adapt_request(request)).await
    }

    async fn get_machine(
        &self,
        request: Request<pb::GetMachineRequest>,
    ) -> Result<Response<pb::MachineDetail>, Status> {
        MachineService::get_machine(&self.0, adapt_request(request)).await
    }
}

struct FilesAdapter(Arc<Files>);

#[tonic::async_trait]
impl HostFiles for FilesAdapter {
    async fn list_directory(
        &self,
        request: Request<pb::ListDirectoryRequest>,
    ) -> Result<Response<pb::ListDirectoryResponse>, Status> {
        FileService::list_directory(self.0.as_ref(), adapt_request(request)).await
    }

    async fn stat(
        &self,
        request: Request<pb::StatRequest>,
    ) -> Result<Response<pb::FileEntry>, Status> {
        FileService::stat(self.0.as_ref(), adapt_request(request)).await
    }
}

struct TransfersAdapter(Arc<TransferSvc>);

#[tonic::async_trait]
impl HostTransfers for TransfersAdapter {
    async fn start_transfer(
        &self,
        request: Request<pb::StartTransferRequest>,
    ) -> Result<Response<pb::Transfer>, Status> {
        TransferService::start_transfer(self.0.as_ref(), adapt_request(request)).await
    }
}

struct BrowserAdapter(BrowserBridgeSvc);

#[tonic::async_trait]
impl HostBrowser for BrowserAdapter {
    async fn get_browser_bridge_status(
        &self,
        request: Request<pb::GetBrowserBridgeStatusRequest>,
    ) -> Result<Response<pb::GetBrowserBridgeStatusResponse>, Status> {
        BrowserBridgeService::get_browser_bridge_status(&self.0, adapt_request(request)).await
    }

    async fn execute_browser_action(
        &self,
        request: Request<pb::ExecuteBrowserActionRequest>,
    ) -> Result<Response<pb::ExecuteBrowserActionResponse>, Status> {
        BrowserBridgeService::execute_browser_action(&self.0, adapt_request(request)).await
    }
}

struct ProvidersAdapter(ProviderSvc);

#[tonic::async_trait]
impl HostProviders for ProvidersAdapter {
    async fn list_providers(
        &self,
        request: Request<pb::ListProvidersRequest>,
    ) -> Result<Response<pb::ListProvidersResponse>, Status> {
        ProviderService::list_providers(&self.0, adapt_request(request)).await
    }

    async fn get_providers_config(
        &self,
        request: Request<pb::GetProvidersConfigRequest>,
    ) -> Result<Response<pb::ProvidersConfig>, Status> {
        ProviderService::get_providers_config(&self.0, adapt_request(request)).await
    }

    async fn request_provider_import(
        &self,
        request: Request<pb::RequestProviderImportRequest>,
    ) -> Result<Response<pb::SetProvidersConfigResponse>, Status> {
        ProviderService::request_provider_import(&self.0, adapt_request(request)).await
    }
}

// ---------------------------------------------------------------------------
// Session control and the live session feed for To-do automation (TODO2WIRE)
// ---------------------------------------------------------------------------

/// How often the To-do list looks over its in-progress items for quiet ones.
const SESSION_TICK: Duration = Duration::from_secs(5 * 60);
/// How often watched pull requests are read.
const PR_WATCH_EVERY: Duration = Duration::from_secs(5 * 60);

/// The To-do list's own work on sessions, done as the user through the
/// session service's handlers, in this process.
/// The effective To-do automation settings: the channel's own, else
/// Settings, else the defaults. Each call reads them afresh, so a change in
/// Settings applies at once.
struct AutomationAdapter(Arc<Store>);

fn sync_mode(mode: i32) -> SyncMode {
    match pb::TodoSyncMode::try_from(mode) {
        Ok(pb::TodoSyncMode::Auto) => SyncMode::Auto,
        _ => SyncMode::Ask,
    }
}

impl TodoAutomationSource for AutomationAdapter {
    fn settings(&self, channel: Option<&str>) -> TodoAutomation {
        let set = humatad_core::todo_automation_for(&self.0, channel);
        let defaults = TodoAutomation::default();
        TodoAutomation {
            on_pr_merged: sync_mode(set.on_pr_merged),
            on_todo_closed_archive_sessions: sync_mode(set.on_todo_closed_archive_sessions),
            auto_wake_on_ci_fail: set.auto_wake_on_ci_fail,
            auto_wake_on_review_comments: set.auto_wake_on_review_comments,
            idle_nudge_days: if set.idle_nudge_days == 0 { defaults.idle_nudge_days } else { set.idle_nudge_days },
        }
    }
}

struct SessionsAdapter(Arc<OnceLock<SessionSvc>>);

fn as_user<T>(message: T) -> Request<T> {
    let mut request = Request::new(message);
    request.extensions_mut().insert(Caller::User(UserVia::App));
    request
}

fn session_id(id: &str) -> pb::SessionId {
    pb::SessionId { value: id.to_owned() }
}

impl SessionsAdapter {
    fn svc(&self) -> Result<SessionSvc, Status> {
        self.0.get().cloned().ok_or_else(|| {
            Status::unavailable(
                "The session service isn't up yet, so the To-do list couldn't act on the session. Try again in a moment.",
            )
        })
    }
}

/// Why a session with unsaved work stays, for the person to act on.
fn unsaved_reason(plan: &pb::ArchivePlan) -> Option<String> {
    if plan.unpushed.is_empty() {
        return None;
    }
    let commits: u32 = plan.unpushed.iter().map(|repo| repo.unpushed_commits).sum();
    let files: u32 = plan.unpushed.iter().map(|repo| repo.dirty_files).sum();
    let mut parts = Vec::new();
    if commits > 0 {
        parts.push(format!("{commits} unpushed commit{}", if commits == 1 { "" } else { "s" }));
    }
    if files > 0 {
        parts.push(format!("{files} uncommitted file{}", if files == 1 { "" } else { "s" }));
    }
    if parts.is_empty() {
        parts.push("work that was never pushed".to_owned());
    }
    Some(format!("has {}; push or commit them first", parts.join(" and ")))
}

#[tonic::async_trait]
impl SessionControl for SessionsAdapter {
    async fn send_note(&self, session: &str, text: &str) -> Result<(), Status> {
        let svc = self.svc()?;
        SessionService::send_note(
            &svc,
            as_user(pb::SendNoteRequest {
                idempotency_key: None,
                to: Some(session_id(session)),
                text: text.to_owned(),
                reason: String::new(),
            }),
        )
        .await
        .map(|_| ())
    }

    async fn archive_if_clean(&self, session: &str) -> Result<ArchiveOutcome, Status> {
        let svc = self.svc()?;
        let id = session_id(session);
        // A running terminal is never ended for the person: only a session
        // whose agent is over is archived on its own.
        let current = SessionService::get_session(
            &svc,
            as_user(pb::GetSessionRequest { id: Some(id.clone()) }),
        )
        .await?
        .into_inner();
        match current.status() {
            pb::SessionStatus::Archived => return Ok(ArchiveOutcome::Archived),
            pb::SessionStatus::Exited
            | pb::SessionStatus::Stopped
            | pb::SessionStatus::MachineStopped => {}
            _ => {
                return Ok(ArchiveOutcome::Kept {
                    reason: "still running; end it first".to_owned(),
                });
            }
        }
        let plan = SessionService::plan_archive(
            &svc,
            as_user(pb::PlanArchiveRequest { id: Some(id.clone()) }),
        )
        .await?
        .into_inner();
        if let Some(reason) = unsaved_reason(&plan) {
            return Ok(ArchiveOutcome::Kept { reason });
        }
        SessionService::archive_session(
            &svc,
            as_user(pb::ArchiveSessionRequest {
                idempotency_key: None,
                id: Some(id),
                choice: pb::ArchiveChoice::Archive as i32,
                plan: plan.token,
                end: None,
            }),
        )
        .await?;
        Ok(ArchiveOutcome::Archived)
    }

    async fn restore(&self, session: &str) -> Result<(), Status> {
        let svc = self.svc()?;
        SessionService::restore_session(
            &svc,
            as_user(pb::RestoreSessionRequest {
                idempotency_key: None,
                id: Some(session_id(session)),
                machine: None,
                stopped: false,
            }),
        )
        .await
        .map(|_| ())
    }
}

fn seen_status(status: pb::SessionStatus) -> SeenStatus {
    use pb::SessionStatus as S;
    match status {
        S::Unspecified | S::Moving => SeenStatus::Unknown,
        S::Starting => SeenStatus::Starting,
        // A shell, or an agent with no turn source, is running with no turns:
        // it counts as working so its end is a finished turn.
        S::Working | S::RunningUnknownTurn => SeenStatus::Working,
        S::NeedsInput => SeenStatus::NeedsInput,
        S::Idle => SeenStatus::Idle,
        S::ProbablyIdle => SeenStatus::ProbablyIdle,
        S::Exited => SeenStatus::Exited,
        S::Offline => SeenStatus::Offline,
        S::ResumeFailed => SeenStatus::ResumeFailed,
        S::Archived => SeenStatus::Archived,
        S::MachineStopped | S::Stopped => SeenStatus::Stopped,
    }
}

fn seen_of(session: &pb::Session) -> Option<SessionSeen> {
    let id = session.id.as_ref()?.value.clone();
    Some(SessionSeen {
        id,
        title: session.title.clone(),
        agent: session.agent.as_ref().map(|agent| agent.value.clone()).unwrap_or_default(),
        model: session.model.clone(),
        status: seen_status(session.status()),
        exit_code: session.exit_code,
        board_item: session
            .board_item
            .as_ref()
            .map(|item| item.value.clone())
            .filter(|item| !item.is_empty()),
        last_output_ms: session
            .last_output_at
            .as_ref()
            .map(|at| at.seconds.saturating_mul(1000).saturating_add(i64::from(at.nanos) / 1_000_000)),
    })
}

/// Tells the board what the sessions were at the start, and again after the
/// feed dropped events, so the next event compares with the real status.
struct Seeder {
    board: humata2_svc_board::BoardSvc,
    sessions: Arc<OnceLock<SessionSvc>>,
    done: AsyncMutex<bool>,
}

impl Seeder {
    /// Seeds once the session service exists; a no-op after that.
    async fn ensure(&self) {
        let mut done = self.done.lock().await;
        if *done {
            return;
        }
        let Some(svc) = self.sessions.get().cloned() else {
            return;
        };
        let listed = SessionService::list_sessions(
            &svc,
            as_user(pb::ListSessionsRequest {
                include_archived: true,
                ..Default::default()
            }),
        )
        .await;
        match listed {
            Ok(response) => {
                let seen: Vec<SessionSeen> =
                    response.into_inner().sessions.iter().filter_map(seen_of).collect();
                let board = self.board.clone();
                if tokio::task::spawn_blocking(move || board.seed_sessions(&seen)).await.is_ok() {
                    *done = true;
                }
            }
            Err(status) => say!(
                "humatad2: the To-do list couldn't read the sessions at start, so it will try again at the next change: {}",
                status.message()
            ),
        }
    }

    /// Reads the sessions again, after events were missed.
    async fn reseed(&self) {
        *self.done.lock().await = false;
        self.ensure().await;
    }
}

/// Feeds every session change on the event feed to the To-do list.
async fn pump_sessions(
    board: humata2_svc_board::BoardSvc,
    mut events: broadcast::Receiver<Arc<pb::Event>>,
    seeder: Arc<Seeder>,
) {
    loop {
        match events.recv().await {
            Ok(event) => {
                seeder.ensure().await;
                let Some(pb::event::Payload::Session(session)) = &event.payload else {
                    continue;
                };
                let Some(seen) = seen_of(session) else {
                    continue;
                };
                let board = board.clone();
                match tokio::task::spawn_blocking(move || board.observe_session(&seen, now_ms())).await {
                    Ok(Ok(())) => {}
                    Ok(Err(status)) => say!(
                        "humatad2: the To-do list couldn't note a session change: {}",
                        status.message()
                    ),
                    Err(error) => say!("humatad2: the To-do list's session task stopped: {error}"),
                }
            }
            Err(broadcast::error::RecvError::Lagged(_)) => seeder.reseed().await,
            Err(broadcast::error::RecvError::Closed) => return,
        }
    }
}

/// Every few minutes, looks over the in-progress items for quiet sessions.
async fn tick_sessions(board: humata2_svc_board::BoardSvc, seeder: Arc<Seeder>) {
    let mut every = tokio::time::interval_at(tokio::time::Instant::now() + SESSION_TICK, SESSION_TICK);
    every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        every.tick().await;
        seeder.ensure().await;
        let board = board.clone();
        match tokio::task::spawn_blocking(move || board.session_tick(now_ms())).await {
            Ok(Ok(())) => {}
            Ok(Err(status)) => say!(
                "humatad2: the To-do list couldn't look over its sessions: {}",
                status.message()
            ),
            Err(error) => say!("humatad2: the To-do list's tick task stopped: {error}"),
        }
    }
}

/// Starts the host's To-do automation tasks on the current runtime.
fn start_automation(
    board: &humata2_svc_board::BoardSvc,
    feed: &Feed,
    sessions: Arc<OnceLock<SessionSvc>>,
    watches_prs: bool,
) {
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        say!("humatad2: the To-do list's session automation didn't start: the daemon built its board outside its runtime");
        return;
    };
    // Subscribed now, so no change between the build and the task's first run is missed.
    let events = feed.subscribe_events();
    let seeder = Arc::new(Seeder {
        board: board.clone(),
        sessions,
        done: AsyncMutex::new(false),
    });
    runtime.spawn(pump_sessions(board.clone(), events, seeder.clone()));
    runtime.spawn(tick_sessions(board.clone(), seeder));
    if watches_prs {
        let _guard = runtime.enter();
        drop(board.clone().spawn_pr_watcher(PR_WATCH_EVERY, Arc::new(Notify::new())));
    }
}

// ---------------------------------------------------------------------------
// Public constructors for `serve.rs`
// ---------------------------------------------------------------------------

/// Builds a [`humata2_svc_board::BoardSvc`] wired to `humatad`'s concrete services.
pub(crate) fn build(
    guard: Arc<Guard>,
    store: Arc<Store>,
    feed: Arc<Feed>,
    inbox: Arc<Inbox>,
    machine: String,
    tell: Option<Tell>,
    repos: Option<RepoSvc>,
    forwarder: Option<Arc<Forwarder>>,
    sessions: Arc<OnceLock<SessionSvc>>,
) -> humata2_svc_board::BoardSvc {
    let link = guard.host_link();
    let live_feed = feed.clone();
    let on_host = forwarder.as_ref().is_none_or(|forwarder| !forwarder.on_machine());
    let mut svc = humata2_svc_board::BoardSvc::new(
        Arc::new(GuardAdapter(guard)),
        store.clone(),
        Arc::new(FeedAdapter(feed)),
        Arc::new(InboxAdapter(inbox)),
        machine,
    );
    if let Some(tell) = tell {
        svc = svc.telling(Arc::new(TellAdapter(tell)));
    }
    if let Some(repos) = repos {
        svc = svc.with_repos(Arc::new(ReposAdapter(repos)));
    }
    if let Some(forwarder) = forwarder {
        svc = svc.forwarding(Arc::new(ForwarderAdapter { forwarder, link, store: store.clone() }));
    }
    if on_host {
        if let Some(gh) = humata2_svc_board::GhReviewRequests::for_host() {
            // The host reads the pull requests that ask for the user's review.
            svc = svc.with_review_requests(Arc::new(gh));
        }
        // The host acts on its sessions and reads watched pull requests, as
        // Settings (or a channel's own) says.
        svc = svc.with_automation(Arc::new(AutomationAdapter(store.clone())));
        svc = svc.with_session_control(Arc::new(SessionsAdapter(sessions.clone())));
        let watches_prs = match GhPrStatus::for_host() {
            Some(gh) => {
                svc = svc.with_pr_status(Arc::new(gh));
                true
            }
            None => false,
        };
        start_automation(&svc, &live_feed, sessions, watches_prs);
    }
    svc
}

/// Wraps `humatad`'s host machine, provider, file, transfer and browser bridge services as [`HostCalls`].
fn host_calls(
    machines: MachineSvc,
    providers: ProviderSvc,
    files: Arc<Files>,
    transfers: Arc<TransferSvc>,
    browser: BrowserBridgeSvc,
) -> HostCalls {
    HostCalls {
        machines: Arc::new(MachinesAdapter(machines)),
        providers: Arc::new(ProvidersAdapter(providers)),
        files: Arc::new(FilesAdapter(files)),
        transfers: Arc::new(TransfersAdapter(transfers)),
        browser: Arc::new(BrowserAdapter(browser)),
    }
}

fn map_board_replayed(outcome: humata2_svc_board::Replayed) -> PeerReplayed {
    match outcome {
        humata2_svc_board::Replayed::Held => PeerReplayed::Held,
        humata2_svc_board::Replayed::Applied => PeerReplayed::Applied,
        humata2_svc_board::Replayed::Rejected => PeerReplayed::Rejected,
    }
}

struct PeerBoardAdapter(Arc<humata2_svc_board::BoardSvc>);

#[tonic::async_trait]
impl PeerBoardPort for PeerBoardAdapter {
    async fn replay_at(
        &self,
        machine: &str,
        entry: &pb::JournalEntry,
        link_up: i64,
    ) -> PeerReplayed {
        map_board_replayed(self.0.replay_at(machine, entry, link_up).await)
    }

    async fn forwarded(&self, machine: &str, call: pb::ForwardedCall) -> pb::ForwardedResult {
        self.0.forwarded(machine, call).await
    }
}

struct LiveBoard(humata2_svc_board::BoardSvc);

impl BoardInstance for LiveBoard {
    fn peer_board(&self) -> Arc<dyn PeerBoardPort> {
        Arc::new(PeerBoardAdapter(Arc::new(self.0.clone())))
    }

    fn answer_host_calls(
        &self,
        machines: MachineSvc,
        providers: ProviderSvc,
        files: Arc<Files>,
        transfers: Arc<TransferSvc>,
        browser: BrowserBridgeSvc,
    ) {
        self.0
            .answer_host_calls(host_calls(machines, providers, files, transfers, browser));
    }

    fn register(self: Box<Self>, routes: &mut RoutesBuilder) {
        // The To-do list is the board store seen as one ranked list (BOARDSIMPLE).
        routes.add_service(pb::todo_service_server::TodoServiceServer::new(self.0.clone()));
        routes.add_service(pb::board_service_server::BoardServiceServer::new(self.0));
    }
}

pub struct BoardPlugin;

impl BoardPluginTrait for BoardPlugin {
    fn build(&self, ctx: BoardContext) -> Box<dyn BoardInstance> {
        Box::new(LiveBoard(build(
            ctx.guard,
            ctx.store,
            ctx.feed,
            ctx.inbox,
            ctx.machine,
            ctx.tell,
            ctx.repos,
            ctx.forwarder,
            ctx.sessions,
        )))
    }
}
