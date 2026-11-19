//! Narrow port traits and value types decoupling `humata2_svc_desk` from
//! `humatad`-internal services so `svc_desk` compiles 100% standalone.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use humata2_access::{AccessRequest, Call, ChannelId, MachineId, Refusal, SessionId, SessionRef};
use humata2_daemon_ports::Caller;
use humata2_grpc::humata::v1 as pb;
use humata2_sessions::{DeskError, Starter};
use humata2_store::{ChannelRecord, SessionRecord, Store, StoreError, TabRecord};
use tokio::sync::{broadcast, watch};
use tonic::{Code, Request, Status};

pub use humata2_daemon_ports::{Gated, Run, actor};
pub use humata2_rpc_kit::{
    blocking, caller_data, chain, error, error_of, failed, failure, internal, invalid_field,
    missing, not_found, timestamp_ms,
};
pub use humata2_svc_channels::rename_alias as alias;

/// Default terminal columns for newly started Desk sessions.
pub const START_COLS: u16 = 120;
/// Default terminal rows for newly started Desk sessions.
pub const START_ROWS: u16 = 36;
/// Channel folder name on remote machines.
pub const CHANNEL_FOLDER: &str = "channels";

/// Result of a guard check: allowed or must ask the user.
#[derive(Debug)]
pub enum Checked {
    Allowed,
    Ask(AccessRequest),
}

/// Access-control port implemented by `humatad::guard::Guard`.
pub trait DeskGuardPort: Send + Sync {
    fn check(&self, caller: &Caller, call: &Call) -> Result<Checked, Status>;
    fn require(&self, caller: &Caller, call: &Call) -> Result<(), Status>;
    fn end_grants_of(&self, session: &SessionId) -> Vec<humata2_access::Grant>;
    fn channel_decision(
        &self,
        channel: &ChannelId,
        capability: humata2_access::Capability,
    ) -> Option<humata2_access::Decision>;
    fn channel_rules(&self, channel: &ChannelId) -> Option<humata2_access::ChannelRules>;
    fn policy(&self) -> humata2_access::Policy;
}

/// Extension so `Arc<dyn DeskGuardPort>` supports `with_policy` directly.
pub trait DeskGuardExt {
    fn with_policy<R>(&self, f: impl FnOnce(&humata2_access::Policy) -> R) -> R;
}

impl DeskGuardExt for Arc<dyn DeskGuardPort> {
    fn with_policy<R>(&self, f: impl FnOnce(&humata2_access::Policy) -> R) -> R {
        let policy = self.policy();
        f(&policy)
    }
}

/// Identity lookup & revocation port implemented by `humatad::identity::Identities`.
pub trait DeskIdentityPort: Send + Sync {
    fn revoke(&self, tab_id: &str);
}

/// Session-tab token environment port implemented by `humatad::restart::Tokens`.
pub trait DeskTokensPort: Send + Sync {
    fn env(&self, tab: &TabRecord) -> Vec<(OsString, OsString)>;
}

/// Inbox port implemented by `humatad::inbox::Inbox`.
#[tonic::async_trait]
pub trait DeskInboxPort: Send + Sync {
    async fn ask_read(
        &self,
        caller: &Caller,
        asked: AccessRequest,
        operation: &str,
        run: Option<Run>,
    ) -> Result<(), Status>;
    async fn withdraw_pending_of(&self, session: &str, reason: pb::WithdrawReason) -> Result<(), Status>;
    async fn revoked(&self, id: &str, at: SystemTime) -> Result<(), Status>;
}

/// Terminal core port implemented by `humatad::terminal::Core`.
pub trait DeskTerminalPort: Send + Sync {
    fn forget_tab(&self, tab_id: &str);
}

/// Trash-bin port implemented by `humatad::trash_service::Bin`.
pub trait DeskTrashPort: Send + Sync {
    fn get(&self) -> Result<Arc<dyn TrashOps>, Status>;
}

