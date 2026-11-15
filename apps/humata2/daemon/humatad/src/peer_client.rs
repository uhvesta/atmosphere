//! The host's end of the peer link (GRPC.md §2, §8): it dials a machine's
//! `humatad2 serve --stdio`, proves both ends hold the machine's enrollment
//! key (Hello, then ProveHost), opens the link's one Uplink, and keeps the
//! link up, dialling again with backoff when it drops (HO-6).
//!
//! A dead link is noticed only by the keepalive (TR-9): an HTTP/2 PING
//! after 30 s with nothing read, and the link ends when one goes 20 s
//! unanswered. The machine's end runs the same timers (`peer_link`). An
//! idle link runs no other timer; a down one waits 5 s, then 10, 20 and 30
//! at most between dials (HO-5).
//!
//! The host keeps its copy of each machine's key as hex in a 0600 file,
//! `peer-keys/<machine>` in its state folder, read with the machine's own
//! checks (`link_proof`). Until the Mac writes the key at enrollment, a
//! machine with no such file gets no link.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll};
use std::time::Duration;

use humata2_grpc::humata::v1 as pb;
use hyper_util::rt::TokioIo;
use pb::peer_service_client::PeerServiceClient;
use pb::peer_unproven::Reason;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::Status;
use tonic::transport::{Channel, Endpoint, Uri};

use crate::daemon::{API_MAJOR, API_MINOR};
use crate::link_proof::{EnrollmentKey, HOST_NONCE_MIN, KeyError, MACHINE_NONCE_BYTES, Side};
use crate::status;

/// The folder of the host's copies of machines' keys, in its state folder.
pub(crate) const KEYS_DIR: &str = "peer-keys";

/// A PING after the link has read nothing for this long (TR-9, D54).
const KEEPALIVE: Duration = Duration::from_secs(30);
/// A PING unanswered this long ends the link.
const KEEPALIVE_TIMEOUT: Duration = Duration::from_secs(20);

/// The first wait before dialling again, doubled after each failure.
const RETRY_FIRST: Duration = Duration::from_secs(5);
/// The longest wait between dials (HO-5: every 5–30 s).
const RETRY_MOST: Duration = Duration::from_secs(30);

/// How many minor versions behind the host a machine may be (GRPC.md §8).
const MINOR_BEHIND: u32 = 2;

/// A byte stream carrying one link: a child's stdio, or a test's pipe.
pub(crate) trait Duplex: AsyncRead + AsyncWrite + Send + Unpin + 'static {}

impl<T: AsyncRead + AsyncWrite + Send + Unpin + 'static> Duplex for T {}

pub(crate) type Pipe = Box<dyn Duplex>;

/// Opens a fresh byte stream to a machine's `serve --stdio`.
pub(crate) trait Dial: Send + Sync {
    fn dial(&self) -> Pin<Box<dyn Future<Output = Result<Pipe, LinkError>> + Send + '_>>;
}

/// Dials by running the machine's link command (`Link::peer`): SSH or the
/// provider's connect command, whose stdio is the link.
pub(crate) struct CommandDial {
    machine: String,
    invocation: humata2_machines::Invocation,
}

impl CommandDial {
    pub(crate) fn new(machine: String, invocation: humata2_machines::Invocation) -> Self {
        Self { machine, invocation }
    }

    fn spawn(&self) -> Result<Pipe, LinkError> {
        let spawn_failed = |source| LinkError::Spawn {
            machine: self.machine.clone(),
            program: self.invocation.program.clone(),
            source,
        };
        let mut command = tokio::process::Command::new(&self.invocation.program);
        command
            .args(&self.invocation.args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        for (name, value) in &self.invocation.env {
            match value {
                humata2_machines::EnvValue::Value(value) => {
                    command.env(name, value);
                }
                humata2_machines::EnvValue::HostVariable(variable) => {
                    return Err(LinkError::HostVariable {
                        machine: self.machine.clone(),
                        variable: variable.clone(),
                    });
                }
            }
        }
        let mut child = command.spawn().map_err(spawn_failed)?;
        let (Some(stdout), Some(stdin)) = (child.stdout.take(), child.stdin.take()) else {
            return Err(spawn_failed(std::io::Error::other("the link command has no stdin or stdout")));
        };
        Ok(Box::new(ChildPipe {
            _child: child,
            io: tokio::io::join(stdout, stdin),
        }))
    }
}

impl Dial for CommandDial {
    fn dial(&self) -> Pin<Box<dyn Future<Output = Result<Pipe, LinkError>> + Send + '_>> {
        Box::pin(async move { self.spawn() })
    }
}

