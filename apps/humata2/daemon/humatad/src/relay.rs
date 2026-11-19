//! The peer link's relay (D139, GRPC.md §2, §3). After the host has proved
//! itself, `humatad2 serve --stdio` forwards an allowlist of ordinary client
//! calls to the machine's own socket daemon, which runs the same code it
//! runs for a local call. This process keeps no store and no holders.
//!
//! The relay is a generic gRPC proxy: a request on the link is handed to a
//! connection to the socket daemon as the same HTTP/2 request, headers,
//! message frames and trailers untouched, in both directions. Messages are
//! never decoded, so compressed chunks stay as they are, streams of every
//! shape work, and the HTTP/2 flow control of the two connections is the
//! backpressure. Dropping a call on either side drops it on the other.
//!
//! The relay carries no credential of its own on each call. The socket
//! connection proves itself once (`relay_auth`): the link calls BeginRelay
//! on it before answering Hello and uses the nonce it gets as the machine's
//! Hello nonce, then, when ProveHost succeeds, calls CompleteRelay with the
//! host's nonce and proof as received. The daemon checks the host's proof
//! over that nonce and marks the connection as the host's. Until
//! then relayed calls answer peer_unproven. Every authentication header the
//! host sent is removed. When the connection ends for good, `Relay::lost`
//! resolves so the link process can exit and the host redial (TR-7:
//! nothing here polls).

use std::convert::Infallible;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll};

use humata2_grpc::humata::v1 as pb;
use hyper_util::rt::TokioIo;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::UnixStream;
use tokio::sync::{Notify, OnceCell};
use tonic::body::Body;
use tonic::codegen::{Service, http};
use tonic::server::NamedService;
use tonic::transport::server::Router;
use tonic::transport::{Channel, Endpoint, Uri};
use tonic::{Code, Status};
use tower::ServiceExt;

use crate::error::DaemonError;
use crate::identity;
use crate::status;

/// The metadata key the host puts the id it assigned to a call's subject
/// under (D139: the host mints a session's id before relaying its start).
/// The relay passes it through as it is, so only its tests name it.
#[cfg(test)]
const HOST_ASSIGNED_ID_HEADER: &str = "humata-host-assigned-id";

/// Headers the relay removes from the host's request: every one the socket
/// daemon's authentication reads, and its own.
const STRIPPED: [&str; 4] = [
    identity::TOKEN_HEADER,
    identity::LOGIN_HEADER,
    identity::SCRIPT_HEADER,
    // UP-1: the host's caller's release is the host's business; the
    // machine's daemon would take it for a CLI's.
    crate::upgrade::build::BUNDLE_HEADER,
];

/// The services the host may call whole.
const WHOLE_SERVICES: [&str; 5] = [
    pb::terminal_service_server::SERVICE_NAME,
    pb::file_service_server::SERVICE_NAME,
    pb::trash_service_server::SERVICE_NAME,
    pb::diff_service_server::SERVICE_NAME,
    // A transfer's far end (FM-8, FM-13): the host's TransferService drives
    // it; no app, CLI or agent calls it through the host.
    pb::transfer_end_service_server::SERVICE_NAME,
];

/// The services the host may call only by these methods.
const LISTED_METHODS: [(&str, &[&str]); 8] = [
    (
        pb::session_service_server::SERVICE_NAME,
        &[
            "StartSession",
            "StopSession",
            // D801 J: a move pauses the agent it is taking away.
            "FreezeSession",
            "ResumeSession",
            "GetSession",
            "ListSessions",
            "RenameSession",
            "PreviewBranchNames",
            "PlanArchive",
            "ArchiveSession",
            "RestoreSession",
            // D187, D208: a context-only fork of a machine's session runs
            // there, in its parent's clones.
            "ForkSession",
            // D773: giving such a fork its own copy runs there too, where the
            // parent's clones are.
            "GiveOwnCopy",
            // CV-8: continuing a machine's session in another channel runs
            // there, where its conversation is.
            "ContinueInChannel",
            // F3: pushing a machine's session runs on that machine, with its
            // own git and gh, in its own clones.
            "PlanPush",
            "PushAndOpenPullRequests",
        ],
    ),
    (
        pb::channel_service_server::SERVICE_NAME,
        // D314: a channel's rename moves its folder on every machine it
        // lists; each machine derives the folder from the name.
        &["CreateChannel", "GetChannel", "RenameChannel"],
    ),
    (
        pb::operation_service_server::SERVICE_NAME,
        &["GetOperation", "WatchOperation", "CancelOperation"],
    ),
    (pb::event_service_server::SERVICE_NAME, &["Subscribe"]),
    // D189: the host reads a machine's conversation for its mirror there,
    // where the file and the host variable values are. Search and read stay
    // off the list: the host keeps the copy.
    (pb::history_service_server::SERVICE_NAME, &["ReadMirror"]),
    // D195: the host's driver stages, checks and tidies a session that moves.
    // The carry service itself refuses every caller but the host.
    (
        pb::session_carry_service_server::SERVICE_NAME,
        &[
            "Pack",
            "Unpack",
            "Release",
            "Abort",
            "PreviewFork",
            "Target",
            // PR-4, PR-21: the host reads a review's clones on the machine.
            "RunReviewGit",
        ],
    ),
    // MV-17: a Desk session, plain or a channel's own, starts on a machine
    // through the host, and the host reads where a moved one landed.
    (
        pb::desk_service_server::SERVICE_NAME,
        &[
            "StartDeskSession",
            "ListDeskSessions",
            // A Desk session that works on a machine is closed and cleaned
            // up there: its folder is that machine's (MV-17).
            "CloseDeskSession",
            "PlanDeskCleanUp",
            "CleanUpDesk",
            // DV-2 to DV-8: a Desk folder's versions are kept on the machine
            // its session works on.
            "ListDocVersions",
            "ReadDocVersion",
            "SaveDocVersion",
            "RestoreDocVersion",
            "DiffDocVersion",
            // D458: each machine's own Desk folders get their repositories
            // through the host's Migrate item for that machine.
            "CheckDeskFolders",
            "MigrateDeskFolders",
            // DK-15, MV-17: a Desk session that works on a machine moves into
            // a channel there; the host plans and starts the move.
            "PlanDeskMove",
            "MoveDeskSessionToChannel",
        ],
    ),
    // CH-16, D394: viewing a remote session on the host holds the machine's
    // view of it open, so its read mark clears there too.
    (pb::mark_service_server::SERVICE_NAME, &["ViewSession"]),
];

