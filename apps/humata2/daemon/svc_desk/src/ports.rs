//! Narrow port traits and value types decoupling `humata2_svc_desk` from
//! `humatad`-internal services so `svc_desk` compiles 100% standalone.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use std::any::Any;
use std::future::Future;

use humata2_access::{AccessRequest, Call, ChannelId, MachineId, Refusal, SessionId, SessionRef};
use humata2_daemon_ports::{Caller, Deferred};
use humata2_grpc::humata::v1 as pb;
use humata2_revive::FollowStep;
use humata2_sessions::Agents;
use humata2_store::{SessionRecord, Store, StoreError, TabRecord};
use prost::Message;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};
use tokio::sync::watch;
use tonic::{Code, Request, Response, Status};

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
/// Metadata key naming a channel's folder on a Desk start the host relays
/// to another machine; it must match `humata2_svc_peer::host_relay::CHANNEL_FOLDER`.
pub const CHANNEL_FOLDER: &str = "humata-channel-folder";

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
    /// Ends `tab_id`'s token at once; Err only when saving that failed, which
    /// the next start makes good, since the tab is gone by then.
    fn revoke(&self, tab_id: &str) -> Result<(), StoreError>;
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
    /// Puts a call a rule sends to Ask me in the Inbox, with `run` to do it
    /// once approved, and answers the status the caller is given meanwhile
    /// (`humatad::inbox::Inbox::defer`).
    async fn defer(&self, caller: &Caller, asked: AccessRequest, key: Option<String>, run: Run) -> Status;
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
    /// The events after `after` the feed still keeps, then the live ones;
    /// with the sequence number the stream picks up from.
    fn subscribe(&self, after: Option<u64>) -> (u64, Events);
}

/// The feed's live events as one subscriber receives them
/// (`humatad::events::Feed`'s receiver).
#[tonic::async_trait]
pub trait LiveEvents: Send {
    /// The next event; `Lagged` when the subscriber fell behind and missed
    /// some. Cancel-safe.
    async fn recv(&mut self) -> Result<pb::Event, RecvError>;
    fn try_recv(&mut self) -> Result<pb::Event, TryRecvError>;
}

/// A subscription to the feed: the kept events it resumed after, then the
/// live ones.
pub struct Events {
    kept: VecDeque<pb::Event>,
    live: Box<dyn LiveEvents>,
}

impl Events {
    pub fn new(kept: impl IntoIterator<Item = pb::Event>, live: Box<dyn LiveEvents>) -> Self {
        Self {
            kept: kept.into_iter().collect(),
            live,
        }
    }

    /// The next event. Cancel-safe, so it can sit in a `select!`.
    pub async fn recv(&mut self) -> Result<pb::Event, RecvError> {
        match self.kept.pop_front() {
            Some(event) => Ok(event),
            None => self.live.recv().await,
        }
    }

    pub fn try_recv(&mut self) -> Result<pb::Event, TryRecvError> {
        match self.kept.pop_front() {
            Some(event) => Ok(event),
            None => self.live.try_recv(),
        }
    }
}

/// Session-event notifier port implemented by `humatad::session_service::Tell`.
#[tonic::async_trait]
pub trait DeskTellPort: Send + Sync {
    async fn session(&self, session_id: &str, change: pb::Change, by: Option<pb::Actor>);
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
    /// Removes the links to renamed folders once their holders have all
    /// ended, watching while any remains (`ChannelSvc::watch_aliases`).
    fn watch_aliases(&self);
}

/// Operation progress handle.
#[derive(Clone)]
pub struct Progress {
    inner: Arc<dyn ProgressPort>,
}

/// Backing trait for [`Progress`]. It is `Any` so the daemon can reach its
/// own handle behind a [`Progress`] it made (see [`Progress::port`]).
#[tonic::async_trait]
pub trait ProgressPort: Any + Send + Sync {
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
    /// The port behind this handle, for the daemon that made it to downcast
    /// to its own type.
    pub fn port(&self) -> &(dyn Any + Send + Sync) {
        &*self.inner
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
}

/// Link state of a remote machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkState {
    Up,
    Down,
}

