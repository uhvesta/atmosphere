//! A channel across machines and a session on another machine (CH-5, CH-7,
//! CH-19, SL-1, SL-3, PER-11, HO-5, HO-6; D139, D144, D146; flows 26.1,
//! 26.2, 26.9, 26.21), through a real host daemon and a fake machine.
//!
//! The fake machine is a temp home with a fake `ssh` and a `humatad2`
//! wrapper that execs the real binary. Its `--stdio` link relays to a real
//! machine-role socket daemon that this test starts under the same home,
//! with the state folder the host's add wrote the enrollment key into, its
//! own holder, its own channels root and the test agent (D146: a link given
//! `--socket` relays; one given only `--state` doesn't).
//!
//! Written against the shape after stage I (`remote_work.rs` and
//! `remote_terminal.rs` wired into ChannelService, SessionService and
//! TerminalService). Enabling is a move to `tests/remote.rs`. What the
//! tests rely on:
//!
//! - CreateChannel with a remote machine adds steps `record:<machine>` and
//!   `clone_main:<machine>:<owner/name>` to the host's operation, and the
//!   channel is created on the machine under the host's channel id.
//! - A channel created while a machine is offline leaves those steps
//!   Waiting and the operation Running; when the link returns the machine
//!   clones and the steps end Done.
//! - StartSession naming a remote machine answers with an operation that
//!   ends Succeeded; GetSession and ListSessions show the machine's status,
//!   and OFFLINE while its link is down; an offline machine refuses start
//!   and stop with `machine_offline` at once.
//! - TerminalService.ListTabs and Attach for a remote session are relayed,
//!   and an Attach survives a link drop (DISCONNECTED, then RECONNECTED).
//! - The host publishes the session's status changes on the event feed.
//! - A repo added while the machine is offline waits there, then clones
//!   when the link returns (flow 26.21).

mod common;

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use common::{WAIT, World, binary, connect, error_of, folder, holder_answers, repo_ref, start_request, stored_tabs};
use humata2_fake_machine::FakeMachine;
use humata2_grpc::humata::v1 as pb;
use humata2_store::Store;
use rustix::process::{Pid, Signal, kill_process};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tokio::net::UnixStream;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::Channel;
use tonic::{Request, Streaming};

const FILES: [&str; 3] = ["humatad2", "humata2_holder", "humata"];
const BOX: &str = "box-1";
/// The host redials after a backoff of 5 s, then 10 s while dials fail.
const REDIAL: Duration = Duration::from_secs(60);

