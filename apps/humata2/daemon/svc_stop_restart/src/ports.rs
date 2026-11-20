//! Narrow port traits and shared helpers decoupling `humata2_svc_stop_restart`
//! from `humatad`-internal services (`Guard`, `Inbox`, `SessionSvc`,
//! `RelayAuth`, `Uplinks`, `Relay`, `Upgrade`, `Work`, `Identities`).

use std::ffi::OsString;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use humata2_access::{AccessRequest, Call, Capability, Refusal};
use humata2_daemon_ports::{Caller, Deferred, Gated, PeerLinkState, Run, TabIdentity};
use humata2_sessions::Starter;
use humata2_store::StoreError;
use prost::Message;
use tokio::sync::watch;
use tokio_stream::Stream;
use tonic::transport::Channel;
use tonic::{Code, Request, Response, Status, Streaming};

use crate::pb;

pub use humata2_daemon_ports::{Release, UserVia, actor, user_via};
pub use humata2_rpc_kit::compression::Compression;
pub use humata2_rpc_kit::{
    API_MAJOR, API_MINOR, chain, error, error_of, failed, failure, internal, lock, missing,
    not_built_status as not_built, status_error_of,
};

/// Result of a guard check: allowed or must ask the user.
#[derive(Debug)]
pub enum Checked {
    Allowed,
    Ask(AccessRequest),
}

/// Connection identifier placed in request extensions by the transport layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ConnId(pub u64);

/// Peer transport facts placed in request extensions for relay completion.
#[derive(Clone, Debug, Default)]
pub struct PeerFacts {
    pub pid: Option<u32>,
    pub uid: Option<u32>,
}

/// Marker placed in request extensions when a local CLI call came from outside
/// every Humata session (D263).
#[derive(Clone, Copy, Debug, Default)]
pub struct OutsideEverySession(pub bool);

/// An in-flight session rebuild that was cut short by a daemon restart.
#[derive(Clone, Debug)]
pub struct CutRebuild {
    pub session_id: String,
    pub operation_id: String,
}

impl CutRebuild {
    pub fn note(&self, operations: &crate::restart::Operations) {
        operations.record(
            &self.operation_id,
            "rebuild",
            "recovering",
            crate::restart::OperationState::Pending {
                waits_for: "the restart to undo it".to_owned(),
            },
        );
    }
}

/// SSH connection target for one-off `humatad2 stop` over SSH.
#[derive(Clone, Debug)]
pub struct SshTarget {
    pub host: String,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub remote_bin: String,
}

/// Stream of `MachineToHost` messages returned by `DaemonService.PeerUplink`.
pub type UplinkStream = Pin<Box<dyn Stream<Item = Result<pb::MachineToHost, Status>> + Send>>;

/// Access-control port implemented by `humatad::guard::Guard`.
pub trait StopGuardPort: Send + Sync {
    fn check(&self, caller: &Caller, call: &Call) -> Result<Checked, Status>;
}

/// Inbox port for `StopSvc`, `Doctor`, and `stop::boot::Settle`.
#[tonic::async_trait]
pub trait StopInboxPort: Send + Sync {
    async fn ask_deferred(
        &self,
        caller: &Caller,
        asked: AccessRequest,
        response_type: &'static str,
        run: Run,
    ) -> Result<Deferred, Status>;

    async fn withdraw_all_pending(&self, reason: pb::WithdrawReason) -> Result<(), Status>;
}

/// Session repair port implemented by `humatad::session_service::SessionSvc`.
#[tonic::async_trait]
pub trait DoctorSessionsPort: Send + Sync {
    async fn repair_session(
        &self,
        caller: &Caller,
        id: String,
        approved: bool,
    ) -> Result<Gated<pb::Operation>, Status>;

    async fn repair_tab(
        &self,
        caller: &Caller,
        id: String,
        approved: bool,
    ) -> Result<Gated<()>, Status>;
}

/// Session restart lifecycle hooks implemented by `humatad::session_service`.
#[tonic::async_trait]
pub trait RestartSessionsPort: Send + Sync {
    fn cut_rebuilds(&self, starter: &Starter) -> Vec<CutRebuild>;
    async fn after_restart(&self);
    async fn undo_cut_rebuilds(
        &self,
        starter: &Starter,
        cut: Vec<CutRebuild>,
        operations: &crate::restart::Operations,
    );
}

