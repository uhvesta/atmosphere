//! A real `humatad2` in a temp folder, a test agent that answers every line
//! with `got: <line>`, and the means to stop everything as a reboot would
//! and start the daemon again on the same folder.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use humata2_clients::Clients;
use humata2_clients::model::{Agent, MachineId};
use humata2_clients_live::{LiveComments, LiveDaemon, LiveMachines, LiveOptions};
use humata2_fake_machine::FakeMachine;

use crate::leftovers::{CaseClock, Leftovers};
use humata2_grpc::humata::v1 as pb;
use hyper_util::rt::TokioIo;
use sha2::{Digest, Sha256};
use tokio::net::UnixStream;
use tonic::transport::{Channel, Endpoint, Uri};
use tca::{MemoryStorage, SystemClock};
use std::sync::Arc;

const AGENT_NAME: &str = "fake";

/// Why the world couldn't be set up. Tests stop on any of these.
#[derive(Debug, thiserror::Error)]
pub enum WorldError {
    /// A needed file or folder couldn't be made or read.
    #[error("{what}: {source}")]
    Io {
        what: String,
        #[source]
        source: std::io::Error,
    },
    /// A tool path from the test's environment is missing.
    #[error("set {variable} to the binary's path (the bazel target passes it)")]
    NoBinary { variable: &'static str },
    /// git failed while making a remote.
    #[error("git {args} failed: {stderr}")]
    Git { args: String, stderr: String },
    /// The daemon never listened: it closed its ready pipe without a word,
    /// or said nothing in time.
    #[error("humatad2 never started listening on {}: its log says why", socket.display())]
    NeverListened { socket: PathBuf },
    /// Connecting to the daemon failed.
    #[error("couldn't connect to humatad2: {source}")]
    Connect {
        #[source]
        source: humata2_clients_live::ConnectError,
    },
    /// Processes started in the folder are still alive.
    #[error("processes started in {} did not stop", folder.display())]
    WontDie { folder: PathBuf },
    /// A call to the host while linking devbox-1 failed or came back wrong.
    #[error("linking devbox-1: {step} failed: {detail}")]
    Link { step: &'static str, detail: String },
}

/// The second machine of [`World::start_with_machine`].
pub const DEVBOX: &str = "devbox-1";
/// What the host's person types to add it; the daemon names the machine
/// `devbox-1` from it.
const DEVBOX_HOST: &str = "devbox-1.internal";
/// The most a wait on the daemons themselves takes. A safety net, never a
/// pace: each wait ends on a pushed event. The slowest is a link coming
/// back: the host dials again 5 s after a link ends, then 10, 20 and at
/// most 30 s after each dial that fails.
pub const LONG: Duration = Duration::from_secs(150);

fn io(what: impl Into<String>) -> impl FnOnce(std::io::Error) -> WorldError {
    let what = what.into();
    move |source| WorldError::Io { what, source }
}

fn binary(variable: &'static str) -> Result<PathBuf, WorldError> {
    let path = std::env::var_os(variable).ok_or(WorldError::NoBinary { variable })?;
    std::fs::canonicalize(&path).map_err(io(format!("find {}", Path::new(&path).display())))
}

fn git(args: &[&str]) -> Result<(), WorldError> {
    let output = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .output()
        .map_err(io("run git"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(WorldError::Git {
            args: args.join(" "),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

/// Stands in for an agent: answers every line typed with `got: <line>`, and
/// keeps each line in its conversation file.
fn agent_script(conv: &Path) -> String {
    format!(
        r#"CONV_DIR='{}'
mode=$1; id=$2
case "$mode" in
  --session-id) f="$CONV_DIR/$id.jsonl"; : > "$f" ;;
  --resume) f="$CONV_DIR/$id.jsonl" ;;
  --fresh) f="$CONV_DIR/other.jsonl"; : > "$f" ;;
esac
printf '%s' "$HUMATA_SESSION_TOKEN" > "$CONV_DIR/../agent.token.part" && mv "$CONV_DIR/../agent.token.part" "$CONV_DIR/../agent.token"
echo "agent up $mode"
while IFS= read -r line; do echo "got: $line"; printf '%s\n' "$line" >> "$f"; printf w > "$CONV_DIR/../wrote"; done
"#,
        conv.display()
    )
}

/// Like [`agent_script`], but it reports its turn as a real agent's hooks
/// do: `humata turn idle` when it is up and after each answer, `humata turn
/// working` while it answers. `humata` takes the session from
/// `HUMATA_SESSION_TOKEN`, which the daemon sets in the agent's terminal.
fn hooked_agent_script(conv: &Path, socket: &Path, humata: &Path) -> String {
    format!(
        r#"CONV_DIR='{}'
turn() {{ HUMATA_SOCKET='{}' '{}' turn "$1" < /dev/null > /dev/null 2>&1; }}
mode=$1; id=$2
case "$mode" in
  --session-id) f="$CONV_DIR/$id.jsonl"; : > "$f" ;;
  --resume) f="$CONV_DIR/$id.jsonl" ;;
  --fresh) f="$CONV_DIR/other.jsonl"; : > "$f" ;;
esac
echo "agent up $mode"
turn idle
while IFS= read -r line; do
  turn working
  echo "got: $line"; printf '%s\n' "$line" >> "$f"; printf w > "$CONV_DIR/../wrote"
  turn idle
done
"#,
        conv.display(),
        socket.display(),
        humata.display()
    )
}

/// GitHub as the tests see it: logs each call beside itself, answers no
/// open pull requests, opens PR 7, signs in as `octo` and lists the repos
/// in `repos.jsonl` beside itself (one JSON line each, as `World::repo`
/// appends them). While a `signed-out` file sits beside it, listing and the
/// login check fail as a signed-out `gh` does.
const FAKE_GH: &str = r#"#!/bin/sh
here=$(dirname "$0")
echo "$*" >> "$here/gh.log"
case "$1 $2" in
  "pr list") echo '[]' ;;
  "pr create") echo "https://github.com/owner/api/pull/7" ;;
  "api --paginate"|"api user")
    if [ -f "$here/signed-out" ]; then
      echo "To get started with GitHub CLI, please run:  gh auth login" >&2
      exit 4
    fi
    case "$*" in
      "api --paginate user/repos"*) cat "$here/repos.jsonl" 2>/dev/null ;;
      "api user"*) echo octo ;;
      *) echo "fake gh: unexpected $*" >&2; exit 1 ;;
    esac
    ;;
  *) echo "fake gh: unexpected $*" >&2; exit 1 ;;
