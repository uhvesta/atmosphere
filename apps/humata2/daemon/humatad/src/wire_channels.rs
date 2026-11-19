//! Wiring adapters connecting `humata2_svc_channels` to `humatad`'s concrete
//! types (`machine_service::Work`, `remote_work::RemoteWork`).
//!
//! `humata2_svc_channels` currently exports the extracted domain modules
//! (`gh_repos`, `move_folder`, `rename_alias`) and the narrow port traits
//! (`ports::{ChannelMachines, ChannelRemoteWork}`). Once `humatad` flips to
//! `humata2_svc_channels`, `channel_service.rs` re-exports those 3 modules
//! from `humata2_svc_channels` instead of compiling the duplicate local copies,
//! and `serve.rs` can construct the port adapters via `machines(...)` /
//! `remote_work(...)` below.

use std::sync::Arc;

use humata2_grpc::humata::v1 as pb;
use humata2_svc_channels::ports::{ChannelMachines, ChannelRemoteWork};
use tonic::Status;

pub(crate) use humata2_svc_channels::{gh_repos, move_folder, rename_alias};

/// Adapts `crate::machine_service::Work` to `humata2_svc_channels::ports::ChannelMachines`.
pub(crate) struct ChannelMachinesAdapter(pub(crate) Arc<crate::machine_service::Work>);

impl ChannelMachines for ChannelMachinesAdapter {
    fn reachability(&self, machine: &str) -> pb::Reachability {
        match self.0.machines.get(machine) {
            Ok(entry) => pb::Reachability::try_from(self.0.machine(&entry).reachability)
                .unwrap_or(pb::Reachability::Unspecified),
            Err(_) => pb::Reachability::Unspecified,
        }
    }
}

/// Adapts `crate::remote_work::RemoteWork` to `humata2_svc_channels::ports::ChannelRemoteWork`.
pub(crate) struct ChannelRemoteWorkAdapter(pub(crate) crate::remote_work::RemoteWork);

#[tonic::async_trait]
impl ChannelRemoteWork for ChannelRemoteWorkAdapter {
    async fn check_repo_on(&self, _machine: &str, _repo: &str) -> Result<bool, Status> {
        let _ = &self.0;
        Ok(false)
    }
}

/// Builds a `ChannelMachines` port from `humatad`'s `machine_service::Work`.
pub(crate) fn machines(work: Arc<crate::machine_service::Work>) -> Arc<dyn ChannelMachines> {
    Arc::new(ChannelMachinesAdapter(work))
}

/// Builds a `ChannelRemoteWork` port from `humatad`'s `remote_work::RemoteWork`.
pub(crate) fn remote_work(remote: crate::remote_work::RemoteWork) -> Arc<dyn ChannelRemoteWork> {
    Arc::new(ChannelRemoteWorkAdapter(remote))
}
