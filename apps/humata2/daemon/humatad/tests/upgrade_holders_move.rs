//! LIVEHOLDER T6 (D801): "Migrate the holders" moves each running terminal
//! onto the current holder build in place. The holder process keeps its
//! pid, the program in it never restarts and never sees a hangup, and the
//! item ends Approved once nothing is left to move. A move waits for no
//! idle moment. Only a holder too old to move (spawned before LIVEHOLDER,
//! so its record has no `moves` line) is still restarted at its next idle
//! moment, as `tests/upgrade_holders.rs` covers.
//!
//! Release `b` ships `HUMATA2_HOLDER_FAULTS` as its holder: a build that
//! behaves as the plain holder with no fault switched on, and whose bytes,
//! and so whose sha256, differ. Holders spawned at release `a` are the
//! plain `HUMATA2_HOLDER`, which can move.
//!
//! Every test ends only the holders its own daemon started, in its own
//! temporary folder.

use humatad_test_common as common;
use humatad_test_common::upgrade;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::{
    AGENT_NAME, Attach, Build, Daemon, World, agent_tab, binary, create_channel, folder, get_session,
    holder_answers, is_running, machine_id, open, start_session, stored_tabs, wait_running, wait_status,
};
use humata2_grpc::humata::v1 as pb;
use humata2_store::TabKind;
use hyper_util::rt::TokioIo;
use tokio::net::UnixStream;
use tonic::transport::{Channel, Endpoint, Uri};

/// A safety net for waits on a loaded machine, never a time anything is
/// expected to take.
const LONG: Duration = Duration::from_secs(90);

const RELEASE_A: [(&str, &str); 4] = [
    ("humatad2", "HUMATAD2"),
    ("humata2_holder", "HUMATA2_HOLDER"),
    ("humata", "HUMATA"),
    ("stack", "STACK"),
];

const RELEASE_B: [(&str, &str); 4] = [
    ("humatad2", "HUMATAD2"),
    ("humata2_holder", "HUMATA2_HOLDER_FAULTS"),
    ("humata", "HUMATA"),
    ("stack", "STACK"),
];

fn manifest_a() -> Vec<u8> {
    upgrade::manifest(&upgrade::admin_sha(), "a")
}

fn manifest_b() -> Vec<u8> {
    upgrade::manifest_with_holder(
        &upgrade::admin_sha(),
        &upgrade::sha256_of(&binary("HUMATA2_HOLDER_FAULTS")),
        "b",
    )
}

fn put_file(variable: &str, to: &Path) {
    std::fs::copy(binary(variable), to)
        .unwrap_or_else(|error| panic!("couldn't copy {variable} to {}: {error}", to.display()));
    std::fs::set_permissions(to, std::fs::Permissions::from_mode(0o755)).expect("the file is made runnable");
}

/// The test agent: every line typed to it is answered, and `run:<command>`
/// runs the command, as Claude Code runs a hook's. It notes a hangup in
/// `<conv>/hup-<conversation>` and says how it was started.
fn agent_script(world: &World) -> String {
    format!(
        r#"CONV_DIR='{}'
mode=$1; id=$2
case "$mode" in
  --session-id) f="$CONV_DIR/$id.jsonl"; : > "$f" ;;
  --resume) f="$CONV_DIR/$id.jsonl" ;;
esac
trap 'echo hup >> "$CONV_DIR/hup-$id"' HUP
# Reads keys itself, as agent CLIs do, so typing never waits out the
# holder's keys limit (see common/mod.rs).
stty -icanon 2>/dev/null
echo "agent up $mode"
while IFS= read -r line; do
  case "$line" in
    run:*) eval "${{line#run:}}" ;;
    *) echo "got: $line" ;;
  esac
  printf '%s\n' "$line" >> "$f"
done
"#,
        world.conv.display(),
    )
}

fn world_with_agent(dir: &Path) -> World {
    let world = World::new(dir);
    let script = dir.join("turn-agent.sh");
    std::fs::write(&script, agent_script(&world)).expect("the agent script is written");
    let agents = serde_json::json!([{
        "name": AGENT_NAME,
        "program": "/bin/sh",
        "start_args": [script, "--session-id", "{conversation}"],
        "resume_args": [script, "--resume", "{conversation}"],
        "conversation_file": format!("{}/{{conversation}}.jsonl", world.conv.display()),
        "turn_end": "hooks",
    }]);
    std::fs::write(&world.agents, agents.to_string()).expect("the agents file is written");
    world
}

