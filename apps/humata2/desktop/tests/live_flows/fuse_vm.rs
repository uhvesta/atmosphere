//! FUSEFSWIRE T13: the FUSE mount on a REAL remote machine, a Lima VM, as
//! `humatad2 --role machine` runs it (installed by the host's Add machine
//! flow, a systemd user service of a throwaway user). It builds on the rig
//! of `real_vm.rs`: this Mac's admin `humatad2`, real ssh, an ssh wrapper
//! that holds the link down.
//!
//! GATES: each test returns at once, printing why, unless its VM is given.
//! - Cases 1 to 5 run on the VM `h2-fuse-r1`: `FUSE_VM_SSH_CONFIG` names its
//!   Lima `ssh.config` (`FUSE_VM_SSH_HOST` is its host, default
//!   `lima-h2-fuse-r1`). It needs `/dev/fuse`, `fuse3` and `user_allow_other`.
//!   Only cases 2 and 5 stop it (`limactl stop --force h2-fuse-r1`, then
//!   `limactl start h2-fuse-r1`) and only this VM: the name is fixed here and
//!   the ssh config's folder must carry it.
//! - Case 6 runs on `h2-terminal-c1` (`TERMINAL_LAB_SSH_CONFIG`, as
//!   `real_vm.rs`) when that VM has no `/dev/fuse`. It is never stopped.
//!
//! How the remote is driven: a machine daemon's Settings are not reachable
//! from the host, so the test writes `fusefs_enabled = true` into the
//! throwaway user's `~/.humata2/config/settings.toml` and restarts the
//! user's `humatad2.service` (it keeps its helper: `KillMode=process`).
//! What the remote's Settings and agents see is read on the VM as the
//! session's agent does: `humata settings get fusefs_status` and
//! `humatad2 mcp` initialize, with the token and mount variable of a shell
//! the app opened there.
//!
//! No test sleeps: VM-side waits are one remote loop polling every 2 ms;
//! host-side waits end on the host's feed.
//!
//! Run, with the VMs up:
//!
//! ```sh
//! bazel test //apps/humata2/desktop:live_flows_fuse_vm_test --strategy=TestRunner=local \
//!   --test_env=FUSE_VM_SSH_CONFIG="$HOME/.lima/h2-fuse-r1/ssh.config" \
//!   --test_env=TERMINAL_LAB_SSH_CONFIG="$HOME/.lima/h2-terminal-c1/ssh.config" \
//!   --test_output=streamed
//! ```
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

use humata2_clients::model::{MachineId, SessionId, Status};
use humata2_grpc::humata::v1 as pb;

use super::real_vm::{
    Admin, INSTALL, Rig, Vm, VmError, activate, focus_terminal, open_a_shell_on_the_vm, rig_of, shell_pid, tagged,
};
use super::typing::{UPPER, ready, run, wait_for};
use super::world::LONG;
use super::{App, runtime};

/// The only VM these tests may stop and start.
const FUSE_VM: &str = "h2-fuse-r1";
/// Where the throwaway user's machine daemon mounts the tree.
const MNT: &str = "$HOME/.humata2/state/mnt";
const SOCK: &str = "$HOME/.humata2/humatad.sock";
const BIN: &str = "$HOME/.humata2/bin";
/// The most a VM-side wait takes.
const WAIT_SECS: u64 = 120;

/// The h2-fuse-r1 gate: its ssh config and Lima host, or a printed reason.
fn fuse_gate() -> Option<(PathBuf, String)> {
    match std::env::var_os("FUSE_VM_SSH_CONFIG") {
        Some(path) => {
            let host = std::env::var("FUSE_VM_SSH_HOST").unwrap_or_else(|_| "lima-h2-fuse-r1".to_owned());
            Some((PathBuf::from(path), host))
        }
        None => {
            eprintln!(
                "skipped: set FUSE_VM_SSH_CONFIG to the h2-fuse-r1 Lima ssh.config (…/.lima/h2-fuse-r1/ssh.config) and run //apps/humata2/desktop:live_flows_fuse_vm_test"
            );
            None
        }
    }
}

async fn fuse_rig(config: PathBuf, host: String) -> Rig {
    let vm = Vm::start_on(config, host, true)
        .await
        .expect("set up h2-fuse-r1 and add it as a machine");
    rig_of(vm).await
}

// ---- the VM, as the throwaway user -------------------------------------

