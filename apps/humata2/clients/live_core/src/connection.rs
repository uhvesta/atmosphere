//! The socket, the shared transport behind every live client over one
//! daemon connection, operation progress polling, and how a gRPC failure
//! becomes the app's [`DaemonError`].

use std::error::Error as _;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use humata2_clients::DaemonError;
use humata2_clients::model::{Agent, MachineId, MigrateThing, RequestId};
use humata2_clients::progress::{OpKind, OpUpdate};
use crate::humata2_grpc::humata::v1 as pb;
use crate::humata2_grpc::humata::v1::{
    daemon_service_client::DaemonServiceClient, operation_service_client::OperationServiceClient,
};
use hyper_util::rt::TokioIo;
use prost::Message;
use tokio::net::UnixStream;
use tonic::metadata::AsciiMetadataValue;
use tonic::service::Interceptor;
use tonic::service::interceptor::InterceptedService;
use tonic::transport::{Channel, Endpoint, Uri};
use tonic::{Code, Request, Status};

use crate::map::{self, AgentNames};
use crate::recorded::{Call, Observer, Recorded};

/// Replies and requests can be large (a snapshot, a pasted image).
pub const MESSAGE_MAX_BYTES: usize = 64 << 20;

const POLL: Duration = Duration::from_millis(100);

/// Adds the app's credential to every call. The one place it goes; no
/// credential is sent yet.
#[derive(Clone, Default)]
pub struct Credentials {
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

pub type Wire = InterceptedService<Recorded<Channel>, Credentials>;

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

type ProgressSink = Arc<dyn Fn(OpUpdate) + Send + Sync>;

struct ConnectionInner {
    wire: Wire,
    agents: AgentNames,
    /// This machine's id as the app knows it.
    machine: MachineId,
    /// What the daemon said about itself when the app connected.
    facts: DaemonFacts,
    /// The runtime it connected on. The app polls the client's futures on
    /// its own executor, so whatever needs tokio (tasks, timers) goes here.
    runtime: tokio::runtime::Handle,
    /// Registered by `LiveDaemon` so operations finished in any domain crate
    /// publish `Event::Operation` progress updates to subscribers.
    progress_sink: OnceLock<ProgressSink>,
}

/// Shared transport, server facts, and runtime handle over one daemon socket.
/// Cheap to clone.
#[derive(Clone)]
pub struct LiveConnection {
    inner: Arc<ConnectionInner>,
}

impl AsRef<LiveConnection> for LiveConnection {
    fn as_ref(&self) -> &LiveConnection {
        self
    }
}

impl LiveConnection {
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
        // Errors about this machine name it as the app does (D778).
        humata2_clients::machine_names::remember(&machine, crate::map::THIS_MAC);
        let facts = DaemonFacts {
            machine: machine.clone(),
            version: info.daemon_version.clone(),
            reports_build: info.build.is_some(),
            bundle: info.build.as_ref().map(|build| build.bundle.clone()).unwrap_or_default(),
        };
        Ok(Self {
            inner: Arc::new(ConnectionInner {
                wire,
                agents,
                machine,
                facts,
                runtime: tokio::runtime::Handle::current(),
                progress_sink: OnceLock::new(),
            }),
        })
    }

    /// The intercepted gRPC channel.
    #[must_use]
    pub fn wire(&self) -> Wire {
        self.inner.wire.clone()
    }

    /// Which daemon agent name each app agent runs under.
    #[must_use]
    pub fn agents(&self) -> &AgentNames {
        &self.inner.agents
    }

    /// This machine's id as the app knows it.
    #[must_use]
    pub fn machine(&self) -> &MachineId {
        &self.inner.machine
    }

    /// What the daemon said about itself when the app connected.
    #[must_use]
    pub fn facts(&self) -> &DaemonFacts {
        &self.inner.facts
    }

    /// Registers a sink notified of [`OpUpdate`] progress changes while
    /// [`Self::finish_operation`] polls a running operation.
    pub fn set_progress_sink(&self, sink: impl Fn(OpUpdate) + Send + Sync + 'static) {
        let _ = self.inner.progress_sink.set(Arc::new(sink));
    }