/// Tab token and host environment provider for revived tabs.
pub trait TabEnvPort: Send + Sync {
    fn host_variables(&self, tab_id: &str) -> Result<Vec<(OsString, OsString)>, String>;
    fn revoke(&self, tab_id: &str) -> Result<(), String>;
    fn mint_tab_env(
        &self,
        tab_id: &str,
        session: &humata2_store::SessionRecord,
        kind: TabIdentity,
        socket: &std::path::Path,
    ) -> Result<Vec<(OsString, OsString)>, String>;
    fn tools_path(&self, host_path: Option<OsString>) -> Option<OsString>;
    /// The variables that point a local agent tab at its session's composed
    /// config home (D895): the agent's own home variable, the FUSE root and
    /// the machine name. An agent with no composed home (a session's shell,
    /// D916) gets only the FUSE root and the machine name; a remote session
    /// gets nothing, and then its home is as it was.
    fn session_compose_env(
        &self,
        session: &humata2_store::SessionRecord,
        agent: &str,
    ) -> Result<Vec<(OsString, OsString)>, String>;
    /// The composed config home of `session`'s `agent`, where that agent's
    /// own conversations are kept; None unless the folder exists, so a
    /// session launched before composing, or an agent with none, is read
    /// from the agent's usual dir.
    fn session_home(&self, session: &str, agent: &str) -> Option<std::path::PathBuf>;
}

/// Relay authentication port implemented by `humatad::relay_auth::RelayAuth`.
pub trait DaemonRelayAuthPort: Send + Sync {
    fn begin(&self, conn: u64, now: Instant) -> [u8; 32];
    fn complete(
        &self,
        conn: u64,
        facts: &PeerFacts,
        host_nonce: &[u8],
        proof: &[u8],
        now: Instant,
    ) -> Result<(), Status>;
}

/// Uplink port implemented by `humatad::away::Uplinks`.
pub trait DaemonUplinksPort: Send + Sync {
    fn reachable(&self) -> bool;
    fn open(
        &self,
        caller: Option<&Caller>,
        stream: Streaming<pb::HostToMachine>,
    ) -> Result<UplinkStream, Status>;
}

/// Proven peer-link channel port implemented by `humatad::relay::Relay`.
pub trait DaemonLinkPort: Send + Sync {
    fn proven_channel(&self) -> Option<Channel>;
}

/// Upgrade runner port implemented by `humatad::upgrade::run::Runner`.
#[tonic::async_trait]
pub trait DaemonUpgradePort: Send + Sync {
    async fn upgrade(
        &self,
        request: Request<pb::UpgradeRequest>,
    ) -> Result<Response<pb::UpgradeResponse>, Status>;
}

/// Remote machine link & SSH port implemented by `humatad::machine_service::Work`.
pub trait RemoteStopPort: Send + Sync {
    fn stops(&self) -> Option<&crate::stop::remote::HostStops>;
    fn publish_machines(&self);
    fn watch_link(&self, machine: &str) -> watch::Receiver<PeerLinkState>;
    fn channel(&self, machine: &str) -> Result<Channel, String>;
    fn link_up(&self, machine: &str, runtime: &tokio::runtime::Handle);
    fn stop_sessions_command(
        &self,
        machine: &str,
        stop_id: &str,
        by_machine: &str,
        reason: &str,
    ) -> Result<humata2_machines::Invocation, humata2_machines::ManageError>;
}

/// Extracts the authenticated [`Caller`] from a tonic request's extensions.
pub fn caller<T>(request: &Request<T>) -> Result<Caller, Status> {
    request.extensions().get::<Caller>().cloned().ok_or_else(|| {
        internal(
            "this call reached its handler without being authenticated",
            "",
        )
    })
}

/// Extracts an optional [`Caller`] from a tonic request's extensions.
pub fn caller_if_any<T>(request: &Request<T>) -> Result<Option<Caller>, Status> {
    Ok(request.extensions().get::<Caller>().cloned())
}

/// Extracts the connection id from a tonic request's extensions.
pub fn conn_id<T>(request: &Request<T>) -> Option<u64> {
    request.extensions().get::<ConnId>().map(|id| id.0)
}

/// Extracts peer transport facts from a tonic request's extensions.
pub fn peer_facts<T>(request: &Request<T>) -> PeerFacts {
    request
        .extensions()
        .get::<PeerFacts>()
        .cloned()
        .unwrap_or_default()
}

/// Whether the request came from a local caller outside every session.
pub fn outside_every_session<T>(request: &Request<T>) -> bool {
    request
        .extensions()
        .get::<OutsideEverySession>()
        .is_some_and(|flag| flag.0)
}

