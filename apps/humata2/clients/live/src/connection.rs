//! The socket, the shared state behind every clone of [`LiveDaemon`], and
//! how a gRPC failure becomes the app's [`DaemonError`].

use std::collections::BTreeMap;
use std::error::Error as _;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex, MutexGuard};

use humata2_clients::DaemonError;
use humata2_clients::model::{Agent, MachineId};
use humata2_grpc::humata::v1 as pb;
use humata2_grpc::humata::v1::{
    channel_service_client::ChannelServiceClient, daemon_service_client::DaemonServiceClient,
    event_service_client::EventServiceClient, operation_service_client::OperationServiceClient,
    session_service_client::SessionServiceClient, terminal_service_client::TerminalServiceClient,
};
use hyper_util::rt::TokioIo;
use prost::Message;
use tokio::net::UnixStream;
use tonic::metadata::AsciiMetadataValue;
use tonic::service::Interceptor;
use tonic::service::interceptor::InterceptedService;
use tonic::transport::{Channel, Endpoint, Uri};
use tonic::{Code, Request, Status};

use crate::events::Subscriber;
use crate::map::AgentNames;
use crate::terminals::Attach;

/// Replies and requests can be large (a snapshot, a pasted image).
const MESSAGE_MAX_BYTES: usize = 64 << 20;

/// Adds the app's credential to every call. The one place it goes; no
/// credential is sent yet.
#[derive(Clone, Default)]
pub(crate) struct Credentials {
    token: Option<AsciiMetadataValue>,
}

impl Interceptor for Credentials {
    fn call(&mut self, mut request: Request<()>) -> Result<Request<()>, Status> {
        if let Some(token) = &self.token {
            request.metadata_mut().insert("authorization", token.clone());
        }
        Ok(request)
    }
}

pub(crate) type Wire = InterceptedService<Channel, Credentials>;

/// Where the daemon is and which agent names it knows.
#[derive(Clone, Debug)]
pub struct LiveOptions {
    socket: PathBuf,
    agents: AgentNames,
}

impl LiveOptions {
    /// The daemon's socket; agents map by their lowercase command name
    /// (claude, codex, copilot, opencode; Shell -> "zsh").
    #[must_use]
    pub fn new(socket: PathBuf) -> Self {
        Self {
            socket,
            agents: AgentNames::default(),
        }
    }

    /// Starts sessions of `agent` with the daemon's agent `name`, and maps
    /// that name back.
    #[must_use]
    pub fn with_agent(mut self, agent: Agent, name: impl Into<String>) -> Self {
        self.agents.set(agent, name.into());
        self
    }
}

/// Why the app couldn't reach its daemon.
#[derive(Debug, thiserror::Error)]
pub enum ConnectError {
    /// Nothing answers on the socket.
    #[error(
        "Humata's daemon isn't running at {socket}. Open Humata to start it, or run `humatad2 serve`"
    )]
    Connect {
        /// The socket.
        socket: PathBuf,
        /// Why the connection failed.
        #[source]
        source: tonic::transport::Error,
    },
    /// The daemon answered, but not the server-info question.
    #[error(
        "Humata's daemon at {socket} is running but didn't say who it is. Restart it with `humatad2 serve`; if that doesn't help, update Humata and the daemon to the same version"
    )]
    ServerInfo {
        /// The socket.
        socket: PathBuf,
        /// What the daemon answered.
        #[source]
        source: tonic::Status,
    },
    /// The daemon didn't name its machine.
    #[error(
        "Humata's daemon at {socket} didn't say which machine it runs on. Update Humata and the daemon to the same version"
    )]
    NoMachine {
        /// The socket.
        socket: PathBuf,
    },
}

#[derive(Default)]
pub(crate) struct State {
    pub(crate) subscribers: Vec<Subscriber>,
    /// Whether the one EventService subscription is running.
    pub(crate) subscribed: bool,
    /// Each session's tabs, in the app's order.
    pub(crate) tabs: BTreeMap<String, Vec<pb::Tab>>,
    /// Open attaches by (session, tab).
    pub(crate) attaches: BTreeMap<(String, String), Arc<Attach>>,
    /// Every tab the app has opened, by (session, tab index), with the
    /// size it last asked for: what is attached again once its session
    /// runs again.
    pub(crate) wanted: BTreeMap<(String, usize), Option<(u16, u16)>>,
}

pub(crate) struct Shared {
    wire: Wire,
    pub(crate) agents: AgentNames,
    /// This machine's id as the app knows it.
    pub(crate) machine: MachineId,
    pub(crate) generation: AtomicU64,
    state: Mutex<State>,
    /// The runtime it connected on. The app polls the client's futures on
    /// its own executor, so whatever needs tokio (tasks, timers) goes here.
    runtime: tokio::runtime::Handle,
}

/// The app's daemon client over one socket. Cheap to clone.
#[derive(Clone)]
pub struct LiveDaemon {
    pub(crate) shared: Arc<Shared>,
}

impl LiveDaemon {
    /// Connects to the daemon's socket and asks who it is.
    pub async fn connect(options: LiveOptions) -> Result<Self, ConnectError> {
        let LiveOptions { socket, agents } = options;
        let path = socket.clone();
        let channel = Endpoint::from_static("http://[::]:50051")
            .connect_with_connector(tower::service_fn(move |_: Uri| {
                let path = path.clone();
                async move {
                    Ok::<_, std::io::Error>(TokioIo::new(UnixStream::connect(path).await?))
                }
            }))
            .await
            .map_err(|source| ConnectError::Connect {
                socket: socket.clone(),
                source,
            })?;
        let wire = InterceptedService::new(channel, Credentials::default());
        let info = DaemonServiceClient::new(wire.clone())
            .get_server_info(pb::GetServerInfoRequest {})
            .await
            .map_err(|source| ConnectError::ServerInfo {
                socket: socket.clone(),
                source,
            })?
            .into_inner();
        let machine = info
            .machine
            .filter(|machine| !machine.value.is_empty())
            .ok_or(ConnectError::NoMachine { socket })?;
        Ok(Self {
            shared: Arc::new(Shared {
                wire,
                agents,
                machine: MachineId::new(machine.value),
                generation: AtomicU64::new(0),
                state: Mutex::new(State::default()),
                runtime: tokio::runtime::Handle::current(),
            }),
        })
    }
}

