//! Provider TOML lifecycle contract extensions and sleep/wake ephemeral-state
//! restoration tests.

use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use humata2_machines::{
    known_hosts, parse_list, parse_status, parse_status_report, Action, Builds, BuildsError,
    ConfigError, Drive, EntryState, Exit, HostKey, IdentityError, InstalledFile, Invocation, Link,
    MachineName, MachineStatus, Machines, MachinesConfig, ManageError, Output, Payload, Platform,
    ProblemKind, Progress, Provider, Purpose, Reach, RetryPolicy, RunError, Runner, Setup,
    SetupError, SshAccess, SshTarget, Step, StoredLink, SystemRunner,
};

const KEY_A: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl";
const KEY_B: &str =
    "ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBEmKSENjQEezOmxkZMy7opKgwFB9nkt5YRrYMjNuG5N87uRgg6CLrbo5wAdT/y6v0mKV0U2w0WZ2YB/++Tpockg=";

fn name(value: &str) -> MachineName {
    MachineName::new(value).unwrap()
}

fn ok(stdout: &[u8]) -> Output {
    Output {
        exit: Exit::Code(0),
        stdout: stdout.to_vec(),
        stderr: Vec::new(),
    }
}

struct ScriptedRunner {
    responses: Mutex<VecDeque<Output>>,
    seen: Mutex<Vec<Invocation>>,
}

impl ScriptedRunner {
    fn new(responses: Vec<Output>) -> Self {
        Self {
            responses: Mutex::new(responses.into()),
            seen: Mutex::new(Vec::new()),
        }
    }

    fn seen(&self) -> Vec<Invocation> {
        self.seen.lock().unwrap().clone()
    }
}

impl Runner for ScriptedRunner {
    fn run(&self, invocation: &Invocation) -> Result<Output, RunError> {
        self.seen.lock().unwrap().push(invocation.clone());
        let next = self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| panic!("unexpected invocation: {invocation:?}"));
        Ok(next)
    }
}

struct FakeBuilds {
    payloads: Vec<Payload>,
    bundle: Option<Arc<[u8]>>,
}

