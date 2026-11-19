//! Wiring adapter connecting `humata2_svc_board::BoardSvc` to `humatad`'s
//! concrete services (`Guard`, `Feed`, `Inbox`, `Forwarder`, `Tell`, `RepoSvc`,
//! `MachineSvc`, `Files`, `TransferSvc`).

#![cfg(feature = "svc_board")]

use std::sync::Arc;

use humata2_access::{AccessRequest, Call};
use humata2_daemon_ports::{BoardAsk, Caller, Run, UserVia};
use humata2_grpc::humata::v1 as pb;
use humata2_grpc::humata::v1::file_service_server::FileService;
use humata2_grpc::humata::v1::machine_service_server::MachineService;
use humata2_grpc::humata::v1::repo_service_server::RepoService;
use humata2_grpc::humata::v1::transfer_service_server::TransferService;
use humata2_store::Store;
use humata2_svc_board::ports::{
    AwayCall, BoardFeed, BoardForwarder, BoardGuard, BoardInbox, Checked, HostCalls, HostFiles,
    HostMachines, HostRepos, HostTransfers, Method, OwnedItem, Row, Scope, SessionTell,
};
use tonic::{Request, Response, Status};

use crate::away::{self, Forwarder, HostLink};
use crate::events::Feed;
use crate::files::Files;
use crate::guard::{self, Guard};
use crate::inbox::Inbox;
use crate::machine_service::MachineSvc;
use crate::repo_service::RepoSvc;
use crate::session_service::Tell;
use crate::status;
use crate::transfer_service::TransferSvc;

// ---------------------------------------------------------------------------
// Caller conversion (`humata2_daemon_ports::Caller` -> `crate::identity::Caller`)
// ---------------------------------------------------------------------------

fn to_local_caller(caller: &Caller) -> Caller {
    caller.clone()
}

fn adapt_request<T>(request: Request<T>) -> Request<T> {
    request
}

// ---------------------------------------------------------------------------
// Guard adapter
// ---------------------------------------------------------------------------

struct GuardAdapter(Arc<Guard>);

impl BoardGuard for GuardAdapter {
    fn check(&self, caller: &Caller, call: &Call) -> Result<Checked, Status> {
        let local = to_local_caller(caller);
        match self.0.check(&local, call)? {
            guard::Checked::Allowed => Ok(Checked::Allowed),
            guard::Checked::Ask(asked) => Ok(Checked::Ask(Box::new(asked))),
        }
    }

    fn check_away(&self, caller: &Caller, call: &Call) -> Result<AwayCall, Status> {
        let local = to_local_caller(caller);
        match self.0.check_away(&local, call)? {
            guard::AwayCall::Journal => Ok(AwayCall::Journal),
            guard::AwayCall::Asks(asked) => Ok(AwayCall::Asks(asked)),
            guard::AwayCall::LastCopy => Ok(AwayCall::LastCopy),
            guard::AwayCall::Reachable => Ok(AwayCall::Reachable),
        }
    }

    fn lists_low_risk(&self, asked: &AccessRequest) -> bool {
        self.0.lists_low_risk(asked)
    }

    fn policy_changed(&self) {
        self.0.policy_changed();
    }

    fn host_link(&self) -> String {
        self.0.machine().to_owned()
    }
}

// ---------------------------------------------------------------------------
// Event feed adapter
// ---------------------------------------------------------------------------

struct FeedAdapter(Arc<Feed>);

impl BoardFeed for FeedAdapter {
    fn publish_in(
        &self,
        change: pb::Change,
        payload: pb::event::Payload,
        scope: Scope,
        by: Option<pb::Actor>,
    ) {
        self.0.publish_in(
            change,
            payload,
            crate::events::Scope {
                channel: scope.channel,
                session: scope.session,
            },
            by,
        );
    }
}

// ---------------------------------------------------------------------------
// Inbox adapter
// ---------------------------------------------------------------------------

struct InboxAdapter(Arc<Inbox>);

#[tonic::async_trait]
impl BoardInbox for InboxAdapter {
    async fn withdraw_about_item(&self, key: &str) -> Result<(), Status> {
        self.0.withdraw_about_item(key).await
    }

