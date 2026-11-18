//! Starts a real `humatad2` on a socket of its own, connects to it over
//! gRPC, and stops it and the holders it started when the test ends. By
//! default it is the admin test daemon run with `--admin`, so calls with no
//! credential act as the user; `Build::Release` is the daemon as shipped.

#![allow(dead_code)]

use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use humata2_grpc::humata::v1 as pb;
use hyper_util::rt::TokioIo;
use prost::Message;
use rustix::process::{Pid, Signal};
use tempfile::TempDir;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::UnixStream;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::{Channel, Endpoint, Uri};
use tonic::{Code, Status, Streaming};

pub const WAIT: Duration = Duration::from_secs(15);

/// Runs the daemon's binary once, to its help text, before this test
/// process starts its first daemon. This is not slack for a loaded
/// machine: [`WAIT`] still bounds every daemon's own start. It moves one
/// cost that isn't the daemon's out of that bound. On macOS the first
/// exec of a binary that was just linked and isn't signed (about 95 MB
/// here) is assessed by the system before it runs (syspolicyd and
/// XProtect read the whole file), once per new binary. A test process
/// starts as many daemons at once as the machine has cores, so the whole
/// first batch waited on that one assessment and could spend all of
/// [`WAIT`] there before any of the daemon's code ran, while every later
/// start was quick (gate g70: the first ten tests failed, the next eleven
/// passed in five seconds).
fn warm(build: Build, faulty: bool) {
    static ADMIN: OnceLock<()> = OnceLock::new();
    static RELEASE: OnceLock<()> = OnceLock::new();
    static FAULTS: OnceLock<()> = OnceLock::new();
    let (once, variable) = match build {
        Build::Admin | Build::AdminForThisProcess if faulty => (&FAULTS, "HUMATAD2_FAULTS"),
        Build::Admin | Build::AdminForThisProcess => (&ADMIN, "HUMATAD2"),
        // A box that is asked to take faults runs the faults daemon
        // without `--admin` (D240).
        Build::Release if faulty => (&FAULTS, "HUMATAD2_FAULTS"),
        Build::Release => (&RELEASE, "HUMATAD2_RELEASE"),
    };
    once.get_or_init(|| {
        let _ = Command::new(binary(variable)).arg("--help").output();
    });
}

/// The variables that ask for the fault build (`humatad2_faults` and
/// `humata2_holder_faults`): a start with any of them in its environment
/// runs those, so every point of the daemon and of its crates is live.
const FAULT_VARIABLES: [&str; 5] = [
    "HUMATA2_FAIL_AT",
    "HUMATA2_STOP_AT",
    "HUMATA2_HOLD_AT",
    "HUMATA2_POWER_CUT_ROOTS",
    "HUMATA2_FAULT_TRACE",
];

/// Whether `env` asks for the fault build.
pub fn wants_faults(env: &[(String, String)]) -> bool {
    env.iter().any(|(name, _)| FAULT_VARIABLES.contains(&name.as_str()))
}

pub fn binary(variable: &str) -> PathBuf {
    let path = std::env::var_os(variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("set {variable} to the binary's path"));
    std::fs::canonicalize(path).unwrap()
}

/// Whether the test numbered `index` is this Bazel shard's. rules_rust
/// doesn't shard a test binary, so without this every shard runs every
/// test: the test runs in shard `index % TEST_TOTAL_SHARDS`, and the others
/// pass it at once. Writing the status file tells Bazel the binary shards.
/// Outside Bazel's sharding every test runs.
pub fn in_this_shard(index: usize) -> bool {
    if let Some(status) = std::env::var_os("TEST_SHARD_STATUS_FILE") {
        std::fs::write(status, b"").expect("the shard status file is written");
    }
    let read = |name: &str| std::env::var(name).ok().and_then(|value| value.parse::<usize>().ok());
    match (read("TEST_TOTAL_SHARDS"), read("TEST_SHARD_INDEX")) {
        (Some(total), Some(shard)) if total > 0 => index % total == shard,
        _ => true,
    }
}

/// A folder for a daemon: its socket and state go in here. Under /tmp,
/// since socket paths must be short.
pub fn folder() -> TempDir {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::Builder::new()
        .prefix("h2d")
        .tempdir_in("/tmp")
        .unwrap();
    // The daemon refuses a socket folder other users can open.
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    dir
}

/// The state folder a daemon in `dir` uses, made the way the daemon wants
/// it, for a test to seed before the daemon starts.
pub fn state_folder(dir: &Path) -> PathBuf {
    let state = dir.join("state");
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&state)
        .unwrap();
    state
}

/// Which humatad2 a test runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Build {
    /// The admin test daemon, run with `--admin`.
    Admin,
    /// The release daemon: a call with no credential is refused.
    Release,
    /// The admin test daemon run with `--admin-pid` for this test's own
    /// process: the test plays the app, and every other caller (a `humata`
    /// it starts, say) is judged as the release daemon would.
    AdminForThisProcess,
}

pub struct Daemon {
    child: Option<Child>,
    build: Build,
    /// More `serve` arguments, as `--cli-login-seconds 2`.
    extra: Vec<String>,
    /// Environment variables for the next launch only, as
    /// `HUMATA2_STOP_AT`: a restart starts without them.
    env_once: Vec<(String, String)>,
    /// The holder file every launch spawns holders from, when a test
    /// chooses it (an installed `bin/humata2_holder`, which an upgrade
    /// replaces); unset, the test holder.
    holder: Option<PathBuf>,
    pub socket: PathBuf,
    pub state: PathBuf,
    pub dir: TempDir,
}

impl Daemon {
    pub async fn start() -> Self {
        Self::start_in(folder()).await
    }

    pub async fn start_in(dir: TempDir) -> Self {
        Self::start_build(Build::Admin, dir).await
    }

    pub async fn start_build(build: Build, dir: TempDir) -> Self {
        Self::start_with(build, dir, &[]).await
    }

    /// As [`Self::start_build`], with more `serve` arguments.
    pub async fn start_with(build: Build, dir: TempDir, extra: &[&str]) -> Self {
        Self::launched(build, dir, extra, &[]).await
    }

