//! SPEC BD-15 and BD-9 (frame 22, an agent's move of the person's item
//! waits as a request on the item's card) against a real `humatad2`. The
//! person makes a channel item and moves it to In progress; the agent is a
//! shell script in the session's agent tab that, on one typed line, runs
//! the real `humata board move <item> done` from inside its tab and waits
//! for the answer. The person opens the board, presses the card's Review,
//! and answers from the item's drawer. The app is headless: no window is
//! shown.
//!
//! Each test makes its own daemon in its own temp folder.
#![cfg(target_os = "macos")]

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui_kit::{
    AnyWindowHandle, Bounds, HeadlessAppContext, Role, WindowBounds, WindowOptions, point, px, size,
};
use humata2_app::{AppAction, Overlay, Route};
use humata2_clients::model::{
    Agent, ChannelId, Item, ItemId, ItemStatus, MachineId, Request, Scope, SessionId,
    SessionOptions,
};
use humata2_clients::{Clients, Daemon, Event};
use humata2_clients_live::{LiveComments, LiveDaemon, LiveOptions};
use humata2_engine::Resources;
use humata2_flow::{Action, Driver as _, FrameMode, GpuiDriver, Target};
use humata2_ui::bridge::Ui;
use humata2_ui::record::Recorder;
use tca::{MemoryStorage, SystemClock};
use tokio::runtime::Runtime;

const DEADLINE: Duration = Duration::from_secs(60);

/// The daemon's name for the test agent.
const AGENT_NAME: &str = "fake";
/// The line that makes the agent move the item to Done through `humata`.
const CALL_LINE: &str = "move the item to done";
const FIRST_MESSAGE: &str = "hello agent";
const CHANNEL: &str = "payments";
const SESSION: &str = "API rate limiter";
const ITEM_TITLE: &str = "Retry failed charges";
/// The drawer's answers to an agent's ask to move an In progress item to
/// Done (frame 22).
const APPROVE: &str = "Move to Done";
const DENY: &str = "Keep in progress";

/// Why the daemon and its agent couldn't be set up. The test stops on any.
#[derive(Debug, thiserror::Error)]
enum WorldError {
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
    /// The daemon never listened.
    #[error("humatad2 never started listening on {}", socket.display())]
    NeverListened { socket: PathBuf },
    /// Connecting to the daemon failed.
    #[error("couldn't connect to humatad2: {source}")]
    Connect {
        #[source]
        source: humata2_clients_live::ConnectError,
    },
}

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

/// Stands in for an agent. It keeps each typed line in its conversation
/// file. On [`CALL_LINE`] it runs `humata --json --wait 600 board move
/// <item> done` from inside its tab, so the call carries the tab's session
/// token; the item's key is read from `out/item`, which the test writes
/// once the item is made. It leaves what happened in `out`: `started`
/// before the call, then `stdout`, `stderr` and (last, by rename) `code`.
fn agent_script(conv: &Path, out: &Path, wrote: &Path, socket: &Path, humata: &Path) -> String {
    format!(
        r#"CONV_DIR='{conv}'
OUT='{out}'
WROTE='{wrote}'
mode=$1; id=$2
case "$mode" in
  --session-id) f="$CONV_DIR/$id.jsonl"; : > "$f" ;;
  --resume) f="$CONV_DIR/$id.jsonl" ;;
esac
echo "agent up $mode"
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$f"; printf . > "$WROTE"
  case "$line" in
    '{call}')
      if [ -n "$HUMATA_SESSION_TOKEN" ]; then echo yes > "$OUT/started"; else echo no > "$OUT/started"; fi
      printf . > "$WROTE"
      item=$(cat "$OUT/item")
      HUMATA_SOCKET='{socket}' '{humata}' --json --wait 600 board move "$item" done > "$OUT/stdout" 2> "$OUT/stderr" < /dev/null
      printf '%s' "$?" > "$OUT/code.part"
      mv "$OUT/code.part" "$OUT/code"; printf . > "$WROTE"
      ;;
  esac
  echo "got: $line"
done
"#,
        conv = conv.display(),
        out = out.display(),
        wrote = wrote.display(),
        call = CALL_LINE,
        socket = socket.display(),
        humata = humata.display(),
    )
}

/// What the agent left on disk: its conversation and its one call.
#[derive(Clone)]
struct AgentFiles {
    conv: PathBuf,
    out: PathBuf,
}

impl AgentFiles {
    /// Every line typed into the agent, in order.
    fn typed(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(&self.conv) else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
            .flat_map(|text| text.lines().map(str::to_owned).collect::<Vec<_>>())
            .collect()
    }

    fn read(&self, name: &str) -> Option<String> {
        std::fs::read_to_string(self.out.join(name)).ok()
    }

    /// Whether the agent began its call, and whether its tab had a session
    /// token then.
    fn started_with_token(&self) -> Option<bool> {
        self.read("started").map(|text| text.trim() == "yes")
    }

    /// The call's exit code, once it has returned.
    fn exit_code(&self) -> Option<i32> {
        self.read("code")?.trim().parse().ok()
    }

    fn describe(&self) -> String {
        format!(
            "agent was typed {:?}\nagent call: started {:?}, exit code {:?}\n  stdout {:?}\n  stderr {:?}\n",
            self.typed(),
            self.read("started"),
            self.read("code"),
            self.read("stdout"),
            self.read("stderr"),
        )
    }
}

