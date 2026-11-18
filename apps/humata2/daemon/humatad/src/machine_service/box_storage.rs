//! MachineService on a machine's own daemon. Only the host keeps machines,
//! so most of it isn't built here; what the host relays over its link is
//! the Storage page's three calls, which act on this machine's own storage.
//! The host's relayed host-variable calls (HO-7, HO-13) act on this machine's
//! own variables the same way. The host's call arrives as the user (D139). Nobody else is answered:
//! an agent or script is refused, and a person at this machine's own CLI
//! gets what they always got, "not built" (the host's page is where
//! storage is shown).
//!
//! Its agents' machine lookups go up the Uplink to the host, which keeps
//! the machines and answers as that agent: its channel's machines, This
//! Mac among them (TP-1, D768).
//!
//! The host also relays a change of this machine's Humata folder (FC-10):
//! its plan, then the change, which may move the channels under the old
//! folder. They run here on this machine's own store and folders, as This
//! Mac's do on the host.

use std::sync::Arc;

use humata2_access::Refusal;
use humata2_grpc::humata::v1 as pb;
use tonic::{Request, Response, Status};

use crate::away::{Forwarder, Method, ask_host};
use crate::host_vars::HostVars;
use super::change_root::{self, RootPlans};
use super::root_of;
use crate::identity::{self, Caller, UserVia};
use crate::operations::{Operations, Started};
use crate::status;
use crate::storage::Storage;

pub(crate) struct BoxMachineSvc {
    storage: Arc<Storage>,
    /// This daemon's own machine id.
    machine: String,
    operations: Arc<Operations>,
    /// Root-change plans handed out here, good once.
    plans: RootPlans,
    /// The machine's end of the Uplink, for its agents' lookups (D768).
    host: Arc<Forwarder>,
    /// D784: starts a stopped shell again, as the restart doctor does.
    sessions: Arc<std::sync::OnceLock<crate::session_service::SessionSvc>>,
}

impl BoxMachineSvc {
    pub(crate) fn new(
        storage: Arc<Storage>,
        machine: String,
        operations: Arc<Operations>,
        host: Arc<Forwarder>,
    ) -> Self {
        Self { storage, machine, operations, plans: RootPlans::default(), host, sessions: Arc::default() }
    }

    /// D784: the session service, which starts a stopped shell again.
    pub(crate) fn use_sessions(&self, sessions: crate::session_service::SessionSvc) {
        let _ = self.sessions.set(sessions);
    }

    /// An agent's lookup, asked of the host (D768). Anyone else gets what
    /// a machine's daemon always answered: the machines are the host's.
    async fn ask_host<Req: prost::Message, Res: prost::Message + Default>(
        &self,
        request: Request<Req>,
        method: Method,
    ) -> Result<Response<Res>, Status> {
        match identity::caller(&request)? {
            Caller::Agent(agent) => {
                let session = agent.session.id.to_string();
                ask_host(&self.host, &session, method, request.into_inner(), None).await
            }
            Caller::Peer => Err(status::refused(&Refusal::PeerCannotCall)),
            Caller::User(_) | Caller::Script(_) => Err(status::not_built(method.row().name)),
        }
    }

    /// Runs `work` on a blocking thread, as the variables file is read and
    /// written (on disk before it returns).
    async fn on_variables<R: Send + 'static>(
        &self,
        work: impl FnOnce(&HostVars) -> Result<R, crate::host_vars::HostVarsError> + Send + 'static,
    ) -> Result<R, Status> {
        tokio::task::spawn_blocking(move || work(&HostVars::installed()?))
            .await
            .map_err(|error| status::internal(format!("the host variable work stopped: {error}"), ""))?
            .map_err(|error| error.status())
    }

    /// Only the host's relayed call is answered.
    fn host_only(caller: &Caller, method: &str) -> Result<(), Status> {
        match caller {
            Caller::User(UserVia::Host) => Ok(()),
            Caller::User(_) => Err(status::not_built(method)),
            Caller::Peer => Err(status::refused(&Refusal::PeerCannotCall)),
            Caller::Agent(_) | Caller::Script(_) => Err(status::refused(&Refusal::UserOnly { capability: None })),
        }
    }

    /// Runs `work` on a blocking thread, as `Storage` does file work.
    async fn blocking<R: Send + 'static>(
        &self,
        work: impl FnOnce(&Storage) -> Result<R, Status> + Send + 'static,
    ) -> Result<R, Status> {
        let storage = self.storage.clone();
        tokio::task::spawn_blocking(move || work(&storage))
            .await
            .map_err(|error| status::internal(format!("the storage work stopped: {error}"), ""))?
    }
}

