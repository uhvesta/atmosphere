//! `humata host`, `humata provider` and `humata skills sync` against a real
//! `humatad2` and a fake machine (a fake `ssh` on the daemon's PATH: no
//! network, never the real ~/.ssh, never the real home folder). The real
//! `humata` binary runs as the user and as an agent, so these show that the
//! CLI and the daemon agree on what each command sends, what comes back and
//! which exit code a refusal gets. They check exit codes, JSON fields and
//! state, never the wording of a message.
//!
//! Lives in tests/pending/ until `seam_cli_hosts.py` moves it to
//! tests/hosts_live.rs (its own target, `hosts_live_test`: the glob that
//! gives every other test file a target excludes it). The path below is
//! for that final place.

#[path = "../../daemon/humatad/tests/common/mod.rs"]
mod common;

use std::cell::RefCell;
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use common::{Attach, Build, Daemon, WAIT, binary, folder, open, state_folder};
use humata2_fake_machine::{FakeMachine, HOST_KEY};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{ChannelRecord, SessionRecord, Store};
use serde_json::Value as Json;
use sha2::{Digest, Sha256};
use tempfile::TempDir;

/// How long a run waits for its operation. Each operation finishes in
/// seconds; the bound is generous because a test machine busy with a
/// build can starve the fake machine's scripts for a minute.
const OP_WAIT: &str = "300";

/// `humata`'s exit codes (CLI.md §7).
const OK: i32 = 0;
const USAGE: i32 = 2;
const DENIED: i32 = 4;
const PRECONDITION: i32 = 7;
const UNREACHABLE: i32 = 8;
const PENDING: i32 = 12;

const FILES: [&str; 5] = ["humatad2", "humata2_holder", "humata", "stack", "humata2_fusefs"];

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

/// One run of `humata`.
struct Run {
    code: i32,
    /// What it printed on stdout (null when that wasn't JSON, as for a usage
    /// error from the argument parser).
    json: Json,
    stdout: String,
    stderr: String,
}

impl Run {
    fn result(&self) -> &Json {
        &self.json["result"]
    }

    fn kind(&self) -> &str {
        self.json["error"]["kind"].as_str().unwrap_or_default()
    }
}

/// The admin daemon with the fake machine's `ssh` first on its PATH, its
/// skills read from `<dir>/home`, and a home folder of its own.
struct Host {
    child: RefCell<Child>,
    socket: PathBuf,
    dir: TempDir,
}

