//! `LiveMachines` against a real `humatad2` (admin build) on a socket of its
//! own, with a fake machine behind a fake `ssh`: no network, and never the
//! real home folder or `~/.ssh`. Adds, retry and cancel, the interactive
//! login, providers, a skills sync, and a call the daemon doesn't build yet.
//!
//! Waiting for enabling: this file lives in tests/pending/ until the
//! `Machines` seam lines are in `clients/live/src/lib.rs` and `connection.rs`;
//! then move it to tests/machines.rs and add the `machines_test` target.

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use humata2_clients::DaemonError;
use humata2_clients::machines::{
    AddEvent, AddOutcome, AddRequest, AddStream, AddTarget, LoginEvent, LoginReason, LoginStream, LoginTarget,
    PromptKind,
    Machines, OperationId, StepKind, StepStatus,
};
use humata2_clients::model::MachineId;
use humata2_clients_live::{LiveDaemon, LiveMachines, LiveOptions};
use humata2_fake_machine::FakeMachine;
use humata2_grpc::humata::v1 as pb;
use hyper_util::rt::TokioIo;
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tokio::net::UnixStream;
use tonic::transport::{Channel, Endpoint, Uri};

const FILES: [&str; 3] = ["humatad2", "humata2_holder", "humata"];
const WAIT: Duration = Duration::from_secs(15);
const FOLLOW: Duration = Duration::from_secs(90);

fn binary(variable: &str) -> PathBuf {
    let path = std::env::var_os(variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("set {variable} to the binary's path"));
    std::fs::canonicalize(path).unwrap()
}

