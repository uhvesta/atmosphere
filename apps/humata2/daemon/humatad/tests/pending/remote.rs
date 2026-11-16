//! A channel across machines and a session on another machine (CH-5, CH-7,
//! CH-19, SL-1, SL-3, PER-11, HO-5, HO-6; D139, D144; flows 26.1, 26.2, 26.9,
//! 26.21), through a real host daemon and a fake machine whose `humatad2` is
//! the real binary.
//!
//! Written against the shape after every seam of the MACH remote-sessions
//! report is applied (`daemon/humatad/src/remote_work.rs` wired into
//! ChannelService and SessionService, the machine honouring
//! `humata-host-assigned-id`, remote reports merged into GetSession) and the
//! relay stages enabled. Assumptions, to settle when this file is enabled
//! (enabling is a move to `tests/remote.rs`):
//!
//! - `FakeMachine` gains `serve_args(&[String])` (the flags its machine-role
//!   socket daemon starts with, so `--channels`, `--agents`,
//!   `--default-agent` and `--remote-template` from `World::args` apply
//!   there too, with the machine's channels root under `machine.home()`),
//!   `kill_daemon()` and `start_daemon()` (the socket daemon dies and comes
//!   back, as a reboot or crash would, holders surviving a daemon crash but
//!   not a reboot), and `state_folder()` (the machine daemon's `--state`).
//! - The host serves `--channels`, `--agents`, `--default-agent` and
//!   `--remote-template` from the same `World`, so the file-remote repos
//!   clone on both sides.
//! - `ChannelService.AddRepo` and `GetGrid` are built on the host (flow
//!   26.21) and an added repo takes the same path as `CreateChannel`.
//! - The local machine shows in `ListMachines` under its own name.

mod common;

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use common::{WAIT, World, binary, connect, folder, repo_ref};
use humata2_fake_machine::FakeMachine;
use humata2_grpc::humata::v1 as pb;
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tokio::net::UnixStream;
use tonic::transport::Channel;

const FILES: [&str; 3] = ["humatad2", "humata2_holder", "humata"];
const BOX: &str = "box-1";