/// A link command's stdout and stdin; dropping it kills the command.
struct ChildPipe {
    _child: Child,
    io: tokio::io::Join<ChildStdout, ChildStdin>,
}

impl AsyncRead for ChildPipe {
    fn poll_read(mut self: Pin<&mut Self>, context: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.io).poll_read(context, buf)
    }
}

impl AsyncWrite for ChildPipe {
    fn poll_write(mut self: Pin<&mut Self>, context: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.io).poll_write(context, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.io).poll_flush(context)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.io).poll_shutdown(context)
    }
}

/// A link that is up: both ends proved, the Uplink open.
pub(crate) struct Up {
    machine: String,
    daemon_version: String,
    from_machine: tonic::Streaming<pb::MachineToHost>,
    /// Held so the host's side of the Uplink stays open.
    _to_machine: mpsc::Sender<pb::HostToMachine>,
    _channel: Channel,
}

impl Up {
    /// The machine's humatad2 version, as its Hello said.
    pub(crate) fn daemon_version(&self) -> &str {
        &self.daemon_version
    }

    /// Resolves when the link ends, with why. What the machine sends on
    /// the Uplink isn't acted on yet: no journal or mirror is kept.
    pub(crate) async fn closed(mut self) -> LinkError {
        loop {
            match self.from_machine.message().await {
                Ok(Some(_message)) => {}
                Ok(None) => return LinkError::Ended { machine: self.machine },
                Err(source) => {
                    return LinkError::Lost {
                        machine: self.machine,
                        source,
                    };
                }
            }
        }
    }
}

/// Speaks gRPC over `pipe`, runs the handshake under `key`, and opens the
/// Uplink.
pub(crate) async fn open(pipe: Pipe, key: &EnrollmentKey, machine: &str) -> Result<Up, LinkError> {
    let channel = channel_over(pipe, machine).await?;
    let mut peer = PeerServiceClient::new(channel.clone());
    let call_failed = |step: &'static str| {
        move |source: Status| LinkError::Call {
            machine: machine.to_owned(),
            step,
            source,
        }
    };

    let mut nonce = vec![0u8; HOST_NONCE_MIN];
    rand::fill(&mut nonce[..]);
    let hello = peer
        .hello(pb::HelloRequest {
            expected_machine: Some(pb::MachineId {
                value: machine.to_owned(),
            }),
            host_api_major: API_MAJOR,
            host_api_minor: API_MINOR,
            nonce: nonce.clone(),
            sync_marker: String::new(),
            chunk_codecs: Vec::new(),
        })
        .await
        .map_err(|source| refused_or(source, machine, call_failed("Hello")))?
        .into_inner();
    if hello.api_major != API_MAJOR || hello.api_minor.saturating_add(MINOR_BEHIND) < API_MINOR {
        return Err(LinkError::VersionUnsupported {
            machine: machine.to_owned(),
            version: hello.daemon_version,
            major: hello.api_major,
            minor: hello.api_minor,
        });
    }
    if hello.machine_nonce.len() != MACHINE_NONCE_BYTES
        || !key.verify(Side::Machine, &nonce, &hello.machine_nonce, &hello.proof)
    {
        return Err(LinkError::MachineProofWrong {
            machine: machine.to_owned(),
        });
    }
    peer.prove_host(pb::ProveHostRequest {
        proof: key.proof(Side::Host, &nonce, &hello.machine_nonce),
    })
    .await
    .map_err(|source| refused_or(source, machine, call_failed("ProveHost")))?;

    let (to_machine, outgoing) = mpsc::channel(16);
    // Nothing is stored yet, so the host resumes from nothing.
    let resume = pb::HostToMachine {
        message: Some(pb::host_to_machine::Message::Resume(pb::UplinkResume {
            from: Some(pb::UplinkAck::default()),
        })),
    };
    if to_machine.send(resume).await.is_err() {
        return Err(LinkError::Ended {
            machine: machine.to_owned(),
        });
    }
    let mut from_machine = peer
        .uplink(ReceiverStream::new(outgoing))
        .await
        .map_err(call_failed("Uplink"))?
        .into_inner();
    match from_machine.message().await.map_err(call_failed("Uplink"))? {
        Some(pb::MachineToHost {
            message: Some(pb::machine_to_host::Message::Resume(_)),
        }) => {}
        _ => {
            return Err(LinkError::UplinkOutOfOrder {
                machine: machine.to_owned(),
            });
        }
    }
    Ok(Up {
        machine: machine.to_owned(),
        daemon_version: hello.daemon_version,
        from_machine,
        _to_machine: to_machine,
        _channel: channel,
    })
}

