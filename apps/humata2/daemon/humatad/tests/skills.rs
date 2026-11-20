//! SkillsService against the real daemon and a fake machine whose humatad2
//! is the real binary, so the peer link really carries ApplySkills: a first
//! sync needs the plan the user looked at, the machine ends with the files in
//! its managed folder, an offline machine is skipped and named, a changed
//! host skill syncs by itself when the link next comes up, and agents are
//! refused.
//!
//! Waits for enabling: this file lives in tests/pending/ until the daemon
//! takes `--skills-home` and registers SkillsService (see the enable lines);
//! then move it to tests/skills.rs, where the BUILD glob gives it its target.

use humatad_test_common as common;

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

const FILES: [&str; 5] = ["humatad2", "humata2_holder", "humata", "stack", "humata2_fusefs"];

/// A tools bundle for linux-x86_64 whose humatad2 runs the real one.
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

/// The admin daemon with the fake machine's `ssh` first on its PATH and its
/// skills read from `<dir>/home`, never the real home folder.
struct Host {
    child: Child,
    socket: PathBuf,
    dir: TempDir,
}

impl Host {
    async fn start(machine: &FakeMachine) -> Self {
        Self::start_in(machine, folder()).await
    }

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
            .arg("--skills-home")
            .arg(dir.path().join("home"))
            .env("PATH", format!("{}:/usr/bin:/bin", machine.ssh_dir().display()))
            .spawn()
            .unwrap();
        let deadline = Instant::now() + WAIT;
        while UnixStream::connect(&socket).await.is_err() {
            assert!(Instant::now() < deadline, "humatad2 never started listening");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Self { child, socket, dir }
    }

    async fn restart(mut self, machine: &FakeMachine) -> Self {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let dir = std::mem::replace(&mut self.dir, folder());
        Self::start_in(machine, dir).await
    }

    fn skills_folder(&self, agent: &str) -> PathBuf {
        self.dir.path().join("home").join(format!(".{agent}/skills"))
    }

    async fn machines(&self) -> pb::machine_service_client::MachineServiceClient<Channel> {
        pb::machine_service_client::MachineServiceClient::new(connect(&self.socket).await)
    }

    async fn skills(&self) -> pb::skills_service_client::SkillsServiceClient<Channel> {
        pb::skills_service_client::SkillsServiceClient::new(connect(&self.socket).await)
    }

    async fn operations(&self) -> pb::operation_service_client::OperationServiceClient<Channel> {
        pb::operation_service_client::OperationServiceClient::new(connect(&self.socket).await)
    }

