//! DaemonService: versions, role and who a connection is.

use std::sync::Arc;

use humata2_chunk::Link;
use humata2_daemon_ports::{Caller, Release, UserVia, actor, user_via};
use humata2_rpc_kit::compression::Compression;
use humata2_rpc_kit::{API_MAJOR, API_MINOR};
use tonic::{Request, Response, Status, Streaming};

use crate::pb;
use crate::ports::{
    self, DaemonLinkPort, DaemonRelayAuthPort, DaemonUpgradePort, DaemonUplinksPort, UplinkStream,
};

pub struct Daemon {
    machine: String,
    role: pb::DaemonRole,
    /// Says which codecs chunks sent here may use; none means raw only.
    compression: Option<Arc<Compression>>,
    /// What this start brought back, once it's done (PER-31).
    last_start: Option<Arc<crate::restart::Started>>,
    /// The socket's relay state (D144). None on the peer link, where
    /// BeginRelay and CompleteRelay answer "not built".
    relay: Option<Arc<dyn DaemonRelayAuthPort>>,
    /// The host's link as this daemon sees it (AWAY). None on the peer
    /// link, where PeerUplink answers "not built".
    uplinks: Option<Arc<dyn DaemonUplinksPort>>,
    /// What serves RepairRestart (PER-30). None on the peer link, where it
    /// answers "not built".
    doctor: Option<Arc<crate::doctor::Doctor>>,
    /// What serves StopSessions and StartSessions (D261, D265). None on the
    /// peer link, where they answer "not built".
    stop: Option<Arc<crate::stop::service::StopSvc>>,
    /// On the peer link: the connection to this machine's own daemon, which
    /// StopSessions and StartSessions are handed to (D262). None on the
    /// socket, which answers them itself.
    link: Option<Arc<dyn DaemonLinkPort>>,
    /// What this daemon's release is, for ServerInfo and BeginRelay (UP-1,
    /// D281). None on the peer link: its ServerInfo carries no build, and
    /// Hello reports the build it relays to instead.
    build: Option<Release>,
    /// What serves Upgrade (UP-1, D281). None on the peer link, which
    /// hands it to the machine's own daemon.
    upgrade: Option<Arc<dyn DaemonUpgradePort>>,
}

impl Daemon {
    pub fn new(machine: String, role: pb::DaemonRole) -> Self {
        Self {
            machine,
            role,
            compression: None,
            last_start: None,
            relay: None,
            uplinks: None,
            doctor: None,
            stop: None,
            link: None,
            build: None,
            upgrade: None,
        }
    }

    /// Hands StopSessions and StartSessions to the machine's own daemon
    /// over `link`: the host's stop reaches a box on its peer link (D262).
    pub fn with_link(self, link: Arc<dyn DaemonLinkPort>) -> Self {
        Self {
            link: Some(link),
            ..self
        }
    }

    /// The proven connection to the machine's own daemon, for a call the
    /// peer link hands on.
    fn daemon_client(
        &self,
        what: &str,
    ) -> Result<
        pb::daemon_service_client::DaemonServiceClient<tonic::transport::Channel>,
        Status,
    > {
        let channel = self
            .link
            .as_ref()
            .and_then(|link| link.proven_channel())
            .ok_or_else(|| {
                ports::failure(
                    tonic::Code::Unavailable,
                    pb::Error {
                        message: format!(
                            "the machine's daemon doesn't answer on this link, so {what} didn't reach it"
                        ),
                        fix: "the link ends and the host dials a new one; try again once the machine is shown online".to_owned(),
                        retry: None,
                        kind: None,
                    },
                )
            })?;
        Ok(pb::daemon_service_client::DaemonServiceClient::new(channel))
    }

    /// Answers StopSessions and StartSessions from `stop` (D261, D265).
    pub fn with_stop(self, stop: crate::stop::service::StopSvc) -> Self {
        Self {
            stop: Some(Arc::new(stop)),
            ..self
        }
    }

    /// Answers RepairRestart from `doctor` (PER-30, D242).
    pub fn with_doctor(self, doctor: crate::doctor::Doctor) -> Self {
        Self {
            doctor: Some(Arc::new(doctor)),
            ..self
        }
    }

    /// Takes the peer link's PeerUplink and answers host_reachable (AWAY).
    pub fn with_uplinks(self, uplinks: Arc<dyn DaemonUplinksPort>) -> Self {
        Self {
            uplinks: Some(uplinks),
            ..self
        }
    }

    /// Tells the user what this start brought back, once `last_start` is set.
    pub fn started(self, last_start: Arc<crate::restart::Started>) -> Self {
        Self {
            last_start: Some(last_start),
            ..self
        }
    }

    /// Offers callers the codecs `compression`'s setting ranks (TR-11).
    pub fn offering(self, compression: Arc<Compression>) -> Self {
        Self {
            compression: Some(compression),
            ..self
        }
    }

    /// Reports `build` as this daemon's release (UP-1, D281).
    pub fn with_build(self, build: Release) -> Self {
        Self {
            build: Some(build),
            ..self
        }
    }

    /// Answers Upgrade from `upgrade` (UP-1, D281).
    pub fn with_upgrade(self, upgrade: Arc<dyn DaemonUpgradePort>) -> Self {
        Self {
            upgrade: Some(upgrade),
            ..self
        }
    }