/// A real `humatad2` in a temp folder with the test agent.
/// D442: the push for the agent's files. The agent writes a byte to the
/// fifo `wrote` after each line it keeps and each file it leaves in `out`,
/// and a thread wakes the test's thread (parked in a wait) and anyone
/// awaiting [`World::wrote`]. The fifo is held open both ways here, so an
/// agent's write never waits and the thread never sees its end.
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
        Ok(Self { fifo, news })
    }
}

impl Drop for Wrote {
    fn drop(&mut self) {
        use std::io::Write;
        let _ = self.fifo.write_all(b"q");
    }
}

struct World {
    dir: tempfile::TempDir,
    daemon: Option<Child>,
    /// The fifo the agent writes to after each write of its files.
    wrote: Wrote,
}

impl World {
    /// Makes the folder and starts the daemon in it.
    async fn start() -> Result<Self, WorldError> {
        use std::os::unix::fs::PermissionsExt;
        // A short path: a Unix socket's path has a small limit.
        let dir = tempfile::Builder::new()
            .prefix("h2i")
            .tempdir_in("/tmp")
            .map_err(io("make a temp folder"))?;
        // The daemon refuses a socket folder other users can open.
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))
            .map_err(io("restrict the temp folder"))?;
        let root = dir.path().to_path_buf();
        let wrote = Wrote::open(&root)?;
        let world = Self {
            dir,
            daemon: None,
            wrote,
        };
        let agent = world.agent();
        std::fs::create_dir_all(&agent.conv).map_err(io("make conversations folder"))?;
        std::fs::create_dir_all(&agent.out).map_err(io("make the agent's output folder"))?;
        std::fs::create_dir_all(root.join("remotes")).map_err(io("make remotes folder"))?;
        let script = root.join("agent.sh");
        std::fs::write(
            &script,
            agent_script(&agent.conv, &agent.out, &root.join("wrote"), &world.socket(), &binary("HUMATA")?),
        )
        .map_err(io("write the agent"))?;
        let spec = serde_json::json!([{
            "name": AGENT_NAME,
            "program": "/bin/sh",
            "start_args": [script, "--session-id", "{conversation}"],
            "resume_args": [script, "--resume", "{conversation}"],
            "conversation_file": format!("{}/{{conversation}}.jsonl", agent.conv.display()),
        }]);
        std::fs::write(root.join("agents.json"), spec.to_string())
            .map_err(io("write agents.json"))?;
        let mut world = world;
        world.spawn_daemon().await?;
        Ok(world)
    }

    fn socket(&self) -> PathBuf {
        self.dir.path().join("d.sock")
    }

    /// Wakes once the agent has written one of its files since the last
    /// wake: what a wait on [`AgentFiles`] awaits.
    async fn wrote(&self) {
        self.wrote.news.notified().await;
    }

    fn agent(&self) -> AgentFiles {
        AgentFiles {
            conv: self.dir.path().join("conv"),
            out: self.dir.path().join("out"),
        }
    }

    async fn spawn_daemon(&mut self) -> Result<(), WorldError> {
        let root = self.dir.path().to_path_buf();
        let socket = self.socket();
        let child = Command::new(binary("HUMATAD2")?)
            .arg("serve")
            // Calls with no credential are the person's, as the app's are.
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
            .env("PS1", "$ ")
            .env("ENV", "/dev/null")
            // The agent's call must carry only what its tab was given.
            .env_remove("HUMATA_SESSION_TOKEN")
            .env_remove("HUMATA_SOCKET")
            .spawn()
            .map_err(io("start humatad2"))?;
        self.daemon = Some(child);
        let deadline = Instant::now() + Duration::from_secs(30);
        while tokio::net::UnixStream::connect(&socket).await.is_err() {
            if Instant::now() >= deadline {
                return Err(WorldError::NeverListened { socket });
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Ok(())
    }

    /// Clients on the live daemon, with fakes for what has no real client
    /// yet, as the app starts them.
    async fn clients(&self) -> Result<Clients, WorldError> {
        let daemon = LiveDaemon::connect(
            LiveOptions::new(self.socket()).with_agent(Agent::Claude, AGENT_NAME),
        )
        .await
        .map_err(|source| WorldError::Connect { source })?;
        Ok(Clients {
            daemon: Arc::new(daemon.clone()),
            desk: Arc::new(daemon.clone()),
            comments: Arc::new(LiveComments::new(&daemon)),
            history: Arc::new(humata2_clients_live::history::LiveHistory::new(&daemon)),
            marks: Arc::new(humata2_clients_live::marks::LiveMarks::new(&daemon)),
            views: Arc::new(humata2_clients_live::LiveViews::new(&daemon)),
            pull_requests: Arc::new(humata2_clients_live::LivePullRequests::new(&daemon)),
            review_chat: Arc::new(humata2_clients_live::LiveReviewChat::new(&daemon)),
            todo: Arc::new(humata2_clients_live::todo::LiveTodo::new(&daemon)),
            terminals: Arc::new(daemon),
            clock: Arc::new(SystemClock::new()),
            clipboard: Arc::new(humata2_clients::FakeClipboard::default()),
            reveal: Arc::new(humata2_clients::FakeReveal::default()),
            files: Arc::new(humata2_clients::FakeFileSystem::default()),
            file_manager: Arc::new(humata2_clients::FakeFileManager::default()),
            keychain: Arc::new(humata2_clients::FakeKeychain::default()),
            local_daemon: Arc::new(humata2_clients::FakeLocalDaemon::default()),
            machines: Arc::new(humata2_clients::machines::FakeMachines::new()),
            storage: Arc::new(MemoryStorage::new()),
        })
    }

    /// A bare remote `owner/name` on `main` with one commit; returns the
    /// name the daemon clones it by.
    fn repo(&self, owner: &str, name: &str) -> Result<String, WorldError> {
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
        Ok(format!("{owner}/{name}"))
    }
}

impl Drop for World {
    fn drop(&mut self) {
        if let Some(mut child) = self.daemon.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        // Holders and the agent outlive the daemon; each has the folder in
        // its command line. The agent's waiting `humata` ends with them.
        let _ = Command::new("pkill")
            .args(["-9", "-f"])
            .arg(self.dir.path())
            .status();
    }
}

fn runtime() -> Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("a runtime starts")
}