    async fn watch(&self, operation: &pb::Operation) -> pb::Operation {
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
        let mut last = None;
        while let Some(next) = tokio::time::timeout(Duration::from_secs(60), stream.message())
            .await
            .expect("the operation finished")
            .unwrap()
        {
            last = Some(next);
        }
        last.unwrap()
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

    /// box-1 once its peer link is up.
    async fn linked(&self) {
        let deadline = Instant::now() + WAIT;
        loop {
            if self
                .list()
                .await
                .iter()
                .any(|machine| machine.name == "box-1" && !machine.daemon_version.is_empty())
            {
                return;
            }
            assert!(Instant::now() < deadline, "the peer link to box-1 never came up");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Adds box-1 and waits for its link.
    async fn add_box(&self) {
        self.add_box_as("").await;
    }

    /// As [`Self::add_box`], with `round` in the calls' idempotency keys so
    /// a second add after a removal is a new call.
    async fn add_box_as(&self, round: &str) {
        let first = self
            .machines()
            .await
            .add_ssh_machine(add("", &format!("first{round}")))
            .await
            .unwrap()
            .into_inner();
        let failed = self.watch(&first).await;
        let Some(pb::error::Kind::HostKeyUnknown(key)) = failed.error.and_then(|error| error.kind) else {
            panic!("expected the host key to need confirming");
        };
        let added = self
            .machines()
            .await
            .add_ssh_machine(add(&key.presented_fingerprint, &format!("second{round}")))
            .await
            .unwrap()
            .into_inner();
        let last = self.watch(&added).await;
        assert_eq!(last.state, pb::OperationState::Succeeded as i32, "{:?}", last.error);
        self.linked().await;
    }

    /// Removes box-1 and waits for the removal to finish.
    async fn remove_box(&self, key: &str) {
        let plan = self
            .machines()
            .await
            .plan_remove_machine(pb::PlanRemoveMachineRequest { id: box_id() })
            .await
            .unwrap()
            .into_inner();
        let operation = self
            .machines()
            .await
            .remove_machine(pb::RemoveMachineRequest {
                id: box_id(),
                plan: plan.token,
                idempotency_key: Some(pb::IdempotencyKey { value: key.to_owned() }),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner();
        let done = self.watch(&operation).await;
        assert_eq!(done.state, pb::OperationState::Succeeded as i32, "{:?}", done.error);
        assert!(self.list().await.is_empty());
    }

    async fn preview(&self) -> pb::SkillsSyncPlan {
        self.skills()
            .await
            .preview_skills_sync(pb::PreviewSkillsSyncRequest { machine: box_id() })
            .await
            .unwrap()
            .into_inner()
    }

    async fn sync(&self, key: &str, plans: Vec<pb::PlanToken>) -> Result<pb::Operation, tonic::Status> {
        self.skills()
            .await
            .sync_skills(pb::SyncSkillsRequest {
                idempotency_key: Some(pb::IdempotencyKey { value: key.to_owned() }),
                machines: vec![box_id().unwrap()],
                plans,
            })
            .await
            .map(tonic::Response::into_inner)
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

fn box_id() -> Option<pb::MachineId> {
    Some(pb::MachineId {
        value: "box-1".to_owned(),
    })
}

fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn on_machine(machine: &FakeMachine, relative: &str) -> PathBuf {
    machine.home().join("Humata/.skills").join(relative)
}

/// Waits until the machine's copy of `relative` holds `bytes`.
async fn machine_has(machine: &FakeMachine, relative: &str, bytes: &[u8]) {
    let deadline = Instant::now() + WAIT;
    while std::fs::read(on_machine(machine, relative)).ok().as_deref() != Some(bytes) {
        assert!(Instant::now() < deadline, "{relative} never reached the machine");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn step_state(operation: &pb::Operation, key: &str) -> pb::StepState {
    let step = operation.steps.iter().find(|step| step.key == key).unwrap();
    pb::StepState::try_from(step.state).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_first_sync_applies_with_its_plan_and_the_next_preview_is_empty() {
    let machine = FakeMachine::new();
    let host = Host::start(&machine).await;
    write(&host.skills_folder("claude").join("alpha/SKILL.md"), b"alpha v1");
    write(&host.skills_folder("claude").join("alpha/.env"), b"SECRET=1");
    host.add_box().await;

    let plan = host.preview().await;
    assert!(plan.first_sync);
    assert_eq!(plan.changes.len(), 1);
    assert_eq!(plan.changes[0].skill, "alpha");
    assert_eq!(plan.changes[0].kind(), pb::skill_change::Kind::Add);
    assert_eq!(plan.excluded_files, 1);
    assert!(plan.token.is_some());

    // A first sync without its plan is refused, and nothing started.
    let refused = host.sync("s0", Vec::new()).await.unwrap_err();
    assert_eq!(refused.code(), Code::FailedPrecondition);
    assert!(!on_machine(&machine, "claude/alpha/SKILL.md").exists());

    let operation = host.sync("s1", plan.token.clone().into_iter().collect()).await.unwrap();
    assert_eq!(operation.kind(), pb::OperationKind::SkillsSync);
    let done = host.watch(&operation).await;
    assert_eq!(done.state, pb::OperationState::Succeeded as i32, "{:?}", done.error);
    assert_eq!(step_state(&done, "sync:box-1"), pb::StepState::Done);
    machine_has(&machine, "claude/alpha/SKILL.md", b"alpha v1").await;
    assert!(!on_machine(&machine, "claude/alpha/.env").exists());

    // The plan was used once.
    let again = host.sync("s2", plan.token.into_iter().collect()).await;
    assert!(again.is_ok(), "nothing needed a plan the second time");

    let next = host.preview().await;
    assert!(!next.first_sync);
    assert!(next.changes.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_machine_removed_and_added_again_has_a_first_sync_that_asks_again() {
    let machine = FakeMachine::new();
    let host = Host::start(&machine).await;
    write(&host.skills_folder("claude").join("alpha/SKILL.md"), b"alpha v1");
    host.add_box().await;
    let plan = host.preview().await;
    let operation = host.sync("s1", plan.token.into_iter().collect()).await.unwrap();
    let done = host.watch(&operation).await;
    assert_eq!(done.state, pb::OperationState::Succeeded as i32, "{:?}", done.error);
    assert!(!host.preview().await.first_sync);

    host.remove_box("r1").await;
    host.add_box_as("-again").await;

    // SK-5: the machine added again is new, so its first sync is shown and
    // asked for, and it doesn't go without the plan.
    let again = host.preview().await;
    assert!(again.first_sync);
    assert_eq!(again.changes.len(), 1);
    assert_eq!(again.changes[0].kind(), pb::skill_change::Kind::Add);
    let refused = host.sync("s2", Vec::new()).await.unwrap_err();
    assert_eq!(refused.code(), Code::FailedPrecondition);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plan_for_other_skills_is_refused() {
    let machine = FakeMachine::new();
    let host = Host::start(&machine).await;
    write(&host.skills_folder("claude").join("alpha/SKILL.md"), b"v1");
    host.add_box().await;
    let plan = host.preview().await;
    write(&host.skills_folder("claude").join("alpha/SKILL.md"), b"v2");
    let refused = host.sync("s1", plan.token.into_iter().collect()).await.unwrap_err();
    assert!(matches!(error_of(&refused).kind, Some(pb::error::Kind::PlanStale(_))));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_offline_machine_is_skipped_and_named_then_syncs_when_its_link_returns() {
    let machine = FakeMachine::new();
    let host = Host::start(&machine).await;
    write(&host.skills_folder("claude").join("alpha/SKILL.md"), b"alpha v1");
    host.add_box().await;

    // The machine goes away and the host restarts: its link can't come up.
    // The plan is made after, from what the host knows (plan tokens live in
    // memory and don't survive a restart).
    machine.set("down");
    let host = host.restart(&machine).await;
    let plan = host.preview().await;
    let operation = host.sync("s1", plan.token.into_iter().collect()).await.unwrap();
    let done = host.watch(&operation).await;
    assert_eq!(done.state, pb::OperationState::Succeeded as i32, "{:?}", done.error);
    assert_eq!(step_state(&done, "sync:box-1"), pb::StepState::Skipped);
    let step = done.steps.iter().find(|step| step.key == "sync:box-1").unwrap();
    assert!(step.output_tail.contains("box-1"));
    assert!(!on_machine(&machine, "claude/alpha/SKILL.md").exists());

    // Back: the link returns and the sync the user asked for goes ahead.
    machine.clear("down");
    machine_has(&machine, "claude/alpha/SKILL.md", b"alpha v1").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_changed_host_skill_syncs_by_itself_when_the_link_next_comes_up() {
    let machine = FakeMachine::new();
    let host = Host::start(&machine).await;
    write(&host.skills_folder("claude").join("alpha/SKILL.md"), b"alpha v1");
    host.add_box().await;
    let plan = host.preview().await;
    let done = host.watch(&host.sync("s1", plan.token.into_iter().collect()).await.unwrap()).await;
    assert_eq!(done.state, pb::OperationState::Succeeded as i32, "{:?}", done.error);
    machine_has(&machine, "claude/alpha/SKILL.md", b"alpha v1").await;

    write(&host.skills_folder("claude").join("alpha/SKILL.md"), b"alpha v2");
    let changed = host.preview().await;
    assert_eq!(changed.changes.len(), 1);
    assert_eq!(changed.changes[0].kind(), pb::skill_change::Kind::Update);

    let host = host.restart(&machine).await;
    machine_has(&machine, "claude/alpha/SKILL.md", b"alpha v2").await;
    assert!(host.preview().await.changes.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_machine_that_opted_out_does_not_sync_by_itself() {
    let machine = FakeMachine::new();
    let host = Host::start(&machine).await;
    write(&host.skills_folder("claude").join("alpha/SKILL.md"), b"alpha v1");
    host.add_box().await;
    let plan = host.preview().await;
    let done = host.watch(&host.sync("s1", plan.token.into_iter().collect()).await.unwrap()).await;
    assert_eq!(done.state, pb::OperationState::Succeeded as i32, "{:?}", done.error);
    machine_has(&machine, "claude/alpha/SKILL.md", b"alpha v1").await;

    write(
        &host.dir.path().join("state/skills-settings.json"),
        br#"{"auto_off": ["box-1"]}"#,
    );
    write(&host.skills_folder("claude").join("alpha/SKILL.md"), b"alpha v2");
    let host = host.restart(&machine).await;
    host.linked().await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(
        std::fs::read(on_machine(&machine, "claude/alpha/SKILL.md")).unwrap(),
        b"alpha v1"
    );
    assert_eq!(host.preview().await.changes.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pick_is_kept_for_the_machine() {
    let machine = FakeMachine::new();
    let host = Host::start(&machine).await;
    write(&host.skills_folder("claude").join("alpha/SKILL.md"), b"alpha v1");
    host.add_box().await;
    host.skills()
        .await
        .resolve_skill_clash(pb::ResolveSkillClashRequest {
            idempotency_key: None,
            machine: box_id(),
            agent: "claude".to_owned(),
            skill: "alpha".to_owned(),
            r#use: pb::skill_clash::Use::Synced as i32,
        })
        .await
        .unwrap();
    // An unspecified pick is refused.
    let refused = host
        .skills()
        .await
        .resolve_skill_clash(pb::ResolveSkillClashRequest {
            idempotency_key: None,
            machine: box_id(),
            agent: "claude".to_owned(),
            skill: "alpha".to_owned(),
            r#use: pb::skill_clash::Use::Unspecified as i32,
        })
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::InvalidArgument);
}

const SESSION: &str = "se_skills";
const TOKEN_HEADER: &str = "humata-session-token";

/// An admin daemon with one session, and the token its shell tab got.
async fn daemon_with_agent() -> (Daemon, String) {
    let dir = folder();
    let store = Store::open(state_folder(dir.path()).join("store")).unwrap();
    store
        .put_session(&SessionRecord {
            id: SESSION.to_owned(),
            channel: "ch_skills".to_owned(),
            machine: String::new(),
            title: "skills".to_owned(),
            agent: "zsh".to_owned(),
            folder: dir.path().to_owned(),
            created_unix_ms: 0,
        })
        .unwrap();
    drop(store);
    let tools = bundle(dir.path());
    let ssh_config = dir.path().join("ssh_config");
    let home = dir.path().join("home");
    let daemon = Daemon::start_with(
        Build::Admin,
        dir,
        &[
            "--tools",
            tools.to_str().unwrap(),
            "--ssh-config",
            ssh_config.to_str().unwrap(),
            "--skills-home",
            home.to_str().unwrap(),
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
async fn an_agent_cannot_preview_sync_or_pick_for_skills() {
    let (daemon, token) = daemon_with_agent().await;
    let mut skills = pb::skills_service_client::SkillsServiceClient::new(daemon.channel().await);

    skills
        .preview_skills_sync(as_agent(pb::PreviewSkillsSyncRequest { machine: box_id() }, &token))
        .await
        .unwrap_err();
    // Previewing is the user's review of a plan: an agent is told to ask the user.
    let refused = skills
        .preview_skills_sync(as_agent(pb::PreviewSkillsSyncRequest { machine: box_id() }, &token))
        .await
        .unwrap_err();
    let error = error_of(&refused);
    assert!(matches!(error.kind, Some(pb::error::Kind::UserOnly(_))), "{refused:?}");
    assert!(!error.fix.is_empty());

    // Syncing is Ask me (MACHINE_PROVISION): the call waits in the Inbox.
    let asked = skills
        .sync_skills(as_agent(
            pb::SyncSkillsRequest {
                machines: vec![pb::MachineId {
                    value: "box-1".to_owned(),
                }],
                ..pb::SyncSkillsRequest::default()
            },
            &token,
        ))
        .await
        .unwrap_err();
    assert!(matches!(error_of(&asked).kind, Some(pb::error::Kind::ApprovalPending(_))), "{asked:?}");
    skills
        .resolve_skill_clash(as_agent(
            pb::ResolveSkillClashRequest {
                idempotency_key: None,
                machine: box_id(),
                agent: "claude".to_owned(),
                skill: "alpha".to_owned(),
                r#use: pb::skill_clash::Use::Local as i32,
            },
            &token,
        ))
        .await
        .unwrap_err();

    // The refused calls started nothing.
    let mut operations = pb::operation_service_client::OperationServiceClient::new(daemon.channel().await);
    let all = operations
        .list_operations(as_agent(pb::ListOperationsRequest::default(), &token))
        .await
        .unwrap()
        .into_inner();
    assert!(all.operations.is_empty());
}

/// D790: the daemon installs Humata's own skills, reference files and
/// all, for the agents set up in the home, refreshes its own copies at the
/// next start, leaves the person's skills alone, and keeps its own out of
/// the sync.
#[tokio::test(flavor = "multi_thread")]
async fn the_daemon_installs_its_own_skills_for_the_agents_set_up_and_never_syncs_them() {
    let machine = FakeMachine::new();
    let dir = folder();
    // Claude Code is set up in this home; Codex and Copilot are not.
    std::fs::create_dir_all(dir.path().join("home/.claude")).unwrap();
    let host = Host::start_in(&machine, dir).await;

    // The socket listens before the daemon has finished starting, so wait
    // for a call to be answered: the install is done by then.
    host.list().await;
    let board = host.skills_folder("claude").join("humata-board");
    let pointer = std::fs::read(board.join("SKILL.md")).expect("the skill is installed");
    assert!(board.join("board.md").is_file(), "its reference file comes with it");
    assert!(board.join("shared/basics.md").is_file(), "and the shared one");
    assert!(!host.dir.path().join("home/.codex").exists(), "an agent that isn't set up gets nothing");
    assert!(!host.dir.path().join("home/.copilot").exists());

    // Humata's copy is edited, and the person has a skill of their own.
    write(&board.join("SKILL.md"), b"edited by hand");
    write(&host.skills_folder("claude").join("mine/SKILL.md"), b"mine");
    let host = host.restart(&machine).await;
    host.list().await;
    assert_eq!(std::fs::read(board.join("SKILL.md")).unwrap(), pointer);
    assert_eq!(
        std::fs::read(host.skills_folder("claude").join("mine/SKILL.md")).unwrap(),
        b"mine"
    );

    // Only the person's skill is offered to a machine.
    host.add_box().await;
    let plan = host.preview().await;
    let offered: Vec<&str> = plan.changes.iter().map(|change| change.skill.as_str()).collect();
    assert_eq!(offered, ["mine"]);
    assert_eq!(plan.excluded_files, 0);
}