esac
"#;


/// The files of the tools bundle the host installs on a machine, for
/// linux-x86_64, the fake machine's platform.
const BUNDLE_FILES: [&str; 4] = ["humatad2", "humata2_holder", "humata", "stack"];

/// A tools bundle whose `humatad2` runs the real release binary. The
/// link's copy (`serve --stdio`) is told the state folder and socket of the
/// machine's own daemon, so the link relays to that daemon (D146) and
/// sessions run there.
fn bundle(into: &Path, socket: &Path, state: &Path) -> Result<PathBuf, WorldError> {
    let root = into.join("tools");
    std::fs::create_dir_all(root.join("linux-x86_64")).map_err(io("make the bundle folder"))?;
    let mut entries = serde_json::Map::new();
    for file in BUNDLE_FILES {
        let body = if file == "humatad2" {
            format!(
                "#!/bin/sh\ncase \" $* \" in *\" --stdio \"*) exec '{real}' \"$@\" --state '{state}' --socket '{socket}' ;; esac\nexec '{real}' \"$@\"\n",
                real = binary("HUMATAD2_RELEASE")?.display(),
                state = state.display(),
                socket = socket.display(),
            )
        } else {
            format!("#!/bin/sh\n# {file}\nexit 0\n")
        };
        std::fs::write(root.join("linux-x86_64").join(file), &body)
            .map_err(io("write a bundle file"))?;
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
    std::fs::write(root.join("manifest.json"), manifest.to_string())
        .map_err(io("write the manifest"))?;
    Ok(root)
}

/// devbox-1's agent. Like [`agent_script`] it answers each line with
/// `got: <line>`. The session whose first message is `asker` is the asker:
/// it waits on the fifo `go` (made by [`Devbox::start`]) for the id of a
/// session, then asks for access to type into that session, as an agent
/// does (`humata access ask-ahead`). The call waits for an answer in its
/// output file `asked.json`.
fn devbox_agent_script(root: &Path, conv: &Path, socket: &Path, humata: &Path) -> String {
    format!(
        r#"CONV_DIR='{conv}'; ROOT='{root}'
mode=$1; id=$2
case "$mode" in
  --session-id) f="$CONV_DIR/$id.jsonl"; : > "$f" ;;
  --resume) f="$CONV_DIR/$id.jsonl" ;;
  --fresh) f="$CONV_DIR/other.jsonl"; : > "$f" ;;
esac
echo "agent up $mode"
n=0
while IFS= read -r line; do
  n=$((n + 1))
  if [ "$n" = 1 ] && [ "$line" = asker ]; then
    ( target=$(cat "$ROOT/go"); HUMATA_SOCKET='{socket}' '{humata}' --json access ask-ahead session-type "session:$target" --reason 'to compare the schemas' > "$ROOT/asked.json" 2>&1 ) &
  fi
  echo "got: $line"; printf '%s\n' "$line" >> "$f"
done
"#,
        conv = conv.display(),
        root = root.display(),
        socket = socket.display(),
        humata = humata.display(),
    )
}

/// Starts `command`, a `humatad2 serve`, and returns once it says it
/// listens on `socket` (D398): READY=1 on the `--ready-fd` pipe it is
/// given. A daemon that won't listen closes the pipe without a word.
pub(super) async fn spawn_ready(mut command: Command, socket: &Path) -> Result<Child, WorldError> {
    use std::io::Read;
    use std::os::fd::{AsRawFd, BorrowedFd};
    use std::os::unix::process::CommandExt;
    let (mut reader, writer) = std::io::pipe().map_err(io("make the ready pipe"))?;
    let fd = writer.as_raw_fd();
    command.arg(format!("--ready-fd={fd}"));
    // SAFETY: only fcntl runs between fork and exec, on the pipe's end
    // the child is meant to keep.
    unsafe {
        command.pre_exec(move || {
            rustix::io::fcntl_setfd(BorrowedFd::borrow_raw(fd), rustix::io::FdFlags::empty())?;
            Ok(())
        });
    }
    let mut child = command.spawn().map_err(io("start humatad2"))?;
    drop(writer);
    // Up to the line's end: the pipe itself may stay open in a process the
    // daemon started before it was ready.
    let heard = tokio::task::spawn_blocking(move || {
        let mut got = Vec::new();
        let mut byte = [0u8; 1];
        while let Ok(1) = reader.read(&mut byte) {
            if byte[0] == b'\n' {
                break;
            }
            got.push(byte[0]);
        }
        got == b"READY=1"
    });
    match tokio::time::timeout(Duration::from_secs(30), heard).await {
        Ok(Ok(true)) => Ok(child),
        _ => {
            let _ = child.kill();
            let _ = child.wait();
            Err(WorldError::NeverListened {
                socket: socket.to_owned(),
            })
        }
    }
}

