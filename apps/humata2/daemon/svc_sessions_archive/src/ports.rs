//! Narrow port traits for `humata2_svc_sessions_archive` so it compiles
//! standalone without depending on `daemon/humatad`.

use humata2_daemon_ports::{Caller, PortError};
use humata2_svc_sessions_common::pb;
use tonic::Status;

/// Access-control and gate checks for archiving/forgetting sessions.
pub trait ArchiveGuardPort: Send + Sync {
    fn check_archive(&self, caller: &Caller, session: &str) -> Result<(), Status>;
}

/// Peer-relay hooks for remote session archiving and comparison.
#[tonic::async_trait]
pub trait ArchiveRelayPort: Send + Sync {
    async fn archive_on_machine(
        &self,
        machine: &str,
        req: pb::ArchiveSessionRequest,
    ) -> Result<pb::Operation, PortError>;
}

/// Holder-termination hook invoked when archiving a local session.
pub trait ArchiveHoldersPort: Send + Sync {
    fn end_session_holders(&self, session: &str) -> Result<(), String>;
}
