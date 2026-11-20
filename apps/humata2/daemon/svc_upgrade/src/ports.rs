//! Narrow port traits and helpers used by `humata2_svc_upgrade` so the crate
//! compiles standalone without importing sibling services from `humatad`.

use std::ffi::OsString;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use humata2_access::Refusal;
use humata2_daemon_ports::{Caller, PortError};
use humata2_grpc::humata::v1 as pb;
use humata2_machines::{Builds, Machines};
use humata2_rpc_kit::{error, failure};
use humata2_store::TabRecord;
use prost_types::Timestamp;
use tokio::sync::{broadcast, watch};
use tonic::{Code, Request, Status};

// ---------------------------------------------------------------------------
// Identity headers & request extractors
// ---------------------------------------------------------------------------

/// Metadata key carrying a session token.
pub const TOKEN_HEADER: &str = "humata-session-token";
/// Metadata key carrying a CLI login token.
pub const LOGIN_HEADER: &str = "humata-cli-login";
/// Metadata key carrying a script token.
pub const SCRIPT_HEADER: &str = "humata-script-token";

/// Request extension marker inserted by auth middleware when a call carried no
/// credential on a method open to anyone.
#[derive(Clone, Copy, Debug)]
pub struct Unauthenticated;

/// Request extension marker inserted by auth middleware when a call carried no
/// credential and came from outside every Humata session.
#[derive(Clone, Copy, Debug)]
pub struct OutsideSessions;

/// The caller of a method open to anyone: none when the call carried no
/// credential.
pub fn caller_if_any<T>(request: &Request<T>) -> Result<Option<Caller>, Status> {
    if let Some(caller) = request.extensions().get::<Caller>() {
        return Ok(Some(caller.clone()));
    }
    match request.extensions().get::<Unauthenticated>() {
        Some(_) => Ok(None),
        None => Err(Status::internal(
            "the daemon's authentication layer did not run on this call",
        )),
    }
}

/// True when the call had no credential and came from outside every Humata
/// session.
pub fn outside_every_session<T>(request: &Request<T>) -> bool {
    request.extensions().get::<OutsideSessions>().is_some()
}

/// Converts a policy [`Refusal`] into a gRPC [`Status`].
pub fn refused(refusal: &Refusal) -> Status {
    let (fix, kind) = match refusal {
        Refusal::UserOnly { .. } => (
            "ask the user to upgrade from the Humata app",
            pb::error::Kind::UserOnly(pb::UserOnly { capability: 0 }),
        ),
        _ => (
            "check your permissions and try again",
            pb::error::Kind::Denied(pb::Denied::default()),
        ),
    };
    failure(Code::PermissionDenied, error(refusal.to_string(), fix, kind))
}

/// Current wall-clock time as a protobuf [`Timestamp`].
pub fn now() -> Timestamp {
    let since = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    Timestamp {
        seconds: i64::try_from(since.as_secs()).unwrap_or(i64::MAX),
        nanos: since.subsec_nanos() as i32,
    }
}

// ---------------------------------------------------------------------------
// Narrow ports
// ---------------------------------------------------------------------------

/// Host-side machine registry, release builds, and peer link channel lookup
/// used by remote upgrade staging and RPC dispatch.
pub trait RemoteHostPort: Send + Sync {
    fn channel(&self, machine: &str) -> Result<tonic::transport::Channel, PortError>;
    fn machines(&self) -> &Machines;
    fn builds(&self) -> &(dyn Builds + Send + Sync);
}

/// Mutation closure applied to a pending Migrate [`pb::InboxRequest`].
pub type MigrateEdit = Box<dyn for<'a> FnOnce(&'a mut pb::InboxRequest) + Send>;

/// Inbox persistence and event fan-out for VIP Migrate items.
#[tonic::async_trait]
pub trait MigrateInboxPort: Send + Sync {
    fn kept_migrates(&self) -> Vec<pb::InboxRequest>;
    fn migrating(&self, approver: crate::items::Approver);
    async fn post_migrate(&self, request: pb::InboxRequest) -> Result<(), Status>;
    async fn update_migrate(
        &self,
        id: &str,
        change: MigrateEdit,
    ) -> Result<Option<pb::InboxRequest>, Status>;
    async fn settle_migrate(&self, id: &str, done: String) -> Result<bool, Status>;
    async fn withdraw_migrate(
        &self,
        id: &str,
        reason: pb::WithdrawReason,
    ) -> Result<bool, Status>;
}

/// Mints session/tab token environment variables for restarted holder processes.
pub trait HolderTokensPort: Send + Sync {
    fn env(&self, tab: &TabRecord) -> Vec<(OsString, OsString)>;
}

/// Subscribes to daemon events so the holder tracker notices session changes.
pub trait HolderEventsPort: Send + Sync {
    fn subscribe(&self) -> broadcast::Receiver<Arc<pb::Event>>;
}

/// Waits until daemon startup finishes before scanning running holders.
#[tonic::async_trait]
pub trait HolderStartedPort: Send + Sync {
    async fn wait(&self);
}

/// Reads agent turn status and subscribes to turn status changes.
pub trait HolderTurnsPort: Send + Sync {
    fn changes(&self) -> watch::Receiver<u64>;
    fn running(&self, session: &str, holder_pid: u32) -> pb::SessionStatus;
}

/// Publishes session updates and starts agent watchers after holder restarts.
#[tonic::async_trait]
pub trait HolderSessionPort: Send + Sync {
    async fn publish_session(&self, session: &str, change: pb::Change, by: Option<pb::Actor>);
    fn watch_agent(&self, session: String);
}