const RECENT: usize = 40;

/// The last few events the client pushed to the app, kept to explain a
/// timeout.
#[derive(Default)]
struct Heard {
    recent: VecDeque<String>,
}

impl Heard {
    fn hear(&mut self, event: &Event) {
        let summary: String = format!("{event:?}").chars().take(300).collect();
        self.recent.push_back(summary);
        if self.recent.len() > RECENT {
            self.recent.pop_front();
        }
    }

    fn describe(&self) -> String {
        let mut text = String::from("events the client pushed (last first):\n");
        for line in self.recent.iter().rev() {
            text.push_str(&format!("  {line}\n"));
        }
        text
    }
}

/// How the view writes an element's line: its role, then its label.
fn heading(role: Role, label: &str) -> String {
    format!("{role:?} {label:?}")
}

/// The lines of `view` drawn inside the element `role` `label`; `None`
/// when no such element shows.
fn inside(view: &str, role: Role, label: &str) -> Option<String> {
    let heading = heading(role, label);
    let mut lines = view.lines();
    let depth = loop {
        let line = lines.next()?;
        let rest = line.trim_start();
        let states = rest.strip_prefix(heading.as_str());
        if states.is_some_and(|states| states.is_empty() || states.starts_with(' ')) {
            break line.len() - rest.len();
        }
    };
    Some(
        lines
            .take_while(|line| line.len() - line.trim_start().len() > depth)
            .map(|line| format!("{line}\n"))
            .collect(),
    )
}

/// The app, headless, on the clients it was given.
struct App<'r> {
    runtime: &'r Runtime,
    cx: HeadlessAppContext,
    window: AnyWindowHandle,
    ui: Rc<Ui>,
    daemon: Arc<dyn Daemon>,
    heard: Arc<Mutex<Heard>>,
    agent: AgentFiles,
    /// The thread that opened the app: the one gpui wakes when work comes
    /// in from another thread, and the one the waits park (D442).
    thread: std::thread::ThreadId,
}

