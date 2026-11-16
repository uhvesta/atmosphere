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
use crate::relay::Relay;
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
    /// The link to the machine's socket daemon (D139, D144): Hello's nonce
    /// comes from it, and the host's proof is handed to it.
    relay: Relay,
    booted_at: Timestamp,
    phase: Mutex<Phase>,
    host_proof_wrong: AtomicBool,
    /// Whether the link's one Uplink was opened.
    uplink_opened: AtomicBool,
    close: Notify,
}

impl Handshake {
    pub(crate) fn new(
        key_file: PathBuf,
        machine: String,
        compression: Arc<Compression>,
        relay: Relay,
    ) -> Self {
        Self {
            key_file,
            machine,
            compression,
            relay,
            booted_at: attach::now(),
            phase: Mutex::new(Phase::Fresh),
            host_proof_wrong: AtomicBool::new(false),
            uplink_opened: AtomicBool::new(false),
            close: Notify::new(),
        }
    }

    /// The link's relay to the machine's socket daemon.
    pub(crate) fn relay(&self) -> &Relay {
        &self.relay
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
    /// Where synced skills go on this machine (SK-4); none when the link
    /// process has no home folder to put them under.
    skills: Option<crate::peer_skills::MachineSkills>,
}

impl Peer {
    pub(crate) fn new(handshake: Arc<Handshake>) -> Self {
        Self {
            handshake,
            skills: crate::peer_skills::MachineSkills::from_env(),
        }
    }
}

/// A machine nonce from nothing but randomness: what Hello offers when no
/// socket daemon gave one (D146).
fn random_nonce() -> [u8; MACHINE_NONCE_BYTES] {
    let mut nonce = [0u8; MACHINE_NONCE_BYTES];
    rand::fill(&mut nonce[..]);
    nonce
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
        // The phase stays `HelloRefused` while the relay connects below, so
        // a second Hello or an early ProveHost is refused meanwhile. The
        // lock lives only in this block: it is never held across the await.
        let key = {
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
            EnrollmentKey::read(&handshake.key_file).map_err(|error| key_refused(handshake, &error))?
        };
        // D144: the nonce the socket daemon issued to this link, so the
        // host's ProveHost is bound to that connection; random when no
        // daemon answered (D146).
        let machine_nonce = handshake
            .relay
            .begin()
            .await
            .unwrap_or_else(random_nonce);
        let proof = key.proof(Side::Machine, &hello.nonce, &machine_nonce);
        *lock(&handshake.phase) = Phase::Challenged {
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
        // What the proof came to is decided under the phase lock, in this
        // block only; what follows from it happens after, so the lock is
        // never held across an await. `Err(None)` is a wrong proof.
        let outcome: Result<Vec<u8>, Option<Status>> = {
            let mut phase = lock(&handshake.phase);
            match std::mem::replace(&mut *phase, Phase::Closed) {
                Phase::Challenged {
                    key,
                    host_nonce,
                    machine_nonce,
                } => {
                    if key.verify(Side::Host, &host_nonce, &machine_nonce, &proof) {
                        *phase = Phase::Proven;
                        Ok(host_nonce)
                    } else {
                        Err(None)
                    }
                }
                Phase::Proven => {
                    *phase = Phase::Proven;
                    Err(Some(handshake.out_of_order(
                        "humatad2 refused ProveHost: the host already proved itself on this link",
                    )))
                }
                Phase::Closed => Err(Some(handshake.refuse(
                    Reason::HostProofWrong,
                    "the peer link is closing after a wrong host proof",
                    "reconnect from Humata on the Mac",
                ))),
                earlier @ (Phase::Fresh | Phase::HelloRefused) => {
                    *phase = earlier;
                    Err(Some(handshake.out_of_order(
                        "humatad2 refused ProveHost: no Hello on this link was answered first",
                    )))
                }
            }
        };
        match outcome {
            Ok(host_nonce) => {
                // D144: the socket daemon checks the same proof itself. A
                // failure is kept by the relay and answers the host's
                // relayed calls; the link's own calls are unaffected.
                if let Err(failure) = handshake.relay.complete(&host_nonce, &proof).await {
                    crate::log::say!(
                        "humatad2: the peer link couldn't prove itself to this machine's daemon, so relayed calls fail: {}",
                        failure.message()
                    );
                }
                Ok(Response::new(pb::ProveHostResponse {}))
            }
            Err(Some(refused)) => Err(refused),
            Err(None) => {
                handshake.host_proof_wrong.store(true, Ordering::SeqCst);
                handshake.close.notify_one();
                Err(handshake.refuse(
                    Reason::HostProofWrong,
                    "the host's proof on the peer link is wrong, so this machine closed the link: whatever started `humatad2 serve --stdio` doesn't hold this machine's enrollment key",
                    "reconnect from Humata on the Mac. If it fails the same way, the Mac's key and this machine's differ: forget the machine on the Mac and add it again. If you didn't start this link, something else on this machine tried to pose as your Mac",
                ))
            }
        }
    }

    type UplinkStream = ReceiverStream<Result<pb::MachineToHost, Status>>;

    /// One per link: the host's word that the Mac is reachable (AWAY T3).
    /// Before Resume, the socket daemon's PeerUplink is opened on the link's
    /// proven connection, so the daemon counts the host reachable from the
    /// moment the host sees Resume. If that open fails, the link still runs
    /// and the daemon counts the host away until the link is redialled.
    /// Resume says to resume from nothing: this process keeps no journal or
    /// mirror yet (AWAY T5), and the host sends its policy copy whole on
    /// every link (AWAY T4). Then the machine reports its
    /// sessions. Each message the host sends goes to the daemon's
    /// PeerUplink, and each message the daemon sends goes to the host. The
    /// two are carried until one ends: a daemon whose PeerUplink ends with an
    /// error (a newer link replaced it) ends the host's Uplink with that
    /// error, and a clean end of it ends the host's Uplink too. When the
    /// host's side ends, the daemon's requests end. Behind the gate, so only
    /// a proven host gets here.
    async fn uplink(
        &self,
        request: Request<Streaming<pb::HostToMachine>>,
    ) -> Result<Response<Self::UplinkStream>, Status> {
        if self.handshake.uplink_opened.swap(true, Ordering::SeqCst) {
            return Err(self.handshake.out_of_order(
                "humatad2 refused a second Uplink on the peer link: a link has one for its life",
            ));
        }
        let from_host = request.into_inner();
        let (to_host, stream) = tokio::sync::mpsc::channel(1);
        let relay = self.handshake.relay.clone();
        tokio::spawn(async move {
            // The daemon's PeerUplink is open before the host sees Resume.
            let (to_daemon, daemon_inbox) = tokio::sync::mpsc::channel::<pb::HostToMachine>(16);
            let daemon = open_daemon_uplink(&relay, daemon_inbox).await;
            let resume = pb::MachineToHost {
                message: Some(pb::machine_to_host::Message::Resume(pb::UplinkResume {
                    from: Some(pb::UplinkAck::default()),
                })),
            };
            if to_host.send(Ok(resume)).await.is_err() {
                return;
            }
            // D139: after Resume the machine reports its sessions. The
            // reporter is stopped once the carrying ends, which also drops
            // its hold on `to_host`.
            let reporter = tokio::spawn(report_sessions(relay, to_host.clone()));
            carry_uplink(from_host, to_daemon, daemon, &to_host).await;
            reporter.abort();
            let _ = reporter.await;
            // Dropping `to_host` ends the host's Uplink; dropping
            // `to_daemon` ends the daemon's PeerUplink requests.
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
        request: Request<Streaming<pb::SkillsMessage>>,
    ) -> Result<Response<pb::ApplySkillsResponse>, Status> {
        crate::peer_skills::handle(self.skills.as_ref(), request.into_inner())
            .await
            .map(Response::new)
    }

    /// HO-1, HO-3, LD-19: what this machine is, from this process itself
    /// (D139): no store and no socket daemon. A field that can't be read is
    /// empty; the probe never fails because a tool is missing.
    async fn probe(
        &self,
        request: Request<pb::ProbeRequest>,
    ) -> Result<Response<pb::ProbeResponse>, Status> {
        let measure_storage = request.into_inner().measure_storage;
        tokio::task::spawn_blocking(move || host_probe::read(measure_storage))
            .await
            .map(Response::new)
            .map_err(|error| status::internal(format!("reading this machine's health stopped: {error}"), ""))
    }
}

/// Reading the machine the link process runs on (HO-1, HO-3, HO-4, LD-19).
/// Every external command has a short timeout and a failed one leaves its
/// fields empty. The GitHub token is never read (HO-8).
pub(crate) mod host_probe {
    use std::collections::BTreeMap;
    use std::io::Read;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use humata2_grpc::humata::v1 as pb;

    /// How long one external command may run. The commands and the folder
    /// sizes run side by side, so this is about how long a probe takes; it
    /// stays under GetMachine's `PROBE_DEADLINE` (6 s) and leaves a loaded
    /// machine time to start a shell, where a timed-out command would show
    /// as not installed.
    const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
    /// The most output kept from one command.
    const MAX_OUTPUT: u64 = 1 << 20;
    /// The agents the probe looks for: (agent id, command on PATH). The
    /// same ids and commands as the sessions crate's built-in agents.
    const AGENTS: [(&str, &str); 3] = [("claude", "claude"), ("codex", "codex"), ("copilot", "copilot")];
    /// The most entries (files and folders) looked at when measuring the
    /// folders under `~/Humata`; a folder cut short by it is undercounted.
    const MEASURE_ENTRY_CAP: usize = 500_000;

    /// Reads this machine. `measure_storage` also sizes the folders under
    /// `~/Humata` (LD-14: only when asked).
    pub(super) fn read(measure_storage: bool) -> pb::ProbeResponse {
        let home = std::env::var_os("HOME").filter(|home| !home.is_empty()).map(PathBuf::from);
        let installed: Vec<(&str, PathBuf)> = AGENTS
            .iter()
            .filter_map(|(id, command)| locate(command, home.as_deref()).map(|path| (*id, path)))
            .collect();
        let (git, gh, gh_version, linger, agent_versions, folder_bytes) = std::thread::scope(|scope| {
            let folder_bytes = scope.spawn(|| match (&home, measure_storage) {
                (Some(home), true) => folder_sizes(&home.join("Humata")),
                _ => BTreeMap::new(),
            });
            let git = scope.spawn(|| run("git", &["--version"], &[]));
            let gh_version = scope.spawn(|| run("gh", &["--version"], &[("GH_NO_UPDATE_NOTIFIER", "1")]));
            let agents: Vec<_> = installed
                .iter()
                .map(|(_, path)| scope.spawn(move || run(&path.to_string_lossy(), &["--version"], &[])))
                .collect();
            let gh = scope.spawn(|| {
                run(
                    "gh",
                    &["auth", "status", "--hostname", "github.com"],
                    &[("NO_COLOR", "1"), ("GH_PROMPT_DISABLED", "1"), ("GH_NO_UPDATE_NOTIFIER", "1")],
                )
            });
            let linger = scope.spawn(|| {
                if !cfg!(target_os = "linux") {
                    return None;
                }
                let user = std::env::var("USER").or_else(|_| std::env::var("LOGNAME")).ok()?;
                run("loginctl", &["show-user", &user, "-p", "Linger"], &[])
            });
            let agent_versions: Vec<String> = agents
                .into_iter()
                .map(|handle| handle.join().ok().flatten().map(|output| version_number(&output)).unwrap_or_default())
                .collect();
            (
                git.join().ok().flatten(),
                gh.join().ok().flatten(),
                gh_version.join().ok().flatten().map(|output| version_number(&output)).unwrap_or_default(),
                linger.join().ok().flatten(),
                agent_versions,
                folder_bytes.join().unwrap_or_default(),
            )
        });
        let (gh_user, gh_scopes) = gh.map(|output| gh_account(&output)).unwrap_or_default();
        let (disk_free_bytes, disk_total_bytes) = disk(home.as_deref().unwrap_or(Path::new("/")));
        pb::ProbeResponse {
            os: std::env::consts::OS.replace("macos", "darwin"),
            arch: std::env::consts::ARCH.to_owned(),
            git_version: git.map(|output| git_version(&output)).unwrap_or_default(),
            gh_user,
            gh_scopes,
            gh_version,
            agents_installed: installed
                .iter()
                .map(|(id, _)| pb::AgentId { value: (*id).to_owned() })
                .collect(),
            agent_versions,
            disk_free_bytes,
            disk_total_bytes,
            variable_names: Vec::new(),
            linger_enabled: linger.is_some_and(|output| linger_enabled(&output)),
            folder_bytes: folder_bytes.into_iter().collect(),
        }
    }

    /// Runs `program` with `args` and `env` added, with no input, and gives
    /// what it wrote (output, then errors); none when it can't start or
    /// outlives the timeout (then it is killed).
    fn run(program: &str, args: &[&str], env: &[(&str, &str)]) -> Option<String> {
        let mut child = Command::new(program)
            .args(args)
            .envs(env.iter().copied())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .ok()?;
        let out = drain(child.stdout.take());
        let err = drain(child.stderr.take());
        let deadline = Instant::now() + COMMAND_TIMEOUT;
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(15)),
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
            }
        }
        let mut text = out.join().unwrap_or_default();
        text.push('\n');
        text.push_str(&err.join().unwrap_or_default());
        Some(text)
    }