impl Host {
    fn start(machine: &FakeMachine) -> Self {
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
            .arg("--skills-home")
            .arg(dir.path().join("home"))
            .env("HOME", dir.path())
            .env("PATH", format!("{}:/usr/bin:/bin", machine.ssh_dir().display()))
            .spawn()
            .unwrap();
        let deadline = Instant::now() + WAIT;
        while UnixStream::connect(&socket).is_err() {
            assert!(Instant::now() < deadline, "humatad2 never started listening");
            std::thread::sleep(Duration::from_millis(2));
        }
        Self { child: RefCell::new(child), socket, dir }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn skills_folder(&self, agent: &str) -> PathBuf {
        self.dir.path().join("home").join(format!(".{agent}/skills"))
    }

    /// `humata --json <args>` as the user, with nothing on standard input.
    fn run(&self, args: &[&str]) -> Run {
        self.run_with(args, b"")
    }

    /// As [`Self::run`], with `stdin` on standard input.
    fn run_with(&self, args: &[&str], stdin: &[u8]) -> Run {
        let mut child = Command::new(binary("HUMATA"))
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", self.dir.path())
            .env("HUMATA_SOCKET", &self.socket)
            .arg("--json")
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(stdin).unwrap();
        let output = child.wait_with_output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        if stdout.contains("daemon_not_running") {
            panic!("{}", self.gone(args, &stdout));
        }
        Run {
            code: output.status.code().unwrap_or(-1),
            json: serde_json::from_str(&stdout).unwrap_or(Json::Null),
            stdout,
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }

    /// Why `humata <args>` couldn't reach the daemon: whether humatad2 had
    /// exited, how, and the end of its log, so a lost daemon says why.
    fn gone(&self, args: &[&str], stdout: &str) -> String {
        let status = match self.child.borrow_mut().try_wait() {
            Ok(Some(status)) => format!("humatad2 had exited: {status}"),
            Ok(None) => "humatad2 was still running".to_owned(),
            Err(error) => format!("humatad2's state is unknown: {error}"),
        };
        let log = std::fs::read_to_string(self.dir.path().join("state/logs/humatad2.log")).unwrap_or_default();
        let tail: Vec<&str> = log.lines().rev().take(40).collect();
        let tail: Vec<&str> = tail.into_iter().rev().collect();
        format!(
            "humata {} couldn't reach the daemon ({status}; socket there: {}):\n{stdout}\nthe end of its log:\n{}",
            args.join(" "),
            self.socket.exists(),
            tail.join("\n")
        )
    }

    /// The machines `humata host ls` lists.
    fn machines(&self) -> Vec<Json> {
        let run = self.run(&["host", "ls"]);
        assert_eq!(run.code, OK, "{}", run.stdout);
        run.result()["machines"].as_array().cloned().unwrap_or_default()
    }

    fn machine(&self, name: &str) -> Option<Json> {
        self.machines().into_iter().find(|machine| machine["name"] == name)
    }

    /// box-1 once it is online and its peer link reports a daemon version.
    fn linked(&self) -> Json {
        let deadline = Instant::now() + WAIT;
        loop {
            if let Some(machine) = self.machine("box-1")
                && machine["reachability"] == "online"
                && machine["daemon_version"].as_str().is_some_and(|version| !version.is_empty())
            {
                return machine;
            }
            assert!(Instant::now() < deadline, "the peer link to box-1 never came up");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// Adds box-1 over the fake ssh with `humata host add`, the way a person
    /// does: once to read the host's key, once to trust it.
    fn add_box(&self) {
        let first = self.run(&["host", "add", "me@box-1.internal", "--name", "box-1", "--wait", OP_WAIT]);
        let fingerprint = presented_fingerprint(&first);
        let added = self.run(&[
            "host",
            "add",
            "me@box-1.internal",
            "--name",
            "box-1",
            "--trust-host-key",
            &fingerprint,
            "--wait",
            "60",
        ]);
        assert_eq!(added.code, OK, "{}", added.stdout);
        self.linked();
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        let child = self.child.get_mut();
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// The fingerprint a first, unconfirmed `host add` reports.
fn presented_fingerprint(first: &Run) -> String {
    assert_eq!(first.code, PRECONDITION, "{}", first.stdout);
    assert_eq!(first.kind(), "host_key_unknown");
    let fingerprint = first.json["error"]["details"]["presented_fingerprint"]
        .as_str()
        .unwrap_or_default();
    assert!(!fingerprint.is_empty(), "{}", first.stdout);
    fingerprint.to_owned()
}

fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn a_machine_is_added_after_its_key_is_trusted_shown_reconnected_and_removed_by_plan() {
    let machine = FakeMachine::new();
    let host = Host::start(&machine);

    // Only this Mac at first.
    assert!(host.machines().iter().all(|machine| machine["is_this_mac"] == true));
    assert!(host.machine("box-1").is_none());

    // First contact stops at the host's key, naming it, and installs nothing.
    let first = host.run(&["host", "add", "me@box-1.internal", "--name", "box-1", "--wait", OP_WAIT]);
    let fingerprint = presented_fingerprint(&first);
    assert!(machine.installed("humatad2").is_none());
    assert!(host.machine("box-1").is_none());

    // A fingerprint that isn't the host's doesn't add it.
    let wrong = host.run(&[
        "host",
        "add",
        "me@box-1.internal",
        "--name",
        "box-1",
        "--trust-host-key",
        "SHA256:not-the-hosts-key",
        "--wait",
        "60",
    ]);
    assert_ne!(wrong.code, OK, "{}", wrong.stdout);
    assert!(host.machine("box-1").is_none());

    // Trusting the fingerprint adds it, and the run waits for the operation.
    let added = host.run(&[
        "host",
        "add",
        "me@box-1.internal",
        "--name",
        "box-1",
        "--trust-host-key",
        &fingerprint,
        "--wait",
        "60",
    ]);
    assert_eq!(added.code, OK, "{}", added.stdout);
    assert_eq!(added.result()["state"], "succeeded", "{}", added.stdout);
    assert!(machine.installed("humatad2").is_some());
    let listed = host.linked();
    assert_eq!(listed["os"], "linux");
    assert_ne!(listed["is_this_mac"], true);

    let shown = host.run(&["host", "show", "box-1"]);
    assert_eq!(shown.code, OK, "{}", shown.stdout);
    assert_eq!(shown.result()["machine"]["name"], "box-1");

    let reconnected = host.run(&["host", "reconnect", "box-1"]);
    assert_eq!(reconnected.code, OK, "{}", reconnected.stdout);
    assert_eq!(reconnected.result()["name"], "box-1");

    // A removal without --dry-run, --plan or --yes asks for one of them and
    // changes nothing.
    let unconfirmed = host.run(&["host", "remove", "box-1"]);
    assert_eq!(unconfirmed.code, USAGE, "{}", unconfirmed.stdout);
    assert!(host.machine("box-1").is_some());

    // --dry-run prints the plan and its token and changes nothing.
    let planned = host.run(&["host", "remove", "box-1", "--dry-run"]);
    assert_eq!(planned.code, OK, "{}", planned.stdout);
    assert_eq!(planned.json["outcome"], "planned");
    let token = planned.result()["token"].as_str().unwrap_or_default().to_owned();
    assert!(!token.is_empty(), "{}", planned.stdout);
    assert!(host.machine("box-1").is_some());

    // The token does what the plan said, and the run waits for it.
    let removed = host.run(&["host", "remove", "box-1", "--plan", &token, "--wait", OP_WAIT]);
    assert_eq!(removed.code, OK, "{}", removed.stdout);
    assert_eq!(removed.result()["state"], "succeeded", "{}", removed.stdout);
    assert!(host.machine("box-1").is_none());
    assert!(!machine.unit_installed());
}

#[test]
fn an_unconfirmed_removal_goes_ahead_with_yes() {
    let machine = FakeMachine::new();
    let host = Host::start(&machine);
    host.add_box();
    let removed = host.run(&["host", "remove", "box-1", "--yes", "--wait", OP_WAIT]);
    assert_eq!(removed.code, OK, "{}", removed.stdout);
    assert!(host.machine("box-1").is_none());
}

#[test]
fn a_host_variable_is_set_listed_by_name_and_removed_and_its_value_is_never_on_a_command_line_or_printed() {
    let machine = FakeMachine::new();
    let host = Host::start(&machine);
    host.add_box();

    // The value goes to the box over its link, to the box's own daemon
    // (HO-13). This rig's box link has its own --state and no --socket, so
    // it relays nothing (D146): the box can't take the value, and says so at
    // once (HO-5). humatad's host_variables_remote test sets one on a box
    // that has a daemon.
    let secret = "s3cr3t-value-0f-the-variable";
    let set = host.run_with(&["host", "var", "set", "box-1", "API_TOKEN"], format!("{secret}\n").as_bytes());
    assert_eq!(set.code, UNREACHABLE, "{}", set.stdout);
    assert!(!set.stdout.contains(secret));
    assert!(!set.stderr.contains(secret));

    // The value has no place on the command line: it is refused as usage.
    let in_argv = host.run(&["host", "var", "set", "box-1", "API_TOKEN", secret]);
    assert_eq!(in_argv.code, USAGE);
    assert!(!in_argv.stdout.contains(secret));
    assert!(!in_argv.stderr.contains(secret));

    // Nothing was kept on the host in the box's place: its names come from
    // the box, which can't be reached, and nothing prints the value.
    let listed = host.run(&["host", "vars", "box-1"]);
    assert_eq!(listed.code, UNREACHABLE, "{}", listed.stdout);
    assert!(!listed.stdout.contains(secret));
    let removed = host.run(&["host", "var", "rm", "box-1", "API_TOKEN"]);
    assert_eq!(removed.code, UNREACHABLE, "{}", removed.stdout);
}

/// Two providers, as the daemon's own provider tests have them: `fake` makes
/// a marker file, says its identity from a file, and defines status, start
/// and delete but no stop.
fn providers_toml(host: &Host) -> PathBuf {
    let dir = host.dir.path().display();
    write(
        &host.path("identity.json"),
        format!(
            r#"{{"name":"box-1","ssh":{{"host":"box-1.internal","user":"me","port":22,"host_key":"{HOST_KEY}"}},"os":"linux","arch":"x86_64"}}"#
        )
        .as_bytes(),
    );
    let file = host.path("providers.toml");
    write(
        &file,
        format!(
            "[providers.fake]\nlabel = \"Fake\"\nparams = [{{ name = \"zone\", prompt = \"Zone\" }}]\n\
             create = 'touch {dir}/created'\nidentity = 'cat {dir}/identity.json'\nstatus = 'echo stopped'\nstart = 'true'\n\
             delete = 'rm -f {dir}/created'\n"
        )
        .as_bytes(),
    );
    file
}

#[test]
fn a_provider_makes_a_machine_runs_its_actions_and_deletes_what_it_made() {
    let machine = FakeMachine::new();
    let host = Host::start(&machine);
    let file = providers_toml(&host);

    let saved = host.run(&["provider", "config", "edit", "--file", file.to_str().unwrap()]);
    assert_eq!(saved.code, OK, "{}", saved.stdout);
    assert_eq!(saved.result()["problems"].as_array().map(Vec::len), Some(0), "{}", saved.stdout);
    let shown = host.run(&["provider", "config", "show"]);
    assert_eq!(shown.result()["toml"], std::fs::read_to_string(&file).unwrap());

    // The configured provider and the built-in example are listed.
    let listed = host.run(&["provider", "ls"]);
    assert_eq!(listed.code, OK, "{}", listed.stdout);
    let providers = listed.result()["providers"].as_array().cloned().unwrap_or_default();
    let fake = providers.iter().find(|provider| provider["id"] == "fake").expect("fake is listed");
    assert_eq!(fake["has_create"], true);
    assert_eq!(fake["built_in"], false);
    assert!(providers.iter().any(|provider| provider["built_in"] == true));

    // Parameters the provider doesn't declare, or a missing one, are refused
    // before anything runs.
    let undeclared = host.run(&["provider", "create", "fake", "box-1", "--param", "zone=a", "--param", "colour=red"]);
    assert_eq!(undeclared.code, USAGE, "{}", undeclared.stdout);
    let missing = host.run(&["provider", "create", "fake", "box-1"]);
    assert_eq!(missing.code, USAGE, "{}", missing.stdout);
    assert!(!host.path("created").exists());

    let created = host.run(&["provider", "create", "fake", "box-1", "--param", "zone=a", "--wait", OP_WAIT]);
    assert_eq!(created.code, OK, "{}", created.stdout);
    assert_eq!(
        created.result()["state"],
        "succeeded",
        "{}\nssh log:\n{}",
        created.stdout,
        machine.ssh_log()
    );
    assert!(host.path("created").exists());
    let made = host.machine("box-1").expect("the machine was made");
    assert_eq!(made["provider"], "fake");
    host.linked();

    // Status says stopped, so the machine shows as stopped.
    let status = host.run(&["provider", "status", "box-1", "--wait", OP_WAIT]);
    assert_eq!(status.code, OK, "{}", status.stdout);
    assert_eq!(host.machine("box-1").unwrap()["reachability"], "stopped");

    // An action the provider doesn't define is refused.
    let stop = host.run(&["provider", "stop", "box-1", "--wait", OP_WAIT]);
    assert_eq!(stop.code, USAGE, "{}", stop.stdout);

    // Delete removes the machine by its plan and runs the provider's delete.
    let planned = host.run(&["provider", "delete", "box-1", "--dry-run"]);
    assert_eq!(planned.code, OK, "{}", planned.stdout);
    assert_eq!(planned.json["outcome"], "planned");
    assert!(host.machine("box-1").is_some());
    let deleted = host.run(&["provider", "delete", "box-1", "--yes", "--wait", OP_WAIT]);
    assert_eq!(deleted.code, OK, "{}", deleted.stdout);
    assert!(host.machine("box-1").is_none());
    assert!(!host.path("created").exists());
}

#[test]
fn a_first_skills_sync_prints_its_plan_applies_with_its_token_and_the_next_one_has_nothing_to_do() {
    let machine = FakeMachine::new();
    let host = Host::start(&machine);
    write(&host.skills_folder("claude").join("alpha/SKILL.md"), b"alpha v1");
    host.add_box();

    let planned = host.run(&["skills", "sync", "--dry-run", "box-1"]);
    assert_eq!(planned.code, OK, "{}", planned.stdout);
    assert_eq!(planned.json["outcome"], "planned");
    let plan = &planned.result()["plans"][0];
    assert_eq!(plan["first_sync"], true);
    assert_eq!(plan["changes"][0]["skill"], "alpha");
    assert_eq!(plan["changes"][0]["kind"], "add");
    let token = planned.result()["token"].as_str().unwrap_or_default().to_owned();
    assert!(!token.is_empty(), "{}", planned.stdout);
    let synced = machine.home().join("Humata/.skills/claude/alpha/SKILL.md");
    assert!(!synced.exists());

    // Without --plan, --yes or a terminal to ask in, nothing is synced.
    let unconfirmed = host.run(&["skills", "sync", "box-1"]);
    assert_eq!(unconfirmed.code, USAGE, "{}", unconfirmed.stdout);
    assert!(!synced.exists());

    // The plan's token applies it, and the run waits for the operation.
    let applied = host.run(&["skills", "sync", "box-1", "--plan", &token, "--wait", OP_WAIT]);
    assert_eq!(applied.code, OK, "{}", applied.stdout);
    assert_eq!(applied.result()["state"], "succeeded", "{}", applied.stdout);
    let deadline = Instant::now() + WAIT;
    while std::fs::read(&synced).ok().as_deref() != Some(b"alpha v1".as_slice()) {
        assert!(Instant::now() < deadline, "alpha never reached the machine");
        std::thread::sleep(Duration::from_millis(2));
    }

    let next = host.run(&["skills", "sync", "--dry-run", "box-1"]);
    assert_eq!(next.code, OK, "{}", next.stdout);
    let plan = &next.result()["plans"][0];
    assert_eq!(plan["first_sync"], false);
    assert_eq!(plan["changes"].as_array().map(Vec::len), Some(0), "{}", next.stdout);
}

/// A daemon with one session, and that session's agent token, read from
/// its own shell tab; the attach keeps the tab open.
async fn agent_daemon() -> (Daemon, Attach, String) {
    let dir = folder();
    let store = Store::open(state_folder(dir.path()).join("store")).unwrap();
    store
        .put_channel(&ChannelRecord {
            id: "ch_hosts".to_owned(),
            name: "Hosts".to_owned(),
            folder: "hosts".to_owned(),
            branch_prefix: None,
            default_agent: "zsh".to_owned(),
            repos: Vec::new(),
            machines: Vec::new(),
            revision: 1,
            created_unix_ms: 0,
        })
        .unwrap();
    store
        .put_session(&SessionRecord {
            id: "se_hosts".to_owned(),
            channel: "ch_hosts".to_owned(),
            machine: String::new(),
            title: "hosts".to_owned(),
            agent: "zsh".to_owned(),
            folder: dir.path().to_owned(),
            created_unix_ms: 0,
        })
        .unwrap();
    drop(store);
    let daemon = Daemon::start_with(Build::Admin, dir, &[]).await;
    let tab = daemon
        .terminals()
        .await
        .open_shell_tab(pb::OpenShellTabRequest {
            idempotency_key: None,
            session: Some(pb::SessionId {
                value: "se_hosts".to_owned(),
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
        tokio::time::sleep(Duration::from_millis(2)).await;
    };

    (daemon, attach, token)
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_cannot_add_or_remove_a_machine_and_sees_none() {
    let (daemon, _attach, token) = agent_daemon().await;

    // Adding asks the user first (it is held, or refused); either way it
    // adds nothing.
    let (code, printed) = common::humata_as_agent(
        &daemon,
        &token,
        &["host", "add", "me@box-1.internal", "--name", "box-1"],
    )
    .await;
    assert!(code == PENDING || code == DENIED, "{code}: {printed}");
    assert_eq!(printed["ok"], false);

    // Removing is never allowed by default.
    let (code, printed) = common::humata_as_agent(&daemon, &token, &["host", "remove", "ma_box1", "--dry-run"]).await;
    assert_eq!(code, DENIED, "{printed}");
    assert_eq!(printed["error"]["kind"], "denied");

    // An agent sees only this machine (D672), and so does the user:
    // nothing was added.
    let (code, printed) = common::humata_as_agent(&daemon, &token, &["host", "ls"]).await;
    assert_eq!(code, OK, "{printed}");
    let listed = printed["result"]["machines"].as_array().cloned().unwrap_or_default();
    assert_eq!(listed.len(), 1, "{printed}");
    assert_eq!(listed[0]["is_this_mac"], true, "{printed}");
    let (code, printed) = common::humata(&daemon, &["host", "ls"]);
    assert_eq!(code, OK, "{printed}");
    assert!(
        printed["result"]["machines"]
            .as_array()
            .is_some_and(|machines| machines.iter().all(|machine| machine["name"] != "box-1")),
        "{printed}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_reads_the_providers_and_its_import_waits_for_the_user_in_the_inbox() {
    let (daemon, _attach, token) = agent_daemon().await;

    // Listing and showing are read-only, so an agent gets them.
    let (code, printed) = common::humata_as_agent(&daemon, &token, &["provider", "ls"]).await;
    assert_eq!(code, OK, "{printed}");
    let providers = printed["result"]["providers"].as_array().cloned().unwrap_or_default();
    assert!(providers.iter().any(|provider| provider["built_in"] == true), "{printed}");
    let (code, printed) = common::humata_as_agent(&daemon, &token, &["provider", "config", "show"]).await;
    assert_eq!(code, OK, "{printed}");

    // Importing files an Inbox request the user answers; nothing is added yet.
    let file = daemon.dir.path().join("box.toml");
    write(&file, b"[providers.box]\nlabel = \"Box\"\ncreate = 'true'\nconnect = 'sh -c {cmd}'\n");
    let (code, printed) =
        common::humata_as_agent(&daemon, &token, &[
            "provider",
            "import",
            file.to_str().unwrap(),
            "--reason",
            "the task needs a throwaway box",
        ])
        .await;
    assert_eq!(code, PENDING, "{printed}");
    let (code, printed) = common::humata(&daemon, &["provider", "ls"]);
    assert_eq!(code, OK, "{printed}");
    assert!(
        printed["result"]["providers"]
            .as_array()
            .is_some_and(|providers| providers.iter().all(|provider| provider["id"] != "box")),
        "{printed}"
    );
}
