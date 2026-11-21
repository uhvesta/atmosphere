//! Narrow port traits and value types decoupling `humata2_svc_machines` from
//! `humatad`-internal services (`Guard`, `Inbox`, `Feed`, `Operations`,
//! `Storage`, `HostVars`, `Links`, `PeerReports`, `HostStops`, `SkillsHost`,
//! `Upgrade`, `Login`, `RunningHolders`, `SessionSvc`, `Terminal`, `Trash`,
//! `ChannelSvc`, `Away`, `Restart`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use humata2_access::{AccessRequest, Call, Capability, Refusal};
use humata2_daemon_ports::{Caller, Deferred, Gated, Run};
use crate::pb;
use humata2_machines::{HostEntry, OpStep, StepEvent};
use humata2_store::{Store, StoreError};
use tokio::sync::{broadcast, watch};
use tonic::{Code, Request, Response, Status};

pub use humata2_daemon_ports::actor;
pub use humata2_rpc_kit::{
    blocking, chain, error, error_of, failed, failure, internal, invalid_field, lock, missing,
    not_built_status, not_found, now_ms, timestamp_ms,
};

/// Maps a [`StoreError`] to a [`tonic::Status`].
pub fn store_error(err: StoreError, machine: &str, what: &str) -> Status {
    internal(
        format!("couldn't {what} on {machine}: {}", chain(&err)),
        "check that the daemon's state folder is readable and writable",
    )
}

/// Maps an access [`Refusal`] to a [`tonic::Status`].
pub fn refused(refusal: &Refusal) -> Status {
    Status::permission_denied(refusal.to_string())
}

/// Refuses an agent call to a user-only capability.
pub fn user_only_call(capability: Capability, what: &str, fix: &str) -> Status {
    let _ = capability;
    failure(
        Code::PermissionDenied,
        error(
            format!("{what} is the user's to do, at the app; an agent or a script can't do it"),
            fix,
            pb::error::Kind::UserOnly(pb::UserOnly { capability: 0 }),
        ),
    )
}

// ---------------------------------------------------------------------------
// Caller extraction (mirrors humatad::identity::caller)
// ---------------------------------------------------------------------------

/// Extracts the authenticated [`Caller`] from tonic request extensions.
pub fn caller<T>(request: &Request<T>) -> Result<Caller, Status> {
    request.extensions().get::<Caller>().cloned().ok_or_else(|| {
        internal(
            "the call reached the handler without an established caller",
            "restart humatad2",
        )
    })
}

/// Inserts a [`Caller`] into tonic request extensions (used by tests and
/// internal callers).
pub fn with_caller<T>(mut request: Request<T>, caller: Caller) -> Request<T> {
    request.extensions_mut().insert(caller);
    request
}

// ---------------------------------------------------------------------------
// Guard port
// ---------------------------------------------------------------------------

/// Result of a guard check: allowed or must ask the user.
#[derive(Debug)]
pub enum Checked {
    Allowed,
    Ask(AccessRequest),
}

/// Access-control port implemented by `humatad::guard::Guard`.
pub trait MachineGuardPort: Send + Sync {
    fn machine(&self) -> &str;
    fn check(&self, caller: &Caller, call: &Call) -> Result<Checked, Status>;
    fn require(&self, caller: &Caller, call: &Call) -> Result<(), Status>;
    fn principal(&self, caller: &Caller) -> String;
    fn is_present_user(&self, caller: &Caller) -> bool;
}

/// Checks `call` against `guard`, returning `Ok(Some(ask))` if the rule says
/// Ask me and `!approved`, or `Ok(None)` if allowed.
pub fn gate(
    guard: &dyn MachineGuardPort,
    caller: &Caller,
    call: &Call,
    approved: bool,
) -> Result<Option<Box<AccessRequest>>, Status> {
    if approved {
        return Ok(None);
    }
    match guard.check(caller, call)? {
        Checked::Allowed => Ok(None),
        Checked::Ask(ask) => Ok(Some(Box::new(ask))),
    }
}

// ---------------------------------------------------------------------------
// Feed port
// ---------------------------------------------------------------------------