struct Mac {
    daemon: Daemon,
    world: World,
    bin: PathBuf,
    channel: pb::Channel,
    machine: pb::MachineId,
}

impl Mac {
    async fn start() -> Self {
        let dir = folder();
        let world = world_with_agent(dir.path());
        world.remote("acme", "api");
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).expect("the bin folder is made");
        for (name, variable) in RELEASE_A {
            put_file(variable, &bin.join(name));
        }
        let from = upgrade::install(&bin, &manifest_a());
        let mut args = world.args();
        args.extend(["--humata".to_owned(), bin.join("humata").display().to_string()]);
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let daemon = Daemon::start_with_holder(Build::Admin, dir, &refs, &bin.join("humata2_holder")).await;
        assert_eq!(upgrade::build_of(&daemon).await.bundle, from, "the daemon runs release a");
        let (channel, setup) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
        assert_eq!(setup.state, pb::OperationState::Succeeded as i32, "{setup:?}");
        let machine = machine_id(&daemon).await;
        Self {
            daemon,
            world,
            bin,
            channel,
            machine,
        }
    }

    async fn agent_session(&self, title: &str) -> pb::SessionId {
        let (session, operation) = start_session(&self.daemon, &self.channel, title).await;
        assert_eq!(operation.state, pb::OperationState::Succeeded as i32, "{:?}", operation.error);
        wait_running(&self.daemon, &session).await;
        self.agent(&session).await.wait_for("agent up").await;
        session
    }

    async fn agent(&self, session: &pb::SessionId) -> Attach {
        let tab = agent_tab(&self.daemon, session).await;
        Attach::start(&self.daemon, open(&tab, 0)).await
    }

    fn record(&self, name: &str) -> PathBuf {
        self.daemon.state.join("holder_builds").join(name)
    }

    /// Makes the record `name` stand for a holder spawned before LIVEHOLDER:
    /// the sha256 stays, the `moves` line goes.
    async fn too_old_to_move(&self, name: &str) {
        let path = self.record(name);
        common::wait_for_path(&path).await;
        let text = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("couldn't read {}: {error}", path.display()));
        let sha = text.lines().next().unwrap_or_default().trim().to_owned();
        std::fs::write(&path, sha).unwrap_or_else(|error| panic!("couldn't write {}: {error}", path.display()));
    }

    async fn upgrade_to_b(&self) -> String {
        for (name, variable) in RELEASE_B {
            put_file(variable, &self.bin.join(format!(".{name}.next")));
        }
        let bundle = manifest_b();
        let staged = self.bin.join(".bundle.json.next");
        std::fs::write(&staged, &bundle).expect("the staged bundle is written");
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o644)).expect("the bundle is readable");
        let to = upgrade::bundle_id(&bundle);
        // Auto-migrate detects `.bundle.json.next`, swaps release b in, and
        // restarts the daemon in place.
        wait_at_release(&self.daemon, &to).await;
        to
    }

    fn hook_command(&self, event: &str) -> String {
        let file = self.daemon.state.join("claude-hooks.json");
        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(file).expect("the hooks file reads"))
                .expect("the hooks file is JSON");
        json["hooks"][event][0]["hooks"][0]["command"]
            .as_str()
            .unwrap_or_else(|| panic!("no hook for {event}"))
            .to_owned()
    }

    async fn turn(&self, session: &pb::SessionId, event: &str, status: pb::SessionStatus) {
        let mut attach = self.agent(session).await;
        attach.type_text(&format!("run:{}\r", self.hook_command(event))).await;
        wait_status(&self.daemon, session, status).await;
    }

    /// The pid of the holder behind `session`'s agent tab, as stored.
    fn agent_holder(&self, session: &pb::SessionId) -> Option<u32> {
        stored_tabs(&self.daemon.state)
            .into_iter()
            .find(|tab| tab.kind == TabKind::Agent && tab.session.as_deref() == Some(session.value.as_str()))
            .map(|tab| tab.holder_pid)
    }

    fn agent_holder_now(&self, session: &pb::SessionId) -> u32 {
        self.agent_holder(session)
            .unwrap_or_else(|| panic!("session {} has an agent holder", session.value))
    }

    /// Waits until `name`'s holder build record matches release `b`'s holder
    /// sha256 and no pending Holders item remains for this machine.
    async fn wait_moved(&self, names: &[String]) {
        let wanted = upgrade::sha256_of(&binary("HUMATA2_HOLDER_FAULTS"));
        let deadline = Instant::now() + LONG;
        loop {
            let all_on_b = names.iter().all(|name| {
                std::fs::read_to_string(self.record(name))
                    .ok()
                    .and_then(|text| text.lines().next().map(|l| l.trim().to_owned()))
                    .is_some_and(|sha| sha.eq_ignore_ascii_case(&wanted))
            });
            if all_on_b {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "holders {names:?} never recorded new build {wanted}"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        upgrade::inbox_until(&self.daemon, "Holders item settled", |items| {
            (!items
                .iter()
                .any(|item| upgrade::pending_migrate(item, pb::MigrateThing::Holders, &self.machine)))
            .then_some(())
        })
        .await;
    }

    /// Waits until `session`'s agent runs on another holder than `old`.
    async fn restarted(&self, session: &pb::SessionId, old: u32) {
        let deadline = Instant::now() + LONG;
        loop {
            let now = get_session(&self.daemon, session).await;
            let holder = self.agent_holder(session);
            if holder.is_some_and(|pid| pid != old) && is_running(&now) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "session {} was never restarted onto a new holder; its holder is {holder:?} (was {old})",
                session.value
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

async fn try_info(daemon: &Daemon) -> Option<pb::ServerInfo> {
    let socket = daemon.socket.clone();
    let channel: Channel = Endpoint::try_from("http://[::]:50051")
        .ok()?
        .connect_with_connector(tower::service_fn(move |_: Uri| {
            let socket = socket.clone();
            async move { Ok::<_, std::io::Error>(TokioIo::new(UnixStream::connect(socket).await?)) }
        }))
        .await
        .ok()?;
    pb::daemon_service_client::DaemonServiceClient::new(channel)
        .get_server_info(pb::GetServerInfoRequest {})
        .await
        .ok()
        .map(tonic::Response::into_inner)
}

async fn wait_at_release(daemon: &Daemon, bundle: &str) {
    let deadline = Instant::now() + LONG;
    loop {
        if try_info(daemon)
            .await
            .and_then(|info| info.build)
            .is_some_and(|build| build.bundle == bundle)
        {
            return;
        }
        assert!(Instant::now() < deadline, "the daemon never answered as release {bundle}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Opens a machine shell that notes a hangup in `hup` and writes its own
/// pid to `pid`, and waits until it has. Returns the tab.
async fn machine_shell(daemon: &Daemon, hup: &Path, pid: &Path) -> pb::TabId {
    let tab = daemon
        .terminals()
        .await
        .open_machine_shell(pb::OpenMachineShellRequest::default())
        .await
        .expect("a machine shell opens")
        .into_inner()
        .id
        .expect("the machine shell has a tab");
    let mut attach = Attach::start(daemon, open(&tab, 0)).await;
    attach
        .type_text(&format!(
            "trap 'echo hup >> {}' HUP; echo $$ > {}; printf 'set-%s\\n' up\r",
            hup.display(),
            pid.display()
        ))
        .await;
    attach.wait_for("set-up").await;
    tab
}

/// The shell in `tab` answers a line typed to it now.
async fn shell_answers(daemon: &Daemon, tab: &pb::TabId, mark: &str) {
    let mut attach = Attach::start(daemon, open(tab, 0)).await;
    attach.type_text(&format!("printf 'still-%s\\n' {mark}\r")).await;
    attach.wait_for(&format!("still-{mark}")).await;
}

fn shell_holder(daemon: &Daemon, tab: &pb::TabId) -> u32 {
    stored_tabs(&daemon.state)
        .into_iter()
        .find(|stored| stored.id == tab.value)
        .map(|stored| stored.holder_pid)
        .unwrap_or_else(|| panic!("tab {} has a holder", tab.value))
}

fn read_pid(path: &Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("couldn't read {}: {error}", path.display()))
        .trim()
        .to_owned()
}

#[tokio::test(flavor = "multi_thread")]
async fn migrate_moves_a_working_agent_and_a_shell_onto_the_current_holder_with_their_processes_unchanged() {
    let mac = Mac::start().await;
    let login = mac.agent_session("Fix login").await;
    let conversation = get_session(&mac.daemon, &login).await.conversation_id;
    let agent_pid = mac.world.conv.join("agent.pid");
    let mut first = mac.agent(&login).await;
    first
        .type_text(&format!("run:echo $$ > {}; printf 'pid-%s\\n' saved\r", agent_pid.display()))
        .await;
    first.wait_for("pid-saved").await;
    let (shell_hup, shell_pid) = (mac.world.conv.join("shell.hup"), mac.world.conv.join("shell.pid"));
    let shell = machine_shell(&mac.daemon, &shell_hup, &shell_pid).await;
    let (agent_before, shell_before) = (read_pid(&agent_pid), read_pid(&shell_pid));
    let agent_holder = mac.agent_holder_now(&login);
    let shell_holder_before = shell_holder(&mac.daemon, &shell);
    // A working agent: auto-migrate moves both holders in place with no idle
    // moment and no manual click required.
    mac.turn(&login, "UserPromptSubmit", pb::SessionStatus::Working).await;
    mac.upgrade_to_b().await;

    mac.wait_moved(&[format!("{}.sha", login.value), format!("{}.shell.sha", shell.value)])
        .await;
    assert_eq!(mac.agent_holder_now(&login), agent_holder, "the agent's holder process is the same");
    assert_eq!(shell_holder(&mac.daemon, &shell), shell_holder_before, "the shell's holder process is the same");
    for tab in stored_tabs(&mac.daemon.state) {
        assert!(holder_answers(&tab), "holder {} still answers", tab.holder_pid);
    }
    // The programs never restarted and never heard a hangup.
    assert!(is_running(&get_session(&mac.daemon, &login).await), "the session still runs");
    let mut agent = mac.agent(&login).await;
    agent.type_text("after-move\r").await;
    agent.wait_for("got: after-move").await;
    assert!(!agent.shows("agent up --resume"), "the agent was never resumed");
    agent
        .type_text(&format!("run:echo $$ > {}; printf 'pid-%s\\n' again\r", agent_pid.display()))
        .await;
    agent.wait_for("pid-again").await;
    assert_eq!(read_pid(&agent_pid), agent_before, "the same agent process");
    assert!(!mac.world.conv.join(format!("hup-{conversation}")).exists(), "the agent heard no hangup");
    shell_answers(&mac.daemon, &shell, "after-move").await;
    let mut attach = Attach::start(&mac.daemon, open(&shell, 0)).await;
    attach
        .type_text(&format!("echo $$ > {}; printf 'pid-%s\\n' again\r", shell_pid.display()))
        .await;
    attach.wait_for("pid-again").await;
    assert_eq!(read_pid(&shell_pid), shell_before, "the same shell process");
    assert!(!shell_hup.exists(), "the shell heard no hangup");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_holder_too_old_to_move_is_still_restarted_when_idle_while_one_that_can_move_keeps_its_process() {
    let mac = Mac::start().await;
    let login = mac.agent_session("Fix login").await;
    let billing = mac.agent_session("Fix billing").await;
    // Billing's holder stands for one from before LIVEHOLDER.
    mac.too_old_to_move(&format!("{}.sha", billing.value)).await;
    let (login_holder, billing_holder) = (mac.agent_holder_now(&login), mac.agent_holder_now(&billing));
    mac.turn(&billing, "Stop", pb::SessionStatus::Idle).await;
    mac.upgrade_to_b().await;

    // Auto-migrate moves `login` in place and restarts `billing` (too old to
    // move) once idle, without requiring a manual click.
    mac.restarted(&billing, billing_holder).await;
    mac.wait_moved(&[format!("{}.sha", login.value), format!("{}.sha", billing.value)])
        .await;
    mac.agent(&billing).await.wait_for("agent up --resume").await;
    assert_eq!(mac.agent_holder_now(&login), login_holder, "the one that moved kept its holder process");
    let mut agent = mac.agent(&login).await;
    agent.type_text("after-move\r").await;
    agent.wait_for("got: after-move").await;
    assert!(!agent.shows("agent up --resume"), "the moved agent was never resumed");
}

#[tokio::test(flavor = "multi_thread")]
async fn shells_on_holders_that_can_move_hold_the_item_open_until_they_moved() {
    let mac = Mac::start().await;
    let (hup, pid) = (mac.world.conv.join("shell.hup"), mac.world.conv.join("shell.pid"));
    let shell = machine_shell(&mac.daemon, &hup, &pid).await;
    let holder = shell_holder(&mac.daemon, &shell);
    mac.upgrade_to_b().await;

    // Only a shell is on an old holder, and it can move: auto-migrate moves it
    // in place and settles the Holders item.
    mac.wait_moved(&[format!("{}.shell.sha", shell.value)]).await;
    assert_eq!(shell_holder(&mac.daemon, &shell), holder, "the same holder process");
    shell_answers(&mac.daemon, &shell, "moved").await;
    assert!(!hup.exists(), "the shell heard no hangup");
}