/// A tools bundle for linux-x86_64 whose humatad2 runs the real one.
fn bundle(dir: &Path) -> PathBuf {
    let root = dir.join("tools");
    std::fs::create_dir_all(root.join("linux-x86_64")).unwrap();
    let mut entries = serde_json::Map::new();
    for file in FILES {
        let body = if file == "humatad2" {
            format!("#!/bin/sh\nexec '{}' \"$@\"\n", binary("HUMATAD2_RELEASE").display())
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
    std::fs::write(
        root.join("manifest.json"),
        serde_json::json!({ "platforms": { "linux-x86_64": entries } }).to_string(),
    )
    .unwrap();
    root
}

/// The admin host daemon serving `world`, with the fake machine's `ssh` first
/// on its PATH.
struct Host {
    child: Child,
    socket: PathBuf,
    dir: TempDir,
    world: World,
}

impl Host {
    async fn start(machine: &FakeMachine) -> Self {
        let dir = folder();
        let world = World::new(dir.path());
        machine.serve_args(&world.args());
        Self::start_in(machine, dir, world).await
    }

    async fn start_in(machine: &FakeMachine, dir: TempDir, world: World) -> Self {
        let socket = dir.path().join("d.sock");
        let child = Command::new(binary("HUMATAD2"))
            .arg("serve")
            .arg("--admin")
            .arg("--socket")
            .arg(&socket)
            .arg("--state")
            .arg(dir.path().join("state"))
            .arg("--holder")
            .arg(binary("HUMATA2_HOLDER"))
            .arg("--tools")
            .arg(bundle(dir.path()))
            .arg("--ssh-config")
            .arg(dir.path().join("ssh_config"))
            .args(world.args())
            .env("PATH", format!("{}:/usr/bin:/bin", machine.ssh_dir().display()))
            .spawn()
            .unwrap();
        let deadline = Instant::now() + WAIT;
        while UnixStream::connect(&socket).await.is_err() {
            assert!(Instant::now() < deadline, "humatad2 never started listening");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Self { child, socket, dir, world }
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
        let fingerprint = key.presented_fingerprint;
        let added = machines.add_ssh_machine(request(&fingerprint)).await.unwrap().into_inner();
        self.finish(&added).await;
        self.box_is(true).await;
    }

    /// Waits until box-1's link is up (`true`) or down (`false`).
    async fn box_is(&self, up: bool) {
        let deadline = Instant::now() + WAIT;
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

    /// The last state the operation's watch sends, whatever it is.
    async fn watch_last(&self, operation: &pb::Operation) -> pb::Operation {
        let mut stream = self
            .operations()
            .await
            .watch_operation(pb::WatchOperationRequest { id: operation.id.clone(), from_version: 0 })
            .await
            .unwrap()
            .into_inner();
        let mut last = operation.clone();
        while let Some(next) = tokio::time::timeout(Duration::from_secs(60), stream.message()).await.expect("the operation finished").unwrap() {
            last = next;
        }
        last
    }

    /// The operation as it ended, which must be a success.
    async fn finish(&self, operation: &pb::Operation) -> pb::Operation {
        let last = self.watch_last(operation).await;
        assert_eq!(last.state, pb::OperationState::Succeeded as i32, "{:?}", last.error);
        last
    }

    async fn local_machine(&self) -> pb::MachineId {
        let listed = self.machines().await.list_machines(pb::ListMachinesRequest::default()).await.unwrap().into_inner().machines;
        listed.into_iter().find(|machine| machine.name != BOX).and_then(|machine| machine.id).expect("the local machine is listed")
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
                machines: vec![self.local_machine().await, Self::box_id()],
            })
            .await
            .unwrap()
            .into_inner()
    }

    async fn session(&self, id: &pb::SessionId) -> pb::Session {
        self.sessions().await.get_session(pb::GetSessionRequest { id: Some(id.clone()) }).await.unwrap().into_inner()
    }

    async fn wait_status(&self, id: &pb::SessionId, status: pb::SessionStatus) -> pb::Session {
        let deadline = Instant::now() + Duration::from_secs(90);
        loop {
            let session = self.session(id).await;
            if session.status == status as i32 {
                return session;
            }
            assert!(Instant::now() < deadline, "session {} never became {status:?}; it is {:?}", id.value, session.status());
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn step_state(operation: &pb::Operation, key: &str) -> Option<pb::StepState> {
    operation.steps.iter().find(|step| step.key == key).and_then(|step| pb::StepState::try_from(step.state).ok())
}

fn on_machine(machine: &FakeMachine, channel: &pb::Channel, relative: &str) -> PathBuf {
    machine.home().join("Humata").join(&channel.folder).join(relative)
}

/// Starts a session titled `title` on box-1 and returns its id.
async fn start_on_box(host: &Host, channel: &pb::Channel, title: &str) -> pb::SessionId {
    let response = host
        .sessions()
        .await
        .start_session(common::start_request(channel.id.as_ref().unwrap(), &Host::box_id(), title))
        .await
        .unwrap()
        .into_inner();
    host.finish(&response.operation.unwrap()).await;
    response.session.unwrap()
}

/// Flow 26.1: one channel on this Mac and a machine makes the folders and
/// the main clones on both, and the operation shows progress per machine.
#[tokio::test(flavor = "multi_thread")]
async fn a_channel_on_this_mac_and_a_machine_has_folders_and_main_clones_on_both() {
    let machine = FakeMachine::new();
    let host = Host::start(&machine).await;
    host.world.remote("acme", "api");
    host.add_box().await;

    let made = host.create_channel("Payments", &[("acme", "api")]).await;
    let setup = host.finish(made.setup.as_ref().unwrap()).await;
    let channel = made.channel.unwrap();

    for sub in ["main", "sessions", "shared"] {
        assert!(host.world.channels.join(&channel.folder).join(sub).is_dir(), "{sub} is missing on this Mac");
        assert!(on_machine(&machine, &channel, sub).is_dir(), "{sub} is missing on {BOX}");
    }
    assert!(on_machine(&machine, &channel, "main/api/.git").exists());
    assert_eq!(step_state(&setup, "clone_main:acme/api"), Some(pb::StepState::Done));
    assert_eq!(step_state(&setup, &format!("clone_main:{BOX}:acme/api")), Some(pb::StepState::Done));
    // The host named the channel's id: the machine's row carries the same one.
    let there = humata2_store::Store::open(machine.state_folder().join("store")).unwrap();
    assert!(there.channel(&channel.id.as_ref().unwrap().value).unwrap().is_some());
}

/// Flow 26.2: the session runs under the machine's socket daemon, and the
/// host lists it with the machine's status.
#[tokio::test(flavor = "multi_thread")]
async fn a_session_started_on_the_machine_runs_there_and_the_host_shows_its_status() {
    let machine = FakeMachine::new();
    let host = Host::start(&machine).await;
    host.world.remote("acme", "api");
    host.add_box().await;
    let made = host.create_channel("Payments", &[("acme", "api")]).await;
    host.finish(made.setup.as_ref().unwrap()).await;
    let channel = made.channel.unwrap();

    let session = start_on_box(&host, &channel, "Fix it").await;

    let shown = host.wait_status(&session, pb::SessionStatus::Working).await;
    assert_eq!(shown.machine, Some(Host::box_id()));
    let tabs = common::stored_tabs(&machine.state_folder());
    assert_eq!(tabs.len(), 1, "one agent tab on the machine");
    assert!(common::holder_answers(&tabs[0]), "the holder runs under the machine's daemon");
    // The host's own record has the machine's id for it (SL-1).
    let listed = host
        .sessions()
        .await
        .list_sessions(pb::ListSessionsRequest { channel: channel.id.clone(), ..pb::ListSessionsRequest::default() })
        .await
        .unwrap()
        .into_inner()
        .sessions;
    assert!(listed.iter().any(|found| found.id == Some(session.clone()) && found.status == pb::SessionStatus::Working as i32));
}

/// Flow 26.9: the machine's link and daemon die and come back; the host
/// shows the machine offline, then the session reattached, and nothing was
/// started twice.
#[tokio::test(flavor = "multi_thread")]
async fn a_machine_that_goes_away_and_returns_reattaches_its_session_without_starting_it_twice() {
    let machine = FakeMachine::new();
    let host = Host::start(&machine).await;
    host.world.remote("acme", "api");
    host.add_box().await;
    let made = host.create_channel("Payments", &[("acme", "api")]).await;
    host.finish(made.setup.as_ref().unwrap()).await;
    let channel = made.channel.unwrap();
    let session = start_on_box(&host, &channel, "Fix it").await;
    host.wait_status(&session, pb::SessionStatus::Working).await;
    let holders = common::stored_tabs(&machine.state_folder());

    machine.set("down");
    machine.kill_daemon();
    host.box_is(false).await;
    host.wait_status(&session, pb::SessionStatus::Offline).await;
    // Offline: the session can't be stopped or started again from here, at once.
    let refused = host
        .sessions()
        .await
        .stop_session(pb::StopSessionRequest { idempotency_key: None, id: Some(session.clone()) })
        .await
        .unwrap_err();
    assert!(matches!(common::error_of(&refused).kind, Some(pb::error::Kind::MachineOffline(_))));

    machine.clear("down");
    machine.start_daemon();
    host.box_is(true).await;
    host.wait_status(&session, pb::SessionStatus::Working).await;

    let tabs = common::stored_tabs(&machine.state_folder());
    assert_eq!(tabs.len(), holders.len(), "no second session or tab was made");
    assert_eq!(tabs[0].holder_pid, holders[0].holder_pid, "the same holder, reattached");
    let there = humata2_store::Store::open(machine.state_folder().join("store")).unwrap();
    assert_eq!(there.sessions().unwrap().len(), 1);
}

/// Flow 26.21: a repo added while a machine is offline waits there and
/// clones when its link returns.
#[tokio::test(flavor = "multi_thread")]
async fn a_repo_added_while_the_machine_is_offline_waits_then_clones_when_the_link_returns() {
    let machine = FakeMachine::new();
    let host = Host::start(&machine).await;
    host.world.remote("acme", "api");
    host.world.remote("acme", "web");
    host.add_box().await;
    let made = host.create_channel("Payments", &[("acme", "api")]).await;
    host.finish(made.setup.as_ref().unwrap()).await;
    let channel = made.channel.unwrap();
    let id = channel.id.clone().unwrap();

    machine.set("down");
    host.box_is(false).await;
    let added = host
        .channels()
        .await
        .add_repo(pb::AddRepoRequest {
            idempotency_key: Some(pb::IdempotencyKey { value: "add-web".to_owned() }),
            channel: Some(id.clone()),
            repo: Some(repo_ref("acme", "web")),
        })
        .await
        .unwrap()
        .into_inner();

    // The Mac clones at once; the machine's cell waits, offline.
    let deadline = Instant::now() + WAIT;
    loop {
        let grid = host.channels().await.get_grid(pb::GetGridRequest { channel: Some(id.clone()) }).await.unwrap().into_inner();
        let cell = grid.cells.iter().find(|cell| cell.machine == Some(Host::box_id()) && cell.repo == Some(repo_ref("acme", "web")));
        if cell.is_some_and(|cell| cell.state == pb::grid_cell::State::MachineOffline as i32) {
            break;
        }
        assert!(Instant::now() < deadline, "the offline machine's cell never showed as offline");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(!on_machine(&machine, &channel, "main/web").exists());
    let waiting = host.operations().await.get_operation(pb::GetOperationRequest { id: added.id.clone() }).await.unwrap().into_inner();
    assert_eq!(waiting.state, pb::OperationState::Running as i32);
    assert_eq!(step_state(&waiting, &format!("clone_main:{BOX}:acme/web")), Some(pb::StepState::Waiting));

    machine.clear("down");
    host.box_is(true).await;
    let done = host.finish(&added).await;

    assert_eq!(step_state(&done, &format!("clone_main:{BOX}:acme/web")), Some(pb::StepState::Done));
    assert!(on_machine(&machine, &channel, "main/web/.git").exists());
    // The repo that was already there wasn't cloned again.
    assert!(on_machine(&machine, &channel, "main/api/.git").exists());
}