#[tonic::async_trait]
impl pb::machine_service_server::MachineService for BoxMachineSvc {
    async fn get_storage(
        &self,
        request: Request<pb::GetStorageRequest>,
    ) -> Result<Response<pb::StorageReport>, Status> {
        let caller = identity::caller(&request)?;
        Self::host_only(&caller, "humata.v1.MachineService.GetStorage")?;
        let report = self.blocking(|storage| storage.report(Vec::new())).await?;
        Ok(Response::new(report))
    }

    async fn clear_build_cache(
        &self,
        request: Request<pb::ClearBuildCacheRequest>,
    ) -> Result<Response<pb::ClearBuildCacheResponse>, Status> {
        let caller = identity::caller(&request)?;
        Self::host_only(&caller, "humata.v1.MachineService.ClearBuildCache")?;
        let request = request.into_inner();
        let freed = self.blocking(move |storage| storage.clear_build_cache(&request)).await?;
        Ok(Response::new(freed))
    }

    async fn list_running_holders(
        &self,
        request: Request<pb::ListRunningHoldersRequest>,
    ) -> Result<Response<pb::ListRunningHoldersResponse>, Status> {
        let caller = identity::caller(&request)?;
        Self::host_only(&caller, "humata.v1.MachineService.ListRunningHolders")?;
        let holders = crate::running_holders::list(self.storage.store(), &self.storage.holders(), &self.machine)
            .await
            .map_err(|error| error.into_status(&self.machine))?;
        Ok(Response::new(pb::ListRunningHoldersResponse { holders }))
    }

    async fn stop_running_holder(
        &self,
        request: Request<pb::StopRunningHolderRequest>,
    ) -> Result<Response<pb::StopRunningHolderResponse>, Status> {
        let caller = identity::caller(&request)?;
        Self::host_only(&caller, "humata.v1.MachineService.StopRunningHolder")?;
        let tab = super::running_tab(request.get_ref().tab.as_ref())?;
        crate::running_holders::stop(self.storage.store(), &tab)
            .await
            .map_err(|error| error.into_status(&self.machine))?;
        Ok(Response::new(pb::StopRunningHolderResponse {}))
    }

    async fn resume_running_holder(
        &self,
        request: Request<pb::ResumeRunningHolderRequest>,
    ) -> Result<Response<pb::ResumeRunningHolderResponse>, Status> {
        let caller = identity::caller(&request)?;
        Self::host_only(&caller, "humata.v1.MachineService.ResumeRunningHolder")?;
        let tab = super::running_tab(request.get_ref().tab.as_ref())?;
        let Some(sessions) = self.sessions.get() else {
            return Err(status::internal(
                "this machine's daemon started without its session service, so it can't start a shell again; restart its daemon".to_owned(),
                "",
            ));
        };
        crate::running_holders::resume(self.storage.store(), sessions, &caller, &tab)
            .await
            .map_err(|error| error.into_status(&self.machine))?;
        Ok(Response::new(pb::ResumeRunningHolderResponse {}))
    }

    async fn end_running_holder(
        &self,
        request: Request<pb::EndRunningHolderRequest>,
    ) -> Result<Response<pb::EndRunningHolderResponse>, Status> {
        let caller = identity::caller(&request)?;
        Self::host_only(&caller, "humata.v1.MachineService.EndRunningHolder")?;
        let tab = super::running_tab(request.get_ref().tab.as_ref())?;
        crate::running_holders::end(self.storage.store(), &tab)
            .await
            .map_err(|error| error.into_status(&self.machine))?;
        Ok(Response::new(pb::EndRunningHolderResponse {}))
    }

    async fn adopt_running_holder(
        &self,
        request: Request<pb::AdoptRunningHolderRequest>,
    ) -> Result<Response<pb::AdoptRunningHolderResponse>, Status> {
        let caller = identity::caller(&request)?;
        Self::host_only(&caller, "humata.v1.MachineService.AdoptRunningHolder")?;
        let tab = super::running_tab(request.get_ref().tab.as_ref())?;
        let channel = request.get_ref().channel.as_ref().map(|id| id.value.clone()).unwrap_or_default();
        // The host knows the channels; this machine trusts the one it named.
        let builtin = humata2_sessions::Agents::builtin();
        let sessions = self.sessions.get();
        let (agents, home) = sessions.map_or((&builtin, std::path::Path::new("")), |sessions| sessions.agents_and_home());
        let session = crate::running_holders::adopt(
            self.storage.store(),
            &self.storage.holders(),
            agents,
            home,
            &tab,
            &channel,
            false,
        )
        .await
        .map_err(|error| error.into_status(&self.machine))?;
        if let Some(sessions) = sessions {
            sessions.adopted(&session).await;
        }
        Ok(Response::new(pb::AdoptRunningHolderResponse {
            session: Some(pb::SessionId { value: session }),
        }))
    }

