//! ProviderService on a machine's daemon (PROVIDERCLI, D906).
//!
//! Only the host Mac keeps `providers.toml` and approves provider imports, so
//! a machine's daemon forwards `ListProviders`, `GetProvidersConfig`, and
//! `RequestProviderImport` up the Uplink to the host (for both session agents
//! and local/CLI callers on that machine). Direct host mutations stay
//! host-only.

use std::sync::Arc;

use humata2_access::Refusal;
use humata2_grpc::humata::v1 as pb;
use prost::Message;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Code, Request, Response, Status, Streaming};

use crate::away::{Forwarder, Method};
use crate::identity::{self, Caller};
use crate::status;

/// Why a machine's daemon can't answer a provider call locally.
#[derive(Debug, thiserror::Error)]
enum OnMachineError {
    #[error("the host is away, and provider calls need it; try again when the host is back")]
    HostAway,

    #[error("{method} runs on the host: this machine keeps no providers file of its own")]
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
                    "wait until the host Mac is awake and linked, then try again",
                    pb::error::Kind::HostUnreachable(pb::HostUnreachable { since: None }),
                ),
            ),
            OnMachineError::HostOnly { .. } => status::failure(
                Code::FailedPrecondition,
                pb::Error {
                    message,
                    fix: "do it on the host Mac (Settings › Machines › Providers), or send an import request with `humata provider request-import <file> --reason ...`"
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

/// A machine's ProviderService: reads and import requests forward to the host.
pub(crate) struct OnMachine {
    forwarder: Arc<Forwarder>,
}

impl OnMachine {
    pub(crate) fn new(forwarder: Arc<Forwarder>) -> Self {
        Self { forwarder }
    }

    async fn forward<Req: Message, Res: Message + Default>(
        &self,
        request: Request<Req>,
        method: Method,
        key: Option<pb::IdempotencyKey>,
    ) -> Result<Response<Res>, Status> {
        let row = method.row();
        let caller = identity::caller(&request)?;
        if matches!(caller, Caller::Peer) {
            return Err(status::refused(&Refusal::PeerCannotCall));
        }
        if !self.forwarder.forwards() {
            return Err(OnMachineError::HostAway.into());
        }
        let payload = request.into_inner().encode_to_vec();
        let bytes = match caller {
            Caller::Agent(agent) => {
                self.forwarder
                    .call(agent.session.id.as_str(), row, payload, key)
                    .await?
            }
            Caller::User(_) | Caller::Script(_) => {
                self.forwarder.call_as_user(row, payload, key).await?
            }
            Caller::Peer => unreachable!(),
        };
        Res::decode(bytes.as_slice())
            .map(Response::new)
            .map_err(|source| OnMachineError::Reply { method: row.name, source }.into())
    }

    fn host_only<T>(method: &'static str) -> Result<Response<T>, Status> {
        Err(OnMachineError::HostOnly { method }.into())
    }
}

#[tonic::async_trait]
impl pb::provider_service_server::ProviderService for OnMachine {
    type InteractiveLoginStream = ReceiverStream<Result<pb::LoginServerMessage, Status>>;

    async fn list_providers(
        &self,
        request: Request<pb::ListProvidersRequest>,
    ) -> Result<Response<pb::ListProvidersResponse>, Status> {
        self.forward(request, Method::ListProviders, None).await
    }

    async fn get_providers_config(
        &self,
        request: Request<pb::GetProvidersConfigRequest>,
    ) -> Result<Response<pb::ProvidersConfig>, Status> {
        self.forward(request, Method::GetProvidersConfig, None).await
    }

    async fn set_providers_config(
        &self,
        _request: Request<pb::SetProvidersConfigRequest>,
    ) -> Result<Response<pb::SetProvidersConfigResponse>, Status> {
        Self::host_only("/humata.v1.ProviderService/SetProvidersConfig")
    }

    async fn preview_provider_import(
        &self,
        _request: Request<pb::PreviewProviderImportRequest>,
    ) -> Result<Response<pb::ProviderImportPlan>, Status> {
        Self::host_only("/humata.v1.ProviderService/PreviewProviderImport")
    }

    async fn import_provider(
        &self,
        _request: Request<pb::ImportProviderRequest>,
    ) -> Result<Response<pb::SetProvidersConfigResponse>, Status> {
        Self::host_only("/humata.v1.ProviderService/ImportProvider")
    }

    async fn request_provider_import(
        &self,
        request: Request<pb::RequestProviderImportRequest>,
    ) -> Result<Response<pb::SetProvidersConfigResponse>, Status> {
        let key = request.get_ref().idempotency_key.clone();
        self.forward(request, Method::RequestProviderImport, key).await
    }

    async fn remove_provider(
        &self,
        _request: Request<pb::RemoveProviderRequest>,
    ) -> Result<Response<pb::SetProvidersConfigResponse>, Status> {
        Self::host_only("/humata.v1.ProviderService/RemoveProvider")
    }

    async fn create_machine(
        &self,
        _request: Request<pb::CreateMachineRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        Self::host_only("/humata.v1.ProviderService/CreateMachine")
    }

    async fn run_provider_action(
        &self,
        _request: Request<pb::RunProviderActionRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        Self::host_only("/humata.v1.ProviderService/RunProviderAction")
    }

    async fn delete_what_was_made(
        &self,
        _request: Request<pb::DeleteWhatWasMadeRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        Self::host_only("/humata.v1.ProviderService/DeleteWhatWasMade")
    }

    async fn interactive_login(
        &self,
        _request: Request<Streaming<pb::LoginClientMessage>>,
    ) -> Result<Response<Self::InteractiveLoginStream>, Status> {
        Self::host_only("/humata.v1.ProviderService/InteractiveLogin")
    }

    async fn make_unattended(
        &self,
        _request: Request<pb::MakeUnattendedRequest>,
    ) -> Result<Response<pb::Machine>, Status> {
        Self::host_only("/humata.v1.ProviderService/MakeUnattended")
    }
}
