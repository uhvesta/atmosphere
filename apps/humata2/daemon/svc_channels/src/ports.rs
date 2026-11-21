//! Narrow port traits for `humata2_svc_channels` so it compiles standalone
//! without depending on `daemon/humatad`.

use crate::pb;
use tonic::Status;

/// Machine reachability/status lookup used by channel grid and known-repo checks.
pub trait ChannelMachines: Send + Sync {
    fn reachability(&self, machine: &str) -> pb::Reachability;
}

/// Remote machine work execution hook for channel operations.
#[tonic::async_trait]
pub trait ChannelRemoteWork: Send + Sync {
    async fn check_repo_on(&self, machine: &str, repo: &str) -> Result<bool, Status>;
}
