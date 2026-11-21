//! Port traits and helpers decoupling `humata2_svc_transfers` from
//! `humatad`-internal services (`Guard`, `Inbox`, `Bin`, `Links`).

use std::sync::Arc;

use humata2_access::{AccessRequest, Call};
use crate::pb;
use tokio::sync::broadcast;
use tonic::transport::Channel;
use tonic::{Code, Status};

pub use humata2_daemon_ports::{BoardAsk, Caller, Deferred, Run, UserVia, actor};
pub use humata2_rpc_kit::compression::{self, Compression};
pub use humata2_rpc_kit::{
    Answers, blocking, chain, error, failed, failure, internal, invalid_field, lock, missing,
    not_found, now_ms,
};
pub use humata2_svc_files::ports::{Checked, answer_key, caller, refused, timestamp};
pub use humata2_svc_files::{Placed, Roots, Target};

/// Whether a machine's link is up. `generation` grows with every new link, so
/// a holder of an old channel can tell it must re-open its streams (TR-4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkState {
    Down,
    Up { generation: u64 },
}

/// Why a relayed call can't be made.
#[derive(Debug, thiserror::Error)]
pub enum RelayError {
    #[error(
        "{machine} doesn't answer. Its sessions keep running; use Reconnect now on the machine's page once it is reachable"
    )]
    Offline { machine: String },
}

impl From<RelayError> for Status {
    fn from(error: RelayError) -> Self {
        let message = error.to_string();
        match error {
            RelayError::Offline { machine } => failure(
                Code::Unavailable,
                self::error(
                    message,
                    "Sessions there keep running. Use Reconnect now on the machine's page, then try again.",
                    pb::error::Kind::MachineOffline(pb::MachineOffline {
                        machine: None,
                        machine_name: machine,
                        since: None,
                    }),
                ),
            ),
        }
    }
}

/// Access-control port implemented by `humatad::guard::Guard`.
pub trait TransferGuardPort: Send + Sync + 'static {
    fn check(&self, caller: &Caller, call: &Call) -> Result<Checked, Status>;
    fn require(&self, caller: &Caller, call: &Call) -> Result<(), Status>;
}

/// Inbox port implemented by `humatad::inbox::Inbox`.
#[tonic::async_trait]
pub trait TransferInboxPort: Send + Sync + 'static {
    async fn ask(
        &self,
        caller: &Caller,
        asked: AccessRequest,
        operation: &str,
        run: Option<Run>,
        board: Option<BoardAsk>,
    ) -> Result<(), Status>;
}

/// Trash-bin port implemented by `humatad::trash_service::Bin`.
pub trait TrashBinPort: Send + Sync + 'static {
    fn get(&self) -> Result<Arc<humata2_transfers::Trash>, Status>;
}

/// The machine's end of the Uplink (D768): a copy of this machine's agent
/// that names another machine goes to the host, which holds the links and
/// runs it as that agent.
#[tonic::async_trait]
pub trait HostAskPort: Send + Sync + 'static {
    /// Sends the encoded `request` to the host's `method` (`/service/Method`)
    /// as the agent of `session`, and returns the host's encoded answer.
    async fn ask(&self, session: &str, method: &'static str, request: Vec<u8>) -> Result<Vec<u8>, Status>;
}

/// Peer-link provider port implemented by `humatad::host_relay::Links`.
pub trait LinkProvider: Send + Sync + 'static {
    /// Returns the live gRPC channel for `machine`, or `RelayError::Offline`.
    fn channel(&self, machine: &str) -> Result<Channel, RelayError>;
    /// Subscribes to link up/down state changes across all machines.
    fn watch_all(&self) -> broadcast::Receiver<(String, LinkState)>;
    /// Names of machines whose link is currently up.
    fn up_machines(&self) -> Vec<String>;
}