impl FakeBuilds {
    fn sample() -> Self {
        Self {
            payloads: vec![
                Payload::new(InstalledFile::Humatad, Arc::from(&b"daemon-v2"[..])),
                Payload::new(InstalledFile::Holder, Arc::from(&b"holder-v2"[..])),
                Payload::new(InstalledFile::Cli, Arc::from(&b"cli-v2"[..])),
                Payload::new(InstalledFile::Stack, Arc::from(&b"stack-v2"[..])),
                Payload::new(InstalledFile::FuseFs, Arc::from(&b"#!/bin/sh\nexit 4\n"[..])),
            ],
            bundle: Some(Arc::from(&br#"{"version":2}"#[..])),
        }
    }

    fn sha(&self, file: InstalledFile) -> &str {
        self.payloads
            .iter()
            .find(|p| p.file() == file)
            .unwrap()
            .sha256()
    }
}

impl Builds for FakeBuilds {
    fn payloads(&self, _platform: Platform) -> Result<Vec<Payload>, BuildsError> {
        Ok(self.payloads.clone())
    }

    fn bundle_json(&self, _platform: Platform) -> Result<Option<Arc<[u8]>>, BuildsError> {
        Ok(self.bundle.clone())
    }
}

fn check_facts_output(
    builds: &FakeBuilds,
    installed: &[InstalledFile],
    mismatched: &[InstalledFile],
    running: Option<&str>,
    enrollment_key: &str,
) -> Output {
    let mut text = String::from(
        "humata2.os=Linux\n\
         humata2.arch=x86_64\n\
         humata2.user=dev\n\
         humata2.home=/home/dev\n\
         humata2.git=yes\n\
         humata2.disk_free_kib=10485760\n\
         humata2.systemd=yes\n\
         humata2.systemd_init=yes\n\
         humata2.linger=yes\n\
         humata2.crontab=yes\n\
         humata2.setsid=yes\n",
    );
    if let Some(svc) = running {
        text.push_str(&format!("humata2.running={svc}\n"));
    }
    text.push_str(&format!("humata2.enrollment_key={enrollment_key}\n"));
    for file in InstalledFile::ALL {
        let name = file.file_name();
        if mismatched.contains(&file) {
            let wrong = "0".repeat(64);
            text.push_str(&format!("humata2.sha.{name}={wrong}\n"));
        } else if installed.contains(&file) {
            text.push_str(&format!("humata2.sha.{name}={}\n", builds.sha(file)));
        } else {
            text.push_str(&format!("humata2.sha.{name}=none\n"));
        }
    }
    ok(text.as_bytes())
}

fn drive_setup(mut setup: Setup, runner: &dyn Runner, builds: &dyn Builds) -> Progress {
    loop {
        match setup.step(runner, builds) {
            Progress::Continue => continue,
            Progress::RetryAfter { .. } => continue,
            other => return other,
        }
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    // Reuse Payload's sha256 calculation without adding a dev-dependency.
    Payload::new(InstalledFile::Cli, Arc::from(bytes))
        .sha256()
        .to_owned()
}

// ---------------------------------------------------------------------------
// 1. Optional `create`, paired `identity` + `connect`, and new actions/fields
// ---------------------------------------------------------------------------

#[test]
fn create_is_optional_when_connect_identity_or_list_is_declared() {
    let connect_only = Provider::parse(
        "[providers.fleet]\nconnect = \"fleetctl exec {name} -- {cmd}\"\n",
    )
    .unwrap()
    .pop()
    .unwrap();
    assert!(!connect_only.has(Action::Create));
    assert!(connect_only.has_connect());
    assert!(!connect_only.has_identity());
    assert_eq!(connect_only.reach(), Reach::Connect);
    assert!(!connect_only.wake_on_demand());

    let identity_only = Provider::parse(
        "[providers.pool]\nidentity = \"poolctl inspect {name}\"\n",
    )
    .unwrap()
    .pop()
    .unwrap();
    assert!(!identity_only.has(Action::Create));
    assert!(!identity_only.has_connect());
    assert!(identity_only.has_identity());
    assert_eq!(identity_only.reach(), Reach::Identity);

    let list_only = Provider::parse(
        "[providers.inventory]\nlist = \"invctl ls --json\"\n",
    )
    .unwrap()
    .pop()
    .unwrap();
    assert!(!list_only.has(Action::Create));
    assert!(list_only.has(Action::List));
    assert!(list_only.list_invocation().is_some());

    // None of connect, identity, or list -> MissingLink.
    let Err(ConfigError::Problems(problems)) =
        Provider::parse("[providers.empty]\nstart = \"svc start {name}\"\n")
    else {
        panic!("expected MissingLink problem");
    };
    assert!(problems.0.iter().any(|p| p.kind == ProblemKind::MissingLink));

    // Unknown fields remain strictly rejected.
    let Err(ConfigError::Syntax { .. }) =
        Provider::parse("[providers.bad]\nlist = \"true\"\nextra_unknown = \"x\"\n")
    else {
        panic!("expected deny_unknown_fields syntax error");
    };
}

#[test]
fn wake_refresh_list_ssh_and_wake_on_demand_contract() {
    let toml = r#"
[providers.ephemeral]
label = "Ephemeral Pool"
list = "poolctl ls --json"
identity = "poolctl id {name}"
connect = "poolctl stdio {name} -- {cmd}"
start = "poolctl start {name}"
stop = "poolctl stop {name}"
wake = "poolctl wake {name}"
refresh = "poolctl refresh {name}"
ssh = "poolctl ssh {name}"
status = "poolctl status {name}"
"#;
    let provider = Provider::parse(toml).unwrap().pop().unwrap();
    assert!(provider.wake_on_demand());
    for action in [
        Action::Identity,
        Action::List,
        Action::Start,
        Action::Stop,
        Action::Wake,
        Action::Refresh,
        Action::Ssh,
        Action::Status,
    ] {
        assert!(provider.has(action), "missing {action}");
    }
    let fields: Vec<&str> = provider.commands().into_iter().map(|(f, _)| f).collect();
    assert_eq!(
        fields,
        ["connect", "identity", "list", "start", "stop", "wake", "refresh", "ssh", "status"]
    );

    // Fallback: when `wake` is omitted and `start` is declared, `wake_on_demand`
    // defaults to true and `Action::Wake` falls back to `start`.
    let fallback = Provider::parse(
        "[providers.fb]\nidentity = \"id {name}\"\nstart = \"boot {name}\"\n",
    )
    .unwrap()
    .pop()
    .unwrap();
    assert!(fallback.wake_on_demand());
    assert!(fallback.has(Action::Wake));
    let inv = fallback
        .invocation(Action::Wake, &name("box-1"), &BTreeMap::new())
        .unwrap()
        .unwrap();
    assert_eq!(inv.purpose, Purpose::Wake);
    assert_eq!(inv.args[1], "boot box-1");

    // Explicit `wake_on_demand = false` disables fallback to `start`.
    let no_wake = Provider::parse(
        "[providers.nw]\nidentity = \"id {name}\"\nstart = \"boot {name}\"\nwake_on_demand = false\n",
    )
    .unwrap()
    .pop()
    .unwrap();
    assert!(!no_wake.wake_on_demand());
    assert!(!no_wake.has(Action::Wake));
    assert!(no_wake
        .invocation(Action::Wake, &name("box-1"), &BTreeMap::new())
        .unwrap()
        .is_none());

    // `{cmd}` outside `connect` is rejected on new commands too.
    let Err(ConfigError::Problems(problems)) = Provider::parse(
        "[providers.badcmd]\nconnect = \"c {name} {cmd}\"\nwake = \"w {cmd}\"\nrefresh = \"r {cmd}\"\nlist = \"l {cmd}\"\nssh = \"s {cmd}\"\n",
    ) else {
        panic!("expected CmdOutsideConnect problems");
    };
    let bad_fields: Vec<&str> = problems.0.iter().map(|p| p.field.as_str()).collect();
    assert_eq!(bad_fields, ["list", "wake", "refresh", "ssh"]);
}

#[test]
fn structured_status_json_and_plain_text_backward_compatibility() {
    assert_eq!(parse_status(b"running\n"), MachineStatus::Running);
    assert_eq!(parse_status(b"sleeping\n"), MachineStatus::Sleeping);
    assert_eq!(parse_status(b"suspended\n"), MachineStatus::Sleeping);
    assert_eq!(parse_status(b"stopped\n"), MachineStatus::Stopped);
    assert_eq!(parse_status(b"starting\n"), MachineStatus::Starting);
    assert_eq!(parse_status(b"missing\n"), MachineStatus::Missing);

    let report = parse_status_report(
        format!(
            r#"{{"state":"sleeping","instance_id":"inst-99","ssh":{{"host":"10.1.2.3","user":"dev","port":2222,"host_key":"{KEY_A}"}}}}"#
        )
        .as_bytes(),
    )
    .unwrap();
    assert_eq!(report.state, MachineStatus::Sleeping);
    assert_eq!(report.instance_id.as_deref(), Some("inst-99"));
    let ssh = report.ssh.unwrap();
    assert_eq!(ssh.host, "10.1.2.3");
    assert_eq!(
        ssh.access,
        SshAccess::Direct {
            port: 2222,
            user: "dev".to_owned(),
            identity_file: None,
        }
    );
    assert_eq!(ssh.host_key.unwrap(), HostKey::parse(KEY_A).unwrap());

    // Plain-text fallback in parse_status_report.
    let plain = parse_status_report(b"notice\nstarting\n").unwrap();
    assert_eq!(plain.state, MachineStatus::Starting);
    assert_eq!(plain.instance_id, None);
    assert_eq!(plain.ssh, None);

    // Key material is refused.
    assert!(matches!(
        parse_status_report(b"{\"state\":\"running\",\"x\":\"-----BEGIN OPENSSH PRIVATE KEY-----\"}"),
        Err(IdentityError::KeyMaterial)
    ));
}

#[test]
fn paired_identity_and_connect_runs_identity_then_uses_connect_transport() {
    let dir = tempfile::tempdir().unwrap();
    let known = dir.path().join("known_hosts");
    let provider = Provider::parse(
        "[providers.hybrid]\nidentity = \"id {name}\"\nconnect = \"relay {name} -- {cmd}\"\n",
    )
    .unwrap()
    .pop()
    .unwrap();
    let builds = FakeBuilds::sample();

    let id_json = format!(
        r#"{{"name":"box-1","ssh":{{"host":"10.0.0.9","user":"dev","host_key":"{KEY_A}"}},"os":"linux","arch":"x86_64","state":"running"}}"#
    );
    let runner = ScriptedRunner::new(vec![
        ok(id_json.as_bytes()),
        ok(b"Linux x86_64\n"),
        check_facts_output(&builds, &InstalledFile::ALL, &[], Some("systemd"), "none"),
    ]);

    let setup = Setup::new(provider, name("box-1"), BTreeMap::new(), &known).unwrap();
    // Without `create`, starts directly at Step::Identity.
    assert_eq!(setup.current(), Step::Identity);

    let Progress::Ready(ready) = drive_setup(setup, &runner, &builds) else {
        panic!("expected Ready");
    };
    // Transport is `connect`, while `identity` attested and pinned the host key.
    assert!(ready.link.is_connect());
    assert_eq!(
        known_hosts::pinned(&known, &name("box-1")).unwrap(),
        HostKey::parse(KEY_A)
    );
    let purposes: Vec<Purpose> = runner.seen().iter().map(|i| i.purpose).collect();
    assert_eq!(purposes, [Purpose::Identity, Purpose::Probe, Purpose::Check]);
}

// ---------------------------------------------------------------------------
// 2. Atomic known_hosts::replace
// ---------------------------------------------------------------------------

#[test]
fn known_hosts_replace_atomically_updates_only_target_alias() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state/known_hosts");
    let key_a = HostKey::parse(KEY_A).unwrap();
    let key_b = HostKey::parse(KEY_B).unwrap();

    known_hosts::replace(&path, "humata2.box-1", &key_a).unwrap();
    known_hosts::replace(&path, "humata2.box-2", &key_a).unwrap();
    known_hosts::replace(&path, "humata2.box-1", &key_b).unwrap();

    assert_eq!(
        known_hosts::pinned_alias(&path, "humata2.box-1").unwrap(),
        Some(key_b)
    );
    assert_eq!(
        known_hosts::pinned_alias(&path, "humata2.box-2").unwrap(),
        Some(key_a)
    );
    let mode = fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    // No temporary files left beside known_hosts.
    assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
}

// ---------------------------------------------------------------------------
// 3. Machines::list_instances & Machines::refresh_reach
// ---------------------------------------------------------------------------

fn test_machines(dir: &Path) -> Machines {
    Machines::open(MachinesConfig {
        hosts_file: dir.join("hosts.json"),
        known_hosts: dir.join("known_hosts"),
        ssh_config: dir.join("ssh_config"),
        peer_keys: dir.join("peer-keys"),
        retry: RetryPolicy {
            attempts: 2,
            first_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(1),
        },
    })
}

#[test]
fn list_instances_parses_discovered_instances_and_rejects_secrets() {
    let dir = tempfile::tempdir().unwrap();
    let machines = test_machines(dir.path());
    let provider = Provider::parse("[providers.pool]\nlist = \"poolctl ls\"\n")
        .unwrap()
        .pop()
        .unwrap();

    let json = format!(
        r#"[
            {{"name":"dev-1","state":"running","params":{{"size":"large"}},"instance_id":"i-1","ssh":{{"host":"10.0.1.1","user":"ubuntu","port":2222,"host_key":"{KEY_A}"}}}},
            {{"name":"dev-2","state":"sleeping","instance_id":"i-2"}}
        ]"#
    );
    let runner = ScriptedRunner::new(vec![ok(json.as_bytes())]);
    let instances = machines.list_instances(&provider, &runner).unwrap();
    assert_eq!(instances.len(), 2);
    assert_eq!(instances[0].name, "dev-1");
    assert_eq!(instances[0].state, MachineStatus::Running);
    assert_eq!(instances[0].params.get("size").map(String::as_str), Some("large"));
    assert_eq!(instances[0].instance_id.as_deref(), Some("i-1"));
    assert_eq!(instances[0].ssh.as_ref().unwrap().host, "10.0.1.1");
    assert_eq!(instances[1].name, "dev-2");
    assert_eq!(instances[1].state, MachineStatus::Sleeping);
    assert!(instances[1].ssh.is_none());

    // parse_list rejects key material.
    assert!(matches!(
        parse_list(b"[{\"name\":\"x\",\"state\":\"running\",\"instance_id\":\"PRIVATE KEY\"}]"),
        Err(IdentityError::KeyMaterial)
    ));

    // Provider without `list` returns ActionUndefined.
    let no_list = Provider::parse("[providers.nl]\nconnect = \"c {name} {cmd}\"\n")
        .unwrap()
        .pop()
        .unwrap();
    let empty_runner = ScriptedRunner::new(vec![]);
    assert!(matches!(
        machines.list_instances(&no_list, &empty_runner),
        Err(ManageError::ActionUndefined {
            action: Action::List,
            ..
        })
    ));
}

#[test]
fn refresh_reach_runs_refresh_wake_identity_and_atomically_updates_hosts_and_known_hosts() {
    let dir = tempfile::tempdir().unwrap();
    let machines = test_machines(dir.path());
    let builds = FakeBuilds::sample();

    let provider = Provider::parse(
        r#"
[providers.dyncloud]
create = "dyn create {name}"
identity = "dyn id {name}"
status = "dyn status {name}"
wake = "dyn wake {name}"
refresh = "dyn refresh {name}"
"#,
    )
    .unwrap()
    .pop()
    .unwrap();

    // First create and register the machine with KEY_A and initial IP 10.0.0.1.
    let initial_id = format!(
        r#"{{"name":"box-1","ssh":{{"host":"10.0.0.1","user":"dev","port":22,"host_key":"{KEY_A}"}},"os":"linux","arch":"x86_64"}}"#
    );
    let create_runner = ScriptedRunner::new(vec![
        ok(b""),                                                             // create
        ok(initial_id.as_bytes()),                                           // identity
        ok(b"Linux x86_64\n"),                                               // probe
        check_facts_output(&builds, &InstalledFile::ALL, &[], Some("systemd"), "none"), // check
        ok(b""),                                                             // enroll
    ]);
    let plan = machines
        .plan_create(&provider, "box-1", BTreeMap::new())
        .unwrap();
    let mut events = Vec::new();
    let wait = |_: Duration| {};
    let cancelled = || false;
    let mut push = |e| events.push(e);
    let mut drive = Drive {
        runner: &create_runner,
        builds: &builds,
        wait: &wait,
        events: &mut push,
        cancelled: &cancelled,
    };
    let entry = machines.create(plan, &mut drive).unwrap();
    assert_eq!(entry.state, EntryState::Ready);
    assert_eq!(
        known_hosts::pinned(&dir.path().join("known_hosts"), &name("box-1")).unwrap(),
        HostKey::parse(KEY_A)
    );

    // Now the machine went to sleep and on wake gets a new IP 10.0.0.77, port 2202, and KEY_B.
    let refreshed_id = format!(
        r#"{{"name":"box-1","ssh":{{"host":"10.0.0.77","user":"dev","port":2202,"host_key":"{KEY_B}"}},"os":"linux","arch":"x86_64","state":"running"}}"#
    );
    let refresh_runner = ScriptedRunner::new(vec![
        ok(b"refreshed\n"),                    // refresh
        ok(br#"{"state":"sleeping"}"#),        // status -> sleeping
        ok(b"woken\n"),                        // wake
        ok(refreshed_id.as_bytes()),           // identity -> new endpoint & KEY_B
    ]);

    let updated_link = machines
        .refresh_reach(&provider, "box-1", &refresh_runner)
        .unwrap();
    assert!(!updated_link.is_connect());

    let purposes: Vec<Purpose> = refresh_runner.seen().iter().map(|i| i.purpose).collect();
    assert_eq!(
        purposes,
        [Purpose::Refresh, Purpose::Status, Purpose::Wake, Purpose::Identity]
    );

    // hosts.json has the updated StoredLink.
    let stored = machines
        .get("box-1")
        .unwrap()
        .provider
        .unwrap()
        .link
        .unwrap();
    assert_eq!(
        stored,
        StoredLink::Ssh {
            host: "10.0.0.77".to_owned(),
            ssh_config: None,
            port: Some(2202),
            user: Some("dev".to_owned()),
            identity_file: None,
        }
    );

    // known_hosts was atomically replaced with attested KEY_B.
    assert_eq!(
        known_hosts::pinned(&dir.path().join("known_hosts"), &name("box-1")).unwrap(),
        HostKey::parse(KEY_B)
    );
}

// ---------------------------------------------------------------------------
// 4. Setup::ensure_ready: ephemeral binary repair & safe enrollment restoration
// ---------------------------------------------------------------------------

fn direct_link(known_hosts: &Path) -> Link {
    Link::ssh(
        name("box-1"),
        SshTarget {
            host: "10.0.0.1".to_owned(),
            access: SshAccess::Direct {
                port: 22,
                user: "dev".to_owned(),
                identity_file: None,
            },
            host_key: None,
        },
        known_hosts,
    )
}

#[test]
fn ensure_ready_repairs_wiped_bin_restores_missing_enrollment_and_starts_daemon() {
    let dir = tempfile::tempdir().unwrap();
    let known = dir.path().join("known_hosts");
    let builds = FakeBuilds::sample();
    let host_key_bytes: Arc<[u8]> = Arc::from(&b"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"[..]);

    // Wiped ~/.humata2/bin, missing enrollment-key, daemon not running.
    let runner = ScriptedRunner::new(vec![
        ok(b"Linux x86_64\n"),                                    // probe
        check_facts_output(&builds, &[], &[], None, "none"),      // check
        ok(b""),                                                  // enroll (restore)
        ok(b""),                                                  // install humatad2
        ok(b""),                                                  // install holder
        ok(b""),                                                  // install cli
        ok(b""),                                                  // install stack
        ok(b""),                                                  // install fusefs
        ok(b""),                                                  // install bundle.json
        ok(b""),                                                  // start service
    ]);

    let setup = Setup::ensure_ready(direct_link(&known), &known)
        .with_restore_enrollment(Arc::clone(&host_key_bytes));

    let Progress::Ready(_) = drive_setup(setup, &runner, &builds) else {
        panic!("expected Ready after ephemeral repair");
    };

    let purposes: Vec<Purpose> = runner.seen().iter().map(|i| i.purpose).collect();
    assert_eq!(
        purposes,
        [
            Purpose::Probe,
            Purpose::Check,
            Purpose::Enroll,
            Purpose::Install,
            Purpose::Install,
            Purpose::Install,
            Purpose::Install,
            Purpose::Install,
            Purpose::Install,
            Purpose::Service,
        ]
    );
    // The restored enrollment key was sent on stdin.
    let enroll_inv = runner
        .seen()
        .into_iter()
        .find(|i| i.purpose == Purpose::Enroll)
        .unwrap();
    assert_eq!(enroll_inv.stdin.as_deref(), Some(&host_key_bytes[..]));
}

#[test]
fn ensure_ready_repairs_mismatched_binaries_and_skips_matching_enrollment() {
    let dir = tempfile::tempdir().unwrap();
    let known = dir.path().join("known_hosts");
    let builds = FakeBuilds::sample();
    let host_key_bytes: Arc<[u8]> = Arc::from(&b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"[..]);
    let matching_sha = sha256_hex(&host_key_bytes);

    // Humatad & Cli have mismatched hashes; enrollment key already matches; daemon running.
    let runner = ScriptedRunner::new(vec![
        ok(b"Linux x86_64\n"), // probe
        check_facts_output(
            &builds,
            &[InstalledFile::Holder, InstalledFile::Stack],
            &[InstalledFile::Humatad, InstalledFile::Cli],
            Some("systemd"),
            &matching_sha,
        ),
        ok(b""), // repair humatad2
        ok(b""), // repair cli
        ok(b""), // put back the missing fusefs helper
        ok(b""), // write bundle.json
        ok(b""), // restart service
    ]);

    let setup = Setup::ensure_ready(direct_link(&known), &known)
        .with_restore_enrollment(host_key_bytes);

    let Progress::Ready(_) = drive_setup(setup, &runner, &builds) else {
        panic!("expected Ready after binary repair");
    };

    let purposes: Vec<Purpose> = runner.seen().iter().map(|i| i.purpose).collect();
    assert_eq!(
        purposes,
        [
            Purpose::Probe,
            Purpose::Check,
            Purpose::Install,
            Purpose::Install,
            Purpose::Install,
            Purpose::Install,
            Purpose::Service,
        ]
    );
}

#[test]
fn ensure_ready_refuses_to_overwrite_conflicting_enrollment_key() {
    let dir = tempfile::tempdir().unwrap();
    let known = dir.path().join("known_hosts");
    let builds = FakeBuilds::sample();
    let host_key_bytes: Arc<[u8]> = Arc::from(&b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"[..]);
    let other_key_sha = sha256_hex(b"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");

    let runner = ScriptedRunner::new(vec![
        ok(b"Linux x86_64\n"), // probe
        check_facts_output(
            &builds,
            &InstalledFile::ALL,
            &[],
            Some("systemd"),
            &other_key_sha,
        ),
    ]);

    let setup = Setup::ensure_ready(direct_link(&known), &known)
        .with_restore_enrollment(host_key_bytes);

    let Progress::Failed(failure) = drive_setup(setup, &runner, &builds) else {
        panic!("expected EnrollmentKeyConflict failure");
    };
    assert_eq!(failure.step, Step::Check);
    assert!(matches!(
        failure.error,
        SetupError::EnrollmentKeyConflict { .. }
    ));
    // Never ran Enroll or Install.
    assert!(!runner
        .seen()
        .iter()
        .any(|i| matches!(i.purpose, Purpose::Enroll | Purpose::Install)));
}

#[test]
fn ensure_ready_is_noop_when_binaries_enrollment_and_service_are_healthy() {
    let dir = tempfile::tempdir().unwrap();
    let known = dir.path().join("known_hosts");
    let builds = FakeBuilds::sample();
    let host_key_bytes: Arc<[u8]> = Arc::from(&b"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"[..]);
    let matching_sha = sha256_hex(&host_key_bytes);

    let runner = ScriptedRunner::new(vec![
        ok(b"Linux x86_64\n"),
        check_facts_output(
            &builds,
            &InstalledFile::ALL,
            &[],
            Some("systemd"),
            &matching_sha,
        ),
    ]);

    let setup = Setup::ensure_ready(direct_link(&known), &known)
        .with_restore_enrollment(host_key_bytes);

    let Progress::Ready(_) = drive_setup(setup, &runner, &builds) else {
        panic!("expected Ready");
    };
    let purposes: Vec<Purpose> = runner.seen().iter().map(|i| i.purpose).collect();
    assert_eq!(purposes, [Purpose::Probe, Purpose::Check]);
}

#[test]
fn remote_restore_script_writes_when_missing_and_refuses_conflict_on_real_shell() {
    let home = tempfile::tempdir().unwrap();
    let known = home.path().join("known_hosts");
    let builds = FakeBuilds::sample();
    let host_key: Arc<[u8]> = Arc::from(&b"1111111111111111111111111111111111111111111111111111111111111111"[..]);
    let other_key = b"2222222222222222222222222222222222222222222222222222222222222222";

    // Connect template that runs `{cmd}` locally with HOME=home.path().
    let provider = Provider::parse(&format!(
        "[providers.local]\nconnect = \"HOME={} /bin/sh -c {{cmd}}\"\n",
        home.path().display()
    ))
    .unwrap()
    .pop()
    .unwrap();
    let link = provider
        .link(&name("box-1"), &BTreeMap::new())
        .unwrap()
        .unwrap();

    // Force `EnrollmentKeyState::Unknown` so the remote shell script itself enforces the check.
    let runner = LocalEnrollRunner {
        inner: SystemRunner::new(&()),
        builds: &builds,
    };

    // 1. Missing enrollment-key -> restored on disk.
    let setup = Setup::ensure_ready(link.clone(), &known)
        .with_restore_enrollment(Arc::clone(&host_key));
    let Progress::Ready(_) = drive_setup(setup, &runner, &builds) else {
        panic!("expected Ready when key is missing");
    };
    let key_path = home.path().join(".humata2/state/enrollment-key");
    assert_eq!(fs::read(&key_path).unwrap(), &host_key[..]);

    // 2. Replace with a conflicting key on disk -> refused without overwriting!
    fs::write(&key_path, other_key).unwrap();
    let setup = Setup::ensure_ready(link, &known).with_restore_enrollment(host_key);
    let Progress::Failed(failure) = drive_setup(setup, &runner, &builds) else {
        panic!("expected conflict failure");
    };
    assert!(matches!(
        failure.error,
        SetupError::EnrollmentKeyConflict { .. }
    ));
    assert_eq!(
        fs::read(&key_path).unwrap(),
        other_key,
        "conflicting key must never be overwritten"
    );
}

struct LocalEnrollRunner<'a> {
    inner: SystemRunner<'a>,
    builds: &'a FakeBuilds,
}

impl Runner for LocalEnrollRunner<'_> {
    fn run(&self, invocation: &Invocation) -> Result<Output, RunError> {
        match invocation.purpose {
            Purpose::Probe => Ok(ok(b"Linux x86_64\n")),
            Purpose::Check => {
                // Omit humata2.enrollment_key so Setup delegates the missing/conflict
                // check to the real shell script executed by `inner`.
                let mut text = String::from(
                    "humata2.os=Linux\nhumata2.arch=x86_64\nhumata2.user=dev\nhumata2.home=/home/dev\n\
                     humata2.git=yes\nhumata2.disk_free_kib=10485760\nhumata2.systemd=yes\n\
                     humata2.systemd_init=yes\nhumata2.linger=yes\nhumata2.crontab=yes\n\
                     humata2.setsid=yes\nhumata2.running=systemd\n",
                );
                for file in InstalledFile::ALL {
                    text.push_str(&format!(
                        "humata2.sha.{}={}\n",
                        file.file_name(),
                        self.builds.sha(file)
                    ));
                }
                Ok(ok(text.as_bytes()))
            }
            Purpose::Enroll => self.inner.run(invocation),
            other => panic!("unexpected purpose {other:?}"),
        }
    }
}