/// Event-feed port implemented by `humatad::events::Feed`.
pub trait MachineFeedPort: Send + Sync {
    fn publish(&self, change: pb::Change, payload: pb::event::Payload, by: Option<pb::Actor>);
    fn subscribe(&self) -> broadcast::Receiver<Arc<pb::Event>>;
}

// ---------------------------------------------------------------------------
// Inbox port
// ---------------------------------------------------------------------------

/// Inbox port implemented by `humatad::inbox::Inbox`.
#[tonic::async_trait]
pub trait MachineInboxPort: Send + Sync {
    async fn ask(
        &self,
        caller: &Caller,
        asked: AccessRequest,
        operation: &str,
        run: Option<Run>,
    ) -> Result<(), Status>;

    async fn post_host_ask(
        &self,
        key: &str,
        card: pb::InboxRequest,
        run: Run,
    ) -> Result<(), Status>;

    async fn withdraw_host_ask(&self, key: &str, reason: pb::WithdrawReason);

    async fn withdraw_machine(
        &self,
        machine: &str,
        reason: pb::WithdrawReason,
    ) -> Result<(), Status>;

    async fn forget_journal(&self, machine: &str);
    fn forget_journal_now(&self, machine: &str);
}

/// Turns a [`Gated<R>`] result into a tonic response, posting an Inbox ask when
/// the guard requested approval.
pub async fn answer_gated<R>(
    inbox: &dyn MachineInboxPort,
    caller: &Caller,
    operation: &str,
    gated: Gated<R>,
    run: Option<Run>,
) -> Result<Response<R>, Status> {
    match gated {
        Gated::Done(reply) => Ok(Response::new(reply)),
        Gated::Ask(asked) => {
            inbox.ask(caller, *asked, operation, run).await?;
            Err(Status::permission_denied("approval pending"))
        }
    }
}

/// Convenience wrapper matching `humatad::channel_service::answer_gated`.
pub async fn answer_gated_call<R, F, Fut>(
    inbox: &Arc<dyn MachineInboxPort>,
    caller: &Caller,
    _key: Option<String>,
    result_type: &'static str,
    first: Result<Gated<R>, Status>,
    rerun: F,
) -> Result<Response<R>, Status>
where
    R: prost::Message + Send + 'static,
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<Gated<R>, Status>> + Send + 'static,
{
    match first? {
        Gated::Done(reply) => Ok(Response::new(reply)),
        Gated::Ask(asked) => {
            let run: Run = Box::new(move |_| {
                Box::pin(async move {
                    match rerun().await? {
                        Gated::Done(reply) => Ok(deferred(result_type, &reply)),
                        Gated::Ask(_) => Err(Status::permission_denied("approval pending")),
                    }
                })
            });
            inbox.ask(caller, *asked, result_type, Some(run)).await?;
            Err(Status::permission_denied("approval pending"))
        }
    }
}

// ---------------------------------------------------------------------------
// Operations port
// ---------------------------------------------------------------------------

/// Where a persisted operation originated.
#[derive(Clone, Debug, Default)]
pub struct OperationOrigin {
    pub caller_data: String,
    pub idempotency_key: Option<String>,
}

/// Progress handle for a running long-lived operation.
pub trait OperationProgress: Send + Sync {
    fn id(&self) -> String;
    fn operation(&self) -> pb::Operation {
        self.snapshot()
    }
    fn step(&self, index: usize, state: pb::StepState, detail: String);
    fn set_step(&self, key: &str, state: pb::StepState, detail: Option<String>);
    fn step_event(&self, event: &StepEvent);
    fn detail(&self, detail: String);
    fn cancelled(&self) -> bool;
    fn succeed(&self, result: Option<pb::operation::Result>);
    fn end_cancelled(&self);
    fn fail(&self, error: pb::Error);
    fn rerun_with(&self, rerun: Arc<dyn Fn(Arc<dyn OperationProgress>) + Send + Sync>);
    fn finish_ok(&self, result_type: &str, result_bytes: Vec<u8>);
    fn finish_err(&self, error: pb::Error);
    fn snapshot(&self) -> pb::Operation;
}