    /// Runs `task` on the runtime the client connected on, whoever polls
    /// the caller.
    pub fn spawn<F>(&self, task: F) -> tokio::task::JoinHandle<F::Output>
    where
        F: std::future::Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.inner.runtime.spawn(task)
    }

    /// A gRPC failure as the app's error, for a call about `what`.
    #[must_use]
    pub fn error(&self, status: &Status, what: &str) -> DaemonError {
        status_error(status, what, &self.inner.machine)
    }

    fn operations(&self) -> OperationServiceClient<Wire> {
        OperationServiceClient::new(self.inner.wire.clone())
            .max_decoding_message_size(MESSAGE_MAX_BYTES)
            .max_encoding_message_size(MESSAGE_MAX_BYTES)
    }

    /// Waits for an operation to stop running; a failed or cancelled one is
    /// refused with the daemon's words.
    pub async fn finish_operation(
        &self,
        mut operation: pb::Operation,
        what: &str,
    ) -> Result<pb::Operation, DaemonError> {
        let mut shown = None;
        let polled = self.poll_until_stopped(&mut operation, &mut shown, what).await;
        self.clear_progress(shown);
        polled?;
        if operation.state == pb::OperationState::Succeeded as i32 {
            return Ok(operation);
        }
        let reason = match operation.error {
            Some(error) => said(&error.message, &error.fix),
            None => "it didn't finish. Try again".to_owned(),
        };
        Err(DaemonError::Refused {
            what: what.to_owned(),
            reason,
        })
    }

    /// Polls an operation every [`POLL`] until it stops running, publishing
    /// its running step as it moves (PROGRESS).
    async fn poll_until_stopped(
        &self,
        operation: &mut pb::Operation,
        shown: &mut Option<OpUpdate>,
        what: &str,
    ) -> Result<(), DaemonError> {
        self.show_progress(operation, shown);
        while operation.state == pb::OperationState::Running as i32 {
            // On the client's runtime: the caller may be polled elsewhere,
            // and a tokio sleep needs the runtime from when it is made.
            let _ = self.spawn(async { tokio::time::sleep(POLL).await }).await;
            let Some(id) = operation.id.clone() else {
                break;
            };
            *operation = self
                .operations()
                .get_operation(pb::GetOperationRequest { id: Some(id) })
                .await
                .map_err(|status| self.error(&status, what))?
                .into_inner();
            self.show_progress(operation, shown);
        }
        Ok(())
    }

    /// Publishes an operation's update when it differs from the one last
    /// published (`shown`). Nothing is published for a kind no dialog
    /// follows, or for an update with no running step before any was shown.
    fn show_progress(&self, operation: &pb::Operation, shown: &mut Option<OpUpdate>) {
        let Some(update) = map::op_update(operation) else {
            return;
        };
        if update.kind == OpKind::Other
            || (shown.is_none() && update.progress.is_none())
            || shown.as_ref() == Some(&update)
        {
            return;
        }
        if let Some(sink) = self.inner.progress_sink.get() {
            sink(update.clone());
        }
        *shown = Some(update);
    }

    /// Ends the progress line a dialog draws: when the last published update
    /// still had a running step, publishes it again without one.
    fn clear_progress(&self, shown: Option<OpUpdate>) {
        if let Some(last) = shown.filter(|last| last.progress.is_some()) {
            if let Some(sink) = self.inner.progress_sink.get() {
                sink(OpUpdate {
                    progress: None,
                    ..last
                });
            }
        }
    }
}

/// The idempotency key with this `value`: the same value is the same mutation.
#[must_use]
pub fn key_of(value: String) -> Option<pb::IdempotencyKey> {
    Some(pb::IdempotencyKey { value })
}

/// A fresh idempotency key for one mutation.
#[must_use]
pub fn fresh_key() -> Option<pb::IdempotencyKey> {
    Some(pb::IdempotencyKey {
        value: uuid::Uuid::new_v4().to_string(),
    })
}

/// The one answer for what this release of Humata doesn't do yet.
#[must_use]
pub fn not_in_this_release(what: &str) -> DaemonError {
    DaemonError::NotServedYet {
        what: what.to_owned(),
    }
}

