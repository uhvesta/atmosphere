//! `humatad2 mcp`: the MCP server an agent starts (MCP.md), as a client of
//! this machine's daemon. Every call carries the session token from
//! `HUMATA_SESSION_TOKEN`, and the daemon decides what it may do; this
//! process only turns tools into gRPC calls and answers back.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime};

use humata2_grpc::humata::v1 as pb;
use humata2_mcp::{
    Call, Daemon, ErrorKind, Operation, Reply, RequestId, Retry, Server, SessionToken, ToolError,
    WaitEvent,
};
use hyper_util::rt::TokioIo;
use serde_json::{Map, Value, json};
use tokio::net::UnixStream;
use tokio::sync::mpsc;
use tonic::metadata::MetadataValue;
use tonic::transport::{Channel, Endpoint, Uri};
use tonic::{Code, Request, Status};

use crate::error::DaemonError;
use crate::identity::TOKEN_HEADER;
use crate::{serve, status};

#[derive(clap::Args)]
pub(crate) struct McpArgs {
    /// The daemon's socket. Default: ~/.humata2/humatad.sock.
    #[arg(long)]
    socket: Option<PathBuf>,
}

pub(crate) async fn run(args: McpArgs) -> Result<(), DaemonError> {
    let socket = match args.socket {
        Some(socket) => socket,
        None => serve::default_socket()?,
    };
    Server::new(GrpcDaemon::new(socket), SessionToken::from_env())
        .serve_stdio()
        .await
        .map_err(|source| DaemonError::Mcp { source })
}

/// The daemon on its socket. Connects on the first call, and again after
/// the daemon restarts.
struct GrpcDaemon {
    socket: PathBuf,
    channel: Channel,
    token: Mutex<Option<SessionToken>>,
}

impl GrpcDaemon {
    fn new(socket: PathBuf) -> Self {
        let path = socket.clone();
        // The address is a placeholder: the connector dials the socket.
        let channel = Endpoint::from_static("http://[::]:50051").connect_with_connector_lazy(
            tower::service_fn(move |_: Uri| {
                let path = path.clone();
                async move { Ok::<_, std::io::Error>(TokioIo::new(UnixStream::connect(path).await?)) }
            }),
        );
        Self {
            socket,
            channel,
            token: Mutex::new(None),
        }
    }

    /// `message` with the session token in its metadata, marked sensitive
    /// so nothing logs it.
    fn request<T>(&self, message: T) -> Result<Request<T>, ToolError> {
        let mut request = Request::new(message);
        if let Some(token) = lock(&self.token).as_ref() {
            let mut value = MetadataValue::try_from(token.expose()).map_err(|_| not_a_token())?;
            value.set_sensitive(true);
            request.metadata_mut().insert(TOKEN_HEADER, value);
        }
        Ok(request)
    }

    fn failed(&self, status: &Status) -> ToolError {
        from_status(status, &self.socket)
    }

    fn daemon(&self) -> pb::daemon_service_client::DaemonServiceClient<Channel> {
        pb::daemon_service_client::DaemonServiceClient::new(self.channel.clone())
    }

    fn inbox(&self) -> pb::inbox_service_client::InboxServiceClient<Channel> {
        pb::inbox_service_client::InboxServiceClient::new(self.channel.clone())
    }

    fn terminals(&self) -> pb::terminal_service_client::TerminalServiceClient<Channel> {
        pb::terminal_service_client::TerminalServiceClient::new(self.channel.clone())
    }