/// LD-13 to LD-15: the host's Storage page of a machine reads and cleans that
/// machine's own storage there. Kept apart from `LISTED_METHODS` so the
/// allowlist's length line stays untouched.
const MACHINE_METHODS: (&str, &[&str]) = (
    pb::machine_service_server::SERVICE_NAME,
    &[
        "GetStorage",
        "ClearBuildCache",
        "TrimLogs",
        // HO-7, HO-13: a machine's host variables are kept on that machine; the
        // host sends it a value once and reads back only names.
        "ListHostVariables",
        "SetHostVariable",
        "RemoveHostVariable",
        // FC-10: the host plans and makes a change of a box's Humata folder
        // on the box's own daemon, which serves both to the host alone.
        "PlanChangeRoot",
        "ChangeRoot",
        // D784: a machine lists, stops, resumes and ends its own holders.
        "ListRunningHolders",
        "StopRunningHolder",
        "ResumeRunningHolder",
        "EndRunningHolder",
        "AdoptRunningHolder",
        // D784, D783: a machine the host forgot or removed lets go of the
        // host's sessions, its holders still running.
        "LetGoOfHost",
    ],
);

/// Every service the link answers for the host on the relay's behalf: the
/// eleven with an allowlisted method come first, then the client services
/// that are never relayed, so a call to one is refused by name rather than
/// with a bare UNIMPLEMENTED. `Routed<N>` is `ROUTED[N]`.
const ROUTED: [&str; 31] = [
    pb::terminal_service_server::SERVICE_NAME,
    pb::file_service_server::SERVICE_NAME,
    pb::trash_service_server::SERVICE_NAME,
    pb::diff_service_server::SERVICE_NAME,
    pb::transfer_end_service_server::SERVICE_NAME,
    pb::session_service_server::SERVICE_NAME,
    pb::channel_service_server::SERVICE_NAME,
    pb::operation_service_server::SERVICE_NAME,
    pb::event_service_server::SERVICE_NAME,
    pb::history_service_server::SERVICE_NAME,
    pb::session_carry_service_server::SERVICE_NAME,
    pb::desk_service_server::SERVICE_NAME,
    pb::mark_service_server::SERVICE_NAME,
    pb::machine_service_server::SERVICE_NAME,
    pb::access_service_server::SERVICE_NAME,
    pb::cli_auth_service_server::SERVICE_NAME,
    pb::audit_service_server::SERVICE_NAME,
    pb::board_service_server::SERVICE_NAME,
    pb::activity_service_server::SERVICE_NAME,
    pb::transfer_service_server::SERVICE_NAME,
    pb::provider_service_server::SERVICE_NAME,
    pb::skills_service_server::SERVICE_NAME,
    pb::bridge_service_server::SERVICE_NAME,
    pb::inbox_service_server::SERVICE_NAME,
    pb::comment_service_server::SERVICE_NAME,
    pb::pull_request_review_service_server::SERVICE_NAME,
    pb::view_service_server::SERVICE_NAME,
    pb::review_chat_service_server::SERVICE_NAME,
    pb::settings_service_server::SERVICE_NAME,
    pb::agent_service_server::SERVICE_NAME,
    pb::view_state_service_server::SERVICE_NAME,
];

/// Whether the host may relay a call to `path` (`/service/Method`). Shared
/// by the link process, which relays, and the socket daemon, which honours
/// the relay header only for these.
pub(crate) fn allows(path: &str) -> bool {
    if withheld(path) {
        return false;
    }
    let Some((service, method)) = path.strip_prefix('/').and_then(|path| path.split_once('/'))
    else {
        return false;
    };
    WHOLE_SERVICES.contains(&service)
        || LISTED_METHODS
            .iter()
            .chain([&MACHINE_METHODS])
            .any(|(listed, methods)| *listed == service && methods.contains(&method))
}

/// Test daemons only: `HUMATA2_PEER_WITHHOLD=GiveOwnCopy` makes this daemon
/// refuse that method as an older humatad2 would, which doesn't list it.
fn withheld(path: &str) -> bool {
    (humata2_rpc_kit::admin::is_admin_mode() || cfg!(feature = "faults"))
        && std::env::var("HUMATA2_PEER_WITHHOLD")
            .is_ok_and(|method| path.rsplit('/').next() == Some(method.as_str()))
}

/// Calls the link process makes itself on its proven socket connection,
/// which the host can't relay (AWAY): the daemon takes them as the host's.
const LINK_OWN: [&str; 4] = [
    "/humata.v1.DaemonService/PeerUplink",
    // The link hands the host's stop and start on to the machine's daemon
    // (D262): the link serves DaemonService itself, so they can't be relayed.
    "/humata.v1.DaemonService/StopSessions",
    "/humata.v1.DaemonService/StartSessions",
    // UP-1: likewise the host's upgrade of the machine's daemon (D281 f).
    "/humata.v1.DaemonService/Upgrade",
];

/// Whether the proven socket connection may call `path`: what the host may
/// relay, and the link's own calls.
pub(crate) fn proven_may_call(path: &str) -> bool {
    allows(path) || LINK_OWN.contains(&path)
}

/// Why the relay couldn't reach the machine's daemon or prove itself to it.
/// `Display` says what failed; the status the host gets adds what to do.
#[derive(Debug, thiserror::Error)]
pub(crate) enum RelayError {
    #[error("couldn't reach or start the machine's daemon")]
    Daemon {
        #[source]
        source: DaemonError,
    },

    #[error("couldn't open the connection to the machine's daemon")]
    Connect {
        #[source]
        source: tonic::transport::Error,
    },

    #[error("the connection to the machine's daemon ended earlier and the link doesn't reopen it")]
    Gone,

    #[error("this link has no socket to relay to: it was started with its own --state and no --socket")]
    Off,

    #[error("the machine's daemon refused BeginRelay")]
    Begin {
        #[source]
        source: Status,
    },

    #[error("the machine's daemon gave a {len}-byte nonce, not 32")]
    NonceLength { len: usize },

    #[error("the machine's daemon refused CompleteRelay")]
    Complete {
        #[source]
        source: Status,
    },