fn vm_do(vm: &Vm, what: &str, script: &str) -> String {
    vm.on_vm(script).unwrap_or_else(|error| panic!("{what}: {error}"))
}

/// Whether `cond` (a shell command) succeeds on the VM, as the user.
fn vm_test(vm: &Vm, cond: &str) -> bool {
    vm_do(vm, "check the VM", &format!("if {{ {cond}; }}; then echo yes; else echo no; fi")).trim() == "yes"
}

/// Waits on the VM, in one remote loop that polls every 2 ms, until `cond`
/// holds; on giving up it runs `why` (what to print) and fails.
fn wait_vm(vm: &Vm, what: &str, prelude: &str, cond: &str, why: &str) {
    let script = format!(
        "{prelude}\nend=$(( $(date +%s) + {WAIT_SECS} ))\nuntil {{ {cond}; }}; do\n  if [ \"$(date +%s)\" -ge \"$end\" ]; then {{ {why}; }} >&2; exit 1; fi\n  sleep 0.002\ndone"
    );
    vm.on_vm(&script)
        .unwrap_or_else(|error| panic!("timed out waiting on the VM for {what}: {error}"));
}

/// The helper says its mount was refused: a failed mount, not a slow one.
const MOUNT_REFUSED: &str = r#""$HOME/.humata2/bin/humata2_fusefs" --socket "$HOME/.humata2/state/fusefs.sock" status --json 2>/dev/null | grep -q '"mount_state": "failed"'"#;

/// What the daemon said about the mount, for a failure message.
const MOUNT_SAID: &str = "export XDG_RUNTIME_DIR=/run/user/$(id -u); journalctl --user -u humatad2 --no-pager -o cat 2>/dev/null | grep -i fusefs | tail -5";

/// Like [`wait_vm`] for the mount, but gives up at once when the helper
/// reports the mount refused, printing why the daemon couldn't set it up.
fn wait_mount_vm(vm: &Vm, what: &str, cond: &str, why: &str) {
    let script = format!(
        "end=$(( $(date +%s) + {WAIT_SECS} ))\nuntil {{ {cond}; }}; do\n  if {MOUNT_REFUSED}; then {{ echo 'the helper refused the mount; the daemon said:'; {MOUNT_SAID}; {why}; }} >&2; exit 1; fi\n  if [ \"$(date +%s)\" -ge \"$end\" ]; then {{ {why}; }} >&2; exit 1; fi\n  sleep 0.002\ndone"
    );
    vm.on_vm(&script)
        .unwrap_or_else(|error| panic!("waiting on the VM for {what}: {error}"));
}

/// The mount: a fuse filesystem on the user's mount folder.
const MOUNTED: &str = r#"grep -q " $HOME/.humata2/state/mnt fuse" /proc/mounts"#;
const NOT_MOUNTED: &str = r#"! grep -q " $HOME/.humata2/state/mnt fuse" /proc/mounts"#;

fn helper_pids(vm: &Vm) -> Vec<u32> {
    vm_do(
        vm,
        "list the helper",
        r#"pgrep -x -u "$(id -u)" humata2_fusefs || true"#,
    )
    .split_whitespace()
    .filter_map(|pid| pid.parse().ok())
    .collect()
}

/// Waits until one helper runs and the tree is mounted; returns its pid.
fn wait_mounted(vm: &Vm) -> u32 {
    wait_mount_vm(
        vm,
        "one helper and the mounted tree",
        &format!("[ \"$(pgrep -x -u \"$(id -u)\" humata2_fusefs | wc -l)\" = 1 ] && {MOUNTED}"),
        "pgrep -a -u \"$(id -u)\" humata2; grep fuse /proc/mounts",
    );
    helper_pids(vm)[0]
}

fn daemon_pid(vm: &Vm) -> u32 {
    vm_do(
        vm,
        "read the machine daemon's pid",
        "export XDG_RUNTIME_DIR=/run/user/$(id -u); systemctl --user show -p MainPID --value humatad2.service",
    )
    .trim()
    .parse()
    .unwrap_or(0)
}

fn restart_daemon(vm: &Vm) {
    vm_do(
        vm,
        "restart the machine daemon in place",
        "export XDG_RUNTIME_DIR=/run/user/$(id -u); systemctl --user restart humatad2.service",
    );
}

