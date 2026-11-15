//! DaemonService: versions, role and who a connection is.

use humata2_grpc::humata::v1 as pb;
use tonic::{Request, Response, Status};

use crate::identity::{self, Caller};

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

    /// A connection with a session token is that session's agent
    /// (BOUNDARIES.md §2). One with no credential gets no actor: it is
    /// refused everything else, and this tells it so.
    async fn who_am_i(
        &self,
        request: Request<pb::WhoAmIRequest>,
    ) -> Result<Response<pb::WhoAmIResponse>, Status> {
        let Some(caller) = identity::caller_if_any(&request)? else {
            return Ok(Response::new(pb::WhoAmIResponse {
                actor: None,
                channel: None,
                presence_possible: false,
                contained: false,
            }));
        };
        let (actor, channel) = match caller {
            Caller::User => {
                let via = match self.role {
                    pb::DaemonRole::Machine => pb::user_actor::Via::CliOnMachine,
                    _ => pb::user_actor::Via::CliOnHost,
                };
                let user = pb::actor::Actor::User(pb::UserActor {
                    via: via as i32,
                    machine: Some(self.machine()),
                });
                (user, None)
            }
            Caller::Agent(agent) => {
                let channel = pb::ChannelId {
                    value: agent.session.channel.to_string(),
                };
                (pb::actor::Actor::Agent(agent.actor()), Some(channel))
            }
        };
        Ok(Response::new(pb::WhoAmIResponse {
            actor: Some(pb::Actor { actor: Some(actor) }),
            channel,
            presence_possible: false,
            contained: false,
        }))
    }
}