    async fn trim_logs(
        &self,
        request: Request<pb::TrimLogsRequest>,
    ) -> Result<Response<pb::TrimLogsResponse>, Status> {
        let caller = identity::caller(&request)?;
        Self::host_only(&caller, "humata.v1.MachineService.TrimLogs")?;
        let request = request.into_inner();
        let freed = self.blocking(move |storage| storage.trim_logs(&request)).await?;
        Ok(Response::new(freed))
    }

    async fn list_machines(
        &self,
        request: Request<pb::ListMachinesRequest>,
    ) -> Result<Response<pb::ListMachinesResponse>, Status> {
        self.ask_host(request, Method::ListMachines).await
    }

    async fn get_machine(
        &self,
        request: Request<pb::GetMachineRequest>,
    ) -> Result<Response<pb::MachineDetail>, Status> {
        self.ask_host(request, Method::GetMachine).await
    }

    async fn add_ssh_machine(
        &self,
        _request: Request<pb::AddSshMachineRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        Err(status::not_built("humata.v1.MachineService.AddSshMachine"))
    }

    async fn reconnect_machine(
        &self,
        _request: Request<pb::ReconnectMachineRequest>,
    ) -> Result<Response<pb::Machine>, Status> {
        Err(status::not_built("humata.v1.MachineService.ReconnectMachine"))
    }

    async fn plan_remove_machine(
        &self,
        _request: Request<pb::PlanRemoveMachineRequest>,
    ) -> Result<Response<pb::RemoveMachinePlan>, Status> {
        Err(status::not_built("humata.v1.MachineService.PlanRemoveMachine"))
    }

    async fn remove_machine(
        &self,
        _request: Request<pb::RemoveMachineRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        Err(status::not_built("humata.v1.MachineService.RemoveMachine"))
    }

    async fn plan_forget_machine(
        &self,
        _request: Request<pb::PlanForgetMachineRequest>,
    ) -> Result<Response<pb::ForgetMachinePlan>, Status> {
        Err(status::not_built("humata.v1.MachineService.PlanForgetMachine"))
    }

    async fn forget_machine(
        &self,
        _request: Request<pb::ForgetMachineRequest>,
    ) -> Result<Response<pb::ForgetMachineResponse>, Status> {
        Err(status::not_built("humata.v1.MachineService.ForgetMachine"))
    }

    /// This machine's folder now, the channels under it, the sessions running
    /// in them, and the token ChangeRoot takes. The host names the machine
    /// by its own name for it, so the request's id isn't read.
    async fn plan_change_root(
        &self,
        request: Request<pb::PlanChangeRootRequest>,
    ) -> Result<Response<pb::ChangeRootPlan>, Status> {
        let caller = identity::caller(&request)?;
        Self::host_only(&caller, "humata.v1.MachineService.PlanChangeRoot")?;
        let new_root = root_of(&request.into_inner().root)?;
        let here = self.machine.clone();
        let (old_root, found) = self.blocking(move |storage| change_root::own_facts(storage, &here)).await?;
        let token = change_root::issue_plan(&self.plans, &self.machine, &new_root);
        Ok(Response::new(pb::ChangeRootPlan {
            machine: Some(super::machine_id(&self.machine)),
            old_root,
            new_root,
            channels: found.channels.into_iter().map(|value| pb::ChannelId { value }).collect(),
            running: found.running.into_iter().map(|value| pb::SessionId { value }).collect(),
            token: Some(pb::PlanToken { value: token }),
        }))
    }

