//! A host daemon and any number of machines behind one fake `ssh`, for the
//! cross-machine tests (move, remote fork, remote archive and restore,
//! removing a machine, forgetting one, rebuilding one).
//!
//! A machine here is three things the test owns: a fake machine (a home
//! folder and a fake `ssh` that runs commands in it), the machine's own
//! `humatad2` in the machine role (the release binary, started by the test,
//! since a link only relays to the machine's daemon and serves no files
//! itself), and a control folder for its link. The host is the admin
//! daemon; the only `ssh` on its PATH is this rig's, which hands each call
//! to the right machine.
//!
//! Every machine has conversations of its own: its agent writes them under
//! `<its home>/conv`, the host's under `world.conv`, so a test can tell
//! which machine holds a conversation. The machines share only the git
//! remotes, which are the host world's.
//!
//! Use it as
//! `mod common; #[path = "common/remote_rig.rs"] mod remote_rig;`.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use humata2_fake_machine::{FakeMachine, shared_executable};
use humata2_grpc::humata::v1 as pb;
use humata2_machines::StackConfig;
use humata2_store::{Store, TabRecord};
use rustix::process::{Pid, Signal, kill_process_group};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;
use tonic::Streaming;
use tonic::transport::Channel;

use crate::common::{AGENT_NAME, Build, Daemon, World, binary, folder, start_request};

/// The most a test waits for something the daemons do on their own. It is a
/// safety net that fails the test saying what it waited for, never a pace:
/// every wait ends on a pushed change. The slowest thing waited for is a
/// link coming back: the host dials again 5 s after a link ends, then 10,
/// 20 and at most 30 s after each dial that fails, and a link's start is a
/// daemon's start (15 s at most). This is more than twice the two together.
pub const LONG: Duration = Duration::from_secs(120);

/// How soon the host dials a machine again after its link drops
/// (`HUMATA2_PEER_RETRY_MS`, test daemons only): at once, where a shipped
/// host waits 5 s and more (HO-5), so a test that restarts a box waits for
/// the box and not for the host's backoff.
const PEER_RETRY: (&str, &str) = ("HUMATA2_PEER_RETRY_MS", "20");
const REVIVE_SETTLE: (&str, &str) = ("HUMATA2_REVIVE_SETTLE_MS", "50");

const FILES: [&str; 5] = ["humatad2", "humata2_holder", "humata", "stack", "humata2_fusefs"];

/// `humatad2` as a machine gets it from the tools bundle.
const WRAPPER: &str = r##"#!/bin/sh
# humatad2 on one of this test's machines: the real one. A link is told the
# state folder and socket of the machine's own daemon, which the test
# started in the folder `~/.humata2` points at. The folder is named by its
# short, real path, because a socket's path must be short.
# `humatad2 stop` (D262), which the host runs over SSH, is told the same.
case " $* " in *" --stdio "*|" stop "*)
  at=$(cd -P "$HOME/.humata2" && pwd -P) || exit 70
  exec '__REAL__' "$@" --state "$at/state" --socket "$at/d.sock" ;;
esac
exec '__REAL__' "$@"
"##;

/// The host's `ssh`.
const SSH: &str = r##"#!/bin/sh
# The host's `ssh` in this test: each call goes to the fake machine whose
# host name it names. A link command (`serve --stdio`) runs in a process
# group of its own and leaves the group's id in the machine's control
# folder, so the test can end that one link and nothing else.
#
# The rig's folders are beside the `bin` folder this file is in, and a
# machine's control folder names its fake ssh, so every rig of a test runs
# this same file (see `shared_executable`).
root=${0%/bin/ssh}
if [ "$root" = "$0" ]; then
  echo "ssh: this test's ssh ran as $0, not as <rig>/bin/ssh, so its machines are unknown" >&2
  exit 255
