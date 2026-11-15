//! PeerService on the peer link (GRPC.md §8), and the gate in front of the
//! whole link. The host and the machine prove to each other that they hold
//! the machine's enrollment key (`link_proof`): Hello, then ProveHost. Until
//! ProveHost succeeds, every call but those two and DaemonService.WhoAmI is
//! refused, and a wrong host proof closes the link.
//!
//! The link is one process and one connection, so its handshake state is
//! the process's. Hello, ProveHost and an Uplink that only says where to
//! resume are built; the rest of PeerService answers "not built", behind
//! the gate like everything else.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use humata2_chunk::Link;
use humata2_grpc::Timestamp;
use humata2_grpc::humata::v1 as pb;
use tokio::sync::Notify;
use tokio_stream::wrappers::ReceiverStream;
use tonic::body::Body;
use tonic::codegen::http;
use tonic::service::Interceptor;
use tonic::{Request, Response, Status, Streaming};

use crate::attach;
use crate::compression::Compression;
use crate::daemon::{API_MAJOR, API_MINOR};
use crate::identity;
use crate::link_proof::{
    EnrollmentKey, HOST_NONCE_MAX, HOST_NONCE_MIN, KeyError, MACHINE_NONCE_BYTES, Side,
};
use crate::status;

use pb::peer_unproven::Reason;

/// The methods open on the link before the host has proved itself, as
/// (service, method).
const OPEN_BEFORE_PROOF: [(&str, &str); 3] = [
    (pb::peer_service_server::SERVICE_NAME, "Hello"),
    (pb::peer_service_server::SERVICE_NAME, "ProveHost"),
    (pb::daemon_service_server::SERVICE_NAME, "WhoAmI"),
];

/// Where the link's handshake is.
enum Phase {
    /// No Hello yet.
    Fresh,
    /// Hello was refused; nothing can be proved on this link.
    HelloRefused,
    /// Hello was answered; waiting for the host's proof.
    Challenged {
        key: EnrollmentKey,
        host_nonce: Vec<u8>,
        machine_nonce: [u8; MACHINE_NONCE_BYTES],
    },
    /// The host proved itself.
    Proven,
    /// The host's proof was wrong; the link is closing.
    Closed,
}

/// The link's handshake, shared by the gate and PeerService.
pub(crate) struct Handshake {
    key_file: PathBuf,
    machine: String,
    /// What Hello offers the host to decode.
    compression: Arc<Compression>,
    booted_at: Timestamp,
    phase: Mutex<Phase>,
    host_proof_wrong: AtomicBool,
    /// Whether the link's one Uplink was opened.
    uplink_opened: AtomicBool,
    close: Notify,
}

impl Handshake {
    pub(crate) fn new(key_file: PathBuf, machine: String, compression: Arc<Compression>) -> Self {
        Self {
            key_file,
            machine,
            compression,
            booted_at: attach::now(),
            phase: Mutex::new(Phase::Fresh),
            host_proof_wrong: AtomicBool::new(false),
            uplink_opened: AtomicBool::new(false),
            close: Notify::new(),
        }
    }

    /// Resolves once a wrong host proof means the link must close.
    pub(crate) async fn closing(&self) {
        self.close.notified().await;
    }

    /// Whether the link closed because the host's proof was wrong.
    pub(crate) fn host_proof_was_wrong(&self) -> bool {
        self.host_proof_wrong.load(Ordering::SeqCst)
    }

    fn proven(&self) -> bool {
        matches!(*lock(&self.phase), Phase::Proven)
    }

    fn refuse(&self, reason: Reason, message: impl Into<String>, fix: impl Into<String>) -> Status {
        status::peer_unproven(reason, &self.machine, message, fix)
    }

    fn out_of_order(&self, message: &str) -> Status {
        self.refuse(
            Reason::OutOfOrder,
            message,
            "start a new link and call Hello once, then ProveHost once (GRPC.md §8)",
        )
    }
}

/// Marks a request whose method is open before the host has proved
/// itself. Runs before `Gate`, which can't see the path. Extensions are the
/// server's own: a client can't set this marker.
#[derive(Clone, Copy, Debug)]
struct OpenBeforeProof;

/// Marks every call on the link as the peer's, and those open before the
/// host has proved itself.
pub(crate) fn mark(request: http::Request<Body>) -> http::Request<Body> {
    mark_open(identity::mark_peer(request))
}

fn mark_open(mut request: http::Request<Body>) -> http::Request<Body> {
    let open = request
        .uri()
        .path()
        .strip_prefix('/')
        .and_then(|path| path.split_once('/'))
        .is_some_and(|route| OPEN_BEFORE_PROOF.iter().any(|listed| *listed == route));
    if open {
        request.extensions_mut().insert(OpenBeforeProof);
    }
    request
}

/// Refuses every call the handshake doesn't allow yet.
#[derive(Clone)]
pub(crate) struct Gate {
    handshake: Arc<Handshake>,
}

impl Gate {
    pub(crate) fn new(handshake: Arc<Handshake>) -> Self {
        Self { handshake }
    }
}

