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

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use humata2_fake_machine::FakeMachine;
use humata2_grpc::humata::v1 as pb;
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

const FILES: [&str; 3] = ["humatad2", "humata2_holder", "humata"];

/// `humatad2` as a machine gets it from the tools bundle.
const WRAPPER: &str = r##"#!/bin/sh
# humatad2 on one of this test's machines: the real one. A link is told the
# state folder and socket of the machine's own daemon, which the test
# started in the folder `~/.humata2` points at. The folder is named by its
# short, real path, because a socket's path must be short.
case " $* " in *" --stdio "*)
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
name=
for arg in "$@"; do
  case "$arg" in
__MACHINES__  esac
done
if [ -z "$name" ]; then
  echo "ssh: none of this test's machines is named in: $*" >&2
  exit 255
fi
control='__CONTROL__'/$name
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
echo "agent up $mode"
while IFS= read -r line; do echo "got: $line"; printf '%s\n' "$line" >> "$f"; done
"#,
        conv.display()
    )
}

/// The agents file of an agent whose conversations are in `conv`, written
/// into `root`; returns the file's path.
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
    file
}

fn executable(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
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
    async fn start(name: &'static str, links: &Path, template: &str) -> Self {
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
        let mut daemon = Daemon::start_with_env(
            Build::Release,
            dir,
            &Self::serve_args(&channels, &agents, template)
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            &[("HOME", machine.home().display().to_string().as_str())],
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
        kill_process_group(group, Signal::KILL).unwrap_or_else(|error| {
            panic!("couldn't end {}'s link (process group {group:?}): {error}", self.name)
        });
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
    boxes: Vec<Remote>,
}

impl Rig {
    /// A host and one machine per name, added (the host key confirmed) and
    /// linked, with the git remotes `repos` (`owner/name`) made.
    pub async fn start(boxes: &[&'static str], repos: &[(&str, &str)]) -> Self {
        let dir = folder();
        let world = World::new(dir.path());
        for (owner, name) in repos {
            world.remote(owner, name);
        }
        let template = format!("file://{}/{{owner}}/{{name}}.git", world.remotes.display());
        let links = dir.path().join("links");
        let mut remotes = Vec::new();
        for &name in boxes {
            remotes.push(Remote::start(name, &links, &template).await);
        }
        let machines: String = remotes
            .iter()
            .map(|remote| {
                format!(
                    "    '{name}.internal') name='{name}'; ssh='{}/ssh' ;;\n",
                    remote.machine.ssh_dir().display(),
                    name = remote.name
                )
            })
            .collect();
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        executable(
            &bin.join("ssh"),
            &SSH.replace("__MACHINES__", &machines)
                .replace("__CONTROL__", &links.display().to_string())
                .replace("__SETSID__", &binary("HUMATA2_SETSID").display().to_string()),
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
        let host = Daemon::start_with_env(Build::Admin, dir, &extra, &[("PATH", path.as_str())]).await;
        let feed = Self::open_machine_feed(&host).await;
        let rig = Self {
            host,
            world,
            feed: Mutex::new(feed),
            path,
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
        let mut machines = self.machines().await;
        let request = |step: &str, fingerprint: &str| pb::AddSshMachineRequest {
            idempotency_key: Some(pb::IdempotencyKey {
                value: format!("add-{name}-{step}"),
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
        let Some(pb::error::Kind::HostKeyChanged(key)) =
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
        let mut all = vec![("PATH", self.path.as_str())];
        all.extend_from_slice(env);
        self.host.kill();
        // The old feed ended with the old host; the new one is open before
        // the host dials, so no machine change falls between.
        self.host.restart_env(&all).await;
        *self.feed.get_mut() = Self::open_machine_feed(&self.host).await;
        // A machine whose daemon was killed has no link to wait for.
        let names: Vec<&'static str> = self.boxes.iter().filter(|remote| !remote.down).map(|remote| remote.name).collect();
        for name in names {
            self.linked(name).await;
        }
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
