//! Other machines' conversations reach the host (LD-8, LD-23, LD-24; TR-4;
//! D139, D144, D146) through a real host daemon, a real machine-role
//! socket daemon and the peer link between them, as in tests/remote.rs.
//!
//! The machine has a world of its own under its home: its agent writes its
//! conversation files there, where the host never looks. So whatever the
//! host finds came over the machine's link. What the tests rely on:
//!
//! - what a session's agent on the machine writes is found by the host's
//!   SearchSessions, naming that machine, and read back by ReadTurns;
//! - while the machine is offline the host still answers with what it had,
//!   "as of" its last sync, and nothing written since;
//! - when the link comes back the host catches up from where it stopped,
//!   with no event from the session needed: the link coming up is enough;
//! - the host keeps where it stopped across its own restart, so nothing
//!   is mirrored twice.
//!
//! Not here: LD-21's scrubbing on the machine. No call sets a host
//! variable yet (MachineService.SetHostVariable is not built); the mirror
//! crate's tests cover the scrubbing itself until it is.
//!
//! Enabling is a move to `tests/remote_history.rs` with the REMOTE_MIRROR
//! seam (HistoryService.ReadMirror and the host's follower).

mod common;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use common::{AGENT_NAME, WAIT, World, binary, connect, folder, repo_ref, start_request};
use humata2_fake_machine::FakeMachine;
use humata2_grpc::humata::v1 as pb;
use humata2_store::Store;
use pb::history_service_client::HistoryServiceClient;
use rustix::process::{Pid, Signal, kill_process};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tokio::net::UnixStream;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::Channel;
use tonic::Streaming;

const FILES: [&str; 4] = ["humatad2", "humata2_holder", "humata", "stack"];
const BOX: &str = "box-1";
/// The host redials after a backoff of 5 s, then 10 s while dials fail.
const REDIAL: Duration = Duration::from_secs(60);

// ---------------------------------------------------------------------------
// The lines an agent writes to its conversation file (Claude's JSONL).

fn line(kind: &str, content: Value, minute: u32) -> String {
    json!({
        "type": kind,
        "isSidechain": false,
        "message": {"role": kind, "content": content},
        "timestamp": format!("2026-11-10T09:{minute:02}:00.000Z"),
    })
    .to_string()
}

/// A prompt the user typed, `minute` minutes into the morning.
fn user(text: &str, minute: u32) -> String {
    line("user", json!(text), minute)
}

/// What the agent said.
fn says(text: &str, minute: u32) -> String {
    line("assistant", json!([{"type": "text", "text": text}]), minute)
}

/// Adds `lines` to the end of `file` in one write, as the agent going on
/// with its conversation.
fn append(file: &Path, lines: &[String]) {
    let joined: String = lines.iter().map(|line| format!("{line}\n")).collect();
    let mut open = std::fs::OpenOptions::new().append(true).open(file).unwrap();
    open.write_all(joined.as_bytes()).unwrap();
}

/// Waits until `file` holds `text` in a finished line.
async fn written(file: &Path, text: &str) {
    let deadline = Instant::now() + WAIT;
    loop {
        let now = std::fs::read_to_string(file).unwrap_or_default();
        if now.contains(text) && now.ends_with('\n') {
            return;
        }
        assert!(Instant::now() < deadline, "{} never held {text:?}; it holds {now:?}", file.display());
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

// ---------------------------------------------------------------------------
// The machine, its daemon and the host (after tests/remote.rs).

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
            json!({
                "path": format!("linux-x86_64/{file}"),
                "sha256": Sha256::digest(body.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect::<String>(),
            }),
        );
    }
    std::fs::write(root.join("manifest.json"), json!({ "platforms": { "linux-x86_64": entries } }).to_string()).unwrap();
    root
}

