//! Local `SessionService` crate (`humata2_svc_sessions_local`;
//! SPLIT_PHASE3_DAEMON §2.3).
//!
//! Owns local session lifecycle, turns, hooks, lineage, pointers, watch, local
//! fork, own_copy, add_repo, rename_branch, conversation_read, notes, and
//! delegates transfer/archive RPCs through `SessionTransferPort` and
//! `SessionArchivePort` in `humata2_svc_sessions_common`.

pub mod add_repo;
pub mod conversation_read;
pub mod ended;
pub mod fork;
pub mod fork_card;
pub mod hooks;
pub mod lineage;
pub mod notes;
pub mod own_copy;
pub mod pointers;
pub mod ports;
pub mod rename_branch;
pub mod script_steps;
pub mod status_since;
pub mod stop_launching;
pub mod stop_running;
pub mod tab_cap;
pub mod terminal_safety;
pub mod turns;
pub mod turns_list;
pub mod watch;

use std::pin::Pin;
use std::sync::Arc;

use humata2_daemon_ports::Caller;
use humata2_rpc_kit::{failure, not_built_status};
use humata2_svc_sessions_common::{
    SessionArchivePort, SessionCore, SessionTransferPort, clean_title, pb, session_not_found,
};
use tokio_stream::Stream;
use tonic::{Code, Request, Response, Status};

pub use humata2_svc_sessions_common::{Lineage, Loaded, Tell, now_ms, publish_family};
pub use ports::{
    SessionChannelsPort, SessionGuardPort, SessionHoldersPort, SessionOperationsPort,
};

pub type HookStream = Pin<Box<dyn Stream<Item = Result<pb::HookReply, Status>> + Send>>;

/// Local `SessionService` implementation backed by [`SessionCore`] and narrow
/// port traits, delegating transfer and archive RPCs to sibling crates.
#[derive(Clone)]
pub struct SessionLocalSvc {
    core: SessionCore,
    guard: Option<Arc<dyn SessionGuardPort>>,
    operations: Option<Arc<dyn SessionOperationsPort>>,
    channels: Option<Arc<dyn SessionChannelsPort>>,
    holders: Option<Arc<dyn SessionHoldersPort>>,
}

impl SessionLocalSvc {
    pub fn new(core: SessionCore) -> Self {
        Self {
            core,
            guard: None,
            operations: None,
            channels: None,
            holders: None,
        }
    }

    pub fn with_guard(mut self, guard: Arc<dyn SessionGuardPort>) -> Self {
        self.guard = Some(guard);
        self
    }

    pub fn with_operations(mut self, operations: Arc<dyn SessionOperationsPort>) -> Self {
        self.operations = Some(operations);
        self
    }

    pub fn with_channels(mut self, channels: Arc<dyn SessionChannelsPort>) -> Self {
        self.channels = Some(channels);
        self
    }

    pub fn with_holders(mut self, holders: Arc<dyn SessionHoldersPort>) -> Self {
        self.holders = Some(holders);
        self
    }

    pub fn core(&self) -> &SessionCore {
        &self.core
    }

    pub fn set_transfer(&self, port: Arc<dyn SessionTransferPort>) {
        let _ = self.core.transfer.set(port);
    }

    pub fn set_archive(&self, port: Arc<dyn SessionArchivePort>) {
        let _ = self.core.archive.set(port);
    }

    fn caller<T>(request: &Request<T>) -> Result<Caller, Status> {
        request.extensions().get::<Caller>().cloned().ok_or_else(|| {
            failure(
                Code::Unauthenticated,
                pb::Error {
                    message: "this call has no caller identity".to_owned(),
                    fix: "sign in with `humata login` or call from the Humata app".to_owned(),
                    retry: None,
                    kind: None,
                },
            )
        })
    }

    fn check_guard(&self, caller: &Caller, session_id: Option<&str>) -> Result<(), Status> {
        if let Some(guard) = &self.guard {
            guard.check_caller(caller, session_id)?;
        }
        Ok(())
    }

    fn transfer_port(&self) -> Result<&Arc<dyn SessionTransferPort>, Status> {
        self.core
            .transfer
            .get()
            .ok_or_else(|| not_built_status("session transfer port"))
    }

    fn archive_port(&self) -> Result<&Arc<dyn SessionArchivePort>, Status> {
        self.core
            .archive
            .get()
            .ok_or_else(|| not_built_status("session archive port"))
    }
}

#[tonic::async_trait]
impl pb::session_service_server::SessionService for SessionLocalSvc {
    type HookStream = HookStream;

    async fn start_session(
        &self,
        request: Request<pb::StartSessionRequest>,
    ) -> Result<Response<pb::StartSessionResponse>, Status> {
        let caller = Self::caller(&request)?;
        self.check_guard(&caller, None)?;
        let req = request.into_inner();
        let _title = clean_title(&req.title)?;
        let op = self
            .operations
            .as_ref()
            .map(|ops| ops.start_operation(pb::OperationKind::StartSession, req.title));
        Ok(Response::new(pb::StartSessionResponse {
            session: None,
            operation: op,
        }))
    }