impl Shared {
    /// Runs `task` on the runtime the client connected on, whoever polls
    /// the caller.
    pub(crate) fn spawn<F>(&self, task: F) -> tokio::task::JoinHandle<F::Output>
    where
        F: std::future::Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.runtime.spawn(task)
    }

    pub(crate) fn state(&self) -> MutexGuard<'_, State> {
        match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    pub(crate) fn channels(&self) -> ChannelServiceClient<Wire> {
        ChannelServiceClient::new(self.wire.clone())
            .max_decoding_message_size(MESSAGE_MAX_BYTES)
            .max_encoding_message_size(MESSAGE_MAX_BYTES)
    }

    pub(crate) fn sessions(&self) -> SessionServiceClient<Wire> {
        SessionServiceClient::new(self.wire.clone())
            .max_decoding_message_size(MESSAGE_MAX_BYTES)
            .max_encoding_message_size(MESSAGE_MAX_BYTES)
    }

    pub(crate) fn terminal(&self) -> TerminalServiceClient<Wire> {
        TerminalServiceClient::new(self.wire.clone())
            .max_decoding_message_size(MESSAGE_MAX_BYTES)
            .max_encoding_message_size(MESSAGE_MAX_BYTES)
    }

    pub(crate) fn event_stream(&self) -> EventServiceClient<Wire> {
        EventServiceClient::new(self.wire.clone())
            .max_decoding_message_size(MESSAGE_MAX_BYTES)
            .max_encoding_message_size(MESSAGE_MAX_BYTES)
    }

    pub(crate) fn operations(&self) -> OperationServiceClient<Wire> {
        OperationServiceClient::new(self.wire.clone())
            .max_decoding_message_size(MESSAGE_MAX_BYTES)
            .max_encoding_message_size(MESSAGE_MAX_BYTES)
    }

    /// A gRPC failure as the app's error, for a call about `what`.
    pub(crate) fn error(&self, status: &Status, what: &str) -> DaemonError {
        status_error(status, what, &self.machine)
    }
}

/// A fresh idempotency key for one mutation.
pub(crate) fn fresh_key() -> Option<pb::IdempotencyKey> {
    Some(pb::IdempotencyKey {
        value: uuid::Uuid::new_v4().to_string(),
    })
}

/// The one answer for what this release of Humata doesn't do yet.
pub(crate) fn not_in_this_release(what: &str) -> DaemonError {
    DaemonError::NotServedYet {
        what: what.to_owned(),
    }
}

/// `google.rpc.Status`, the wrapper a Humata error travels in.
#[derive(Clone, PartialEq, prost::Message)]
struct RpcStatus {
    #[prost(int32, tag = "1")]
    code: i32,
    #[prost(string, tag = "2")]
    message: String,
    #[prost(message, repeated, tag = "3")]
    details: Vec<Any>,
}

/// `google.protobuf.Any`.
#[derive(Clone, PartialEq, prost::Message)]
struct Any {
    #[prost(string, tag = "1")]
    type_url: String,
    #[prost(bytes = "vec", tag = "2")]
    value: Vec<u8>,
}

/// The daemon's own words for a failure: the `humata.v1.Error` in the
/// status details, else the status message.
fn daemon_words(status: &Status) -> String {
    let error = RpcStatus::decode(status.details())
        .ok()
        .and_then(|wrapper| {
            wrapper
                .details
                .into_iter()
                .find(|any| any.type_url.ends_with("/humata.v1.Error"))
        })
        .and_then(|any| pb::Error::decode(any.value.as_slice()).ok());
    match error {
        Some(error) if !error.fix.is_empty() => format!("{} {}", error.message, error.fix),
        Some(error) => error.message,
        None => status.message().to_owned(),
    }
}

/// How a gRPC status becomes a [`DaemonError`]: NOT_FOUND is NotFound,
/// UNAVAILABLE and transport failures are Offline, the rest is Refused with
/// the daemon's words.
pub(crate) fn status_error(status: &Status, what: &str, machine: &MachineId) -> DaemonError {
    let transport = status
        .source()
        .is_some_and(|source| source.is::<tonic::transport::Error>());
    match status.code() {
        Code::NotFound => DaemonError::NotFound {
            what: what.to_owned(),
        },
        Code::Unavailable => DaemonError::Offline {
            machine: machine.clone(),
        },
        _ if transport => DaemonError::Offline {
            machine: machine.clone(),
        },
        _ => DaemonError::Refused {
            what: what.to_owned(),
            reason: daemon_words(status),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grpc_codes_map_to_daemon_errors() {
        let machine = MachineId::new("ma_1");
        assert!(matches!(
            status_error(&Status::not_found("x"), "the session", &machine),
            DaemonError::NotFound { .. }
        ));
        assert_eq!(
            status_error(&Status::unavailable("x"), "the session", &machine),
            DaemonError::Offline {
                machine: machine.clone()
            }
        );
        assert!(matches!(
            status_error(&Status::invalid_argument("bad"), "the session", &machine),
            DaemonError::Refused { .. }
        ));
    }
}
