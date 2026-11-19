//! Narrow port traits and shared types for `humata2_svc_terminal` so it
//! compiles standalone without depending on `daemon/humatad`.

use std::pin::Pin;
use std::time::{SystemTime, UNIX_EPOCH};

use humata2_daemon_ports::Caller;
use humata2_grpc::humata::v1 as pb;
use prost_types::Timestamp;
use tokio::sync::watch;
use tokio_stream::Stream;
use tonic::Status;
use tonic::transport::Channel;

/// Default initial terminal columns for newly spawned/adopted holders.
pub const START_COLS: u16 = 80;
/// Default initial terminal rows for newly spawned/adopted holders.
pub const START_ROWS: u16 = 24;

/// Server-to-client stream returned by `TerminalService::attach`.
pub type AttachStream =
    Pin<Box<dyn Stream<Item = Result<pb::AttachServerMessage, Status>> + Send + 'static>>;

/// Current wall-clock time as a protobuf `Timestamp`.
#[must_use]
pub fn now() -> Timestamp {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    Timestamp {
        seconds: elapsed.as_secs().cast_signed(),
        nanos: elapsed.subsec_nanos().cast_signed(),
    }
}

/// Reachability state of a peer machine link.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkState {
    Up(u64),
    Down,
}

/// Error returned when a peer machine's relay channel is unavailable.
#[derive(Debug, thiserror::Error)]
pub enum RelayError {
    #[error("machine {machine} is offline")]
    Offline { machine: String },
    #[error("machine {machine} is unknown")]
    Unknown { machine: String },
    #[error("relay error for {machine}: {message}")]
    Other { machine: String, message: String },
}

impl From<RelayError> for Status {
    fn from(error: RelayError) -> Self {
        match error {
            RelayError::Offline { .. } => Status::unavailable(error.to_string()),
            RelayError::Unknown { .. } => Status::not_found(error.to_string()),
            RelayError::Other { .. } => Status::internal(error.to_string()),
        }
    }
}

/// Peer relay links used by `RemoteTerminal`.
pub trait RelayLinks: Send + Sync {
    fn channel(&self, machine: &str) -> Result<Channel, RelayError>;
    fn state(&self, machine: &str) -> LinkState;
    fn watch(&self, machine: &str) -> watch::Receiver<LinkState>;
}

/// Session-service hooks needed by `svc_terminal`.
#[tonic::async_trait]
pub trait TerminalSessions: Send + Sync {
    fn is_moving(&self, session: &str) -> bool;
    fn moving_changes(&self) -> watch::Receiver<u64>;
    fn paused_for_move(&self, socket: &std::path::Path, pid: u32) -> bool;
    fn refuse_input_while_moving(&self, session: &str) -> Result<(), Status>;
    async fn resume_session(&self, caller: &Caller, session: &str) -> Result<(), Status>;
}

/// Lifecycle/read-mark/holder-end hooks needed by `svc_terminal`.
pub trait TerminalLifecycle: Send + Sync {
    fn is_viewed(&self, session: &str) -> bool;
    fn tab_typed(&self, tab: &str);
    fn end_holder_at(&self, socket: &std::path::Path, owner: &str) -> Result<(), String>;
    /// Tells `session`'s followers its shells changed, so a shell tab
    /// removed here leaves their copy of the session too.
    fn shells_changed(&self, session: &str);
}