/// Kills every process whose command line holds `folder` and returns once
/// each has exited, as the kernel tells (kqueue's process exit), not on a
/// clock. A process that starts meanwhile is found on the next round.
fn kill_all_in(folder: &Path, deadline: Instant) -> Result<(), WorldError> {
    use rustix::event::kqueue::{Event, EventFilter, EventFlags, ProcessEvents, kevent, kqueue};
    use rustix::buffer::spare_capacity;
    use rustix::process::{Pid, Signal, kill_process};
    let wont_die = || WorldError::WontDie {
        folder: folder.to_owned(),
    };
    let queue = kqueue().map_err(|error| WorldError::Io {
        what: "make a kqueue".to_owned(),
        source: error.into(),
    })?;
    loop {
        let found = Command::new("pgrep")
            .arg("-f")
            .arg(folder)
            .output()
            .map_err(io("run pgrep"))?;
        let pids: Vec<Pid> = String::from_utf8_lossy(&found.stdout)
            .lines()
            .filter_map(|line| line.trim().parse::<i32>().ok())
            .filter_map(Pid::from_raw)
            .collect();
        if pids.is_empty() {
            return Ok(());
        }
        let mut waiting = 0usize;
        for pid in &pids {
            // Watched first, so an exit between the kill and the watch
            // isn't missed; a pid already gone can't be watched or waited on.
            // RECEIPT makes the add report into its own slot, with data 0
            // when the watch took, and leaves earlier exits queued for the
            // wait below.
            let watch = Event::new(
                EventFilter::Proc {
                    pid: *pid,
                    flags: ProcessEvents::EXIT,
                },
                EventFlags::ADD | EventFlags::ONESHOT | EventFlags::RECEIPT,
                std::ptr::null_mut(),
            );
            let mut receipt: Vec<Event> = Vec::with_capacity(1);
            // SAFETY: the event carries no user pointer.
            let watched = unsafe {
                kevent(&queue, &[watch], spare_capacity(&mut receipt), Some(Duration::ZERO))
            }
            .is_ok_and(|got| got == 1 && receipt.first().is_some_and(|event| event.data() == 0));
            let _ = kill_process(*pid, Signal::KILL);
            if watched {
                waiting += 1;
            }
        }
        while waiting > 0 {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(wont_die());
            }
            let mut events = Vec::with_capacity(waiting);
            // SAFETY: as above.
            let got = unsafe { kevent(&queue, &[], spare_capacity(&mut events), Some(left)) }.map_err(|error| {
                WorldError::Io {
                    what: "wait for processes to exit".to_owned(),
                    source: error.into(),
                }
            })?;
            waiting = waiting.saturating_sub(got);
        }
        if Instant::now() >= deadline {
            return Err(wont_die());
        }
    }
}

/// Clients on the live daemon listening on `socket`, with fakes for what has
/// no real client yet, as the app starts them. `live_machines` is the real
/// machines client; without it the fake one.
pub(super) async fn live_clients(socket: PathBuf, live_machines: bool) -> Result<Clients, WorldError> {
    let daemon = LiveDaemon::connect(LiveOptions::new(socket).with_agent(Agent::Claude, AGENT_NAME))
        .await
        .map_err(|source| WorldError::Connect { source })?;
    // Built before `daemon` moves into the terminals client, as the app does.
    let file_manager = humata2_clients_live::LiveFileManager::new(&daemon);
    let machines: Arc<dyn humata2_clients::machines::Machines> = if live_machines {
        Arc::new(LiveMachines::new(&daemon))
    } else {
        Arc::new(humata2_clients::machines::FakeMachines::new())
    };
    Ok(Clients {
        daemon: Arc::new(daemon.clone()),
        desk: Arc::new(daemon.clone()),
        comments: Arc::new(LiveComments::new(&daemon)),
        history: Arc::new(humata2_clients_live::history::LiveHistory::new(&daemon)),
        marks: Arc::new(humata2_clients_live::marks::LiveMarks::new(&daemon)),
        views: Arc::new(humata2_clients_live::LiveViews::new(&daemon)),
        pull_requests: Arc::new(humata2_clients_live::LivePullRequests::new(&daemon)),
        review_chat: Arc::new(humata2_clients_live::LiveReviewChat::new(&daemon)),
        terminals: Arc::new(daemon),
        clock: Arc::new(SystemClock::new()),
        clipboard: Arc::new(humata2_clients::FakeClipboard::default()),
        reveal: Arc::new(humata2_clients::FakeReveal::default()),
        files: Arc::new(humata2_clients::FakeFileSystem::default()),
        file_manager: Arc::new(file_manager),
        keychain: Arc::new(humata2_clients::FakeKeychain::default()),
        local_daemon: Arc::new(humata2_clients::FakeLocalDaemon::default()),
        machines,
        storage: Arc::new(MemoryStorage::new()),
    })
}

/// A gRPC channel to the daemon listening on `path`.
async fn channel_to(path: PathBuf) -> Channel {
    Endpoint::from_static("http://[::]:50051")
        .connect_with_connector(tower::service_fn(move |_: Uri| {
            let path = path.clone();
            async move { Ok::<_, std::io::Error>(TokioIo::new(UnixStream::connect(path).await?)) }
        }))
        .await
        .expect("the daemon's socket accepts a connection")
}

