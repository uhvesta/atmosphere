//! DaemonService: versions, role and who a connection is.

use std::sync::{Arc, OnceLock};

use humata2_chunk::Link;
use humata2_grpc::humata::v1 as pb;
use tonic::{Request, Response, Status};

use crate::compression::Compression;

use crate::identity::{self, Caller, UserVia};
use crate::relay_auth::RelayAuth;
use crate::status;

/// This API's version. `api_minor` grows with every addition once Humata 2
/// ships; until then the protos change in place.
pub(crate) const API_MAJOR: u32 = 1;
pub(crate) const API_MINOR: u32 = 0;

pub(crate) struct Daemon {
    machine: String,
    role: pb::DaemonRole,
    /// Says which codecs chunks sent here may use; none means raw only.
    compression: Option<Arc<Compression>>,
    /// What this start brought back, once it's done (PER-31).
    last_start: Option<Arc<OnceLock<pb::LastStart>>>,
    /// The socket's relay state (D144). None on the peer link, where
    /// BeginRelay and CompleteRelay answer "not built".
    relay: Option<Arc<RelayAuth>>,
}

impl Daemon {
    pub(crate) fn new(machine: String, role: pb::DaemonRole) -> Self {
        Self {
            machine,
            role,
            compression: None,
            last_start: None,
            relay: None,
        }
    }

    /// Tells the user what this start brought back, once `last_start` is set.
    pub(crate) fn started(self, last_start: Arc<OnceLock<pb::LastStart>>) -> Self {
        Self {
            last_start: Some(last_start),
            ..self
        }
    }

    /// Offers callers the codecs `compression`'s setting ranks (TR-11).
    pub(crate) fn offering(self, compression: Arc<Compression>) -> Self {
        Self {
            compression: Some(compression),
            ..self
        }
    }

    /// Answers BeginRelay and CompleteRelay from `relay` (D144).
    pub(crate) fn with_relay(self, relay: Arc<RelayAuth>) -> Self {
        Self {
            relay: Some(relay),
            ..self
        }
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
        request: Request<pb::GetServerInfoRequest>,
    ) -> Result<Response<pb::ServerInfo>, Status> {
        // Session and folder names are the user's to see.
        let last_start = match identity::caller_if_any(&request)? {
            Some(Caller::User(_)) => self.last_start.as_ref().and_then(|start| start.get().cloned()),
            _ => None,
        };
        Ok(Response::new(pb::ServerInfo {
            api_major: API_MAJOR,
            api_minor: API_MINOR,
            daemon_version: env!("CARGO_PKG_VERSION").to_owned(),
            role: self.role as i32,
            machine: Some(self.machine()),
            features: Vec::new(),
            chunk_codecs: self
                .compression
                .as_ref()
                .map(|compression| compression.offered(Link::Local))
                .unwrap_or_default(),
            last_start,
        }))
    }

    /// A connection with a session token is that session's agent
    /// (BOUNDARIES.md §2), and one with a CLI login from its terminal
    /// session is the user. One with no credential gets no actor: it is
    /// refused everything else, and this tells it so. The peer link gets
    /// no actor either: it is neither the user nor an agent.
    async fn who_am_i(
        &self,
        request: Request<pb::WhoAmIRequest>,
    ) -> Result<Response<pb::WhoAmIResponse>, Status> {
        let no_actor = pb::WhoAmIResponse {
            actor: None,
            channel: None,
            presence_possible: false,
            contained: false,
        };
        let Some(caller) = identity::caller_if_any(&request)? else {
            return Ok(Response::new(no_actor));
        };
        let presence_possible = matches!(caller, Caller::User(UserVia::CliLogin { .. }));
        let (actor, channel) = match caller {
            // Actor has no peer case; the Mac knows it is on the peer link.
            Caller::Peer => return Ok(Response::new(no_actor)),
            Caller::User(_) => {
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
            Caller::Script(script) => {
                let actor = identity::actor(&Caller::Script(script));
                return Ok(Response::new(pb::WhoAmIResponse {
                    actor: Some(actor),
                    channel: None,
                    presence_possible: false,
                    contained: false,
                }));
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
            presence_possible,
            contained: false,
        }))
    }

    /// D144: a fresh nonce for this connection, for the peer link to offer
    /// the host as the machine's Hello nonce. Grants nothing.
    async fn begin_relay(
        &self,
        request: Request<pb::BeginRelayRequest>,
    ) -> Result<Response<pb::BeginRelayResponse>, Status> {
        let (Some(relay), Some(conn)) = (&self.relay, identity::conn_id(&request)) else {
            return Err(status::not_built("humata.v1.DaemonService.BeginRelay"));
        };
        Ok(Response::new(pb::BeginRelayResponse {
            nonce: relay.begin(conn, std::time::Instant::now()).to_vec(),
        }))
    }

    /// D144: the host's nonce and proof as the link received them. On
    /// success this connection is the host's.
    async fn complete_relay(
        &self,
        request: Request<pb::CompleteRelayRequest>,
    ) -> Result<Response<pb::CompleteRelayResponse>, Status> {
        let (Some(relay), Some(conn)) = (&self.relay, identity::conn_id(&request)) else {
            return Err(status::not_built("humata.v1.DaemonService.CompleteRelay"));
        };
        let facts = identity::peer_facts(&request);
        let done = request.into_inner();
        relay
            .complete(
                conn,
                &facts,
                &done.host_nonce,
                &done.proof,
                std::time::Instant::now(),
            )
            .map_err(|refusal| refusal.status())?;
        Ok(Response::new(pb::CompleteRelayResponse {}))
    }
}