/// Result of starting an operation: either a newly started progress handle or
/// an already-existing operation for the same idempotency key.
pub enum OperationStarted {
    Started(Arc<dyn OperationProgress>),
    Existing(pb::Operation),
}

/// Operations tracker port implemented by `humatad::operations::Operations`.
pub trait MachineOperationsPort: Send + Sync {
    fn existing(
        &self,
        caller: &Caller,
        key: Option<&pb::IdempotencyKey>,
    ) -> Result<Option<pb::Operation>, Status>;

    fn start(
        &self,
        caller: &Caller,
        kind: pb::OperationKind,
        key: Option<pb::IdempotencyKey>,
        steps: Vec<String>,
    ) -> Result<OperationStarted, Status>;

    fn start_op(
        &self,
        caller: &Caller,
        kind: pb::OperationKind,
        target: pb::EntityRef,
        steps: &[OpStep],
        retryable: bool,
        key: Option<pb::IdempotencyKey>,
    ) -> Result<OperationStarted, Status>;

    fn start_steps(
        &self,
        caller: &Caller,
        kind: pb::OperationKind,
        key: Option<pb::IdempotencyKey>,
        steps: Vec<pb::OperationStep>,
    ) -> Result<OperationStarted, Status>;

    fn restore(
        &self,
        operation: pb::Operation,
        origin: &OperationOrigin,
    ) -> Option<Arc<dyn OperationProgress>>;
}

// ---------------------------------------------------------------------------
// Storage, Trash, Channels, HostVars, HostProbe ports
// ---------------------------------------------------------------------------

/// Trash-bin port implemented by `humatad::trash_service::Bin`.
pub trait MachineTrashPort: Send + Sync {
    fn move_to_folder(&self, folder: PathBuf);
}

/// Local storage port implemented by `humatad::storage::Storage`.
pub trait MachineStoragePort: Send + Sync {
    fn store(&self) -> &Store;
    fn holders(&self) -> PathBuf;
    fn root(&self) -> PathBuf;
    fn data_dir(&self) -> PathBuf;
    /// Where channels are: the one layout every service of this daemon
    /// shares, so a root change reaches them while the daemon runs.
    fn layout(&self) -> humata2_sessions::Layout;
    fn storage_summary(&self) -> Result<pb::StorageReport, Status>;
    fn report(&self, extra: Vec<pb::RetentionSkip>) -> Result<pb::StorageReport, Status>;
    fn machine_of_session(&self, session: &str) -> Result<Option<String>, Status>;
    fn clear_build_cache(
        &self,
        request: &pb::ClearBuildCacheRequest,
    ) -> Result<pb::ClearBuildCacheResponse, Status>;
    fn trim_logs(&self, request: &pb::TrimLogsRequest) -> Result<pb::TrimLogsResponse, Status>;
    fn running_agents(&self) -> Result<u32, Status>;
    fn trash_bin(&self) -> Option<Arc<dyn MachineTrashPort>>;
}

/// Error from moving channel conversations when changing root.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct MoveError {
    pub message: String,
    pub fix: String,
}

/// Channel folder migration port implemented by `humatad::channel_service`.
pub trait MachineChannelsPort: Send + Sync {
    fn carry_conversations(
        &self,
        store: &Store,
        from_root: &Path,
        to_root: &Path,
    ) -> Result<(), MoveError>;

    fn rewrite_folders(
        &self,
        store: &Store,
        from_root: &Path,
        to_root: &Path,
    ) -> Result<(), Status>;
}

/// Host environment variables error.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct HostVarsError {
    pub message: String,
    pub fix: String,
    pub code: Code,
    /// What clients branch on, such as the field a refused name or value is in.
    pub kind: Option<pb::error::Kind>,
}

impl HostVarsError {
    pub fn status(&self) -> Status {
        failure(
            self.code,
            pb::Error {
                message: self.message.clone(),
                fix: self.fix.clone(),
                retry: None,
                kind: self.kind.clone(),
            },
        )
    }
}