    /// As [`Self::start_with`], spawning holders from `holder` at this
    /// start and every restart: a test that upgrades the daemon names the
    /// installed `bin/humata2_holder`, so holders started after the upgrade
    /// are the new release's.
    pub async fn start_with_holder(build: Build, dir: TempDir, extra: &[&str], holder: &Path) -> Self {
        let mut daemon = Self::unlaunched(build, dir, extra, &[]);
        daemon.holder = Some(holder.to_path_buf());
        daemon.launch().await;
        daemon
    }

    /// The admin test daemon with more environment variables, such as
    /// `HUMATA2_STOP_AT`, for this start only: [`Self::restart`] starts it
    /// again without them.
    pub async fn start_env(dir: TempDir, env: &[(&str, &str)]) -> Self {
        Self::launched(Build::Admin, dir, &[], env).await
    }

    /// [`Self::start_with`] with environment variables for this start only.
    pub async fn start_with_env(
        build: Build,
        dir: TempDir,
        extra: &[&str],
        env: &[(&str, &str)],
    ) -> Self {
        Self::launched(build, dir, extra, env).await
    }

    async fn launched(build: Build, dir: TempDir, extra: &[&str], env: &[(&str, &str)]) -> Self {
        let mut daemon = Self::unlaunched(build, dir, extra, env);
        daemon.launch().await;
        daemon
    }

    fn unlaunched(build: Build, dir: TempDir, extra: &[&str], env: &[(&str, &str)]) -> Self {
        Self {
            child: None,
            build,
            extra: extra.iter().map(|arg| (*arg).to_owned()).collect(),
            env_once: env
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
            holder: None,
            socket: dir.path().join("d.sock"),
            state: dir.path().join("state"),
            dir,
        }
    }