impl Interceptor for Gate {
    fn call(&mut self, request: Request<()>) -> Result<Request<()>, Status> {
        if request.extensions().get::<OpenBeforeProof>().is_some() || self.handshake.proven() {
            return Ok(request);
        }
        Err(self.handshake.refuse(
            Reason::HostNotProven,
            "humatad2 refused the call on the peer link: the host hasn't proved it holds this machine's enrollment key",
            "call PeerService.Hello, then PeerService.ProveHost with the host's proof, before anything else on the link (GRPC.md §8)",
        ))
    }
}

pub(crate) struct Peer {
    handshake: Arc<Handshake>,
}

impl Peer {
    pub(crate) fn new(handshake: Arc<Handshake>) -> Self {
        Self { handshake }
    }
}

fn not_built(method: &str) -> Status {
    status::not_built(&format!("{}.{method}", pb::peer_service_server::SERVICE_NAME))
}

fn key_refused(handshake: &Handshake, error: &KeyError) -> Status {
    let reason = match error {
        KeyError::NotEnrolled { .. } => Reason::NotEnrolled,
        KeyError::OpenToOthers { .. } | KeyError::NotYours { .. } | KeyError::NotAFile { .. } => {
            Reason::KeyUnsafe
        }
        KeyError::Read { .. } | KeyError::Damaged { .. } => Reason::KeyUnreadable,
    };
    handshake.refuse(reason, status::chain(error), error.fix())
}

type Empty<T> = tokio_stream::Empty<Result<T, Status>>;

#[tonic::async_trait]
impl pb::peer_service_server::PeerService for Peer {
    /// The machine's half of the proof. One per link, whatever its outcome.
    async fn hello(
        &self,
        request: Request<pb::HelloRequest>,
    ) -> Result<Response<pb::HelloResponse>, Status> {
        let hello = request.into_inner();
        let handshake = &self.handshake;
        let mut phase = lock(&handshake.phase);
        if !matches!(*phase, Phase::Fresh) {
            return Err(handshake.out_of_order(
                "humatad2 refused a second Hello on the peer link: a link has one",
            ));
        }
        *phase = Phase::HelloRefused;
        if !(HOST_NONCE_MIN..=HOST_NONCE_MAX).contains(&hello.nonce.len()) {
            return Err(status::invalid_field(
                "nonce",
                pb::invalid_field::Problem::OutOfRange,
                format!(
                    "the host's nonce is {} bytes; the machine takes {HOST_NONCE_MIN} to {HOST_NONCE_MAX}",
                    hello.nonce.len()
                ),
                format!(
                    "send {HOST_NONCE_MIN} to {HOST_NONCE_MAX} fresh random bytes on a new link; a short nonce makes the machine's proof easier to replay"
                ),
            ));
        }
        let key = EnrollmentKey::read(&handshake.key_file)
            .map_err(|error| key_refused(handshake, &error))?;
        let mut machine_nonce = [0u8; MACHINE_NONCE_BYTES];
        rand::fill(&mut machine_nonce[..]);
        let proof = key.proof(Side::Machine, &hello.nonce, &machine_nonce);
        *phase = Phase::Challenged {
            key,
            host_nonce: hello.nonce,
            machine_nonce,
        };
        Ok(Response::new(pb::HelloResponse {
            machine: Some(pb::MachineId {
                value: handshake.machine.clone(),
            }),
            proof,
            daemon_version: env!("CARGO_PKG_VERSION").to_owned(),
            api_major: API_MAJOR,
            api_minor: API_MINOR,
            // Neither is kept by this process yet (see `peer_link`).
            journal_high_water: 0,
            running: Vec::new(),
            booted_at: Some(handshake.booted_at),
            machine_nonce: machine_nonce.to_vec(),
            // What this machine's chunked streams to the host may use,
            // ranked for the link as measured so far (TR-11).
            chunk_codecs: handshake.compression.offered(Link::Remote {
                bytes_per_second: None,
            }),
        }))
    }

    /// The host's half. A wrong proof closes the link.
    async fn prove_host(
        &self,
        request: Request<pb::ProveHostRequest>,
    ) -> Result<Response<pb::ProveHostResponse>, Status> {
        let proof = request.into_inner().proof;
        let handshake = &self.handshake;
        let mut phase = lock(&handshake.phase);
        match std::mem::replace(&mut *phase, Phase::Closed) {
            Phase::Challenged {
                key,
                host_nonce,
                machine_nonce,
            } => {
                if key.verify(Side::Host, &host_nonce, &machine_nonce, &proof) {
                    *phase = Phase::Proven;
                    return Ok(Response::new(pb::ProveHostResponse {}));
                }
                drop(phase);
                handshake.host_proof_wrong.store(true, Ordering::SeqCst);
                handshake.close.notify_one();
                Err(handshake.refuse(
                    Reason::HostProofWrong,
                    "the host's proof on the peer link is wrong, so this machine closed the link: whatever started `humatad2 serve --stdio` doesn't hold this machine's enrollment key",
                    "reconnect from Humata on the Mac. If it fails the same way, the Mac's key and this machine's differ: forget the machine on the Mac and add it again. If you didn't start this link, something else on this machine tried to pose as your Mac",
                ))
            }
            Phase::Proven => {
                *phase = Phase::Proven;
                Err(handshake.out_of_order(
                    "humatad2 refused ProveHost: the host already proved itself on this link",
                ))
            }
            Phase::Closed => Err(handshake.refuse(
                Reason::HostProofWrong,
                "the peer link is closing after a wrong host proof",
                "reconnect from Humata on the Mac",
            )),
            earlier @ (Phase::Fresh | Phase::HelloRefused) => {
                *phase = earlier;
                Err(handshake.out_of_order(
                    "humatad2 refused ProveHost: no Hello on this link was answered first",
                ))
            }
        }
    }