    async fn preview_branch_names(
        &self,
        request: Request<pb::PreviewBranchNamesRequest>,
    ) -> Result<Response<pb::PreviewBranchNamesResponse>, Status> {
        let caller = Self::caller(&request)?;
        self.check_guard(&caller, None)?;
        Ok(Response::new(pb::PreviewBranchNamesResponse::default()))
    }

    async fn get_session(
        &self,
        request: Request<pb::GetSessionRequest>,
    ) -> Result<Response<pb::Session>, Status> {
        let caller = Self::caller(&request)?;
        let id = humata2_svc_sessions_common::archive::session_id(request.into_inner().id)?;
        self.check_guard(&caller, Some(&id))?;
        Err(session_not_found(&id))
    }

    async fn list_sessions(
        &self,
        request: Request<pb::ListSessionsRequest>,
    ) -> Result<Response<pb::ListSessionsResponse>, Status> {
        let caller = Self::caller(&request)?;
        self.check_guard(&caller, None)?;
        Ok(Response::new(pb::ListSessionsResponse::default()))
    }

    async fn rename_session(
        &self,
        request: Request<pb::RenameSessionRequest>,
    ) -> Result<Response<pb::Session>, Status> {
        let caller = Self::caller(&request)?;
        let req = request.into_inner();
        let id = humata2_svc_sessions_common::archive::session_id(req.id)?;
        let _title = clean_title(&req.title)?;
        self.check_guard(&caller, Some(&id))?;
        Err(session_not_found(&id))
    }

    async fn stop_session(
        &self,
        request: Request<pb::StopSessionRequest>,
    ) -> Result<Response<pb::Session>, Status> {
        let caller = Self::caller(&request)?;
        let id = humata2_svc_sessions_common::archive::session_id(request.into_inner().id)?;
        self.check_guard(&caller, Some(&id))?;
        if let Some(holders) = &self.holders {
            holders.stop_session_agent(&id).await?;
        }
        Err(session_not_found(&id))
    }

    async fn freeze_session(
        &self,
        request: Request<pb::FreezeSessionRequest>,
    ) -> Result<Response<pb::FreezeSessionResponse>, Status> {
        let caller = Self::caller(&request)?;
        let req = request.into_inner();
        let id = humata2_svc_sessions_common::archive::session_id(req.id)?;
        self.check_guard(&caller, Some(&id))?;
        let ran = match &self.holders {
            Some(holders) => holders.freeze_session_agent(&id, req.thaw).await?,
            None => false,
        };
        Ok(Response::new(pb::FreezeSessionResponse { ran }))
    }

    async fn resume_session(
        &self,
        request: Request<pb::ResumeSessionRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        let caller = Self::caller(&request)?;
        let id = humata2_svc_sessions_common::archive::session_id(request.into_inner().id)?;
        self.check_guard(&caller, Some(&id))?;
        Err(session_not_found(&id))
    }

    async fn list_turns(
        &self,
        request: Request<pb::ListTurnsRequest>,
    ) -> Result<Response<pb::ListTurnsResponse>, Status> {
        let caller = Self::caller(&request)?;
        self.check_guard(&caller, None)?;
        Ok(Response::new(pb::ListTurnsResponse::default()))
    }

    async fn preview_fork(
        &self,
        request: Request<pb::ForkSessionRequest>,
    ) -> Result<Response<pb::ForkPreview>, Status> {
        let caller = Self::caller(&request)?;
        self.check_guard(&caller, None)?;
        Ok(Response::new(pb::ForkPreview::default()))
    }

    async fn fork_session(
        &self,
        request: Request<pb::ForkSessionRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        let caller = Self::caller(&request)?;
        self.check_guard(&caller, None)?;
        Err(not_built_status("ForkSession"))
    }

    async fn give_own_copy(
        &self,
        request: Request<pb::GiveOwnCopyRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        let caller = Self::caller(&request)?;
        self.check_guard(&caller, None)?;
        Err(not_built_status("GiveOwnCopy"))
    }

    async fn continue_in_channel(
        &self,
        request: Request<pb::ContinueInChannelRequest>,
    ) -> Result<Response<pb::StartSessionResponse>, Status> {
        let _port = self.transfer_port()?;
        let _ = request;
        Err(not_built_status("ContinueInChannel"))
    }

    async fn move_session(
        &self,
        request: Request<pb::MoveSessionRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        let _port = self.transfer_port()?;
        let _ = request;
        Err(not_built_status("MoveSession"))
    }

    async fn plan_archive(
        &self,
        request: Request<pb::PlanArchiveRequest>,
    ) -> Result<Response<pb::ArchivePlan>, Status> {
        let _port = self.archive_port()?;
        let _ = request;
        Err(not_built_status("PlanArchive"))
    }

    async fn archive_session(
        &self,
        request: Request<pb::ArchiveSessionRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        let _port = self.archive_port()?;
        let _ = request;
        Err(not_built_status("ArchiveSession"))
    }

    async fn restore_session(
        &self,
        request: Request<pb::RestoreSessionRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        let _port = self.archive_port()?;
        let _ = request;
        Err(not_built_status("RestoreSession"))
    }

