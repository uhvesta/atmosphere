//! MachineService and OperationService against the real daemon: adding a
//! machine over a fake `ssh` (no network, never the real ~/.ssh) streams
//! its steps, needs the host key confirmed, and lists the machine; removing
//! it needs a plan; agents are held to the access policy.

mod common;

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use common::{
    Attach, Build, Daemon, WAIT, approve, binary, connect, error_of, first_pending, folder, open, pending_requests,
    state_folder,
};
use humata2_fake_machine::FakeMachine;
use humata2_grpc::humata::v1 as pb;
use humata2_store::{SessionRecord, Store};
use prost::Message;
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tokio::net::UnixStream;
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;
use tonic::{Code, Request};

const FILES: [&str; 4] = ["humatad2", "humata2_holder", "humata", "stack"];

/// A tools bundle with the three files for linux-x86_64, the fake
/// machine's default platform. Its humatad2 runs the real one, so the
/// peer link really comes up on the fake machine.
fn bundle(dir: &Path) -> PathBuf {
    let root = dir.join("tools");
    std::fs::create_dir_all(root.join("linux-x86_64")).unwrap();
    let mut entries = serde_json::Map::new();
    for file in FILES {
        let body = if file == "humatad2" {
            // A link a test starts gets its state folder named (the default, said out
            // loud), so it relays nothing and starts no daemon of its own (D146).
            format!(
                "#!/bin/sh\ncase \" $* \" in *\" --stdio \"*) exec '{real}' \"$@\" --state \"$HOME/.humata2/state\" ;; esac\nexec '{real}' \"$@\"\n",
                real = binary("HUMATAD2_RELEASE").display()
            )
        } else {
            format!("#!/bin/sh\n# {file}\nexit 0\n")
        };
        std::fs::write(root.join("linux-x86_64").join(file), &body).unwrap();
        entries.insert(
            file.to_owned(),
            serde_json::json!({
                "path": format!("linux-x86_64/{file}"),
                "sha256": Sha256::digest(body.as_bytes())
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>(),
            }),
        );
    }
    let manifest = serde_json::json!({ "platforms": { "linux-x86_64": entries } });
    std::fs::write(root.join("manifest.json"), manifest.to_string()).unwrap();
    root
}

/// The admin daemon with the fake machine's `ssh` first on its PATH.
struct Host {
    child: Child,
    socket: PathBuf,
    _dir: TempDir,
}

impl Host {
    async fn start(machine: &FakeMachine) -> Self {
        Self::start_in(machine, folder()).await
    }