    async fn launch(&mut self) {
        let faulty = wants_faults(&self.env_once);
        warm(self.build, faulty);
        let mut command = match self.build {
            Build::Admin | Build::AdminForThisProcess if faulty => Command::new(binary("HUMATAD2_FAULTS")),
            Build::Admin | Build::AdminForThisProcess => Command::new(binary("HUMATAD2")),
            // A box asked to take faults (D240): the faults daemon, started
            // without `--admin`, serves as the daemon as shipped does.
            Build::Release if faulty => Command::new(binary("HUMATAD2_FAULTS")),
            Build::Release => Command::new(binary("HUMATAD2_RELEASE")),
        };
        // The faults holder whenever the daemon is the faults build, a
        // box's too: a power cut (SIGUSR2) has to reach its holders (D243).
        let holder = self.holder.clone().unwrap_or_else(|| {
            binary(if faulty {
                "HUMATA2_HOLDER_FAULTS"
            } else {
                "HUMATA2_HOLDER"
            })
        });
        command.arg("serve");
        match self.build {
            Build::Admin => {
                command.arg("--admin");
            }
            Build::AdminForThisProcess => {
                command.arg(format!("--admin-pid={}", std::process::id()));
            }
            Build::Release => {}
        }
        command.args(&self.extra);
        command.envs(self.env_once.drain(..));
        let child = command
            .arg("--socket")
            .arg(&self.socket)
            .arg("--state")
            .arg(&self.state)
            .arg("--root")
            .arg(self.dir.path())
            .arg("--holder")
            .arg(holder)
            .arg("--shell")
            .arg("/bin/sh")
            .arg("--shell-arg=-i")
            .env("PS1", "$ ")
            .env("ENV", "/dev/null")
            .spawn()
            .unwrap();
        self.child = Some(child);
        let deadline = Instant::now() + WAIT;
        while UnixStream::connect(&self.socket).await.is_err() {
            if Instant::now() >= deadline {
                // Tells a daemon that gave up from one that is still starting.
                let process = match self.child.as_mut().map(Child::try_wait) {
                    Some(Ok(Some(status))) => format!("it exited with {status}"),
                    Some(Ok(None)) => "its process is still starting".to_owned(),
                    _ => "its process couldn't be asked".to_owned(),
                };
                panic!("humatad2 never started listening within {WAIT:?}: {process}");
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// Waits until the daemon's process ends by itself (a stop point killed
    /// it), however long the work before that point takes; `limit` is a
    /// safety net, as long as that work can take on a loaded machine.
    #[allow(dead_code)]
    pub async fn stopped(&mut self, limit: Duration) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        let waited = tokio::time::timeout(limit, tokio::task::spawn_blocking(move || child.wait())).await;
        assert!(waited.is_ok(), "humatad2 never stopped within {limit:?}");
        let deadline = Instant::now() + WAIT;
        while std::os::unix::net::UnixStream::connect(&self.socket).is_ok() {
            assert!(Instant::now() < deadline, "a stopped humatad2's socket never stopped answering");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// Ends the daemon at once, as a crash would.
    pub fn kill(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
            // On macOS a killed daemon's socket still takes connections
            // for a few milliseconds after it is reaped. A daemon started
            // then would take it for a live one and stop, and a client
            // would reach the dying one, so wait until it refuses.
            let deadline = Instant::now() + WAIT;
            while std::os::unix::net::UnixStream::connect(&self.socket).is_ok() {
                assert!(
                    Instant::now() < deadline,
                    "a killed humatad2's socket never stopped answering"
                );
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }

    pub async fn restart(&mut self) {
        self.kill();
        self.launch().await;
    }

    /// As [`Self::restart`], with environment variables such as
    /// `HUMATA2_STOP_AT` for this start only.
    pub async fn restart_env(&mut self, env: &[(&str, &str)]) {
        self.kill();
        self.env_once = env
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        self.launch().await;
    }

    /// As [`Self::restart_env`], handing the new run a `--ready-fd` pipe as
    /// the app's bridge does (D398), and returning what it says there: up to
    /// a newline, or what came before the pipe closed.
    #[allow(dead_code)]
    pub async fn restart_hearing_ready(&mut self, env: &[(&str, &str)]) -> Vec<u8> {
        use std::io::Read;
        use std::os::fd::{AsRawFd, BorrowedFd};
        use std::os::unix::process::CommandExt;
        self.kill();
        let env: Vec<(String, String)> = env
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        let faulty = wants_faults(&env);
        warm(self.build, faulty);
        let mut command = match self.build {
            Build::Admin | Build::AdminForThisProcess if faulty => Command::new(binary("HUMATAD2_FAULTS")),
            Build::Admin | Build::AdminForThisProcess => Command::new(binary("HUMATAD2")),
            Build::Release if faulty => Command::new(binary("HUMATAD2_FAULTS")),
            Build::Release => Command::new(binary("HUMATAD2_RELEASE")),
        };
        let holder = self.holder.clone().unwrap_or_else(|| {
            binary(if faulty {
                "HUMATA2_HOLDER_FAULTS"
            } else {
                "HUMATA2_HOLDER"
            })
        });
        command.arg("serve");
        match self.build {
            Build::Admin => {
                command.arg("--admin");
            }
            Build::AdminForThisProcess => {
                command.arg(format!("--admin-pid={}", std::process::id()));
            }
            Build::Release => {}
        }
        command.args(&self.extra);
        command.envs(env);
        let (mut reader, writer) = std::io::pipe().expect("make the ready pipe");
        let fd = writer.as_raw_fd();
        command.arg(format!("--ready-fd={fd}"));
        // SAFETY: only fcntl runs between fork and exec.
        unsafe {
            command.pre_exec(move || {
                rustix::io::fcntl_setfd(BorrowedFd::borrow_raw(fd), rustix::io::FdFlags::empty())?;
                Ok(())
            });
        }
        let child = command
            .arg("--socket")
            .arg(&self.socket)
            .arg("--state")
            .arg(&self.state)
            .arg("--root")
            .arg(self.dir.path())
            .arg("--holder")
            .arg(holder)
            .arg("--shell")
            .arg("/bin/sh")
            .arg("--shell-arg=-i")
            .env("PS1", "$ ")
            .env("ENV", "/dev/null")
            .spawn()
            .unwrap();
        drop(writer);
        self.child = Some(child);
        let heard = tokio::task::spawn_blocking(move || {
            let mut got = Vec::new();
            let mut byte = [0u8; 1];
            while let Ok(1) = reader.read(&mut byte) {
                got.push(byte[0]);
                if byte[0] == b'\n' {
                    break;
                }
            }
            got
        });
        let got = tokio::time::timeout(WAIT, heard)
            .await
            .unwrap_or_else(|_| panic!("humatad2 neither said ready nor closed its pipe within {WAIT:?}"))
            .expect("the pipe's reader ran");
        got
    }

    /// Stops this daemon and starts `build` on the same socket and state.
    pub async fn restart_as(&mut self, build: Build) {
        self.kill();
        self.build = build;
        self.launch().await;
    }

    pub async fn channel(&self) -> Channel {
        connect(&self.socket).await
    }

    pub async fn terminals(&self) -> pb::terminal_service_client::TerminalServiceClient<Channel> {
        pb::terminal_service_client::TerminalServiceClient::new(self.channel().await)
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        self.kill();
        // Holders outlive the daemon on purpose; these ones don't.
        if let Ok(store) = humata2_store::Store::open(self.state.join("store")) {
            for tab in store.tabs().unwrap_or_default() {
                if let Some(pid) = i32::try_from(tab.holder_pid).ok().and_then(Pid::from_raw) {
                    let _ = rustix::process::kill_process(pid, Signal::TERM);
                }
            }
        }
        // A holder no tab names (a power cut before its tab was committed,
        // or a tab already gone) still carries this daemon's folder, a
        // fresh one under /tmp, in its command line: stop those by it.
        let _ = Command::new("pkill").arg("-f").arg(format!("{}/", self.dir.path().display())).status();
    }
}

pub async fn connect(socket: &Path) -> Channel {
    try_connect(socket).await.unwrap()
}

/// A channel to the daemon on `socket`, or the error while no daemon
/// listens there, such as between two runs of one that restarts itself.
pub async fn try_connect(socket: &Path) -> Result<Channel, tonic::transport::Error> {
    let socket = socket.to_owned();
    Endpoint::try_from("http://[::]:50051")
        .unwrap()
        .connect_with_connector(tower::service_fn(move |_: Uri| {
            let socket = socket.clone();
            async move { Ok::<_, std::io::Error>(TokioIo::new(UnixStream::connect(socket).await?)) }
        }))
        .await
}

/// A gRPC channel over one byte stream, as the Mac's connector makes over
/// a link's stdio: a child's stdout to read and its stdin to write.
pub async fn channel_over(
    stdout: impl AsyncRead + Send + Unpin + 'static,
    stdin: impl AsyncWrite + Send + Unpin + 'static,
) -> Channel {
    let stdio = Arc::new(Mutex::new(Some(tokio::io::join(stdout, stdin))));
    Endpoint::try_from("http://[::]:50051")
        .unwrap()
        .connect_with_connector(tower::service_fn(move |_: Uri| {
            let stdio = stdio.lock().unwrap().take();
            async move {
                stdio
                    .map(TokioIo::new)
                    .ok_or_else(|| std::io::Error::other("a stdio link carries one connection"))
            }
        }))
        .await
        .unwrap()
}

#[derive(Clone, PartialEq, Message)]
struct RpcStatus {
    #[prost(int32, tag = "1")]
    code: i32,
    #[prost(string, tag = "2")]
    message: String,
    #[prost(message, repeated, tag = "3")]
    details: Vec<Any>,
}

#[derive(Clone, PartialEq, Message)]
struct Any {
    #[prost(string, tag = "1")]
    type_url: String,
    #[prost(bytes = "vec", tag = "2")]
    value: Vec<u8>,
}

/// The Humata error a failed call carries.
pub fn error_of(status: &Status) -> pb::Error {
    let rpc = RpcStatus::decode(status.details()).unwrap();
    let any = rpc
        .details
        .iter()
        .find(|any| any.type_url.ends_with("/humata.v1.Error"))
        .expect("the status carries a humata.v1.Error");
    pb::Error::decode(any.value.as_slice()).unwrap()
}

/// The field and problem of an `invalid_field` failure.
pub fn invalid_field(status: &Status) -> (String, pb::invalid_field::Problem) {
    assert_eq!(status.code(), Code::InvalidArgument, "{status:?}");
    match error_of(status).kind {
        Some(pb::error::Kind::InvalidField(field)) => {
            let problem = field.problem();
            (field.field, problem)
        }
        other => panic!("expected invalid_field, got {other:?}"),
    }
}

/// What kind of thing a `not_found` failure couldn't find.
pub fn not_found(status: &Status) -> pb::EntityKind {
    assert_eq!(status.code(), Code::NotFound, "{status:?}");
    match error_of(status).kind {
        Some(pb::error::Kind::NotFound(missing)) => missing.kind(),
        other => panic!("expected not_found, got {other:?}"),
    }
}

/// One Attach stream and everything read from it so far.
pub struct Attach {
    to: mpsc::Sender<pb::AttachClientMessage>,
    from: Streaming<pb::AttachServerMessage>,
    /// The offset `screen` ends at.
    pub cursor: u64,
    /// The snapshot, if one came, then every output byte after it.
    pub screen: Vec<u8>,
    pub snapshots: u32,
    pub states: Vec<pb::TerminalState>,
    pub notices: Vec<pb::TerminalNotice>,
    pub accepted: Vec<u64>,
    /// Snapshot pieces and outputs that came compressed.
    pub compressed: u32,
    /// Messages that carried a snapshot, pieces counted one by one.
    pub snapshot_pieces: u32,
    /// The terminal the last snapshot named; empty for a raw snapshot.
    pub terminal_uuid: String,
    /// A snapshot whose last piece hasn't come yet.
    piecing: bool,
    decoder: humata2_chunk::Decoder,
    seq: u64,
}

/// Decodes a snapshot piece's or an output's bytes.
fn decoded(
    decoder: &mut humata2_chunk::Decoder,
    codec: i32,
    raw_size: u32,
    crc32: Option<u32>,
    data: Vec<u8>,
) -> Vec<u8> {
    let codec = humata2_chunk::Codec::from_wire(codec).expect("a codec this build knows");
    if codec == humata2_chunk::Codec::None && crc32.is_none() {
        // Raw and whole, as to a client that lists no codecs.
        return data;
    }
    let raw_len = match (codec, raw_size) {
        (humata2_chunk::Codec::None, 0) => data.len() as u32,
        (_, raw_size) => raw_size,
    };
    decoder
        .decode(
            humata2_chunk::Header {
                codec,
                raw_len,
                crc32,
            },
            data,
        )
        .expect("a chunk that decodes")
}

pub fn open(tab: &pb::TabId, from_offset: u64) -> pb::AttachOpen {
    pb::AttachOpen {
        session: None,
        tab: Some(tab.clone()),
        from_offset,
        read_only: false,
        cols: 0,
        rows: 0,
        window_bytes: 0,
        accept_codecs: Vec::new(),
        checkpoint_engine: String::new(),
    }
}

impl Attach {
    /// Attaches as `open` says, failing with the call's error.
    pub async fn try_start(daemon: &Daemon, open: pb::AttachOpen) -> Result<Self, Status> {
        let (to, outgoing) = mpsc::channel(16);
        to.send(pb::AttachClientMessage {
            message: Some(pb::attach_client_message::Message::Open(open.clone())),
        })
        .await
        .unwrap();
        let from = daemon
            .terminals()
            .await
            .attach(ReceiverStream::new(outgoing))
            .await?
            .into_inner();
        Ok(Self {
            to,
            from,
            cursor: open.from_offset,
            screen: Vec::new(),
            snapshots: 0,
            states: Vec::new(),
            notices: Vec::new(),
            accepted: Vec::new(),
            compressed: 0,
            snapshot_pieces: 0,
            terminal_uuid: String::new(),
            piecing: false,
            decoder: humata2_chunk::Decoder::new(),
            seq: 0,
        })
    }

    pub async fn start(daemon: &Daemon, open: pb::AttachOpen) -> Self {
        Self::try_start(daemon, open).await.unwrap()
    }

    pub async fn send(&self, message: pb::attach_client_message::Message) {
        self.to
            .send(pb::AttachClientMessage {
                message: Some(message),
            })
            .await
            .unwrap();
    }

    /// Types `text` and returns its input sequence number.
    pub async fn type_text(&mut self, text: &str) -> u64 {
        self.seq += 1;
        self.send(pb::attach_client_message::Message::Input(
            pb::TerminalInput {
                data: text.as_bytes().to_vec(),
                input_seq: self.seq,
            },
        ))
        .await;
        self.seq
    }

    /// Reads one message and records it. Output must carry on from where
    /// the last left off.
    pub async fn read(&mut self) -> Result<(), Status> {
        let message = tokio::time::timeout(WAIT, self.from.message())
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "the attach went quiet at offset {}; its screen so far: {:?}",
                    self.cursor,
                    String::from_utf8_lossy(&self.screen)
                )
            })?
            .expect("the attach ended");
        match message.message.expect("a message") {
            pb::attach_server_message::Message::Snapshot(snapshot) => {
                self.snapshot_pieces += 1;
                self.terminal_uuid = snapshot.terminal_uuid.clone();
                if snapshot.codec != 0 {
                    self.compressed += 1;
                }
                let vt = decoded(
                    &mut self.decoder,
                    snapshot.codec,
                    snapshot.raw_size,
                    snapshot.crc32,
                    snapshot.vt,
                );
                if self.piecing {
                    assert_eq!(
                        snapshot.offset, self.cursor,
                        "a snapshot's pieces disagree on its offset"
                    );
                    self.screen.extend_from_slice(&vt);
                } else {
                    self.screen = vt;
                }
                self.piecing = snapshot.more;
                if !snapshot.more {
                    self.snapshots += 1;
                }
                self.cursor = snapshot.offset;
            }
            pb::attach_server_message::Message::Output(output) => {
                assert!(!self.piecing, "output came in the middle of a snapshot");
                assert_eq!(
                    output.offset, self.cursor,
                    "output skipped or repeated bytes"
                );
                if output.codec != 0 {
                    self.compressed += 1;
                }
                let data = decoded(
                    &mut self.decoder,
                    output.codec,
                    output.raw_size,
                    output.crc32,
                    output.data,
                );
                self.cursor += data.len() as u64;
                self.screen.extend_from_slice(&data);
            }
            pb::attach_server_message::Message::State(state) => self.states.push(state),
            pb::attach_server_message::Message::Notice(notice) => self.notices.push(notice),
            pb::attach_server_message::Message::InputAccepted(accepted) => {
                self.accepted.push(accepted.input_seq);
            }
        }
        Ok(())
    }

    /// Reads until `screen` holds `text`.
    pub async fn wait_for(&mut self, text: &str) {
        while !self.shows(text) {
            self.read().await.unwrap();
        }
    }

    /// Reads until `done` holds for what's been read.
    pub async fn wait_until(&mut self, done: impl Fn(&Self) -> bool) {
        while !done(self) {
            self.read().await.unwrap();
        }
    }

    /// Reads until the stream fails, and returns how.
    pub async fn wait_for_error(&mut self) -> Status {
        loop {
            if let Err(status) = self.read().await {
                return status;
            }
        }
    }

    pub fn shows(&self, text: &str) -> bool {
        String::from_utf8_lossy(&self.screen).contains(text)
    }
}

// ---------------------------------------------------------------------------
// Channels and sessions: a world of git remotes, a test agent and the serve
// flags that point the daemon at them.

/// The name the test agent goes by.
pub const AGENT_NAME: &str = "fake";

/// What an agent stands in as: `--session-id` starts a conversation file,
/// `--resume` keeps it, `--fresh` starts another. Every line typed to it is
/// answered with `got: <line>` and kept in the conversation.
fn agent_script(conv: &Path) -> String {
    format!(
        r#"CONV_DIR='{}'
mode=$1; id=$2
case "$mode" in
  --session-id) f="$CONV_DIR/$id.jsonl"; : > "$f" ;;
  --resume) f="$CONV_DIR/$id.jsonl" ;;
  --fresh) f="$CONV_DIR/other.jsonl"; : > "$f" ;;
esac
echo "agent up $mode"
while IFS= read -r line; do echo "got: $line"; printf '%s\n' "$line" >> "$f"; done
"#,
        conv.display()
    )
}