    #[error("the call couldn't be carried to the machine's daemon")]
    Carry {
        #[source]
        source: tonic::transport::Error,
    },
}

impl RelayError {
    fn status(&self) -> Status {
        self.failure(None)
    }

    /// [`Self::status`] for a call relayed to `machine`'s daemon. When that
    /// daemon can't be reached or the call breaks on the way, the error is
    /// MachineOffline: the host then follows the call as one whose answer was
    /// lost (D150), not as the machine's refusal. A link that relays nothing
    /// (D146) reaches no daemon either, so it is offline too (HO-5).
    fn status_on(&self, machine: &str) -> Status {
        self.failure(Some(machine))
    }

    fn failure(&self, machine: Option<&str>) -> Status {
        let kind = match (self, machine) {
            (
                RelayError::Daemon { .. }
                | RelayError::Connect { .. }
                | RelayError::Gone
                | RelayError::Off
                | RelayError::Carry { .. },
                Some(machine),
            ) => Some(pb::error::Kind::MachineOffline(pb::MachineOffline {
                machine: None,
                machine_name: machine.to_owned(),
                since: None,
            })),
            _ => None,
        };
        let fix = match self {
            RelayError::Off => "this link relays nothing (D146); start `humatad2 serve --stdio` with --socket, or with neither --state nor --socket as the Mac does",
            _ => "reconnect this machine from the Mac, which starts a new link; if this repeats, run `humatad2 serve --role machine` on the machine to see why its daemon doesn't answer, and check that the machine's enrollment key is the one the Mac holds",
        };
        let code = match self {
            RelayError::Complete { source } | RelayError::Begin { source }
                if source.code() == Code::PermissionDenied || source.code() == Code::Unauthenticated =>
            {
                Code::Unauthenticated
            }
            _ => Code::Unavailable,
        };
        status::failure(
            code,
            pb::Error {
                message: status::chain(self),
                fix: fix.to_owned(),
                retry: None,
                kind,
            },
        )
    }
}

/// The refusal of a method the host may not relay.
fn off_list(path: &str) -> Status {
    let method = path.trim_start_matches('/').replace('/', ".");
    status::failure(
        Code::PermissionDenied,
        pb::Error {
            message: format!(
                "humatad2 refused {method} on the peer link: the host may relay only terminal, file, trash and diff calls, session start/stop/freeze/resume/get/list, channel create/get, operation get/watch/cancel, the event feed and a conversation read for the host's mirror (HistoryService/ReadMirror)"
            ),
            fix: "do this on the Mac's own daemon, or on the machine itself with `humata`".to_owned(),
            retry: None,
            kind: None,
        },
    )
}

/// Anything the socket connection can be.
trait Transport: AsyncRead + AsyncWrite + Send + Unpin + 'static {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin + 'static> Transport for T {}

type Io = Box<dyn Transport>;
type Connecting = Pin<Box<dyn Future<Output = Result<Io, RelayError>> + Send>>;
type Starter = Box<dyn FnOnce() -> Connecting + Send>;

/// Where the socket connection's proof is.
enum Phase {
    /// BeginRelay not called yet.
    Fresh,
    /// BeginRelay answered with this nonce; waiting for ProveHost.
    Begun,
    /// CompleteRelay succeeded: the connection is the host's.
    Ready,
    /// The proof can't be made on this link; relayed calls answer this.
    Failed(Status),
}

struct Shared {
    machine: String,
    /// Connects to the socket daemon, once.
    start: Mutex<Option<Starter>>,
    channel: OnceCell<Channel>,
    phase: Mutex<Phase>,
    /// The socket daemon's build, from BeginRelay (UP-1, D281).
    build: Mutex<Option<pb::BuildInfo>>,
    /// Notified, once, when the socket connection is gone.
    lost: Arc<Notify>,
}

/// The relay: one connection to the machine's socket daemon, shared by
/// every relayed call.
#[derive(Clone)]
pub(crate) struct Relay {
    shared: Arc<Shared>,
}

