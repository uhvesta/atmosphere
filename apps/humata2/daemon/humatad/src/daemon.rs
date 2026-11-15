//! DaemonService: versions, role and who a connection is.

use humata2_grpc::humata::v1 as pb;
use tonic::{Request, Response, Status};

/// This API's version. `api_minor` grows with every addition once Humata 2
/// ships; until then the protos change in place.
const API_MAJOR: u32 = 1;
const API_MINOR: u32 = 0;

pub(crate) struct Daemon {
    machine: String,
    role: pb::DaemonRole,
}

impl Daemon {
    pub(crate) fn new(machine: String, role: pb::DaemonRole) -> Self {
        Self { machine, role }
    }

    fn machine(&self) -> pb::MachineId {
        pb::MachineId {
            value: self.machine.clone(),
        }
    }
}

#[tonic::async_trait]
impl pb::daemon_service_server::DaemonService for Daemon {
    async fn get_server_info(
        &self,
        _request: Request<pb::GetServerInfoRequest>,
    ) -> Result<Response<pb::ServerInfo>, Status> {
        Ok(Response::new(pb::ServerInfo {
            api_major: API_MAJOR,
            api_minor: API_MINOR,
            daemon_version: env!("CARGO_PKG_VERSION").to_owned(),
            role: self.role as i32,
            machine: Some(self.machine()),
            features: Vec::new(),
        }))
    }

    /// Every connection is taken as the user on the CLI until the principal
    /// checks of BOUNDARIES.md §2 are wired in. Only this user can open the
    /// socket, but so can an agent running as them: until then, user-only
    /// calls (typing into a tab, PasteImage) aren't refused to agents.
    async fn who_am_i(
        &self,
        _request: Request<pb::WhoAmIRequest>,
    ) -> Result<Response<pb::WhoAmIResponse>, Status> {
        let via = match self.role {
            pb::DaemonRole::Machine => pb::user_actor::Via::CliOnMachine,
            _ => pb::user_actor::Via::CliOnHost,
        };
        Ok(Response::new(pb::WhoAmIResponse {
            actor: Some(pb::Actor {
                actor: Some(pb::actor::Actor::User(pb::UserActor {
                    via: via as i32,
                    machine: Some(self.machine()),
                })),
            }),
            channel: None,
            presence_possible: false,
            contained: false,
        }))
    }
}