impl<'r> App<'r> {
    fn open(runtime: &'r Runtime, clients: Clients, agent: AgentFiles) -> Self {
        let daemon = clients.daemon.clone();
        let heard = Arc::new(Mutex::new(Heard::default()));
        {
            let daemon = daemon.clone();
            let heard = heard.clone();
            // Every event from the host wakes a wait: what it looks at may
            // be the host's, not the app's (D442).
            let waiter = std::thread::current();
            runtime.spawn(async move {
                let Ok(mut events) = daemon.events().await else {
                    return;
                };
                while let Some(Ok(event)) = events.recv().await {
                    if let Ok(mut heard) = heard.lock() {
                        heard.hear(&event);
                    }
                    waiter.unpark();
                }
            });
        }
        let mut cx = humata2_engine::headless(&Resources::new(humata2_ui::Assets))
            .expect("the headless context starts");
        cx.update(humata2_ui::init);
        let ui = Ui::new(runtime.handle(), clients, Recorder::new());
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                point(px(0.), px(0.)),
                size(px(humata2_ui::WINDOW.0), px(humata2_ui::WINDOW.1)),
            ))),
            show: false,
            focus: false,
            ..WindowOptions::default()
        };
        let (window, _shell) = cx
            .update(|cx| humata2_ui::open(cx, &ui, options))
            .expect("the window opens");
        let mut app = Self {
            runtime,
            cx,
            window,
            ui,
            daemon,
            heard,
            agent,
            thread: std::thread::current().id(),
        };
        app.settle();
        app
    }

    /// Runs what the last step started, lets the sockets move, and draws a
    /// frame as the app would.
    fn settle(&mut self) {
        for _ in 0..5 {
            self.runtime.block_on(async {
                for _ in 0..16 {
                    tokio::task::yield_now().await;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            });
            self.cx.run_until_parked();
        }
        self.cx
            .update_window(self.window, |_, window, cx| {
                humata2_engine::frame::natural_frame(window, cx);
            })
            .expect("the window stays open");
    }

    fn type_text(&mut self, text: &str) {
        let action = Action::Type(text.to_owned());
        let done = GpuiDriver::with_frames(&mut self.cx, self.window, FrameMode::Natural)
            .perform(&action);
        if let Err(error) = done {
            let why = self.diagnose();
            panic!("{action} failed: {error}\n{why}");
        }
        self.settle();
    }

    /// Moves the pointer over a control that may show only while hovered.
    fn hover(&mut self, role: Role, label: &str) {
        let action = Action::Hover(Target::new(role, label));
        let done = GpuiDriver::with_frames(&mut self.cx, self.window, FrameMode::Natural)
            .perform(&action);
        if let Err(error) = done {
            let why = self.diagnose();
            panic!("{action} failed: {error}\n{why}");
        }
        self.settle();
    }

    fn click(&mut self, role: Role, label: &str) {
        let action = Action::Click(Target::new(role, label));
        let done = GpuiDriver::with_frames(&mut self.cx, self.window, FrameMode::Natural)
            .perform(&action);
        if let Err(error) = done {
            let why = self.diagnose();
            panic!("{action} failed: {error}\n{why}");
        }
        self.settle();
    }

    /// What the window shows, as text.
    fn view(&mut self) -> String {
        GpuiDriver::with_frames(&mut self.cx, self.window, FrameMode::Natural)
            .view()
            .expect("the view can be read")
    }

    /// The requests the app holds that still wait for an answer: what the
    /// sidebar's Inbox row counts.
    fn waiting(&self) -> Vec<Request> {
        self.ui
            .read(|state| state.workspace.waiting().cloned().collect())
    }

    /// Settles until `done` holds or the deadline passes. Between looks the
    /// thread parks until something wakes it (D442): gpui, when another
    /// thread hands the app work; an event from the host; a write of the
    /// agent's. What `done` looks at must come with one of them.
    fn wait_until(&mut self, what: &str, mut done: impl FnMut(&mut Self) -> bool) {
        assert_eq!(
            std::thread::current().id(),
            self.thread,
            "waits run on the thread that opened the app, which is the one woken"
        );
        let until = Instant::now() + DEADLINE;
        loop {
            self.settle();
            if done(self) {
                return;
            }
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                let why = self.diagnose();
                panic!("timed out waiting for {what}\n{why}");
            }
            std::thread::park_timeout(left);
        }
    }

    /// What the app, the client, the daemon and the agent hold now, for a
    /// failure.
    fn diagnose(&mut self) -> String {
        let mut text = self.ui.read(|state| {
            let mut text = format!(
                "route {:?}\npanel {:?}\nload {:?}\ntoasts {:?}\ninbox selected {:?}\ninbox answering {:?}\ninbox error {:?}\n",
                state.route,
                state.panel(),
                state.load,
                state.toasts,
                state.inbox.selected,
                state.inbox.pending,
                state.inbox.error,
            );
            for item in &state.workspace.items {
                text.push_str(&format!(
                    "app item {} {:?} {:?} details {:?}\n",
                    item.id.as_str(),
                    item.title,
                    item.status,
                    state.workspace.item_details.get(&item.id)
                ));
            }
            for request in &state.workspace.requests {
                text.push_str(&format!(
                    "app request {} {:?} from {} answer {:?}\n",
                    request.id.as_str(),
                    request.title,
                    request.session.as_str(),
                    request.answer
                ));
            }
            text
        });
        text.push_str(&self.agent.describe());
        if let Ok(heard) = self.heard.lock() {
            text.push_str(&heard.describe());
        }
        let daemon = self.daemon.clone();
        match self.runtime.block_on(daemon.load()) {
            Ok(workspace) => {
                for request in &workspace.requests {
                    text.push_str(&format!(
                        "client load: request {} {:?} answer {:?}\n",
                        request.id.as_str(),
                        request.title,
                        request.answer
                    ));
                }
                if workspace.requests.is_empty() {
                    text.push_str("client load: no requests\n");
                }
            }
            Err(error) => text.push_str(&format!("client load failed: {error}\n")),
        }
        text.push_str(&format!("view:\n{}\n", self.view()));
        text
    }
}

/// The item `id`'s column as the app holds it, if it holds the item.
fn status_in_app(app: &App<'_>, id: &ItemId) -> Option<ItemStatus> {
    app.ui.read(|state| {
        state
            .workspace
            .items
            .iter()
            .find(|item| &item.id == id)
            .map(|item| item.status)
    })
}

/// A channel with one repo on This Mac and a session in it whose agent has
/// taken its first message, and the person's own item on the channel's
/// board, In progress. The item's key is left where the agent reads it.
async fn item_in_progress(world: &World, clients: &Clients) -> (ChannelId, SessionId, ItemId) {
    let mac: MachineId = clients
        .daemon
        .load()
        .await
        .expect("load the workspace")
        .machines
        .first()
        .map(|machine| machine.id.clone())
        .expect("the workspace has This Mac");
    let repo = world.repo("owner", "ledger").expect("make a repo");
    let channel = clients
        .daemon
        .create_channel(CHANNEL, vec![repo.clone()], vec![mac.clone()])
        .await
        .expect("create the channel");
    // The person makes the item, so an agent's move of it waits for them
    // (BD-9).
    let item = clients
        .daemon
        .create_item(Scope::Channel(channel.clone()), ITEM_TITLE)
        .await
        .expect("the person makes a channel item");
    clients
        .daemon
        .move_item(&item, ItemStatus::InProgress, None)
        .await
        .expect("the person moves the item to In progress");
    let agent = world.agent();
    std::fs::write(agent.out.join("item"), item.as_str())
        .expect("leave the item's key for the agent");
    let session = clients
        .daemon
        .create_session(
            &channel,
            Agent::Claude,
            &mac,
            SESSION,
            SessionOptions {
                repos: vec![repo],
                first_message: FIRST_MESSAGE.to_owned(),
                ..SessionOptions::default()
            },
        )
        .await
        .expect("create the session");
    let until = tokio::time::Instant::now() + DEADLINE;
    while !agent.typed().iter().any(|line| line == FIRST_MESSAGE) {
        let woke = tokio::time::timeout_at(until, world.wrote()).await;
        assert!(
            woke.is_ok(),
            "the agent never got its first message\n{}",
            agent.describe()
        );
    }
    (channel, session, item)
}