/// Host variables port implemented by `humatad::host_vars::HostVars`.
pub trait MachineHostVarsPort: Send + Sync {
    fn names(&self) -> Result<Vec<String>, HostVarsError>;
    fn check(&self, name: &str, value: &str) -> Result<(), HostVarsError>;
    fn set(&self, name: &str, value: &str) -> Result<(), HostVarsError>;
    fn remove(&self, name: &str) -> Result<(), HostVarsError>;
}

/// Local host probe port implemented by `humatad::peer::host_probe`.
pub trait MachineHostProbePort: Send + Sync {
    fn read_probe(&self, include_folders: bool) -> pb::ProbeResponse;
}

// ---------------------------------------------------------------------------
// Peer relay / links / reports / dial ports
// ---------------------------------------------------------------------------

/// Live state of a machine's peer link.
#[derive(Clone, Debug)]
pub enum LinkState {
    Up { channel: tonic::transport::Channel },
    Down,
}

/// Error when a machine's peer link is offline.
#[derive(Debug, thiserror::Error)]
#[error("machine {machine} is offline")]
pub struct RelayError {
    pub machine: String,
}

impl From<RelayError> for Status {
    fn from(err: RelayError) -> Self {
        failure(
            Code::Unavailable,
            error(
                err.to_string(),
                "Sessions there keep running. Use Reconnect now on the machine's page, then try again.",
                pb::error::Kind::MachineOffline(pb::MachineOffline {
                    machine: None,
                    machine_name: err.machine,
                    since: None,
                }),
            ),
        )
    }
}

/// Live peer links port implemented by `humatad::host_relay::Links`.
pub trait MachineLinksPort: Send + Sync {
    fn channel(&self, machine: &str) -> Result<tonic::transport::Channel, RelayError>;
    fn up(&self, machine: &str, channel: tonic::transport::Channel);
    fn down(&self, machine: &str);
    fn forget(&self, machine: &str);
    fn watch(&self, machine: &str) -> watch::Receiver<LinkState>;
    fn watch_all(&self) -> watch::Receiver<HashMap<String, LinkState>>;
}

/// Peer reports sink port implemented by `humatad::peer_reports::PeerReports`.
pub trait MachinePeerReportsPort: Send + Sync {
    fn link_up(&self, machine: &str);
    fn link_down(&self, machine: &str, at: SystemTime);
    fn forget(&self, machine: &str);
    fn accept(&self, machine: &str, message: pb::MachineToHost);
}

/// Event emitted by a peer link supervisor.
#[derive(Debug)]
pub enum LinkEvent {
    Up {
        daemon_version: String,
        channel: tonic::transport::Channel,
        /// The stop the machine's Hello told, while its sessions are stopped (D262).
        stopped: Option<pb::SessionsStopped>,
    },
    Down {
        reason: String,
        next_retry_at: Option<SystemTime>,
    },
    IdentityMismatch {
        reason: String,
    },
}

/// Peer link dialer port implemented by `humatad::peer_client`.
pub trait MachinePeerDialPort: Send + Sync {
    fn spawn_supervisor(
        &self,
        entry: HostEntry,
        on_event: Arc<dyn Fn(LinkEvent) + Send + Sync>,
    ) -> tokio::task::AbortHandle;
}

// ---------------------------------------------------------------------------
// Stops, Skills, Upgrade, Login, Holders, Sessions, Terminal, Away, Restart
// ---------------------------------------------------------------------------

/// Stop copy metadata for a machine.
#[derive(Clone, Debug)]
pub struct StopCopy {
    pub pb_record: pb::SessionsStopped,
    pub by_user: bool,
    pub delivered: bool,
}