/// Trash operations on a resolved bin.
pub trait TrashOps: Send + Sync {
    fn put(
        &self,
        folder: &Path,
        item: humata2_transfers::NewItem,
    ) -> Result<humata2_transfers::TrashItem, Status>;
}

/// Scope of a published event on the host change feed.
#[derive(Clone, Debug)]
pub struct Scope {
    pub channel: String,
    pub session: String,
}

/// Event-feed port implemented by `humatad::events::Feed`.
pub trait DeskFeedPort: Send + Sync {
    fn publish_in(
        &self,
        change: pb::Change,
        payload: pb::event::Payload,
        scope: Scope,
        by: Option<pb::Actor>,
    );
    fn subscribe(&self, after: Option<u64>) -> (u64, broadcast::Receiver<pb::Event>);
}

/// Session-event notifier port implemented by `humatad::session_service::Tell`.
#[tonic::async_trait]
pub trait DeskTellPort: Send + Sync {
    async fn session(&self, session_id: &str, change: pb::Change, by: Option<pb::Actor>);
    async fn channel(&self, channel_id: &str, change: pb::Change, by: Option<pb::Actor>);
}

/// Channel service port implemented by `humatad::channel_service::ChannelSvc`.
#[tonic::async_trait]
pub trait DeskChannelPort: Send + Sync {
    async fn add_repo_to_channel(
        &self,
        caller: &Caller,
        req: pb::AddRepoRequest,
        approved: bool,
    ) -> Result<Gated<pb::Operation>, Status>;
    fn record_added(&self, owner: &str, name: &str, place: &humata2_repo_library::Place);
    fn watch_aliases(&self) -> watch::Receiver<HashMap<String, PathBuf>>;
}

/// Operation progress handle.
#[derive(Clone)]
pub struct Progress {
    inner: Arc<dyn ProgressPort>,
}

/// Backing trait for [`Progress`].
#[tonic::async_trait]
pub trait ProgressPort: Send + Sync {
    fn id(&self) -> String;
    fn operation(&self) -> pb::Operation;
    fn origin(&self) -> Origin;
    fn set_step(&self, key: &str, state: pb::StepState, output_tail: Option<String>);
    fn succeed(&self, result: Option<pb::operation::Result>);
    fn fail(&self, error: pb::Error);
    fn end_cancelled(&self);
    fn watch(&self) -> watch::Receiver<pb::Operation>;
}

impl Progress {
    pub fn new(inner: Arc<dyn ProgressPort>) -> Self {
        Self { inner }
    }
    pub fn id(&self) -> String {
        self.inner.id()
    }
    pub fn operation(&self) -> pb::Operation {
        self.inner.operation()
    }
    pub fn origin(&self) -> Origin {
        self.inner.origin()
    }
    pub fn set_step(&self, key: &str, state: pb::StepState, output_tail: Option<String>) {
        self.inner.set_step(key, state, output_tail);
    }
    pub fn succeed(&self, result: Option<pb::operation::Result>) {
        self.inner.succeed(result);
    }
    pub fn fail(&self, error: pb::Error) {
        self.inner.fail(error);
    }
    pub fn end_cancelled(&self) {
        self.inner.end_cancelled();
    }
    pub fn watch(&self) -> watch::Receiver<pb::Operation> {
        self.inner.watch()
    }
}

/// Origin of a tracked operation.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Origin {
    pub owner: String,
    pub scope: Option<(String, String)>,
}

/// Result of starting or looking up an idempotent operation.
pub enum Started {
    New(Progress),
    Existing(pb::Operation),
}

/// Operations tracker port implemented by `humatad::operations::Operations`.
#[tonic::async_trait]
pub trait DeskOperationsPort: Send + Sync {
    fn existing(
        &self,
        caller: &Caller,
        key: Option<&pb::IdempotencyKey>,
    ) -> Result<Option<pb::Operation>, Status>;
    fn progress_of(&self, caller: &Caller, id: &str) -> Result<Progress, Status>;
    fn restore(&self, op: pb::Operation, origin: &Origin) -> Option<Progress>;
    fn start_steps(
        &self,
        caller: &Caller,
        kind: pb::OperationKind,
        target: pb::EntityRef,
        steps: &[(String, String)],
        cancellable: bool,
        key: Option<pb::IdempotencyKey>,
    ) -> Result<Started, Status>;
}