/// devbox-1: a fake machine reached over a fake `ssh`, with its own
/// `humatad2` in the machine role (the release binary) and its own agent,
/// which the host's link relays to. Its daemon, holders and agents die with
/// it.
pub struct Devbox {
    machine: FakeMachine,
    child: Option<Child>,
    dir: tempfile::TempDir,
    socket: PathBuf,
    state: PathBuf,
}

impl Devbox {
    /// Starts the machine's daemon and returns once it listens. Its clones
    /// come from `template`, the host's remote template.
    async fn start(template: &str) -> Result<Self, WorldError> {
        use std::os::unix::fs::PermissionsExt;
        let machine = FakeMachine::new();
        let dir = tempfile::Builder::new()
            .prefix("h2db")
            .tempdir_in("/tmp")
            .map_err(io("make devbox-1's folder"))?;
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))
            .map_err(io("restrict devbox-1's folder"))?;
        let root = dir.path();
        let conv = root.join("conv");
        std::fs::create_dir_all(&conv).map_err(io("make devbox-1's conversations folder"))?;
        let socket = root.join("m.sock");
        let script = root.join("agent.sh");
        std::fs::write(
            &script,
            devbox_agent_script(root, &conv, &socket, &binary("HUMATA")?),
        )
        .map_err(io("write devbox-1's agent"))?;
        let agents = root.join("agents.json");
        let spec = serde_json::json!([{
            "name": AGENT_NAME,
            "program": "/bin/sh",
            "start_args": [script, "--session-id", "{conversation}"],
            "resume_args": [script, "--resume", "{conversation}"],
            "conversation_file": format!("{}/{{conversation}}.jsonl", conv.display()),
        }]);
        std::fs::write(&agents, spec.to_string()).map_err(io("write devbox-1's agents.json"))?;
        // The machine's `~/.humata2` is a short folder: a holder's socket
        // must fit a socket address, and the fake machine's home is long.
        let short = root.join("h");
        std::fs::create_dir(&short).map_err(io("make devbox-1's state parent"))?;
        std::os::unix::fs::symlink(&short, machine.home().join(".humata2"))
            .map_err(io("link devbox-1's .humata2"))?;
        let state = short.join("state");
        // The asker's fifo is there before any agent runs, so writing to it
        // blocks until the asker reads, and nothing is looked at meanwhile.
        let made = Command::new("/usr/bin/mkfifo")
            .arg(root.join("go"))
            .status()
            .map_err(io("make devbox-1's fifo"))?;
        if !made.success() {
            return Err(WorldError::Io {
                what: "make devbox-1's fifo".to_owned(),
                source: std::io::Error::other(format!("mkfifo exited with {made}")),
            });
        }
        // The admin build, admin for this test process only: the test plays
        // the app on devbox-1 and can read its Inbox and link feeds, while
        // every `humata` it starts is judged as the release daemon judges it.
        let mut command = Command::new(binary("HUMATAD2")?);
        command
            .args(["serve", "--role", "machine"])
            .arg(format!("--admin-pid={}", std::process::id()))
            .arg("--socket")
            .arg(&socket)
            .arg("--state")
            .arg(&state)
            .arg("--root")
            .arg(machine.home())
            .arg("--holder")
            .arg(binary("HUMATA2_HOLDER")?)
            .arg("--shell")
            .arg("/bin/sh")
            .arg("--shell-arg=-i")
            .arg("--channels")
            .arg(machine.home().join("Humata"))
            .arg("--agents")
            .arg(&agents)
            .arg("--default-agent")
            .arg(AGENT_NAME)
            .arg("--remote-template")
            .arg(template)
            .env("HOME", machine.home());
        let child = spawn_ready(command, &socket).await?;
        Ok(Self {
            machine,
            child: Some(child),
            dir,
            socket,
            state,
        })
    }

    /// The socket of devbox-1's own daemon: where `humata` run on devbox-1
    /// connects.
    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// devbox-1's folders: its own and the fake machine's (its fake `ssh`
    /// and home).
    fn folders(&self) -> Vec<&Path> {
        let mut folders = vec![self.dir.path()];
        folders.extend(self.machine.ssh_dir().parent());
        folders
    }

    /// devbox-1's folder: the agent's fifo and output are in it.
    pub fn folder(&self) -> &Path {
        self.dir.path()
    }

    /// A gRPC channel to devbox-1's own daemon, the test being its app.
    pub async fn channel(&self) -> Channel {
        channel_to(self.socket.clone()).await
    }

    /// Ends the host's link to devbox-1 as a dropped connection does, and
    /// keeps the host's dials in the fake `ssh` until [`Self::restore_link`].
    /// The flag is set first, so the host's next dial can't get through.
    pub fn cut_link(&self) {
        self.machine.set("hang");
        // The link's copy of humatad2 carries this machine's socket in its
        // command line; nothing else does.
        let pattern = format!("serve --stdio .*--socket {}", self.socket.display());
        let _ = Command::new("pkill").args(["-f", &pattern]).status();
    }

    /// Lets the host's dials reach devbox-1 again. No reconnect is asked
    /// for: the dial the host makes by itself is what the product promises.
    pub fn restore_link(&self) {
        self.machine.clear("hang");
    }
}

impl Drop for Devbox {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        // The holders and agents it started carry its folder in their
        // command lines.
        let _ = Command::new("pkill").arg("-f").arg(self.dir.path()).status();
    }
}