fn stop(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// The machine's socket daemon, under the machine's home, with the
/// machine's own agent: its conversation files are in `world.conv` under
/// that home.
struct MachineDaemon {
    child: Option<Child>,
    socket: PathBuf,
    state: PathBuf,
    world: World,
    /// Holds the socket's short path.
    _dir: TempDir,
}

impl MachineDaemon {
    async fn start(home: &Path, host_world: &World) -> Self {
        let dir = folder();
        let world = World::new(&home.join("w"));
        let mut args = host_world.args();
        // The machine keeps its channels under its own home, and runs its
        // own agent; it clones from the same remotes the host does.
        args[1] = home.join("Humata").display().to_string();
        let agents = args.iter().position(|arg| arg == "--agents").expect("the world names its agents") + 1;
        args[agents] = world.agents.display().to_string();
        // See tests/remote.rs: a holder's socket path must stay short.
        let short = dir.path().join("h");
        std::fs::create_dir(&short).unwrap();
        std::os::unix::fs::symlink(&short, home.join(".humata2")).unwrap();
        let socket = dir.path().join("m.sock");
        let state = short.join("state");
        let child = Command::new(binary("HUMATAD2_RELEASE"))
            .args(["serve", "--role", "machine", "--socket"])
            .arg(&socket)
            .arg("--state")
            .arg(&state)
            .arg("--root")
            .arg(home)
            .arg("--holder")
            .arg(binary("HUMATA2_HOLDER"))
            .arg("--shell")
            .arg("/bin/sh")
            .arg("--shell-arg=-i")
            .args(&args)
            .env("HOME", home)
            .spawn()
            .unwrap();
        let deadline = Instant::now() + WAIT;
        while UnixStream::connect(&socket).await.is_err() {
            assert!(Instant::now() < deadline, "the machine's humatad2 never started listening");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Self { child: Some(child), socket, state, world, _dir: dir }
    }

    fn store(&self) -> Store {
        Store::open(self.state.join("store")).unwrap()
    }

    /// The file the agent of `session` writes its conversation to, on the
    /// machine.
    fn conversation(&self, session: &pb::SessionId) -> PathBuf {
        let agent = self.store().agent(&session.value).unwrap().expect("the session's agent ran on the machine");
        let file = agent.launch.conversation_file;
        assert!(file.starts_with(&self.world.conv), "the machine's agent writes under the machine's home: {}", file.display());
        file
    }
}

impl Drop for MachineDaemon {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            stop(&mut child);
        }
        // Holders outlive their daemon on purpose; these ones don't.
        let Ok(store) = Store::open(self.state.join("store")) else { return };
        for tab in store.tabs().unwrap_or_default() {
            if let Some(pid) = i32::try_from(tab.holder_pid).ok().and_then(Pid::from_raw) {
                let _ = kill_process(pid, Signal::TERM);
            }
        }
    }
}

/// The admin host daemon, with the fake machine's `ssh` first on its PATH.
/// It can be killed and started again on the same state, as a crash does.
struct Host {
    child: Option<Child>,
    command: Vec<std::ffi::OsString>,
    path: String,
    socket: PathBuf,
    _dir: TempDir,
    world: World,
}

impl Host {
    async fn start(machine: &FakeMachine, daemon: &MachineDaemon, dir: TempDir, world: World) -> Self {
        let socket = dir.path().join("d.sock");
        let mut command: Vec<std::ffi::OsString> = vec!["serve".into(), "--admin".into(), "--socket".into(), socket.clone().into()];
        command.extend(["--state".into(), dir.path().join("state").into()]);
        command.extend(["--root".into(), dir.path().as_os_str().to_owned()]);
        command.extend(["--holder".into(), binary("HUMATA2_HOLDER").into()]);
        command.extend(["--shell".into(), "/bin/sh".into(), "--shell-arg=-i".into()]);
        command.extend(["--tools".into(), bundle(dir.path(), &daemon.socket, &daemon.state).into()]);
        command.extend(["--ssh-config".into(), dir.path().join("ssh_config").into()]);
        command.extend(["--skills-home".into(), dir.path().join("home").into()]);
        command.extend(world.args().into_iter().map(Into::into));
        let mut host = Self {
            child: None,
            command,
            path: format!("{}:/usr/bin:/bin", machine.ssh_dir().display()),
            socket,
            _dir: dir,
            world,
        };
        host.launch().await;
        host
    }