    /// Answers BeginRelay and CompleteRelay from `relay` (D144).
    pub fn with_relay(self, relay: Arc<dyn DaemonRelayAuthPort>) -> Self {
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
        let last_start = match ports::caller_if_any(&request)? {
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
            host_reachable: self
                .uplinks
                .as_ref()
                .map_or(self.role != pb::DaemonRole::Machine, |uplinks| {
                    uplinks.reachable()
                }),
            build: self.build.as_ref().map(Release::info),
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
        let Some(caller) = ports::caller_if_any(&request)? else {
            return Ok(Response::new(no_actor));
        };
        let presence_possible = matches!(caller, Caller::User(UserVia::CliLogin { .. }));
        let (actor_kind, channel) = match caller {
            // Actor has no peer case; the Mac knows it is on the peer link.
            Caller::Peer => return Ok(Response::new(no_actor)),
            // Whoami says what identity resolved for this call, as the records do.
            Caller::User(via) => {
                let user = pb::actor::Actor::User(pb::UserActor {
                    via: user_via(&via) as i32,
                    machine: Some(self.machine()),
                });
                (user, None)
            }
            Caller::Script(script) => {
                let script_actor = actor(&Caller::Script(script));
                return Ok(Response::new(pb::WhoAmIResponse {
                    actor: Some(script_actor),
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
            actor: Some(pb::Actor {
                actor: Some(actor_kind),
            }),
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
        let (Some(relay), Some(conn)) = (&self.relay, ports::conn_id(&request)) else {
            return Err(ports::not_built("humata.v1.DaemonService.BeginRelay"));
        };
        Ok(Response::new(pb::BeginRelayResponse {
            nonce: relay.begin(conn, std::time::Instant::now()).to_vec(),
            build: self.build.as_ref().map(Release::info),
        }))
    }

    /// D144: the host's nonce and proof as the link received them. On
    /// success this connection is the host's.
    async fn complete_relay(
        &self,
        request: Request<pb::CompleteRelayRequest>,
    ) -> Result<Response<pb::CompleteRelayResponse>, Status> {
        let (Some(relay), Some(conn)) = (&self.relay, ports::conn_id(&request)) else {
            return Err(ports::not_built("humata.v1.DaemonService.CompleteRelay"));
        };
        let facts = ports::peer_facts(&request);
        let done = request.into_inner();
        relay.complete(
            conn,
            &facts,
            &done.host_nonce,
            &done.proof,
            std::time::Instant::now(),
        )?;
        Ok(Response::new(pb::CompleteRelayResponse {}))
    }

    /// PER-30, D242: the restart doctor's safe repairs.
    async fn repair_restart(
        &self,
        request: Request<pb::RepairRestartRequest>,
    ) -> Result<Response<pb::RepairRestartResponse>, Status> {
        let Some(doctor) = &self.doctor else {
            return Err(ports::not_built("humata.v1.DaemonService.RepairRestart"));
        };
        doctor.repair(request).await
    }

    /// D261: stops this machine's sessions and keeps them stopped.
    async fn stop_sessions(
        &self,
        request: Request<pb::StopSessionsRequest>,
    ) -> Result<Response<pb::StopSessionsResponse>, Status> {
        let Some(stop) = &self.stop else {
            if self.link.is_none() {
                return Err(ports::not_built("humata.v1.DaemonService.StopSessions"));
            }
            // A new request from the message alone: the host's credentials
            // are not the machine daemon's to see (D139).
            return self
                .daemon_client("the stop")?
                .stop_sessions(request.into_inner())
                .await;
        };
        stop.stop(request).await
    }

    /// D265: brings back what a stop ended.
    async fn start_sessions(
        &self,
        request: Request<pb::StartSessionsRequest>,
    ) -> Result<Response<pb::StartSessionsResponse>, Status> {
        let Some(stop) = &self.stop else {
            if self.link.is_none() {
                return Err(ports::not_built("humata.v1.DaemonService.StartSessions"));
            }
            return self
                .daemon_client("the start")?
                .start_sessions(request.into_inner())
                .await;
        };
        stop.start(request).await
    }

    /// UP-1, D281: swaps in the release staged in `bin` and starts it.
    async fn upgrade(
        &self,
        request: Request<pb::UpgradeRequest>,
    ) -> Result<Response<pb::UpgradeResponse>, Status> {
        let Some(upgrade) = &self.upgrade else {
            if self.link.is_none() {
                return Err(ports::not_built("humata.v1.DaemonService.Upgrade"));
            }
            return self
                .daemon_client("the upgrade")?
                .upgrade(request.into_inner())
                .await;
        };
        upgrade.upgrade(request).await
    }

    type PeerUplinkStream = UplinkStream;

    /// AWAY: held by the peer link while the host's Uplink is open; the
    /// host is reachable exactly while one is.
    async fn peer_uplink(
        &self,
        request: Request<Streaming<pb::HostToMachine>>,
    ) -> Result<Response<Self::PeerUplinkStream>, Status> {
        let Some(uplinks) = &self.uplinks else {
            return Err(ports::not_built("humata.v1.DaemonService.PeerUplink"));
        };
        let caller = ports::caller_if_any(&request)?;
        uplinks
            .open(caller.as_ref(), request.into_inner())
            .map(Response::new)
    }
}