/// A tools bundle for linux-x86_64 whose humatad2 runs the real one. Its
/// `--stdio` link is given the machine's own state folder and socket, so it
/// relays to the socket daemon `socket` names, and it writes its pid to
/// `$HOME/link.pid` so a test can kill the link.
fn bundle(dir: &Path, socket: &Path, state: &Path) -> PathBuf {
    let root = dir.join("tools");
    std::fs::create_dir_all(root.join("linux-x86_64")).unwrap();
    let mut entries = serde_json::Map::new();
    for file in FILES {
        let body = if file == "humatad2" {
            format!(
                "#!/bin/sh\ncase \" $* \" in *\" --stdio \"*)\n  echo $$ > \"$HOME/link.pid.new\" && mv \"$HOME/link.pid.new\" \"$HOME/link.pid\"\n  exec '{real}' \"$@\" --state '{state}' --socket '{socket}' ;;\nesac\nexec '{real}' \"$@\"\n",
                real = binary("HUMATAD2_RELEASE").display(),
                state = state.display(),
                socket = socket.display(),
            )
        } else {
            format!("#!/bin/sh\n# {file}\nexit 0\n")
        };
        std::fs::write(root.join("linux-x86_64").join(file), &body).unwrap();
        entries.insert(
            file.to_owned(),
            serde_json::json!({
                "path": format!("linux-x86_64/{file}"),
                "sha256": Sha256::digest(body.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect::<String>(),
            }),
        );
    }
    std::fs::write(root.join("manifest.json"), serde_json::json!({ "platforms": { "linux-x86_64": entries } }).to_string()).unwrap();
    root
}

/// Ends `child` and waits for it.
fn stop(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// The fake machine's socket daemon: a release daemon in the machine role,
/// started by the test, under the machine's home. It can be killed and
/// started again on the same state and socket, as a crash or a reboot does;
/// the holders it started survive that, as they do a daemon crash.
struct MachineDaemon {
    child: Option<Child>,
    args: Vec<String>,
    home: PathBuf,
    socket: PathBuf,
    state: PathBuf,
    /// Holds the socket's short path.
    _dir: TempDir,
}

impl MachineDaemon {
    async fn start(home: &Path, world: &World) -> Self {
        let dir = folder();
        let mut args = world.args();
        // The machine keeps its channels under its own home: `~/Humata`.
        args[1] = home.join("Humata").display().to_string();
        // The fake machine's home is a long path, and a holder's socket
        // (`<state>/holders/tb_<id>.sock`) must fit a socket address. So
        // the machine's `~/.humata2` is a link to a short folder: the add
        // writes the enrollment key through the link, and the daemon and
        // the link process are given the short path itself.
        let short = dir.path().join("h");
        std::fs::create_dir(&short).unwrap();
        std::os::unix::fs::symlink(&short, home.join(".humata2")).unwrap();
        let mut daemon = Self {
            child: None,
            args,
            home: home.to_owned(),
            socket: dir.path().join("m.sock"),
            state: short.join("state"),
            _dir: dir,
        };
        daemon.launch().await;
        daemon
    }

    async fn launch(&mut self) {
        let child = Command::new(binary("HUMATAD2_RELEASE"))
            .args(["serve", "--role", "machine", "--socket"])
            .arg(&self.socket)
            .arg("--state")
            .arg(&self.state)
            .arg("--root")
            .arg(&self.home)
            .arg("--holder")
            .arg(binary("HUMATA2_HOLDER"))
            .arg("--shell")
            .arg("/bin/sh")
            .arg("--shell-arg=-i")
            .args(&self.args)
            .env("HOME", &self.home)
            .spawn()
            .unwrap();
        self.child = Some(child);
        let deadline = Instant::now() + WAIT;
        while UnixStream::connect(&self.socket).await.is_err() {
            assert!(Instant::now() < deadline, "the machine's humatad2 never started listening");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// The power loss: the daemon ends at once. Its holders keep running.
    fn kill(&mut self) {
        if let Some(mut child) = self.child.take() {
            stop(&mut child);
            // A killed daemon's socket can answer for a few milliseconds more.
            let deadline = Instant::now() + WAIT;
            while std::os::unix::net::UnixStream::connect(&self.socket).is_ok() {
                assert!(Instant::now() < deadline, "a killed humatad2's socket never stopped answering");
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }

    fn tabs(&self) -> Vec<humata2_store::TabRecord> {
        stored_tabs(&self.state)
    }

    fn store(&self) -> Store {
        Store::open(self.state.join("store")).unwrap()
    }
}

impl Drop for MachineDaemon {
    fn drop(&mut self) {
        self.kill();
        // Holders outlive their daemon on purpose; these ones don't.
        let Ok(store) = Store::open(self.state.join("store")) else { return };
        for tab in store.tabs().unwrap_or_default() {
            if let Some(pid) = i32::try_from(tab.holder_pid).ok().and_then(Pid::from_raw) {
                let _ = kill_process(pid, Signal::TERM);
            }
        }
    }
}

/// The admin host daemon serving `world`, with the fake machine's `ssh`
/// first on its PATH, and nothing of the real home folder in reach.
struct Host {
    child: Child,
    socket: PathBuf,
    state: PathBuf,
    _dir: TempDir,
    world: World,
}

impl Host {
    async fn start(machine: &FakeMachine, socket: &Path, machine_state: &Path, dir: TempDir, world: World) -> Self {
        let state = dir.path().join("state");
        let host_socket = dir.path().join("d.sock");
        let child = Command::new(binary("HUMATAD2"))
            .arg("serve")
            .arg("--admin")
            .arg("--socket")
            .arg(&host_socket)
            .arg("--state")
            .arg(&state)
            .arg("--root")
            .arg(dir.path())
            .arg("--holder")
            .arg(binary("HUMATA2_HOLDER"))
            .arg("--shell")
            .arg("/bin/sh")
            .arg("--shell-arg=-i")
            .arg("--tools")
            .arg(bundle(dir.path(), socket, machine_state))
            .arg("--ssh-config")
            .arg(dir.path().join("ssh_config"))
            .arg("--skills-home")
            .arg(dir.path().join("home"))
            .args(world.args())
            .env("PATH", format!("{}:/usr/bin:/bin", machine.ssh_dir().display()))
            .spawn()
            .unwrap();
        let deadline = Instant::now() + WAIT;
        while UnixStream::connect(&host_socket).await.is_err() {
            assert!(Instant::now() < deadline, "humatad2 never started listening");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Self { child, socket: host_socket, state, _dir: dir, world }
    }

    async fn channel(&self) -> Channel {
        connect(&self.socket).await
    }

    async fn channels(&self) -> pb::channel_service_client::ChannelServiceClient<Channel> {
        pb::channel_service_client::ChannelServiceClient::new(self.channel().await)
    }

    async fn sessions(&self) -> pb::session_service_client::SessionServiceClient<Channel> {
        pb::session_service_client::SessionServiceClient::new(self.channel().await)
    }

    async fn operations(&self) -> pb::operation_service_client::OperationServiceClient<Channel> {
        pb::operation_service_client::OperationServiceClient::new(self.channel().await)
    }

    async fn machines(&self) -> pb::machine_service_client::MachineServiceClient<Channel> {
        pb::machine_service_client::MachineServiceClient::new(self.channel().await)
    }

    async fn terminals(&self) -> pb::terminal_service_client::TerminalServiceClient<Channel> {
        pb::terminal_service_client::TerminalServiceClient::new(self.channel().await)
    }

    /// Adds box-1 (confirming its host key) and waits for its link.
    async fn add_box(&self) {
        let mut machines = self.machines().await;
        let request = |fingerprint: &str| pb::AddSshMachineRequest {
            idempotency_key: Some(pb::IdempotencyKey { value: format!("add-{fingerprint}") }),
            ssh_host: "me@box-1.internal".to_owned(),
            name: BOX.to_owned(),
            root: String::new(),
            trust_host_key_fingerprint: fingerprint.to_owned(),
        };
        // The first add stops at the host key; its operation names the key.
        let first = machines.add_ssh_machine(request("")).await.unwrap().into_inner();
        let ended = self.watch_last(&first).await;
        let Some(pb::error::Kind::HostKeyChanged(key)) = ended.error.and_then(|error| error.kind) else {
            panic!("the first add asks for the host key to be confirmed");
        };
        let added = machines.add_ssh_machine(request(&key.presented_fingerprint)).await.unwrap().into_inner();
        self.finish(&added).await;
        self.box_is(true, WAIT).await;
    }

    /// Waits until box-1's link is up (`true`) or not (`false`).
    async fn box_is(&self, up: bool, within: Duration) {
        let deadline = Instant::now() + within;
        loop {
            let listed = self.machines().await.list_machines(pb::ListMachinesRequest::default()).await.unwrap().into_inner().machines;
            let online = listed.iter().any(|machine| machine.name == BOX && machine.reachability == pb::Reachability::Online as i32);
            if online == up {
                return;
            }
            assert!(Instant::now() < deadline, "box-1's link never went {}", if up { "up" } else { "down" });
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Asks the host to dial box-1 now instead of at its next backoff. A
    /// redial already under way may refuse it as busy; the wait for the
    /// link covers that.
    async fn reconnect_box(&self) {
        let _ = self.machines().await.reconnect_machine(pb::ReconnectMachineRequest { id: Some(Self::box_id()) }).await;
    }

    /// Every state the operation's watch sends, until `done` holds for one
    /// (that one is returned) or the watch ends (the last is returned).
    async fn watch_until(&self, operation: &pb::Operation, done: impl Fn(&pb::Operation) -> bool) -> pb::Operation {
        let mut stream = self
            .operations()
            .await
            .watch_operation(pb::WatchOperationRequest { id: operation.id.clone(), from_version: 0 })
            .await
            .unwrap()
            .into_inner();
        let mut last = operation.clone();
        while let Some(next) = tokio::time::timeout(REDIAL, stream.message()).await.expect("the operation went quiet").unwrap() {
            last = next;
            if done(&last) {
                break;
            }
        }
        last
    }

    /// The last state the operation's watch sends, whatever it is.
    async fn watch_last(&self, operation: &pb::Operation) -> pb::Operation {
        self.watch_until(operation, |_| false).await
    }

    /// The operation as it ended, which must be a success.
    async fn finish(&self, operation: &pb::Operation) -> pb::Operation {
        let last = self.watch_last(operation).await;
        assert_eq!(last.state, pb::OperationState::Succeeded as i32, "{:?}", last.error);
        last
    }

    /// This host's own machine.
    async fn this_machine(&self) -> pb::MachineId {
        pb::daemon_service_client::DaemonServiceClient::new(self.channel().await)
            .get_server_info(pb::GetServerInfoRequest {})
            .await
            .unwrap()
            .into_inner()
            .machine
            .expect("the host names its machine")
    }

    fn box_id() -> pb::MachineId {
        pb::MachineId { value: BOX.to_owned() }
    }

    /// A channel of `repos` on this machine and box-1, with its setup operation.
    async fn create_channel(&self, name: &str, repos: &[(&str, &str)]) -> pb::CreateChannelResponse {
        self.channels()
            .await
            .create_channel(pb::CreateChannelRequest {
                idempotency_key: Some(pb::IdempotencyKey { value: format!("create-{name}") }),
                name: name.to_owned(),
                repos: repos.iter().map(|(owner, name)| repo_ref(owner, name)).collect(),
                machines: vec![self.this_machine().await, Self::box_id()],
            })
            .await
            .unwrap()
            .into_inner()
    }

    async fn session(&self, id: &pb::SessionId) -> pb::Session {
        self.sessions().await.get_session(pb::GetSessionRequest { id: Some(id.clone()) }).await.unwrap().into_inner()
    }

    /// Polls until the session's status satisfies `want`.
    async fn wait_session(&self, id: &pb::SessionId, what: &str, want: impl Fn(i32) -> bool) -> pb::Session {
        let deadline = Instant::now() + REDIAL;
        loop {
            let session = self.session(id).await;
            if want(session.status) {
                return session;
            }
            assert!(Instant::now() < deadline, "session {} never became {what}; it is {:?}", id.value, session.status());
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn listed(&self, channel: &pb::ChannelId) -> Vec<pb::Session> {
        self.sessions()
            .await
            .list_sessions(pb::ListSessionsRequest { channel: Some(channel.clone()), ..pb::ListSessionsRequest::default() })
            .await
            .unwrap()
            .into_inner()
            .sessions
    }

    /// The session's agent tab, asked of the host (which relays it).
    async fn agent_tab(&self, session: &pb::SessionId) -> pb::TabId {
        self.terminals()
            .await
            .list_tabs(pb::ListTabsRequest { session: Some(session.clone()) })
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

    async fn attach(&self, session: &pb::SessionId, tab: &pb::TabId) -> Attach {
        let (to, outgoing) = mpsc::channel(16);
        let open = pb::AttachOpen {
            session: Some(session.clone()),
            tab: Some(tab.clone()),
            from_offset: 0,
            read_only: false,
            cols: 0,
            rows: 0,
            window_bytes: 0,
            accept_codecs: Vec::new(),
            checkpoint_engine: String::new(),
        };
        to.send(pb::AttachClientMessage { message: Some(pb::attach_client_message::Message::Open(open)) }).await.unwrap();
        let from = self.terminals().await.attach(ReceiverStream::new(outgoing)).await.unwrap().into_inner();
        Attach { to, from, screen: Vec::new(), notices: Vec::new(), seq: 0 }
    }

    async fn events(&self) -> Streaming<pb::Event> {
        pb::event_service_client::EventServiceClient::new(self.channel().await)
            .subscribe(Request::new(pb::SubscribeRequest { topics: vec![pb::Topic::Sessions as i32], ..pb::SubscribeRequest::default() }))
            .await
            .unwrap()
            .into_inner()
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        stop(&mut self.child);
    }
}

/// An Attach stream through the host and what has come over it. The test
/// asks for no codecs, so snapshots and output are raw bytes.
struct Attach {
    to: mpsc::Sender<pb::AttachClientMessage>,
    from: Streaming<pb::AttachServerMessage>,
    screen: Vec<u8>,
    notices: Vec<pb::TerminalNotice>,
    seq: u64,
}

impl Attach {
    async fn type_text(&mut self, text: &str) {
        self.seq += 1;
        let input = pb::TerminalInput { data: text.as_bytes().to_vec(), input_seq: self.seq };
        self.to.send(pb::AttachClientMessage { message: Some(pb::attach_client_message::Message::Input(input)) }).await.unwrap();
    }

    async fn read(&mut self) {
        let message = tokio::time::timeout(REDIAL, self.from.message())
            .await
            .expect("the attach went quiet")
            .expect("the attach failed")
            .expect("the attach ended");
        match message.message.expect("a message") {
            pb::attach_server_message::Message::Snapshot(snapshot) => self.screen.extend_from_slice(&snapshot.vt),
            pb::attach_server_message::Message::Output(output) => self.screen.extend_from_slice(&output.data),
            pb::attach_server_message::Message::Notice(notice) => self.notices.push(notice),
            pb::attach_server_message::Message::State(_) | pb::attach_server_message::Message::InputAccepted(_) => {}
        }
    }

    async fn wait_for(&mut self, text: &str) {
        while !String::from_utf8_lossy(&self.screen).contains(text) {
            self.read().await;
        }
    }

    async fn wait_for_notice(&mut self, kind: pb::terminal_notice::Kind) {
        while !self.notices.iter().any(|notice| notice.kind == kind as i32) {
            self.read().await;
        }
    }
}

/// The fake machine, its socket daemon and the host that reaches it. Fields
/// drop in order: the host's links end before the daemon, the daemon before
/// the machine's folders.
struct Rig {
    host: Host,
    daemon: MachineDaemon,
    machine: FakeMachine,
}

impl Rig {
    /// Everything started, box-1 added and its link up; the git remotes
    /// `repos` (`owner/name`) exist.
    async fn new(repos: &[(&str, &str)]) -> Self {
        let machine = FakeMachine::new();
        // The machine's daemon first: the add writes the enrollment key into
        // the state folder it made, and the link reaches its socket.
        // One world: the machine clones from the same remotes the host does.
        let dir = folder();
        let world = World::new(dir.path());
        let daemon = MachineDaemon::start(machine.home(), &world).await;
        let host = Host::start(&machine, &daemon.socket, &daemon.state, dir, world).await;
        for (owner, name) in repos {
            host.world.remote(owner, name);
        }
        let rig = Self { host, daemon, machine };
        rig.host.add_box().await;
        // The add answers before the host has dialled the link; the tests
        // start from a link that is up and proven (the machine's version is
        // what its Hello said).
        rig.link_pid();
        let deadline = Instant::now() + REDIAL;
        loop {
            let listed = rig.host.machines().await.list_machines(pb::ListMachinesRequest::default()).await.unwrap().into_inner().machines;
            if listed.iter().any(|machine| machine.name == BOX && !machine.daemon_version.is_empty()) {
                break;
            }
            assert!(Instant::now() < deadline, "box-1's link never came up after the add");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        rig
    }

    fn on_box(&self, channel: &pb::Channel, relative: &str) -> PathBuf {
        self.machine.home().join("Humata").join(&channel.folder).join(relative)
    }

    /// The link process's pid, as the machine's `humatad2` wrapper wrote
    /// it. The host dials the link after the add (or a reconnect) answers,
    /// so the file may not be there yet: this waits for it.
    fn link_pid(&self) -> i32 {
        let file = self.machine.home().join("link.pid");
        let deadline = Instant::now() + REDIAL;
        loop {
            if let Some(pid) = std::fs::read_to_string(&file).ok().and_then(|text| text.trim().parse().ok()) {
                return pid;
            }
            assert!(Instant::now() < deadline, "the host never started a link to box-1");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Ends the link process, as a dropped connection does. The host sees
    /// the machine go offline and redials.
    fn kill_link(&self) {
        let pid = Pid::from_raw(self.link_pid()).expect("a link pid");
        let _ = kill_process(pid, Signal::KILL);
        // The next link writes its own; a stale pid must not be killed twice.
        let _ = std::fs::remove_file(self.machine.home().join("link.pid"));
    }

    async fn start_on_box(&self, channel: &pb::Channel, title: &str) -> pb::SessionId {
        let response = self
            .host
            .sessions()
            .await
            .start_session(start_request(channel.id.as_ref().unwrap(), &Host::box_id(), title))
            .await
            .unwrap()
            .into_inner();
        self.host.finish(&response.operation.unwrap()).await;
        response.session.unwrap()
    }
}

fn step_state(operation: &pb::Operation, key: &str) -> Option<pb::StepState> {
    operation.steps.iter().find(|step| step.key == key).and_then(|step| pb::StepState::try_from(step.state).ok())
}

/// Not offline, not stopped: the session's agent is up in some way.
fn is_live(status: i32) -> bool {
    use pb::SessionStatus as S;
    [S::Starting, S::Working, S::NeedsInput, S::Idle, S::RunningUnknownTurn, S::ProbablyIdle].iter().any(|live| *live as i32 == status)
}

fn is_offline(status: i32) -> bool {
    status == pb::SessionStatus::Offline as i32
}

/// Flow 26.1: one channel on this Mac and a machine makes the folders and
/// the main clones on both, and the operation shows progress per machine.
#[tokio::test(flavor = "multi_thread")]
async fn a_channel_on_this_mac_and_a_machine_has_folders_and_main_clones_on_both() {
    let rig = Rig::new(&[("acme", "api"), ("acme", "web")]).await;

    let made = rig.host.create_channel("Payments", &[("acme", "api"), ("acme", "web")]).await;
    let setup = rig.host.finish(made.setup.as_ref().unwrap()).await;
    let channel = made.channel.unwrap();

    for sub in ["main", "sessions", "shared"] {
        assert!(rig.host.world.channels.join(&channel.folder).join(sub).is_dir(), "{sub} is missing on this Mac");
        assert!(rig.on_box(&channel, sub).is_dir(), "{sub} is missing on {BOX}");
    }
    for name in ["api", "web"] {
        assert!(rig.host.world.channels.join(&channel.folder).join("main").join(name).join(".git").exists());
        assert!(rig.on_box(&channel, &format!("main/{name}/.git")).exists(), "{name} isn't cloned on {BOX}");
        assert_eq!(step_state(&setup, &format!("clone_main:acme/{name}")), Some(pb::StepState::Done));
        assert_eq!(step_state(&setup, &format!("clone_main:{BOX}:acme/{name}")), Some(pb::StepState::Done));
    }
    assert_eq!(step_state(&setup, &format!("record:{BOX}")), Some(pb::StepState::Done));
    // The host named the channel's id: the machine's own record carries it.
    let id = channel.id.as_ref().unwrap();
    assert!(rig.daemon.store().channel(&id.value).unwrap().is_some());
}

/// Flow 26.2: the session runs under the machine's socket daemon, the host
/// lists it with the machine's status, and typing through the host reaches
/// the agent there.
#[tokio::test(flavor = "multi_thread")]
async fn a_session_started_on_the_machine_runs_there_and_the_host_relays_its_terminal() {
    let rig = Rig::new(&[("acme", "api")]).await;
    let made = rig.host.create_channel("Payments", &[("acme", "api")]).await;
    rig.host.finish(made.setup.as_ref().unwrap()).await;
    let channel = made.channel.unwrap();

    let session = rig.start_on_box(&channel, "Fix it").await;

    let shown = rig.host.wait_session(&session, "live", is_live).await;
    assert_eq!(shown.machine, Some(Host::box_id()));
    // The holder is the machine daemon's; the host started none of its own.
    let tabs = rig.daemon.tabs();
    assert_eq!(tabs.len(), 1, "one agent tab on the machine");
    assert!(holder_answers(&tabs[0]), "the holder runs under the machine's daemon");
    assert!(stored_tabs(&rig.host.state).is_empty(), "the host ran nothing itself");
    let machine_sessions = rig.daemon.store().sessions().unwrap();
    assert_eq!(machine_sessions.len(), 1);
    assert_eq!(machine_sessions[0].id, session.value, "the machine uses the host's id for it (SL-1)");
    let listed = rig.host.listed(channel.id.as_ref().unwrap()).await;
    assert!(listed.iter().any(|found| found.id == Some(session.clone()) && is_live(found.status)));

    // Typing through the host reaches the agent on the machine, and its
    // answer comes back through the host's Attach.
    let tab = rig.host.agent_tab(&session).await;
    let mut attach = rig.host.attach(&session, &tab).await;
    attach.type_text("hello box\n").await;
    attach.wait_for("got: hello box").await;
}

/// Flow 26.9: the machine's link and daemon die and come back; the host
/// shows the machine offline and the session OFFLINE, then the session
/// again, the same one, and nothing was started twice.
#[tokio::test(flavor = "multi_thread")]
async fn a_machine_that_goes_away_and_returns_reattaches_its_session_without_starting_it_twice() {
    let mut rig = Rig::new(&[("acme", "api")]).await;
    let made = rig.host.create_channel("Payments", &[("acme", "api")]).await;
    rig.host.finish(made.setup.as_ref().unwrap()).await;
    let channel = made.channel.unwrap();
    let session = rig.start_on_box(&channel, "Fix it").await;
    rig.host.wait_session(&session, "live", is_live).await;
    let holders = rig.daemon.tabs();
    let tab = rig.host.agent_tab(&session).await;
    let mut attach = rig.host.attach(&session, &tab).await;
    attach.type_text("before\n").await;
    attach.wait_for("got: before").await;
    let mut events = rig.host.events().await;

    // The power goes: the link and the machine's daemon end, and dials fail.
    rig.machine.set("down");
    rig.kill_link();
    rig.daemon.kill();
    rig.host.box_is(false, WAIT).await;
    rig.host.wait_session(&session, "OFFLINE", is_offline).await;
    // Offline, the machine takes no start or stop from here, at once, and a
    // refused start leaves no record (CH-4).
    let refused = rig.host.sessions().await.stop_session(pb::StopSessionRequest { idempotency_key: None, id: Some(session.clone()) }).await.unwrap_err();
    assert!(matches!(error_of(&refused).kind, Some(pb::error::Kind::MachineOffline(_))));
    let refused = rig
        .host
        .sessions()
        .await
        .start_session(start_request(channel.id.as_ref().unwrap(), &Host::box_id(), "Second"))
        .await
        .unwrap_err();
    assert!(matches!(error_of(&refused).kind, Some(pb::error::Kind::MachineOffline(_))));
    assert_eq!(rig.host.listed(channel.id.as_ref().unwrap()).await.len(), 1);

    // The machine boots: its daemon is back, then the host's dial succeeds.
    rig.daemon.launch().await;
    rig.machine.clear("down");
    rig.host.reconnect_box().await;
    rig.host.box_is(true, REDIAL).await;
    let back = rig.host.wait_session(&session, "live again", is_live).await;
    assert_eq!(back.id, Some(session.clone()));

    // The feed told of it: the session went offline, then came back live.
    let mut offline = false;
    loop {
        let event = tokio::time::timeout(REDIAL, events.message()).await.expect("the feed went quiet").unwrap().expect("the feed ended");
        let Some(pb::event::Payload::Session(told)) = event.payload else { continue };
        if told.id.as_ref() != Some(&session) {
            continue;
        }
        offline |= is_offline(told.status);
        if offline && is_live(told.status) {
            break;
        }
    }

    // Nothing started twice: the same record, the same holder.
    let tabs = rig.daemon.tabs();
    assert_eq!(tabs.len(), holders.len(), "no second tab was made");
    assert_eq!(tabs[0].holder_pid, holders[0].holder_pid, "the same holder, reattached");
    assert!(holder_answers(&tabs[0]));
    let records = rig.daemon.store().sessions().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].id, session.value);
    // The attach rode out the drop and still reaches the agent.
    attach.wait_for_notice(pb::terminal_notice::Kind::Disconnected).await;
    attach.wait_for_notice(pb::terminal_notice::Kind::Reconnected).await;
    attach.type_text("after\n").await;
    attach.wait_for("got: after").await;
}

/// Flow 26.21 in the form CH-19 gives it: work for a machine that is offline
/// waits, and is done when its link returns. Here the work is a channel
/// created with the machine in its list while the machine is away.
#[tokio::test(flavor = "multi_thread")]
async fn a_channel_made_while_the_machine_is_offline_waits_then_clones_when_the_link_returns() {
    let rig = Rig::new(&[("acme", "api")]).await;
    rig.machine.set("down");
    rig.kill_link();
    rig.host.box_is(false, WAIT).await;

    let made = rig.host.create_channel("Payments", &[("acme", "api")]).await;
    let channel = made.channel.unwrap();
    let key = format!("clone_main:{BOX}:acme/api");

    // This Mac finishes its part; the machine's step waits, offline.
    let waiting = rig
        .host
        .watch_until(made.setup.as_ref().unwrap(), |op| {
            step_state(op, "clone_main:acme/api") == Some(pb::StepState::Done) && step_state(op, &key) == Some(pb::StepState::Waiting)
        })
        .await;
    assert_eq!(waiting.state, pb::OperationState::Running as i32);
    assert_eq!(step_state(&waiting, &key), Some(pb::StepState::Waiting));
    assert!(!rig.on_box(&channel, "main/api").exists(), "nothing is cloned on an offline machine");

    // The link returns: the machine clones and the step ends done.
    rig.machine.clear("down");
    rig.host.reconnect_box().await;
    rig.host.box_is(true, REDIAL).await;
    let done = rig.host.finish(made.setup.as_ref().unwrap()).await;

    assert_eq!(step_state(&done, &key), Some(pb::StepState::Done));
    assert!(rig.on_box(&channel, "main/api/.git").exists());
}

/// Flow 26.21: a repo added to a channel while its machine is offline waits
/// there and clones when the link returns. AddRepo takes the path
/// CreateChannel takes, with the same per-machine steps (D157, D163).
#[tokio::test(flavor = "multi_thread")]
async fn a_repo_added_while_the_machine_is_offline_waits_then_clones_when_the_link_returns() {
    let rig = Rig::new(&[("acme", "api"), ("acme", "web")]).await;
    let made = rig.host.create_channel("Payments", &[("acme", "api")]).await;
    rig.host.finish(made.setup.as_ref().unwrap()).await;
    let channel = made.channel.unwrap();

    rig.machine.set("down");
    rig.kill_link();
    rig.host.box_is(false, WAIT).await;
    let added = rig
        .host
        .channels()
        .await
        .add_repo(pb::AddRepoRequest {
            idempotency_key: Some(pb::IdempotencyKey { value: "add-web".to_owned() }),
            channel: channel.id.clone(),
            repo: Some(repo_ref("acme", "web")),
        })
        .await
        .unwrap()
        .into_inner();
    let key = format!("clone_main:{BOX}:acme/web");

    // This Mac clones at once; the machine's step waits.
    let waiting = rig
        .host
        .watch_until(&added, |op| step_state(op, "clone_main:acme/web") == Some(pb::StepState::Done) && step_state(op, &key) == Some(pb::StepState::Waiting))
        .await;
    assert_eq!(waiting.state, pb::OperationState::Running as i32);
    assert!(!rig.on_box(&channel, "main/web").exists());

    rig.machine.clear("down");
    rig.host.reconnect_box().await;
    rig.host.box_is(true, REDIAL).await;
    let done = rig.host.finish(&added).await;

    assert_eq!(step_state(&done, &key), Some(pb::StepState::Done));
    assert!(rig.on_box(&channel, "main/web/.git").exists());
    // The repo that was already there stays as it was.
    assert!(rig.on_box(&channel, "main/api/.git").exists());
}