    async fn defer_board(
        &self,
        caller: &Caller,
        asked: AccessRequest,
        idempotency: Option<String>,
        board: BoardAsk,
        run: Run,
    ) -> Status {
        let local = to_local_caller(caller);
        self.0
            .defer_board(&local, asked, idempotency, board, run)
            .await
    }

    async fn pending_board_asks(&self) -> Vec<pb::InboxRequest> {
        self.0.pending_board_asks().await
    }

    async fn wait_final(
        &self,
        caller: Caller,
        request: pb::WaitForAnswerRequest,
    ) -> Result<pb::WaitResponse, Status> {
        let local = to_local_caller(&caller);
        self.0.wait_final(local, request).await
    }

    async fn withdraw_as(
        &self,
        caller: Caller,
        request: pb::WithdrawRequestRequest,
    ) -> Result<pb::InboxRequest, Status> {
        let local = to_local_caller(&caller);
        self.0.withdraw_as(local, request).await
    }

    async fn approved_on(
        &self,
        machine: &str,
        id: &str,
        session: &str,
    ) -> Option<pb::InboxRequest> {
        self.0.approved_on(machine, id, session).await
    }
}

// ---------------------------------------------------------------------------
// Away / forwarder adapter
// ---------------------------------------------------------------------------

fn away_method(method: Method) -> away::Method {
    match method {
        Method::ListItems => away::Method::ListItems,
        Method::GetItem => away::Method::GetItem,
        Method::GetItemHistory => away::Method::GetItemHistory,
        Method::CreateItem => away::Method::CreateItem,
        Method::UpdateItem => away::Method::UpdateItem,
        Method::MoveItem => away::Method::MoveItem,
        Method::DeleteItem => away::Method::DeleteItem,
        Method::WaitForAnswer => away::Method::WaitForAnswer,
        Method::WithdrawRequest => away::Method::WithdrawRequest,
        Method::SearchRepos => away::Method::SearchRepos,
        Method::ListRepoLibrary => away::Method::ListRepoLibrary,
        Method::ListMachines => away::Method::ListMachines,
        Method::GetMachine => away::Method::GetMachine,
        Method::ListDirectory => away::Method::ListDirectory,
        Method::Stat => away::Method::Stat,
        Method::StartTransfer => away::Method::StartTransfer,
    }
}

struct ForwarderAdapter {
    forwarder: Arc<Forwarder>,
    link: Arc<HostLink>,
    store: Arc<Store>,
}

#[tonic::async_trait]
impl BoardForwarder for ForwarderAdapter {
    fn on_machine(&self) -> bool {
        self.forwarder.on_machine()
    }

    fn forwards(&self) -> bool {
        self.forwarder.forwards()
    }

    async fn call(
        &self,
        session: &str,
        row: &'static Row,
        request: Vec<u8>,
        key: Option<pb::IdempotencyKey>,
    ) -> Result<Vec<u8>, Status> {
        self.forwarder
            .call(session, away_method(row.method).row(), request, key)
            .await
    }

    async fn record_board_journal(
        &self,
        _store: &Store,
        _host_link: &str,
        session: &str,
        operation: &'static str,
        bytes: Vec<u8>,
        base: Option<u64>,
        approved: Option<String>,
    ) -> Result<u64, Status> {
        away::record_board_journal(&self.store, &self.link, session, operation, bytes, base, approved)
            .await
            .map_err(|err| status::internal(status::chain(&err), ""))
    }

    fn owned_item(
        &self,
        store: &Store,
        owner: &str,
        item: &str,
    ) -> Result<Option<OwnedItem>, Status> {
        let found = away::owned_item(store, owner, item)
            .map_err(|err| status::internal(status::chain(&err), ""))?;
        Ok(found.map(|owned| OwnedItem { board: owned.board }))
    }
}

// ---------------------------------------------------------------------------
// Sibling host service adapters
// ---------------------------------------------------------------------------

struct TellAdapter(Tell);