/// A tools bundle for linux-x86_64, the fake machine's default platform.
/// Its humatad2 runs the real one, so the peer link really comes up.
fn bundle(dir: &Path) -> PathBuf {
    let root = dir.join("tools");
    std::fs::create_dir_all(root.join("linux-x86_64")).unwrap();
    let mut entries = serde_json::Map::new();
    for file in FILES {
        let body = if file == "humatad2" {
            // A link a test starts gets its state folder named, so it relays
            // nothing and starts no daemon of its own.
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

fn executable(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A login command and home for a provider: without a terminal it asks for
/// a code and fails (the non-interactive try); with one it reads the code,
/// then runs the shell the daemon asked for.
fn fake_login(dir: &Path) -> PathBuf {
    let home = dir.join("machine-home");
    let fakes = home.join(".test-bin");
    std::fs::create_dir_all(&fakes).unwrap();
    executable(
        &fakes.join("uname"),
        "#!/bin/sh\ncase \"$1\" in -s) echo Linux;; -m) echo x86_64;; *) /usr/bin/uname \"$@\";; esac\n",
    );
    executable(&fakes.join("pgrep"), "#!/bin/sh\nexit 1\n");
    let login = dir.join("login.sh");
    executable(
        &login,
        &format!(
            "#!/bin/sh\nexport HOME='{home}'\nexport PATH='{fakes}:/usr/bin:/bin:/usr/sbin:/sbin'\n\
             if [ ! -t 0 ]; then printf 'Verification code: '; exit 1; fi\n\
             printf 'Verification code: '; read code\n\
             [ \"$code\" = 123456 ] || {{ echo denied; exit 3; }}\n\
             exec sh -c \"$1\"\n",
            home = home.display(),
            fakes = fakes.display()
        ),
    );
    login
}

/// The admin daemon with the fake machine's `ssh` first on its PATH, its
/// skills read from `<dir>/home`, and the app's client connected to it.
/// Stops the daemon when dropped.
struct Host {
    child: Child,
    socket: PathBuf,
    dir: TempDir,
    machines: LiveMachines,
    _daemon: LiveDaemon,
}

impl Host {
    async fn start(machine: &FakeMachine) -> Self {
        let dir = tempfile::Builder::new().prefix("h2m").tempdir_in("/tmp").unwrap();
        // The daemon refuses a socket folder other users can open.
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
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
        // From here the guard stops the daemon if the test fails.
        let guard = Stop(Some(child));
        let deadline = Instant::now() + WAIT;
        while UnixStream::connect(&socket).await.is_err() {
            assert!(Instant::now() < deadline, "humatad2 never started listening");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let daemon = LiveDaemon::connect(LiveOptions::new(socket.clone()))
            .await
            .expect("the app connects to the daemon");
        let machines = LiveMachines::new(&daemon);
        let mut guard = guard;
        Self {
            child: guard.0.take().unwrap(),
            socket,
            dir,
            machines,
            _daemon: daemon,
        }
    }

    /// A raw gRPC channel, for what the app's client has no call for.
    async fn raw(&self) -> Channel {
        let socket = self.socket.clone();
        Endpoint::try_from("http://[::]:50051")
            .unwrap()
            .connect_with_connector(tower::service_fn(move |_: Uri| {
                let socket = socket.clone();
                async move { Ok::<_, std::io::Error>(TokioIo::new(UnixStream::connect(socket).await?)) }
            }))
            .await
            .unwrap()
    }

    /// Saves the providers file.
    async fn save_providers(&self, toml: &str) {
        let mut providers = pb::provider_service_client::ProviderServiceClient::new(self.raw().await);
        let revision = providers
            .get_providers_config(pb::GetProvidersConfigRequest::default())
            .await
            .unwrap()
            .into_inner()
            .revision;
        let saved = providers
            .set_providers_config(pb::SetProvidersConfigRequest {
                idempotency_key: None,
                toml: toml.to_owned(),
                expected_revision: revision,
                presence: None,
            })
            .await
            .unwrap()
            .into_inner();
        assert!(saved.problems.is_empty(), "{:?}", saved.problems);
    }

    /// The machines the daemon lists, by name.
    async fn listed(&self) -> Vec<String> {
        pb::machine_service_client::MachineServiceClient::new(self.raw().await)
            .list_machines(pb::ListMachinesRequest::default())
            .await
            .unwrap()
            .into_inner()
            .machines
            .into_iter()
            .map(|machine| machine.name)
            .collect()
    }

    /// box-1 once its peer link is up and it reports a daemon version.
    async fn linked(&self) {
        let deadline = Instant::now() + WAIT;
        loop {
            if let Ok(detail) = self.machines.detail(&box_1()).await
                && !detail.daemon_version.is_empty()
            {
                return;
            }
            assert!(Instant::now() < deadline, "the peer link to box-1 never came up");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Adds box-1 over SSH, confirming the key it presents, and waits for
    /// its link.
    async fn add_box(&self) {
        let fingerprint = self.fingerprint().await;
        let events = follow(self.machines.add(ssh(Some(&fingerprint))).await.unwrap()).await;
        assert!(matches!(outcome(&events), AddOutcome::Added(_)), "{events:?}");
        self.linked().await;
    }

    /// The host key the machine presents, from an add that doesn't trust it.
    async fn fingerprint(&self) -> String {
        let events = follow(self.machines.add(ssh(None)).await.unwrap()).await;
        match outcome(&events) {
            AddOutcome::HostKey { fingerprint } => fingerprint.clone(),
            other => panic!("expected the host key to need confirming, got {other:?}"),
        }
    }
}

struct Stop(Option<Child>);

impl Drop for Stop {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn box_1() -> MachineId {
    MachineId::new("box-1")
}

fn ssh(fingerprint: Option<&str>) -> AddRequest {
    AddRequest {
        target: AddTarget::Ssh {
            host: "me@box-1.internal".to_owned(),
        },
        name: Some("box-1".to_owned()),
        trusted_fingerprint: fingerprint.map(str::to_owned),
    }
}

/// Every event of an add, to the end of its stream.
async fn follow(mut stream: AddStream) -> Vec<AddEvent> {
    let mut seen = Vec::new();
    loop {
        match tokio::time::timeout(FOLLOW, stream.recv()).await {
            Ok(Some(Ok(event))) => seen.push(event),
            Ok(Some(Err(error))) => panic!("the add's stream broke: {error}"),
            Ok(None) => return seen,
            Err(_) => panic!("the add went quiet after {seen:?}"),
        }
    }
}

fn outcome(events: &[AddEvent]) -> &AddOutcome {
    match events.last() {
        Some(AddEvent::Outcome(outcome)) => outcome,
        other => panic!("the add didn't end with an outcome: {other:?}"),
    }
}

fn operation_of(events: &[AddEvent]) -> OperationId {
    match events.first() {
        Some(AddEvent::Started { operation, .. }) => operation.clone(),
        other => panic!("the add didn't start with its operation: {other:?}"),
    }
}

/// Waits for the fake machine to have seen more ssh calls than `before`.
async fn ssh_ran(machine: &FakeMachine, before: usize) {
    let deadline = Instant::now() + WAIT;
    while machine.ssh_calls() <= before {
        assert!(Instant::now() < deadline, "the add never reached the machine");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn missing(host: &Host) {
    let result = host.machines.detail(&box_1()).await;
    assert!(matches!(result, Err(DaemonError::NotFound { .. })), "{result:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_add_without_the_host_key_ends_asking_for_its_fingerprint() {
    let machine = FakeMachine::new();
    let host = Host::start(&machine).await;

    let events = follow(host.machines.add(ssh(None)).await.unwrap()).await;

    let AddOutcome::HostKey { fingerprint } = outcome(&events) else {
        panic!("expected the host key to need confirming: {events:?}");
    };
    assert!(!fingerprint.is_empty());
    assert!(matches!(&events[0], AddEvent::Started { .. }));
    assert!(machine.installed("humatad2").is_none());
    assert!(host.listed().await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_trusted_add_runs_every_step_and_ends_with_the_machine() {
    let machine = FakeMachine::new();
    let host = Host::start(&machine).await;
    let fingerprint = host.fingerprint().await;

    let events = follow(host.machines.add(ssh(Some(&fingerprint))).await.unwrap()).await;

    let AddEvent::Started { steps, .. } = &events[0] else {
        panic!("expected the add to start first: {events:?}");
    };
    assert_eq!(
        steps,
        &[
            StepKind::Connect,
            StepKind::Check,
            StepKind::Install,
            StepKind::Start,
            StepKind::FindAgents
        ]
    );
    let updates: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            AddEvent::Step(update) => Some((update.step, update.status)),
            _ => None,
        })
        .collect();
    for kind in [StepKind::Connect, StepKind::Check, StepKind::Install, StepKind::Start] {
        assert!(updates.contains(&(kind, StepStatus::Done)), "{kind:?} never finished: {updates:?}");
    }
    assert_eq!(outcome(&events), &AddOutcome::Added(box_1()));
    assert_eq!(host.listed().await, vec!["box-1".to_owned()]);
    let detail = host.machines.detail(&box_1()).await.unwrap();
    assert_eq!(detail.machine, box_1());
    assert_eq!(detail.os, "linux");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_add_names_its_step_and_trying_again_follows_the_same_operation() {
    let machine = FakeMachine::new();
    let host = Host::start(&machine).await;
    let fingerprint = host.fingerprint().await;

    machine.set("down");
    let events = follow(host.machines.add(ssh(Some(&fingerprint))).await.unwrap()).await;
    let AddOutcome::Failed(failure) = outcome(&events) else {
        panic!("expected the add to fail: {events:?}");
    };
    assert_eq!(failure.step, StepKind::Connect);
    assert!(host.listed().await.is_empty());
    let operation = operation_of(&events);

    machine.clear("down");
    let again = follow(host.machines.retry(&operation).await.unwrap()).await;
    assert_eq!(operation_of(&again), operation);
    assert_eq!(outcome(&again), &AddOutcome::Added(box_1()));
    assert_eq!(host.listed().await, vec!["box-1".to_owned()]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancelled_add_ends_without_a_machine() {
    let machine = FakeMachine::new();
    let host = Host::start(&machine).await;
    let fingerprint = host.fingerprint().await;

    machine.set("hang");
    let calls = machine.ssh_calls();
    let mut stream = host.machines.add(ssh(Some(&fingerprint))).await.unwrap();
    let Some(Ok(AddEvent::Started { operation, .. })) = stream.recv().await else {
        panic!("expected the add to start");
    };
    ssh_ran(&machine, calls).await;
    host.machines.cancel(&operation).await.unwrap();
    machine.clear("hang");

    let events = follow(stream).await;

    assert!(
        !events.iter().any(|event| matches!(event, AddEvent::Outcome(AddOutcome::Added(_)))),
        "{events:?}"
    );
    assert!(host.listed().await.is_empty());
    missing(&host).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn providers_are_listed_with_what_they_ask_for() {
    let machine = FakeMachine::new();
    let host = Host::start(&machine).await;
    host.save_providers(
        "[providers.fake]\nlabel = \"Fake\"\n\
         params = [{ name = \"zone\", prompt = \"Zone\", default = \"eu\" }, { name = \"size\", prompt = \"Size\" }]\n\
         create = 'make-machine {name}'\nconnect = 'reach-machine {name} -- {cmd}'\n",
    )
    .await;

    let choices = host.machines.choices().await.unwrap();

    let fake = choices
        .providers
        .iter()
        .find(|provider| provider.id == "fake")
        .expect("the saved provider is offered");
    assert_eq!(fake.label, "Fake");
    assert!(!fake.interactive);
    let zone = fake.params.iter().find(|param| param.name == "zone").unwrap();
    assert_eq!((zone.label.as_str(), zone.default.as_str(), zone.required), ("Zone", "eu", false));
    let size = fake.params.iter().find(|param| param.name == "size").unwrap();
    assert!(size.required);
}

async fn read_login(
    stream: &mut LoginStream,
    done: impl Fn(&[u8], &Option<LoginEvent>) -> bool,
) -> (Vec<u8>, Option<LoginEvent>) {
    let mut output = Vec::new();
    let mut last = None;
    while !done(output.as_slice(), &last) {
        match tokio::time::timeout(FOLLOW, stream.recv()).await {
            Ok(Some(Ok(LoginEvent::Output(bytes)))) => output.extend_from_slice(&bytes),
            Ok(Some(Ok(event))) => last = Some(event),
            Ok(Some(Err(error))) => panic!("the login's stream broke: {error}"),
            Ok(None) => break,
            Err(_) => panic!("the login went quiet after {:?}", String::from_utf8_lossy(&output)),
        }
    }
    (output, last)
}

fn shows(output: &[u8], text: &str) -> bool {
    String::from_utf8_lossy(output).contains(text)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_provider_that_needs_a_login_is_logged_in_by_hand_and_becomes_a_machine() {
    let machine = FakeMachine::new();
    let host = Host::start(&machine).await;
    let login = fake_login(host.dir.path());
    host.save_providers(&format!(
        "[providers.corp]\nlabel = \"Corp\"\ncreate = \"true\"\nconnect = \"{} {{cmd}}\"\n",
        login.display()
    ))
    .await;

    let events = follow(
        host.machines
            .add(AddRequest {
                target: AddTarget::Provider {
                    provider: "corp".to_owned(),
                    params: BTreeMap::new(),
                },
                name: Some("box-1".to_owned()),
                trusted_fingerprint: None,
            })
            .await
            .unwrap(),
    )
    .await;
    let AddOutcome::NeedsLogin { reason: LoginReason::Prompted { kind, line } } = outcome(&events) else {
        panic!("expected the connect to need a login after a prompt: {events:?}");
    };
    assert_eq!(*kind, PromptKind::Code);
    assert!(!line.is_empty());
    assert_eq!(&events.iter().find_map(|event| match event {
        AddEvent::Started { steps, .. } => steps.first().copied(),
        _ => None,
    }), &Some(StepKind::Create));
    assert!(host.listed().await.is_empty());

    let target = LoginTarget::Operation(operation_of(&events));
    let mut stream = host.machines.open_login(&target).await.unwrap();
    let (output, seen) = read_login(&mut stream, |output, _| shows(output, "Verification code")).await;
    assert!(shows(&output, "Verification code"));
    // The daemon sends the prompt's kind before its output (D167).
    assert!(
        matches!(seen, Some(LoginEvent::Prompt { kind: PromptKind::Code, .. })),
        "the terminal's prompt is reported with its kind: {seen:?}"
    );
    host.machines.login_resize(&target, 100, 30).await.unwrap();
    host.machines.login_input(&target, b"123456\n".to_vec()).await.unwrap();
    // Done is pressed once the machine's shell shows its prompt.
    read_login(&mut stream, |output, _| shows(output, "$ ")).await;
    host.machines.login_done(&target).await.unwrap();
    let (_, last) = read_login(&mut stream, |_, last| last.is_some()).await;

    let Some(LoginEvent::Ready { machine: ready, .. }) = last else {
        panic!("expected the login to end ready, got {last:?}");
    };
    assert_eq!(ready, box_1());
    assert_eq!(host.listed().await, vec!["box-1".to_owned()]);
    // The login has ended: soon typing into it finds nothing open.
    let deadline = Instant::now() + WAIT;
    loop {
        match host.machines.login_input(&target, b"x".to_vec()).await {
            Err(DaemonError::NotFound { .. }) => break,
            other => assert!(Instant::now() < deadline, "the ended login stayed open: {other:?}"),
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn typing_into_a_login_that_isnt_open_is_not_found() {
    let machine = FakeMachine::new();
    let host = Host::start(&machine).await;
    let target = LoginTarget::Machine(box_1());

    for result in [
        host.machines.login_input(&target, b"x".to_vec()).await,
        host.machines.login_resize(&target, 80, 24).await,
        host.machines.login_done(&target).await,
        host.machines.login_cancel(&target).await,
    ] {
        assert!(matches!(result, Err(DaemonError::NotFound { .. })), "{result:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_first_skills_sync_is_previewed_then_applied_with_its_plan() {
    let machine = FakeMachine::new();
    let host = Host::start(&machine).await;
    let skill = host.dir.path().join("home/.claude/skills/alpha/SKILL.md");
    std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
    std::fs::write(&skill, b"alpha v1").unwrap();
    host.add_box().await;

    let preview = host.machines.skills_preview(&box_1()).await.unwrap();
    assert!(preview.first_sync);
    assert_eq!(preview.added, vec!["alpha".to_owned()]);
    assert!(preview.clashes.is_empty());

    host.machines.sync_skills(&box_1()).await.unwrap();

    let on_machine = machine.home().join("Humata/.skills/claude/alpha/SKILL.md");
    let deadline = Instant::now() + WAIT;
    while std::fs::read(&on_machine).ok().as_deref() != Some(b"alpha v1".as_slice()) {
        assert!(Instant::now() < deadline, "the skill never reached the machine");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let next = host.machines.skills_preview(&box_1()).await.unwrap();
    assert!(!next.first_sync);
    assert!(next.added.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn host_variables_answer_not_served_yet() {
    let machine = FakeMachine::new();
    let host = Host::start(&machine).await;
    host.add_box().await;

    let set = host.machines.set_variable(&box_1(), "TOKEN", "secret").await;
    let removed = host.machines.remove_variable(&box_1(), "TOKEN").await;

    assert!(matches!(set, Err(DaemonError::NotServedYet { .. })), "{set:?}");
    assert!(matches!(removed, Err(DaemonError::NotServedYet { .. })), "{removed:?}");
}

/// What the daemon says of box-1's reachability, once it says the machine
/// answers.
async fn answering(host: &Host) {
    let deadline = Instant::now() + WAIT;
    loop {
        let listed = pb::machine_service_client::MachineServiceClient::new(host.raw().await)
            .list_machines(pb::ListMachinesRequest::default())
            .await
            .unwrap()
            .into_inner()
            .machines;
        if listed
            .iter()
            .any(|machine| machine.name == "box-1" && machine.reachability == pb::Reachability::Online as i32)
        {
            return;
        }
        assert!(Instant::now() < deadline, "the daemon never listed box-1 as answering: {listed:?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// HO-3: the app that starts after a machine was added lists it from the
/// daemon, not only the machines an event has mentioned since.
#[tokio::test(flavor = "multi_thread")]
async fn an_app_that_starts_later_lists_the_machines_the_daemon_has() {
    use humata2_clients::Daemon as _;
    use humata2_clients::model::MachineKind;

    let machine = FakeMachine::new();
    let host = Host::start(&machine).await;
    host.add_box().await;
    answering(&host).await;

    // A client of its own, connected after the add: it saw none of its events.
    let later = LiveDaemon::connect(LiveOptions::new(host.socket.clone()))
        .await
        .expect("a second app connects to the daemon");
    let workspace = later.load().await.expect("the workspace loads");

    let names: Vec<&str> = workspace.machines.iter().map(|machine| machine.name.as_str()).collect();
    let added = workspace
        .machines
        .iter()
        .find(|machine| machine.id == box_1())
        .unwrap_or_else(|| panic!("the workspace lists box-1 beside this machine, got {names:?}"));
    assert_eq!(added.name, "box-1");
    assert!(added.online, "box-1 answers, so it is listed as online");
    assert!(matches!(added.kind, MachineKind::Ssh { .. }), "{:?}", added.kind);
    let this: Vec<_> = workspace
        .machines
        .iter()
        .filter(|machine| machine.kind == MachineKind::ThisMac)
        .collect();
    assert_eq!(this.len(), 1, "this machine is listed once: {names:?}");
    assert_eq!(workspace.machines.len(), 2, "{names:?}");
}