    async fn launch(&mut self) {
        let child = Command::new(binary("HUMATAD2")).args(&self.command).env("PATH", &self.path).spawn().unwrap();
        self.child = Some(child);
        let deadline = Instant::now() + WAIT;
        while UnixStream::connect(&self.socket).await.is_err() {
            assert!(Instant::now() < deadline, "humatad2 never started listening");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// The crash: the host ends at once.
    fn kill(&mut self) {
        if let Some(mut child) = self.child.take() {
            stop(&mut child);
            let deadline = Instant::now() + WAIT;
            while std::os::unix::net::UnixStream::connect(&self.socket).is_ok() {
                assert!(Instant::now() < deadline, "a killed humatad2's socket never stopped answering");
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }

    async fn channel(&self) -> Channel {
        connect(&self.socket).await
    }

    async fn machines(&self) -> pb::machine_service_client::MachineServiceClient<Channel> {
        pb::machine_service_client::MachineServiceClient::new(self.channel().await)
    }

    async fn history(&self) -> HistoryServiceClient<Channel> {
        HistoryServiceClient::new(self.channel().await)
    }

    async fn watch_last(&self, operation: &pb::Operation) -> pb::Operation {
        let mut stream = pb::operation_service_client::OperationServiceClient::new(self.channel().await)
            .watch_operation(pb::WatchOperationRequest { id: operation.id.clone(), from_version: 0 })
            .await
            .unwrap()
            .into_inner();
        let mut last = operation.clone();
        while let Some(next) = tokio::time::timeout(REDIAL, stream.message()).await.expect("the operation went quiet").unwrap() {
            last = next;
        }
        last
    }

    async fn finish(&self, operation: &pb::Operation) {
        let last = self.watch_last(operation).await;
        assert_eq!(last.state, pb::OperationState::Succeeded as i32, "{:?}", last.error);
    }

    fn box_id() -> pb::MachineId {
        pb::MachineId { value: BOX.to_owned() }
    }

    async fn this_machine(&self) -> pb::MachineId {
        pb::daemon_service_client::DaemonServiceClient::new(self.channel().await)
            .get_server_info(pb::GetServerInfoRequest {})
            .await
            .unwrap()
            .into_inner()
            .machine
            .expect("the host names its machine")
    }

    /// Adds box-1 (confirming its host key).
    async fn add_box(&self) {
        let mut machines = self.machines().await;
        let request = |fingerprint: &str| pb::AddSshMachineRequest {
            idempotency_key: Some(pb::IdempotencyKey { value: format!("add-{fingerprint}") }),
            ssh_host: "me@box-1.internal".to_owned(),
            name: BOX.to_owned(),
            root: String::new(),
            trust_host_key_fingerprint: fingerprint.to_owned(),
        };
        let first = machines.add_ssh_machine(request("")).await.unwrap().into_inner();
        let ended = self.watch_last(&first).await;
        let Some(pb::error::Kind::HostKeyUnknown(key)) = ended.error.and_then(|error| error.kind) else {
            panic!("the first add asks for the host key to be confirmed");
        };
        let added = machines.add_ssh_machine(request(&key.presented_fingerprint)).await.unwrap().into_inner();
        self.finish(&added).await;
    }

    /// Waits until box-1's link is up and proven (`true`), or down.
    async fn box_is(&self, up: bool, within: Duration) {
        let deadline = Instant::now() + within;
        loop {
            let listed = self.machines().await.list_machines(pb::ListMachinesRequest::default()).await.unwrap().into_inner().machines;
            let online = listed
                .iter()
                .any(|machine| machine.name == BOX && machine.reachability == pb::Reachability::Online as i32 && !machine.daemon_version.is_empty());
            if online == up {
                return;
            }
            assert!(Instant::now() < deadline, "box-1's link never went {}", if up { "up" } else { "down" });
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn reconnect_box(&self) {
        let _ = self.machines().await.reconnect_machine(pb::ReconnectMachineRequest { id: Some(Self::box_id()) }).await;
    }

    /// The session's agent tab, asked of the host (which relays it).
    async fn agent_tab(&self, session: &pb::SessionId) -> pb::TabId {
        pb::terminal_service_client::TerminalServiceClient::new(self.channel().await)
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

    async fn attach(&self, session: &pb::SessionId) -> Attach {
        let tab = self.agent_tab(session).await;
        let (to, outgoing) = mpsc::channel(16);
        let open = pb::AttachOpen {
            session: Some(session.clone()),
            tab: Some(tab),
            from_offset: 0,
            read_only: false,
            cols: 0,
            rows: 0,
            window_bytes: 0,
            accept_codecs: Vec::new(),
            checkpoint_engine: String::new(),
            machine: None,
        };
        to.send(pb::AttachClientMessage { message: Some(pb::attach_client_message::Message::Open(open)) }).await.unwrap();
        let from = pb::terminal_service_client::TerminalServiceClient::new(self.channel().await)
            .attach(ReceiverStream::new(outgoing))
            .await
            .unwrap()
            .into_inner();
        Attach { to, from, screen: Vec::new(), seq: 0 }
    }

    /// Every hit of the user's search for `text`, all pages.
    async fn search(&self, text: &str) -> Vec<pb::SearchHit> {
        let mut client = self.history().await;
        let mut request = pb::SearchSessionsRequest { query: text.to_owned(), ..Default::default() };
        let mut hits = Vec::new();
        for _ in 0..100 {
            let response = client.search_sessions(request.clone()).await.unwrap().into_inner();
            hits.extend(response.hits);
            let next = response.page.map(|page| page.next_page_token).unwrap_or_default();
            if next.is_empty() {
                return hits;
            }
            request.page = Some(pb::PageRequest { page_size: 0, page_token: next });
        }
        panic!("the search never reached a last page");
    }

    /// The hits for `text` once there are some, within `REDIAL`: the
    /// mirror follows the machine, it isn't asked to.
    async fn found(&self, text: &str) -> Vec<pb::SearchHit> {
        let deadline = Instant::now() + REDIAL;
        loop {
            let hits = self.search(text).await;
            if !hits.is_empty() {
                return hits;
            }
            assert!(Instant::now() < deadline, "the host never found {text:?} of the machine's conversation");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Waits until the host shows `session` as probably idle: its agent's
    /// output went quiet.
    async fn quiet(&self, session: &pb::SessionId) {
        let deadline = Instant::now() + REDIAL;
        loop {
            let shown = pb::session_service_client::SessionServiceClient::new(self.channel().await)
                .get_session(pb::GetSessionRequest { id: Some(session.clone()) })
                .await
                .unwrap()
                .into_inner();
            if shown.status == pb::SessionStatus::ProbablyIdle as i32 {
                return;
            }
            assert!(Instant::now() < deadline, "the host never showed the session's agent as quiet");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// The texts of every entry the host's mirror holds for `session`.
    async fn texts(&self, session: &pb::SessionId) -> Vec<String> {
        let mut client = self.history().await;
        let mut request = pb::ReadTurnsRequest { session: Some(session.clone()), first_turn: None, last_turn: None, page: None };
        let mut texts = Vec::new();
        for _ in 0..100 {
            let response = client.read_turns(request.clone()).await.unwrap().into_inner();
            texts.extend(response.entries.into_iter().map(|entry| entry.text));
            let next = response.page.map(|page| page.next_page_token).unwrap_or_default();
            if next.is_empty() {
                return texts;
            }
            request.page = Some(pb::PageRequest { page_size: 0, page_token: next });
        }
        panic!("the read never reached a last page");
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        self.kill();
    }
}

/// An Attach stream through the host. The test asks for no codecs.
struct Attach {
    to: mpsc::Sender<pb::AttachClientMessage>,
    from: Streaming<pb::AttachServerMessage>,
    screen: Vec<u8>,
    seq: u64,
}

impl Attach {
    async fn type_text(&mut self, text: &str) {
        self.seq += 1;
        let input = pb::TerminalInput { data: text.as_bytes().to_vec(), input_seq: self.seq };
        self.to.send(pb::AttachClientMessage { message: Some(pb::attach_client_message::Message::Input(input)) }).await.unwrap();
    }

    async fn wait_for(&mut self, text: &str) {
        while !String::from_utf8_lossy(&self.screen).contains(text) {
            let message = tokio::time::timeout(REDIAL, self.from.message())
                .await
                .expect("the attach went quiet")
                .expect("the attach failed")
                .expect("the attach ended");
            match message.message.expect("a message") {
                pb::attach_server_message::Message::Snapshot(snapshot) => self.screen.extend_from_slice(&snapshot.vt),
                pb::attach_server_message::Message::Output(output) => self.screen.extend_from_slice(&output.data),
                _ => {}
            }
        }
    }
}

/// Fields drop in order: the host's links end before the machine's daemon,
/// the daemon before the machine's folders.
struct Rig {
    host: Host,
    daemon: MachineDaemon,
    machine: FakeMachine,
}

impl Rig {
    /// Everything started, box-1 added and its link up, and a channel on
    /// this Mac and box-1 with one repository.
    async fn new() -> (Self, pb::Channel) {
        let machine = FakeMachine::new();
        let dir = folder();
        let world = World::new(dir.path());
        let daemon = MachineDaemon::start(machine.home(), &world).await;
        let host = Host::start(&machine, &daemon, dir, world).await;
        host.world.remote("acme", "api");
        let rig = Self { host, daemon, machine };
        rig.host.add_box().await;
        rig.host.box_is(true, REDIAL).await;
        let made = pb::channel_service_client::ChannelServiceClient::new(rig.host.channel().await)
            .create_channel(pb::CreateChannelRequest {
                idempotency_key: Some(pb::IdempotencyKey { value: "create-payments".to_owned() }),
                name: "Payments".to_owned(),
                repos: vec![repo_ref("acme", "api")],
                machines: vec![rig.host.this_machine().await, Host::box_id()],
            })
            .await
            .unwrap()
            .into_inner();
        rig.host.finish(made.setup.as_ref().unwrap()).await;
        (rig, made.channel.unwrap())
    }

    async fn start_on_box(&self, channel: &pb::Channel, title: &str) -> pb::SessionId {
        let response = pb::session_service_client::SessionServiceClient::new(self.host.channel().await)
            .start_session(start_request(channel.id.as_ref().unwrap(), &Host::box_id(), title))
            .await
            .unwrap()
            .into_inner();
        self.host.finish(&response.operation.unwrap()).await;
        response.session.unwrap()
    }

    /// A session on box-1 whose agent has the prompt `text` in its
    /// conversation, typed to it through the host, and the file the agent
    /// keeps it in on the machine.
    async fn session_with(&self, channel: &pb::Channel, text: &str) -> (pb::SessionId, PathBuf) {
        let session = self.start_on_box(channel, "Fix login").await;
        let mut attach = self.host.attach(&session).await;
        attach.wait_for("agent up").await;
        attach.type_text(&format!("{}\r", user(text, 0))).await;
        attach.wait_for("got: ").await;
        let file = self.daemon.conversation(&session);
        written(&file, text).await;
        (session, file)
    }

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

    /// The machine drops off the network: its link ends and dials fail.
    /// Its daemon and the session's agent keep running there.
    async fn go_offline(&self) {
        self.machine.set("down");
        let pid = Pid::from_raw(self.link_pid()).expect("a link pid");
        let _ = kill_process(pid, Signal::KILL);
        let _ = std::fs::remove_file(self.machine.home().join("link.pid"));
        self.host.box_is(false, WAIT).await;
    }

    /// The machine is reachable again and the host's link to it is up.
    async fn come_back(&self) {
        self.machine.clear("down");
        self.host.reconnect_box().await;
        self.host.box_is(true, REDIAL).await;
    }
}

fn machine_of(hit: &pb::SearchHit) -> &str {
    hit.machine.as_ref().map_or("", |machine| machine.value.as_str())
}

/// When a hit's session last caught up, in an order that sorts by time.
fn as_of(hit: &pb::SearchHit) -> (i64, i32) {
    let at = hit.as_of.as_ref().expect("a hit says as of when");
    (at.seconds, at.nanos)
}

// ---------------------------------------------------------------------------

/// LD-8, LD-24: a prompt typed to an agent on another machine is found on
/// the host, and the hit names that machine; the host reads it back too.
#[tokio::test(flavor = "multi_thread")]
async fn a_prompt_on_another_machine_is_found_on_the_host_naming_that_machine() {
    let (rig, channel) = Rig::new().await;

    let (session, _file) = rig.session_with(&channel, "reconcile the pangolin invoices").await;

    let hits = rig.host.found("pangolin").await;
    assert_eq!(hits.len(), 1, "{hits:?}");
    let hit = &hits[0];
    assert_eq!(hit.session, Some(session.clone()));
    assert_eq!(hit.channel, channel.id);
    assert_eq!(machine_of(hit), BOX);
    assert_eq!(hit.machine_name, BOX);
    assert_eq!(hit.agent, AGENT_NAME);
    assert_eq!(hit.turn, 1);
    assert!(hit.as_of.is_some(), "a remote hit says as of when it was synced");
    assert_eq!(rig.host.texts(&session).await, ["reconcile the pangolin invoices"]);
}

/// LD-8, LD-23, TR-4: while the machine is offline the host answers with
/// what it had, as of its last sync; when the link comes back it catches
/// up from where it stopped, with nothing from the session needed.
#[tokio::test(flavor = "multi_thread")]
async fn a_machine_that_was_offline_catches_up_when_its_link_returns() {
    let (rig, channel) = Rig::new().await;
    let (session, file) = rig.session_with(&channel, "reconcile the pangolin invoices").await;
    let before = rig.host.found("pangolin").await;
    assert_eq!(before.len(), 1, "{before:?}");

    rig.go_offline().await;
    append(&file, &[user("ship the numbat release", 8), says("Tagged the numbat release", 9)]);

    // Offline: what the host had is still found, as of the same sync, and
    // nothing written since has crossed.
    let offline = rig.host.search("pangolin").await;
    assert_eq!(offline.len(), 1, "{offline:?}");
    assert_eq!(as_of(&offline[0]), as_of(&before[0]));
    assert!(rig.host.search("numbat").await.is_empty(), "nothing crosses while the machine is offline");

    rig.come_back().await;

    let gained = rig.host.found("numbat").await;
    assert_eq!(gained.len(), 1, "{gained:?}");
    assert_eq!(gained[0].session, Some(session.clone()));
    assert_eq!(machine_of(&gained[0]), BOX);
    assert_eq!(gained[0].turn, 2);
    assert!(as_of(&gained[0]) >= as_of(&before[0]), "the catch-up moved the session's as-of");
    // The first turn is there once, and the new entries follow it.
    assert_eq!(rig.host.search("pangolin").await.len(), 1);
    assert_eq!(
        rig.host.texts(&session).await,
        ["reconcile the pangolin invoices", "ship the numbat release", "Tagged the numbat release"]
    );
}

/// LD-8, TR-4: the host keeps where each remote session's mirror stopped
/// across its own crash. What the agent wrote while the host was down is
/// caught up once the link is back, and nothing is mirrored twice.
#[tokio::test(flavor = "multi_thread")]
async fn a_host_restart_keeps_where_the_remote_mirror_stopped() {
    let (mut rig, channel) = Rig::new().await;
    let (session, file) = rig.session_with(&channel, "reconcile the pangolin invoices").await;
    assert_eq!(rig.host.found("pangolin").await.len(), 1);

    rig.host.kill();
    append(&file, &[user("ship the numbat release", 8), says("Tagged the numbat release", 9)]);
    rig.host.launch().await;
    rig.host.box_is(true, REDIAL).await;

    let gained = rig.host.found("numbat").await;
    assert_eq!(gained.len(), 1, "{gained:?}");
    assert_eq!(gained[0].turn, 2);
    assert_eq!(rig.host.search("pangolin").await.len(), 1, "the first turn wasn't mirrored again");
    assert_eq!(
        rig.host.texts(&session).await,
        ["reconcile the pangolin invoices", "ship the numbat release", "Tagged the numbat release"]
    );
}

/// N1 on another machine: an agent there may write the end of a turn after
/// the last event of its session. The machine's own daemon sees its file
/// change and tells the host, which reads it, with no link coming up and
/// nothing polled.
#[tokio::test(flavor = "multi_thread")]
async fn what_an_agent_on_another_machine_writes_after_its_sessions_last_event_reaches_the_host() {
    let (rig, channel) = Rig::new().await;
    let (session, file) = rig.session_with(&channel, "reconcile the pangolin invoices").await;
    assert_eq!(rig.host.found("pangolin").await.len(), 1);
    // The output went quiet: the session's last event, and the host's read
    // that it triggered is done.
    rig.host.quiet(&session).await;
    tokio::time::sleep(Duration::from_secs(1)).await;

    // The reply lands in the file after that, with no output and no event.
    append(&file, &[says("Reconciled the axolotl ledger", 1)]);

    let deadline = Instant::now() + WAIT;
    let hits = loop {
        let hits = rig.host.search("axolotl").await;
        if !hits.is_empty() {
            break hits;
        }
        assert!(Instant::now() < deadline, "the host never read what the machine's agent wrote after its last event");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].session, Some(session.clone()));
    assert_eq!(machine_of(&hits[0]), BOX);
    assert_eq!(
        rig.host.texts(&session).await,
        ["reconcile the pangolin invoices", "Reconciled the axolotl ledger"]
    );
}
