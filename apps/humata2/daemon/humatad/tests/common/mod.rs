//! Starts a real `humatad2` on a socket of its own, connects to it over
//! gRPC, and stops it and the holders it started when the test ends. By
//! default it is the admin test daemon run with `--admin`, so calls with no
//! credential act as the user; `Build::Release` is the daemon as shipped.

#![allow(dead_code)]

use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use humata2_grpc::humata::v1 as pb;
use hyper_util::rt::TokioIo;
use prost::Message;
use rustix::process::{Pid, Signal};
use tempfile::TempDir;
use tokio::net::UnixStream;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::{Channel, Endpoint, Uri};
use tonic::{Code, Status, Streaming};

pub const WAIT: Duration = Duration::from_secs(15);

fn binary(variable: &str) -> PathBuf {
    let path = std::env::var_os(variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("set {variable} to the binary's path"));
    std::fs::canonicalize(path).unwrap()
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
}

pub struct Daemon {
    child: Option<Child>,
    build: Build,
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
        let mut daemon = Self {
            child: None,
            build,
            socket: dir.path().join("d.sock"),
            state: dir.path().join("state"),
            dir,
        };
        daemon.launch().await;
        daemon
    }

    async fn launch(&mut self) {
        let mut command = match self.build {
            Build::Admin => Command::new(binary("HUMATAD2")),
            Build::Release => Command::new(binary("HUMATAD2_RELEASE")),
        };
        command.arg("serve");
        if self.build == Build::Admin {
            command.arg("--admin");
        }
        let child = command
            .arg("--socket")
            .arg(&self.socket)
            .arg("--state")
            .arg(&self.state)
            .arg("--root")
            .arg(self.dir.path())
            .arg("--holder")
            .arg(binary("HUMATA2_HOLDER"))
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
            assert!(Instant::now() < deadline, "humatad2 never started listening");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// Ends the daemon at once, as a crash would.
    pub fn kill(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    pub async fn restart(&mut self) {
        self.kill();
        self.launch().await;
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
        let Ok(store) = humata2_store::Store::open(self.state.join("store")) else {
            return;
        };
        for tab in store.tabs().unwrap_or_default() {
            if let Some(pid) = i32::try_from(tab.holder_pid).ok().and_then(Pid::from_raw) {
                let _ = rustix::process::kill_process(pid, Signal::TERM);
            }
        }
    }
}

pub async fn connect(socket: &Path) -> Channel {
    let socket = socket.to_owned();
    Endpoint::try_from("http://[::]:50051")
        .unwrap()
        .connect_with_connector(tower::service_fn(move |_: Uri| {
            let socket = socket.clone();
            async move { Ok::<_, std::io::Error>(TokioIo::new(UnixStream::connect(socket).await?)) }
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
    seq: u64,
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
    }
}

impl Attach {
    /// Attaches as `open` says, failing with the call's error.
    pub async fn try_start(
        daemon: &Daemon,
        open: pb::AttachOpen,
    ) -> Result<Self, Status> {
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
        self.send(pb::attach_client_message::Message::Input(pb::TerminalInput {
            data: text.as_bytes().to_vec(),
            input_seq: self.seq,
        }))
        .await;
        self.seq
    }

    /// Reads one message and records it. Output must carry on from where
    /// the last left off.
    pub async fn read(&mut self) -> Result<(), Status> {
        let message = tokio::time::timeout(WAIT, self.from.message())
            .await
            .expect("the attach went quiet")?
            .expect("the attach ended");
        match message.message.expect("a message") {
            pb::attach_server_message::Message::Snapshot(snapshot) => {
                self.snapshots += 1;
                self.screen = snapshot.vt;
                self.cursor = snapshot.offset;
            }
            pb::attach_server_message::Message::Output(output) => {
                assert_eq!(output.offset, self.cursor, "output skipped or repeated bytes");
                self.cursor += output.data.len() as u64;
                self.screen.extend_from_slice(&output.data);
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