fi
name=
for arg in "$@"; do
  case "$arg" in
    */*) ;;
    *.internal) [ -f "$root/links/${arg%.internal}/ssh" ] && name=${arg%.internal} ;;
  esac
done
if [ -z "$name" ]; then
  echo "ssh: none of this test's machines is named in: $*" >&2
  exit 255
fi
control=$root/links/$name
IFS= read -r ssh < "$control/ssh"
for last in "$@"; do :; done
case "$last" in
  *'serve --stdio'*)
    echo $$ > "$control/link.new" && mv "$control/link.new" "$control/link"
    exec '__SETSID__' "$ssh" "$@" ;;
esac
exec "$ssh" "$@"
"##;

/// What an agent stands in as, writing its conversations under `conv`: the
/// same script as the shared harness's, which keeps its own private.
fn agent_script(conv: &Path) -> String {
    format!(
        r#"CONV_DIR='{}'
mode=$1; id=$2
case "$mode" in
  --session-id) f="$CONV_DIR/$id.jsonl"; : > "$f" ;;
  --resume) f="$CONV_DIR/$id.jsonl" ;;
  --fresh) f="$CONV_DIR/other.jsonl"; : > "$f" ;;
esac
# Reads keys itself, as an agent's CLI does: a holder types into a program
# still editing lines only after waiting out its keys limit (5 s).
stty -icanon 2>/dev/null
echo "agent up $mode"
while IFS= read -r line; do echo "got: $line"; printf '%s\n' "$line" >> "$f"; done
"#,
        conv.display()
    )
}

/// The agents file of an agent whose conversations are in `conv`, written
/// into `root`, with [`cli_agents`] beside it; returns the file's path.
fn agents_file(root: &Path, conv: &Path) -> PathBuf {
    let script = root.join("agent.sh");
    std::fs::write(&script, agent_script(conv)).unwrap();
    let agents = serde_json::json!([{
        "name": AGENT_NAME,
        "program": "/bin/sh",
        "start_args": [script, "--session-id", "{conversation}"],
        "resume_args": [script, "--resume", "{conversation}"],
        "conversation_file": format!("{}/{{conversation}}.jsonl", conv.display()),
    }]);
    let file = root.join("agents.json");
    std::fs::write(&file, agents.to_string()).unwrap();
    with_cli_agents(&file, root);
    file
}

/// The agent that stands in as Codex on every machine.
pub const CODEX_AGENT: &str = "codex";
/// The agent that stands in as Copilot on every machine.
pub const COPILOT_AGENT: &str = "copilot";

/// What stands in for the Codex and Copilot CLIs: it only answers, and
/// never writes its conversation, so what a test put there is what a copy
/// carries.
const CLI_AGENT: &str = r#"# Reads keys itself, as an agent's CLI does: a holder types into a program
# still editing lines only after waiting out its keys limit (5 s).
stty -icanon 2>/dev/null
echo "agent up $1"
while IFS= read -r line; do echo "got: $line"; done
"#;

/// Adds to the agents file `file` the Codex and Copilot agents, run by a
/// script written into `root`, each keeping its conversations under the
/// machine's home where that CLI does. A test writes what they hold.
fn with_cli_agents(file: &Path, root: &Path) {
    let script = root.join("cli-agent.sh");
    std::fs::write(&script, CLI_AGENT).unwrap();
    let agent = |name: &str, format: &str, conversation_file: &str| {
        serde_json::json!({
            "name": name,
            "program": "/bin/sh",
            "start_args": [script, "--session-id", "{conversation}"],
            "resume_args": [script, "--resume", "{conversation}"],
            "conversation_file": conversation_file,
            "conversation_format": format,
            "version_args": [],
        })
    };
    let mut agents: Vec<serde_json::Value> =
        serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
    agents.push(agent(
        CODEX_AGENT,
        "codex-rollout",
        "{home}/.codex/sessions/2026/11/16/rollout-2026-11-16T00-00-00-{conversation}.jsonl",
    ));
    agents.push(agent(
        COPILOT_AGENT,
        "copilot-session-state",
        "{home}/.copilot/session-state/{conversation}/events.jsonl",
    ));
    std::fs::write(file, serde_json::Value::Array(agents).to_string()).unwrap();
}

/// A tools bundle for linux-x86_64, the fake machine's platform, whose
/// humatad2 is [`WRAPPER`] around the real binary.
fn tools(dir: &Path) -> PathBuf {
    let root = dir.join("tools");
    std::fs::create_dir_all(root.join("linux-x86_64")).unwrap();
    let mut entries = serde_json::Map::new();
    for file in FILES {
        let body = if file == "humatad2" {
            let real = binary("HUMATAD2_RELEASE").display().to_string();
            WRAPPER.replace("__REAL__", &real)
        } else {
            format!("#!/bin/sh\n# {file}\nexit 0\n")
        };
        std::fs::write(root.join("linux-x86_64").join(file), &body).unwrap();
        let sha256: String = Sha256::digest(body.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        entries.insert(
            file.to_owned(),
            serde_json::json!({ "path": format!("linux-x86_64/{file}"), "sha256": sha256 }),
        );
    }
    let manifest = serde_json::json!({ "platforms": { "linux-x86_64": entries } });
    std::fs::write(root.join("manifest.json"), manifest.to_string()).unwrap();
    root
}

/// The token a shell printed between `TOKEN<` and `>`, if it has. The typed
/// command spells those markers as octal escapes, so its own echo never
/// matches them.
fn printed_token(screen: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(screen);
    let (_, rest) = text.split_once("TOKEN<")?;
    let (token, _) = rest.split_once('>')?;
    Some(token.to_owned())
}

/// Not offline, not stopped: the session's agent is up in some way.
pub fn is_live(session: &pb::Session) -> bool {
    use pb::SessionStatus as S;
    [
        S::Starting,
        S::Working,
        S::NeedsInput,
        S::Idle,
        S::RunningUnknownTurn,
        S::ProbablyIdle,
    ]
    .iter()
    .any(|live| *live as i32 == session.status)
}

pub fn is_offline(session: &pb::Session) -> bool {
    session.status == pb::SessionStatus::Offline as i32
}

/// `env` with `{dir}`, `{state}` and `{channels}` replaced by the daemon's
/// folder, its state folder and the folder its channels are in, so a test
/// can name them in `HUMATA2_POWER_CUT_ROOTS` before the rig has made them.
fn filled(env: &[(&str, &str)], dir: &Path, channels: &Path) -> Vec<(String, String)> {
    env.iter()
        .map(|(name, value)| {
            let value = value
                .replace("{dir}", &dir.display().to_string())
                .replace("{state}", &dir.join("state").display().to_string())
                .replace("{channels}", &channels.display().to_string());
            ((*name).to_owned(), value)
        })
        .collect()
}

/// One machine. Fields drop in order: its daemon ends before its folders go.
pub struct Remote {
    pub name: &'static str,
    /// Where this rig's `ssh` leaves the link's process group.
    control: PathBuf,
    /// The machine's own humatad2: what its link relays to.
    daemon: Daemon,
    machine: FakeMachine,
    /// Its daemon was killed and not started again.
    down: bool,
}

impl Remote {
    async fn start(name: &'static str, links: &Path, template: &str, build: Build, env: &[(&str, &str)]) -> Self {
        let machine = FakeMachine::new();
        // The machine's `~/.humata2` is its daemon's folder: the add writes
        // the enrollment key into the state folder the daemon uses, and the
        // link finds the daemon's socket there.
        let dir = folder();
        std::os::unix::fs::symlink(dir.path(), machine.home().join(".humata2")).unwrap();
        let channels = machine.home().join("Humata");
        std::fs::create_dir_all(&channels).unwrap();
        // Conversations of this machine alone.
        let conv = machine.home().join("conv");
        std::fs::create_dir_all(&conv).unwrap();
        let agents = agents_file(machine.home(), &conv);
        let control = links.join(name);
        std::fs::create_dir_all(&control).unwrap();
        // What the rig's `ssh` runs for this machine.
        std::fs::write(control.join("ssh"), format!("{}\n", machine.ssh_dir().join("ssh").display())).unwrap();
        // A box asked for a fault build (D243) has its state folder made the
        // way the daemon wants it before the power-cut roots name it.
        let mut launch_env = vec![("HOME".to_owned(), machine.home().display().to_string())];
        if !env.is_empty() {
            crate::common::state_folder(dir.path());
            launch_env.extend(filled(env, dir.path(), &channels));
        }
        let launch_env: Vec<(&str, &str)> =
            launch_env.iter().map(|(name, value)| (name.as_str(), value.as_str())).collect();
        let mut daemon = Daemon::start_with_env(
            build,
            dir,
            &Self::serve_args(&channels, &agents, template)
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            &launch_env,
        )
        .await;
        // `start_with_env` keeps the extra arguments for restarts.
        let _ = &mut daemon;
        Self {
            name,
            control,
            daemon,
            machine,
            down: false,
        }
    }

    fn serve_args(channels: &Path, agents: &Path, template: &str) -> Vec<String> {
        vec![
            "--role".to_owned(),
            "machine".to_owned(),
            "--channels".to_owned(),
            channels.display().to_string(),
            "--agents".to_owned(),
            agents.display().to_string(),
            "--default-agent".to_owned(),
            AGENT_NAME.to_owned(),
            "--remote-template".to_owned(),
            template.to_owned(),
        ]
    }

    /// The machine's own daemon: its socket and its folder, for a terminal
    /// on the machine to talk to.
    pub fn daemon(&self) -> &Daemon {
        &self.daemon
    }

    /// The machine's home folder.
    pub fn home(&self) -> &Path {
        self.machine.home()
    }

    /// Where the machine keeps its channels: `~/Humata`.
    pub fn channels(&self) -> PathBuf {
        self.home().join("Humata")
    }

    /// `relative` in `channel`'s folder on this machine.
    pub fn path(&self, channel: &pb::Channel, relative: &str) -> PathBuf {
        self.channels().join(&channel.folder).join(relative)
    }

    /// The machine's own store.
    pub fn store(&self) -> Store {
        Store::open(self.daemon.state.join("store")).unwrap()
    }

    /// Every tab the machine's store knows of.
    pub fn tabs(&self) -> Vec<TabRecord> {
        crate::common::stored_tabs(&self.daemon.state)
    }

    /// The conversation id this machine's store pinned for `session`'s
    /// agent (the host's own view of a remote session doesn't carry it).
    pub fn conversation_id(&self, session: &pb::SessionId) -> String {
        self.store()
            .agent(&session.value)
            .unwrap()
            .unwrap_or_else(|| panic!("{} has no agent for session {}", self.name, session.value))
            .launch
            .conversation_id
    }

    /// The conversation file this machine's agent writes for `id`.
    pub fn conversation(&self, id: &str) -> PathBuf {
        self.home().join("conv").join(format!("{id}.jsonl"))
    }

    /// This machine's Humata trash, on its disk.
    pub fn trash(&self) -> PathBuf {
        self.channels().join(".trash")
    }

    /// The power loss: the machine's daemon ends at once. Its holders keep
    /// running.
    pub fn kill(&mut self) {
        self.daemon.kill();
        self.down = true;
    }

    /// Starts the machine's daemon again on the same state and socket, as a
    /// reboot does. `env` is for this start only; the machine's `HOME` is
    /// always there.
    pub async fn relaunch(&mut self, env: &[(&str, &str)]) {
        let home = self.home().display().to_string();
        let mut all = vec![("HOME", home.as_str())];
        all.extend_from_slice(env);
        self.daemon.restart_env(&all).await;
        self.down = false;
    }

    /// Holds every dial the host makes to this machine in the fake `ssh`,
    /// as a machine that is off does, until [`Rig::link_back`]: for a host
    /// started again while this machine's daemon is down, so its first dial
    /// reaches the machine once it is back instead of a machine with no
    /// daemon.
    #[allow(dead_code)]
    pub fn hold_dials(&self) {
        self.machine.set("hang");
    }

    /// Every dial the host makes to this machine is refused at once, as a
    /// machine that is gone for good refuses it, until
    /// [`Self::take_dials`]. A dial held by [`Self::hold_dials`] is let go
    /// and refused too.
    #[allow(dead_code)]
    pub fn refuse_dials(&self) {
        self.machine.set("down");
        self.machine.clear("hang");
    }

    /// Dials reach this machine again after [`Self::refuse_dials`].
    #[allow(dead_code)]
    pub fn take_dials(&self) {
        self.machine.clear("down");
    }

    /// Waits until this machine's daemon ends by itself, at a stop point
    /// its start's environment named.
    pub async fn stopped(&mut self) {
        self.daemon.stopped(LONG).await;
        self.down = true;
    }

    /// The process group of this machine's link while one is up, as this
    /// rig's `ssh` left it when the host dialled: what a test pauses
    /// (SIGSTOP) to make a link that is up stop answering.
    pub fn link_group(&self) -> Option<Pid> {
        std::fs::read_to_string(self.control.join("link"))
            .ok()
            .and_then(|text| text.trim().parse::<i32>().ok())
            .and_then(Pid::from_raw)
    }

    /// [`Self::cut`] when a link left its group and the group still runs;
    /// nothing otherwise.
    fn cut_if_linked(&self) {
        let file = self.control.join("link");
        let group = std::fs::read_to_string(&file)
            .ok()
            .and_then(|text| text.trim().parse::<i32>().ok())
            .and_then(Pid::from_raw);
        if let Some(group) = group {
            // A group that ended by itself is what was wanted.
            let _ = kill_process_group(group, Signal::KILL);
            let _ = std::fs::remove_file(&file);
        }
    }

    /// Ends this machine's link at once, as a dropped connection does:
    /// every process of it, and nothing else. The link command runs in a
    /// process group of its own, whose id this rig's `ssh` left in the
    /// control folder when the host dialled.
    fn cut(&self) {
        let file = self.control.join("link");
        let group = std::fs::read_to_string(&file)
            .ok()
            .and_then(|text| text.trim().parse::<i32>().ok())
            .and_then(Pid::from_raw)
            .unwrap_or_else(|| {
                panic!(
                    "{}'s link left no process group in {}; is its link up?",
                    self.name,
                    file.display()
                )
            });
        // Since D317 a link can end by itself (its machine daemon gone), so a
        // group that no longer exists is a link already ended. macOS answers
        // EPERM, not ESRCH, for a group whose processes have all exited but
        // aren't reaped yet; that group is ended too.
        match kill_process_group(group, Signal::KILL) {
            Ok(()) | Err(rustix::io::Errno::SRCH | rustix::io::Errno::PERM) => {}
            Err(error) => panic!("couldn't end {}'s link (process group {group:?}): {error}", self.name),
        }
        // The group is gone: the next cut must find the next link's.
        let _ = std::fs::remove_file(&file);
    }
}

/// The host and its machines, each added and linked. Fields drop in order:
/// the host first, so its links end before the machines do.
pub struct Rig {
    /// The admin host daemon.
    pub host: Daemon,
    /// The host's world: its channels, the git remotes every machine shares,
    /// and the host's own conversations.
    pub world: World,
    /// The host's machine events from before any machine was added: what a
    /// wait on a machine's reachability wakes on.
    feed: Mutex<Streaming<pb::Event>>,
    /// The host's PATH, with this rig's `ssh` first. A restart of the host
    /// needs it again, since a start's environment is for that start only.
    path: String,
    /// What the boxes run, for a box put in place of one (`replace_box`).
    build: Build,
    boxes: Vec<Remote>,
}

impl Rig {
    /// A host and one machine per name, added (the host key confirmed) and
    /// linked, with the git remotes `repos` (`owner/name`) made.
    pub async fn start(boxes: &[&'static str], repos: &[(&str, &str)]) -> Self {
        Self::start_with(boxes, repos, Build::Release, &[], &[]).await
    }

    /// As [`Rig::start`], with each machine's daemon the `--admin` test
    /// daemon: the test reads a machine's own state (its feed, its server
    /// info) as the user, and makes an agent's calls with the session's
    /// token from the test process, which stands in for the tab's shell.
    pub async fn start_seen(boxes: &[&'static str], repos: &[(&str, &str)]) -> Self {
        Self::start_with(boxes, repos, Build::Admin, &[], &[]).await
    }

    /// As [`Self::start_seen`], with environment variables for the host's
    /// first start, such as `HUMATA2_GH` for the host's fake `gh`; `{dir}`
    /// in a value stands for the host's folder. The boxes get none.
    #[allow(dead_code)]
    pub async fn start_seen_with_host_env(
        boxes: &[&'static str],
        repos: &[(&str, &str)],
        host_env: &[(&str, &str)],
    ) -> Self {
        Self::start_with(boxes, repos, Build::Admin, host_env, &[]).await
    }

    /// As [`Self::start_seen`], with environment variables for the first start
    /// of each box's daemon, such as `HUMATA2_GH` for a box's fake `gh`. Each
    /// daemon gets only what is listed here; no environment of the test
    /// process changes. The host's daemon gets none.
    pub async fn start_seen_with_env(
        boxes: &[&'static str],
        repos: &[(&str, &str)],
        box_env: &[(&str, &str)],
    ) -> Self {
        Self::start_with(boxes, repos, Build::Admin, &[], box_env).await
    }

    /// As [`Self::start`], with environment variables for the first start of
    /// the host and of each box (a start's environment is for that start
    /// only). With `HUMATA2_STOP_AT`, `HUMATA2_POWER_CUT_ROOTS` or the like
    /// in `host_env` or `box_env`, that side runs the faults daemon (D240).
    /// `{dir}`, `{state}` and `{channels}` in a value stand for the side's
    /// folder, state folder and channels folder (D243).
    pub async fn start_with_env(
        boxes: &[&'static str],
        repos: &[(&str, &str)],
        host_env: &[(&str, &str)],
        box_env: &[(&str, &str)],
    ) -> Self {
        Self::start_with(boxes, repos, Build::Release, host_env, box_env).await
    }

    /// As [`Rig::start`], with the host's `config/stack.toml` holding
    /// `settings` before the host starts, so each box is given it as it is
    /// added (D411).
    pub async fn start_with_stack_settings(
        boxes: &[&'static str],
        repos: &[(&str, &str)],
        settings: &str,
    ) -> Self {
        Self::start_inner(boxes, repos, Build::Release, &[], &[], Some(settings)).await
    }

    async fn start_with(
        boxes: &[&'static str],
        repos: &[(&str, &str)],
        build: Build,
        host_env: &[(&str, &str)],
        box_env: &[(&str, &str)],
    ) -> Self {
        Self::start_inner(boxes, repos, build, host_env, box_env, None).await
    }

    async fn start_inner(
        boxes: &[&'static str],
        repos: &[(&str, &str)],
        build: Build,
        host_env: &[(&str, &str)],
        box_env: &[(&str, &str)],
        stack_settings: Option<&str>,
    ) -> Self {
        let dir = folder();
        if let Some(settings) = stack_settings {
            // The humata home of the host is the folder its state folder is in.
            let file = StackConfig::host_path(dir.path());
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, settings).unwrap();
        }
        let world = World::new(dir.path());
        // The host starts a session of any agent its machines run.
        with_cli_agents(&world.agents, &world.root);
        for (owner, name) in repos {
            world.remote(owner, name);
        }
        let template = format!("file://{}/{{owner}}/{{name}}.git", world.remotes.display());
        let links = dir.path().join("links");
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        shared_executable(
            &bin.join("ssh"),
            &SSH.replace("__SETSID__", &binary("HUMATA2_SETSID").display().to_string()),
        );
        // Nothing of the real home folder: no skills of the person running
        // the tests are read, or sent to a machine.
        let skills = dir.path().join("skills-home");
        std::fs::create_dir_all(&skills).unwrap();
        let mut extra = world.args();
        extra.extend([
            "--tools".to_owned(),
            tools(dir.path()).display().to_string(),
            "--ssh-config".to_owned(),
            dir.path().join("ssh_config").display().to_string(),
            "--skills-home".to_owned(),
            skills.display().to_string(),
        ]);
        let extra: Vec<&str> = extra.iter().map(String::as_str).collect();
        // The daemon's own PATH, given to it alone: this rig's `ssh` first,
        // and nothing of the test process's environment changes.
        let path = format!("{}:/usr/bin:/bin", bin.display());
        let mut launch_env = vec![
            ("PATH".to_owned(), path.clone()),
            (PEER_RETRY.0.to_owned(), PEER_RETRY.1.to_owned()),
            (REVIVE_SETTLE.0.to_owned(), REVIVE_SETTLE.1.to_owned()),
        ];
        if !host_env.is_empty() {
            crate::common::state_folder(dir.path());
            std::fs::create_dir_all(&world.channels).unwrap();
            launch_env.extend(filled(host_env, dir.path(), &world.channels));
        }
        let launch_env: Vec<(&str, &str)> =
            launch_env.iter().map(|(name, value)| (name.as_str(), value.as_str())).collect();
        let starting_boxes = async {
            let mut remotes = Vec::new();
            for &name in boxes {
                remotes.push(Remote::start(name, &links, &template, build, box_env).await);
            }
            remotes
        };
        // Neither needs the other until a box is added, so they start side
        // by side.
        let (remotes, host) = tokio::join!(
            starting_boxes,
            Daemon::start_with_env(Build::Admin, dir, &extra, &launch_env)
        );
        let feed = Self::open_machine_feed(&host).await;
        let rig = Self {
            host,
            world,
            feed: Mutex::new(feed),
            path,
            build,
            boxes: remotes,
        };
        for remote in &rig.boxes {
            rig.add(remote.name).await;
        }
        rig
    }

    async fn open_machine_feed(host: &Daemon) -> Streaming<pb::Event> {
        pb::event_service_client::EventServiceClient::new(host.channel().await)
            .subscribe(pb::SubscribeRequest {
                topics: vec![pb::Topic::Machines as i32],
                ..Default::default()
            })
            .await
            .expect("the host's event feed opens")
            .into_inner()
    }

    pub fn the(&self, name: &str) -> &Remote {
        self.boxes
            .iter()
            .find(|remote| remote.name == name)
            .unwrap_or_else(|| panic!("this rig has no machine {name}"))
    }

    pub fn the_mut(&mut self, name: &str) -> &mut Remote {
        self.boxes
            .iter_mut()
            .find(|remote| remote.name == name)
            .unwrap_or_else(|| panic!("this rig has no machine {name}"))
    }

    /// A connection to the host's socket.
    pub async fn channel(&self) -> Channel {
        self.host.channel().await
    }

    async fn machines(&self) -> pb::machine_service_client::MachineServiceClient<Channel> {
        pb::machine_service_client::MachineServiceClient::new(self.channel().await)
    }

    /// Where the host's `config/stack.toml` is: in the folder its state
    /// folder is in (D411).
    pub fn host_stack_settings(&self) -> PathBuf {
        StackConfig::host_path(
            self.host
                .state
                .parent()
                .expect("the host's state folder is in a folder of its own"),
        )
    }

    /// Writes the host's `config/stack.toml`, replacing what was there.
    pub fn set_host_stack_settings(&self, text: &str) {
        let file = self.host_stack_settings();
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, text).unwrap();
    }

    /// The sha256 the host's tools manifest names for `file` on the boxes'
    /// platform.
    pub fn manifest_sha(&self, file: &str) -> String {
        let path = self.host.dir.path().join("tools/manifest.json");
        let manifest: serde_json::Value = serde_json::from_slice(
            &std::fs::read(&path).unwrap_or_else(|error| panic!("couldn't read {}: {error}", path.display())),
        )
        .unwrap();
        manifest["platforms"]["linux-x86_64"][file]["sha256"]
            .as_str()
            .unwrap_or_else(|| panic!("the tools manifest names no {file} for linux-x86_64: {manifest}"))
            .to_owned()
    }

    /// The host's own machine.
    pub async fn host_id(&self) -> pb::MachineId {
        crate::common::machine_id(&self.host).await
    }

    /// The id of the machine added under `name`.
    pub fn box_id(name: &str) -> pb::MachineId {
        pb::MachineId {
            value: name.to_owned(),
        }
    }

    /// `relative` in `channel`'s folder on the host's disk.
    pub fn host_path(&self, channel: &pb::Channel, relative: &str) -> PathBuf {
        self.world.channels.join(&channel.folder).join(relative)
    }

    /// The conversation file the host's own agent writes for `id`.
    pub fn host_conversation(&self, id: &str) -> PathBuf {
        self.world.conversation(id)
    }

    /// Adds `name`, confirming the host key it presents, and waits for its
    /// link.
    async fn add(&self, name: &str) {
        self.add_as(name, "first").await;
    }

    /// A fresh machine in place of `name`, which the host has removed: the
    /// same ssh alias now reaches a new machine, which is added under the
    /// same name, as a person who reuses the alias does (D783). The old
    /// machine and its folders are gone.
    pub async fn replace_box(&mut self, name: &'static str) {
        let at = self
            .boxes
            .iter()
            .position(|remote| remote.name == name)
            .unwrap_or_else(|| panic!("this rig has no machine {name}"));
        drop(self.boxes.remove(at));
        let links = self.host.dir.path().join("links");
        let template = format!("file://{}/{{owner}}/{{name}}.git", self.world.remotes.display());
        let fresh = Remote::start(name, &links, &template, self.build, &[]).await;
        self.boxes.push(fresh);
        self.add_as(name, "again").await;
    }

    /// Adds `name` again, the same machine with its daemon and holders as
    /// they are, after the host forgot or removed it, and returns once its
    /// link is up.
    #[allow(dead_code)]
    pub async fn add_again(&self, name: &str) {
        self.add_as(name, "again").await;
    }

    /// Adds `name` as [`add`](Self::add) does; `round` keeps each add's
    /// idempotency keys its own.
    async fn add_as(&self, name: &str, round: &str) {
        let mut machines = self.machines().await;
        let request = |step: &str, fingerprint: &str| pb::AddSshMachineRequest {
            idempotency_key: Some(pb::IdempotencyKey {
                value: format!("add-{name}-{round}-{step}"),
            }),
            ssh_host: format!("me@{name}.internal"),
            name: name.to_owned(),
            root: String::new(),
            trust_host_key_fingerprint: fingerprint.to_owned(),
        };
        let first = machines
            .add_ssh_machine(request("ask", ""))
            .await
            .expect("the add starts")
            .into_inner();
        let asked = self.ended(&first).await;
        let Some(pb::error::Kind::HostKeyUnknown(key)) =
            asked.error.clone().and_then(|error| error.kind)
        else {
            panic!("the first add of {name} should stop at its host key; it ended {asked:?}");
        };
        let added = machines
            .add_ssh_machine(request("trust", &key.presented_fingerprint))
            .await
            .expect("the confirmed add starts")
            .into_inner();
        let ended = self.ended(&added).await;
        assert_eq!(
            ended.state,
            pb::OperationState::Succeeded as i32,
            "adding {name} failed: {:?}",
            ended.error
        );
        self.linked(name).await;
    }

    /// The operation as its watch last showed it, once the watch ends.
    pub async fn ended(&self, operation: &pb::Operation) -> pb::Operation {
        let mut watch =
            pb::operation_service_client::OperationServiceClient::new(self.channel().await)
                .watch_operation(pb::WatchOperationRequest {
                    id: operation.id.clone(),
                    from_version: 0,
                })
                .await
                .expect("the operation can be watched")
                .into_inner();
        let deadline = tokio::time::Instant::now() + LONG;
        let mut last = operation.clone();
        loop {
            match tokio::time::timeout_at(deadline, watch.message()).await {
                Ok(Ok(Some(next))) => last = next,
                Ok(Ok(None)) => return last,
                Ok(Err(status)) => {
                    panic!("the operation's watch failed ({status:?}); it last was {last:?}")
                }
                Err(_) => {
                    panic!("the operation never ended within {LONG:?}; it last was {last:?}")
                }
            }
        }
    }

    /// The machine `name` once `reached` holds for it. Nothing sleeps: the
    /// machine is read, and read again each time the feed tells of a
    /// machine. The feed was opened before the read, so no change falls
    /// between a read and the next event.
    async fn machine_until(
        &self,
        name: &str,
        what: &str,
        reached: impl Fn(&pb::Machine) -> bool,
    ) -> pb::Machine {
        let mut feed = self.feed.lock().await;
        let deadline = tokio::time::Instant::now() + LONG;
        loop {
            let now = self
                .machines()
                .await
                .list_machines(pb::ListMachinesRequest::default())
                .await
                .expect("the host lists its machines")
                .into_inner()
                .machines
                .into_iter()
                .find(|machine| machine.name == name);
            if let Some(machine) = now.as_ref().filter(|machine| reached(machine)) {
                return machine.clone();
            }
            match tokio::time::timeout_at(deadline, feed.message()).await {
                Ok(Ok(Some(_))) => {}
                Ok(ended) => panic!(
                    "the host's event feed ended ({ended:?}) while {name} wasn't {what} yet; \
                     the host last showed {now:?}"
                ),
                Err(_) => {
                    panic!("{name} was never {what} within {LONG:?}; the host last showed {now:?}")
                }
            }
        }
    }

    /// Waits until the host shows `name` online with its link up. ONLINE
    /// alone isn't the link: an add shows it once ssh worked, before the
    /// link is dialled. The daemon version is what the link's own handshake
    /// said, and the host takes the link into use before it shows either.
    pub async fn linked(&self, name: &str) -> pb::Machine {
        self.machine_until(name, "online with its link up", |machine| {
            machine.reachability == pb::Reachability::Online as i32
                && !machine.daemon_version.is_empty()
        })
        .await
    }

    /// Ends `name`'s link as a dropped connection does and keeps it away;
    /// returns once the host shows the machine offline. The host dials again
    /// by itself after its backoff. Each dial then waits in the fake `ssh`
    /// (`hang`) until [`Self::link_back`], so the machine stays away for as
    /// long as the test needs, however slow the test is.
    pub async fn link_lost(&self, name: &str) -> pb::Machine {
        let remote = self.the(name);
        remote.machine.set("hang");
        remote.cut();
        self.machine_until(name, "offline", |machine| {
            machine.reachability == pb::Reachability::Offline as i32
        })
        .await
    }

    /// Lets the host's dials reach `name` again, and returns once the host
    /// shows its link up. No reconnect is asked for: the dial the host makes
    /// by itself is what the product promises.
    pub async fn link_back(&self, name: &str) -> pb::Machine {
        self.the(name).machine.clear("hang");
        self.linked(name).await
    }

    /// Ends the host at once, as a crash does; its holders keep running.
    pub async fn kill_host(&mut self) {
        self.host.kill();
    }

    /// Waits until the host's process ends by itself, at a stop point.
    #[allow(dead_code)]
    pub async fn host_stopped(&mut self) {
        self.host.stopped(LONG).await;
    }

    /// Starts the host again on the same state (ending it first if it runs)
    /// with `env` for this start, and returns once every machine's link is
    /// up again. The host's PATH, with this rig's `ssh` first, is always
    /// there: a start's environment is for that start only.
    pub async fn restart_host(&mut self, env: &[(&str, &str)]) {
        self.restart_host_unlinked(env).await;
        // A machine whose daemon was killed has no link to wait for.
        let names: Vec<&'static str> = self.boxes.iter().filter(|remote| !remote.down).map(|remote| remote.name).collect();
        for name in names {
            self.linked(name).await;
        }
    }

    /// As [`Self::restart_host`], returning once the host listens, without
    /// waiting for any link: for a test that keeps a machine away (its
    /// dials held by [`Self::link_lost`]) across the restart.
    pub async fn restart_host_unlinked(&mut self, env: &[(&str, &str)]) {
        let mut all = vec![("PATH", self.path.as_str()), PEER_RETRY, REVIVE_SETTLE];
        all.extend_from_slice(env);
        self.host.kill();
        // The old feed ended with the old host; the new one is open before
        // the host dials, so no machine change falls between.
        self.host.restart_env(&all).await;
        *self.feed.get_mut() = Self::open_machine_feed(&self.host).await;
    }

    /// Ends box `name`'s daemon (if it still runs), waits until the host
    /// shows the machine offline, starts the daemon again on the same state
    /// with `env` for this start only, and returns once the host shows its
    /// link up. With `HUMATA2_STOP_AT`, `HUMATA2_HOLD_AT`,
    /// `HUMATA2_POWER_CUT_ROOTS` or `HUMATA2_FAULT_TRACE` in `env`, the box
    /// runs the faults daemon, without `--admin` (D240); otherwise the
    /// daemon as shipped. Nothing sleeps: the host's feed wakes each wait.
    pub async fn restart_box_env(&mut self, name: &str, env: &[(&str, &str)]) {
        self.the_mut(name).kill();
        self.link_ended(name).await;
        self.the_mut(name).relaunch(env).await;
        self.link_back(name).await;
    }

    /// A machine whose daemon ended has its link's process still up for a
    /// while: the host would show it online, and a call made then would fail
    /// as offline. This ends the link, as the dropped connection of a lost
    /// machine does (a link already gone is fine), keeps the host's dials in
    /// the fake `ssh` until [`Self::link_back`], and returns once the host
    /// shows it offline.
    pub async fn link_ended(&self, name: &str) -> pb::Machine {
        let remote = self.the(name);
        remote.machine.set("hang");
        remote.cut_if_linked();
        self.machine_until(name, "offline", |machine| {
            machine.reachability == pb::Reachability::Offline as i32
        })
        .await
    }

    /// Waits until box `name`'s daemon ends by itself, at a stop point its
    /// last start named, and until the host shows the machine offline.
    pub async fn box_stopped(&mut self, name: &str) {
        self.the_mut(name).stopped().await;
        self.link_ended(name).await;
    }

    /// Lets the host's dials reach `name` again, as [`Self::link_back`]
    /// does, without waiting for the link: for a host that is about to stop
    /// at a point once the link is up.
    pub fn dials_through(&self, name: &str) {
        self.the(name).machine.clear("hang");
    }

    /// A channel of `repos` (`owner/name`) on the machines `on`, created
    /// through the host once its setup has succeeded.
    pub async fn create_channel(
        &self,
        name: &str,
        repos: &[(&str, &str)],
        on: &[pb::MachineId],
    ) -> pb::Channel {
        let made = pb::channel_service_client::ChannelServiceClient::new(self.channel().await)
            .create_channel(pb::CreateChannelRequest {
                idempotency_key: Some(pb::IdempotencyKey {
                    value: format!("create-{name}"),
                }),
                name: name.to_owned(),
                repos: repos
                    .iter()
                    .map(|(owner, name)| crate::common::repo_ref(owner, name))
                    .collect(),
                machines: on.to_vec(),
            })
            .await
            .expect("the channel is created")
            .into_inner();
        let setup = self.ended(&made.setup.expect("the creation has a setup")).await;
        assert_eq!(
            setup.state,
            pb::OperationState::Succeeded as i32,
            "the setup of channel {name} failed: {:?}",
            setup.error
        );
        made.channel.expect("the creation names the channel")
    }

    /// Starts a session titled `title` on `on` and waits for its operation
    /// to succeed.
    pub async fn start_on(
        &self,
        channel: &pb::Channel,
        on: &pb::MachineId,
        title: &str,
    ) -> pb::SessionId {
        let response = pb::session_service_client::SessionServiceClient::new(self.channel().await)
            .start_session(start_request(channel.id.as_ref().unwrap(), on, title))
            .await
            .expect("the session starts")
            .into_inner();
        let ended = self.ended(&response.operation.expect("a start has an operation")).await;
        assert_eq!(
            ended.state,
            pb::OperationState::Succeeded as i32,
            "starting {title} on {on:?} failed: {:?}",
            ended.error
        );
        response.session.expect("a start names its session")
    }

    /// The token the machine `name`'s daemon gave a shell tab of `session`,
    /// which the session's agent holds: the tab is opened on the machine and
    /// its shell prints the token. Nothing sleeps; the wait ends on the
    /// tab's output.
    pub async fn agent_token(&self, name: &str, session: &pb::SessionId) -> String {
        let daemon = self.the(name).daemon();
        let tab = daemon
            .terminals()
            .await
            .open_shell_tab(pb::OpenShellTabRequest {
                idempotency_key: None,
                session: Some(session.clone()),
                repo: None,
                folder: String::new(),
            })
            .await
            .unwrap()
            .into_inner()
            .id
            .unwrap();
        let mut attach = crate::common::Attach::start(daemon, crate::common::open(&tab, 0)).await;
        attach
            .type_text("printf 'TOKEN\\074%s\\076\\n' \"$HUMATA_SESSION_TOKEN\"\r")
            .await;
        attach.wait_until(|attach| printed_token(&attach.screen).is_some()).await;
        printed_token(&attach.screen).unwrap()
    }

    /// The session as the host shows it.
    pub async fn session(&self, id: &pb::SessionId) -> pb::Session {
        pb::session_service_client::SessionServiceClient::new(self.channel().await)
            .get_session(pb::GetSessionRequest {
                id: Some(id.clone()),
            })
            .await
            .expect("the host shows the session")
            .into_inner()
    }

    /// The session once `want` holds for it. Nothing sleeps: it is read, and
    /// read again each time the Sessions feed tells of a change. The feed is
    /// opened before the first read, so no change falls between.
    pub async fn wait_session(
        &self,
        id: &pb::SessionId,
        what: &str,
        want: impl Fn(&pb::Session) -> bool,
    ) -> pb::Session {
        let mut feed = pb::event_service_client::EventServiceClient::new(self.channel().await)
            .subscribe(pb::SubscribeRequest {
                topics: vec![pb::Topic::Sessions as i32],
                ..Default::default()
            })
            .await
            .expect("the host's Sessions feed opens")
            .into_inner();
        let deadline = tokio::time::Instant::now() + LONG;
        loop {
            let now = self.session(id).await;
            if want(&now) {
                return now;
            }
            match tokio::time::timeout_at(deadline, feed.message()).await {
                Ok(Ok(Some(_))) => {}
                Ok(ended) => panic!(
                    "the host's Sessions feed ended ({ended:?}) while session {} wasn't {what}; \
                     it showed {now:?}",
                    id.value
                ),
                Err(_) => panic!(
                    "session {} was never {what} within {LONG:?}; the host last showed {now:?}",
                    id.value
                ),
            }
        }
    }
}