    /// The agent this connection is, per the daemon.
    async fn agent(&self) -> Result<(pb::AgentActor, Option<pb::ChannelId>), ToolError> {
        let request = self.request(pb::WhoAmIRequest {})?;
        let reply = self
            .daemon()
            .who_am_i(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        match reply.actor.and_then(|actor| actor.actor) {
            Some(pb::actor::Actor::Agent(agent)) => Ok((agent, reply.channel)),
            // The token went missing on the way, and the daemon took this
            // for the user: never act as them.
            _ => Err(not_a_token()),
        }
    }

    async fn whoami(&self) -> Result<Reply, ToolError> {
        let (agent, channel) = self.agent().await?;
        let result = json!({
            "session": agent.session.map(|id| id.value),
            "channel": channel.map(|id| id.value),
            "machine": agent.machine.map(|id| id.value),
            "agent": agent.agent.map(|id| id.value),
            "host_reachable": true,
            "policy_as_of": Value::Null,
        });
        Ok(applied(result, None))
    }

    async fn ask(&self, call: Call) -> Result<Reply, ToolError> {
        let choices = call
            .arguments
            .get("choices")
            .and_then(Value::as_array)
            .map(|choices| {
                choices
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let request = self.request(pb::AskUserRequest {
            idempotency_key: Some(key(&call)),
            question: text(&call.arguments, "question"),
            choices,
            wait_seconds: 0,
        })?;
        let response = self
            .inbox()
            .ask_user(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        outcome(response)
    }

    async fn session_type(&self, call: Call) -> Result<Reply, ToolError> {
        let when = match call.arguments.get("when").and_then(Value::as_str) {
            Some("turn_end") => pb::type_request::When::TurnEnds,
            _ => pb::type_request::When::Now,
        };
        let request = self.request(pb::TypeRequest {
            idempotency_key: Some(key(&call)),
            session: Some(pb::SessionId {
                value: text(&call.arguments, "session"),
            }),
            tab: None,
            text: text(&call.arguments, "text"),
            when: when as i32,
        })?;
        let typed = self
            .terminals()
            .r#type(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        let result = json!({
            "typed": typed.typed,
            "queued_id": typed.queued.map(|id| id.value),
            "turn_end_uncertain": typed.turn_end_uncertain,
        });
        Ok(applied(result, None))
    }
}

impl Daemon for GrpcDaemon {
    async fn bind(&self, token: &SessionToken) -> Result<(), ToolError> {
        *lock(&self.token) = Some(token.clone());
        self.agent().await.map(|_| ())
    }

    async fn call(&self, call: Call) -> Result<Reply, ToolError> {
        match call.operation {
            Operation::Whoami => self.whoami().await,
            Operation::InboxAsk => self.ask(call).await,
            Operation::SessionType => self.session_type(call).await,
            other => Err(not_built(other)),
        }
    }

    async fn wait(
        &self,
        request: &RequestId,
        wait: Duration,
    ) -> Result<mpsc::Receiver<WaitEvent>, ToolError> {
        let message = self.request(pb::WaitForAnswerRequest {
            id: Some(pb::RequestId {
                value: request.as_str().to_owned(),
            }),
            wait_seconds: u32::try_from(wait.as_secs()).unwrap_or(u32::MAX),
        })?;
        let mut answers = self
            .inbox()
            .wait_for_answer(message)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        let socket = self.socket.clone();
        let (sender, receiver) = mpsc::channel(4);
        tokio::spawn(async move {
            loop {
                // Dropping the stream when the server stops listening ends
                // the daemon's side of the wait too.
                let next = tokio::select! {
                    () = sender.closed() => return,
                    next = answers.message() => next,
                };
                let event = match next {
                    Ok(Some(response)) if response.keepalive => WaitEvent::Keepalive,
                    Ok(Some(response)) => WaitEvent::Done(outcome(response)),
                    // Ended without an answer: the server reports pending.
                    Ok(None) => return,
                    Err(status) => WaitEvent::Done(Err(from_status(&status, &socket))),
                };
                let done = matches!(event, WaitEvent::Done(_));
                if sender.send(event).await.is_err() || done {
                    return;
                }
            }
        });
        Ok(receiver)
    }

    async fn withdraw(&self, request: &RequestId) -> Result<(), ToolError> {
        let message = self.request(pb::WithdrawRequestRequest {
            idempotency_key: None,
            id: Some(pb::RequestId {
                value: request.as_str().to_owned(),
            }),
        })?;
        self.inbox()
            .withdraw_request(message)
            .await
            .map(|_| ())
            .map_err(|status| self.failed(&status))
    }
}

/// What the daemon's answer to an Inbox request means for the call.
fn outcome(response: pb::WaitResponse) -> Result<Reply, ToolError> {
    let id = RequestId::new(response.id.map(|id| id.value).unwrap_or_default());
    if let Some(error) = response.error {
        return Err(tool_error(Code::FailedPrecondition, error));
    }
    match pb::RequestState::try_from(response.state) {
        Ok(pb::RequestState::Pending) => Ok(Reply::Pending { request: id }),
        Ok(pb::RequestState::Answered | pb::RequestState::Approved) => {
            let result = match response.answer.and_then(|answer| answer.answer) {
                Some(pb::request_answer::Answer::Reply(reply)) => json!({ "reply": reply.text }),
                Some(_) => json!({ "approved": true }),
                None => json!({}),
            };
            Ok(applied(result, Some(id)))
        }
        _ => Err(ToolError {
            kind: ErrorKind::Internal,
            message: format!(
                "the daemon answered request {} with a state this MCP server doesn't know",
                id.as_str()
            ),
            fix: "call inbox_wait with the request id to see where it stands".to_owned(),
            retry: None,
            details: Map::new(),
        }),
    }
}

fn applied(result: Value, request: Option<RequestId>) -> Reply {
    Reply::Applied {
        result,
        request,
        stale_as_of: None,
    }
}

fn key(call: &Call) -> pb::IdempotencyKey {
    pb::IdempotencyKey {
        value: call.idempotency_key.as_str().to_owned(),
    }
}

/// A string argument; the server checked the schema already.
fn text(arguments: &Map<String, Value>, field: &str) -> String {
    arguments
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn not_built(operation: Operation) -> ToolError {
    ToolError {
        kind: ErrorKind::PeerVersionUnsupported,
        message: format!(
            "{} isn't built into this humatad2 yet",
            operation.tool_name()
        ),
        fix: "ask the user with inbox_ask to do it in the app; updating Humata on this machine brings the tool".to_owned(),
        retry: None,
        details: Map::new(),
    }
}

/// The token isn't one the daemon takes as an agent's.
fn not_a_token() -> ToolError {
    ToolError {
        kind: ErrorKind::IdentityMismatch,
        message: "HUMATA_SESSION_TOKEN isn't a session token humatad2 gave out".to_owned(),
        fix: "start the agent from a Humata session tab, which sets it; don't copy a token from another tab, since the daemon refuses it".to_owned(),
        retry: None,
        details: Map::new(),
    }
}

fn from_status(status: &Status, socket: &Path) -> ToolError {
    if let Some(error) = status::error_of(status) {
        return tool_error(status.code(), error);
    }
    if status.code() == Code::Unavailable {
        let mut details = Map::new();
        details.insert("socket".to_owned(), json!(socket.display().to_string()));
        return ToolError {
            kind: ErrorKind::DaemonNotRunning,
            message: format!("humatad2 isn't answering on {}", socket.display()),
            fix: "start it with `humatad2 serve` on this machine, or open the Humata app, which starts it; the call can then be made again".to_owned(),
            retry: Some(Retry {
                retryable: true,
                after: None,
                same_key: true,
            }),
            details,
        };
    }
    ToolError {
        kind: ErrorKind::Internal,
        message: format!("humatad2 failed the call: {}", status.message()),
        fix: "try again; if it keeps failing, the daemon's log on this machine says why".to_owned(),
        retry: None,
        details: Map::new(),
    }
}

fn tool_error(code: Code, error: pb::Error) -> ToolError {
    let kind = match &error.kind {
        Some(kind) => kind_of(kind),
        // GRPC.md §10: a method this daemon doesn't have.
        None if code == Code::Unimplemented => ErrorKind::PeerVersionUnsupported,
        None => ErrorKind::Internal,
    };
    let retry = error.retry.map(|advice| Retry {
        retryable: true,
        after: advice.after.map(|after| {
            SystemTime::now()
                + Duration::new(
                    u64::try_from(after.seconds).unwrap_or(0),
                    u32::try_from(after.nanos).unwrap_or(0),
                )
        }),
        same_key: advice.same_key,
    });
    let details = error.kind.as_ref().map(details_of).unwrap_or_default();
    ToolError {
        kind,
        message: error.message,
        fix: error.fix,
        retry,
        details,
    }
}

macro_rules! kinds {
    ($($kind:ident),* $(,)?) => {
        /// `humata.v1.Error`'s kind as agents see it.
        fn kind_of(kind: &pb::error::Kind) -> ErrorKind {
            match kind {
                $(pb::error::Kind::$kind(_) => ErrorKind::$kind,)*
            }
        }
    };
}

kinds! {
    NotFound, MachineOffline, HostUnreachable, Denied, UserOnly, PresenceRequired,
    IdentityMismatch, PeerUnproven, RevisionConflict, PlanStale, NameTaken, InvalidField, UnpushedWork,
    SessionBusy, ParentReadOnly, MainCloneReadOnly, OutsideHumataFolders, DiskFull, RateLimited,
    GithubCli, GitFailed, PushRefused, SshFailed, HostKeyChanged, ChecksumMismatch, ResumeFailed,
    AgentLacksFeature, ProviderStepFailed, ProviderIdentityInvalid, LoginNeedsUser,
    PeerVersionUnsupported, OffsetTrimmed, NotCancellable, AlreadyAnswered, RequestWithdrawn,
    UserAway, NotInChannel, PrClosed, ReviewPartlyPosted, SkillNameClash, LingerUnavailable,
    DaemonAlreadyRunning, DaemonNotRunning, JournalEntryRejected, RepoNotVisible, StepFailed,
    PassphraseLocked, Internal,
}

/// The fields of the kinds agents most often branch on, with enum values
/// lowercase and without their prefix (D25). Other kinds carry none yet.
fn details_of(kind: &pb::error::Kind) -> Map<String, Value> {
    let fields = match kind {
        pb::error::Kind::Denied(denied) => json!({
            "source": denied.source().as_str_name().strip_prefix("SOURCE_").map(str::to_ascii_lowercase),
            "capability": capability_name(denied.capability),
            "request": denied.request.as_ref().map(|id| id.value.clone()),
            "user_note": denied.user_note,
        }),
        pb::error::Kind::UserOnly(user_only) => json!({
            "capability": capability_name(user_only.capability),
        }),
        pb::error::Kind::NotFound(missing) => json!({
            "kind": missing.kind().as_str_name().strip_prefix("ENTITY_KIND_").map(str::to_ascii_lowercase),
            "id": missing.id,
            "in_trash": missing.in_trash,
        }),
        pb::error::Kind::InvalidField(field) => json!({
            "field": field.field,
            "problem": field.problem().as_str_name().strip_prefix("PROBLEM_").map(str::to_ascii_lowercase),
        }),
        pb::error::Kind::RequestWithdrawn(withdrawn) => json!({
            "request": withdrawn.request.as_ref().map(|id| id.value.clone()),
        }),
        pb::error::Kind::UserAway(away) => json!({
            "channel": away.channel.as_ref().map(|id| id.value.clone()),
        }),
        pb::error::Kind::Internal(internal) => json!({
            "incident_id": internal.incident_id,
            "log_path": internal.log_path,
        }),
        _ => return Map::new(),
    };
    match fields {
        Value::Object(fields) => fields,
        _ => Map::new(),
    }
}

fn capability_name(value: i32) -> Option<String> {
    pb::Capability::try_from(value)
        .ok()
        .and_then(|capability| {
            capability
                .as_str_name()
                .strip_prefix("CAPABILITY_")
                .map(str::to_ascii_lowercase)
        })
        .filter(|name| name != "unspecified")
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