impl Relay {
    /// A relay for the link of `machine`. `connect` returns the connection
    /// to the socket daemon, starting the daemon if nobody listens; it is
    /// called once, at `begin`.
    pub(crate) fn new<F, Fut, S>(machine: String, connect: F) -> Self
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<S, RelayError>> + Send + 'static,
        S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        let start: Starter = Box::new(move || {
            Box::pin(async move { connect().await.map(|io| Box::new(io) as Io) })
        });
        Self {
            shared: Arc::new(Shared {
                machine,
                start: Mutex::new(Some(start)),
                channel: OnceCell::new(),
                phase: Mutex::new(Phase::Fresh),
                build: Mutex::new(None),
                lost: Arc::new(Notify::new()),
            }),
        }
    }

    /// A relay to the daemon on `socket`. `connect_or_start` is
    /// `bridge::connect_or_start`: it connects, or starts a machine daemon
    /// there and waits for it (see the enable lines). With no socket (D146:
    /// the link was given its own state folder and no `--socket`) the relay
    /// is off, as [`Relay::off`].
    pub(crate) fn to_socket<F, Fut>(socket: Option<PathBuf>, machine: String, connect_or_start: F) -> Self
    where
        F: FnOnce(PathBuf) -> Fut + Send + 'static,
        Fut: Future<Output = Result<UnixStream, DaemonError>> + Send + 'static,
    {
        let Some(socket) = socket else {
            return Self::off(machine);
        };
        Self::new(machine, move || async move {
            connect_or_start(socket)
                .await
                .map_err(|source| RelayError::Daemon { source })
        })
    }

    /// A relay with no daemon to reach (D146): nothing is connected or
    /// started, `begin` answers `None` so Hello uses a random nonce, and
    /// relayed calls answer unavailable.
    pub(crate) fn off(machine: String) -> Self {
        Self::new(machine, || async { Err::<UnixStream, _>(RelayError::Off) })
    }

    /// Resolves when the socket connection has ended for good: the link
    /// has nothing left to relay to and should end, so the host dials a
    /// new one.
    pub(crate) async fn lost(&self) {
        self.shared.lost.notified().await;
    }

    /// Connects to the socket daemon and asks it for this connection's
    /// nonce (BeginRelay), for Hello to offer as the machine's nonce. None
    /// when no daemon could be reached or it refused: Hello then uses a
    /// random nonce, and relayed calls answer unavailable.
    pub(crate) async fn begin(&self) -> Option<[u8; 32]> {
        match self.try_begin().await {
            Ok(nonce) => {
                *lock(&self.shared.phase) = Phase::Begun;
                Some(nonce)
            }
            Err(failure) => {
                *lock(&self.shared.phase) = Phase::Failed(failure);
                None
            }
        }
    }

    async fn try_begin(&self) -> Result<[u8; 32], Status> {
        // A failure here answers the host's relayed calls (and is only logged
        // for ProveHost), so a daemon never reached is the machine offline.
        let channel = self
            .channel()
            .await
            .map_err(|error| error.status_on(&self.shared.machine))?;
        let reply = pb::daemon_service_client::DaemonServiceClient::new(channel)
            .begin_relay(pb::BeginRelayRequest {})
            .await
            .map_err(|source| RelayError::Begin { source }.status())?;
        let reply = reply.into_inner();
        *lock(&self.shared.build) = reply.build;
        let nonce = reply.nonce;
        <[u8; 32]>::try_from(nonce)
            .map_err(|nonce| RelayError::NonceLength { len: nonce.len() }.status())
    }

    /// The host's ProveHost succeeded with `host_nonce` and `proof`, as the
    /// link received them: has the socket daemon check them and take this
    /// connection as the host's (CompleteRelay). Relayed calls are
    /// forwarded only after this succeeds.
    pub(crate) async fn complete(&self, host_nonce: &[u8], proof: &[u8]) -> Result<(), Status> {
        {
            match &*lock(&self.shared.phase) {
                Phase::Begun => {}
                Phase::Failed(failure) => return Err(failure.clone()),
                Phase::Fresh | Phase::Ready => return Err(self.unproven()),
            }
        }
        let channel = self.channel().await.map_err(|error| error.status())?;
        let done = pb::daemon_service_client::DaemonServiceClient::new(channel)
            .complete_relay(pb::CompleteRelayRequest {
                host_nonce: host_nonce.to_vec(),
                proof: proof.to_vec(),
            })
            .await;
        match done {
            Ok(_) => {
                *lock(&self.shared.phase) = Phase::Ready;
                Ok(())
            }
            Err(source) => {
                let failure = RelayError::Complete { source }.status();
                *lock(&self.shared.phase) = Phase::Failed(failure.clone());
                Err(failure)
            }
        }
    }

    /// The proven connection to the socket daemon, for the link's own calls
    /// (the machine's session reports, D139). `None` unless the host's
    /// proof was accepted: the relay is off, no daemon answered, or the
    /// proof was refused. Never connects and never waits. The daemon treats
    /// these calls as the host's, so they must stay on the allowlist.
    pub(crate) fn proven_channel(&self) -> Option<Channel> {
        if !matches!(&*lock(&self.shared.phase), Phase::Ready) {
            return None;
        }
        self.shared.channel.get().cloned()
    }

    /// The socket daemon's build, as BeginRelay gave it (UP-1, D281). None
    /// until the connection has begun and a daemon answered with one.
    pub(crate) fn build(&self) -> Option<pb::BuildInfo> {
        lock(&self.shared.build).clone()
    }

    fn unproven(&self) -> Status {
        status::peer_unproven(
            pb::peer_unproven::Reason::HostNotProven,
            &self.shared.machine,
            "humatad2 refused the call on the peer link: the host hasn't proved it holds this machine's enrollment key",
            "call PeerService.Hello, then PeerService.ProveHost with the host's proof, before anything else on the link (GRPC.md §8)",
        )
    }

    /// The connection to the socket daemon, made on first use.
    async fn channel(&self) -> Result<Channel, RelayError> {
        let channel = self.shared.channel.get_or_try_init(|| self.connect()).await?;
        Ok(channel.clone())
    }

    async fn connect(&self) -> Result<Channel, RelayError> {
        let start = lock(&self.shared.start).take().ok_or(RelayError::Gone)?;
        let io = start().await?;
        let slot = Arc::new(Mutex::new(Some(Watched {
            io,
            lost: self.shared.lost.clone(),
        })));
        // One connection, as on the link's own stdio: when it ends the
        // relay is over, and the channel isn't to dial another.
        Endpoint::from_static("http://socket.daemon")
            .initial_stream_window_size(Some(1 << 20))
            .initial_connection_window_size(Some(4 << 20))
            .connect_with_connector(tower::service_fn(move |_: Uri| {
                let io = lock(&slot).take();
                async move {
                    io.map(TokioIo::new)
                        .ok_or_else(|| std::io::Error::other("the relay's connection ended"))
                }
            }))
            .await
            .map_err(|source| RelayError::Connect { source })
    }

    /// Forwards `request` to the socket daemon and returns its response,
    /// body and trailers still streaming.
    async fn forward(&self, mut request: http::Request<Body>) -> Result<http::Response<Body>, Status> {
        if !allows(request.uri().path()) {
            return Err(off_list(request.uri().path()));
        }
        match &*lock(&self.shared.phase) {
            Phase::Ready => {}
            Phase::Failed(failure) => return Err(failure.clone()),
            Phase::Fresh | Phase::Begun => return Err(self.unproven()),
        }
        let machine = &self.shared.machine;
        let mut channel = self.channel().await.map_err(|error| error.status_on(machine))?;
        strip_credentials(request.headers_mut());
        let ready = channel.ready().await.map_err(|source| {
            self.shared.lost.notify_one();
            RelayError::Carry { source }.status_on(machine)
        })?;
        ready.call(request).await.map_err(|source| {
            self.shared.lost.notify_one();
            RelayError::Carry { source }.status_on(machine)
        })
    }
}

/// Removes every header the socket daemon's authentication reads. The
/// host's assigned id and everything else stay.
fn strip_credentials(headers: &mut http::HeaderMap) {
    for name in STRIPPED {
        headers.remove(name);
    }
}

/// The socket connection, which tells the relay when it is dropped: the
/// HTTP/2 client drops it when the connection ends.
struct Watched {
    io: Io,
    lost: Arc<Notify>,
}

impl Drop for Watched {
    fn drop(&mut self) {
        self.lost.notify_one();
    }
}

impl AsyncRead for Watched {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.io).poll_read(context, buf)
    }
}

impl AsyncWrite for Watched {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.io).poll_write(context, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.io).poll_flush(context)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.io).poll_shutdown(context)
    }
}

