//! Narrow port traits, callbacks, and value types decoupling `humata2_svc_peer`
//! from `humatad`-internal services so `svc_peer` compiles 100% standalone.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use humata2_daemon_ports::{Caller, UserVia};
use humata2_grpc::humata::v1 as pb;
use humata2_store::StoreError;
use tokio::sync::{broadcast, watch};
use tonic::{Code, Request, Status};

pub use humata2_daemon_ports::actor;
pub use humata2_rpc_kit::{
    API_MAJOR, API_MINOR, Role, blocking, chain, error, error_of, failed, failure, internal,
    invalid_field, missing, not_found, now_ms, timestamp_ms,
};

/// Default restart wait duration (matches `humatad::restart::RESTART_WAIT`).
pub const RESTART_WAIT: Duration = Duration::from_secs(30);

/// Default restart fix hint (matches `humatad::restart::RESTART_FIX`).
pub const RESTART_FIX: &str = "wait for the daemon to finish starting, then retry";

/// Error type for peer link / relay startup (decoupled from `humatad::error::DaemonError`).
#[derive(Debug)]
pub struct PeerError(pub String);

impl std::fmt::Display for PeerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PeerError {}

impl From<std::io::Error> for PeerError {
    fn from(e: std::io::Error) -> Self {
        Self(e.to_string())
    }
}

impl From<tonic::transport::Error> for PeerError {
    fn from(e: tonic::transport::Error) -> Self {
        Self(e.to_string())
    }
}

/// Callback for connecting to or starting the local daemon (injected into `peer_link`).
pub type ConnectOrStartFn = Arc<
    dyn Fn() -> Pin<Box<dyn Future<Output = Result<tokio::net::UnixStream, PeerError>> + Send>>
        + Send
        + Sync,
>;

/// Callback for awaiting the daemon stop signal (injected into `peer_link`).
pub type StopSignalFn =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Bridge port for stdio peer framing (decoupled from `humatad::bridge`).
pub mod bridge {
    use super::PeerError;
    use tokio::io::{AsyncRead, AsyncWrite};

    pub async fn run_stdio<R, W>(_reader: R, _writer: W) -> Result<(), PeerError>
    where
        R: AsyncRead + Unpin + Send,
        W: AsyncWrite + Unpin + Send,
    {
        Ok(())
    }
}

/// Attach timestamp helper (matches `humatad::attach::now`).
pub mod attach {
    use super::{SystemTime, UNIX_EPOCH};
    use prost_types::Timestamp;

    pub fn now() -> Timestamp {
        let dur = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        Timestamp {
            seconds: i64::try_from(dur.as_secs()).unwrap_or(i64::MAX),
            nanos: i32::try_from(dur.subsec_nanos()).unwrap_or(0),
        }
    }
}

/// Identity header constants and peer request marker (matches `humatad::identity`).
pub mod identity {
    use humata2_daemon_ports::Caller;
    use tonic::body::Body;
    use tonic::codegen::http;

    pub const TOKEN_HEADER: &str = "x-humata-token";
    pub const LOGIN_HEADER: &str = "x-humata-login";
    pub const SCRIPT_HEADER: &str = "x-humata-script";

    pub fn mark_peer(mut request: http::Request<Body>) -> http::Request<Body> {
        request.extensions_mut().insert(Caller::Peer);
        request
    }
}

/// Build a `peer_unproven` gRPC status error.
pub fn peer_unproven(
    reason: pb::peer_unproven::Reason,
    machine: &str,
    message: impl Into<String>,
    fix: impl Into<String>,
) -> Status {
    failure(
        Code::Unauthenticated,
        pb::Error {
            message: message.into(),
            fix: fix.into(),
            retry: None,
            kind: Some(pb::error::Kind::PeerUnproven(pb::PeerUnproven {
                reason: reason as i32,
                machine: Some(pb::MachineId {
                    value: machine.to_owned(),
                }),
            })),
        },
    )
}

/// Build an unimplemented `not_built` gRPC status error.
pub fn not_built(method: &str) -> Status {
    humata2_rpc_kit::not_built_status(method)
}

/// Policy source port (decoupled from `humatad::away::PolicySource`).
pub trait PolicySourcePort: Send + Sync {
    fn snapshot(&self, machine: &str) -> Result<pb::PolicySnapshot, std::io::Error>;
    fn changes(&self) -> watch::Receiver<u64>;
    fn linked(&self, machine: &str);
    fn acked(&self, machine: &str, rev: u64);
}

/// Board service port (decoupled from `humatad::board_service::BoardSvc`).
#[tonic::async_trait]
pub trait PeerBoardPort: Send + Sync {
    async fn replay_at(&self, machine: &str, entry: &pb::JournalEntry, link_up: i64) -> Replayed;
    async fn forwarded(&self, machine: &str, call: pb::ForwardedCall) -> pb::ForwardedResult;
}

/// Relay registration handle returned by [`PeerInboxPort::relays`].
pub trait PeerRelaysPort: Send + Sync {
    /// The registration lasts until the returned guard is dropped, which the
    /// link does when it ends.
    fn register(
        &self,
        machine: &str,
        tx: tokio::sync::mpsc::Sender<pb::HostToMachine>,
    ) -> Box<dyn std::any::Any + Send>;
    fn refused(&self, machine: &str, refused: pb::InboxAnswerRefused);
}

/// Inbox port (decoupled from `humatad::inbox::Inbox`).
#[tonic::async_trait]
pub trait PeerInboxPort: Send + Sync {
    async fn replay(&self, machine: &str, entry: &pb::JournalEntry) -> Replayed;
    async fn replayed_through(&self, machine: &str) -> u64;
    fn relays(&self) -> Arc<dyn PeerRelaysPort>;
}