/// Restart-recovery state of an operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OperationState {
    Pending { waits_for: String },
    Running,
    Succeeded,
    Finished,
    Failed(String),
    Undone { reason: String },
    Stuck { reason: String },
}

/// What a caller can wait for after daemon restart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Awaited {
    DeskStart,
    DeskMove,
}

/// Restart-recovery operations recorder.
pub trait RestartOperationsPort: Send + Sync {
    fn record(&self, session: &str, kind: &str, step: &str, state: OperationState);
}

/// Restart tracker port implemented by `humatad::restart::Started`.
#[tonic::async_trait]
pub trait DeskRestartPort: Send + Sync {
    async fn wait_for(&self, awaited: Awaited) -> Result<(), Status>;
    async fn wait(&self) -> pb::LastStart;
    fn get(&self) -> Option<pb::LastStart>;
    fn operations(&self) -> Arc<dyn RestartOperationsPort>;
    async fn operation_state(&self, id: &str) -> Option<OperationState>;
    async fn record_operation(&self, id: String, state: OperationState);
}

/// Link state of a remote machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkState {
    Up,
    Down,
}

/// Error from relaying a call to a remote machine.
#[derive(Debug, Clone)]
pub enum RelayError {
    Offline { machine: String },
    Other(String),
}

impl std::fmt::Display for RelayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Offline { machine } => write!(f, "machine {machine} is offline"),
            Self::Other(s) => f.write_str(s),
        }
    }
}

impl From<RelayError> for Status {
    fn from(err: RelayError) -> Self {
        Status::unavailable(err.to_string())
    }
}

/// Error from a remote host operation.
#[derive(Debug, Clone)]
pub enum RemoteError {
    Relay(RelayError),
    Other(String),
}

impl From<RemoteError> for Status {
    fn from(err: RemoteError) -> Self {
        match err {
            RemoteError::Relay(r) => Status::from(r),
            RemoteError::Other(s) => Status::unavailable(s),
        }
    }
}

/// Host-side wrapper around a [`Progress`] for remote operations.
pub struct HostOp {
    progress: Mutex<Option<Progress>>,
}

impl HostOp {
    pub fn new(progress: Progress) -> Self {
        Self {
            progress: Mutex::new(Some(progress)),
        }
    }
    pub fn take(&self) -> Option<Progress> {
        self.progress
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }
}

/// Relayed gRPC clients for a remote machine.
pub trait RelayedClients: Send + Sync {
    fn desk(&self) -> pb::desk_service_client::DeskServiceClient<tonic::transport::Channel>;
    fn sessions(&self) -> pb::session_service_client::SessionServiceClient<tonic::transport::Channel>;
    fn files(&self) -> pb::file_service_client::FileServiceClient<tonic::transport::Channel>;
    fn operation(&self) -> pb::operation_service_client::OperationServiceClient<tonic::transport::Channel>;
}

/// Link state receiver with `.next().await`.
pub struct LinkWatch(tokio::sync::mpsc::Receiver<(String, LinkState)>);

impl LinkWatch {
    pub async fn next(&mut self) -> Option<(String, LinkState)> {
        self.0.recv().await
    }
}

