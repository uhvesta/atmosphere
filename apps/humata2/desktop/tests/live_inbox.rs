//! SPEC flow 26.4 (approve an Inbox request after opening the requester;
//! IN-9, IN-11, IN-12) against a real `humatad2`. The agent is a shell
//! script in the session's agent tab: on one typed line it runs the real
//! `humata` CLI from inside its tab, as that session, and waits for the
//! answer. The app is headless: no window is shown.
//!
//! The test makes its own daemon in its own temp folder.
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
use humata2_app::Route;
use humata2_clients::model::{
    Agent, Capability, ChannelId, MachineId, Request, RuleSetting, SessionId, SessionOptions,
};
use humata2_clients::{Clients, Daemon, Event};
use humata2_clients_live::{LiveComments, LiveDaemon, LiveOptions};
use humata2_engine::Resources;
use humata2_flow::{Action, Driver as _, FrameMode, GpuiDriver, Target};
use humata2_session::Panel;
use humata2_ui::bridge::Ui;
use humata2_ui::record::Recorder;
use tca::{MemoryStorage, SystemClock};
use tokio::runtime::Runtime;

const DEADLINE: Duration = Duration::from_secs(60);

/// The daemon's name for the test agent.
const AGENT_NAME: &str = "fake";
/// The line that makes the agent list every channel through `humata`.
const CALL_LINE: &str = "list channels";
/// Typed after the answer, to see what reached the terminal before it.
const AFTER_LINE: &str = "after the answer";
const FIRST_MESSAGE: &str = "hello agent";
const CHANNEL: &str = "payments";
const SESSION: &str = "API rate limiter";
/// The rule the agent's call falls under: reading other channels (D127.4:
/// Allow unless the person sets it to Ask).
const RULE: &str = "global_read";

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
/// file. On [`CALL_LINE`] it runs `humata --json --wait 600 channels` from
/// inside its tab, so the call carries the tab's session token, and leaves
/// what happened in `out`: `started` before the call, then `stdout`,
/// `stderr` and (last, by rename) `code`. While the call waits the script
/// reads nothing, so anything typed into the terminal meanwhile is the
/// next line it keeps.
fn agent_script(conv: &Path, out: &Path, socket: &Path, humata: &Path) -> String {
    format!(
        r#"CONV_DIR='{conv}'
OUT='{out}'
mode=$1; id=$2
case "$mode" in
  --session-id) f="$CONV_DIR/$id.jsonl"; : > "$f" ;;
  --resume) f="$CONV_DIR/$id.jsonl" ;;
esac
echo "agent up $mode"
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$f"
  case "$line" in
    '{call}')
      if [ -n "$HUMATA_SESSION_TOKEN" ]; then echo yes > "$OUT/started"; else echo no > "$OUT/started"; fi
      HUMATA_SOCKET='{socket}' '{humata}' --json --wait 600 channels > "$OUT/stdout" 2> "$OUT/stderr" < /dev/null
      printf '%s' "$?" > "$OUT/code.part"
      mv "$OUT/code.part" "$OUT/code"
      ;;
  esac
  echo "got: $line"