/// One HTTP/2 connection over `pipe`, with the link's keepalive.
async fn channel_over(pipe: Pipe, machine: &str) -> Result<Channel, LinkError> {
    let pipe = Arc::new(Mutex::new(Some(pipe)));
    Endpoint::from_static("http://peer.link")
        .http2_keep_alive_interval(KEEPALIVE)
        .keep_alive_timeout(KEEPALIVE_TIMEOUT)
        .keep_alive_while_idle(true)
        .connect_with_connector(tower::service_fn(move |_: Uri| {
            let pipe = pipe.lock().unwrap_or_else(PoisonError::into_inner).take();
            async move {
                pipe.map(TokioIo::new)
                    .ok_or_else(|| std::io::Error::other("a peer link carries one connection"))
            }
        }))
        .await
        .map_err(|source| LinkError::Connect {
            machine: machine.to_owned(),
            source,
        })
}

/// The machine's `peer_unproven` refusal as its own error, or `otherwise`.
fn refused_or(source: Status, machine: &str, otherwise: impl FnOnce(Status) -> LinkError) -> LinkError {
    match status::error_of(&source).and_then(|error| error.kind) {
        Some(pb::error::Kind::PeerUnproven(refused)) => LinkError::MachineRefused {
            machine: machine.to_owned(),
            reason: refused.reason(),
            source,
        },
        _ => otherwise(source),
    }
}

/// What the link did, for whoever shows the machine's state.
pub(crate) enum LinkEvent<'a> {
    Up { daemon_version: &'a str },
    Down { error: &'a LinkError },
}

pub(crate) type Report = Box<dyn Fn(LinkEvent<'_>) + Send + Sync>;

/// Keeps `machine`'s link up: dials, opens it, and when it drops or a dial
/// fails, reports it and dials again after a backoff. Returns only when a
/// failure that another dial can't fix (a wrong key, an old machine) ends
/// it; the user's Reconnect starts it again.
pub(crate) async fn supervise(dial: Box<dyn Dial>, key_file: PathBuf, machine: String, report: Report) -> LinkError {
    let mut wait = RETRY_FIRST;
    loop {
        let error = match connect(&*dial, &key_file, &machine).await {
            Ok(up) => {
                report(LinkEvent::Up {
                    daemon_version: up.daemon_version(),
                });
                wait = RETRY_FIRST;
                up.closed().await
            }
            Err(error) => error,
        };
        report(LinkEvent::Down { error: &error });
        if error.permanent() {
            return error;
        }
        tokio::time::sleep(wait).await;
        wait = (wait * 2).min(RETRY_MOST);
    }
}

async fn connect(dial: &dyn Dial, key_file: &Path, machine: &str) -> Result<Up, LinkError> {
    let key = EnrollmentKey::read(key_file).map_err(|source| LinkError::HostKey {
        machine: machine.to_owned(),
        source,
    })?;
    let pipe = dial.dial().await?;
    open(pipe, &key, machine).await
}

/// Why a peer link didn't come up or ended. `Display` says what failed;
/// `fix` says what to do.
#[derive(Debug, thiserror::Error)]
pub(crate) enum LinkError {
    #[error("the host has no usable enrollment key for {machine}")]
    HostKey {
        machine: String,
        #[source]
        source: KeyError,
    },

    #[error("couldn't start the peer link command `{program}` for {machine}")]
    Spawn {
        machine: String,
        program: String,
        #[source]
        source: std::io::Error,
    },

    #[error("the peer link command for {machine} needs host variable {variable}, which the link can't set yet")]
    HostVariable { machine: String, variable: String },

    #[error("couldn't speak gRPC over the peer link to {machine}")]
    Connect {
        machine: String,
        #[source]
        source: tonic::transport::Error,
    },

    #[error("{machine} refused the peer link ({reason:?})")]
    MachineRefused {
        machine: String,
        reason: Reason,
        #[source]
        source: Status,
    },

    #[error(
        "{machine} answered the peer link without proof that it holds the enrollment key on record: it was re-imaged, swapped, or isn't the machine that was added"
    )]
    MachineProofWrong { machine: String },

    #[error("{machine} runs humatad2 {version} (API {major}.{minor}), which this Mac's Humata can't talk to")]
    VersionUnsupported {
        machine: String,
        version: String,
        major: u32,
        minor: u32,
    },

    #[error("the peer link to {machine} failed at {step}")]
    Call {
        machine: String,
        step: &'static str,
        #[source]
        source: Status,
    },

    #[error("{machine}'s Uplink didn't start by saying where to resume")]
    UplinkOutOfOrder { machine: String },

    #[error("{machine} ended the peer link")]
    Ended { machine: String },

    #[error("the peer link to {machine} was lost")]
    Lost {
        machine: String,
        #[source]
        source: Status,
    },
}

