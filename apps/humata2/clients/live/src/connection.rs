//! The socket, the shared state behind every clone of [`LiveDaemon`], and
//! how a gRPC failure becomes the app's [`DaemonError`].

use std::collections::BTreeMap;
use std::error::Error as _;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use humata2_clients::DaemonError;
use humata2_clients::model::{Agent, ChannelId, MachineId, MigrateThing, RequestId};
use humata2_grpc::humata::v1 as pb;
use humata2_grpc::humata::v1::{
    access_service_client::AccessServiceClient, agent_service_client::AgentServiceClient, board_service_client::BoardServiceClient, inbox_service_client::InboxServiceClient,
    channel_service_client::ChannelServiceClient, comment_service_client::CommentServiceClient,
    daemon_service_client::DaemonServiceClient, desk_service_client::DeskServiceClient, diff_service_client::DiffServiceClient,
    event_service_client::EventServiceClient, operation_service_client::OperationServiceClient,
    session_service_client::SessionServiceClient, settings_service_client::SettingsServiceClient,
    terminal_service_client::TerminalServiceClient,
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
use crate::recorded::{Call, Observer, Recorded};

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

pub(crate) type Wire = InterceptedService<Recorded<Channel>, Credentials>;

/// Where the daemon is and which agent names it knows.
#[derive(Clone)]
pub struct LiveOptions {
    socket: PathBuf,
    agents: AgentNames,
    observer: Option<Observer>,
}

impl std::fmt::Debug for LiveOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveOptions")
            .field("socket", &self.socket)
            .field("agents", &self.agents)
            .field("observes_calls", &self.observer.is_some())
            .finish()
    }
}

impl LiveOptions {
    /// The daemon's socket; agents map by their lowercase command name
    /// (claude, codex, copilot, opencode; Shell -> "zsh").
    #[must_use]
    pub fn new(socket: PathBuf) -> Self {
        Self {
            socket,
            agents: AgentNames::default(),
            observer: None,
        }
    }

    /// Reports every gRPC call (method, time to its headers, outcome; never
    /// bodies) to `observer`, for bug-report recordings (D159).
    #[must_use]
    pub fn observe_calls(mut self, observer: impl Fn(Call) + Send + Sync + 'static) -> Self {
        self.observer = Some(Arc::new(observer));
        self
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
    /// Whether this host's restart report has been pushed or loaded.
    pub(crate) restart_known: bool,
    /// Whether a server-info read for the restart report is in flight.
    pub(crate) restart_reading: bool,
    /// Whether a cue to read it came while a read was in flight, which may
    /// have started before the daemon published the report.
    pub(crate) restart_again: bool,
    /// The channel name of each setup (operation id), for its events.
    pub(crate) setup_names: BTreeMap<String, String>,
    /// The channel each setup (operation id) makes, so a retry can name it.
    pub(crate) setup_channels: BTreeMap<String, ChannelId>,
    /// What the Inbox last showed of each request, for answering it.
    pub(crate) inbox: crate::inbox::Seen,
    /// The last Clean up list answered, with the token CleanUpDesk checks.
    pub(crate) desk_plan: Option<crate::desk::HeldPlan>,
    /// Each moving session (by id) with the move being watched for it.
    pub(crate) moves: BTreeMap<String, crate::events::Move>,
    /// The session this client views, held open on the daemon (D392).
    pub(crate) viewing: Option<crate::read_marks::Viewing>,
}

impl State {
    /// A cue that the restart report may be there now. True when the caller
    /// should read it; a cue during a read is kept for [`Self::restart_read_ended`].
    pub(crate) fn restart_cue(&mut self) -> bool {
        if self.restart_known {
            return false;
        }
        if self.restart_reading {
            self.restart_again = true;
            return false;
        }
        self.restart_reading = true;
        true
    }

    /// A read of the report ended; `known` is whether it found the daemon's
    /// answer. True when it must be read again: it wasn't known, and a cue
    /// came while that read was in flight.
    pub(crate) fn restart_read_ended(&mut self, known: bool) -> bool {
        self.restart_known |= known;
        let again = std::mem::take(&mut self.restart_again) && !self.restart_known;
        self.restart_reading = again;
        again
    }
}

pub(crate) struct Shared {
    pub(crate) wire: Wire,
    pub(crate) agents: AgentNames,
    /// This machine's id as the app knows it.
    pub(crate) machine: MachineId,
    /// What the daemon said about itself when the app connected.
    facts: DaemonFacts,
    state: Mutex<State>,
    /// The runtime it connected on. The app polls the client's futures on
    /// its own executor, so whatever needs tokio (tasks, timers) goes here.
    runtime: tokio::runtime::Handle,
}

/// What a daemon said about itself when the app connected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DaemonFacts {
    /// The machine it runs on.
    pub machine: MachineId,
    /// Its version, as it reported it.
    pub version: String,
    /// Whether it says which build it is. An older daemon (before UPGRADE)
    /// doesn't, and can't hold a Migrate item (D374).
    pub reports_build: bool,
    /// The bundle id of its build; empty when it reports none.
    pub bundle: String,
}

