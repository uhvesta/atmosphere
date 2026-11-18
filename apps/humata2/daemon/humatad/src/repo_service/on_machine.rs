//! RepoService on a machine's daemon (RL-12, D473, D568). The repo library
//! stays on the host and is never copied, so a machine's agents search it
//! and list it there: each call goes up the Uplink as a `ForwardedCall`,
//! and the host runs it as that agent under its own rules. While the host
//! is away the call says so and answers nothing, since a stale copy would
//! offer repos the user has since removed. Removing, forgetting and saving
//! the scope are the user's alone, on the host.

use std::sync::Arc;

use humata2_access::Refusal;
use humata2_grpc::humata::v1 as pb;
use prost::Message;
use tonic::{Code, Request, Response, Status};

use crate::away::{Forwarder, Method};
use crate::identity::{self, Caller};
use crate::status;

/// Why a machine's daemon can't answer a repo call.
#[derive(Debug, thiserror::Error)]
enum OnMachineError {
    #[error("the host is away, and repo search needs it; it answers again when the host is back")]
    HostAway,

    #[error("{method} runs on the host: this machine keeps no repo library of its own")]
    HostOnly { method: &'static str },

    #[error("the host's answer to {method} couldn't be read")]
    Reply {
        method: &'static str,
        #[source]
        source: prost::DecodeError,
    },
}

impl From<OnMachineError> for Status {
    fn from(error: OnMachineError) -> Self {
        let message = status::chain(&error);
        match error {
            OnMachineError::HostAway => status::failure(
                Code::Unavailable,
                status::error(
                    message,
                    "wait until the host Mac is awake and linked, then search again; nothing was added meanwhile",
                    pb::error::Kind::HostUnreachable(pb::HostUnreachable { since: None }),
                ),
            ),
            OnMachineError::HostOnly { .. } => status::failure(
                Code::FailedPrecondition,
                pb::Error {
                    message,
                    fix: "do it in Humata on the host Mac (Settings › Repositories), or with `humata repos` there"
                        .to_owned(),
                    retry: None,
                    kind: None,
                },
            ),
            OnMachineError::Reply { .. } => status::internal(
                format!("{message}; update Humata on this machine so both ends speak the same version"),
                "",
            ),
        }
    }
}

/// A machine's RepoService: its agents' reads go to the host.
pub(crate) struct OnMachine {
    forwarder: Arc<Forwarder>,
}

impl OnMachine {
    /// `forwarder` is the machine's end of the Uplink.
    pub(crate) fn new(forwarder: Arc<Forwarder>) -> Self {
        Self { forwarder }
    }

    /// Sends an agent's `request` to `method` on the host and answers with
    /// the host's response.
    async fn forward<Req: Message, Res: Message + Default>(
        &self,
        request: Request<Req>,
        method: Method,
    ) -> Result<Response<Res>, Status> {
        let row = method.row();
        let session = match identity::caller(&request)? {
            Caller::Agent(agent) => agent.session.id.to_string(),
            Caller::Peer => return Err(status::refused(&Refusal::PeerCannotCall)),
            Caller::User(_) | Caller::Script(_) => return Err(OnMachineError::HostOnly { method: row.name }.into()),
        };
        if !self.forwarder.forwards() {
            return Err(OnMachineError::HostAway.into());
        }
        let bytes = self
            .forwarder
            .call(&session, row, request.into_inner().encode_to_vec(), None)
            .await?;
        Res::decode(bytes.as_slice())
            .map(Response::new)
            .map_err(|source| OnMachineError::Reply { method: row.name, source }.into())
    }

    fn host_only<T>(method: &'static str) -> Result<Response<T>, Status> {
        Err(OnMachineError::HostOnly { method }.into())
    }
}

#[tonic::async_trait]
impl pb::repo_service_server::RepoService for OnMachine {
    async fn search_repos(
        &self,
        request: Request<pb::SearchReposRequest>,
    ) -> Result<Response<pb::SearchReposResponse>, Status> {
        self.forward(request, Method::SearchRepos).await
    }

    async fn list_repo_library(
        &self,
        request: Request<pb::ListRepoLibraryRequest>,
    ) -> Result<Response<pb::ListRepoLibraryResponse>, Status> {
        self.forward(request, Method::ListRepoLibrary).await
    }

    async fn list_repo_owners(
        &self,
        _request: Request<pb::ListRepoOwnersRequest>,
    ) -> Result<Response<pb::ListRepoOwnersResponse>, Status> {
        Self::host_only("/humata.v1.RepoService/ListRepoOwners")
    }

    async fn remove_from_repo_library(
        &self,
        _request: Request<pb::RemoveFromRepoLibraryRequest>,
    ) -> Result<Response<pb::ListRepoLibraryResponse>, Status> {
        Self::host_only("/humata.v1.RepoService/RemoveFromRepoLibrary")
    }

    async fn forget_repo_library(
        &self,
        _request: Request<pb::ForgetRepoLibraryRequest>,
    ) -> Result<Response<pb::ForgetRepoLibraryResponse>, Status> {
        Self::host_only("/humata.v1.RepoService/ForgetRepoLibrary")
    }

    async fn get_repo_scope(
        &self,
        _request: Request<pb::GetRepoScopeRequest>,
    ) -> Result<Response<pb::RepoScope>, Status> {
        Self::host_only("/humata.v1.RepoService/GetRepoScope")
    }

    async fn update_repo_scope(
        &self,
        _request: Request<pb::UpdateRepoScopeRequest>,
    ) -> Result<Response<pb::RepoScope>, Status> {
        Self::host_only("/humata.v1.RepoService/UpdateRepoScope")
    }
}