/// Remote stops port implemented by `humatad::stop::remote::HostStops`.
#[tonic::async_trait]
pub trait MachineStopsPort: Send + Sync {
    fn get(&self, machine: &str) -> Option<StopCopy>;
    fn this_machine(&self) -> Option<pb::SessionsStopped>;
    fn link_up(
        &self,
        machine: &str,
        told: Option<&pb::SessionsStopped>,
        channel: tonic::transport::Channel,
    );
    fn remove(&self, machine: &str) -> Result<(), Status>;
    fn forget(&self, machine: &str);
    async fn stop_remote(
        &self,
        machine: &str,
        stop_id: String,
        reason: String,
        via: pb::StopVia,
        by: String,
    ) -> Result<StopCopy, Status>;
    async fn start_remote(&self, machine: &str) -> Result<bool, Status>;
}

/// Skills host port implemented by `humatad::skills_service::SkillsHost`.
#[tonic::async_trait]
pub trait MachineSkillsPort: Send + Sync {
    fn health(&self, machine: &str) -> (bool, Option<pb::Error>);
    fn link_up(&self, machine: &str, channel: tonic::transport::Channel);
    fn link_down(&self, machine: &str);
    async fn forget(&self, machine: &str) -> Result<(), Status>;
    fn forget_now(&self, machine: &str) -> Result<(), Status>;
}

/// Upgrade watch port implemented by `humatad::upgrade::items::Watch`.
pub trait MachineUpgradePort: Send + Sync {
    fn link_up(&self, machine: &str, daemon_version: &str);
    fn forget(&self, machine: &str);
}

/// Login check port implemented by `humatad::login_service`.
pub trait MachineLoginPort: Send + Sync {
    fn needs_login(&self, entry: &HostEntry, reachability: pb::Reachability) -> pb::Reachability;
    fn login_needs_user(&self, machine: &str, reason: &str) -> pb::LoginNeedsUser;
}

/// Running holders port implemented by `humatad::running_holders`.
#[tonic::async_trait]
pub trait MachineHoldersPort: Send + Sync {
    async fn list(&self, machine: &str) -> Result<Vec<pb::RunningHolder>, Status>;
    async fn stop(&self, machine: &str, tab: &str) -> Result<(), Status>;
    async fn resume(&self, machine: &str, caller: &Caller, tab: &str) -> Result<(), Status>;
    async fn end(&self, machine: &str, tab: &str) -> Result<(), Status>;
    async fn adopt(&self, machine: &str, caller: &Caller, tab: &str, channel: &str) -> Result<String, Status>;
    async fn let_go(&self, sessions: Vec<String>, channels: Vec<String>) -> Result<u32, Status>;
}

/// Minimal session view needed by machine scripts `wait_idle`.
#[derive(Clone, Debug)]
pub struct ScriptSessionView {
    pub agent: String,
    pub turn_end: pb::TurnEndSource,
    pub status: pb::SessionStatus,
}

/// Session service port for machine scripts.
#[tonic::async_trait]
pub trait MachineSessionsPort: Send + Sync {
    async fn start_for_script(
        &self,
        machine: &str,
        agent: &str,
        channel: &str,
        title: &str,
        key: &str,
    ) -> Result<String, Status>;

    async fn script_view(&self, session: &str) -> Result<ScriptSessionView, Status>;
}

/// Terminal port for machine scripts.
#[tonic::async_trait]
pub trait MachineTerminalPort: Send + Sync {
    async fn type_for_script(
        &self,
        session: &str,
        text: &str,
        enter: bool,
        key: &str,
    ) -> Result<(), Status>;
}

/// Away forwarder port implemented by `humatad::away::Forwarder`.
#[tonic::async_trait]
pub trait MachineAwayPort: Send + Sync {
    async fn forward(
        &self,
        method: &str,
        caller: &Caller,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, Status>;
}

/// Restart operation state port implemented by `humatad::restart::Started`.
#[derive(Clone, Debug)]
pub enum RestartOpState {
    Pending { waits_for: String },
    Running,
    Done,
    Failed { reason: String },
}

pub trait MachineRestartPort: Send + Sync {
    fn set_operation_state(&self, name: &str, state: RestartOpState);
}

/// Helper to create a `Deferred` from a prost message.
pub fn deferred<M: prost::Message>(result_type: &str, msg: &M) -> Deferred {
    Deferred {
        result: msg.encode_to_vec(),
        result_type: result_type.to_owned(),
    }
}