/// The test agent that repaints its screen on SIGWINCH, as a program does
/// when its terminal is resized (D393). A repaint clears the screen and
/// shows the agent's last answer, or the text of `repaint-text` in the
/// conversations folder when that file is there, then a `**` line. Once
/// `exit-after-repaint` is there, the agent exits after a repaint. Otherwise
/// it answers as [`agent_script`] does.
fn redrawing_agent_script(conv: &Path) -> String {
    format!(
        r#"CONV_DIR='{}'
mode=$1; id=$2
case "$mode" in
  --session-id) f="$CONV_DIR/$id.jsonl"; : > "$f" ;;
  --resume) f="$CONV_DIR/$id.jsonl" ;;
  --fresh) f="$CONV_DIR/other.jsonl"; : > "$f" ;;
esac
last="agent up $mode"
repaint() {{
  if [ -f "$CONV_DIR/repaint-text" ]; then last=$(cat "$CONV_DIR/repaint-text"); fi
  printf '\033[H\033[2J%s\n**\n' "$last"
  if [ -f "$CONV_DIR/exit-after-repaint" ]; then exit 0; fi
  woke=1
}}
trap repaint WINCH
echo "$last"
while :; do
  woke=0
  IFS= read -r line
  rc=$?
  # A repaint that interrupts the read is not the end of the input: bash
  # says so with a status over 128, dash with 1.
  if [ "$rc" -ne 0 ] && [ "$woke" = 1 ]; then continue; fi
  if [ "$rc" -ne 0 ]; then break; fi
  last="got: $line"
  echo "$last"
  printf '%s\n' "$line" >> "$f"