pub struct World {
    /// Dropped first: devbox-1 dies before the host's folder goes.
    devbox: Option<Devbox>,
    dir: tempfile::TempDir,
    daemon: Option<Child>,
    /// The agent reports its turn through `humata turn`, as a real
    /// agent's hooks do (see [`World::start_with_turn_hooks`]).
    turn_hooks: bool,
    /// The fifo the agents write to after each conversation write.
    wrote: Wrote,
    /// Everything started from this world's folders, ended with it.
    leftovers: Leftovers,
    /// Dropped last, so the time it tells includes the world's end.
    clock: CaseClock,
}

/// D399: the push for the conversations. Each agent writes a byte to the
/// fifo `wrote` after it adds to its conversation file, and a thread wakes
/// the test's thread (parked in a wait) and anyone awaiting
/// [`World::wrote`]. The fifo is held open both ways here, so an agent's
/// write never waits and the thread never sees its end.
struct Wrote {
    fifo: std::fs::File,
    news: Arc<tokio::sync::Notify>,
}

impl Wrote {
    fn open(root: &Path) -> Result<Self, WorldError> {
        use std::io::Read;
        let path = root.join("wrote");
        let made = Command::new("/usr/bin/mkfifo")
            .arg(&path)
            .status()
            .map_err(io("make the wrote fifo"))?;
        if !made.success() {
            return Err(WorldError::Io {
                what: "make the wrote fifo".to_owned(),
                source: std::io::Error::other(format!("mkfifo exited with {made}")),
            });
        }
        let fifo = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(io("open the wrote fifo"))?;
        let mut reader = fifo.try_clone().map_err(io("open the wrote fifo"))?;
        let news = Arc::new(tokio::sync::Notify::new());
        let thread = std::thread::current();
        let told = news.clone();
        std::thread::spawn(move || {
            let mut byte = [0u8; 1];
            while let Ok(1) = reader.read(&mut byte) {
                if byte[0] == b'q' {
                    return;
                }
                told.notify_one();
                thread.unpark();
            }
        });
        Ok(Self {
            fifo,
            news,
        })
    }
}

impl Drop for Wrote {
    fn drop(&mut self) {
        use std::io::Write;
        let _ = self.fifo.write_all(b"q");
    }
}

impl World {
    /// Makes the folder and starts the daemon in it.
    pub async fn start() -> Result<Self, WorldError> {
        Self::start_in(false, false).await
    }

    /// Like [`World::start`], and the host has a second machine, devbox-1,
    /// added over a fake `ssh` and linked: its own `humatad2` in the machine
    /// role and its own agent. The clients use the real machines client.
    /// Returns once the host shows devbox-1 online with its link up.
    pub async fn start_with_machine() -> Result<Self, WorldError> {
        Self::start_in(false, true).await
    }

    /// Like [`World::start`], but the agent reports its turn the way a
    /// real agent's hooks do (SE-21, D169): `humata turn working` while it
    /// answers a line, `humata turn idle` once it has answered and when it
    /// comes up, so the session reads Idle whenever it waits for a line.
    pub async fn start_with_turn_hooks() -> Result<Self, WorldError> {
        Self::start_in(true, false).await
    }