/// Turns the mount on in the remote daemon's settings file and restarts it
/// (it reads the file at start).
fn enable_fuse(vm: &Vm) {
    vm_do(
        vm,
        "turn the FUSE mount on in the machine daemon's settings",
        r#"set -eu
f="$HOME/.humata2/config/settings.toml"
mkdir -p "$(dirname "$f")"
{ echo 'fusefs_enabled = true'; [ ! -f "$f" ] || grep -v '^fusefs_enabled' "$f" || true; } > "$f.new"
mv "$f.new" "$f""#,
    );
    restart_daemon(vm);
}

/// Shell text that sets `tok`, `root` from the environment of process `pid`
/// (a shell the app opened) and defines `st` (the remote's fusefs_status as
/// compact lowercase text), `is` (its state is one of two names),
/// `told` (the MCP initialize instructions name the files tools).
fn session_prelude(pid: u32) -> String {
    format!(
        r#"env_of() {{ tr '\0' '\n' < /proc/{pid}/environ | sed -n "s/^$1=//p" | head -1; }}
tok=$(env_of HUMATA_SESSION_TOKEN); root=$(env_of HUMATA_FUSE_ROOT)
st_raw() {{ HUMATA_SESSION_TOKEN="$tok" HUMATA_SOCKET="{SOCK}" "{BIN}/humata" --json settings get fusefs_status 2>&1; }}
st() {{ st_raw | tr -d ' \n"_' | tr A-Z a-z; }}
is() {{ st | grep -Eq "state:(state)?($1|$2)[,}}]"; }}
told() {{ printf '%s\n' '{{"jsonrpc":"2.0","id":0,"method":"initialize","params":{{"protocolVersion":"2025-06-18","capabilities":{{}},"clientInfo":{{"name":"t","version":"0"}}}}}}' \
  | HUMATA_SESSION_TOKEN="$tok" HUMATA_FUSE_ROOT="$root" timeout 30 "{BIN}/humatad2" mcp --socket "{SOCK}" | head -1 | grep -q files_list; }}
"#
    )
}

#[derive(Clone, Copy, Debug)]
enum Fuse {
    Mounted,
    CannotSetUp,
}

impl Fuse {
    fn cond(self) -> &'static str {
        match self {
            Fuse::Mounted => "is mounted 3",
            Fuse::CannotSetUp => "is cannotsetup 4",
        }
    }
}

/// Waits until the remote's Settings (read as the shell `pid`'s agent) show `state`.
fn wait_settings(vm: &Vm, pid: u32, state: Fuse) {
    wait_vm(
        vm,
        &format!("the remote's Settings to read {state:?}"),
        &session_prelude(pid),
        state.cond(),
        "echo \"last answer of settings get fusefs_status: $(st_raw)\"",
    );
}

/// The remote's raw fusefs_status answer, read as shell `pid`'s agent.
fn settings_text(vm: &Vm, pid: u32) -> String {
    vm_do(vm, "read the remote's Settings", &format!("{}st_raw", session_prelude(pid)))
}

/// Waits until agents there are (or are not) told to use the files tools.
fn wait_told(vm: &Vm, pid: u32, told: bool) {
    let cond = if told { "told" } else { "! told" };
    wait_vm(
        vm,
        &format!("agents to be told to use the files tools: {told}"),
        &session_prelude(pid),
        cond,
        "echo 'the MCP initialize reply did not change'",
    );
}