/// Review service port (decoupled from `humatad::review::Review`).
#[tonic::async_trait]
pub trait PeerReviewPort: Send + Sync {
    async fn replay_offered(&self, machine: &str, entry: &pb::JournalEntry) -> Replayed;
}

/// Replay outcome for a journal entry (matches `humatad::board_service::Replayed`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Replayed {
    Held,
    Applied,
    Rejected,
}

/// Upgrade watch port (decoupled from `humatad::upgrade::items::Watch`).
#[tonic::async_trait]
pub trait PeerUpgradeWatchPort: Send + Sync {
    async fn holders(
        &self,
        machine: &str,
        report: pb::HoldersReport,
        first: bool,
        channel: &tonic::transport::Channel,
    );
    async fn linked(&self, machine: &str, build: Option<&pb::BuildInfo>);
    async fn unsupported(&self, machine: &str);
}

/// Event feed port (decoupled from `humatad::events::Feed`).
pub trait PeerFeedPort: Send + Sync {
    fn subscribe(&self, after: Option<u64>) -> (u64, broadcast::Receiver<pb::Event>);
}

/// Published event wrapper (decoupled from `humatad::events::Published`).
#[derive(Clone, Debug)]
pub struct Published(pub pb::Event);

/// Resolved local target for a [`Placed`] file.
#[derive(Clone, Debug)]
pub struct PlacedTarget {
    pub path: PathBuf,
}

impl PlacedTarget {
    pub fn unfollowed(self) -> Result<PathBuf, Status> {
        Ok(self.path)
    }
}

/// Placed file reference (decoupled from `humatad::files::Placed`).
#[derive(Clone, Debug)]
pub struct Placed {
    pub machine: String,
    pub location: pb::FileLocation,
    pub relative: String,
    pub path: PathBuf,
}

impl Placed {
    pub fn machine(&self) -> &str {
        &self.machine
    }

    pub fn into_target(self) -> PlacedTarget {
        PlacedTarget { path: self.path }
    }
}

/// Operation progress handle (decoupled from `humatad::operations::Progress`).
#[derive(Clone)]
pub struct Progress {
    inner: Arc<dyn ProgressPort>,
}

pub trait ProgressPort: Send + Sync {
    fn id(&self) -> String;
    fn operation(&self) -> pb::Operation;
    fn set_step(&self, key: &str, state: pb::StepState, output_tail: Option<String>);
    fn set_units(&self, _key: &str, _done: u64, _total: u64) {}
    fn succeed(&self, result: Option<pb::operation::Result>);
    fn fail(&self, error: pb::Error);
    fn end_cancelled(&self);
}

/// Reports a step's progress in units (decoupled from `humatad::operations::Units`).
#[derive(Clone)]
pub struct Units {
    inner: Arc<dyn ProgressPort>,
}

impl Units {
    pub fn set(&self, key: &str, done: u64, total: u64) {
        self.inner.set_units(key, done, total);
    }
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
    pub fn set_step(&self, key: &str, state: pb::StepState, output_tail: Option<String>) {
        self.inner.set_step(key, state, output_tail);
    }
    pub fn units(&self) -> Units {
        Units {
            inner: self.inner.clone(),
        }
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
}

/// Result of starting an idempotent operation.
pub enum Started {
    New(Progress),
    Existing(pb::Operation),
}

/// Operations tracker port (decoupled from `humatad::operations::Operations`).
pub trait PeerOperationsPort: Send + Sync {
    fn existing(&self, key: &str) -> Option<pb::Operation>;
    fn start_steps(
        &self,
        kind: &str,
        key: &str,
        target: &str,
        steps: &[&str],
    ) -> Result<Started, Status>;
}

/// Machine clock callback (matches `humatad::machine_service::Clock`).
pub type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

/// Machine work tracker port (decoupled from `humatad::machine_service::Work`).
pub trait MachineWorkPort: Send + Sync {}

/// Host variables reader (decoupled from `humatad::host_vars::Installed`).
pub mod host_vars {
    use std::path::Path;

    pub struct Installed;
    impl Installed {
        pub fn read(_state: &Path, _name: &str) -> Option<String> {
            None
        }
    }
    impl humata2_machines::HostVariables for Installed {
        fn value(&self, _name: &str) -> Result<Option<String>, humata2_machines::HostVariablesError> {
            Ok(None)
        }
    }
}

/// The stop folder and the host's copies of its machines' stops.
pub mod stop {
    pub use humata2_svc_stop_restart::stop::{StopFolder, StopState, boot};

    pub mod remote {
        pub use humata2_svc_stop_restart::stop::remote::HostStops;
    }
}

/// Session service peer hooks (decoupled from `humatad::session_service`).
pub mod session_hooks {
    use tonic::Status;

    pub fn use_peer_upgrade<T>(_watch: T) {}

    pub fn peer_failed(machine: &str, status: Status) -> Status {
        Status::new(status.code(), format!("{machine}: {}", status.message()))
    }
}

/// Extract caller from gRPC request extensions.
pub fn caller_of<T>(request: &Request<T>) -> Result<Caller, Status> {
    request
        .extensions()
        .get::<Caller>()
        .cloned()
        .ok_or_else(|| Status::unauthenticated("missing caller identity"))
}

/// Map [`StoreError`] to gRPC [`Status`].
pub fn store_error(err: StoreError) -> Status {
    internal("store error", &err.to_string())
}