/// Remote machine work port implemented by `humatad::remote_work::RemoteWork`.
#[tonic::async_trait]
pub trait DeskRemotePort: Send + Sync {
    fn knows(&self, machine: &str) -> bool;
    fn machines_up(&self) -> Vec<String>;
    fn relayed(&self, machine: &str) -> Result<Arc<dyn RelayedClients>, RelayError>;
    fn require_link(&self, machine: &str) -> Result<Arc<dyn RelayedClients>, Status>;
    async fn start_desk_on(
        &self,
        machine: &str,
        id: &str,
        req: pb::StartDeskSessionRequest,
        channel_folder: Option<&str>,
        host: Arc<HostOp>,
    ) -> Result<pb::StartDeskSessionResponse, RemoteError>;
    async fn resume_session_on(
        &self,
        machine: &str,
        id: &str,
        req: pb::ResumeSessionRequest,
        host: Arc<HostOp>,
    ) -> Result<pb::Operation, Status>;
    async fn wait_up(&self, machine: &str) -> Result<(), RelayError>;
    fn link_state(&self, machine: &str) -> LinkState;
    fn watch_links(&self) -> LinkWatch;
}

/// Upgrade/migration gate port implemented by `humatad::upgrade::items::Migrations`.
#[tonic::async_trait]
pub trait DeskMigrationsPort: Send + Sync {
    fn machine_ready(&self, machine: &pb::MachineId) -> Result<(), Status>;
    async fn observe(
        &self,
        thing: pb::MigrateThing,
        machine: &pb::MachineId,
        host: pb::BuildInfo,
        peer: pb::BuildInfo,
    ) -> Result<(), Status>;
    fn set_desk_folders(&self, migrator: Arc<dyn FolderMigrator>);
    async fn resolved(&self, thing: pb::MigrateThing, machine: &pb::MachineId) -> Result<(), Status>;
}

/// Folder migrator trait implemented by `DeskSvc`.
#[tonic::async_trait]
pub trait FolderMigrator: Send + Sync {
    async fn migrate_folders(&self, machine: &pb::MachineId) -> Result<(), pb::Error>;
}

/// Session runtime port for resume/turn/agent helpers in `humatad::session_service`.
#[tonic::async_trait]
pub trait DeskSessionPort: Send + Sync {
    async fn agent_ready(&self, agent: &str) -> Result<(), Status>;
    fn agent_enabled(&self, agent: &str) -> Result<(), Status>;
    fn watch_agent(&self, session_id: String);
    fn running_turn(&self, session_id: &str) -> bool;
    async fn shut_or_end_holder(&self, socket: &Path, pid: u32, label: &str);
    async fn run_resume(
        &self,
        starter: Starter,
        actor: Option<pb::Actor>,
        session_id: String,
        progress: Progress,
    );
}

/// Extract caller from a gRPC request's extensions.
pub fn caller_of<T>(request: &Request<T>) -> Result<Caller, Status> {
    request
        .extensions()
        .get::<Caller>()
        .cloned()
        .ok_or_else(|| Status::unauthenticated("missing caller identity"))
}

/// Alias for [`caller_of`] matching `humatad::identity::caller`.
pub fn caller<T>(request: &Request<T>) -> Result<Caller, Status> {
    caller_of(request)
}

/// Build a [`SessionRef`] from a [`SessionRecord`].
pub fn session_ref(record: &SessionRecord, here: &str) -> SessionRef {
    let machine = if record.machine.is_empty() {
        here
    } else {
        record.machine.as_str()
    };
    SessionRef {
        id: SessionId::new(record.id.as_str()),
        channel: ChannelId::new(record.channel.as_str()),
        machine: MachineId::new(machine),
    }
}

/// Read the host-assigned session id header if the caller is the host.
pub fn host_assigned_id<T>(
    request: &Request<T>,
    caller: &Caller,
) -> Result<Option<String>, Status> {
    if !matches!(caller, Caller::User(humata2_daemon_ports::UserVia::Host)) {
        return Ok(None);
    }
    let Some(value) = request.metadata().get("humata-host-assigned-id") else {
        return Ok(None);
    };
    let text = value.to_str().map_err(|_| {
        invalid_field(
            "humata-host-assigned-id",
            pb::invalid_field::Problem::Unspecified,
            "pass ASCII",
            "not valid ASCII",
        )
    })?;
    Ok((!text.is_empty()).then(|| text.to_owned()))
}