    /// Makes the plan's folder the one new channels use here; with
    /// `move_channels`, the channels under the old one move into it too,
    /// refused while any of their sessions runs. Done when it answers.
    async fn change_root(
        &self,
        request: Request<pb::ChangeRootRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        let caller = identity::caller(&request)?;
        Self::host_only(&caller, "humata.v1.MachineService.ChangeRoot")?;
        let request = request.into_inner();
        let new_root = root_of(&request.root)?;
        if request.move_channels {
            let here = self.machine.clone();
            let (_, found) = self.blocking(move |storage| change_root::own_facts(storage, &here)).await?;
            if let Some(refusal) = change_root::busy_refusal(&self.machine, &found.running) {
                return Err(refusal);
            }
        }
        if let Some(operation) = self.operations.existing(&caller, request.idempotency_key.as_ref())? {
            return Ok(Response::new(operation));
        }
        let token = request
            .plan
            .map(|plan| plan.value)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| status::missing("plan"))?;
        change_root::take_plan(&self.plans, &token, &self.machine, &new_root)?;
        let progress = match change_root::start_operation(
            &self.operations,
            &caller,
            &self.machine,
            request.move_channels,
            request.idempotency_key,
        )? {
            Started::New(progress) => progress,
            Started::Existing(operation) => return Ok(Response::new(operation)),
        };
        let started = change_root::apply_here(
            progress,
            self.storage.clone(),
            self.machine.clone(),
            new_root,
            request.move_channels,
        )
        .await?;
        Ok(Response::new(started))
    }

    async fn set_vm_tier(
        &self,
        _request: Request<pb::SetVmTierRequest>,
    ) -> Result<Response<pb::Machine>, Status> {
        Err(status::not_built("humata.v1.MachineService.SetVmTier"))
    }

    async fn list_host_variables(
        &self,
        request: Request<pb::ListHostVariablesRequest>,
    ) -> Result<Response<pb::ListHostVariablesResponse>, Status> {
        let caller = identity::caller(&request)?;
        Self::host_only(&caller, "humata.v1.MachineService.ListHostVariables")?;
        let names = self.on_variables(|variables| variables.names()).await?;
        Ok(Response::new(pb::ListHostVariablesResponse { names }))
    }

    async fn set_host_variable(
        &self,
        request: Request<pb::SetHostVariableRequest>,
    ) -> Result<Response<pb::SetHostVariableResponse>, Status> {
        let caller = identity::caller(&request)?;
        Self::host_only(&caller, "humata.v1.MachineService.SetHostVariable")?;
        let request = request.into_inner();
        self.on_variables(move |variables| variables.set(&request.name, &request.value))
            .await?;
        let running = self.blocking(|storage| storage.running_agents()).await?;
        Ok(Response::new(pb::SetHostVariableResponse {
            running_sessions_keep_old_value: running,
        }))
    }

    async fn remove_host_variable(
        &self,
        request: Request<pb::RemoveHostVariableRequest>,
    ) -> Result<Response<pb::SetHostVariableResponse>, Status> {
        let caller = identity::caller(&request)?;
        Self::host_only(&caller, "humata.v1.MachineService.RemoveHostVariable")?;
        let request = request.into_inner();
        self.on_variables(move |variables| variables.remove(&request.name).map(|_| ()))
            .await?;
        let running = self.blocking(|storage| storage.running_agents()).await?;
        Ok(Response::new(pb::SetHostVariableResponse {
            running_sessions_keep_old_value: running,
        }))
    }

    async fn trust_new_host_key(
        &self,
        _request: Request<pb::TrustNewHostKeyRequest>,
    ) -> Result<Response<pb::Machine>, Status> {
        Err(status::not_built("humata.v1.MachineService.TrustNewHostKey"))
    }

    async fn stop_machine_sessions(
        &self,
        _request: Request<pb::StopMachineSessionsRequest>,
    ) -> Result<Response<pb::StopMachineSessionsResponse>, Status> {
        Err(status::not_built("humata.v1.MachineService.StopMachineSessions"))
    }

    async fn start_machine_sessions(
        &self,
        _request: Request<pb::StartMachineSessionsRequest>,
    ) -> Result<Response<pb::StartMachineSessionsResponse>, Status> {
        Err(status::not_built("humata.v1.MachineService.StartMachineSessions"))
    }

    // SEAM (MACHINESCRIPTS T3a): scripts run on the host only.

    async fn list_machine_scripts(
        &self,
        _request: Request<pb::ListMachineScriptsRequest>,
    ) -> Result<Response<pb::ListMachineScriptsResponse>, Status> {
        Err(status::not_built("humata.v1.MachineService.ListMachineScripts"))
    }

    async fn check_machine_scripts(
        &self,
        _request: Request<pb::CheckMachineScriptsRequest>,
    ) -> Result<Response<pb::CheckMachineScriptsResponse>, Status> {
        Err(status::not_built("humata.v1.MachineService.CheckMachineScripts"))
    }

    async fn run_machine_script(
        &self,
        _request: Request<pb::RunMachineScriptRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        Err(status::not_built("humata.v1.MachineService.RunMachineScript"))
    }

    async fn list_machine_script_runs(
        &self,
        _request: Request<pb::ListMachineScriptRunsRequest>,
    ) -> Result<Response<pb::ListMachineScriptRunsResponse>, Status> {
        Err(status::not_built("humata.v1.MachineService.ListMachineScriptRuns"))
    }
}
