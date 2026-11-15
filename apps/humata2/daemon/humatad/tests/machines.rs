//! MachineService and OperationService against the real daemon: adding a
//! machine over a fake `ssh` (no network, never the real ~/.ssh) streams
//! its steps, needs the host key confirmed, and lists the machine; removing
//! it needs a plan; agents are held to the access policy.

mod common;

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use common::{Attach, Build, Daemon, WAIT, binary, connect, error_of, folder, open, state_folder};
use humata2_fake_machine::FakeMachine;
use humata2_grpc::humata::v1 as pb;
use humata2_store::{SessionRecord, Store};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tokio::net::UnixStream;
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;
use tonic::{Code, Request};

const FILES: [&str; 3] = ["humatad2", "humata2_holder", "humata"];

/// A tools bundle with the three files for linux-x86_64, the fake
/// machine's default platform.
fn bundle(dir: &Path) -> PathBuf {
    let root = dir.join("tools");
    std::fs::create_dir_all(root.join("linux-x86_64")).unwrap();
    let mut entries = serde_json::Map::new();
    for file in FILES {
        let body = format!("#!/bin/sh\n# {file}\nexit 0\n");
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
        let dir = folder();
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
    let Some(pb::error::Kind::HostKeyChanged(key)) = last.error.as_ref().and_then(|error| error.kind.clone()) else {
        panic!("expected host_key_changed, got {:?}", last.error);
    };
    assert!(key.expected_fingerprint.is_empty());
    assert!(!key.presented_fingerprint.is_empty());
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

/// The fingerprint a first, unconfirmed add reports.
async fn presented_fingerprint(host: &Host) -> String {
    let first = host.machines().await.add_ssh_machine(add("", "first")).await.unwrap().into_inner();
    let last = host.watch(&first).await.pop().unwrap();
    let Some(pb::error::Kind::HostKeyChanged(key)) = last.error.and_then(|error| error.kind) else {
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
    let daemon = Daemon::start_with(
        Build::Admin,
        dir,
        &[
            "--tools",
            tools.to_str().unwrap(),
            "--ssh-config",
            ssh_config.to_str().unwrap(),
        ],
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

    // Adding asks the user first (MACHINE_PROVISION is Ask me).
    machines.add_ssh_machine(as_agent(add("", "k"), &token)).await.unwrap_err();

    // Removing is never allowed by default (MACHINE_DESTROY).
    let refused = machines
        .plan_remove_machine(as_agent(pb::PlanRemoveMachineRequest { id: machine_id() }, &token))
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::PermissionDenied, "{refused:?}");
    assert!(matches!(error_of(&refused).kind, Some(pb::error::Kind::Denied(_))));

    // An agent sees no machines, and its refused calls started nothing.
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
