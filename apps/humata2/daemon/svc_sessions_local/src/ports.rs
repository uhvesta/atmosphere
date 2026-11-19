//! Narrow port traits for `svc_sessions_local` to decouple from humatad internals.

use std::future::Future;
use std::pin::Pin;

use humata2_daemon_ports::Caller;
use humata2_svc_sessions_common::pb;
use tonic::Status;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Authorization and presence guard port.
pub trait SessionGuardPort: Send + Sync {
    fn check_caller(&self, caller: &Caller, session_id: Option<&str>) -> Result<(), Status>;
}

/// Long-running operation tracking port.
pub trait SessionOperationsPort: Send + Sync {
    fn start_operation(&self, kind: pb::OperationKind, title: String) -> pb::Operation;
}

/// Channel service port for adding a repo to a channel from `AddRepoToSession`.
pub trait SessionChannelsPort: Send + Sync {
    fn ensure_channel_repo<'a>(
        &'a self,
        channel_id: &'a str,
        repo: pb::RepoRef,
    ) -> BoxFuture<'a, Result<(), Status>>;
}

/// Terminal holder port for stopping, freezing, and typing into a session's agent.
pub trait SessionHoldersPort: Send + Sync {
    fn stop_session_agent<'a>(&'a self, session_id: &'a str) -> BoxFuture<'a, Result<(), Status>>;
    fn freeze_session_agent<'a>(
        &'a self,
        session_id: &'a str,
        thaw: bool,
    ) -> BoxFuture<'a, Result<bool, Status>>;
}