    async fn start_in(turn_hooks: bool, with_machine: bool) -> Result<Self, WorldError> {
        use std::os::unix::fs::PermissionsExt;
        let started = Instant::now();
        let dir = tempfile::Builder::new()
            .prefix("h2f")
            .tempdir_in("/tmp")
            .map_err(io("make a temp folder"))?;
        // The daemon refuses a socket folder other users can open.
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))
            .map_err(io("restrict the temp folder"))?;
        let root = dir.path();
        let conv = root.join("conv");
        std::fs::create_dir_all(&conv).map_err(io("make conversations folder"))?;
        std::fs::create_dir_all(root.join("remotes")).map_err(io("make remotes folder"))?;
        let script = root.join("agent.sh");
        let text = if turn_hooks {
            hooked_agent_script(&conv, &root.join("d.sock"), &binary("HUMATA")?)
        } else {
            agent_script(&conv)
        };
        std::fs::write(&script, text).map_err(io("write the agent"))?;
        let mut kind = serde_json::json!({
            "name": AGENT_NAME,
            "program": "/bin/sh",
            "start_args": [script, "--session-id", "{conversation}"],
            "resume_args": [script, "--resume", "{conversation}"],
            "conversation_file": format!("{}/{{conversation}}.jsonl", conv.display()),
        });
        if turn_hooks {
            kind["turn_end"] = serde_json::json!("hooks");
        }
        let spec = serde_json::json!([kind]);
        std::fs::write(root.join("agents.json"), spec.to_string())
            .map_err(io("write agents.json"))?;
        // GitHub is a fake `gh` (the admin daemon reads HUMATA2_GH): it logs
        // each call to gh.log, finds no open pull request and opens PR 7.
        let gh = root.join("gh");
        std::fs::write(&gh, FAKE_GH).map_err(io("write the fake gh"))?;
        std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755))
            .map_err(io("make the fake gh runnable"))?;
        let devbox = if with_machine {
            let template = format!(
                "file://{}/remotes/{{owner}}/{{name}}.git",
                dir.path().display()
            );
            Some(Devbox::start(&template).await?)
        } else {
            None
        };
        let wrote = Wrote::open(dir.path())?;
        let (leftovers, clock) = {
            let mut dirs = vec![dir.path()];
            if let Some(devbox) = &devbox {
                dirs.extend(devbox.folders());
            }
            (Leftovers::watch(&dirs), CaseClock::start(started, &dirs))
        };
        let mut world = Self {
            devbox,
            dir,
            daemon: None,
            turn_hooks,
            wrote,
            leftovers,
            clock,
        };
        world.spawn_daemon().await?;
        if world.devbox.is_some() {
            world.add_devbox().await?;
        }
        world.clock.up();
        Ok(world)
    }

    /// devbox-1, for a world made by [`World::start_with_machine`].
    pub fn devbox(&self) -> &Devbox {
        self.devbox
            .as_ref()
            .expect("the world was made with start_with_machine")
    }

    /// A gRPC channel to the host's daemon.
    pub async fn host_channel(&self) -> Channel {
        let path = self.socket();
        Endpoint::from_static("http://[::]:50051")
            .connect_with_connector(tower::service_fn(move |_: Uri| {
                let path = path.clone();
                async move { Ok::<_, std::io::Error>(TokioIo::new(UnixStream::connect(path).await?)) }
            }))
            .await
            .expect("the host's socket accepts a connection")
    }

    fn link_error(step: &'static str, detail: impl std::fmt::Debug) -> WorldError {
        WorldError::Link {
            step,
            detail: format!("{detail:?}"),
        }
    }

    /// Adds devbox-1 as a person does: the first add stops at the host key,
    /// the second confirms it. Returns once the host shows the machine
    /// online with its link up (a daemon version is what the link's own
    /// handshake said).
    async fn add_devbox(&self) -> Result<(), WorldError> {
        let channel = self.host_channel().await;
        let mut feed = pb::event_service_client::EventServiceClient::new(channel.clone())
            .subscribe(pb::SubscribeRequest {
                topics: vec![pb::Topic::Machines as i32],
                ..Default::default()
            })
            .await
            .map_err(|status| Self::link_error("open the machines feed", status))?
            .into_inner();
        let mut machines = pb::machine_service_client::MachineServiceClient::new(channel.clone());
        let request = |step: &str, fingerprint: &str| pb::AddSshMachineRequest {
            idempotency_key: Some(pb::IdempotencyKey {
                value: format!("add-{DEVBOX}-{step}"),
            }),
            ssh_host: DEVBOX_HOST.to_owned(),
            name: DEVBOX.to_owned(),
            root: String::new(),
            trust_host_key_fingerprint: fingerprint.to_owned(),
        };
        let first = machines
            .add_ssh_machine(request("ask", ""))
            .await
            .map_err(|status| Self::link_error("start the add", status))?
            .into_inner();
        let asked = Self::operation_ended(&channel, &first).await?;
        let Some(pb::error::Kind::HostKeyUnknown(key)) =
            asked.error.clone().and_then(|error| error.kind)
        else {
            return Err(Self::link_error("the first add stops at the host key", asked));
        };
        let added = machines
            .add_ssh_machine(request("trust", &key.presented_fingerprint))
            .await
            .map_err(|status| Self::link_error("start the confirmed add", status))?
            .into_inner();
        let ended = Self::operation_ended(&channel, &added).await?;
        if ended.state != pb::OperationState::Succeeded as i32 {
            return Err(Self::link_error("the confirmed add succeeds", ended));
        }
        // Listed, and listed again each time the feed tells of a change. The
        // feed was opened before the add, so no change falls between.
        let deadline = tokio::time::Instant::now() + LONG;
        loop {
            let listed = machines
                .list_machines(pb::ListMachinesRequest::default())
                .await
                .map_err(|status| Self::link_error("list the machines", status))?
                .into_inner()
                .machines;
            let linked = listed.iter().any(|machine| {
                machine.name == DEVBOX
                    && machine.reachability == pb::Reachability::Online as i32
                    && !machine.daemon_version.is_empty()
            });
            if linked {
                return Ok(());
            }
            match tokio::time::timeout_at(deadline, feed.message()).await {
                Ok(Ok(Some(_))) => {}
                other => {
                    return Err(Self::link_error(
                        "the host shows devbox-1 linked",
                        format!("{other:?}; the host last listed {listed:?}"),
                    ));
                }
            }
        }
    }

    /// The operation as its watch last showed it, once the watch ends.
    async fn operation_ended(
        channel: &Channel,
        operation: &pb::Operation,
    ) -> Result<pb::Operation, WorldError> {
        let mut watch = pb::operation_service_client::OperationServiceClient::new(channel.clone())
            .watch_operation(pb::WatchOperationRequest {
                id: operation.id.clone(),
                from_version: 0,
            })
            .await
            .map_err(|status| Self::link_error("watch an operation", status))?
            .into_inner();
        let deadline = tokio::time::Instant::now() + LONG;
        let mut last = operation.clone();
        loop {
            match tokio::time::timeout_at(deadline, watch.message()).await {
                Ok(Ok(Some(next))) => last = next,
                Ok(Ok(None)) => return Ok(last),
                other => {
                    return Err(Self::link_error(
                        "an operation ends",
                        format!("{other:?}; it last was {last:?}"),
                    ));
                }
            }
        }
    }

    /// The daemon's socket.
    pub fn socket(&self) -> PathBuf {
        self.dir.path().join("d.sock")
    }

    /// The session token the agent's tab was started with. The agent writes
    /// it before it reads its first message, so it is there once
    /// `running_session` returns.
    pub fn agent_token(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("agent.token"))
            .expect("the agent wrote its session token when it started")
    }

    /// The folder the agent keeps its conversations in.
    pub fn conversations(&self) -> PathBuf {
        self.dir.path().join("conv")
    }

    /// Wakes once an agent has added to a conversation since the last
    /// wake: what a wait on [`Self::conversation_text`] awaits.
    pub async fn wrote(&self) {
        self.wrote.news.notified().await;
    }

    /// Every conversation's text, joined.
    pub fn conversation_text(&self) -> String {
        let Ok(entries) = std::fs::read_dir(self.conversations()) else {
            return String::new();
        };
        entries
            .flatten()
            .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
            .collect()
    }

    /// Deletes every conversation, as if they had been lost.
    pub fn lose_conversations(&self) -> Result<(), WorldError> {
        for entry in std::fs::read_dir(self.conversations())
            .map_err(io("read conversations"))?
            .flatten()
        {
            std::fs::remove_file(entry.path()).map_err(io("delete a conversation"))?;
        }
        Ok(())
    }

    async fn spawn_daemon(&mut self) -> Result<(), WorldError> {
        let root = self.dir.path().to_path_buf();
        let socket = self.socket();
        let _ = std::fs::remove_file(&socket);
        let mut command = Command::new(binary("HUMATAD2")?);
        command.arg("serve");
        if self.turn_hooks {
            // The agent's hooks run this `humata` (D169).
            command.arg("--humata").arg(binary("HUMATA")?);
        }
        if let Some(devbox) = &self.devbox {
            // devbox-1's tools bundle (its link relays to its own daemon),
            // its fake `ssh` first on the host's PATH, and nothing of the
            // person running the tests: no skills read, no real ~/.ssh.
            let tools = bundle(&root, devbox.socket(), &devbox.state)?;
            std::fs::create_dir_all(root.join("skills-home")).map_err(io("make the skills home"))?;
            command
                .arg("--tools")
                .arg(tools)
                .arg("--ssh-config")
                .arg(root.join("ssh_config"))
                .arg("--skills-home")
                .arg(root.join("skills-home"))
                .env("PATH", format!("{}:/usr/bin:/bin", devbox.machine.ssh_dir().display()));
        }
        command
            .arg("--admin")
            .arg("--channels")
            .arg(root.join("ch"))
            .arg("--agents")
            .arg(root.join("agents.json"))
            .arg("--default-agent")
            .arg(AGENT_NAME)
            .arg("--remote-template")
            .arg(format!(
                "file://{}/remotes/{{owner}}/{{name}}.git",
                root.display()
            ))
            .arg("--socket")
            .arg(&socket)
            .arg("--state")
            .arg(root.join("state"))
            .arg("--root")
            .arg(&root)
            .arg("--holder")
            .arg(binary("HUMATA2_HOLDER")?)
            .arg("--shell")
            .arg("/bin/sh")
            .arg("--shell-arg=-i")
            .env("HUMATA2_GH", root.join("gh"))
            .env("PS1", "$ ")
            .env("ENV", "/dev/null");
        self.daemon = Some(spawn_ready(command, &socket).await?);
        Ok(())
    }

    /// Clients on the live daemon, with fakes for what has no real client
    /// yet, as the app starts them.
    pub async fn clients(&self) -> Result<Clients, WorldError> {
        // The real machines client where the world has a second machine to
        // show; the fake otherwise, as before.
        live_clients(self.socket(), self.devbox.is_some()).await
    }

    /// A bare remote `owner/name` on `main` with one commit; returns the
    /// name the daemon clones it by.
    pub fn repo(&self, owner: &str, name: &str) -> Result<String, WorldError> {
        let root = self.dir.path();
        let bare = root.join("remotes").join(owner).join(format!("{name}.git"));
        std::fs::create_dir_all(&bare).map_err(io("make the remote folder"))?;
        let bare_text = bare.display().to_string();
        git(&["init", "--bare", "--initial-branch=main", &bare_text])?;
        let work = root.join(format!("scratch-{owner}-{name}"));
        std::fs::create_dir_all(&work).map_err(io("make the scratch folder"))?;
        let work_text = work.display().to_string();
        git(&["init", "--initial-branch=main", &work_text])?;
        std::fs::write(work.join("README.md"), format!("{owner}/{name}\n"))
            .map_err(io("write the readme"))?;
        git(&["-C", &work_text, "add", "."])?;
        git(&["-C", &work_text, "commit", "-m", "first"])?;
        git(&["-C", &work_text, "push", &bare_text, "main"])?;
        // The fake gh sees every repo the world makes.
        use std::io::Write as _;
        let mut list = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(root.join("repos.jsonl"))
            .map_err(io("open the fake gh's repo list"))?;
        let line = serde_json::json!({
            "full_name": format!("{owner}/{name}"),
            "description": null,
            "private": false,
            "default_branch": "main",
            "pushed_at": "2026-11-15T10:00:00Z",
        });
        writeln!(list, "{line}").map_err(io("add the repo to the fake gh's list"))?;
        Ok(format!("{owner}/{name}"))
    }

    /// The folder the daemon keeps its channels in: one folder per channel,
    /// with `main/<repo>` clones, `sessions` and `shared`.
    pub fn channels_folder(&self) -> PathBuf {
        self.dir.path().join("ch")
    }

    /// Makes the fake `gh` act signed out: listing repos and the login
    /// check fail until the world is dropped.
    pub fn sign_gh_out(&self) -> Result<(), WorldError> {
        std::fs::write(self.dir.path().join("signed-out"), "").map_err(io("sign gh out"))
    }

    /// The branches the bare remote `owner/name` has.
    pub fn remote_branches(&self, owner: &str, name: &str) -> Vec<String> {
        let bare = self
            .dir
            .path()
            .join("remotes")
            .join(owner)
            .join(format!("{name}.git"));
        std::process::Command::new("git")
            .arg("-C")
            .arg(&bare)
            .args(["for-each-ref", "--format=%(refname:short)", "refs/heads"])
            .output()
            .map(|out| {
                String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Each call the app's daemon made to `gh`, one line each.
    pub fn gh_calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.dir.path().join("gh.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// What a reboot does: the daemon and every holder and agent started in
    /// this folder die at once. Nothing is cleaned up first.
    pub async fn reboot(&mut self) -> Result<(), WorldError> {
        if let Some(mut child) = self.daemon.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let folder = self.dir.path().to_path_buf();
        // Holders keep running when the daemon dies, and each has the
        // folder in its command line.
        let deadline = Instant::now() + Duration::from_secs(15);
        tokio::task::spawn_blocking(move || kill_all_in(&folder, deadline))
            .await
            .map_err(|error| WorldError::Io {
                what: "wait for the folder's processes to die".to_owned(),
                source: std::io::Error::other(error),
            })??;
        self.spawn_daemon().await
    }

    /// The first machine of the workspace: This Mac.
    pub async fn this_mac(clients: &Clients) -> Option<MachineId> {
        let workspace = clients.daemon.load().await.ok()?;
        workspace.machines.first().map(|machine| machine.id.clone())
    }
}

impl Drop for World {
    fn drop(&mut self) {
        if let Some(mut child) = self.daemon.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        drop(self.devbox.take());
        // Holders, their shells and what runs in them (a `yes` flood), and
        // fake `ssh` waits: none outlives the test.
        let left = self.leftovers.end();
        if let Err(left) = left
            && !std::thread::panicking()
        {
            panic!(
                "these processes the test started were still running after it was killed; \
                 find what keeps starting them, or what holds them in the kernel (ps state U):\n{}",
                left.join("\n")
            );
        }
    }
}

/// A `git` that stands in front of the real one for the daemon: a `clone`
/// reads one line from the `release` pipe beside it before it starts, so it
/// waits until the test writes that line. Every other call goes straight
/// through.
const HOLDING_GIT: &str = r#"#!/bin/sh
here=$(dirname "$0")
for arg in "$@"; do
  if [ "$arg" = clone ]; then
    read -r _ < "$here/release"
    break
  fi
done
PATH=${PATH#"$here":}
exec git "$@"
"#;

impl World {
    /// Like [`World::start`], but the daemon's `git clone` waits until
    /// [`World::release_clones`] is called. The test says when, so no timing
    /// is involved.
    pub async fn start_holding_clones() -> Result<Self, WorldError> {
        use std::os::unix::fs::PermissionsExt;
        let mut world = Self::start().await?;
        // Replace the daemon `start` made with one that finds the holding
        // `git` first on its PATH. Nothing has used it yet.
        if let Some(mut child) = world.daemon.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let root = world.dir.path().to_path_buf();
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).map_err(io("make the holding git's folder"))?;
        let git = bin.join("git");
        std::fs::write(&git, HOLDING_GIT).map_err(io("write the holding git"))?;
        std::fs::set_permissions(&git, std::fs::Permissions::from_mode(0o755))
            .map_err(io("make the holding git runnable"))?;
        let made = Command::new("mkfifo")
            .arg(bin.join("release"))
            .status()
            .map_err(io("make the holding git's release pipe"))?;
        if !made.success() {
            return Err(WorldError::Io {
                what: "make the holding git's release pipe".to_owned(),
                source: std::io::Error::other(format!("mkfifo exited with {made}")),
            });
        }
        let path = match std::env::var_os("PATH") {
            Some(path) => format!("{}:{}", bin.display(), path.to_string_lossy()),
            None => bin.display().to_string(),
        };
        let socket = world.socket();
        let _ = std::fs::remove_file(&socket);
        let child = Command::new(binary("HUMATAD2")?)
            .arg("serve")
            .arg("--admin")
            .arg("--channels")
            .arg(root.join("ch"))
            .arg("--agents")
            .arg(root.join("agents.json"))
            .arg("--default-agent")
            .arg(AGENT_NAME)
            .arg("--remote-template")
            .arg(format!(
                "file://{}/remotes/{{owner}}/{{name}}.git",
                root.display()
            ))
            .arg("--socket")
            .arg(&socket)
            .arg("--state")
            .arg(root.join("state"))
            .arg("--root")
            .arg(&root)
            .arg("--holder")
            .arg(binary("HUMATA2_HOLDER")?)
            .arg("--shell")
            .arg("/bin/sh")
            .arg("--shell-arg=-i")
            .env("HUMATA2_GH", root.join("gh"))
            .env("PS1", "$ ")
            .env("ENV", "/dev/null")
            .env("PATH", path)
            .spawn()
            .map_err(io("start humatad2 with the holding git"))?;
        world.daemon = Some(child);
        let deadline = Instant::now() + Duration::from_secs(30);
        while tokio::net::UnixStream::connect(&socket).await.is_err() {
            if Instant::now() >= deadline {
                return Err(WorldError::NeverListened { socket });
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Ok(world)
    }

    /// Lets every held `git clone`, now and later, go ahead: a thread hands
    /// each clone that opens the release pipe its line. The thread ends with
    /// the test process.
    pub fn release_clones(&self) {
        let pipe = self.dir.path().join("bin").join("release");
        std::thread::spawn(move || {
            use std::io::Write;
            // Opening for writing waits until a clone opens it to read.
            while let Ok(mut writer) = std::fs::OpenOptions::new().write(true).open(&pipe) {
                // A clone that has just read its line may close the pipe
                // between the open and the write: the write then fails with
                // a broken pipe. That line was for nobody, so the next
                // clone still needs one: keep going rather than end.
                let _ = writer.write_all(b"go\n");
            }
        });
    }
}