/// Why a call can't be relayed to a remote machine. `answer` is the status
/// the link gave, which already says what failed and how to fix it.
#[derive(Debug, Clone, thiserror::Error)]
pub enum RelayError {
    /// The machine's link is down.
    #[error("{machine} is offline: {}", .answer.message())]
    Offline {
        machine: String,
        #[source]
        answer: Status,
    },
    /// The call couldn't be sent for another reason.
    #[error("{}", .answer.message())]
    Refused {
        #[source]
        answer: Status,
    },
}

impl From<RelayError> for Status {
    fn from(error: RelayError) -> Self {
        match error {
            RelayError::Offline { answer, .. } | RelayError::Refused { answer } => answer,
        }
    }
}

/// Why work on a remote machine failed.
#[derive(Debug, Clone, thiserror::Error)]
pub enum RemoteError {
    #[error(transparent)]
    Relay(#[from] RelayError),
    /// The machine answered with an error, or broke a promise of its call.
    /// `answer` is what the caller is told.
    #[error("{}", .answer.message())]
    Failed {
        #[source]
        answer: Status,
    },
}

impl From<RemoteError> for Status {
    fn from(error: RemoteError) -> Self {
        match error {
            RemoteError::Relay(relay) => Status::from(relay),
            RemoteError::Failed { answer } => answer,
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
    /// Reads the progress while nothing has taken it.
    pub fn with_progress<T>(&self, read: impl FnOnce(&Progress) -> T) -> Option<T> {
        self.progress.lock().unwrap_or_else(PoisonError::into_inner).as_ref().map(read)
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
    pub fn new(changes: tokio::sync::mpsc::Receiver<(String, LinkState)>) -> Self {
        Self(changes)
    }
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
    /// Waits until `machine`'s link is up; Err once it never can be (the
    /// machine was removed, or the daemon is stopping).
    async fn wait_up(&self, machine: &str) -> Result<(), RelayError>;
    /// The machines' link changes from now on.
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

/// The session service's work a Desk session shares with a channel session,
/// implemented by `humatad::session_service` (its agent watch, its turn,
/// its resume, its Moving status and the agent gate).
#[tonic::async_trait]
pub trait DeskSessionPort: Send + Sync {
    /// Refuses an agent that is switched off or isn't installed, installing
    /// it first when its switch says to (`agent_gate::ready`).
    async fn agent_ready(&self, agents: &Agents, agent: &str) -> Result<(), Status>;
    /// Refuses an agent whose switch is off (`agent_gate::enabled`).
    fn agent_enabled(&self, agents: &Agents, agent: &str) -> Result<(), Status>;
    /// The first of `preferred` that is on, else the first agent that is on
    /// (`agent_gate::pick`); every agent off is refused.
    fn pick_agent(&self, agents: &Agents, preferred: &[String]) -> Result<String, Status>;
    /// Watches `session`'s agent the way a channel session's is watched (SE-21).
    fn watch_agent(&self, session: String);
    /// What the agent of `session`, running in holder `holder_pid`, is doing
    /// in its turn: Idle, Working or Needs you.
    fn running_turn(&self, session: &str, holder_pid: u32) -> pb::SessionStatus;
    /// Shows `session` as Moving (to machine `to`, for `operation`) until the
    /// answered guard is dropped (MV-4).
    fn moving(&self, session: &str, to: &str, operation: &str) -> Moving;
    /// What each conversation of `session` records of its folder, moved from
    /// `from` to `to` at its next resume (`rename_folder::session_follows`).
    #[allow(clippy::too_many_arguments)]
    fn session_follows(
        &self,
        store: &Store,
        agents: &Agents,
        home: &Path,
        channels: &Path,
        session: &str,
        from: &Path,
        to: &Path,
    ) -> Result<Vec<FollowStep>, StoreError>;
    /// Brings `loaded`'s agent back for `progress`, which it ends.
    async fn run_resume(&self, by: pb::Actor, loaded: Loaded, progress: Progress);
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

/// Clean a Desk session title: runs of whitespace become one space. An
/// empty title is allowed; the caller gives it the fallback name.
pub fn clean_title(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
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

/// A trash item's id as the host's Bin view lists an item on `machine`:
/// `machine:id`. It must match `humata2_svc_files::forward::qualified`, or a
/// restore from that view can't find the Desk session the item holds.
pub fn qualified(machine: &str, id: &str) -> String {
    if machine.is_empty() {
        id.to_owned()
    } else {
        format!("{machine}:{id}")
    }
}

/// Checks `call` against `guard` unless the user already `approved` it:
/// `Some(asked)` when the rule says Ask me, so the call waits in the Inbox
/// (`channel_service::gate`). A rule of Never is refused by `guard.check`.
pub fn gate(
    guard: &Arc<dyn DeskGuardPort>,
    caller: &Caller,
    call: &Call,
    approved: bool,
) -> Result<Option<AccessRequest>, Status> {
    if approved {
        return Ok(None);
    }
    match guard.check(caller, call)? {
        Checked::Allowed => Ok(None),
        Checked::Ask(asked) => Ok(Some(asked)),
    }
}

/// Answers a unary call whose body ran as `first`. A body that stopped at an
/// Ask puts the call in the Inbox, with `again` to run it once the user
/// approves it: `again` must do the whole call afresh, with `approved` set,
/// and own what it needs (TP-13). The response's full name is `type_name`
/// (`channel_service::answer_gated`).
pub async fn answer_gated<T, Fut>(
    inbox: &Arc<dyn DeskInboxPort>,
    caller: &Caller,
    key: Option<String>,
    type_name: &'static str,
    first: Result<Gated<T>, Status>,
    again: impl FnOnce() -> Fut + Send + 'static,
) -> Result<Response<T>, Status>
where
    T: Message,
    Fut: Future<Output = Result<Gated<T>, Status>> + Send + 'static,
{
    match first? {
        Gated::Done(done) => Ok(Response::new(done)),
        Gated::Ask(asked) => {
            let run: Run = Box::new(move |_| {
                Box::pin(async move {
                    match again().await? {
                        Gated::Done(done) => Ok(Deferred {
                            result: done.encode_to_vec(),
                            result_type: type_name.to_owned(),
                        }),
                        Gated::Ask(_) => Err(internal(
                            "an approved call was sent to Ask me a second time",
                            "repeat the call",
                        )),
                    }
                })
            });
            Err(inbox.defer(caller, *asked, key, run).await)
        }
    }
}

/// Current unix time in milliseconds.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// Ending a terminal's holder (`humatad::holder_end`).
pub mod holder_end {
    use std::path::{Path, PathBuf};

    use humata2_holder::{EndError, HolderError, Stopped};

    /// A holder that neither took its shutdown nor could be ended by its pid.
    #[derive(Debug, thiserror::Error)]
    #[error(
        "couldn't stop {owner}: its holder at {} didn't take its shutdown, and its process {pid} couldn't be ended by its pid either; stop again, and if the process stays, check with `ps -p {pid}` that it is a humata2 holder, then end it by hand",
        socket.display()
    )]
    pub struct HolderEndError {
        owner: String,
        socket: PathBuf,
        pid: u32,
        #[source]
        source: EndError,
    }

    /// Shuts down the holder at `socket`, whose process is `pid`. When it
    /// doesn't take the shutdown, or its socket is gone, its process is
    /// ended by its pid instead, which is signalled only while the process
    /// list names it as a live holder. `owner` names whose holder it is, for
    /// the log and the error. Errs only when the process couldn't be ended.
    pub async fn shut_or_end(socket: &Path, pid: u32, owner: &str) -> Result<(), HolderEndError> {
        let stopped = humata2_holder::shutdown_or_end(socket, pid)
            .await
            .map_err(|source| HolderEndError {
                owner: owner.to_owned(),
                socket: socket.to_owned(),
                pid,
                source,
            })?;
        if let Stopped::Ended { reason } = stopped
            // A socket that is gone is how a holder that already stopped
            // looks, so only the other failures are news.
            && !matches!(reason, HolderError::Connect { .. })
        {
            humata2_rpc_kit::log::say!(
                "humatad2: the holder of {owner} didn't take its shutdown: {}; its process {pid} was ended by its pid instead",
                humata2_rpc_kit::chain(&reason)
            );
        }
        Ok(())
    }
}

/// Type alias for `Arc<dyn DeskTellPort>` (`session_service::Tell`).
pub type Tell = Arc<dyn DeskTellPort>;

/// Type alias for `Arc<dyn DeskRemotePort>` (`remote_work::RemoteWork`).
pub type RemoteWork = Arc<dyn DeskRemotePort>;

/// A session read from the store for a resume: what [`DeskSessionPort::run_resume`]
/// brings back.
pub struct Loaded {
    pub session: SessionRecord,
}

impl Loaded {
    /// Reads session `id`, or None when the store has no such session.
    /// Blocking.
    pub fn read(store: &Store, id: &str) -> Result<Option<Self>, StoreError> {
        Ok(store.session(id)?.map(|session| Self { session }))
    }
}

/// A session shown as Moving for as long as this is held
/// ([`DeskSessionPort::moving`]).
pub struct Moving {
    _held: Box<dyn Any + Send + Sync>,
}

impl Moving {
    /// Holds `held`, whose drop ends the Moving status.
    pub fn new(held: Box<dyn Any + Send + Sync>) -> Self {
        Self { _held: held }
    }
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

/// Moves each agent conversation file in `moves` with its folder's key
/// (`channel_service::carry_conversations`).
pub use humata2_svc_channels::move_folder::{MoveError, carry_conversations};

/// 1-arg carry failure mapper (`carry_service::failed`).
pub fn carry_failed(err: impl std::fmt::Display) -> Status {
    Status::internal(err.to_string())
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
}

/// Event feed type alias (`events::Feed`).
pub type Feed = dyn DeskFeedPort;

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::{Arc, Mutex, PoisonError};

    use humata2_access::{
        AccessRequest, AnswerAt, Call, Capability, ChannelId, ChannelRules, Decision, Grant, Policy, SessionId,
        Target,
    };
    use humata2_daemon_ports::{Caller, Gated, Run, UserVia};
    use humata2_store::{Placement, SessionRecord, Store};
    use prost::Message as _;
    use tonic::{Code, Status};

    use super::{
        Checked, DeskGuardPort, DeskInboxPort, Loaded, answer_gated, carry_conversations, gate, pb,
    };

    const CHANNEL: &str = "ch_1";

    fn ask() -> AccessRequest {
        AccessRequest {
            capability: Capability::DeskMoveToChannel,
            channel: ChannelId::new(CHANNEL),
            requester: SessionId::new("se_agent"),
            call: call(),
            always_let: None,
            ahead: false,
            answer_at: AnswerAt::Host,
        }
    }

    fn call() -> Call {
        Call::Use {
            capability: Capability::DeskMoveToChannel,
            target: Target::Channel(ChannelId::new(CHANNEL)),
        }
    }

    fn user() -> Caller {
        Caller::User(UserVia::App)
    }

    /// A guard whose rule for every call is Ask me, or Allow.
    struct Rule {
        asks: bool,
    }

    impl DeskGuardPort for Rule {
        fn check(&self, _caller: &Caller, _call: &Call) -> Result<Checked, Status> {
            Ok(if self.asks { Checked::Ask(ask()) } else { Checked::Allowed })
        }
        fn require(&self, _caller: &Caller, _call: &Call) -> Result<(), Status> {
            Ok(())
        }
        fn end_grants_of(&self, _session: &SessionId) -> Vec<Grant> {
            Vec::new()
        }
        fn channel_decision(&self, _channel: &ChannelId, _capability: Capability) -> Option<Decision> {
            None
        }
        fn channel_rules(&self, _channel: &ChannelId) -> Option<ChannelRules> {
            None
        }
        fn policy(&self) -> Policy {
            Policy::default()
        }
    }

    /// An Inbox that keeps what it was asked to defer.
    #[derive(Default)]
    struct Kept {
        deferred: Mutex<Vec<(AccessRequest, Option<String>, Run)>>,
    }

    #[tonic::async_trait]
    impl DeskInboxPort for Kept {
        async fn ask_read(
            &self,
            _caller: &Caller,
            _asked: AccessRequest,
            _operation: &str,
            _run: Option<Run>,
        ) -> Result<(), Status> {
            Ok(())
        }
        async fn withdraw_pending_of(&self, _session: &str, _reason: pb::WithdrawReason) -> Result<(), Status> {
            Ok(())
        }
        async fn revoked(&self, _id: &str, _at: std::time::SystemTime) -> Result<(), Status> {
            Ok(())
        }
        async fn defer(&self, _caller: &Caller, asked: AccessRequest, key: Option<String>, run: Run) -> Status {
            self.deferred.lock().unwrap_or_else(PoisonError::into_inner).push((asked, key, run));
            Status::permission_denied("waiting in the Inbox")
        }
    }

    fn operation(id: &str) -> pb::Operation {
        pb::Operation {
            id: Some(pb::OperationId { value: id.to_owned() }),
            ..pb::Operation::default()
        }
    }

    #[test]
    fn a_rule_of_ask_me_stops_the_call_until_the_user_approved_it() {
        let asks: Arc<dyn DeskGuardPort> = Arc::new(Rule { asks: true });
        let allows: Arc<dyn DeskGuardPort> = Arc::new(Rule { asks: false });

        assert_eq!(gate(&asks, &user(), &call(), false).unwrap(), Some(ask()));
        assert_eq!(gate(&asks, &user(), &call(), true).unwrap(), None);
        assert_eq!(gate(&allows, &user(), &call(), false).unwrap(), None);
    }

    #[tokio::test]
    async fn an_asked_call_waits_in_the_inbox_and_approving_it_runs_it_again() {
        let kept = Arc::new(Kept::default());
        let inbox: Arc<dyn DeskInboxPort> = kept.clone();

        let answered = answer_gated(
            &inbox,
            &user(),
            Some("key_1".to_owned()),
            "humata.v1.Operation",
            Ok(Gated::<pb::Operation>::Ask(Box::new(ask()))),
            || async { Ok(Gated::Done(operation("op_approved"))) },
        )
        .await;

        let refused = answered.expect_err("an asked call isn't answered until it is approved");
        assert_eq!(refused.code(), Code::PermissionDenied, "the caller gets the Inbox's answer");
        let (asked, key, run) = kept.deferred.lock().unwrap().pop().expect("the call waits in the Inbox");
        assert_eq!((asked, key.as_deref()), (ask(), Some("key_1")));
        let approved = run("rq_1".to_owned()).await.unwrap();
        assert_eq!(approved.result_type, "humata.v1.Operation");
        assert_eq!(pb::Operation::decode(approved.result.as_slice()).unwrap(), operation("op_approved"));
    }

    #[tokio::test]
    async fn an_allowed_call_is_answered_at_once() {
        let kept = Arc::new(Kept::default());
        let inbox: Arc<dyn DeskInboxPort> = kept.clone();

        let answered = answer_gated(
            &inbox,
            &user(),
            None,
            "humata.v1.Operation",
            Ok(Gated::Done(operation("op_now"))),
            || async { Ok(Gated::Done(operation("op_again"))) },
        )
        .await
        .unwrap();

        assert_eq!(answered.into_inner(), operation("op_now"));
        assert!(kept.deferred.lock().unwrap().is_empty(), "nothing waits in the Inbox");
    }

    #[test]
    fn a_saved_session_is_read_for_its_resume_and_an_unknown_one_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let record = SessionRecord {
            id: "se_desk".to_owned(),
            channel: humata2_sessions::DESK_CHANNEL.to_owned(),
            machine: "ma_host".to_owned(),
            title: "Notes".to_owned(),
            agent: "claude".to_owned(),
            folder: dir.path().join("desk/notes"),
            created_unix_ms: 1,
        };
        store.put_session(&record).unwrap();

        let loaded = Loaded::read(&store, "se_desk").unwrap().expect("the saved session is read");
        assert_eq!(loaded.session, record);
        assert!(Loaded::read(&store, "se_gone").unwrap().is_none());
    }

    /// A Desk session restored into a new folder takes its conversation
    /// with it, naming the new folder.
    #[test]
    fn a_restored_desk_sessions_conversation_moves_to_its_new_folder() {
        let dir = tempfile::tempdir().unwrap();
        let (old, new) = (dir.path().join("desk/notes"), dir.path().join("desk/notes-2"));
        let from = dir.path().join("conv/key-old/c1.jsonl");
        let to = dir.path().join("conv/key-new/c1.jsonl");
        fs::create_dir_all(from.parent().unwrap()).unwrap();
        fs::write(&from, format!("{{\"cwd\":\"{}\"}}\n", old.display())).unwrap();
        let placement = Placement {
            name: String::new(),
            folder: String::new(),
            old_path: old,
            new_path: new.clone(),
            now_unix_ms: 0,
        };

        carry_conversations(&placement, &[(from.clone(), to.clone())]).unwrap();

        assert_eq!(fs::read_to_string(&to).unwrap(), format!("{{\"cwd\":\"{}\"}}\n", new.display()));
        assert!(!from.exists(), "the conversation left its old place");
    }
}