/// One service name on the link, answered by relaying (when the method is
/// on the allowlist) or by refusing the method by name.
#[derive(Clone)]
struct Routed<const N: usize>(Relay);

impl<const N: usize> NamedService for Routed<N> {
    const NAME: &'static str = ROUTED[N];
}

impl<const N: usize> Service<http::Request<Body>> for Routed<N> {
    type Response = http::Response<Body>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<http::Response<Body>, Infallible>> + Send>>;

    fn poll_ready(&mut self, _context: &mut Context<'_>) -> Poll<Result<(), Infallible>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: http::Request<Body>) -> Self::Future {
        let relay = self.0.clone();
        Box::pin(async move {
            Ok(match relay.forward(request).await {
                Ok(response) => response,
                Err(refusal) => refusal.into_http(),
            })
        })
    }
}

/// Adds `$make::<N>($arg.clone())` to `$router` for each listed N.
macro_rules! add_routed {
    ($router:expr, $make:ident, $arg:expr, $($n:literal)*) => {{
        let router = $router;
        $( let router = router.add_service($make::<$n>($arg.clone())); )*
        router
    }};
}

/// Adds the relay's services to the peer link's `router`, in place of
/// `not_built::register_peer_link`. They sit behind whatever layers the
/// router has, so the handshake gate covers them.
pub(crate) fn register<L>(router: Router<L>, relay: &Relay) -> Router<L> {
    add_routed!(
        router,
        Routed,
        relay,
        0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30
    )
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    //! The proxy in front of a fake socket daemon, over in-memory pipes.

    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use prost::Message;
    use prost::bytes::{Buf, BufMut, Bytes};
    use tokio::io::DuplexStream;
    use tokio::sync::mpsc;
    use tokio_stream::wrappers::ReceiverStream;
    use tokio_stream::{Stream, StreamExt};
    use tonic::codec::{Codec, DecodeBuf, Decoder, EncodeBuf, Encoder};
    use tonic::codegen::http::uri::PathAndQuery;
    use tonic::server::{Grpc, StreamingService};
    use tonic::service::Routes;
    use tonic::transport::Server;
    use tonic::{Request, Response, Streaming};

    use super::*;

    const WAIT: Duration = Duration::from_secs(10);
    const MACHINE: &str = "ma_test";
    const NONCE: [u8; 32] = [7; 32];
    const HOST_NONCE: [u8; 32] = [9; 32];
    const PROOF: [u8; 32] = [3; 32];

    #[test]
    fn the_allowlist_is_exactly_what_d139_names() {
        for allowed in [
            "/humata.v1.TerminalService/Attach",
            "/humata.v1.TerminalService/AnythingElse",
            "/humata.v1.FileService/ReadFile",
            "/humata.v1.TrashService/ListTrash",
            "/humata.v1.DiffService/GetDiff",
            "/humata.v1.TransferEndService/WriteAt",
            "/humata.v1.SessionService/StartSession",
            "/humata.v1.SessionService/StopSession",
            "/humata.v1.SessionService/FreezeSession",
            "/humata.v1.SessionService/ResumeSession",
            "/humata.v1.SessionService/GetSession",
            "/humata.v1.SessionService/ListSessions",
            "/humata.v1.SessionService/RenameSession",
            "/humata.v1.SessionService/PreviewBranchNames",
            "/humata.v1.SessionService/PlanArchive",
            "/humata.v1.SessionService/ArchiveSession",
            "/humata.v1.SessionService/RestoreSession",
            "/humata.v1.SessionService/ForkSession",
            "/humata.v1.SessionService/GiveOwnCopy",
            "/humata.v1.SessionService/ContinueInChannel",
            "/humata.v1.SessionService/PlanPush",
            "/humata.v1.SessionService/PushAndOpenPullRequests",
            "/humata.v1.ChannelService/CreateChannel",
            "/humata.v1.ChannelService/GetChannel",
            "/humata.v1.OperationService/GetOperation",
            "/humata.v1.OperationService/WatchOperation",
            "/humata.v1.OperationService/CancelOperation",
            "/humata.v1.EventService/Subscribe",
            "/humata.v1.HistoryService/ReadMirror",
            "/humata.v1.SessionCarryService/Pack",
            "/humata.v1.SessionCarryService/Unpack",
            "/humata.v1.SessionCarryService/Release",
            "/humata.v1.SessionCarryService/Abort",
            "/humata.v1.SessionCarryService/PreviewFork",
            "/humata.v1.SessionCarryService/Target",
            "/humata.v1.SessionCarryService/RunReviewGit",
            "/humata.v1.DeskService/ListDocVersions",
            "/humata.v1.DeskService/ReadDocVersion",
            "/humata.v1.DeskService/SaveDocVersion",
            "/humata.v1.DeskService/RestoreDocVersion",
            "/humata.v1.DeskService/DiffDocVersion",
            "/humata.v1.DeskService/CheckDeskFolders",
            "/humata.v1.DeskService/MigrateDeskFolders",
            "/humata.v1.DeskService/PlanDeskMove",
            "/humata.v1.DeskService/MoveDeskSessionToChannel",
            "/humata.v1.MarkService/ViewSession",
            "/humata.v1.MachineService/GetStorage",
            "/humata.v1.MachineService/ClearBuildCache",
            "/humata.v1.MachineService/TrimLogs",
            "/humata.v1.MachineService/ListHostVariables",
            "/humata.v1.MachineService/SetHostVariable",
            "/humata.v1.MachineService/RemoveHostVariable",
            "/humata.v1.MachineService/PlanChangeRoot",
            "/humata.v1.MachineService/ChangeRoot",
            "/humata.v1.MachineService/ListRunningHolders",
            "/humata.v1.MachineService/StopRunningHolder",
            "/humata.v1.MachineService/ResumeRunningHolder",
            "/humata.v1.MachineService/EndRunningHolder",
            "/humata.v1.MachineService/AdoptRunningHolder",
            "/humata.v1.MachineService/LetGoOfHost",
        ] {
            assert!(allows(allowed), "{allowed}");
        }
        for refused in [
            "/humata.v1.SessionService/DeleteSession",
            "/humata.v1.MarkService/SetMarks",
            "/humata.v1.MachineService/ListMachines",
            "/humata.v1.MachineService/RemoveMachine",
            "/humata.v1.MachineService/TrustNewHostKey",
            "/humata.v1.MarkService/ListPinned",
            "/humata.v1.ChannelService/DeleteChannel",
            "/humata.v1.OperationService/RetryOperation",
            "/humata.v1.AccessService/ListRules",
            "/humata.v1.InboxService/Answer",
            "/humata.v1.HistoryService/SearchSessions",
            "/humata.v1.HistoryService/ReadTurns",
            "/humata.v1.HistoryService/ReadMirrorX",
            "/humata.v1.TransferService/Anything",
            "/humata.v1.PeerService/Hello",
            "/humata.v1.DaemonService/WhoAmI",
            "/humata.v1.DaemonService/BeginRelay",
            "/humata.v1.DaemonService/CompleteRelay",
            "/humata.v1.TerminalServiceX/Attach",
            "/humata.v1.SessionService/",
            "/humata.v1.SessionService",
            "humata.v1.SessionService/ListSessions",
            "",
        ] {
            assert!(!allows(refused), "{refused}");
        }
    }

    #[test]
    fn the_host_cant_relay_the_links_own_uplink_but_the_link_may_call_it() {
        let uplink = "/humata.v1.DaemonService/PeerUplink";
        assert!(!allows(uplink));
        assert!(proven_may_call(uplink));
        assert!(proven_may_call("/humata.v1.SessionService/ListSessions"));
        assert!(!proven_may_call("/humata.v1.DaemonService/WhoAmI"));
    }

    #[test]
    fn the_link_may_hand_on_the_hosts_stop_and_start_but_the_host_cant_relay_them() {
        for path in ["/humata.v1.DaemonService/StopSessions", "/humata.v1.DaemonService/StartSessions"] {
            assert!(proven_may_call(path), "{path}");
            assert!(!allows(path), "{path}");
        }
    }

    #[test]
    fn the_routed_table_leads_with_every_service_the_allowlist_names() {
        let named: Vec<&str> = WHOLE_SERVICES
            .iter()
            .copied()
            .chain(
                LISTED_METHODS
                    .iter()
                    .chain([&MACHINE_METHODS])
                    .map(|(service, _)| *service),
            )
            .collect();
        assert_eq!(&ROUTED[..named.len()], named.as_slice());
    }

    #[test]
    fn stripping_removes_every_credential_and_keeps_the_rest() {
        let mut headers = http::HeaderMap::new();
        for name in STRIPPED {
            headers.insert(name, http::HeaderValue::from_static("x"));
        }
        headers.insert(HOST_ASSIGNED_ID_HEADER, http::HeaderValue::from_static("se_1"));
        headers.insert("grpc-encoding", http::HeaderValue::from_static("gzip"));
        strip_credentials(&mut headers);
        for name in STRIPPED {
            assert!(!headers.contains_key(name), "{name}");
        }
        assert_eq!(headers[HOST_ASSIGNED_ID_HEADER], "se_1");
        assert_eq!(headers["grpc-encoding"], "gzip");
    }

    /// Messages as they are on the wire, with no decoding.
    #[derive(Clone, Copy, Default)]
    struct RawCodec;
    struct RawEncoder;
    struct RawDecoder;

    impl Codec for RawCodec {
        type Encode = Bytes;
        type Decode = Bytes;
        type Encoder = RawEncoder;
        type Decoder = RawDecoder;
        fn encoder(&mut self) -> RawEncoder {
            RawEncoder
        }
        fn decoder(&mut self) -> RawDecoder {
            RawDecoder
        }
    }

    impl Encoder for RawEncoder {
        type Item = Bytes;
        type Error = Status;
        fn encode(&mut self, item: Bytes, dst: &mut EncodeBuf<'_>) -> Result<(), Status> {
            dst.put_slice(&item);
            Ok(())
        }
    }

    impl Decoder for RawDecoder {
        type Item = Bytes;
        type Error = Status;
        fn decode(&mut self, src: &mut DecodeBuf<'_>) -> Result<Option<Bytes>, Status> {
            Ok(Some(src.copy_to_bytes(src.remaining())))
        }
    }

    #[derive(Clone, Default)]
    struct Daemon {
        seen: Arc<Mutex<Vec<(String, http::HeaderMap)>>>,
        completed: Arc<Mutex<Vec<pb::CompleteRelayRequest>>>,
        /// Set when a call this fake was holding open ended.
        released: Arc<AtomicBool>,
        released_note: Arc<Notify>,
    }

    /// The fake socket daemon's services: `ROUTED[..9]` and DaemonService.
    const FAKE_NAMES: [&str; 10] = [
        ROUTED[0],
        ROUTED[1],
        ROUTED[2],
        ROUTED[3],
        ROUTED[4],
        ROUTED[5],
        ROUTED[6],
        ROUTED[7],
        ROUTED[8],
        pb::daemon_service_server::SERVICE_NAME,
    ];

    /// The fake socket daemon's service for `FAKE_NAMES[N]`: BeginRelay
    /// answers `NONCE`, CompleteRelay is recorded, every other message is
    /// echoed once, `Subscribe` sends each three times, and
    /// `WatchOperation` stays open after echoing until the call is
    /// cancelled.
    #[derive(Clone)]
    struct Fake<const N: usize>(Daemon);

    impl<const N: usize> NamedService for Fake<N> {
        const NAME: &'static str = FAKE_NAMES[N];
    }

    impl<const N: usize> Service<http::Request<Body>> for Fake<N> {
        type Response = http::Response<Body>;
        type Error = Infallible;
        type Future = Pin<Box<dyn Future<Output = Result<http::Response<Body>, Infallible>> + Send>>;

        fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Infallible>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, request: http::Request<Body>) -> Self::Future {
            let daemon = self.0.clone();
            Box::pin(async move {
                let path = request.uri().path().to_owned();
                lock(&daemon.seen).push((path.clone(), request.headers().clone()));
                let mut grpc = Grpc::new(RawCodec);
                Ok(grpc.streaming(Echo { daemon, path }, request).await)
            })
        }
    }

    struct Echo {
        daemon: Daemon,
        path: String,
    }

    type Replies = Pin<Box<dyn Stream<Item = Result<Bytes, Status>> + Send>>;

    impl StreamingService<Bytes> for Echo {
        type Response = Bytes;
        type ResponseStream = Replies;
        type Future = std::future::Ready<Result<Response<Replies>, Status>>;

        fn call(&mut self, request: Request<Streaming<Bytes>>) -> Self::Future {
            let mut input = request.into_inner();
            let (replies, out) = mpsc::channel(8);
            let daemon = self.daemon.clone();
            let path = self.path.clone();
            tokio::spawn(async move {
                let times = if path.ends_with("/Subscribe") { 3 } else { 1 };
                while let Some(Ok(message)) = input.next().await {
                    let reply = if path.ends_with("/BeginRelay") {
                        Bytes::from(
                            pb::BeginRelayResponse {
                                nonce: NONCE.to_vec(),
                                build: None,
                            }
                            .encode_to_vec(),
                        )
                    } else if path.ends_with("/CompleteRelay") {
                        if let Ok(done) = pb::CompleteRelayRequest::decode(message.clone()) {
                            lock(&daemon.completed).push(done);
                        }
                        Bytes::new()
                    } else {
                        message
                    };
                    for _ in 0..times {
                        if replies.send(Ok(reply.clone())).await.is_err() {
                            return;
                        }
                    }
                }
                if path.ends_with("/WatchOperation") {
                    replies.closed().await;
                    daemon.released.store(true, Ordering::SeqCst);
                    daemon.released_note.notify_one();
                }
            });
            let stream: Replies = Box::pin(ReceiverStream::new(out));
            std::future::ready(Ok(Response::new(stream)))
        }
    }

    struct Rig {
        host: Channel,
        daemon: Daemon,
        relay: Relay,
        /// The wire between the relay and the fake daemon: ending this
        /// task cuts the socket connection.
        fake: tokio::task::JoinHandle<()>,
    }

    /// Serves `router` on one in-memory connection that never ends.
    async fn serve(router: Router, end: DuplexStream) {
        let incoming = tokio_stream::once(Ok::<_, std::io::Error>(end)).chain(tokio_stream::pending());
        let _ = router.serve_with_incoming(incoming).await;
    }

    /// A relay in front of a fake daemon, with its link served; nothing
    /// begun yet.
    async fn unproven_rig() -> Rig {
        let daemon = Daemon::default();
        // The relay's end and the daemon's end are joined by a task that
        // copies both ways. A server keeps a connection it accepted until
        // the other side closes it, so cutting the connection means ending
        // that task, which closes both middle ends.
        let (socket_client, mut relay_side) = tokio::io::duplex(1 << 16);
        let (mut daemon_side, socket_server) = tokio::io::duplex(1 << 16);
        let fake = Server::builder().add_routes(Routes::default());
        let fake = add_routed!(fake, Fake, daemon, 0 1 2 3 4 5 6 7 8 9);
        tokio::spawn(serve(fake, socket_server));
        let fake_task = tokio::spawn(async move {
            let _ = tokio::io::copy_bidirectional(&mut relay_side, &mut daemon_side).await;
        });

        let relay = Relay::new(MACHINE.to_owned(), move || async move {
            Ok::<_, RelayError>(socket_client)
        });
        let (host_client, host_server) = tokio::io::duplex(1 << 16);
        let link = register(Server::builder().add_routes(Routes::default()), &relay);
        tokio::spawn(serve(link, host_server));
        Rig {
            host: connect_over(host_client).await,
            daemon,
            relay,
            fake: fake_task,
        }
    }

    /// As `unproven_rig`, with the connection begun and completed.
    async fn rig() -> Rig {
        let rig = unproven_rig().await;
        assert_eq!(rig.relay.begin().await, Some(NONCE));
        rig.relay.complete(&HOST_NONCE, &PROOF).await.unwrap();
        rig
    }

    async fn connect_over(io: DuplexStream) -> Channel {
        let io = Arc::new(Mutex::new(Some(io)));
        Endpoint::from_static("http://host.link")
            .connect_with_connector(tower::service_fn(move |_: Uri| {
                let io = lock(&io).take();
                async move {
                    io.map(TokioIo::new)
                        .ok_or_else(|| std::io::Error::other("one connection"))
                }
            }))
            .await
            .unwrap()
    }

    fn path(text: &'static str) -> PathAndQuery {
        PathAndQuery::from_static(text)
    }

    async fn client(rig: &Rig) -> tonic::client::Grpc<Channel> {
        let mut client = tonic::client::Grpc::new(rig.host.clone());
        client.ready().await.unwrap();
        client
    }

    const LIST: &str = "/humata.v1.SessionService/ListSessions";
    const ATTACH: &str = "/humata.v1.TerminalService/Attach";
    const SUBSCRIBE: &str = "/humata.v1.EventService/Subscribe";
    const WATCH: &str = "/humata.v1.OperationService/WatchOperation";

    #[tokio::test]
    async fn a_unary_call_passes_through_byte_identical() {
        let rig = rig().await;
        // Not valid protobuf on purpose: the relay never decodes.
        let message = Bytes::from_static(&[0xff, 0x00, 0x13, 0x37, 0xfe]);
        let reply = tokio::time::timeout(
            WAIT,
            client(&rig)
                .await
                .unary(Request::new(message.clone()), path(LIST), RawCodec),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(reply.into_inner(), message);
    }

    #[tokio::test]
    async fn a_server_stream_passes_through_in_order() {
        let rig = rig().await;
        let message = Bytes::from_static(b"\x00\x01\x02");
        let reply = client(&rig)
            .await
            .server_streaming(Request::new(message.clone()), path(SUBSCRIBE), RawCodec)
            .await
            .unwrap();
        let received: Vec<Bytes> =
            tokio::time::timeout(WAIT, reply.into_inner().map(Result::unwrap).collect())
                .await
                .unwrap();
        assert_eq!(received, vec![message; 3]);
    }

    #[tokio::test]
    async fn a_bidirectional_stream_passes_through_both_ways() {
        let rig = rig().await;
        let sent: Vec<Bytes> = (0u8..20)
            .map(|byte| Bytes::from(vec![byte; 1 + usize::from(byte) * 700]))
            .collect();
        let reply = client(&rig)
            .await
            .streaming(Request::new(tokio_stream::iter(sent.clone())), path(ATTACH), RawCodec)
            .await
            .unwrap();
        let received: Vec<Bytes> =
            tokio::time::timeout(WAIT, reply.into_inner().map(Result::unwrap).collect())
                .await
                .unwrap();
        assert_eq!(received, sent);
    }

    #[tokio::test]
    async fn a_client_stream_passes_through() {
        let rig = rig().await;
        let sent = vec![Bytes::from_static(b"only one reply comes back")];
        let reply = client(&rig)
            .await
            .client_streaming(Request::new(tokio_stream::iter(sent.clone())), path(ATTACH), RawCodec)
            .await
            .unwrap();
        assert_eq!(reply.into_inner(), sent[0]);
    }

    #[tokio::test]
    async fn a_method_off_the_allowlist_is_refused_and_never_reaches_the_daemon() {
        let rig = rig().await;
        let reached = lock(&rig.daemon.seen).len();
        for off in [
            "/humata.v1.SessionService/DeleteSession",
            "/humata.v1.AccessService/ListRules",
        ] {
            let refused = client(&rig)
                .await
                .unary(Request::new(Bytes::new()), PathAndQuery::from_static(off), RawCodec)
                .await
                .unwrap_err();
            assert_eq!(refused.code(), Code::PermissionDenied, "{off}");
        }
        // DaemonService is the link's own, never relayed: whatever the link
        // answers for the relay's own calls, they don't reach the daemon.
        client(&rig)
            .await
            .unary(
                Request::new(Bytes::new()),
                PathAndQuery::from_static("/humata.v1.DaemonService/BeginRelay"),
                RawCodec,
            )
            .await
            .unwrap_err();
        assert_eq!(lock(&rig.daemon.seen).len(), reached);
    }

    #[tokio::test]
    async fn calls_before_the_proof_completes_are_refused_unproven_and_never_reach_the_daemon() {
        let rig = unproven_rig().await;
        let refused = client(&rig)
            .await
            .unary(Request::new(Bytes::new()), path(LIST), RawCodec)
            .await
            .unwrap_err();
        assert_eq!(refused.code(), Code::Unauthenticated);
        assert!(lock(&rig.daemon.seen).is_empty());

        // Begun but not completed is still unproven.
        assert_eq!(rig.relay.begin().await, Some(NONCE));
        let refused = client(&rig)
            .await
            .unary(Request::new(Bytes::new()), path(LIST), RawCodec)
            .await
            .unwrap_err();
        assert_eq!(refused.code(), Code::Unauthenticated);
        assert_eq!(lock(&rig.daemon.seen).len(), 1, "only BeginRelay reached it");
    }

    #[tokio::test]
    async fn complete_hands_the_hosts_nonce_and_proof_on_unchanged() {
        let rig = rig().await;
        let completed = lock(&rig.daemon.completed);
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].host_nonce, HOST_NONCE);
        assert_eq!(completed[0].proof, PROOF);
    }

    #[tokio::test]
    async fn the_daemon_sees_none_of_the_hosts_credentials_and_the_assigned_id() {
        let rig = rig().await;
        let mut request = Request::new(Bytes::from_static(b"x"));
        for name in [
            identity::TOKEN_HEADER,
            identity::LOGIN_HEADER,
            identity::SCRIPT_HEADER,
        ] {
            request.metadata_mut().insert(name, "forged".parse().unwrap());
        }
        request
            .metadata_mut()
            .insert(HOST_ASSIGNED_ID_HEADER, "se_host".parse().unwrap());
        client(&rig)
            .await
            .unary(request, path(LIST), RawCodec)
            .await
            .unwrap();

        let seen = lock(&rig.daemon.seen);
        let (_, headers) = seen.last().unwrap();
        assert_eq!(headers[HOST_ASSIGNED_ID_HEADER], "se_host");
        for name in [identity::TOKEN_HEADER, identity::LOGIN_HEADER, identity::SCRIPT_HEADER] {
            assert!(!headers.contains_key(name), "{name}");
        }
    }

    #[tokio::test]
    async fn the_host_cancelling_ends_the_call_at_the_daemon() {
        let rig = rig().await;
        let reply = client(&rig)
            .await
            .server_streaming(Request::new(Bytes::from_static(b"w")), path(WATCH), RawCodec)
            .await
            .unwrap();
        let mut replies = reply.into_inner();
        assert!(replies.next().await.is_some());
        assert!(!rig.daemon.released.load(Ordering::SeqCst));
        drop(replies);
        tokio::time::timeout(WAIT, rig.daemon.released_note.notified())
            .await
            .expect("the daemon's call kept running after the host cancelled");
    }

    #[tokio::test]
    async fn the_relay_reports_when_the_socket_connection_ends() {
        let rig = rig().await;
        rig.fake.abort();
        tokio::time::timeout(WAIT, rig.relay.lost())
            .await
            .expect("the relay never said its connection was lost");
    }

    #[tokio::test]
    async fn no_daemon_means_no_nonce_and_unavailable_calls_but_a_working_link() {
        let relay = Relay::new(MACHINE.to_owned(), || async {
            Err::<DuplexStream, _>(RelayError::Gone)
        });
        assert_eq!(relay.begin().await, None);

        let (host_client, host_server) = tokio::io::duplex(1 << 16);
        let link = register(Server::builder().add_routes(Routes::default()), &relay);
        tokio::spawn(serve(link, host_server));
        let mut client = tonic::client::Grpc::new(connect_over(host_client).await);
        client.ready().await.unwrap();
        let failed = client
            .unary(Request::new(Bytes::new()), path(LIST), RawCodec)
            .await
            .unwrap_err();
        assert_eq!(failed.code(), Code::Unavailable);
        assert!(relay.complete(&HOST_NONCE, &PROOF).await.is_err());
    }

    #[tokio::test]
    async fn a_link_that_relays_nothing_answers_relayed_calls_as_the_machine_offline() {
        let relay = Relay::off(MACHINE.to_owned());
        assert_eq!(relay.begin().await, None);

        let (host_client, host_server) = tokio::io::duplex(1 << 16);
        let link = register(Server::builder().add_routes(Routes::default()), &relay);
        tokio::spawn(serve(link, host_server));
        let mut client = tonic::client::Grpc::new(connect_over(host_client).await);
        client.ready().await.unwrap();
        let failed = client
            .unary(Request::new(Bytes::new()), path(LIST), RawCodec)
            .await
            .unwrap_err();
        assert_eq!(failed.code(), Code::Unavailable);
        let kind = crate::status::error_of(&failed).and_then(|error| error.kind);
        assert!(matches!(kind, Some(pb::error::Kind::MachineOffline(_))), "{failed:?}");
    }
}