    fn drain<R: Read + Send + 'static>(stream: Option<R>) -> std::thread::JoinHandle<String> {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(stream) = stream {
                let _ = stream.take(MAX_OUTPUT).read_to_end(&mut bytes);
            }
            String::from_utf8_lossy(&bytes).into_owned()
        })
    }

    /// The version number in `git --version`'s output.
    fn git_version(output: &str) -> String {
        output
            .lines()
            .find_map(|line| line.trim().strip_prefix("git version ")?.split_whitespace().next())
            .unwrap_or_default()
            .to_owned()
    }

    /// The first version number in what `--version` printed: the first word that
    /// starts with a digit and has a dot, such as `2.83.0` in "gh version 2.83.0
    /// (2025-11-04)", `2.4.1` in "2.4.1 (Claude Code)" or `0.98.0` in "codex-cli
    /// 0.98.0". Empty when nothing looks like one (HO-4).
    fn version_number(output: &str) -> String {
        output
            .split_whitespace()
            .map(|word| {
                word.trim_matches(|c: char| !c.is_ascii_alphanumeric())
                    .trim_start_matches(['v', 'V'])
            })
            .find(|word| {
                word.starts_with(|c: char| c.is_ascii_digit())
                    && word.contains('.')
                    && word.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
            })
            .unwrap_or_default()
            .to_owned()
    }

    /// The github.com account `gh auth status` says is active (the first
    /// when none says so) and its token scopes; both empty when signed out.
    /// Reads only the account and scope lines, never the token's.
    fn gh_account(output: &str) -> (String, Vec<String>) {
        struct Account {
            user: String,
            active: bool,
            scopes: Vec<String>,
        }
        let mut accounts: Vec<Account> = Vec::new();
        for line in output.lines() {
            let line = line.trim().trim_start_matches(|c: char| !c.is_alphanumeric());
            if let Some(rest) = line.strip_prefix("Logged in to github.com") {
                let rest = rest.trim();
                let user = rest
                    .strip_prefix("account ")
                    .or_else(|| rest.strip_prefix("as "))
                    .and_then(|named| named.split_whitespace().next())
                    .unwrap_or_default();
                if !user.is_empty() {
                    accounts.push(Account {
                        user: user.to_owned(),
                        active: true,
                        scopes: Vec::new(),
                    });
                }
            } else if let Some(value) = line.strip_prefix("Active account:")
                && let Some(last) = accounts.last_mut()
            {
                last.active = value.trim() == "true";
            } else if let Some(value) = line.strip_prefix("Token scopes:")
                && let Some(last) = accounts.last_mut()
            {
                last.scopes = value
                    .split(',')
                    .map(|scope| scope.trim().trim_matches(['\'', '"']).to_owned())
                    .filter(|scope| !scope.is_empty() && scope != "none")
                    .collect();
            }
        }
        let index = accounts.iter().position(|account| account.active).unwrap_or(0);
        accounts
            .into_iter()
            .nth(index)
            .map(|account| (account.user, account.scopes))
            .unwrap_or_default()
    }

    /// Whether `loginctl show-user -p Linger` says linger is on.
    fn linger_enabled(output: &str) -> bool {
        output.lines().any(|line| line.trim() == "Linger=yes")
    }

    /// The executable file `command` is on PATH or in `~/.local/bin`, if it is.
    fn locate(command: &str, home: Option<&Path>) -> Option<PathBuf> {
        let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
            .map(|path| std::env::split_paths(&path).collect())
            .unwrap_or_default();
        if let Some(home) = home {
            dirs.push(home.join(".local/bin"));
        }
        dirs.into_iter().map(|dir| dir.join(command)).find(|path| {
            std::fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        })
    }

    /// Free and total bytes of the filesystem holding `path`; zeros when it
    /// can't be read.
    fn disk(path: &Path) -> (u64, u64) {
        match rustix::fs::statvfs(path) {
            Ok(stat) => (
                stat.f_bavail.saturating_mul(stat.f_frsize),
                stat.f_blocks.saturating_mul(stat.f_frsize),
            ),
            Err(_) => (0, 0),
        }
    }

    /// The bytes of files under each top-level folder of `root`, without
    /// following symlinks, looking at no more than `MEASURE_ENTRY_CAP`
    /// entries in all: past that the rest is left uncounted.
    fn folder_sizes(root: &Path) -> BTreeMap<String, u64> {
        let mut sizes = BTreeMap::new();
        let Ok(top) = std::fs::read_dir(root) else {
            return sizes;
        };
        let mut seen = 0usize;
        for entry in top.flatten() {
            if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                continue;
            }
            let mut bytes = 0u64;
            let mut pending = vec![entry.path()];
            while let Some(dir) = pending.pop() {
                let Ok(children) = std::fs::read_dir(&dir) else {
                    continue;
                };
                for child in children.flatten() {
                    seen += 1;
                    if seen > MEASURE_ENTRY_CAP {
                        sizes.insert(entry.file_name().to_string_lossy().into_owned(), bytes);
                        return sizes;
                    }
                    let Ok(meta) = std::fs::symlink_metadata(child.path()) else {
                        continue;
                    };
                    if meta.is_dir() {
                        pending.push(child.path());
                    } else if meta.is_file() {
                        bytes = bytes.saturating_add(meta.len());
                    }
                }
            }
            sizes.insert(entry.file_name().to_string_lossy().into_owned(), bytes);
        }
        sizes
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn git_version_is_the_number_alone() {
            assert_eq!(git_version("git version 2.43.0\n"), "2.43.0");
            assert_eq!(git_version("git version 2.39.5 (Apple Git-154)\n"), "2.39.5");
            assert_eq!(git_version("\nsomething else"), "");
        }

        #[test]
        fn a_version_is_the_first_number_in_what_the_tool_printed() {
            assert_eq!(version_number("gh version 2.83.0 (2025-11-04)\nhttps://github.com/cli/cli/releases/tag/v2.83.0\n"), "2.83.0");
            assert_eq!(version_number("2.4.1 (Claude Code)\n"), "2.4.1");
            assert_eq!(version_number("codex-cli 0.98.0\n"), "0.98.0");
            assert_eq!(version_number("\nOpenAI Codex v0.98.0-beta.1 (build 7)"), "0.98.0-beta.1");
            assert_eq!(version_number("GitHub Copilot CLI 0.0.355.\n"), "0.0.355");
        }

        #[test]
        fn no_version_is_empty_rather_than_a_guess() {
            for output in ["", "\n", "command not found", "error: 3 files changed on 2025-11-04", "claude 12"] {
                assert_eq!(version_number(output), "", "{output:?}");
            }
        }

        #[test]
        fn a_signed_in_gh_gives_its_user_and_scopes() {
            let output = "github.com\n  \u{2713} Logged in to github.com account octocat (keyring)\n  - Active account: true\n  - Git operations protocol: https\n  - Token: gho_************\n  - Token scopes: 'gist', 'read:org', 'repo'\n";
            let (user, scopes) = gh_account(output);
            assert_eq!(user, "octocat");
            assert_eq!(scopes, ["gist", "read:org", "repo"]);
        }

        #[test]
        fn an_older_gh_shape_is_read_too() {
            let output = "github.com\n  \u{2713} Logged in to github.com as hubot (oauth_token)\n  \u{2713} Token scopes: repo, read:org\n";
            let (user, scopes) = gh_account(output);
            assert_eq!(user, "hubot");
            assert_eq!(scopes, ["repo", "read:org"]);
        }

        #[test]
        fn the_active_account_wins_over_the_first() {
            let output = "github.com\n  \u{2713} Logged in to github.com account one (keyring)\n  - Active account: false\n  - Token scopes: 'repo'\n  \u{2713} Logged in to github.com account two (keyring)\n  - Active account: true\n  - Token scopes: 'gist'\n";
            let (user, scopes) = gh_account(output);
            assert_eq!(user, "two");
            assert_eq!(scopes, ["gist"]);
        }

        #[test]
        fn a_signed_out_or_missing_gh_gives_nothing() {
            for output in [
                "You are not logged into any GitHub hosts. To log in, run: gh auth login\n",
                "github.com\n  X Failed to log in to github.com account octocat (keyring)\n  - Active account: true\n",
                "",
            ] {
                assert_eq!(gh_account(output), (String::new(), Vec::new()));
            }
        }

        #[test]
        fn linger_is_on_only_when_loginctl_says_yes() {
            assert!(linger_enabled("Linger=yes\n"));
            assert!(!linger_enabled("Linger=no\n"));
            assert!(!linger_enabled(""));
        }

        #[test]
        fn a_missing_command_gives_nothing_instead_of_failing() {
            assert!(run("humata-no-such-command", &[], &[]).is_none());
        }

        #[test]
        fn folders_are_sized_without_following_links() {
            let root = tempfile::tempdir().unwrap();
            let one = root.path().join("one/deep");
            std::fs::create_dir_all(&one).unwrap();
            std::fs::write(one.join("a"), [0u8; 10]).unwrap();
            std::fs::write(root.path().join("one/b"), [0u8; 5]).unwrap();
            std::fs::create_dir(root.path().join("two")).unwrap();
            std::os::unix::fs::symlink(root.path().join("one"), root.path().join("two/link")).unwrap();
            std::fs::write(root.path().join("loose"), [0u8; 99]).unwrap();
            let sizes = folder_sizes(root.path());
            assert_eq!(sizes.get("one"), Some(&15));
            assert_eq!(sizes.get("two"), Some(&0));
            assert_eq!(sizes.len(), 2);
        }

        #[test]
        fn the_disk_under_a_folder_has_a_size() {
            let root = tempfile::tempdir().unwrap();
            let (free, total) = disk(root.path());
            assert!(total > 0 && free <= total);
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The reports the machine sends the host for `session`: its status, and
/// its pointers when it has any (SL-3). Nothing for a session with no id.
fn session_reports(session: &pb::Session, at: Timestamp) -> Vec<pb::MachineToHost> {
    let Some(id) = session.id.clone() else {
        return Vec::new();
    };
    let mut reports = vec![pb::MachineToHost {
        message: Some(pb::machine_to_host::Message::Status(pb::SessionStatusReport {
            session: Some(id.clone()),
            status: session.status,
            exit_code: session.exit_code,
            ..pb::SessionStatusReport::default()
        })),
    }];
    if !session.pointers.is_empty() {
        reports.push(pb::MachineToHost {
            message: Some(pb::machine_to_host::Message::Pointers(pb::PointersReport {
                session: Some(id),
                pointers: session.pointers.clone(),
                at: Some(at),
            })),
        });
    }
    reports
}

/// The host's side of the Uplink is gone.
struct HostGone;

async fn send_reports(
    to_host: &tokio::sync::mpsc::Sender<Result<pb::MachineToHost, Status>>,
    session: &pb::Session,
) -> Result<(), HostGone> {
    for report in session_reports(session, attach::now()) {
        to_host.send(Ok(report)).await.map_err(|_| HostGone)?;
    }
    Ok(())
}

/// Every session on the machine, following pages.
async fn list_all(
    sessions: &mut pb::session_service_client::SessionServiceClient<tonic::transport::Channel>,
) -> Result<Vec<pb::Session>, Status> {
    let mut all = Vec::new();
    let mut token = String::new();
    loop {
        let page = pb::PageRequest {
            page_size: 0,
            page_token: token,
        };
        let reply = sessions
            .list_sessions(Request::new(pb::ListSessionsRequest {
                page: Some(page),
                ..pb::ListSessionsRequest::default()
            }))
            .await?
            .into_inner();
        all.extend(reply.sessions);
        token = reply.page.map(|page| page.next_page_token).unwrap_or_default();
        if token.is_empty() {
            return Ok(all);
        }
    }
}

/// What the machine tells the host about its sessions (D139, HO-5, SL-3):
/// subscribe to the socket daemon's session events first, so nothing
/// between the listing and the subscription is missed, then one status
/// report per session and an `UplinkAck` that ends the snapshot, then one
/// report per session event. A resync means every session is reported
/// again. Nothing here polls (TR-7). Without a proven relay it sends
/// nothing; it ends when the subscription or the Uplink does.
async fn report_sessions(
    relay: Relay,
    to_host: tokio::sync::mpsc::Sender<Result<pb::MachineToHost, Status>>,
) {
    let Some(channel) = relay.proven_channel() else {
        return;
    };
    let mut events = pb::event_service_client::EventServiceClient::new(channel.clone());
    let mut sessions = pb::session_service_client::SessionServiceClient::new(channel);
    let subscribed = events
        .subscribe(Request::new(pb::SubscribeRequest {
            topics: vec![pb::Topic::Sessions as i32],
            ..pb::SubscribeRequest::default()
        }))
        .await;
    let mut feed = match subscribed {
        Ok(reply) => reply.into_inner(),
        Err(failure) => {
            crate::log::say!(
                "humatad2: the peer link couldn't follow this machine's sessions, so the host sees none of their changes: {}",
                failure.message()
            );
            return;
        }
    };
    let listed = match list_all(&mut sessions).await {
        Ok(listed) => listed,
        Err(failure) => {
            crate::log::say!(
                "humatad2: the peer link couldn't list this machine's sessions, so the host sees none of them: {}",
                failure.message()
            );
            return;
        }
    };
    for session in &listed {
        if send_reports(&to_host, session).await.is_err() {
            return;
        }
    }
    // Any message of another kind ends the host's snapshot.
    let end = pb::MachineToHost {
        message: Some(pb::machine_to_host::Message::Ack(pb::UplinkAck::default())),
    };
    if to_host.send(Ok(end)).await.is_err() {
        return;
    }
    while let Ok(Some(event)) = feed.message().await {
        match event.payload {
            Some(pb::event::Payload::Session(session)) => {
                // The proto has no "gone" report: a deleted session sends
                // nothing and is dropped at the host's next snapshot.
                if event.change == pb::Change::Deleted as i32 {
                    continue;
                }
                if send_reports(&to_host, &session).await.is_err() {
                    return;
                }
            }
            Some(pb::event::Payload::Resync(_)) => {
                let Ok(listed) = list_all(&mut sessions).await else {
                    return;
                };
                for session in &listed {
                    if send_reports(&to_host, session).await.is_err() {
                        return;
                    }
                }
            }
            _ => {}
        }
    }
}

/// Opens the socket daemon's PeerUplink on the link's proven connection,
/// with the host's messages going in on `inbox`. None when it can't be
/// opened (the link has no proven connection, or the daemon refused it): a
/// line says so, and the link carries on without the daemon.
async fn open_daemon_uplink(
    relay: &Relay,
    inbox: tokio::sync::mpsc::Receiver<pb::HostToMachine>,
) -> Option<Streaming<pb::MachineToHost>> {
    let opened = match relay.proven_channel() {
        Some(channel) => pb::daemon_service_client::DaemonServiceClient::new(channel)
            .peer_uplink(ReceiverStream::new(inbox))
            .await
            .map(Response::into_inner),
        None => Err(Status::unavailable(
            "the peer link has no proven connection to this machine's daemon: the daemon didn't accept the host's proof, or no daemon answered when the link started",
        )),
    };
    match opened {
        Ok(daemon) => Some(daemon),
        Err(failure) => {
            crate::log::say!(
                "humatad2: the peer link couldn't open its uplink on this machine's daemon, so the machine acts as if its host were away until the link is redialled: {}",
                failure.message()
            );
            None
        }
    }
}

/// The daemon's next PeerUplink message, or none ever when there is no
/// daemon, so that branch of `carry_uplink` never fires.
async fn next_daemon_message(
    daemon: &mut Option<Streaming<pb::MachineToHost>>,
) -> Result<Option<pb::MachineToHost>, Status> {
    match daemon {
        Some(stream) => stream.message().await,
        None => std::future::pending().await,
    }
}

/// Carries the host's messages to the daemon's PeerUplink and the daemon's
/// to the host, until the host's side ends. The host's end (or a failure of
/// it) returns, which drops `to_daemon` and so ends the daemon's requests.
/// The link outlives the machine's daemon: a daemon that stops or restarts
/// ends its PeerUplink, but not the host's Uplink, so the host doesn't see
/// the machine lost (the daemon counts the host away until the next link).
/// Only a newer link replacing this one (`Aborted`) ends the host's Uplink,
/// with that status.
async fn carry_uplink(
    mut from_host: Streaming<pb::HostToMachine>,
    to_daemon: tokio::sync::mpsc::Sender<pb::HostToMachine>,
    mut daemon: Option<Streaming<pb::MachineToHost>>,
    to_host: &tokio::sync::mpsc::Sender<Result<pb::MachineToHost, Status>>,
) {
    loop {
        tokio::select! {
            host = from_host.message() => match host {
                Ok(Some(message)) => {
                    // With no daemon nothing reads `to_daemon`, so a send
                    // could wait forever once its buffer fills.
                    if daemon.is_some() {
                        let _ = to_daemon.send(message).await;
                    }
                }
                Ok(None) | Err(_) => return,
            },
            from_daemon = next_daemon_message(&mut daemon) => match from_daemon {
                Ok(Some(message)) => {
                    if to_host.send(Ok(message)).await.is_err() {
                        return;
                    }
                }
                Err(status) if status.code() == tonic::Code::Aborted => {
                    let _ = to_host.send(Err(status)).await;
                    return;
                }
                Ok(None) => daemon = None,
                Err(failure) => {
                    crate::log::say!(
                        "humatad2: this machine's daemon ended the peer link's uplink, so it counts its host away until the host links again; the link itself stays up: {}",
                        failure.message()
                    );
                    daemon = None;
                }
            },
        }
    }
}

#[cfg(test)]
mod uplink_tests {
    use super::*;

    fn session(id: &str, status: pb::SessionStatus, exit_code: i32, pointers: usize) -> pb::Session {
        pb::Session {
            id: Some(pb::SessionId { value: id.to_owned() }),
            status: status as i32,
            exit_code,
            pointers: vec![pb::Pointer::default(); pointers],
            ..pb::Session::default()
        }
    }

    #[test]
    fn a_session_is_reported_by_its_status_and_exit_code() {
        let reports = session_reports(&session("se_1", pb::SessionStatus::Exited, 3, 0), attach::now());
        assert_eq!(reports.len(), 1);
        let Some(pb::machine_to_host::Message::Status(report)) = &reports[0].message else {
            panic!("a status report");
        };
        assert_eq!(report.session.as_ref().map(|id| id.value.as_str()), Some("se_1"));
        assert_eq!(report.status, pb::SessionStatus::Exited as i32);
        assert_eq!(report.exit_code, 3);
    }

    #[test]
    fn pointers_follow_the_status_only_when_the_session_has_some() {
        let reports = session_reports(&session("se_1", pb::SessionStatus::Working, 0, 2), attach::now());
        assert_eq!(reports.len(), 2);
        let Some(pb::machine_to_host::Message::Pointers(report)) = &reports[1].message else {
            panic!("a pointers report");
        };
        assert_eq!(report.pointers.len(), 2);
    }

    #[test]
    fn a_session_with_no_id_is_not_reported() {
        assert!(session_reports(&pb::Session::default(), attach::now()).is_empty());
    }
}