/// Sets up the item and the agent's waiting move, opens the board and the
/// item's drawer from its card's Review, and returns the app with the
/// request it shows.
fn ask_open_in_the_drawer<'r>(
    runtime: &'r Runtime,
) -> (World, App<'r>, SessionId, ItemId, Request) {
    let (world, clients, channel, session, item) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        let (channel, session, item) = item_in_progress(&world, &clients).await;
        (world, clients, channel, session, item)
    });
    let daemon = clients.daemon.clone();
    let agent = world.agent();
    let mut app = App::open(runtime, clients, agent.clone());
    app.wait_until("the item to be listed In progress", |app| {
        status_in_app(app, &item) == Some(ItemStatus::InProgress)
    });

    // The agent moves the person's item to Done from its tab; the move
    // waits as a request (BD-9).
    runtime
        .block_on(daemon.send(&session, CALL_LINE))
        .expect("type the line that makes the agent call");
    app.wait_until("the agent to start its call", |app| {
        app.agent.started_with_token().is_some()
    });
    assert_eq!(
        agent.started_with_token(),
        Some(true),
        "the agent's tab has a session token, so its call is that session's"
    );
    app.wait_until("the agent's move to wait in the Inbox", |app| {
        app.waiting().len() == 1
    });
    assert_eq!(agent.exit_code(), None, "the agent's move waits for the answer");
    let request = app.waiting().remove(0);
    assert_eq!(request.session, session, "the requester is the agent's session");
    assert_eq!(request.channel, channel);
    assert_eq!(
        status_in_app(&app, &item),
        Some(ItemStatus::InProgress),
        "a move that waits for the person doesn't move the item yet"
    );

    // The whole workspace's board (the sidebar now opens the To-do list);
    // the item's card offers Review.
    app.open_board();
    assert!(app.ui.read(|state| matches!(state.route, Route::Board(_))));
    app.wait_until("the item's card to offer Review", |app| {
        let view = app.view();
        inside(&view, Role::ListItem, &format!("{} {ITEM_TITLE}", item.as_str()))
            .is_some_and(|card| card.contains(&heading(Role::Button, "Review")))
    });
    app.click(Role::Button, "Review");
    app.wait_until("the item's drawer with the agent's ask", |app| {
        let view = app.view();
        view.contains(&heading(Role::Button, APPROVE)) && view.contains(&heading(Role::Button, DENY))
    });
    assert_eq!(agent.exit_code(), None, "opening the drawer answers nothing");
    (world, app, session, item, request)
}

/// BD-15, frame 22: Move to Done in the drawer lets the agent's waiting
/// move through. The call returns success, the request leaves the Inbox,
/// and the item is Done.
#[test]
fn approving_an_agents_move_from_the_items_drawer_moves_it_and_returns_the_call() {
    let runtime = runtime();
    let (world, mut app, _session, item, request) = ask_open_in_the_drawer(&runtime);

    app.click(Role::Button, APPROVE);
    app.wait_until("the agent's waiting move to return", |app| {
        app.agent.exit_code().is_some()
    });
    assert_eq!(
        app.agent.exit_code(),
        Some(0),
        "the approved move succeeds\n{}",
        app.agent.describe()
    );
    app.wait_until("the answered request to leave the Inbox", |app| {
        !app.waiting().iter().any(|waiting| waiting.id == request.id)
    });
    app.wait_until("the item to be Done", |app| {
        status_in_app(app, &item) == Some(ItemStatus::Done)
    });

    drop(app);
    drop(world);
}

/// BD-15, frame 22: Keep in progress denies the agent's move. The call
/// returns a failure, the request leaves the Inbox, and the item stays In
/// progress.
#[test]
fn denying_an_agents_move_from_the_items_drawer_keeps_it_in_progress() {
    let runtime = runtime();
    let (world, mut app, _session, item, request) = ask_open_in_the_drawer(&runtime);

    app.click(Role::Button, DENY);
    app.wait_until("the agent's waiting move to return", |app| {
        app.agent.exit_code().is_some()
    });
    let code = app.agent.exit_code();
    assert!(
        code.is_some_and(|code| code != 0),
        "the denied move fails, got {code:?}\n{}",
        app.agent.describe()
    );
    app.wait_until("the answered request to leave the Inbox", |app| {
        !app.waiting().iter().any(|waiting| waiting.id == request.id)
    });
    assert_eq!(status_in_app(&app, &item), Some(ItemStatus::InProgress));
    // A fresh load agrees: the denial moved nothing.
    let daemon = app.daemon.clone();
    let workspace = runtime.block_on(daemon.load()).expect("load the workspace");
    let status = workspace
        .items
        .iter()
        .find(|listed| listed.id == item)
        .map(|listed| listed.status);
    assert_eq!(status, Some(ItemStatus::InProgress));

    drop(app);
    drop(world);
}

// ---------------------------------------------------------------------
// The board screens: the New item form, the dependency picker, Start a
// session for this item, the three views, and the session panel
// (flows/05-long-running-project.md steps 2, 4 and 5; macro case steps
// 1, 2, 5, 9 and 10).
// ---------------------------------------------------------------------