/// The app's daemon client over one socket. Cheap to clone.
#[derive(Clone)]
pub struct LiveDaemon {
    pub(crate) shared: Arc<Shared>,
}

impl LiveDaemon {
    /// Connects to the daemon's socket and asks who it is.
    pub async fn connect(options: LiveOptions) -> Result<Self, ConnectError> {
        let LiveOptions {
            socket,
            agents,
            observer,
        } = options;
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
        let wire = InterceptedService::new(Recorded::new(channel, observer), Credentials::default());
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
        let machine = MachineId::new(machine.value);
        let facts = DaemonFacts {
            machine: machine.clone(),
            version: info.daemon_version.clone(),
            reports_build: info.build.is_some(),
            bundle: info.build.as_ref().map(|build| build.bundle.clone()).unwrap_or_default(),
        };
        Ok(Self {
            shared: Arc::new(Shared {
                wire,
                agents,
                machine,
                facts,
                state: Mutex::new(State::default()),
                runtime: tokio::runtime::Handle::current(),
            }),
        })
    }
}

impl LiveDaemon {
    /// What the daemon said about itself when the app connected.
    #[must_use]
    pub fn facts(&self) -> &DaemonFacts {
        &self.shared.facts
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

    pub(crate) fn daemon_info(&self) -> DaemonServiceClient<Wire> {
        DaemonServiceClient::new(self.wire.clone())
            .max_decoding_message_size(MESSAGE_MAX_BYTES)
            .max_encoding_message_size(MESSAGE_MAX_BYTES)
    }

    pub(crate) fn channels(&self) -> ChannelServiceClient<Wire> {
        ChannelServiceClient::new(self.wire.clone())
            .max_decoding_message_size(MESSAGE_MAX_BYTES)
            .max_encoding_message_size(MESSAGE_MAX_BYTES)
    }

    pub(crate) fn inbox(&self) -> InboxServiceClient<Wire> {
        InboxServiceClient::new(self.wire.clone())
            .max_decoding_message_size(MESSAGE_MAX_BYTES)
            .max_encoding_message_size(MESSAGE_MAX_BYTES)
    }

    pub(crate) fn access(&self) -> AccessServiceClient<Wire> {
        AccessServiceClient::new(self.wire.clone())
            .max_decoding_message_size(MESSAGE_MAX_BYTES)
            .max_encoding_message_size(MESSAGE_MAX_BYTES)
    }

    pub(crate) fn agents(&self) -> AgentServiceClient<Wire> {
        AgentServiceClient::new(self.wire.clone())
            .max_decoding_message_size(MESSAGE_MAX_BYTES)
            .max_encoding_message_size(MESSAGE_MAX_BYTES)
    }

    pub(crate) fn settings(&self) -> SettingsServiceClient<Wire> {
        SettingsServiceClient::new(self.wire.clone())
            .max_decoding_message_size(MESSAGE_MAX_BYTES)
            .max_encoding_message_size(MESSAGE_MAX_BYTES)
    }

    pub(crate) fn boards(&self) -> BoardServiceClient<Wire> {
        BoardServiceClient::new(self.wire.clone())
            .max_decoding_message_size(MESSAGE_MAX_BYTES)
            .max_encoding_message_size(MESSAGE_MAX_BYTES)
    }

    pub(crate) fn comments(&self) -> CommentServiceClient<Wire> {
        CommentServiceClient::new(self.wire.clone())
            .max_decoding_message_size(MESSAGE_MAX_BYTES)
            .max_encoding_message_size(MESSAGE_MAX_BYTES)
    }

    pub(crate) fn diffs(&self) -> DiffServiceClient<Wire> {
        DiffServiceClient::new(self.wire.clone())
            .max_decoding_message_size(MESSAGE_MAX_BYTES)
            .max_encoding_message_size(MESSAGE_MAX_BYTES)
    }

    pub(crate) fn sessions(&self) -> SessionServiceClient<Wire> {
        SessionServiceClient::new(self.wire.clone())
            .max_decoding_message_size(MESSAGE_MAX_BYTES)
            .max_encoding_message_size(MESSAGE_MAX_BYTES)
    }

    pub(crate) fn desk(&self) -> DeskServiceClient<Wire> {
        DeskServiceClient::new(self.wire.clone())
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

/// The idempotency key with this `value`: the same value is the same mutation.
pub(crate) fn key_of(value: String) -> Option<pb::IdempotencyKey> {
    Some(pb::IdempotencyKey { value })
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

/// The `humata.v1.Error` in a status's details, when it carries one.
pub(crate) fn daemon_error(status: &Status) -> Option<pb::Error> {
    RpcStatus::decode(status.details())
        .ok()
        .and_then(|wrapper| {
            wrapper
                .details
                .into_iter()
                .find(|any| any.type_url.ends_with("/humata.v1.Error"))
        })
        .and_then(|any| pb::Error::decode(any.value.as_slice()).ok())
}

/// A `migration_needed` error (UP-1, D281, errors.proto kind 60) as the
/// app's, naming what is behind and the Migrate item that fixes it.
fn migration_needed(error: &pb::Error, machine: &MachineId) -> Option<DaemonError> {
    let Some(pb::error::Kind::MigrationNeeded(needed)) = &error.kind else {
        return None;
    };
    let thing = match pb::MigrateThing::try_from(needed.thing).unwrap_or(pb::MigrateThing::Unspecified) {
        pb::MigrateThing::Cli => MigrateThing::Cli,
        pb::MigrateThing::Store => MigrateThing::Store,
        pb::MigrateThing::Holders => MigrateThing::Holders,
        pb::MigrateThing::HostTools => MigrateThing::HostTools,
        pb::MigrateThing::DeskFolders => MigrateThing::DeskFolders,
        pb::MigrateThing::Daemon | pb::MigrateThing::Unspecified => MigrateThing::Daemon,
    };
    Some(DaemonError::MigrationNeeded {
        thing,
        machine: needed
            .machine
            .as_ref()
            .filter(|named| !named.value.is_empty())
            .map_or_else(|| machine.clone(), |named| MachineId::new(named.value.clone())),
        request: needed
            .request
            .as_ref()
            .filter(|named| !named.value.is_empty())
            .map(|named| RequestId::new(named.value.clone())),
    })
}

/// The daemon's own words for a failure: the `humata.v1.Error` in the
/// status details, else the status message.
fn daemon_words(status: &Status) -> String {
    match daemon_error(status) {
        Some(error) if !error.fix.is_empty() => format!("{} {}", error.message, error.fix),
        Some(error) => error.message,
        None => status.message().to_owned(),
    }
}

/// Whether the daemon refused because the thing changed after the revision
/// the caller named.
fn is_revision_conflict(status: &Status) -> bool {
    daemon_error(status)
        .is_some_and(|error| matches!(error.kind, Some(pb::error::Kind::RevisionConflict(_))))
}

/// How a gRPC status becomes a [`DaemonError`]: `migration_needed` is
/// MigrationNeeded, NOT_FOUND is NotFound,
/// UNAVAILABLE and transport failures are Offline, a revision conflict is
/// Conflict, the rest is Refused with the daemon's words.
pub(crate) fn status_error(status: &Status, what: &str, machine: &MachineId) -> DaemonError {
    if let Some(needed) = daemon_error(status).and_then(|error| migration_needed(&error, machine)) {
        return needed;
    }
    if let Some(pb::error::Kind::NoDefaultBranch(missing)) = daemon_error(status).and_then(|error| error.kind) {
        let repo = missing.repo.unwrap_or_default();
        return DaemonError::NoDefaultBranch {
            repo: if repo.owner.is_empty() {
                repo.name
            } else {
                format!("{}/{}", repo.owner, repo.name)
            },
        };
    }
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
        _ if is_revision_conflict(status) => DaemonError::Conflict {
            what: what.to_owned(),
            reason: daemon_words(status),
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
    fn a_cue_during_a_read_that_found_nothing_reads_again() {
        let mut state = State::default();
        assert!(state.restart_cue());
        // An event arrives while that read is in flight; it may have begun before the daemon had the report.
        assert!(!state.restart_cue());
        assert!(state.restart_read_ended(false), "the cue that came during the read isn't lost");
        assert!(!state.restart_read_ended(false), "with no new cue, it waits for the next one");
        assert!(state.restart_cue());
    }

    #[test]
    fn a_known_report_is_never_read_again() {
        let mut state = State::default();
        assert!(state.restart_cue());
        assert!(!state.restart_cue());
        assert!(!state.restart_read_ended(true));
        assert!(!state.restart_cue());
    }

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

    #[test]
    fn a_migration_needed_error_maps_to_the_item_that_fixes_it() {
        let error = pb::Error {
            message: "behind".into(),
            fix: "migrate".into(),
            kind: Some(pb::error::Kind::MigrationNeeded(pb::MigrationNeeded {
                thing: pb::MigrateThing::Daemon as i32,
                machine: Some(pb::MachineId { value: "ma_2".into() }),
                request: Some(pb::RequestId { value: "rq_9".into() }),
                ..Default::default()
            })),
            ..Default::default()
        };
        let wrapper = RpcStatus {
            code: Code::FailedPrecondition as i32,
            message: "behind".into(),
            details: vec![Any {
                type_url: "type.googleapis.com/humata.v1.Error".into(),
                value: error.encode_to_vec(),
            }],
        };
        let status = Status::with_details(Code::FailedPrecondition, "behind", wrapper.encode_to_vec().into());
        assert_eq!(
            status_error(&status, "the call", &MachineId::new("ma_1")),
            DaemonError::MigrationNeeded {
                thing: MigrateThing::Daemon,
                machine: MachineId::new("ma_2"),
                request: Some(RequestId::new("rq_9")),
            }
        );
    }
}