/// Status for an unauthenticated caller.
pub fn unauthenticated() -> Status {
    failure(
        Code::Unauthenticated,
        error(
            "this call needs a credential",
            "sign in with `humata login` or run from the app",
            pb::error::Kind::Denied(pb::Denied {
                source: pb::denied::Source::Unspecified as i32,
                capability: 0,
                request: None,
                user_note: String::new(),
            }),
        ),
    )
}

/// Status for a script caller whose rule requires interactive approval.
pub fn needs_approval(_capability: Capability) -> Status {
    failure(
        Code::PermissionDenied,
        error(
            "this action requires interactive approval, which a script cannot request",
            "run the command as the user or allow it in policy",
            pb::error::Kind::Denied(pb::Denied {
                source: pb::denied::Source::Unspecified as i32,
                capability: 0,
                request: None,
                user_note: String::new(),
            }),
        ),
    )
}

/// Checks `call` against `guard`; returns `Ok(Some(asked))` when the rule says
/// Ask me and the call has not yet been approved.
pub fn gate(
    guard: &Arc<dyn StopGuardPort>,
    caller: &Caller,
    call: &Call,
    approved: bool,
) -> Result<Option<Box<AccessRequest>>, Status> {
    if approved {
        return Ok(None);
    }
    match guard.check(caller, call)? {
        Checked::Allowed => Ok(None),
        Checked::Ask(asked) => Ok(Some(Box::new(asked))),
    }
}

/// Current wall-clock time as a protobuf `Timestamp`.
pub fn now_timestamp() -> humata2_grpc_core::Timestamp {
    let since = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    humata2_grpc_core::Timestamp {
        seconds: i64::try_from(since.as_secs()).unwrap_or(i64::MAX),
        nanos: i32::try_from(since.subsec_nanos()).unwrap_or(0),
    }
}

/// Formats a [`StoreError`] as a gRPC `Status`.
pub fn store_error(err: StoreError, _machine: &str, doing: &str) -> Status {
    internal(format!("couldn't {doing}: {}", chain(&err)), "")
}

/// Formats a [`Refusal`] as a gRPC `PermissionDenied` status.
pub fn refused(refusal: &Refusal) -> Status {
    failure(
        Code::PermissionDenied,
        error(
            refusal.to_string(),
            "check your permissions and try again",
            pb::error::Kind::Denied(pb::Denied {
                source: pb::denied::Source::Unspecified as i32,
                capability: 0,
                request: None,
                user_note: String::new(),
            }),
        ),
    )
}

/// Refusal status for a call that only the user may make.
pub fn user_only_call(_capability: Capability, doing: &str, fix: &str) -> Status {
    failure(
        Code::PermissionDenied,
        error(
            format!("only the user can do {doing}"),
            fix,
            pb::error::Kind::Denied(pb::Denied {
                source: pb::denied::Source::Unspecified as i32,
                capability: 0,
                request: None,
                user_note: String::new(),
            }),
        ),
    )
}

/// Answers a gated RPC: if `first` is `Done`, returns it directly; if `Ask`,
/// posts to the inbox and decodes the deferred response once approved.
pub async fn answer_gated<T, F, Fut>(
    inbox: &Arc<dyn StopInboxPort>,
    caller: &Caller,
    response_type: &'static str,
    first: Result<Gated<T>, Status>,
    retry: F,
) -> Result<Response<T>, Status>
where
    T: Message + Default + Send + 'static,
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = Result<Gated<T>, Status>> + Send + 'static,
{
    match first? {
        Gated::Done(value) => Ok(Response::new(value)),
        Gated::Ask(asked) => {
            let run: Run = Box::new(move |_approval_id| {
                Box::pin(async move {
                    match retry().await? {
                        Gated::Done(value) => Ok(Deferred {
                            result: value.encode_to_vec(),
                            result_type: response_type.to_owned(),
                        }),
                        Gated::Ask(_) => Err(internal(
                            "approved call unexpectedly asked for approval again",
                            "",
                        )),
                    }
                })
            });
            let deferred = inbox
                .ask_deferred(caller, *asked, response_type, run)
                .await?;
            let decoded = T::decode(deferred.result.as_slice()).map_err(|err| {
                internal(
                    format!("couldn't decode approved {response_type}: {err}"),
                    "",
                )
            })?;
            Ok(Response::new(decoded))
        }
    }
}