/// A channel with a repo on This Mac, made through the daemon.
async fn channel_setup(world: &World, clients: &Clients) -> (ChannelId, String, MachineId) {
    let mac: MachineId = clients
        .daemon
        .load()
        .await
        .expect("load the workspace")
        .machines
        .first()
        .map(|machine| machine.id.clone())
        .expect("the workspace has This Mac");
    let repo = world.repo("owner", "ledger").expect("make a repo");
    let channel = clients
        .daemon
        .create_channel(CHANNEL, vec![repo.clone()], vec![mac.clone()])
        .await
        .expect("create the channel");
    (channel, repo, mac)
}

/// The card or row label the board gives an item.
fn label_of(item: &Item) -> String {
    format!("{} {}", item.id.as_str(), item.title)
}

/// Whether the view draws every card or row in `labels`.
fn all_shown(app: &mut App<'_>, labels: &[String]) -> bool {
    let view = app.view();
    labels
        .iter()
        .all(|label| view.contains(&heading(Role::ListItem, label)))
}

impl App<'_> {
    /// The label the board gives the app's item `id`.
    fn label_of_listed(&self, id: &ItemId) -> String {
        self.ui
            .read(|state| state.workspace.items.iter().find(|i| &i.id == id).cloned())
            .map(|item| label_of(&item))
            .expect("the app lists the item")
    }

    /// Whether the view draws `role` `label`.
    fn shows(&mut self, role: Role, label: &str) -> bool {
        self.view().contains(&heading(role, label))
    }

    /// The app's item titled `title`.
    fn item_titled(&self, title: &str) -> Option<Item> {
        self.ui.read(|state| {
            state
                .workspace
                .items
                .iter()
                .find(|item| item.title == title)
                .cloned()
        })
    }

    /// The item as the daemon lists it now, read by the live client.
    fn daemon_item(&self, id: &ItemId) -> Option<Item> {
        let daemon = self.daemon.clone();
        let workspace = self
            .runtime
            .block_on(daemon.load())
            .expect("load the workspace from the daemon");
        workspace.items.iter().find(|item| &item.id == id).cloned()
    }

    /// The item as the daemon lists it, with the sessions its drawer
    /// lists.
    fn daemon_item_sessions(&self, id: &ItemId) -> Vec<SessionId> {
        let daemon = self.daemon.clone();
        let workspace = self
            .runtime
            .block_on(daemon.load())
            .expect("load the workspace from the daemon");
        workspace
            .item_details
            .get(id)
            .map(|details| details.sessions.clone())
            .unwrap_or_default()
    }

    /// Opens the channel's page (a click can't reach it without a
    /// sidebar row per channel, so this sends the navigation the row
    /// sends) and its Board tab.
    /// Shows the whole workspace's board, as "Go to Board" in the palette does.
    fn open_board(&mut self) {
        self.cx.update(|cx| {
            self.ui
                .send(AppAction::Switch(Route::Board(Scope::Global)), cx)
        });
        self.settle();
    }

    fn open_channel_board(&mut self, channel: &ChannelId) {
        let channel = channel.clone();
        self.cx.update(|cx| {
            self.ui
                .send(AppAction::Navigate(Route::Channel(channel)), cx)
        });
        self.settle();
        self.wait_until("the channel's tabs", |app| app.shows(Role::Tab, "To-do"));
        // A channel's page opens on the tab it showed last. Already on
        // Board, the board's own view switch shows a Tab "Board" too.
        if !self.shows(Role::Button, "New item") {
            self.click(Role::Tab, "To-do");
        }
        self.wait_until("the channel's board", |app| {
            app.shows(Role::Button, "New item")
        });
    }

    /// Waits for `title` to be a Todo card on the board the app shows,
    /// and returns it.
    fn wait_for_card_in_to_do(&mut self, title: &str) -> Item {
        self.wait_until(&format!("{title} to show in To do"), |app| {
            app.item_titled(title).is_some_and(|item| {
                app.ui.read(|state| {
                    state
                        .board
                        .column(ItemStatus::Todo)
                        .iter()
                        .any(|card| card.id == item.id)
                })
            })
        });
        let item = self.item_titled(title).expect("listed after the wait");
        let label = label_of(&item);
        self.wait_until(&format!("{title}'s card"), |app| {
            app.shows(Role::ListItem, &label)
        });
        item
    }

    /// Clicks the To do column's add button, types `title` and creates.
    fn new_item_in_to_do(&mut self, title: &str) -> Item {
        self.click(Role::Button, "Add item to To do");
        self.wait_until("the New item form", |app| {
            app.ui.read(|state| state.board.new_title.is_some())
        });
        self.click(Role::TextInput, "Item title");
        self.type_text(title);
        // What was typed must reach the form before Create item, or the
        // item is made with a shorter title.
        self.wait_until("the typed title to reach the form", |app| {
            app.ui
                .read(|state| state.board.new_title.as_deref() == Some(title))
        });
        self.click(Role::Button, "Create item");
        self.wait_for_card_in_to_do(title)
    }
}