    type UplinkStream = ReceiverStream<Result<pb::MachineToHost, Status>>;

    /// One per link. This process keeps no journal, mirror or policy yet
    /// (see `peer_link`), so it says to resume from nothing, reads what the
    /// host sends without acting on it, and ends its side when the host
    /// ends its own. Behind the gate, so only a proven host gets here.
    async fn uplink(
        &self,
        request: Request<Streaming<pb::HostToMachine>>,
    ) -> Result<Response<Self::UplinkStream>, Status> {
        if self.handshake.uplink_opened.swap(true, Ordering::SeqCst) {
            return Err(self.handshake.out_of_order(
                "humatad2 refused a second Uplink on the peer link: a link has one for its life",
            ));
        }
        let mut from_host = request.into_inner();
        let (to_host, stream) = tokio::sync::mpsc::channel(1);
        let resume = pb::MachineToHost {
            message: Some(pb::machine_to_host::Message::Resume(pb::UplinkResume {
                from: Some(pb::UplinkAck::default()),
            })),
        };
        tokio::spawn(async move {
            if to_host.send(Ok(resume)).await.is_err() {
                return;
            }
            // Dropping `to_host` once the host's side ends ends this one.
            while let Ok(Some(_message)) = from_host.message().await {}
        });
        Ok(Response::new(ReceiverStream::new(stream)))
    }

    async fn launch_session(
        &self,
        _request: Request<pb::LaunchSessionRequest>,
    ) -> Result<Response<pb::LaunchSessionResponse>, Status> {
        Err(not_built("LaunchSession"))
    }

    async fn stop_holder(
        &self,
        _request: Request<pb::StopHolderRequest>,
    ) -> Result<Response<pb::StopHolderResponse>, Status> {
        Err(not_built("StopHolder"))
    }

    async fn resume_holder(
        &self,
        _request: Request<pb::ResumeHolderRequest>,
    ) -> Result<Response<pb::LaunchSessionResponse>, Status> {
        Err(not_built("ResumeHolder"))
    }

    async fn list_holders(
        &self,
        _request: Request<pb::ListHoldersRequest>,
    ) -> Result<Response<pb::ListHoldersResponse>, Status> {
        Err(not_built("ListHolders"))
    }

    async fn prepare_clone(
        &self,
        _request: Request<pb::PrepareCloneRequest>,
    ) -> Result<Response<pb::PrepareCloneResponse>, Status> {
        Err(not_built("PrepareClone"))
    }

    async fn receive_bundle(
        &self,
        _request: Request<Streaming<pb::BundleChunk>>,
    ) -> Result<Response<pb::ReceiveBundleResponse>, Status> {
        Err(not_built("ReceiveBundle"))
    }

    type CreateBundleStream = Empty<pb::BundleChunk>;

    async fn create_bundle(
        &self,
        _request: Request<pb::CreateBundleRequest>,
    ) -> Result<Response<Self::CreateBundleStream>, Status> {
        Err(not_built("CreateBundle"))
    }

    type CreatePushPackStream = Empty<pb::BundleChunk>;

    async fn create_push_pack(
        &self,
        _request: Request<pb::CreatePushPackRequest>,
    ) -> Result<Response<Self::CreatePushPackStream>, Status> {
        Err(not_built("CreatePushPack"))
    }

    type ExportConversationStream = Empty<pb::ConversationChunk>;

    async fn export_conversation(
        &self,
        _request: Request<pb::ExportConversationRequest>,
    ) -> Result<Response<Self::ExportConversationStream>, Status> {
        Err(not_built("ExportConversation"))
    }

    async fn import_conversation(
        &self,
        _request: Request<Streaming<pb::ImportConversationMessage>>,
    ) -> Result<Response<pb::ImportConversationResponse>, Status> {
        Err(not_built("ImportConversation"))
    }

    async fn apply_skills(
        &self,
        _request: Request<Streaming<pb::SkillsMessage>>,
    ) -> Result<Response<pb::ApplySkillsResponse>, Status> {
        Err(not_built("ApplySkills"))
    }

    async fn probe(
        &self,
        _request: Request<pb::ProbeRequest>,
    ) -> Result<Response<pb::ProbeResponse>, Status> {
        Err(not_built("Probe"))
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