impl LinkError {
    /// Whether dialling again can't help: the keys, the machine or its
    /// version must change first.
    pub(crate) fn permanent(&self) -> bool {
        match self {
            LinkError::HostKey { .. }
            | LinkError::HostVariable { .. }
            | LinkError::MachineProofWrong { .. }
            | LinkError::VersionUnsupported { .. }
            | LinkError::UplinkOutOfOrder { .. } => true,
            LinkError::MachineRefused { reason, .. } => *reason != Reason::OutOfOrder,
            LinkError::Spawn { .. }
            | LinkError::Connect { .. }
            | LinkError::Call { .. }
            | LinkError::Ended { .. }
            | LinkError::Lost { .. } => false,
        }
    }

    /// What the person can do about it.
    pub(crate) fn fix(&self) -> &'static str {
        match self {
            LinkError::HostKey { source, .. } => source.fix(),
            LinkError::Spawn { .. } => "check that ssh is installed and on PATH for humatad2; Humata dials again on its own",
            LinkError::HostVariable { .. } => {
                "connect this machine over plain SSH for now, or set the command up without host variables"
            }
            LinkError::Connect { .. } | LinkError::Call { .. } | LinkError::Ended { .. } | LinkError::Lost { .. } => {
                "nothing to do if the machine or the network is just down: its sessions keep running, and Humata dials again every 5 to 30 s. Use Reconnect now to try at once"
            }
            LinkError::MachineRefused { reason, .. } => match reason {
                Reason::NotEnrolled => {
                    "the machine has no enrollment key: its state was wiped or replaced. Forget the machine and add it again, which writes a new key"
                }
                Reason::KeyUnsafe => {
                    "the machine's key file could be read or replaced by someone else, so don't just fix its permissions: forget the machine and add it again, which writes a new key"
                }
                Reason::HostProofWrong => {
                    "the Mac's key and the machine's differ. Forget the machine and add it again; if you didn't change either, something may be posing as one of them"
                }
                _ => "check the machine's key file belongs to you and is readable, then use Reconnect now; if it is gone or damaged, forget the machine and add it again",
            },
            LinkError::MachineProofWrong { .. } => {
                "don't reconnect it as it is: if the machine was rebuilt, forget it and add it again as a new machine (LD-12); if it wasn't, something may be posing as it"
            }
            LinkError::VersionUnsupported { .. } => "update the machine's humatad2 with Reconnect now, which installs this Mac's build",
            LinkError::UplinkOutOfOrder { .. } => {
                "update the machine's humatad2 with Reconnect now; if it keeps failing, report it with the machine's peer-link.log"
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! Real `serve_link`s on in-memory pipes, on tokio's paused clock: time
    //! moves only when every task waits, so an hour of keepalives takes
    //! none. A relay between the two ends counts the host's PINGs and can
    //! stop passing anything from the machine, as a dead network does.

    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use humata2_grpc::humata::v1 as pb;
    use humata2_machines::{Bundle, Machines, MachinesConfig, Platform, RetryPolicy};
    use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
    use tokio::sync::mpsc;
    use tokio::time::Instant;

    use super::*;
    use crate::compression::Compression;
    use crate::events::Feed;
    use crate::link_proof;
    use crate::machine_service::Work;
    use crate::peer::Handshake;
    use crate::peer_link::serve_link;

    const MACHINE: &str = "ma_test";
    const HOUR: Duration = Duration::from_secs(3600);
    const PREFACE: usize = 24;
    const PING: u8 = 6;

    /// `byte` repeated as a 32-byte key, written as hex to `path`, 0600.
    fn write_key(path: &Path, byte: u8) -> PathBuf {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, format!("{byte:02x}").repeat(32)).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
        path.to_owned()
    }

    /// One dial's relay: how many PINGs the host sent through it, and
    /// whether it has stopped passing the machine's bytes.
    #[derive(Default)]
    struct Relay {
        pings: AtomicUsize,
        frozen: AtomicBool,
    }

    /// A machine served in this process, with its key in `state`.
    struct Machine {
        state: PathBuf,
        relays: Mutex<Vec<Arc<Relay>>>,
    }

    impl Machine {
        fn new(dir: &Path, key: Option<u8>) -> Arc<Self> {
            let state = dir.join("machine");
            std::fs::create_dir_all(&state).unwrap();
            if let Some(byte) = key {
                write_key(&state.join(link_proof::KEY_FILE), byte);
            }
            Arc::new(Self {
                state,
                relays: Mutex::new(Vec::new()),
            })
        }

        fn dials(&self) -> usize {
            self.relays.lock().unwrap().len()
        }

        fn relay(&self, dial: usize) -> Arc<Relay> {
            self.relays.lock().unwrap()[dial].clone()
        }

        fn pipe(&self) -> Pipe {
            let (host, relay_host) = tokio::io::duplex(64 * 1024);
            let (relay_machine, machine) = tokio::io::duplex(64 * 1024);
            let (input, output) = tokio::io::split(machine);
            let handshake = Arc::new(Handshake::new(
                self.state.join(link_proof::KEY_FILE),
                MACHINE.to_owned(),
                Arc::new(Compression::new(&self.state)),
            ));
            tokio::spawn(serve_link(input, output, handshake, MACHINE.to_owned(), std::future::pending()));
            let relay = Arc::new(Relay::default());
            self.relays.lock().unwrap().push(relay.clone());
            let (from_host, to_host) = tokio::io::split(relay_host);
            let (from_machine, to_machine) = tokio::io::split(relay_machine);
            tokio::spawn(host_to_machine(from_host, to_machine, relay.clone()));
            tokio::spawn(machine_to_host(from_machine, to_host, relay));
            Box::new(host)
        }
    }

    /// Passes the host's bytes on whole frames, counting PINGs (not acks).
    async fn host_to_machine(
        mut from: tokio::io::ReadHalf<DuplexStream>,
        mut to: tokio::io::WriteHalf<DuplexStream>,
        relay: Arc<Relay>,
    ) -> Option<()> {
        let mut preface = [0u8; PREFACE];
        from.read_exact(&mut preface).await.ok()?;
        to.write_all(&preface).await.ok()?;
        loop {
            let mut head = [0u8; 9];
            from.read_exact(&mut head).await.ok()?;
            let length = u32::from_be_bytes([0, head[0], head[1], head[2]]) as usize;
            let mut payload = vec![0; length];
            from.read_exact(&mut payload).await.ok()?;
            if head[3] == PING && head[4] & 1 == 0 {
                relay.pings.fetch_add(1, Ordering::SeqCst);
            }
            to.write_all(&head).await.ok()?;
            to.write_all(&payload).await.ok()?;
        }
    }

    /// Passes the machine's bytes until frozen; then nothing, not even
    /// its end, as a network that went dead.
    async fn machine_to_host(
        mut from: tokio::io::ReadHalf<DuplexStream>,
        mut to: tokio::io::WriteHalf<DuplexStream>,
        relay: Arc<Relay>,
    ) -> Option<()> {
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            let read = from.read(&mut buf).await.ok()?;
            if relay.frozen.load(Ordering::SeqCst) {
                std::future::pending::<()>().await;
            }
            if read == 0 {
                return None;
            }
            to.write_all(&buf[..read]).await.ok()?;
        }
    }