/// Flow 5 step 2; macro case steps 1 and 5. New item in To do three times
/// gives each the channel's key and a card in To do. A dependency set from
/// the drawer's picker is listed and held by the daemon, and the picker
/// leaves out the item itself and an item that would make a loop.
#[test]
fn new_items_get_the_channels_key_and_a_dependency_is_set_from_the_picker() {
    let runtime = runtime();
    let (world, clients, channel) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        let (channel, _, _) = channel_setup(&world, &clients).await;
        (world, clients, channel)
    });
    let mut app = App::open(&runtime, clients, world.agent());
    app.wait_until("the channel to load", |app| {
        app.ui
            .read(|state| state.workspace.channels.iter().any(|c| c.id == channel))
    });
    app.open_channel_board(&channel);

    let schema = app.new_item_in_to_do("Schema");
    let write = app.new_item_in_to_do("Write path");
    let read = app.new_item_in_to_do("Read path");
    let keys: Vec<&str> = [&schema, &write, &read]
        .iter()
        .map(|item| item.id.as_str())
        .collect();
    for item in [&schema, &write, &read] {
        assert!(
            item.id.as_str().starts_with("PAY-"),
            "{} has the channel's key",
            item.id.as_str()
        );
        assert_eq!(item.scope, Scope::Channel(channel.clone()));
        assert_eq!(item.status, ItemStatus::Todo);
        let held = app.daemon_item(&item.id).expect("the daemon holds the item");
        assert_eq!(held.status, ItemStatus::Todo);
        assert_eq!(held.scope, Scope::Channel(channel.clone()));
    }
    let mut distinct = keys.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(distinct.len(), 3, "each item has its own key: {keys:?}");

    // Write path depends on Schema, set through the picker.
    app.click(Role::ListItem, &label_of(&write));
    app.wait_until("Write path's drawer", |app| {
        app.ui
            .read(|state| state.board.drawer.as_ref() == Some(&write.id))
    });
    // Its + shows while the pointer is over the field.
    app.hover(Role::Button, "Add dependency");
    app.wait_until("Write path's Add dependency", |app| {
        app.shows(Role::Button, "Add dependency")
    });
    app.click(Role::Button, "Add dependency");
    app.wait_until("the picker to offer the other items", |app| {
        app.shows(Role::Button, &label_of(&schema)) && app.shows(Role::Button, &label_of(&read))
    });
    assert!(
        !app.shows(Role::Button, &label_of(&write)),
        "the picker leaves out the item itself\n{}",
        app.view()
    );
    app.click(Role::Button, &label_of(&schema));
    app.wait_until("the daemon to hold the edge", |app| {
        app.daemon_item(&write.id)
            .is_some_and(|held| held.depends_on == vec![schema.id.clone()])
    });
    app.wait_until("the app to hold the edge", |app| {
        app.item_titled("Write path")
            .is_some_and(|item| item.depends_on == vec![schema.id.clone()])
    });
    // The drawer lists what the item depends on.
    app.wait_until("the drawer to list the dependency", |app| {
        app.shows(Role::Button, &label_of(&schema))
    });
    // Making the edge changes neither item's column.
    assert_eq!(
        app.daemon_item(&schema.id).map(|held| held.status),
        Some(ItemStatus::Todo)
    );

    // Schema now can't wait on Write path (a loop), nor on itself; Read
    // path is still offered.
    app.click(Role::ListItem, &label_of(&schema));
    app.wait_until("Schema's drawer", |app| {
        app.ui
            .read(|state| state.board.drawer.as_ref() == Some(&schema.id))
    });
    app.hover(Role::Button, "Add dependency");
    app.wait_until("Schema's Add dependency", |app| {
        app.shows(Role::Button, "Add dependency")
    });
    app.click(Role::Button, "Add dependency");
    app.wait_until("the picker to offer Read path", |app| {
        app.shows(Role::Button, &label_of(&read))
    });
    let view = app.view();
    assert!(
        !view.contains(&heading(Role::Button, &label_of(&write))),
        "the picker leaves out an item that would make a loop\n{view}"
    );
    assert!(
        !view.contains(&heading(Role::Button, &label_of(&schema))),
        "the picker leaves out the item itself\n{view}"
    );
    assert_eq!(
        app.daemon_item(&schema.id).map(|held| held.depends_on),
        Some(Vec::new()),
        "opening the picker wrote nothing"
    );

    drop(app);
    drop(world);
}

/// Flow 5 step 5; macro case step 2. Start a session for this item, from
/// the filled-in sheet, opens the session page; the item is In progress
/// with the session listed, and the daemon holds both.
#[test]
fn starting_a_session_for_a_to_do_item_opens_it_and_moves_the_item_in_progress() {
    let runtime = runtime();
    let (world, clients, channel, item) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        let (channel, _, _) = channel_setup(&world, &clients).await;
        let item = clients
            .daemon
            .create_item(Scope::Channel(channel.clone()), ITEM_TITLE)
            .await
            .expect("make the item");
        (world, clients, channel, item)
    });
    let mut app = App::open(&runtime, clients, world.agent());
    app.wait_until("the channel and its machine to load", |app| {
        app.item_titled(ITEM_TITLE).is_some()
            && app.ui.read(|state| {
                state
                    .workspace
                    .channel_machines(&channel)
                    .any(|machine| machine.online)
            })
    });
    app.open_channel_board(&channel);
    let card = label_of(&app.item_titled(ITEM_TITLE).expect("listed"));
    app.wait_until("the item's card", |app| app.shows(Role::ListItem, &card));
    app.click(Role::ListItem, &card);
    app.wait_until("the item's drawer", |app| {
        app.shows(Role::Button, "Start a session for this item")
    });
    app.click(Role::Button, "Start a session for this item");
    app.wait_until("the sheet, filled in for the item", |app| {
        app.ui.read(|state| match state.overlays.last() {
            Some(Overlay::NewSession(sheet)) => sheet.item.as_ref() == Some(&item),
            _ => false,
        })
    });
    // Nothing else: the sheet's title, channel and machine come from the
    // item.
    app.click(Role::Button, "Start session");

    app.wait_until("the session to be listed", |app| {
        app.session_by_item(&item).is_some()
    });
    let session = app.session_by_item(&item).expect("listed after the wait");
    app.wait_until("the session page to show", |app| {
        app.ui
            .read(|state| state.route == Route::Session(session.clone()))
    });
    app.wait_until("the item to be In progress in the app", |app| {
        status_in_app(app, &item) == Some(ItemStatus::InProgress)
    });
    // The daemon holds the item In progress, with the session on it.
    let held = app.daemon_item(&item).expect("the daemon holds the item");
    assert_eq!(held.status, ItemStatus::InProgress);
    assert_eq!(held.session, Some(session.clone()));
    assert!(app.daemon_item_sessions(&item).contains(&session));

    // Back on the board, the card is in In progress and its drawer lists
    // the session.
    app.open_channel_board(&channel);
    app.wait_until("the card to show", |app| app.shows(Role::ListItem, &card));
    assert!(app.ui.read(|state| state
        .board
        .column(ItemStatus::InProgress)
        .iter()
        .any(|listed| listed.id == item)));
    app.click(Role::ListItem, &card);
    let title = app
        .ui
        .read(|state| state.workspace.session(&session).map(|s| s.title.clone()))
        .expect("the app lists the session");
    app.wait_until("the drawer to list the session", |app| {
        app.shows(Role::Button, &title)
    });

    drop(app);
    drop(world);
}