done
"#,
        conv = conv.display(),
        out = out.display(),
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
struct World {
    dir: tempfile::TempDir,
    daemon: Option<Child>,
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
        let world = Self { dir, daemon: None };
        let agent = world.agent();
        std::fs::create_dir_all(&agent.conv).map_err(io("make conversations folder"))?;
        std::fs::create_dir_all(&agent.out).map_err(io("make the agent's output folder"))?;
        std::fs::create_dir_all(root.join("remotes")).map_err(io("make remotes folder"))?;
        let script = root.join("agent.sh");
        std::fs::write(
            &script,
            agent_script(&agent.conv, &agent.out, &world.socket(), &binary("HUMATA")?),
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
            terminals: Arc::new(daemon),
            clock: Arc::new(SystemClock::new()),
            clipboard: Arc::new(humata2_clients::FakeClipboard::default()),
            files: Arc::new(humata2_clients::FakeFileSystem::default()),
            file_manager: Arc::new(humata2_clients::FakeFileManager::default()),
            keychain: Arc::new(humata2_clients::FakeKeychain::default()),
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
        let summary = match event {
            Event::TerminalData {
                session,
                tab,
                bytes,
                ..
            } => format!(
                "TerminalData {} tab {tab}: {} bytes",
                session.as_str(),
                bytes.len()
            ),
            other => format!("{other:?}").chars().take(300).collect(),
        };
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

/// `text` as the view writes it inside a line.
fn written(text: &str) -> String {
    let quoted = format!("{text:?}");
    quoted[1..quoted.len() - 1].to_owned()
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
}

impl<'r> App<'r> {
    fn open(runtime: &'r Runtime, clients: Clients, agent: AgentFiles) -> Self {
        let daemon = clients.daemon.clone();
        let heard = Arc::new(Mutex::new(Heard::default()));
        {
            let daemon = daemon.clone();
            let heard = heard.clone();
            runtime.spawn(async move {
                let Ok(mut events) = daemon.events().await else {
                    return;
                };
                while let Some(Ok(event)) = events.recv().await {
                    if let Ok(mut heard) = heard.lock() {
                        heard.hear(&event);
                    }
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

    /// What the window draws inside the element `role` `label`, if it shows.
    fn inside(&mut self, role: Role, label: &str) -> Option<String> {
        inside(&self.view(), role, label)
    }

    /// The requests the app holds that still wait for an answer: what the
    /// sidebar's Inbox row counts.
    fn waiting(&self) -> Vec<Request> {
        self.ui
            .read(|state| state.workspace.waiting().cloned().collect())
    }

    /// Settles until `done` holds or the deadline passes.
    fn wait_until(&mut self, what: &str, mut done: impl FnMut(&mut Self) -> bool) {
        let until = Instant::now() + DEADLINE;
        loop {
            self.settle();
            if done(self) {
                return;
            }
            if Instant::now() >= until {
                let why = self.diagnose();
                panic!("timed out waiting for {what}\n{why}");
            }
            std::thread::sleep(Duration::from_millis(100));
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

/// A channel with one repo on This Mac and a session in it whose agent has
/// taken its first message.
async fn running_session(world: &World, clients: &Clients) -> (ChannelId, SessionId) {
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
    let agent = world.agent();
    let until = Instant::now() + DEADLINE;
    while !agent.typed().iter().any(|line| line == FIRST_MESSAGE) {
        assert!(
            Instant::now() < until,
            "the agent never got its first message\n{}",
            agent.describe()
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    (channel, session)
}

/// 26.4: the agent's call waits in the Inbox; the person opens the Inbox,
/// reads the card, opens the requester's session from it and allows there.
/// The agent's waiting call returns what it asked for, the request leaves
/// the Inbox, and nothing is typed into the agent's terminal (IN-12).
#[test]
fn a_request_allowed_from_the_requesters_session_returns_the_agents_waiting_call() {
    let runtime = runtime();
    let (world, clients, channel, session) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        let (channel, session) = running_session(&world, &clients).await;
        (world, clients, channel, session)
    });
    let daemon = clients.daemon.clone();
    let agent = world.agent();
    let mut app = App::open(&runtime, clients, agent.clone());
    app.wait_until("the session to be listed", |app| {
        app.ui
            .read(|state| state.workspace.session(&session).is_some())
    });
    assert!(
        app.waiting().is_empty(),
        "nothing waits before the agent asks"
    );

    // The person makes reading other channels Ask for this channel, then
    // the agent lists every channel from its tab and waits for the answer.
    runtime
        .block_on(daemon.set_rule(&channel, &Capability::new(RULE), RuleSetting::Ask))
        .expect("set the channel's rule for reading other channels to Ask");
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

    // 1. The Inbox counts the request, and the global Inbox lists it.
    app.wait_until("the Inbox to count the agent's request", |app| {
        app.waiting().len() == 1
    });
    assert_eq!(
        agent.exit_code(),
        None,
        "the agent's call waits for the answer"
    );
    let request = app.waiting().remove(0);
    assert_eq!(request.session, session, "the requester is the agent's session");
    assert_eq!(request.channel, channel);
    assert!(!request.title.is_empty(), "the request says what is asked");
    let asked_by = request
        .asked_by
        .clone()
        .expect("the request names the rule that sent it to the person");
    assert_eq!(asked_by.capability, Capability::new(RULE));
    assert_eq!(asked_by.setting, RuleSetting::Ask);

    app.click(Role::Link, "Inbox");
    assert!(app.ui.read(|state| state.route == Route::Inbox));
    app.wait_until("the global Inbox to list the request", |app| {
        app.inside(Role::ListBoxOption, &request.title).is_some()
    });

    // 2. Selected, its card says who asks, for what, why and by which rule
    // (IN-9, the parts the daemon sends).
    app.click(Role::ListBoxOption, &request.title);
    app.wait_until("the request's card", |app| {
        app.inside(Role::Group, &request.title).is_some()
    });
    let card = app
        .inside(Role::Group, &request.title)
        .expect("shown after the wait");
    // The daemon sends no reason with a read a rule held back (only the
    // agent's own asks and board changes carry one), so the reason is
    // looked for only when there is one.
    for (part, what) in [
        (SESSION, "the requester's session"),
        (CHANNEL, "the requester's channel"),
        (asked_by.label.as_str(), "the rule that asked"),
        (request.reason.as_str(), "the reason"),
    ]
    .into_iter()
    .filter(|(part, _)| !part.is_empty())
    {
        assert!(
            card.contains(&written(part)),
            "the card shows {what} ({part:?}):\n{card}"
        );
    }

    // 3. Open session ↗ lands on the requester's session with its Inbox
    // panel and this request open (IN-11).
    app.click(Role::Button, "Open session ↗");
    app.wait_until(
        "the requester's session with its Inbox panel and the request selected",
        |app| {
            app.ui.read(|state| {
                state.route == Route::Session(session.clone())
                    && state.panel() == Some(Panel::Inbox)
                    && state.inbox.selected.as_ref() == Some(&request.id)
            })
        },
    );
    app.wait_until("the request open in the session's Inbox panel", |app| {
        app.inside(Role::Group, &request.title).is_some()
    });
    assert_eq!(agent.exit_code(), None, "opening the session answers nothing");

    // 4. Allow: the waiting call returns the channels, and the request is
    // gone from the panel, the count and the global Inbox.
    app.click(Role::Button, "Allow");
    app.wait_until("the agent's waiting call to return", |app| {
        app.agent.exit_code().is_some()
    });
    assert_eq!(
        agent.exit_code(),
        Some(0),
        "the allowed call succeeds\n{}",
        agent.describe()
    );
    let printed = agent.read("stdout").unwrap_or_default();
    let listed: serde_json::Value = serde_json::from_str(&printed)
        .unwrap_or_else(|error| panic!("the call prints JSON ({error}): {printed:?}"));
    assert!(
        listed.to_string().contains(CHANNEL),
        "the call returns the channel list: {listed}"
    );

    app.wait_until("the answered request to leave the Inbox count", |app| {
        app.waiting().is_empty()
    });
    app.wait_until("the request to leave the session's Inbox panel", |app| {
        let view = app.view();
        inside(&view, Role::Group, &request.title).is_none()
            && inside(&view, Role::Button, &request.title).is_none()
    });
    app.click(Role::Link, "Inbox");
    app.wait_until("the request to leave the global Inbox", |app| {
        app.inside(Role::ListBoxOption, &request.title).is_none()
    });

    // IN-12: the answer reached the waiting call, so nothing was typed into
    // the agent's terminal. The agent read nothing while it waited; the
    // next line it reads is the one typed now.
    runtime
        .block_on(daemon.send(&session, AFTER_LINE))
        .expect("type a line after the answer");
    app.wait_until("the agent to read the line typed after the answer", |app| {
        app.agent.typed().iter().any(|line| line.contains(AFTER_LINE))
    });
    let typed = agent.typed();
    let since_the_call: Vec<&str> = typed
        .iter()
        .skip_while(|line| line.as_str() != CALL_LINE)
        .skip(1)
        .map(String::as_str)
        .collect();
    assert_eq!(
        since_the_call,
        [AFTER_LINE],
        "answering types nothing into the agent's terminal"
    );

    drop(app);
    drop(world);
}