#[tonic::async_trait]
impl SessionTell for TellAdapter {
    async fn session(&self, id: &str, change: pb::Change, by: Option<pb::Actor>) {
        self.0.session(id, change, by).await;
    }
}

struct ReposAdapter(RepoSvc);

#[tonic::async_trait]
impl HostRepos for ReposAdapter {
    async fn search_repos(
        &self,
        request: Request<pb::SearchReposRequest>,
    ) -> Result<Response<pb::SearchReposResponse>, Status> {
        RepoService::search_repos(&self.0, adapt_request(request)).await
    }

    async fn list_repo_library(
        &self,
        request: Request<pb::ListRepoLibraryRequest>,
    ) -> Result<Response<pb::ListRepoLibraryResponse>, Status> {
        RepoService::list_repo_library(&self.0, adapt_request(request)).await
    }
}

struct MachinesAdapter(MachineSvc);

#[tonic::async_trait]
impl HostMachines for MachinesAdapter {
    async fn list_for_machine_agent(
        &self,
        request: Request<pb::ListMachinesRequest>,
    ) -> Result<Response<pb::ListMachinesResponse>, Status> {
        self.0.list_for_machine_agent(adapt_request(request)).await
    }

    async fn get_machine(
        &self,
        request: Request<pb::GetMachineRequest>,
    ) -> Result<Response<pb::MachineDetail>, Status> {
        MachineService::get_machine(&self.0, adapt_request(request)).await
    }
}

struct FilesAdapter(Arc<Files>);

#[tonic::async_trait]
impl HostFiles for FilesAdapter {
    async fn list_directory(
        &self,
        request: Request<pb::ListDirectoryRequest>,
    ) -> Result<Response<pb::ListDirectoryResponse>, Status> {
        FileService::list_directory(self.0.as_ref(), adapt_request(request)).await
    }

    async fn stat(
        &self,
        request: Request<pb::StatRequest>,
    ) -> Result<Response<pb::FileEntry>, Status> {
        FileService::stat(self.0.as_ref(), adapt_request(request)).await
    }
}

struct TransfersAdapter(Arc<TransferSvc>);

#[tonic::async_trait]
impl HostTransfers for TransfersAdapter {
    async fn start_transfer(
        &self,
        request: Request<pb::StartTransferRequest>,
    ) -> Result<Response<pb::Transfer>, Status> {
        TransferService::start_transfer(self.0.as_ref(), adapt_request(request)).await
    }
}

// ---------------------------------------------------------------------------
// Public constructors for `serve.rs`
// ---------------------------------------------------------------------------

/// Builds a [`humata2_svc_board::BoardSvc`] wired to `humatad`'s concrete services.
pub(crate) fn build(
    guard: Arc<Guard>,
    store: Arc<Store>,
    feed: Arc<Feed>,
    inbox: Arc<Inbox>,
    machine: String,
    tell: Option<Tell>,
    repos: Option<RepoSvc>,
    forwarder: Option<Arc<Forwarder>>,
) -> humata2_svc_board::BoardSvc {
    let link = guard.host_link();
    let mut svc = humata2_svc_board::BoardSvc::new(
        Arc::new(GuardAdapter(guard)),
        store.clone(),
        Arc::new(FeedAdapter(feed)),
        Arc::new(InboxAdapter(inbox)),
        machine,
    );
    if let Some(tell) = tell {
        svc = svc.telling(Arc::new(TellAdapter(tell)));
    }
    if let Some(repos) = repos {
        svc = svc.with_repos(Arc::new(ReposAdapter(repos)));
    }
    if let Some(forwarder) = forwarder {
        svc = svc.forwarding(Arc::new(ForwarderAdapter { forwarder, link, store }));
    }
    svc
}

/// Wraps `humatad`'s host machine, file, and transfer services as [`HostCalls`].
pub(crate) fn host_calls(
    machines: MachineSvc,
    files: Arc<Files>,
    transfers: Arc<TransferSvc>,
) -> HostCalls {
    HostCalls {
        machines: Arc::new(MachinesAdapter(machines)),
        files: Arc::new(FilesAdapter(files)),
        transfers: Arc::new(TransfersAdapter(transfers)),
    }
}