/// Refusal for a repository string not written as `owner/name`.
#[must_use]
pub fn bad_repo(text: &str) -> DaemonError {
    DaemonError::Refused {
        what: format!("The repository {text:?}"),
        reason: "write it as owner/name, such as acme/app".to_owned(),
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
#[must_use]
pub fn daemon_error(status: &Status) -> Option<pb::Error> {
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
        Some(error) => said(&error.message, &error.fix),
        None => status.message().to_owned(),
    }
}

/// What failed and what to do, as two sentences: the daemon words each as
/// a clause with no end stop, so the fix starts a new sentence instead of
/// running on from the failure.
#[must_use]
pub fn said(message: &str, fix: &str) -> String {
    let (message, fix) = (message.trim(), fix.trim());
    match (message.is_empty(), fix.is_empty()) {
        (true, true) => String::new(),
        (false, true) => message.to_owned(),
        (true, false) => fix.to_owned(),
        (false, false) => {
            let stop = if message.ends_with(['.', '!', '?', ':']) { "" } else { "." };
            let mut letters = fix.chars();
            let first: String = letters.next().map(char::to_uppercase).into_iter().flatten().collect();
            format!("{message}{stop} {first}{}", letters.as_str())
        }
    }
}

/// Whether the daemon refused because the thing changed after the revision
/// the caller named.
fn is_revision_conflict(status: &Status) -> bool {
    daemon_error(status)
        .is_some_and(|error| matches!(error.kind, Some(pb::error::Kind::RevisionConflict(_))))
}

/// Whether an UNAVAILABLE carries the daemon's own error that isn't about
/// a machine being out of reach: the daemon answered, so it isn't offline.
fn answered_unavailable(status: &Status) -> bool {
    daemon_error(status).is_some_and(|error| {
        !error.message.is_empty()
            && !matches!(
                error.kind,
                Some(pb::error::Kind::MachineOffline(_) | pb::error::Kind::HostUnreachable(_))
            )
    })
}

/// How a gRPC status becomes a [`DaemonError`]: `migration_needed` is
/// MigrationNeeded, NOT_FOUND is NotFound, UNAVAILABLE and transport
/// failures are Offline unless the daemon worded the UNAVAILABLE itself
/// (then Refused, in its words), a revision conflict is
/// Conflict, the rest is Refused with the daemon's words.
#[must_use]
pub fn status_error(status: &Status, what: &str, machine: &MachineId) -> DaemonError {
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
        // An UNAVAILABLE the daemon worded itself is its answer (an agent
        // that couldn't start, a restart not done): the machine answered,
        // so it is shown in the daemon's words. Only a bare UNAVAILABLE, the
        // link breaking, or one marked offline is the machine not answering.
        Code::Unavailable if answered_unavailable(status) => DaemonError::Refused {
            what: what.to_owned(),
            reason: daemon_words(status),
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
    fn a_failure_and_its_fix_read_as_two_sentences() {
        assert_eq!(said("it failed", "restore it"), "it failed. Restore it");
        assert_eq!(said("it failed.", "restore it"), "it failed. Restore it");
        assert_eq!(said("it failed", ""), "it failed");
        assert_eq!(said("", "restore it"), "restore it");
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

    /// A status with `code` that carries `error` as its details, as the daemon sends.
    fn worded(code: Code, error: pb::Error) -> Status {
        let wrapper = RpcStatus {
            code: code as i32,
            message: error.message.clone(),
            details: vec![Any {
                type_url: "type.googleapis.com/humata.v1.Error".into(),
                value: error.encode_to_vec(),
            }],
        };
        Status::with_details(code, error.message.clone(), wrapper.encode_to_vec().into())
    }

    #[test]
    fn an_unavailable_the_daemon_worded_is_its_answer_not_the_machine_offline() {
        let machine = MachineId::new("ma_1");
        let status = worded(
            Code::Unavailable,
            pb::Error {
                message: "couldn't start copilot".into(),
                fix: "install it".into(),
                ..Default::default()
            },
        );
        let error = status_error(&status, "Starting the session", &machine);
        let DaemonError::Refused { reason, .. } = &error else {
            panic!("expected the daemon's own words, got {error:?}");
        };
        // The fix starts its own sentence, so its first letter may be raised.
        let said = reason.to_lowercase();
        assert!(said.contains("couldn't start copilot") && said.contains("install it"), "{reason}");
    }

    #[test]
    fn an_unavailable_marked_offline_is_still_the_machine_offline() {
        let machine = MachineId::new("ma_1");
        let status = worded(
            Code::Unavailable,
            pb::Error {
                message: "devbox doesn't answer".into(),
                kind: Some(pb::error::Kind::MachineOffline(pb::MachineOffline::default())),
                ..Default::default()
            },
        );
        assert_eq!(
            status_error(&status, "Starting the session", &machine),
            DaemonError::Offline { machine }
        );
    }

    #[test]
    fn an_offline_error_names_the_machine_never_its_id() {
        let machine = MachineId::new("ma_e7c4d32c64da4587a5d6420ae2ab96a1");
        let unknown = DaemonError::Offline { machine: machine.clone() }.to_string();
        assert!(!unknown.contains("ma_e7c4"), "{unknown}");
        humata2_clients::machine_names::remember(&machine, crate::map::THIS_MAC);
        let named = DaemonError::Offline { machine }.to_string();
        assert!(named.starts_with("This Mac "), "{named}");
        assert!(!named.contains("ma_e7c4"), "{named}");
    }

    #[test]
    fn this_mac_is_named_once_the_app_knows_its_id() {
        let machine = MachineId::new("ma_this_mac_named");
        let _ = crate::map::this_machine(machine.clone());
        let needed = DaemonError::MigrationNeeded {
            thing: MigrateThing::Daemon,
            machine,
            request: None,
        }
        .to_string();
        assert!(needed.starts_with("This Mac's "), "{needed}");
        assert!(!needed.contains("ma_this_mac_named"), "{needed}");
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
