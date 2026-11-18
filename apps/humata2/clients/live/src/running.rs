//! D784's running holders over MachineService: list, stop, resume and end
//! by tab. Each is relayed by the daemon to the machine it names.

use std::sync::Arc;

use humata2_clients::model::{ChannelId, MachineId, SessionId};
use humata2_clients::{DaemonError, HolderTab, RunningHolder, agent_of_kind};
use humata2_grpc::humata::v1 as pb;
use humata2_grpc::humata::v1::machine_service_client::MachineServiceClient;
use tonic::{Code, Status};

use crate::connection::{Shared, not_in_this_release};
use crate::map;

fn machine_id(machine: &MachineId) -> Option<pb::MachineId> {
    Some(pb::MachineId {
        value: machine.as_str().to_owned(),
    })
}

/// A gRPC failure as the app's error; a daemon without D784 is
/// [`DaemonError::NotServedYet`].
fn failure(shared: &Shared, status: &Status, what: &str) -> DaemonError {
    if status.code() == Code::Unimplemented {
        return not_in_this_release(what);
    }
    shared.error(status, what)
}

fn holder(found: pb::RunningHolder, machine: &MachineId) -> RunningHolder {
    let stopped = found.stopped;
    RunningHolder {
        tab: found.tab.map(|tab| tab.value).filter(|tab| !tab.is_empty()),
        session: found
            .session
            .as_ref()
            .filter(|session| !session.value.is_empty())
            .map(|session| map::session_id(Some(session))),
        title: found.title,
        channel: found
            .channel
            .as_ref()
            .filter(|channel| !channel.value.is_empty())
            .map(|channel| map::channel_id(Some(channel))),
        desk: found.desk,
        agent: agent_of_kind(&found.agent),
        pid: (!stopped && found.pid != 0).then_some(found.pid),
        folder: found.folder,
        started_unix_ms: found
            .started
            .filter(|_| !stopped)
            .map(|at| map::unix_ms(at.seconds, at.nanos)),
        attached: found.attached && !stopped,
        machine: found
            .machine
            .as_ref()
            .filter(|id| !id.value.is_empty())
            .map_or_else(|| machine.clone(), |id| map::machine_id(Some(id))),
        stopped,
        agent_tab: found.agent_tab,
    }
}

impl Shared {
    /// The daemon's tab id for `tab`.
    async fn holder_tab(self: &Arc<Self>, tab: &HolderTab) -> Result<pb::TabId, DaemonError> {
        match tab {
            HolderTab::Listed(id) => Ok(pb::TabId { value: id.clone() }),
            HolderTab::Session { session, index } => self.tab(session, *index).await?.id.ok_or_else(|| {
                DaemonError::NotFound {
                    what: format!("Tab {index} of session {session}"),
                }
            }),
        }
    }

    pub(crate) async fn running_holders(
        self: &Arc<Self>,
        machine: MachineId,
    ) -> Result<Vec<RunningHolder>, DaemonError> {
        let what = "Listing the terminals running on that machine";
        let mut client = MachineServiceClient::new(self.wire.clone());
        let reply = client
            .list_running_holders(pb::ListRunningHoldersRequest {
                machine: machine_id(&machine),
            })
            .await
            .map_err(|status| failure(self, &status, what))?;
        Ok(reply.into_inner().holders.into_iter().map(|found| holder(found, &machine)).collect())
    }

    pub(crate) async fn stop_running_holder(
        self: &Arc<Self>,
        machine: MachineId,
        tab: HolderTab,
    ) -> Result<(), DaemonError> {
        let what = "Stopping that terminal";
        let tab = self.holder_tab(&tab).await?;
        let mut client = MachineServiceClient::new(self.wire.clone());
        client
            .stop_running_holder(pb::StopRunningHolderRequest {
                machine: machine_id(&machine),
                tab: Some(tab),
            })
            .await
            .map_err(|status| failure(self, &status, what))?;
        Ok(())
    }

    pub(crate) async fn resume_running_holder(
        self: &Arc<Self>,
        machine: MachineId,
        tab: HolderTab,
    ) -> Result<(), DaemonError> {
        let what = "Resuming that terminal";
        let tab = self.holder_tab(&tab).await?;
        let mut client = MachineServiceClient::new(self.wire.clone());
        client
            .resume_running_holder(pb::ResumeRunningHolderRequest {
                machine: machine_id(&machine),
                tab: Some(tab),
            })
            .await
            .map_err(|status| failure(self, &status, what))?;
        Ok(())
    }

    pub(crate) async fn adopt_running_holder(
        self: &Arc<Self>,
        machine: MachineId,
        tab: String,
        channel: Option<ChannelId>,
    ) -> Result<SessionId, DaemonError> {
        let what = "Adopting that terminal";
        let mut client = MachineServiceClient::new(self.wire.clone());
        let reply = client
            .adopt_running_holder(pb::AdoptRunningHolderRequest {
                machine: machine_id(&machine),
                tab: Some(pb::TabId { value: tab }),
                channel: channel.map(|channel| pb::ChannelId {
                    value: channel.as_str().to_owned(),
                }),
            })
            .await
            .map_err(|status| failure(self, &status, what))?;
        let session = reply.into_inner().session;
        if session.as_ref().is_none_or(|session| session.value.is_empty()) {
            return Err(DaemonError::NotFound {
                what: "The session the adopted terminal became".to_owned(),
            });
        }
        Ok(map::session_id(session.as_ref()))
    }

    pub(crate) async fn end_running_holder(
        self: &Arc<Self>,
        machine: MachineId,
        tab: HolderTab,
    ) -> Result<(), DaemonError> {
        let what = "Ending that terminal";
        let tab = self.holder_tab(&tab).await?;
        let mut client = MachineServiceClient::new(self.wire.clone());
        client
            .end_running_holder(pb::EndRunningHolderRequest {
                machine: machine_id(&machine),
                tab: Some(tab),
            })
            .await
            .map_err(|status| failure(self, &status, what))?;
        Ok(())
    }
}