    async fn plan_delete(
        &self,
        request: Request<pb::PlanDeleteRequest>,
    ) -> Result<Response<pb::DeletePlan>, Status> {
        let _port = self.archive_port()?;
        let _ = request;
        Err(not_built_status("PlanDelete"))
    }

    async fn delete_session(
        &self,
        request: Request<pb::DeleteSessionRequest>,
    ) -> Result<Response<pb::DeleteSessionResponse>, Status> {
        let _port = self.archive_port()?;
        let _ = request;
        Err(not_built_status("DeleteSession"))
    }

    async fn plan_delete_sessions(
        &self,
        request: Request<pb::PlanDeleteSessionsRequest>,
    ) -> Result<Response<pb::DeleteSessionsPlan>, Status> {
        let _port = self.archive_port()?;
        let _ = request;
        Err(not_built_status("PlanDeleteSessions"))
    }

    async fn delete_sessions(
        &self,
        request: Request<pb::DeleteSessionsRequest>,
    ) -> Result<Response<pb::DeleteSessionsResponse>, Status> {
        let _port = self.archive_port()?;
        let _ = request;
        Err(not_built_status("DeleteSessions"))
    }

    async fn rebuild_session(
        &self,
        request: Request<pb::RebuildSessionRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        let _port = self.transfer_port()?;
        let _ = request;
        Err(not_built_status("RebuildSession"))
    }

    async fn add_repo_to_session(
        &self,
        request: Request<pb::AddRepoToSessionRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        let caller = Self::caller(&request)?;
        let req = request.into_inner();
        let id = humata2_svc_sessions_common::archive::session_id(req.id)?;
        self.check_guard(&caller, Some(&id))?;
        if let (Some(channels), Some(repo)) = (&self.channels, req.repo) {
            channels.ensure_channel_repo("", repo).await?;
        }
        Err(session_not_found(&id))
    }

    async fn plan_drop_repo(
        &self,
        request: Request<pb::PlanDropRepoRequest>,
    ) -> Result<Response<pb::DropRepoPlan>, Status> {
        let _port = self.archive_port()?;
        let _ = request;
        Err(not_built_status("PlanDropRepo"))
    }

    async fn drop_repo(
        &self,
        request: Request<pb::DropRepoRequest>,
    ) -> Result<Response<pb::Session>, Status> {
        let _port = self.archive_port()?;
        let _ = request;
        Err(not_built_status("DropRepo"))
    }

    async fn plan_push(
        &self,
        request: Request<pb::PlanPushRequest>,
    ) -> Result<Response<pb::PushPlan>, Status> {
        let _port = self.transfer_port()?;
        let _ = request;
        Err(not_built_status("PlanPush"))
    }

    async fn push_and_open_pull_requests(
        &self,
        request: Request<pb::PushAndOpenPullRequestsRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        let _port = self.transfer_port()?;
        let _ = request;
        Err(not_built_status("PushAndOpenPullRequests"))
    }

    async fn read_conversation(
        &self,
        request: Request<pb::ReadConversationRequest>,
    ) -> Result<Response<pb::ReadConversationResponse>, Status> {
        let caller = Self::caller(&request)?;
        self.check_guard(&caller, None)?;
        Ok(Response::new(pb::ReadConversationResponse::default()))
    }

    async fn send_note(
        &self,
        request: Request<pb::SendNoteRequest>,
    ) -> Result<Response<pb::SendNoteResponse>, Status> {
        let caller = Self::caller(&request)?;
        self.check_guard(&caller, None)?;
        Err(not_built_status("SendNote"))
    }

    async fn ask_session(
        &self,
        request: Request<pb::AskSessionRequest>,
    ) -> Result<Response<pb::AskSessionResponse>, Status> {
        let caller = Self::caller(&request)?;
        self.check_guard(&caller, None)?;
        Err(not_built_status("AskSession"))
    }

    async fn answer_question(
        &self,
        request: Request<pb::AnswerQuestionRequest>,
    ) -> Result<Response<pb::AnswerQuestionResponse>, Status> {
        let caller = Self::caller(&request)?;
        self.check_guard(&caller, None)?;
        Ok(Response::new(pb::AnswerQuestionResponse::default()))
    }

    async fn report_turn(
        &self,
        request: Request<pb::ReportTurnRequest>,
    ) -> Result<Response<pb::ReportTurnResponse>, Status> {
        let caller = Self::caller(&request)?;
        self.check_guard(&caller, None)?;
        Ok(Response::new(pb::ReportTurnResponse::default()))
    }

    async fn hook(
        &self,
        request: Request<pb::HookRequest>,
    ) -> Result<Response<Self::HookStream>, Status> {
        let caller = Self::caller(&request)?;
        self.check_guard(&caller, None)?;
        Err(not_built_status("Hook"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ended_hooks_notify_listeners() {
        let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let ended = ended::Ended::default();
        let h = hits.clone();
        ended.listen(move |_| {
            h.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });
        ended.tell("se_1");
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}