done
"#,
        conv.display()
    )
}

/// Runs `git` with an identity, failing the test when it fails; returns
/// what it printed, trimmed.
pub fn git(args: &[&str]) -> String {
    let output = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// Everything a channel or session test needs beside the daemon's own
/// folders: where channels go, bare remotes to clone from, the test agent
/// and its conversations.
pub struct World {
    pub root: PathBuf,
    pub channels: PathBuf,
    pub remotes: PathBuf,
    pub conv: PathBuf,
    pub agents: PathBuf,
}

impl World {
    /// Makes the world's folders and files inside `root`.
    pub fn new(root: &Path) -> Self {
        let world = Self {
            root: root.to_owned(),
            channels: root.join("ch"),
            remotes: root.join("remotes"),
            conv: root.join("conv"),
            agents: root.join("agents.json"),
        };
        std::fs::create_dir_all(&world.conv).unwrap();
        std::fs::create_dir_all(&world.remotes).unwrap();
        let script = root.join("agent.sh");
        std::fs::write(&script, agent_script(&world.conv)).unwrap();
        let agents = serde_json::json!([{
            "name": AGENT_NAME,
            "program": "/bin/sh",
            "start_args": [script, "--session-id", "{conversation}"],
            "resume_args": [script, "--resume", "{conversation}"],
            "conversation_file": format!("{}/{{conversation}}.jsonl", world.conv.display()),
        }]);
        std::fs::write(&world.agents, agents.to_string()).unwrap();
        world
    }

    /// The `serve` flags that use this world.
    pub fn args(&self) -> Vec<String> {
        let template = format!("file://{}/{{owner}}/{{name}}.git", self.remotes.display());
        vec![
            "--channels".to_owned(),
            self.channels.display().to_string(),
            "--agents".to_owned(),
            self.agents.display().to_string(),
            "--default-agent".to_owned(),
            AGENT_NAME.to_owned(),
            "--remote-template".to_owned(),
            template,
        ]
    }

    /// The same world, whose test agent repaints its screen on a resize (see
    /// [`redrawing_agent_script`]). Only the agent's script differs.
    pub fn redrawing(root: &Path) -> Self {
        let world = Self::new(root);
        std::fs::write(root.join("agent.sh"), redrawing_agent_script(&world.conv)).unwrap();
        world
    }

    /// A bare repository `owner/name` on branch `main` with one commit.
    pub fn remote(&self, owner: &str, name: &str) -> PathBuf {
        let bare = self.remotes.join(owner).join(format!("{name}.git"));
        std::fs::create_dir_all(&bare).unwrap();
        let bare_text = bare.display().to_string();
        git(&["init", "--bare", "--initial-branch=main", &bare_text]);
        let work = self.root.join(format!("scratch-{owner}-{name}"));
        std::fs::create_dir_all(&work).unwrap();
        let work_text = work.display().to_string();
        git(&["init", "--initial-branch=main", &work_text]);
        std::fs::write(work.join("README.md"), format!("{owner}/{name}\n")).unwrap();
        git(&["-C", &work_text, "add", "."]);
        git(&["-C", &work_text, "commit", "-m", "first"]);
        git(&["-C", &work_text, "push", &bare_text, "main"]);
        bare
    }

    /// The commit `main` is at in the remote `bare`.
    pub fn remote_head(bare: &Path) -> String {
        git(&[
            "--git-dir",
            &bare.display().to_string(),
            "rev-parse",
            "main",
        ])
    }

    /// The conversation file the agent of `conversation` writes.
    pub fn conversation(&self, conversation: &str) -> PathBuf {
        self.conv.join(format!("{conversation}.jsonl"))
    }
}

/// A daemon serving this world, and the world.
pub async fn start_world() -> (Daemon, World) {
    let dir = folder();
    let world = World::new(dir.path());
    let args = world.args();
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    (Daemon::start_with(Build::Admin, dir, &refs).await, world)
}

/// A daemon serving a world whose test agent repaints on a resize (see
/// [`World::redrawing`]), and the world.
pub async fn start_redrawing_world() -> (Daemon, World) {
    let dir = folder();
    let world = World::redrawing(dir.path());
    let args = world.args();
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    (Daemon::start_with(Build::Admin, dir, &refs).await, world)
}

impl Daemon {
    pub async fn channel_service(
        &self,
    ) -> pb::channel_service_client::ChannelServiceClient<Channel> {
        pb::channel_service_client::ChannelServiceClient::new(self.channel().await)
    }

    pub async fn session_service(
        &self,
    ) -> pb::session_service_client::SessionServiceClient<Channel> {
        pb::session_service_client::SessionServiceClient::new(self.channel().await)
    }

    pub async fn operation_service(
        &self,
    ) -> pb::operation_service_client::OperationServiceClient<Channel> {
        pb::operation_service_client::OperationServiceClient::new(self.channel().await)
    }
}

pub fn repo_ref(owner: &str, name: &str) -> pb::RepoRef {
    pb::RepoRef {
        owner: owner.to_owned(),
        name: name.to_owned(),
    }
}

/// Waits until `operation` is no longer running, and returns it as it ended.
pub async fn finished(daemon: &Daemon, operation: &pb::Operation) -> pb::Operation {
    let mut client = daemon.operation_service().await;
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let now = client
            .get_operation(pb::GetOperationRequest {
                id: operation.id.clone(),
            })
            .await
            .unwrap()
            .into_inner();
        if now.state != pb::OperationState::Running as i32 {
            return now;
        }
        assert!(
            Instant::now() < deadline,
            "operation {:?} never finished",
            operation.id
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Creates a channel of `repos` (`(owner, name)`) and waits for its setup.
pub async fn create_channel(
    daemon: &Daemon,
    name: &str,
    repos: &[(&str, &str)],
) -> (pb::Channel, pb::Operation) {
    let response = daemon
        .channel_service()
        .await
        .create_channel(pb::CreateChannelRequest {
            idempotency_key: None,
            name: name.to_owned(),
            repos: repos
                .iter()
                .map(|(owner, name)| repo_ref(owner, name))
                .collect(),
            machines: Vec::new(),
        })
        .await
        .unwrap()
        .into_inner();
    let setup = finished(daemon, &response.setup.unwrap()).await;
    (response.channel.unwrap(), setup)
}

/// This daemon's own machine.
pub async fn machine_id(daemon: &Daemon) -> pb::MachineId {
    server_info(daemon).await.machine.unwrap()
}

pub async fn server_info(daemon: &Daemon) -> pb::ServerInfo {
    pb::daemon_service_client::DaemonServiceClient::new(daemon.channel().await)
        .get_server_info(pb::GetServerInfoRequest {})
        .await
        .unwrap()
        .into_inner()
}

/// The start's account, once the daemon has brought everything back.
pub async fn last_start(daemon: &Daemon) -> pb::LastStart {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        if let Some(start) = server_info(daemon).await.last_start {
            return start;
        }
        assert!(
            Instant::now() < deadline,
            "the daemon never finished bringing sessions back"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

pub fn start_request(
    channel: &pb::ChannelId,
    machine: &pb::MachineId,
    title: &str,
) -> pb::StartSessionRequest {
    pb::StartSessionRequest {
        idempotency_key: None,
        channel: Some(channel.clone()),
        title: title.to_owned(),
        agent: None,
        model: String::new(),
        machine: Some(machine.clone()),
        repos: Vec::new(),
        first_message: String::new(),
        board_item: None,
        branch: String::new(),
        rebuild: None,
        review: None,
    }
}

/// Starts a session titled `title` and waits for its operation.
pub async fn start_session(
    daemon: &Daemon,
    channel: &pb::Channel,
    title: &str,
) -> (pb::SessionId, pb::Operation) {
    let machine = machine_id(daemon).await;
    let response = daemon
        .session_service()
        .await
        .start_session(start_request(channel.id.as_ref().unwrap(), &machine, title))
        .await
        .unwrap()
        .into_inner();
    let operation = finished(daemon, &response.operation.unwrap()).await;
    (response.session.unwrap(), operation)
}

pub async fn get_session(daemon: &Daemon, id: &pb::SessionId) -> pb::Session {
    daemon
        .session_service()
        .await
        .get_session(pb::GetSessionRequest {
            id: Some(id.clone()),
        })
        .await
        .unwrap()
        .into_inner()
}

/// The session's agent runs: Working, or ProbablyIdle when it has been
/// quiet a while (a loaded machine gets there sooner).
#[allow(dead_code)]
pub fn is_running(session: &pb::Session) -> bool {
    [pb::SessionStatus::Working as i32, pb::SessionStatus::ProbablyIdle as i32].contains(&session.status)
}

/// Polls until the session's agent runs (`is_running`), and returns it.
#[allow(dead_code)]
pub async fn wait_running(daemon: &Daemon, id: &pb::SessionId) -> pb::Session {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        let session = get_session(daemon, id).await;
        if is_running(&session) {
            return session;
        }
        assert!(
            Instant::now() < deadline,
            "session {} never ran; it is {:?}, exit code {}, resume failure {:?}",
            id.value,
            session.status(),
            session.exit_code,
            session.resume_failed
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Polls until the session has `status`, and returns it.
pub async fn wait_status(
    daemon: &Daemon,
    id: &pb::SessionId,
    status: pb::SessionStatus,
) -> pb::Session {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        let session = get_session(daemon, id).await;
        if session.status == status as i32 {
            return session;
        }
        assert!(
            Instant::now() < deadline,
            "session {} never became {status:?}; it is {:?}",
            id.value,
            session.status()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// The session's agent tab.
pub async fn agent_tab(daemon: &Daemon, id: &pb::SessionId) -> pb::TabId {
    daemon
        .terminals()
        .await
        .list_tabs(pb::ListTabsRequest {
            session: Some(id.clone()),
        })
        .await
        .unwrap()
        .into_inner()
        .tabs
        .into_iter()
        .find(|tab| tab.kind == pb::tab::Kind::Agent as i32)
        .expect("the session has an agent tab")
        .id
        .unwrap()
}

/// Every tab the store knows of (the daemon may be running or not).
pub fn stored_tabs(state: &Path) -> Vec<humata2_store::TabRecord> {
    humata2_store::Store::open(state.join("store"))
        .unwrap()
        .tabs()
        .unwrap()
}

/// Whether the holder behind `tab` still answers on its socket.
pub fn holder_answers(tab: &humata2_store::TabRecord) -> bool {
    std::os::unix::net::UnixStream::connect(&tab.holder_socket).is_ok()
}

/// Kills the holders of `tabs` at once, as a reboot would, and waits until
/// each is gone.
pub async fn kill_holders(tabs: &[humata2_store::TabRecord]) {
    for tab in tabs {
        if let Some(pid) = i32::try_from(tab.holder_pid).ok().and_then(Pid::from_raw) {
            let _ = rustix::process::kill_process(pid, Signal::KILL);
        }
    }
    let deadline = Instant::now() + WAIT;
    for tab in tabs {
        while holder_answers(tab) {
            assert!(
                Instant::now() < deadline,
                "holder {} never stopped answering",
                tab.holder_pid
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

/// Runs `humata --json <args>` against the daemon as the user; returns the
/// exit code and the JSON printed.
pub fn humata(daemon: &Daemon, args: &[&str]) -> (i32, serde_json::Value) {
    let output = Command::new(binary("HUMATA"))
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", daemon.dir.path())
        .env("HUMATA_SOCKET", &daemon.socket)
        .arg("--json")
        .args(args)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    let json = serde_json::from_str(&text).unwrap_or_else(|error| {
        panic!(
            "humata printed no JSON ({error}): {text:?}; stderr {:?}",
            output.stderr
        )
    });
    (output.status.code().unwrap_or(-1), json)
}

/// Polls until `path` exists.
pub async fn wait_for_path(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "{} never appeared",
            path.display()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Opens a shell tab in `session` and returns the token its shell was
/// given (an agent's identity), with the attach that keeps the tab in use.
pub async fn shell_token(daemon: &Daemon, session: &pb::SessionId) -> (String, Attach) {
    let tab = daemon
        .terminals()
        .await
        .open_shell_tab(pb::OpenShellTabRequest {
            idempotency_key: None,
            session: Some(session.clone()),
            repo: None,
            folder: String::new(),
        })
        .await
        .unwrap()
        .into_inner()
        .id
        .unwrap();
    let mut attach = Attach::start(daemon, open(&tab, 0)).await;
    let file = daemon.dir.path().join(format!("token-{}", tab.value));
    attach
        .type_text(&format!(
            "printf '%s' \"$HUMATA_SESSION_TOKEN\" > {}\r",
            file.display()
        ))
        .await;
    let deadline = Instant::now() + WAIT;
    loop {
        match std::fs::read_to_string(&file) {
            Ok(text) if text.len() == 32 => return (text, attach),
            _ => {}
        }
        assert!(
            Instant::now() < deadline,
            "the shell never wrote its session token"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// `humata --json <args>` run by the shell of the tab `attach` is on, so it
/// is in that tab's process tree and carries the token the shell was
/// given. Returns the JSON it printed, or null if it printed none (a
/// refused call may exit non-zero with nothing on stdout).
pub async fn humata_in_tab(daemon: &Daemon, attach: &mut Attach, args: &str) -> serde_json::Value {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let number = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let file = daemon.dir.path().join(format!("in-tab-{number}.json"));
    let partial = daemon.dir.path().join(format!("in-tab-{number}.part"));
    attach
        .type_text(&format!(
            "HUMATA_SOCKET='{}' '{}' --json {args} > '{}' 2>/dev/null; mv '{}' '{}'\r",
            daemon.socket.display(),
            binary("HUMATA").display(),
            partial.display(),
            partial.display(),
            file.display(),
        ))
        .await;
    wait_for_path(&file).await;
    let text = std::fs::read_to_string(&file).unwrap();
    serde_json::from_str(&text).unwrap_or(serde_json::Value::Null)
}

/// `humata --json <args>` as the agent holding `token`; the exit code and
/// the JSON printed.
pub async fn humata_as_agent(
    daemon: &Daemon,
    token: &str,
    args: &[&str],
) -> (i32, serde_json::Value) {
    let output = tokio::process::Command::new(binary("HUMATA"))
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", daemon.dir.path())
        .env("HUMATA_SOCKET", &daemon.socket)
        .env("HUMATA_SESSION_TOKEN", token)
        .arg("--json")
        .args(args)
        .output()
        .await
        .unwrap();
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    let json = serde_json::from_str(&text).unwrap_or_else(|error| {
        panic!(
            "humata printed no JSON ({error}): {text:?}; stderr {:?}",
            output.stderr
        )
    });
    (output.status.code().unwrap_or(-1), json)
}

impl Daemon {
    pub async fn inbox_service(&self) -> pb::inbox_service_client::InboxServiceClient<Channel> {
        pb::inbox_service_client::InboxServiceClient::new(self.channel().await)
    }
}

/// The ids of the requests waiting for the user.
pub async fn pending_requests(daemon: &Daemon) -> Vec<pb::RequestId> {
    daemon
        .inbox_service()
        .await
        .list_requests(pb::ListRequestsRequest::default())
        .await
        .unwrap()
        .into_inner()
        .requests
        .into_iter()
        .filter(|request| request.state() == pb::RequestState::Pending)
        .filter_map(|request| request.id)
        .collect()
}

/// Polls until a request is waiting for the user, and returns its id.
pub async fn first_pending(daemon: &Daemon) -> pb::RequestId {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(id) = pending_requests(daemon).await.into_iter().next() {
            return id;
        }
        assert!(Instant::now() < deadline, "no request reached the Inbox");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// The user approves request `id`.
pub async fn approve(daemon: &Daemon, id: &pb::RequestId) {
    daemon
        .inbox_service()
        .await
        .answer_request(pb::AnswerRequestRequest {
            id: Some(id.clone()),
            answer: Some(pb::RequestAnswer {
                answer: Some(pb::request_answer::Answer::Approve(pb::Approve {})),
            }),
            ..Default::default()
        })
        .await
        .unwrap();
}

/// `humata --json <args>` as a script holding `token` (`HUMATA_TOKEN`), with
/// no session token; the exit code and the JSON printed.
pub async fn humata_as_script(
    daemon: &Daemon,
    token: &str,
    args: &[&str],
) -> (i32, serde_json::Value) {
    let output = tokio::process::Command::new(binary("HUMATA"))
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", daemon.dir.path())
        .env("HUMATA_SOCKET", &daemon.socket)
        .env("HUMATA_TOKEN", token)
        .arg("--json")
        .args(args)
        .output()
        .await
        .unwrap();
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    let json = serde_json::from_str(&text).unwrap_or_else(|error| {
        panic!(
            "humata printed no JSON ({error}): {text:?}; stderr {:?}",
            output.stderr
        )
    });
    (output.status.code().unwrap_or(-1), json)
}

/// A request that carries `token` as a script's credential.
pub fn as_script<T>(message: T, token: &str) -> tonic::Request<T> {
    let mut request = tonic::Request::new(message);
    request.metadata_mut().insert(
        "humata-script-token",
        tonic::metadata::MetadataValue::try_from(token).unwrap(),
    );
    request
}

impl Daemon {
    pub async fn cli_auth_service(
        &self,
    ) -> pb::cli_auth_service_client::CliAuthServiceClient<Channel> {
        pb::cli_auth_service_client::CliAuthServiceClient::new(self.channel().await)
    }
}

/// The user makes a script token; returns its id and secret, or why not.
pub async fn create_script_token(
    daemon: &Daemon,
    name: &str,
    capabilities: &[pb::Capability],
    channels: &[&pb::ChannelId],
    days: u32,
) -> Result<(String, String), Status> {
    daemon
        .cli_auth_service()
        .await
        .create_script_token(pb::CreateScriptTokenRequest {
            name: name.to_owned(),
            capabilities: capabilities.iter().map(|c| *c as i32).collect(),
            channels: channels.iter().map(|c| (*c).clone()).collect(),
            days,
            ..Default::default()
        })
        .await
        .map(|reply| {
            let reply = reply.into_inner();
            (reply.token_id, reply.token)
        })
}

/// As [`start_world`], with more `serve` arguments.
pub async fn start_world_with(extra: &[&str]) -> (Daemon, World) {
    let dir = folder();
    let world = World::new(dir.path());
    let mut args = world.args();
    args.extend(extra.iter().map(|arg| (*arg).to_owned()));
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    (Daemon::start_with(Build::Admin, dir, &refs).await, world)
}

impl Daemon {
    /// The daemon's process id, while it runs.
    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().map(Child::id)
    }

    /// Sends SIGTERM, as a service manager does, and waits within [`WAIT`]
    /// for the daemon to exit. One still running then is killed, and the
    /// test fails.
    pub async fn terminate(&mut self) -> std::process::ExitStatus {
        let mut child = self.child.take().expect("the daemon is running");
        let pid = i32::try_from(child.id())
            .ok()
            .and_then(Pid::from_raw)
            .expect("a process id");
        rustix::process::kill_process(pid, Signal::TERM).expect("SIGTERM reaches the daemon");
        let waited = tokio::task::spawn_blocking(move || child.wait());
        match tokio::time::timeout(WAIT, waited).await {
            Ok(status) => status
                .expect("the wait finished")
                .expect("the daemon's status"),
            Err(_) => {
                let _ = rustix::process::kill_process(pid, Signal::KILL);
                panic!("humatad2 was still running {WAIT:?} after SIGTERM");
            }
        }
    }
}

/// Starts a second `humatad2 serve` on `state`, with a socket of its own
/// in `dir`, and waits within [`WAIT`] for it to exit: for a daemon that
/// must refuse to start. Returns its exit status and what it wrote to
/// stderr.
pub async fn serve_and_exit(dir: &Path, state: &Path) -> (std::process::ExitStatus, String) {
    let mut command = tokio::process::Command::new(binary("HUMATAD2"));
    command
        .arg("serve")
        .arg("--admin")
        .arg("--socket")
        .arg(dir.join("second.sock"))
        .arg("--state")
        .arg(state)
        .arg("--root")
        .arg(dir)
        .arg("--holder")
        .arg(binary("HUMATA2_HOLDER"))
        .arg("--shell")
        .arg("/bin/sh")
        .arg("--shell-arg=-i")
        .kill_on_drop(true);
    let output = tokio::time::timeout(WAIT, command.output())
        .await
        .expect("the second daemon was still running")
        .expect("the second daemon ran");
    (
        output.status,
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}