impl App<'_> {
    /// The session the daemon started for `item`: the one the app lists
    /// on the item's card.
    fn session_by_item(&self, item: &ItemId) -> Option<SessionId> {
        self.ui.read(|state| {
            state
                .workspace
                .items
                .iter()
                .find(|listed| &listed.id == item)
                .and_then(|listed| listed.session.clone())
        })
    }
}

/// Flow 5 step 4; macro case step 9. Board, Table and Dashboard, with the
/// scope set to the channel, show the same items; a dashboard figure
/// opens exactly the items it counts.
#[test]
fn the_three_views_show_the_same_items_and_a_dashboard_figure_opens_what_it_counts() {
    let runtime = runtime();
    let (world, clients, ids) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        let (channel, _, _) = channel_setup(&world, &clients).await;
        let mut ids = Vec::new();
        for title in ["Schema", "Write path", "Read path"] {
            ids.push(
                clients
                    .daemon
                    .create_item(Scope::Channel(channel.clone()), title)
                    .await
                    .expect("make the item"),
            );
        }
        // One item is In progress, so the figures differ.
        clients
            .daemon
            .move_item(&ids[2], ItemStatus::InProgress, None)
            .await
            .expect("move the item");
        (world, clients, ids)
    });
    let mut app = App::open(&runtime, clients, world.agent());
    app.wait_until("the items to load", |app| {
        ids.iter().all(|id| status_in_app(app, id).is_some())
    });
    let labels: Vec<String> = ids.iter().map(|id| app.label_of_listed(id)).collect();

    app.open_board();
    app.wait_until("the board", |app| app.shows(Role::Button, "New item"));
    app.wait_until("the cards on the board", |app| all_shown(app, &labels));

    // Set the scope to the channel, then walk the three views.
    app.click(Role::Button, "Scope: All channels");
    app.click(Role::Button, &format!("# {CHANNEL}"));
    app.wait_until("the scope to be the channel", |app| {
        app.shows(Role::Button, &format!("Scope: # {CHANNEL}"))
    });
    app.wait_until("the cards in the channel's scope", |app| {
        all_shown(app, &labels)
    });

    app.click(Role::Tab, "Table");
    app.wait_until("the same items in the table", |app| {
        !app.shows(Role::Button, "Add item to To do") && all_shown(app, &labels)
    });

    app.click(Role::Tab, "Dashboard");
    app.wait_until("the dashboard's figures", |app| {
        app.shows(Role::Button, "In progress: 1")
    });
    // The Unassigned figure counts every item still open here: all three.
    assert!(
        app.shows(Role::Button, "Unassigned: 3"),
        "{}",
        app.view()
    );

    // In progress opens exactly its one item.
    app.click(Role::Button, "In progress: 1");
    app.wait_until("the list the figure opened", |app| {
        app.shows(Role::Button, "Show all")
    });
    let view = app.view();
    assert!(
        view.contains(&heading(Role::ListItem, &labels[2])),
        "the counted item is listed\n{view}"
    );
    assert!(
        !view.contains(&heading(Role::ListItem, &labels[0]))
            && !view.contains(&heading(Role::ListItem, &labels[1])),
        "no item the figure didn't count is listed\n{view}"
    );
    app.click(Role::Button, "Show all");
    app.wait_until("every item to be listed again", |app| {
        all_shown(app, &labels)
    });

    // Unassigned opens all three.
    app.click(Role::Tab, "Dashboard");
    app.wait_until("the dashboard again", |app| {
        app.shows(Role::Button, "Unassigned: 3")
    });
    app.click(Role::Button, "Unassigned: 3");
    app.wait_until("the unassigned list", |app| {
        app.shows(Role::Button, "Show all") && all_shown(app, &labels)
    });

    // Back on the board view, the same items are cards again.
    app.click(Role::Button, "Show all");
    app.click(Role::Tab, "Board");
    app.wait_until("the cards again", |app| {
        app.shows(Role::Button, "Add item to To do") && all_shown(app, &labels)
    });

    drop(app);
    drop(world);
}