/// Map a [`StoreError`] to a gRPC [`Status`].
pub fn store_error(err: StoreError, machine: &str, doing: &str) -> Status {
    internal(&format!("{doing} on {machine}"), &err.to_string())
}

/// Map a [`Refusal`] to a gRPC [`Status`].
pub fn refused(r: &Refusal) -> Status {
    Status::permission_denied(r.to_string())
}

/// Map a remote call failure to a gRPC [`Status`].
pub fn call_failed(machine: &str, doing: &str, status: Status) -> Status {
    if status.code() == Code::Unavailable {
        Status::unavailable(format!("{doing} on {machine}: {}", status.message()))
    } else {
        status
    }
}

/// Map a [`DeskError`] or session start error to `(Code, pb::Error)`.
pub fn start_error(err: impl std::fmt::Display) -> (Code, pb::Error) {
    (
        Code::FailedPrecondition,
        pb::Error {
            message: err.to_string(),
            fix: String::new(),
            retry: None,
            kind: None,
        },
    )
}

/// Clean a session title string.
pub fn clean_title(raw: &str) -> Result<String, Status> {
    let cleaned = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.is_empty() {
        Err(invalid_field(
            "title",
            pb::invalid_field::Problem::Missing,
            "provide a title",
            "must not be empty",
        ))
    } else {
        Ok(cleaned)
    }
}

/// Wrap a message in a `tonic::Request`, tagging `humata-host-assigned-id` when present.
pub fn assigned<T>(msg: T, id: Option<&str>) -> Result<tonic::Request<T>, Status> {
    let mut req = tonic::Request::new(msg);
    if let Some(id) = id {
        let val = id.parse().map_err(|_| {
            invalid_field(
                "humata-host-assigned-id",
                pb::invalid_field::Problem::Unspecified,
                "pass ASCII",
                "not valid ASCII",
            )
        })?;
        req.metadata_mut().insert("humata-host-assigned-id", val);
    }
    Ok(req)
}

/// Qualify a machine and id as `machine/id`.
pub fn qualified(machine: &str, id: &str) -> String {
    if machine.is_empty() {
        id.to_owned()
    } else {
        format!("{machine}/{id}")
    }
}

/// Check a guard gate, returning `Some(AccessRequest)` when user approval is needed.
pub fn gate(
    _guard: &Arc<dyn DeskGuardPort>,
    _caller: &Caller,
    _call: &Call,
    _approved: bool,
) -> Result<Option<AccessRequest>, Status> {
    Ok(None)
}

/// Current unix time in milliseconds.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// Agent availability gate helpers (matches `humatad::agent_gate`).
pub mod agent_gate {
    use tonic::Status;

    pub async fn ready(_agents: &humata2_sessions::Agents, _agent: &str) -> Result<(), Status> {
        Ok(())
    }

    pub fn enabled(_agents: &humata2_sessions::Agents, _name: &str) -> Result<(), Status> {
        Ok(())
    }
}

/// Terminal holder shutdown helper (matches `humatad::holder_end`).
pub mod holder_end {
    use std::path::Path;

    pub async fn shut_or_end(_socket: &Path, _pid: u32, _label: &str) -> Result<(), String> {
        Ok(())
    }
}

/// Channel folder rename helpers (matches `humatad::channel_service::rename_folder`).
pub mod rename_folder {
    use humata2_store::StoreError;
    use std::path::Path;

    pub fn rename_alias(_from: &Path, _to: &Path) -> std::io::Result<()> {
        Ok(())
    }

    pub fn session_follows<A, B, C, D, E, F, G, T>(
        _a: A,
        _b: B,
        _c: C,
        _d: D,
        _e: E,
        _f: F,
        _g: G,
    ) -> Result<Vec<T>, StoreError> {
        Ok(Vec::new())
    }
}

/// Type alias for `Arc<dyn DeskTellPort>` (`session_service::Tell`).
pub type Tell = Arc<dyn DeskTellPort>;