    struct InProcess(Arc<Machine>);

    impl Dial for InProcess {
        fn dial(&self) -> Pin<Box<dyn Future<Output = Result<Pipe, LinkError>> + Send + '_>> {
            Box::pin(async move { Ok(self.0.pipe()) })
        }
    }

    /// What the supervisor reported.
    #[derive(Debug, PartialEq)]
    enum Seen {
        Up,
        Down { permanent: bool },
    }

    fn recorder() -> (Report, mpsc::UnboundedReceiver<Seen>) {
        let (seen, events) = mpsc::unbounded_channel();
        let report: Report = Box::new(move |event| {
            let _ = seen.send(match event {
                LinkEvent::Up { .. } => Seen::Up,
                LinkEvent::Down { error } => Seen::Down {
                    permanent: error.permanent(),
                },
            });
        });
        (report, events)
    }

    fn host_key(dir: &Path, byte: u8) -> PathBuf {
        write_key(&dir.join(KEYS_DIR).join(MACHINE), byte)
    }

    #[tokio::test(start_paused = true)]
    async fn the_handshake_succeeds_when_both_ends_hold_the_same_key() {
        let dir = tempfile::tempdir().unwrap();
        let machine = Machine::new(dir.path(), Some(0xab));
        let key = EnrollmentKey::read(&host_key(dir.path(), 0xab)).unwrap();

        let up = open(machine.pipe(), &key, MACHINE).await.unwrap();

        assert!(!up.daemon_version().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_machine_with_another_key_is_refused_and_not_dialled_again() {
        let dir = tempfile::tempdir().unwrap();
        let machine = Machine::new(dir.path(), Some(0xab));
        let key = EnrollmentKey::read(&host_key(dir.path(), 0xcd)).unwrap();

        let refused = open(machine.pipe(), &key, MACHINE).await.err().unwrap();
        assert!(matches!(refused, LinkError::MachineProofWrong { .. }), "{refused:?}");
        assert!(refused.permanent());

        let (report, mut events) = recorder();
        let ended = tokio::time::timeout(
            HOUR,
            supervise(Box::new(InProcess(machine.clone())), host_key(dir.path(), 0xcd), MACHINE.into(), report),
        )
        .await
        .expect("a wrong key ends the supervisor");
        assert!(matches!(ended, LinkError::MachineProofWrong { .. }), "{ended:?}");
        assert_eq!(events.recv().await, Some(Seen::Down { permanent: true }));
        assert_eq!(machine.dials(), 2, "one dial by `open`, one by the supervisor");
    }

    #[tokio::test(start_paused = true)]
    async fn a_machine_with_no_key_refuses_as_not_enrolled() {
        let dir = tempfile::tempdir().unwrap();
        let machine = Machine::new(dir.path(), None);
        let key = EnrollmentKey::read(&host_key(dir.path(), 0xab)).unwrap();

        let refused = open(machine.pipe(), &key, MACHINE).await.err().unwrap();

        assert!(
            matches!(refused, LinkError::MachineRefused { reason: Reason::NotEnrolled, .. }),
            "{refused:?}"
        );
        assert!(refused.permanent());
    }

    #[tokio::test(start_paused = true)]
    async fn an_idle_link_pings_and_stays_up() {
        let dir = tempfile::tempdir().unwrap();
        let machine = Machine::new(dir.path(), Some(0xab));
        let (report, mut events) = recorder();
        tokio::spawn(supervise(
            Box::new(InProcess(machine.clone())),
            host_key(dir.path(), 0xab),
            MACHINE.into(),
            report,
        ));
        assert_eq!(events.recv().await, Some(Seen::Up));

        let quiet = tokio::time::timeout(HOUR, events.recv()).await;

        assert!(quiet.is_err(), "the link changed: {quiet:?}");
        let pings = machine.relay(0).pings.load(Ordering::SeqCst);
        assert!(pings >= 1, "the host sends keepalive PINGs");
        assert!(pings <= 120, "{pings} PINGs in an hour");
        assert_eq!(machine.dials(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_link_with_no_answers_ends_and_is_dialled_again() {
        let dir = tempfile::tempdir().unwrap();
        let machine = Machine::new(dir.path(), Some(0xab));
        let (report, mut events) = recorder();
        tokio::spawn(supervise(
            Box::new(InProcess(machine.clone())),
            host_key(dir.path(), 0xab),
            MACHINE.into(),
            report,
        ));
        assert_eq!(events.recv().await, Some(Seen::Up));

        machine.relay(0).frozen.store(true, Ordering::SeqCst);
        let frozen_at = Instant::now();
        let down = tokio::time::timeout(Duration::from_secs(120), events.recv()).await;
        assert_eq!(down, Ok(Some(Seen::Down { permanent: false })));
        assert!(
            frozen_at.elapsed() <= KEEPALIVE + KEEPALIVE_TIMEOUT + Duration::from_secs(1),
            "noticed after {:?}",
            frozen_at.elapsed()
        );

        let again = tokio::time::timeout(Duration::from_secs(60), events.recv()).await;
        assert_eq!(again, Ok(Some(Seen::Up)));
        assert_eq!(machine.dials(), 2);
    }

    /// The reachability of `machine` in the next machine event on `live`,
    /// waiting at most two minutes of the test clock.
    async fn next_reach(live: &mut tokio::sync::broadcast::Receiver<Arc<pb::Event>>) -> pb::Reachability {
        tokio::time::timeout(Duration::from_secs(120), next_machine_event(live))
            .await
            .expect("the machine's reachability changes within two minutes")
    }

    async fn next_machine_event(live: &mut tokio::sync::broadcast::Receiver<Arc<pb::Event>>) -> pb::Reachability {
        loop {
            let event = live.recv().await.unwrap();
            if let Some(pb::event::Payload::Machine(machine)) = &event.payload
                && machine.name == MACHINE
            {
                return machine.reachability();
            }
        }
    }

    #[tokio::test(start_paused = true)]
    async fn the_feed_tells_when_the_link_comes_up_and_when_it_is_lost() {
        let dir = tempfile::tempdir().unwrap();
        let hosts = dir.path().join("hosts.json");
        std::fs::write(
            &hosts,
            format!(
                r#"{{"version":1,"machines":[{{"name":"{MACHINE}","ssh_host":"user@devbox","root":"~/Humata","state":"ready"}}]}}"#
            ),
        )
        .unwrap();
        let machines = Machines::open(MachinesConfig {
            hosts_file: hosts,
            known_hosts: dir.path().join("known_hosts"),
            ssh_config: dir.path().join("ssh_config"),
            retry: RetryPolicy::default(),
        });
        let builds = Box::new(Bundle::beside(dir.path(), Platform::host().unwrap()));
        let feed = Arc::new(Feed::default());
        let (_, mut live) = feed.subscribe(None);
        let work = Arc::new(Work::new(machines, builds, feed, dir.path().to_owned()));
        let machine = Machine::new(dir.path(), Some(0xab));
        host_key(dir.path(), 0xab);

        work.start_link(MACHINE, Box::new(InProcess(machine.clone())), &tokio::runtime::Handle::current());
        assert_eq!(next_reach(&mut live).await, pb::Reachability::Online);

        machine.relay(0).frozen.store(true, Ordering::SeqCst);
        assert_eq!(next_reach(&mut live).await, pb::Reachability::Offline);
        assert_eq!(next_reach(&mut live).await, pb::Reachability::Online);
    }
}