    /// The daemon on `dir`'s state, as a restart finds it.
    async fn start_in(machine: &FakeMachine, dir: TempDir) -> Self {
        let socket = dir.path().join("d.sock");
        let tools = bundle(dir.path());
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
            .arg(&tools)
            .arg("--ssh-config")
            .arg(dir.path().join("ssh_config"))
            .env("PATH", format!("{}:/usr/bin:/bin", machine.ssh_dir().display()))
            .spawn()
            .unwrap();
        let deadline = Instant::now() + WAIT;
        while UnixStream::connect(&socket).await.is_err() {
            assert!(Instant::now() < deadline, "humatad2 never started listening");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Self {
            child,
            socket,
            _dir: dir,
        }
    }

    async fn machines(&self) -> pb::machine_service_client::MachineServiceClient<Channel> {
        pb::machine_service_client::MachineServiceClient::new(connect(&self.socket).await)
    }

    async fn operations(&self) -> pb::operation_service_client::OperationServiceClient<Channel> {
        pb::operation_service_client::OperationServiceClient::new(connect(&self.socket).await)
    }

    /// Every version the operation's watch sends, to the last.
    async fn watch(&self, operation: &pb::Operation) -> Vec<pb::Operation> {
        let mut stream = self
            .operations()
            .await
            .watch_operation(pb::WatchOperationRequest {
                id: operation.id.clone(),
                from_version: 0,
            })
            .await
            .unwrap()
            .into_inner();
        let mut seen = Vec::new();
        while let Some(next) = tokio::time::timeout(Duration::from_secs(60), stream.message())
            .await
            .expect("the operation finished")
            .unwrap()
        {
            seen.push(next);
        }
        seen
    }

    /// Stops the daemon and starts it again on the same state.
    async fn restart(mut self, machine: &FakeMachine) -> Self {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let dir = std::mem::replace(&mut self._dir, folder());
        Self::start_in(machine, dir).await
    }

    /// The host's copy of box-1's enrollment key, if it keeps one.
    fn peer_key(&self) -> Option<Vec<u8>> {
        std::fs::read(self.peer_key_path()).ok()
    }

    fn peer_key_path(&self) -> PathBuf {
        self._dir.path().join("state/peer-keys/box-1")
    }

    /// box-1 once its peer link is up and it reports a daemon version.
    async fn linked(&self) -> pb::Machine {
        let deadline = Instant::now() + WAIT;
        loop {
            if let Some(machine) = self.list().await.into_iter().find(|machine| machine.name == "box-1")
                && !machine.daemon_version.is_empty()
            {
                return machine;
            }
            assert!(Instant::now() < deadline, "the peer link to box-1 never came up");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn list(&self) -> Vec<pb::Machine> {
        self.machines()
            .await
            .list_machines(pb::ListMachinesRequest::default())
            .await
            .unwrap()
            .into_inner()
            .machines
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn add(fingerprint: &str, key: &str) -> pb::AddSshMachineRequest {
    pb::AddSshMachineRequest {
        idempotency_key: Some(pb::IdempotencyKey { value: key.to_owned() }),
        ssh_host: "me@box-1.internal".to_owned(),
        name: "box-1".to_owned(),
        root: String::new(),
        trust_host_key_fingerprint: fingerprint.to_owned(),
    }
}

fn machine_id() -> Option<pb::MachineId> {
    Some(pb::MachineId {
        value: "box-1".to_owned(),
    })
}

fn state_of(operation: &pb::Operation) -> pb::OperationState {
    pb::OperationState::try_from(operation.state).unwrap()
}

fn step_state(operation: &pb::Operation, key: &str) -> pb::StepState {
    let step = operation.steps.iter().find(|step| step.key == key).unwrap();
    pb::StepState::try_from(step.state).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_machine_is_added_once_its_host_key_is_confirmed_then_removed_by_plan() {
    let machine = FakeMachine::new();
    let host = Host::start(&machine).await;

    // First contact: nothing is installed until the user confirms the key.
    let first = host.machines().await.add_ssh_machine(add("", "k1")).await.unwrap().into_inner();
    assert_eq!(pb::OperationKind::try_from(first.kind).unwrap(), pb::OperationKind::AddMachine);
    let seen = host.watch(&first).await;
    let last = seen.last().unwrap();
    assert_eq!(state_of(last), pb::OperationState::Failed);
    assert_eq!(step_state(last, "connect"), pb::StepState::Failed);
    // A key Humata has never seen is a new key to confirm, not a changed one.
    let Some(pb::error::Kind::HostKeyUnknown(key)) = last.error.as_ref().and_then(|error| error.kind.clone()) else {
        panic!("expected host_key_unknown, got {:?}", last.error);
    };
    assert_eq!(key.algorithm, "ssh-ed25519");
    assert!(key.presented_fingerprint.starts_with("SHA256:"), "{key:?}");
    assert!(machine.installed("humatad2").is_none());
    assert!(host.list().await.is_empty());

    // The same key again finds the same operation.
    let again = host.machines().await.add_ssh_machine(add("", "k1")).await.unwrap().into_inner();
    assert_eq!(again.id, first.id);

    // Confirmed: every step runs, and versions only go up.
    let added = host
        .machines()
        .await
        .add_ssh_machine(add(&key.presented_fingerprint, "k2"))
        .await
        .unwrap()
        .into_inner();
    let seen = host.watch(&added).await;
    assert!(seen.windows(2).all(|pair| pair[0].version < pair[1].version));
    let last = seen.last().unwrap();
    assert_eq!(state_of(last), pb::OperationState::Succeeded, "{:?}", last.error);
    for key in ["connect", "check", "install", "start"] {
        assert_eq!(step_state(last, key), pb::StepState::Done, "{key}");
    }
    assert_eq!(step_state(last, "find_agents"), pb::StepState::Skipped);
    assert_eq!(last.result, Some(pb::operation::Result::Machine(machine_id().unwrap())));
    for file in FILES {
        assert!(machine.installed(file).is_some(), "{file} wasn't installed");
    }
    let listed = host.list().await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "box-1");
    assert_eq!(listed[0].os, "linux");
    assert_eq!(
        pb::Reachability::try_from(listed[0].reachability).unwrap(),
        pb::Reachability::Online
    );

    // Reconnecting with the same build answers with the machine.
    let reconnected = host
        .machines()
        .await
        .reconnect_machine(pb::ReconnectMachineRequest { id: machine_id() })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(reconnected.name, "box-1");

    // Removing needs a plan for this machine.
    let no_plan = host
        .machines()
        .await
        .remove_machine(pb::RemoveMachineRequest {
            id: machine_id(),
            plan: Some(pb::PlanToken {
                value: "pl_made_up".to_owned(),
            }),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert!(matches!(error_of(&no_plan).kind, Some(pb::error::Kind::PlanStale(_))));
    assert_eq!(host.list().await.len(), 1);

    let plan = host
        .machines()
        .await
        .plan_remove_machine(pb::PlanRemoveMachineRequest { id: machine_id() })
        .await
        .unwrap()
        .into_inner();
    let removing = host
        .machines()
        .await
        .remove_machine(pb::RemoveMachineRequest {
            id: machine_id(),
            plan: plan.token.clone(),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    let last = host.watch(&removing).await.pop().unwrap();
    assert_eq!(state_of(&last), pb::OperationState::Succeeded, "{:?}", last.error);
    assert!(host.list().await.is_empty());
    assert!(!machine.unit_installed());
    assert_eq!(host.peer_key(), None, "the host still keeps box-1's enrollment key");

    // A plan is used once.
    let reused = host
        .machines()
        .await
        .remove_machine(pb::RemoveMachineRequest {
            id: machine_id(),
            plan: plan.token,
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert_eq!(reused.code(), Code::FailedPrecondition);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_added_machine_holds_the_hosts_key_and_its_link_comes_up_again_after_a_restart() {
    use std::os::unix::fs::PermissionsExt;
    let machine = FakeMachine::new();
    let host = Host::start(&machine).await;
    let fingerprint = presented_fingerprint(&host).await;
    let added = host.machines().await.add_ssh_machine(add(&fingerprint, "k")).await.unwrap().into_inner();
    let last = host.watch(&added).await.pop().unwrap();
    assert_eq!(state_of(&last), pb::OperationState::Succeeded, "{:?}", last.error);

    let key = host.peer_key().expect("the host keeps box-1's enrollment key");
    assert_eq!(machine.enrollment_key(), Some(key.clone()));
    let mode = std::fs::metadata(host.peer_key_path()).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);

    let linked = host.linked().await;
    assert_eq!(pb::Reachability::try_from(linked.reachability).unwrap(), pb::Reachability::Online);
    assert!(linked.last_seen_at.is_some());
    assert!(linked.next_retry_at.is_none());

    // A restart finds the machine ready and brings its link up again.
    let host = host.restart(&machine).await;
    let linked = host.linked().await;
    assert_eq!(pb::Reachability::try_from(linked.reachability).unwrap(), pb::Reachability::Online);
    assert_eq!(host.peer_key(), Some(key));
}

fn trust(fingerprint: &str) -> pb::TrustNewHostKeyRequest {
    pb::TrustNewHostKeyRequest {
        idempotency_key: None,
        id: machine_id(),
        new_fingerprint: fingerprint.to_owned(),
        presence: None,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_changed_host_key_is_trusted_again_only_when_its_fingerprint_is_the_one_presented() {
    let machine = FakeMachine::new();
    let host = Host::start(&machine).await;
    let fingerprint = presented_fingerprint(&host).await;
    let added = host.machines().await.add_ssh_machine(add(&fingerprint, "k")).await.unwrap().into_inner();
    let last = host.watch(&added).await.pop().unwrap();
    assert_eq!(state_of(&last), pb::OperationState::Succeeded, "{:?}", last.error);
    let known = host._dir.path().join("state/known_hosts");
    let pinned = std::fs::read_to_string(&known).unwrap();

    // The machine now shows another key than the one Humata pinned.
    let other = "humata2.box-1 ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBEmKSENjQEezOmxkZMy7opKgwFB9nkt5YRrYMjNuG5N87uRgg6CLrbo5wAdT/y6v0mKV0U2w0WZ2YB/++Tpockg=\n";
    std::fs::write(&known, other).unwrap();

    // A fingerprint that isn't the one it presents pins nothing.
    let wrong = host
        .machines()
        .await
        .trust_new_host_key(trust("SHA256:not-the-key-the-machine-presents"))
        .await
        .expect_err("a different fingerprint is refused");
    assert_eq!(wrong.code(), Code::FailedPrecondition);
    // A key that differs from the one the person confirmed names both.
    let Some(pb::error::Kind::HostKeyChanged(key)) = error_of(&wrong).kind else {
        panic!("expected host_key_changed, got {wrong:?}");
    };
    assert_eq!(key.expected_fingerprint, "SHA256:not-the-key-the-machine-presents");
    assert_eq!(key.presented_fingerprint, fingerprint);
    assert_eq!(std::fs::read_to_string(&known).unwrap(), other, "nothing was pinned");

    // The one it presents, once the user has checked it, replaces the pin.
    let trusted = host.machines().await.trust_new_host_key(trust(&fingerprint)).await.unwrap().into_inner();
    assert_eq!(trusted.name, "box-1");
    assert_eq!(std::fs::read_to_string(&known).unwrap(), pinned);
}

/// The fingerprint a first, unconfirmed add reports.
async fn presented_fingerprint(host: &Host) -> String {
    let first = host.machines().await.add_ssh_machine(add("", "first")).await.unwrap().into_inner();
    let last = host.watch(&first).await.pop().unwrap();
    let Some(pb::error::Kind::HostKeyUnknown(key)) = last.error.and_then(|error| error.kind) else {
        panic!("expected the host key to need confirming");
    };
    key.presented_fingerprint
}

/// Waits until the fake `ssh` has been run more than `calls` times.
async fn ssh_ran(machine: &FakeMachine, calls: usize) {
    let deadline = Instant::now() + WAIT;
    while machine.ssh_calls() <= calls {
        assert!(Instant::now() < deadline, "ssh never ran");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// The feed's machine events, after `after` (unset: from now).
async fn machine_feed(host: &Host, after: Option<u64>) -> tonic::Streaming<pb::Event> {
    pb::event_service_client::EventServiceClient::new(connect(&host.socket).await)
        .subscribe(pb::SubscribeRequest {
            after: after.map(|value| pb::EventSeq { value }),
            topics: vec![pb::Topic::Machines as i32],
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner()
}

/// The next event on `feed`. A machine's next change can be a whole add
/// away (install over the fake ssh, then the link's dial), so this waits as
/// long as `Host::watch` waits for an operation, not the short `WAIT`. The
/// one `Resync` a start sends every feed once it has brought things back
/// (D341) is passed over, since it can land after the subscribe.
async fn next_event(feed: &mut tonic::Streaming<pb::Event>) -> pb::Event {
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let event = feed.message().await.unwrap().expect("the feed stays open");
            if !matches!(event.payload, Some(pb::event::Payload::Resync(_))) {
                return event;
            }
        }
    })
    .await
    .expect("an event came")
}

fn seq_of(event: &pb::Event) -> u64 {
    event.seq.as_ref().unwrap().value
}

fn machine_in(event: &pb::Event) -> &pb::Machine {
    match &event.payload {
        Some(pb::event::Payload::Machine(machine)) => machine,
        other => panic!("expected a machine, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_feed_tells_each_machine_change_and_a_resume_starts_after_the_last_seen() {
    let machine = FakeMachine::new();
    let host = Host::start(&machine).await;
    let fingerprint = presented_fingerprint(&host).await;
    let mut feed = machine_feed(&host, None).await;

    let adding = host.machines().await.add_ssh_machine(add(&fingerprint, "k2")).await.unwrap().into_inner();
    let created = next_event(&mut feed).await;
    assert_eq!(created.change(), pb::Change::Created, "{created:?}");
    assert_eq!(machine_in(&created).name, "box-1");
    let mut seen = vec![created.clone()];
    while machine_in(seen.last().unwrap()).reachability() != pb::Reachability::Online {
        let event = next_event(&mut feed).await;
        assert_eq!(event.change(), pb::Change::Updated, "{event:?}");
        seen.push(event);
    }
    assert!(seen.windows(2).all(|pair| seq_of(&pair[0]) < seq_of(&pair[1])));
    let last = host.watch(&adding).await.pop().unwrap();
    assert_eq!(state_of(&last), pb::OperationState::Succeeded, "{:?}", last.error);
    assert_eq!(machine_in(seen.last().unwrap()), &host.list().await[0]);

    // Resuming after the first event starts exactly at the second.
    let mut resumed = machine_feed(&host, Some(seq_of(&created))).await;
    assert_eq!(next_event(&mut resumed).await, seen[1]);

    // A removal ends with the machine deleted, as it last was.
    let plan = host
        .machines()
        .await
        .plan_remove_machine(pb::PlanRemoveMachineRequest { id: machine_id() })
        .await
        .unwrap()
        .into_inner();
    host.machines()
        .await
        .remove_machine(pb::RemoveMachineRequest {
            id: machine_id(),
            plan: plan.token,
            ..Default::default()
        })
        .await
        .unwrap();
    let deleted = loop {
        let event = next_event(&mut feed).await;
        if event.change() == pb::Change::Deleted {
            break event;
        }
    };
    assert_eq!(machine_in(&deleted).name, "box-1");
    assert!(host.list().await.is_empty());

    // A point older than anything kept (a past run's) is told to resync.
    let mut stale = machine_feed(&host, Some(1)).await;
    let first = tokio::time::timeout(Duration::from_secs(60), stale.message())
        .await
        .expect("an event came")
        .unwrap()
        .expect("the feed stays open");
    assert!(matches!(first.payload, Some(pb::event::Payload::Resync(_))), "{first:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_feed_tells_an_operation_from_its_start_to_how_it_ended() {
    let machine = FakeMachine::new();
    let host = Host::start(&machine).await;
    let fingerprint = presented_fingerprint(&host).await;
    let mut feed = pb::event_service_client::EventServiceClient::new(connect(&host.socket).await)
        .subscribe(pb::SubscribeRequest {
            topics: vec![pb::Topic::Operations as i32],
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    let mut narrowed = pb::event_service_client::EventServiceClient::new(connect(&host.socket).await)
        .subscribe(pb::SubscribeRequest {
            topics: vec![pb::Topic::Operations as i32],
            channels: vec![pb::ChannelId {
                value: "ch_other".to_owned(),
            }],
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();

    let adding = host.machines().await.add_ssh_machine(add(&fingerprint, "k2")).await.unwrap().into_inner();
    let watched = host.watch(&adding).await;
    let mut told = Vec::new();
    let mut by = Vec::new();
    loop {
        let event = next_event(&mut feed).await;
        let Some(pb::event::Payload::Operation(operation)) = event.payload.clone() else {
            panic!("expected an operation, got {event:?}");
        };
        by.push(event.by.clone().and_then(|by| by.actor));
        told.push((event.change(), operation));
        if told.last().is_some_and(|(_, operation)| state_of(operation) != pb::OperationState::Running) {
            break;
        }
    }

    assert_eq!(told[0].0, pb::Change::Created);
    assert_eq!(told[0].1.id, adding.id);
    assert!(told[1..].iter().all(|(change, _)| *change == pb::Change::Updated));
    // The user started it; its steps are the daemon's own.
    assert!(matches!(by[0], Some(pb::actor::Actor::User(_))), "{:?}", by[0]);
    assert!(by[1..].iter().all(Option::is_none));
    assert!(told.windows(2).all(|pair| pair[0].1.version < pair[1].1.version));
    assert_eq!(told.last().map(|(_, operation)| operation), watched.last());
    // The user's operation belongs to no channel.
    let heard = tokio::time::timeout(Duration::from_millis(200), async {
        loop {
            let event = narrowed.message().await.unwrap().expect("the feed stays open");
            // The start's one Resync (D341) is not the operation.
            if !matches!(event.payload, Some(pb::event::Payload::Resync(_))) {
                return event;
            }
        }
    })
    .await;
    assert!(heard.is_err(), "a feed narrowed to a channel heard of the user's operation: {heard:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_add_cancelled_while_it_connects_ends_cancelled_and_leaves_nothing() {
    let machine = FakeMachine::new();
    let host = Host::start(&machine).await;
    let fingerprint = presented_fingerprint(&host).await;

    machine.set("hang");
    let calls = machine.ssh_calls();
    let adding = host.machines().await.add_ssh_machine(add(&fingerprint, "k2")).await.unwrap().into_inner();
    assert!(adding.cancellable);
    ssh_ran(&machine, calls).await;
    let asked = host
        .operations()
        .await
        .cancel_operation(pb::CancelOperationRequest {
            id: adding.id.clone(),
            idempotency_key: None,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(asked.id, adding.id);
    machine.clear("hang");

    let last = host.watch(&adding).await.pop().unwrap();
    assert_eq!(state_of(&last), pb::OperationState::Cancelled, "{:?}", last.error);
    assert!(!last.cancellable);
    for key in ["check", "install", "start"] {
        assert_ne!(step_state(&last, key), pb::StepState::Done, "{key} ran after the cancel");
    }
    assert_eq!(step_state(&last, "check"), pb::StepState::Waiting);
    assert!(host.list().await.is_empty());
    assert!(machine.installed("humatad2").is_none());

    // Cancelling again answers with how it ended.
    let again = host
        .operations()
        .await
        .cancel_operation(pb::CancelOperationRequest {
            id: adding.id.clone(),
            idempotency_key: None,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(state_of(&again), pb::OperationState::Cancelled);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_add_tried_again_runs_under_the_same_id_and_a_removal_cannot_be_cancelled() {
    let machine = FakeMachine::new();
    let host = Host::start(&machine).await;
    let fingerprint = presented_fingerprint(&host).await;

    // The machine is down: the add fails at connect.
    machine.set("down");
    let adding = host.machines().await.add_ssh_machine(add(&fingerprint, "k2")).await.unwrap().into_inner();
    let failed = host.watch(&adding).await.pop().unwrap();
    assert_eq!(state_of(&failed), pb::OperationState::Failed);
    assert!(host.list().await.is_empty());

    // Back up: trying again adds it, same operation, later versions.
    machine.clear("down");
    let retry = |key: &str| pb::RetryOperationRequest {
        id: adding.id.clone(),
        idempotency_key: Some(pb::IdempotencyKey { value: key.to_owned() }),
    };
    let retried = host.operations().await.retry_operation(retry("r1")).await.unwrap().into_inner();
    assert_eq!(retried.id, adding.id);
    assert!(retried.version > failed.version);
    let resent = host.operations().await.retry_operation(retry("r1")).await.unwrap().into_inner();
    assert_eq!(resent.id, adding.id);
    let last = host.watch(&retried).await.pop().unwrap();
    assert_eq!(state_of(&last), pb::OperationState::Succeeded, "{:?}", last.error);
    assert!(last.error.is_none());
    assert_eq!(host.list().await.len(), 1);
    assert!(machine.installed("humatad2").is_some());

    // Only a failed operation can be tried again.
    let refused = host.operations().await.retry_operation(retry("r2")).await.unwrap_err();
    assert_eq!(refused.code(), Code::FailedPrecondition);

    // A removal is past its point of no return from its first step.
    let plan = host
        .machines()
        .await
        .plan_remove_machine(pb::PlanRemoveMachineRequest { id: machine_id() })
        .await
        .unwrap()
        .into_inner();
    machine.set("hang");
    let calls = machine.ssh_calls();
    let removing = host
        .machines()
        .await
        .remove_machine(pb::RemoveMachineRequest {
            id: machine_id(),
            plan: plan.token,
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    assert!(!removing.cancellable);
    ssh_ran(&machine, calls).await;
    let refused = host
        .operations()
        .await
        .cancel_operation(pb::CancelOperationRequest {
            id: removing.id.clone(),
            idempotency_key: None,
        })
        .await
        .unwrap_err();
    assert!(matches!(error_of(&refused).kind, Some(pb::error::Kind::NotCancellable(_))));
    machine.clear("hang");
    let last = host.watch(&removing).await.pop().unwrap();
    assert_eq!(state_of(&last), pb::OperationState::Succeeded, "{:?}", last.error);
    assert!(host.list().await.is_empty());
}

/// AddSshMachine, the call `humata host add` makes, for a name whose
/// removal is still running is refused as being removed (ABORTED, no
/// `name_taken`), and the same add goes through once the removal is done
/// (D760).
#[tokio::test(flavor = "multi_thread")]
async fn an_add_while_the_machine_is_being_removed_is_refused_until_the_removal_ends() {
    let machine = FakeMachine::new();
    let host = Host::start(&machine).await;
    let fingerprint = presented_fingerprint(&host).await;
    let added = host.machines().await.add_ssh_machine(add(&fingerprint, "k1")).await.unwrap().into_inner();
    let added = host.watch(&added).await.pop().unwrap();
    assert_eq!(state_of(&added), pb::OperationState::Succeeded, "{:?}", added.error);

    let plan = host
        .machines()
        .await
        .plan_remove_machine(pb::PlanRemoveMachineRequest { id: machine_id() })
        .await
        .unwrap()
        .into_inner();
    machine.set("hang");
    let calls = machine.ssh_calls();
    let removing = host
        .machines()
        .await
        .remove_machine(pb::RemoveMachineRequest {
            id: machine_id(),
            plan: plan.token,
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    ssh_ran(&machine, calls).await;

    let refused = host.machines().await.add_ssh_machine(add(&fingerprint, "k2")).await.unwrap_err();
    assert_eq!(refused.code(), Code::Aborted, "{refused:?}");
    assert!(
        !matches!(error_of(&refused).kind, Some(pb::error::Kind::NameTaken(_))),
        "an add during a removal isn't told the name is taken: {refused:?}"
    );

    machine.clear("hang");
    let removed = host.watch(&removing).await.pop().unwrap();
    assert_eq!(state_of(&removed), pb::OperationState::Succeeded, "{:?}", removed.error);
    host.machines().await.add_ssh_machine(add(&fingerprint, "k3")).await.expect("the name is free once the removal is done");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bad_ssh_host_or_unknown_machine_is_the_calls_own_error() {
    let machine = FakeMachine::new();
    let host = Host::start(&machine).await;

    let mut bad = add("", "k");
    bad.ssh_host = "-oProxyCommand=evil".to_owned();
    let refused = host.machines().await.add_ssh_machine(bad).await.unwrap_err();
    assert_eq!(refused.code(), Code::InvalidArgument);
    assert_eq!(machine.ssh_calls(), 0);

    let missing = host
        .machines()
        .await
        .reconnect_machine(pb::ReconnectMachineRequest {
            id: Some(pb::MachineId {
                value: "nowhere".to_owned(),
            }),
        })
        .await
        .unwrap_err();
    assert_eq!(missing.code(), Code::NotFound);

    let operation = host
        .operations()
        .await
        .get_operation(pb::GetOperationRequest {
            id: Some(pb::OperationId {
                value: "op_unknown".to_owned(),
            }),
        })
        .await
        .unwrap_err();
    assert_eq!(operation.code(), Code::NotFound);
}

const SESSION: &str = "se_machines";
const TOKEN_HEADER: &str = "humata-session-token";

/// An admin daemon with one session, and the token its shell tab got.
async fn daemon_with_agent() -> (Daemon, String) {
    daemon_with_agent_on(&[]).await
}

/// [`daemon_with_agent`] with environment variables for the daemon, such as a
/// PATH that finds a fake `ssh`.
async fn daemon_with_agent_on(env: &[(&str, &str)]) -> (Daemon, String) {
    let dir = folder();
    let store = Store::open(state_folder(dir.path()).join("store")).unwrap();
    store
        .put_session(&SessionRecord {
            id: SESSION.to_owned(),
            channel: "ch_machines".to_owned(),
            machine: String::new(),
            title: "machines".to_owned(),
            agent: "zsh".to_owned(),
            folder: dir.path().to_owned(),
            created_unix_ms: 0,
        })
        .unwrap();
    drop(store);
    let tools = bundle(dir.path());
    let ssh_config = dir.path().join("ssh_config");
    let daemon = Daemon::start_with_env(
        Build::Admin,
        dir,
        &[
            "--tools",
            tools.to_str().unwrap(),
            "--ssh-config",
            ssh_config.to_str().unwrap(),
        ],
        env,
    )
    .await;
    let tab = daemon
        .terminals()
        .await
        .open_shell_tab(pb::OpenShellTabRequest {
            idempotency_key: None,
            session: Some(pb::SessionId {
                value: SESSION.to_owned(),
            }),
            repo: None,
            folder: String::new(),
        })
        .await
        .unwrap()
        .into_inner()
        .id
        .unwrap();
    let file = daemon.dir.path().join("token");
    let mut attach = Attach::start(&daemon, open(&tab, 0)).await;
    attach
        .type_text(&format!("printf '%s' \"$HUMATA_SESSION_TOKEN\" > {}\r", file.display()))
        .await;
    let deadline = Instant::now() + WAIT;
    let token = loop {
        match std::fs::read_to_string(&file) {
            Ok(text) if !text.is_empty() => break text,
            _ => {}
        }
        assert!(Instant::now() < deadline, "the token was never written");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    (daemon, token)
}

fn as_agent<T>(message: T, token: &str) -> Request<T> {
    let mut request = Request::new(message);
    request
        .metadata_mut()
        .insert(TOKEN_HEADER, MetadataValue::try_from(token).unwrap());
    request
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_cannot_add_or_remove_a_machine_on_its_own() {
    let (daemon, token) = daemon_with_agent().await;
    let mut machines = pb::machine_service_client::MachineServiceClient::new(daemon.channel().await);

    // Adding asks the user first (MACHINE_PROVISION is Ask me): the call waits in the Inbox.
    let asked = machines.add_ssh_machine(as_agent(add("", "k"), &token)).await.unwrap_err();
    assert!(matches!(error_of(&asked).kind, Some(pb::error::Kind::ApprovalPending(_))), "{asked:?}");

    // Removing is never allowed by default (MACHINE_DESTROY).
    let refused = machines
        .plan_remove_machine(as_agent(pb::PlanRemoveMachineRequest { id: machine_id() }, &token))
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::PermissionDenied, "{refused:?}");
    assert!(matches!(error_of(&refused).kind, Some(pb::error::Kind::Denied(_))));

    // An agent sees only its channel's machines (TP-1), none here, and its
    // refused calls started nothing.
    let listed = machines
        .list_machines(as_agent(pb::ListMachinesRequest::default(), &token))
        .await
        .unwrap()
        .into_inner();
    assert!(listed.machines.is_empty());
    let mut operations = pb::operation_service_client::OperationServiceClient::new(daemon.channel().await);
    let all = operations
        .list_operations(as_agent(pb::ListOperationsRequest::default(), &token))
        .await
        .unwrap()
        .into_inner();
    assert!(all.operations.is_empty());
}

/// The agent's wait on Inbox request `id`, to its answer.
async fn agent_waits(daemon: &Daemon, token: &str, id: &pb::RequestId) -> pb::WaitResponse {
    let mut stream = daemon
        .inbox_service()
        .await
        .wait_for_answer(as_agent(
            pb::WaitForAnswerRequest {
                id: Some(id.clone()),
                wait_seconds: 10,
            },
            token,
        ))
        .await
        .unwrap()
        .into_inner();
    loop {
        let message = tokio::time::timeout(WAIT, stream.message())
            .await
            .expect("the wait ended in time")
            .unwrap()
            .expect("the wait ended with an answer");
        if !message.keepalive {
            return message;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agents_add_waits_in_the_inbox_and_runs_once_the_user_approves_it() {
    let machine = FakeMachine::new();
    let path = format!("{}:/usr/bin:/bin", machine.ssh_dir().display());
    let (daemon, token) = daemon_with_agent_on(&[("PATH", path.as_str())]).await;
    let mut machines = pb::machine_service_client::MachineServiceClient::new(daemon.channel().await);
    let mut operations = pb::operation_service_client::OperationServiceClient::new(daemon.channel().await);

    // The call is put in the Inbox: nothing ran, nothing was started.
    let waiting = machines.add_ssh_machine(as_agent(add("", "k"), &token)).await.unwrap_err();
    assert_eq!(waiting.code(), Code::FailedPrecondition, "{waiting:?}");
    let Some(pb::error::Kind::ApprovalPending(pending)) = error_of(&waiting).kind else {
        panic!("expected approval_pending, got {waiting:?}");
    };
    let id = first_pending(&daemon).await;
    assert_eq!(pending.request, Some(id.clone()));
    assert_eq!(machine.ssh_calls(), 0);
    let started = operations.list_operations(pb::ListOperationsRequest::default()).await.unwrap().into_inner();
    assert!(started.operations.is_empty());

    // The same call again with the same key is the same request.
    let again = machines.add_ssh_machine(as_agent(add("", "k"), &token)).await.unwrap_err();
    assert!(matches!(error_of(&again).kind, Some(pb::error::Kind::ApprovalPending(_))), "{again:?}");
    assert_eq!(pending_requests(&daemon).await, vec![id.clone()]);

    // Once the user approves, the whole call runs and its operation is the answer.
    approve(&daemon, &id).await;
    let answered = agent_waits(&daemon, &token, &id).await;
    assert_eq!(answered.state(), pb::RequestState::Approved);
    assert!(answered.error.is_none(), "{:?}", answered.error);
    assert_eq!(answered.result_type, "humata.v1.Operation");
    let operation = pb::Operation::decode(answered.result.as_slice()).unwrap();
    assert_eq!(pb::OperationKind::try_from(operation.kind).unwrap(), pb::OperationKind::AddMachine);
    let found = operations
        .get_operation(pb::GetOperationRequest {
            id: operation.id.clone(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(found.id, operation.id);
    ssh_ran(&machine, 0).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_users_own_add_is_not_sent_to_the_inbox() {
    let machine = FakeMachine::new();
    let host = Host::start(&machine).await;

    let added = host.machines().await.add_ssh_machine(add("", "k")).await.unwrap().into_inner();
    assert_eq!(pb::OperationKind::try_from(added.kind).unwrap(), pb::OperationKind::AddMachine);
    let inbox = pb::inbox_service_client::InboxServiceClient::new(connect(&host.socket).await)
        .list_requests(pb::ListRequestsRequest::default())
        .await
        .unwrap()
        .into_inner();
    assert!(inbox.requests.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_linked_machines_page_shows_the_health_its_probe_read() {
    let machine = FakeMachine::new();
    let host = Host::start(&machine).await;
    let fingerprint = presented_fingerprint(&host).await;
    let added = host.machines().await.add_ssh_machine(add(&fingerprint, "k")).await.unwrap().into_inner();
    let last = host.watch(&added).await.pop().unwrap();
    assert_eq!(state_of(&last), pb::OperationState::Succeeded, "{:?}", last.error);
    host.linked().await;

    let detail = host
        .machines()
        .await
        .get_machine(pb::GetMachineRequest { id: machine_id() })
        .await
        .unwrap()
        .into_inner();
    let health = detail.health.expect("a linked machine has health");
    assert!(health.read_at.is_some());
    assert!(health.disk_total_bytes > 0);
    // The git version is the number alone, empty on a machine without git.
    assert!(health.git_version.is_empty() || health.git_version.starts_with(|c: char| c.is_ascii_digit()));
    // Versions are numbers alone, one for each agent found, in its order (HO-4).
    assert!(health.gh_version.is_empty() || health.gh_version.starts_with(|c: char| c.is_ascii_digit()));
    assert_eq!(health.agent_versions.len(), health.agents_installed.len());
    assert!(health.agent_versions.iter().all(|version| version.is_empty() || version.starts_with(|c: char| c.is_ascii_digit())));
    // Folders are a list, each under ~/Humata, with no channel named by the daemon.
    assert!(detail.folders.iter().all(|folder| folder.path.starts_with("~/Humata/") && folder.channel.is_none()));
}

/// A tool that prints `output` when asked for its version, as `gh` does for
/// `--version` and `version`, and fails for anything else (not signed in).
/// The fake `git` prints its version whatever it is asked, as the fake
/// machine's own does.
fn says_its_version(output: &str) -> String {
    format!("case \"$1\" in\n  --version|version) cat <<'VERSION'\n{output}\nVERSION\n  ;;\n  *) exit 1 ;;\nesac")
}

/// Adds box-1 with its host key confirmed; every version of the add, to the
/// last.
async fn versions_of_a_confirmed_add(host: &Host) -> Vec<pb::Operation> {
    let fingerprint = presented_fingerprint(host).await;
    let added = host
        .machines()
        .await
        .add_ssh_machine(add(&fingerprint, "confirmed"))
        .await
        .unwrap()
        .into_inner();
    host.watch(&added).await
}

fn check_step(operation: &pb::Operation) -> &pb::OperationStep {
    operation
        .steps
        .iter()
        .find(|step| step.key == "check")
        .expect("an add has a check step")
}

fn fact<'a>(step: &'a pb::OperationStep, name: &str) -> Option<&'a str> {
    step.facts.get(name).map(String::as_str)
}

#[tokio::test(flavor = "multi_thread")]
async fn the_check_step_of_an_add_carries_what_it_read_from_the_machine() {
    let machine = FakeMachine::new();
    machine.set_tool("git", "echo 'git version 2.47.1'");
    machine.set_tool(
        "gh",
        &says_its_version("gh version 2.83.0 (2025-11-04)\nhttps://github.com/cli/cli/releases/tag/v2.83.0"),
    );
    let host = Host::start(&machine).await;

    let seen = versions_of_a_confirmed_add(&host).await;

    let last = seen.last().unwrap();
    assert_eq!(state_of(last), pb::OperationState::Succeeded, "{:?}", last.error);
    let check = check_step(last);
    assert_eq!(fact(check, "os"), Some("linux"));
    assert_eq!(fact(check, "arch"), Some("x86_64"));
    assert_eq!(fact(check, "git"), Some("2.47.1"));
    assert_eq!(fact(check, "gh"), Some("2.83.0"));
    let free: u64 = fact(check, "disk_free_bytes")
        .expect("the check read the free disk")
        .parse()
        .expect("the free disk is a whole number of bytes");
    assert!(free > 0);

    // Whoever watches sees the facts no later than the step turning done.
    let first_done = seen
        .iter()
        .find(|operation| step_state(operation, "check") == pb::StepState::Done)
        .expect("the check step finished");
    assert_eq!(check_step(first_done).facts, check.facts);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_check_step_of_an_add_names_no_gh_version_when_gh_gives_none() {
    let machine = FakeMachine::new();
    machine.set_tool("git", "echo 'git version 2.47.1'");
    // It is on PATH and fails whatever it is asked, printing nothing.
    machine.set_tool("gh", "exit 127");
    let host = Host::start(&machine).await;

    let seen = versions_of_a_confirmed_add(&host).await;

    let last = seen.last().unwrap();
    assert_eq!(state_of(last), pb::OperationState::Succeeded, "{:?}", last.error);
    let check = check_step(last);
    assert_eq!(fact(check, "gh").unwrap_or_default(), "");
    assert_eq!(fact(check, "git"), Some("2.47.1"));
    assert_eq!(fact(check, "os"), Some("linux"));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_storage_page_reads_this_machines_own_storage_and_no_other() {
    let machine = FakeMachine::new();
    let host = Host::start(&machine).await;

    let report = host
        .machines()
        .await
        .get_storage(pb::GetStorageRequest { id: None })
        .await
        .unwrap()
        .into_inner();
    assert!(report.measured_at.is_some());
    assert!(report.total_bytes > 0);
    assert!(report.rows.iter().any(|row| row.kind == pb::StorageKind::Logs as i32));

    let other = host
        .machines()
        .await
        .get_storage(pb::GetStorageRequest {
            id: Some(pb::MachineId {
                value: "nowhere".to_owned(),
            }),
        })
        .await
        .unwrap_err();
    assert_eq!(other.code(), Code::NotFound);
}