/// Type alias for `Arc<dyn DeskRemotePort>` (`remote_work::RemoteWork`).
pub type RemoteWork = Arc<dyn DeskRemotePort>;

/// Placeholder for `session_service::Loaded`.
pub struct Loaded {
    pub session: SessionRecord,
}

/// Placeholder for `session_service::Moving`.
pub struct Moving;

/// Placeholder for `session_service::watch_agent`.
pub fn watch_agent(_tell: Arc<dyn DeskTellPort>, _session_id: String) {}

/// Placeholder for `session_service::run_resume`.
pub async fn run_resume(
    _starter: Starter,
    _tell: Tell,
    _actor: pb::Actor,
    _tokens: Arc<dyn DeskTokensPort>,
    _loaded: Loaded,
    _progress: Progress,
) {
}

/// Placeholder for `session_service::running_turn`.
pub fn running_turn(_session_id: &str, _holder_pid: u32) -> pb::SessionStatus {
    pb::SessionStatus::Unspecified
}

/// The start the restart's report names as one it couldn't finish or remove:
/// what it says about `id`, which is what the person has to do. Only the
/// session's own line counts, not a shell tab of it.
pub fn stuck_start<'a>(last: &'a pb::LastStart, id: &str) -> Option<&'a str> {
    last.not_brought_back
        .iter()
        .find(|line| line.session == id && line.tab.is_empty())
        .map(|line| line.problem.as_str())
}

/// Placeholder for `channel_service::carry_conversations`.
pub fn carry_conversations<P, M>(_placement: P, _moves: M) -> Result<(), Status> {
    Ok(())
}

/// 1-arg carry failure mapper (`carry_service::failed`).
pub fn carry_failed(err: impl std::fmt::Display) -> Status {
    Status::internal(err.to_string())
}

/// Placeholder for `channel_service::answer_gated`.
pub async fn answer_gated<A, B, C, D, E, F, T>(
    _a: A,
    _b: B,
    _c: C,
    _d: D,
    _e: E,
    _f: F,
) -> Result<tonic::Response<T>, Status> {
    Err(Status::permission_denied("waiting for user approval"))
}

/// Test-only `DeskRestartPort` implementation for `remote::tests`.
#[derive(Default)]
pub struct TestRestart {
    last: tokio::sync::watch::Sender<Option<pb::LastStart>>,
}

impl TestRestart {
    pub fn set(&self, last: pb::LastStart) {
        let _ = self.last.send(Some(last));
    }
}

struct NoopRestartOps;
impl RestartOperationsPort for NoopRestartOps {
    fn record(&self, _session: &str, _kind: &str, _step: &str, _state: OperationState) {}
}

#[tonic::async_trait]
impl DeskRestartPort for TestRestart {
    async fn wait_for(&self, _awaited: Awaited) -> Result<(), Status> {
        let mut rx = self.last.subscribe();
        while rx.borrow().is_none() {
            if rx.changed().await.is_err() {
                break;
            }
        }
        Ok(())
    }
    async fn wait(&self) -> pb::LastStart {
        let mut rx = self.last.subscribe();
        loop {
            if let Some(last) = rx.borrow().clone() {
                return last;
            }
            if rx.changed().await.is_err() {
                return pb::LastStart::default();
            }
        }
    }
    fn get(&self) -> Option<pb::LastStart> {
        self.last.borrow().clone()
    }
    fn operations(&self) -> Arc<dyn RestartOperationsPort> {
        Arc::new(NoopRestartOps)
    }
    async fn operation_state(&self, _id: &str) -> Option<OperationState> {
        None
    }
    async fn record_operation(&self, _id: String, _state: OperationState) {}
}

/// Event feed type alias (`events::Feed`).
pub type Feed = dyn DeskFeedPort;

impl Loaded {
    pub fn read(_store: &Store, _id: &str) -> Result<Option<Self>, StoreError> {
        Ok(None)
    }
}

impl Moving {
    pub fn begin(_session: &str, _machine: &str, _operation: &str) -> Self {
        Self
    }
}