fn env_of(vm: &Vm, pid: u32, name: &str) -> Option<String> {
    let out = vm_do(
        vm,
        "read a process's environment",
        &format!("tr '\\0' '\\n' < /proc/{pid}/environ | sed -n 's/^{name}=//p' | head -1"),
    );
    let text = out.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

// ---- the host and the app -----------------------------------------------

/// Waits on the host's machines feed until the VM's machine is online with
/// a daemon version (`online`) or not online.
async fn wait_machine(rig: &Rig, online: bool, limit: Duration) {
    let channel = rig.vm.host_channel().await.expect("connect to the host");
    let mut feed = pb::event_service_client::EventServiceClient::new(channel.clone())
        .subscribe(pb::SubscribeRequest {
            topics: vec![pb::Topic::Machines as i32],
            ..Default::default()
        })
        .await
        .expect("open the machines feed")
        .into_inner();
    let mut machines = pb::machine_service_client::MachineServiceClient::new(channel);
    let deadline = tokio::time::Instant::now() + limit;
    loop {
        let listed = machines
            .list_machines(pb::ListMachinesRequest::default())
            .await
            .expect("list the machines")
            .into_inner()
            .machines;
        let now = listed.iter().any(|machine| {
            machine.name == rig.vm.name
                && if online {
                    machine.reachability == pb::Reachability::Online as i32 && !machine.daemon_version.is_empty()
                } else {
                    machine.reachability != pb::Reachability::Online as i32
                }
        });
        if now {
            return;
        }
        match tokio::time::timeout_at(deadline, feed.message()).await {
            Ok(Ok(Some(_))) => {}
            other => panic!("the host never showed the machine online={online}: {other:?}; it last listed {listed:?}"),
        }
    }
}

fn status_of(app: &App<'_>, id: &SessionId) -> Option<Status> {
    app.ui.read(|state| {
        state
            .workspace
            .sessions
            .iter()
            .find(|session| &session.id == id)
            .map(|session| session.status.clone())
    })
}

/// The session is listed and its agent has not ended.
fn not_ended(status: &Option<Status>) -> bool {
    !matches!(status, None | Some(Status::Exited { .. }))
}

/// Alive: listed, and neither ended, stopped nor out of reach.
fn running(status: &Option<Status>) -> bool {
    not_ended(status) && !matches!(status, Some(Status::Offline | Status::Stopped | Status::AgentStopped))
}

fn assert_not_ended(app: &App<'_>, id: &SessionId, when: &str) {
    assert!(not_ended(&status_of(app, id)), "the session ended {when}: {:?}", status_of(app, id));
}

static TAGS: AtomicU8 = AtomicU8::new(0);

/// A tag no earlier answer on the screen carries.
fn fresh_tag(prefix: &str) -> String {
    let n = TAGS.fetch_add(1, Ordering::SeqCst);
    format!("xq{prefix}{}{}", (b'a' + n % 26) as char, (b'a' + n / 26) as char)
}

/// The shell pid of the shown terminal, once its prompt is there.
fn current_shell_pid(app: &mut App<'_>) -> u32 {
    ready(app, "$");
    shell_pid(app, &fresh_tag("p"))
}

/// Has the shown shell print what the shell command `expr` prints (digits
/// and `x` only) and returns it.
fn printed(app: &mut App<'_>, expr: &str) -> String {
    let tag = fresh_tag("v");
    run(app, &format!("echo {tag}$({expr})q"));
    app.wait_within(&format!("the shell to print {tag}"), LONG, |app| {
        let view = app.view();
        !tagged(&view, &tag).is_empty()
    });
    let view = app.view();
    tagged(&view, &tag).pop().expect("a printed value")
}

/// A machine of the workspace that isn't `vm`: This Mac.
fn mac_of(app: &App<'_>, vm: &MachineId) -> MachineId {
    app.ui
        .read(|state| state.workspace.machines.iter().find(|m| &m.id != vm).map(|m| m.id.clone()))
        .expect("the workspace lists This Mac")
}

/// FUSE on in the VM's machine daemon, the host showing it online, the
/// tree mounted, one shell open on the VM with the mount's variable, and the
/// remote's Settings reading Mounted. Returns the session, its shell pid and
/// the helper's pid.
fn mounted_shell(runtime: &tokio::runtime::Runtime, rig: &Rig, app: &mut App<'_>) -> (SessionId, u32, u32) {
    enable_fuse(&rig.vm);
    runtime.block_on(wait_machine(rig, true, INSTALL));
    let helper = wait_mounted(&rig.vm);
    let id = open_a_shell_on_the_vm(app, &rig.machine);
    let pid = shell_pid(app, &fresh_tag("p"));
    assert!(
        env_of(&rig.vm, pid, "HUMATA_FUSE_ROOT").is_some(),
        "an agent started while the mount is up gets HUMATA_FUSE_ROOT"
    );
    wait_settings(&rig.vm, pid, Fuse::Mounted);
    (id, pid, helper)
}

fn limactl(config: &Path, verb: &[&str]) {
    let folder = config.parent().expect("the ssh config sits in the VM's Lima folder");
    assert_eq!(
        folder.file_name().and_then(|name| name.to_str()),
        Some(FUSE_VM),
        "only {FUSE_VM} may be stopped and started: FUSE_VM_SSH_CONFIG must be its ssh.config"
    );
    let binary = std::env::var_os("LIMACTL").map(PathBuf::from).unwrap_or_else(|| {
        let brew = PathBuf::from("/opt/homebrew/bin/limactl");
        if brew.exists() { brew } else { PathBuf::from("limactl") }
    });
    let lima_home = folder.parent().expect("the Lima home");
    let status = Command::new(binary)
        .args(verb)
        .arg(FUSE_VM)
        .env("LIMA_HOME", lima_home)
        .env("PATH", "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin")
        .status()
        .unwrap_or_else(|error| panic!("couldn't run limactl {verb:?} {FUSE_VM}: {error}; install Lima or set LIMACTL"));
    assert!(status.success(), "limactl {verb:?} {FUSE_VM} failed ({status})");
}

// ---- case 1, the remote part ---------------------------------------------

/// Case 1 (remote part): the machine daemon restarts in place with FUSE on.
/// The live helper and the mount stay, no second helper starts, and a
/// running session keeps `HUMATA_FUSE_ROOT` and its shell.
#[test]
fn fuse_vm_1_a_machine_daemon_restarted_in_place_keeps_its_helper_and_mount() {
    let Some((config, host)) = fuse_gate() else { return };
    let runtime = runtime();
    let rig = runtime.block_on(fuse_rig(config, host));
    let clients = runtime.block_on(rig.vm.clients()).expect("connect to the host");
    let mut app = App::open(&runtime, clients);
    activate(&mut app);
    let (id, pid, helper) = mounted_shell(&runtime, &rig, &mut app);
    let root = env_of(&rig.vm, pid, "HUMATA_FUSE_ROOT");
    let before = daemon_pid(&rig.vm);

    restart_daemon(&rig.vm);
    let after = daemon_pid(&rig.vm);
    assert_ne!(before, after, "the machine daemon was replaced");

    wait_settings(&rig.vm, pid, Fuse::Mounted);
    assert_eq!(helper_pids(&rig.vm), vec![helper], "the new daemon adopted the live helper and started no second one");
    assert!(vm_test(&rig.vm, MOUNTED), "the tree stays mounted");
    assert_eq!(env_of(&rig.vm, pid, "HUMATA_FUSE_ROOT"), root, "the running session keeps its mount variable");

    runtime.block_on(wait_machine(&rig, true, INSTALL));
    app.wait_within("the session to run again", LONG, |app| running(&status_of(app, &id)));
    assert_not_ended(&app, &id, "while the daemon restarted");
    app.show_session(&id);
    focus_terminal(&mut app);
    assert_eq!(current_shell_pid(&mut app), pid, "the same shell, not a new one");
    drop(app);
    drop(rig);
}

// ---- case 2 ----------------------------------------------------------------

/// Case 2: a remote power cut with FUSE on. The host shows the machine
/// offline; after boot the daemon, the mount and Settings come back, the
/// session resumes with the mount variable, and no session ended.
#[test]
fn fuse_vm_2_a_remote_power_cut_brings_the_daemon_mount_and_sessions_back() {
    let Some((config, host)) = fuse_gate() else { return };
    let runtime = runtime();
    let rig = runtime.block_on(fuse_rig(config.clone(), host));
    let clients = runtime.block_on(rig.vm.clients()).expect("connect to the host");
    let mut app = App::open(&runtime, clients);
    activate(&mut app);
    let (id, _pid, _helper) = mounted_shell(&runtime, &rig, &mut app);

    limactl(&config, &["stop", "--force"]);
    runtime.block_on(wait_machine(&rig, false, INSTALL));
    app.wait_within("the session to show its machine out of reach", LONG, |app| {
        matches!(status_of(app, &id), Some(Status::Offline))
    });
    assert_not_ended(&app, &id, "when its machine lost power");

    limactl(&config, &["start"]);
    rig.vm.refresh_link_config().expect("Lima's new ssh port is known again");
    wait_vm(
        &rig.vm,
        "the machine daemon service to run after boot",
        "export XDG_RUNTIME_DIR=/run/user/$(id -u)",
        "systemctl --user is-active --quiet humatad2.service",
        "systemctl --user status humatad2.service --no-pager",
    );
    runtime.block_on(wait_machine(&rig, true, INSTALL));
    wait_mounted(&rig.vm);
    app.wait_within("the session to run again", LONG, |app| running(&status_of(app, &id)));
    assert_not_ended(&app, &id, "after the machine booted");
    app.show_session(&id);
    focus_terminal(&mut app);
    let pid = current_shell_pid(&mut app);
    assert!(
        env_of(&rig.vm, pid, "HUMATA_FUSE_ROOT").is_some(),
        "the resumed session gets the mount variable"
    );
    wait_settings(&rig.vm, pid, Fuse::Mounted);
    assert_not_ended(&app, &id, "once everything was back");
    drop(app);
    drop(rig);
}

// ---- case 3 ----------------------------------------------------------------

/// Case 3: the link drops and comes back. While it is down the session
/// shows its machine out of reach, and agents on the VM are not told to use
/// the files tools: the mount there can still be set up. On reconnect the
/// mount is there, Settings reads Mounted and no session ended.
#[test]
fn fuse_vm_3_a_dropped_link_changes_the_status_and_the_mount_survives_the_reconnect() {
    let Some((config, host)) = fuse_gate() else { return };
    let runtime = runtime();
    let rig = runtime.block_on(fuse_rig(config, host));
    let clients = runtime.block_on(rig.vm.clients()).expect("connect to the host");
    let mut app = App::open(&runtime, clients);
    activate(&mut app);
    let (id, pid, helper) = mounted_shell(&runtime, &rig, &mut app);

    rig.vm.cut_link();
    runtime.block_on(wait_machine(&rig, false, LONG));
    app.wait_within("the session to show its machine out of reach", LONG, |app| {
        matches!(status_of(app, &id), Some(Status::Offline))
    });
    assert_not_ended(&app, &id, "while the link was down");
    // On the VM nothing changed: still mounted, so agents are not sent to
    // the files tools.
    assert!(vm_test(&rig.vm, MOUNTED), "the tree stays mounted while the host can't be reached");
    assert_eq!(helper_pids(&rig.vm), vec![helper]);
    wait_told(&rig.vm, pid, false);

    rig.vm.restore_link();
    runtime.block_on(wait_machine(&rig, true, INSTALL));
    app.wait_within("the session to run again", LONG, |app| running(&status_of(app, &id)));
    wait_settings(&rig.vm, pid, Fuse::Mounted);
    assert_eq!(helper_pids(&rig.vm), vec![helper], "reconnecting started no second helper");
    assert_not_ended(&app, &id, "after the link came back");
    drop(app);
    drop(rig);
}

// ---- case 4 ----------------------------------------------------------------

/// Case 4: a session moved from This Mac to the mounted VM and back. At the
/// VM it gets the mount variable and the VM's tree lists it under that
/// machine; moved back, the VM's tree drops it.
#[test]
fn fuse_vm_4_a_moved_session_gets_the_mount_at_its_new_place_and_the_old_mount_drops_it() {
    let Some((config, host)) = fuse_gate() else { return };
    let runtime = runtime();
    let rig = runtime.block_on(fuse_rig(config, host));
    let clients = runtime.block_on(rig.vm.clients()).expect("connect to the host");
    let mut app = App::open(&runtime, clients);
    activate(&mut app);
    enable_fuse(&rig.vm);
    runtime.block_on(wait_machine(&rig, true, INSTALL));
    wait_mounted(&rig.vm);
    let mac = mac_of(&app, &rig.machine);
    let id = open_a_shell_on_the_vm(&mut app, &mac);
    let listed = format!("{MNT}/m/{}/s/{}", rig.machine.as_str(), id.as_str());

    runtime
        .block_on(app.daemon.move_session(&id, &rig.machine))
        .expect("the move to the VM");
    app.wait_within("the session to be on the VM", LONG, |app| {
        app.ui.read(|state| {
            state
                .workspace
                .sessions
                .iter()
                .any(|session| session.id == id && session.machine == rig.machine)
        }) && running(&status_of(app, &id))
    });
    app.show_session(&id);
    focus_terminal(&mut app);
    let pid = current_shell_pid(&mut app);
    assert!(
        env_of(&rig.vm, pid, "HUMATA_FUSE_ROOT").is_some(),
        "the session gets the mount variable where it moved to"
    );
    wait_vm(
        &rig.vm,
        "the VM's tree to list the session under the VM",
        "",
        &format!("[ -e \"{listed}\" ]"),
        &format!("ls \"{MNT}/m\" \"{MNT}/s\""),
    );

    runtime
        .block_on(app.daemon.move_session(&id, &mac))
        .expect("the move back to This Mac");
    app.wait_within("the session to be back on This Mac", LONG, |app| {
        app.ui.read(|state| {
            state
                .workspace
                .sessions
                .iter()
                .any(|session| session.id == id && session.machine == mac)
        }) && running(&status_of(app, &id))
    });
    wait_vm(
        &rig.vm,
        "the VM's tree to drop the session",
        "",
        &format!("[ ! -e \"{listed}\" ]"),
        &format!("ls -R \"{MNT}/m\""),
    );
    assert_not_ended(&app, &id, "by the moves");
    drop(app);
    drop(rig);
}

// ---- case 5 ----------------------------------------------------------------

/// Case 5: the real mount. An agent shell on the VM reads and writes through
/// `$HUMATA_FUSE_ROOT`. The helper, killed, comes back once; killed again
/// within 30 s, Settings reads "can't set up" and agents are told to use the
/// files tools.
#[test]
fn fuse_vm_5_the_real_mount_reads_and_writes_and_a_twice_killed_helper_is_given_up_on() {
    let Some((config, host)) = fuse_gate() else { return };
    let runtime = runtime();
    let rig = runtime.block_on(fuse_rig(config, host));
    let clients = runtime.block_on(rig.vm.clients()).expect("connect to the host");
    let mut app = App::open(&runtime, clients);
    activate(&mut app);
    let (id, pid, helper) = mounted_shell(&runtime, &rig, &mut app);
    wait_told(&rig.vm, pid, false);

    // Read and write through the mount from the agent's own shell.
    let meta = format!("\"$HUMATA_FUSE_ROOT/s/{}/meta.json\"", id.as_str());
    let note = format!("\"$HUMATA_FUSE_ROOT/s/{}/context/xqnote.txt\"", id.as_str());
    assert_eq!(printed(&mut app, &format!("cat {meta} | grep -c machine_id")), "1", "the session's meta.json reads through the mount");
    run(&mut app, &format!("echo xqwrote > {note}"));
    run(&mut app, &format!("cat {note}{UPPER}"));
    wait_for(&mut app, "XQWROTE");

    // The first kill: it comes back, once.
    vm_do(&rig.vm, "kill the helper", &format!("kill -KILL {helper}"));
    wait_mount_vm(
        &rig.vm,
        "a new helper and the tree again",
        &format!("[ \"$(pgrep -x -u \"$(id -u)\" humata2_fusefs | wc -l)\" = 1 ] && ! pgrep -x -u \"$(id -u)\" humata2_fusefs | grep -qx {helper} && {MOUNTED}"),
        "pgrep -a -u \"$(id -u)\" humata2; grep fuse /proc/mounts",
    );
    let second = helper_pids(&rig.vm);
    assert_eq!(second.len(), 1, "one helper, not two");
    wait_settings(&rig.vm, pid, Fuse::Mounted);

    // The second kill, within 30 s of the new helper's mount.
    vm_do(&rig.vm, "kill the new helper", &format!("kill -KILL {}", second[0]));
    wait_settings(&rig.vm, pid, Fuse::CannotSetUp);
    let text = settings_text(&rig.vm, pid);
    assert!(text.contains("reason"), "Settings carries a reason: {text}");
    wait_vm(&rig.vm, "no helper to stay down", "", "! pgrep -x -u \"$(id -u)\" humata2_fusefs", "pgrep -a humata2");
    assert!(helper_pids(&rig.vm).is_empty(), "no third helper starts by itself");
    wait_told(&rig.vm, pid, true);
    drop(app);
    drop(rig);
}

// ---- case 6 ----------------------------------------------------------------

/// Runs a `humata` command on the VM as the agent of shell `pid`; approves
/// what waits in the Inbox when the call is pending (exit 12) and runs it
/// again.
fn humata_as_agent(runtime: &tokio::runtime::Runtime, rig: &Rig, pid: u32, args: &str) -> String {
    let script = format!(
        "{}HUMATA_SESSION_TOKEN=\"$tok\" HUMATA_SOCKET=\"{SOCK}\" \"{BIN}/humata\" --json {args}",
        session_prelude(pid)
    );
    for _ in 0..4 {
        match rig.vm.on_vm(&script) {
            Ok(out) => return out,
            Err(VmError::Remote { status, .. }) if status.contains("12") => runtime.block_on(approve_pending(rig)),
            Err(error) => panic!("humata {args}: {error}"),
        }
    }
    panic!("humata {args} stayed pending in the Inbox")
}

async fn approve_pending(rig: &Rig) {
    let channel = rig.vm.host_channel().await.expect("connect to the host");
    let mut inbox = pb::inbox_service_client::InboxServiceClient::new(channel);
    let ids = inbox
        .list_requests(pb::ListRequestsRequest::default())
        .await
        .expect("list the Inbox")
        .into_inner()
        .requests
        .into_iter()
        .filter_map(|request| request.id)
        .collect::<Vec<_>>();
    if ids.is_empty() {
        return;
    }
    inbox
        .answer_all(pb::AnswerAllRequest {
            idempotency_key: Some(pb::IdempotencyKey {
                value: format!("approve-{}", ids.iter().map(|id| id.value.as_str()).collect::<Vec<_>>().join("-")),
            }),
            ids,
            approve: true,
            ..Default::default()
        })
        .await
        .expect("approve what waits");
}

/// Case 6: a VM with no FUSE. Settings reads "can't set up" with a reason,
/// agents there are not given the mount variable and are told to use the
/// files tools, and `humata files` (the calls the MCP tools make) reaches
/// every location live.
#[test]
fn fuse_vm_6_a_machine_with_no_fuse_says_why_and_agents_use_the_files_tools() {
    let Some(config) = std::env::var_os("TERMINAL_LAB_SSH_CONFIG").map(PathBuf::from) else {
        eprintln!("skipped: set TERMINAL_LAB_SSH_CONFIG to the h2-terminal-c1 Lima ssh.config; case 6 runs on the VM that has no /dev/fuse");
        return;
    };
    let host = std::env::var("TERMINAL_LAB_SSH_HOST").unwrap_or_else(|_| "lima-h2-terminal-c1".to_owned());
    let probe = Admin { config: config.clone(), host: host.clone() };
    match probe.run("look for /dev/fuse", "[ -c /dev/fuse ] && echo yes || echo no") {
        Ok(answer) if answer.trim() == "no" => {}
        Ok(_) => {
            eprintln!("skipped: this VM has /dev/fuse; run case 6 on h2-fuse-r1 with /dev/fuse hidden from the throwaway user, or on a VM without it");
            return;
        }
        Err(error) => panic!("couldn't look at the VM: {error}"),
    }
    let runtime = runtime();
    let vm = runtime
        .block_on(Vm::start_on(config, host, false))
        .expect("set up the VM and add it as a machine");
    let rig = runtime.block_on(rig_of(vm));
    let clients = runtime.block_on(rig.vm.clients()).expect("connect to the host");
    let mut app = App::open(&runtime, clients);
    activate(&mut app);

    enable_fuse(&rig.vm);
    runtime.block_on(wait_machine(&rig, true, INSTALL));
    let id = open_a_shell_on_the_vm(&mut app, &rig.machine);
    let pid = shell_pid(&mut app, &fresh_tag("p"));
    assert!(
        env_of(&rig.vm, pid, "HUMATA_FUSE_ROOT").is_none(),
        "no mount, so no folder to point agents at"
    );
    wait_settings(&rig.vm, pid, Fuse::CannotSetUp);
    assert!(settings_text(&rig.vm, pid).contains("reason"), "Settings says why");
    wait_told(&rig.vm, pid, true);

    // The files tools reach every location, live: this machine's shared and
    // channel folders, and This Mac's.
    let mac = mac_of(&app, &rig.machine);
    let channel_id = rig.channel.as_str().to_owned();
    vm_do(&rig.vm, "write a file to put", "printf xqfuse5 > \"$HOME/xqsource.txt\"");
    for (n, place) in [
        format!("{}:shared/xq-vm-shared.txt", rig.machine.as_str()),
        format!("{}:channel/xq-vm-channel.txt", rig.machine.as_str()),
        format!("{}:shared/xq-mac-shared.txt", mac.as_str()),
    ]
    .into_iter()
    .enumerate()
    {
        humata_as_agent(
            &runtime,
            &rig,
            pid,
            &format!("--channel {channel_id} files put \"$HOME/xqsource.txt\" {place}"),
        );
        let read = humata_as_agent(&runtime, &rig, pid, &format!("--channel {channel_id} files cat {place}"));
        let compact: String = read.split_whitespace().collect();
        assert!(compact.contains("\"bytes\":7"), "location {n} ({place}) reads back what was put: {read}");
    }
    assert_not_ended(&app, &id, "while the files tools ran");
    drop(app);
    drop(rig);
}
