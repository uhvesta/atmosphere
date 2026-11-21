//! The Hosts page, the new-session picker and the new-channel sheet on
//! machines, in the app against a real admin `humatad2` that reaches its
//! machine through a fake `ssh` (no network, never the real `~/.ssh`), with
//! the real `LiveMachines` client. The app is headless: no window is shown.
//!
//! Flows: 05 step 9 and macro case 05 step 12 (add a machine over SSH, the
//! host key card; a failed add and Try again; a provider and its params; a
//! login by hand and the unattended offer), macro case 03 steps 6 and 7 (the
//! picker lists box-1, and a session started there runs there) and macro
//! case 02 step 8 (an offline machine can't be ticked).
//!
//! Each test makes its own daemon and fake machine in its own temp folder.
#![cfg(target_os = "macos")]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::{
    AnyWindowHandle, Bounds, HeadlessAppContext, Role, WindowBounds, WindowOptions, point, px, size,
};
use humata2_app::{AppAction, Overlay, Route};
use humata2_clients::machines::Machines;
use humata2_clients::machines::{StepKind, StepStatus};
use humata2_clients::model::{Agent, ChannelId, MachineId, Status};
use humata2_clients::{Clients, Daemon};
use humata2_clients_live::{LiveComments, LiveDaemon, LiveMachines, LiveOptions};
use humata2_engine::Resources;
use humata2_fake_machine::{FakeMachine, HOST_KEY};
use humata2_flow::{Action, Driver as _, FrameMode, GpuiDriver, Target};
use humata2_ui::bridge::Ui;
use humata2_ui::record::Recorder;
use sha2::{Digest, Sha256};
use tca::{MemoryStorage, SystemClock};
use tokio::runtime::Runtime;

const DEADLINE: Duration = Duration::from_secs(90);
const FILES: [&str; 5] = ["humatad2", "humata2_holder", "humata", "stack", "humata2_fusefs"];
/// What the person types; the daemon names the machine `box-1` from it.
const HOST: &str = "box-1.internal";
const NAME: &str = "box-1";
/// The scripted agent's name on the host and on the fake machine.
const AGENT: &str = "fake";

fn runtime() -> Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("a runtime starts")
}

fn binary(variable: &str) -> PathBuf {
    let path = std::env::var_os(variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("set {variable} to the binary's path (the bazel target passes it)"));
    std::fs::canonicalize(path).expect("the binary exists")
}

/// A tools bundle for linux-x86_64, the fake machine's default platform.
/// Its humatad2 runs the real one, so the peer link really comes up.
///
/// With `relay` (socket, state) the link's wrapper is given the machine
/// daemon's own state folder and socket, so the link relays to that daemon
/// (D146) and sessions can run there.
fn bundle(dir: &Path, relay: Option<(&Path, &Path)>) -> PathBuf {
    let root = dir.join("tools");
    std::fs::create_dir_all(root.join("linux-x86_64")).expect("make the bundle folder");
    let mut entries = serde_json::Map::new();
    for file in FILES {
        let body = if file == "humatad2" {
            // A link a test starts gets its state folder named, so it relays
            // nothing and starts no daemon of its own.
            let link = match relay {
                Some((socket, state)) => {
                    format!("--state '{}' --socket '{}'", state.display(), socket.display())
                }
                None => "--state \"$HOME/.humata2/state\"".to_owned(),
            };
            format!(
                "#!/bin/sh\ncase \" $* \" in *\" --stdio \"*) exec '{real}' \"$@\" {link} ;; esac\nexec '{real}' \"$@\"\n",
                real = binary("HUMATAD2_RELEASE").display()
            )
        } else {
            format!("#!/bin/sh\n# {file}\nexit 0\n")
        };
        std::fs::write(root.join("linux-x86_64").join(file), &body).expect("write a bundle file");
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
    std::fs::write(root.join("manifest.json"), manifest.to_string()).expect("write the manifest");
    root
}

/// The admin daemon with the fake machine's `ssh` first on its PATH.
/// Stops it when dropped.
struct Host {
    child: Child,
    socket: PathBuf,
    dir: tempfile::TempDir,
}

impl Host {
    async fn start(machine: &FakeMachine) -> Self {
        Self::start_with(machine, None).await
    }

    /// As `start`; with a `remote`, the host knows the scripted agent and the
    /// remote template, and box-1's link relays to the remote's daemon.
    async fn start_with(machine: &FakeMachine, remote: Option<&Remote>) -> Self {
        let dir = tempfile::Builder::new()
            .prefix("h2lm")
            .tempdir_in("/tmp")
            .expect("make a temp folder");
        // The daemon refuses a socket folder other users can open.
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))
            .expect("restrict the temp folder");
        let socket = dir.path().join("d.sock");
        let tools = bundle(dir.path(), remote.map(|remote| (remote.socket.as_path(), remote.state.as_path())));
        let mut command = Command::new(binary("HUMATAD2"));
        command
            .arg("serve")
            .arg("--admin")
            .arg("--channels")
            .arg(dir.path().join("ch"))
            .arg("--socket")
            .arg(&socket)
            .arg("--state")
            .arg(dir.path().join("state"))
            .arg("--root")
            .arg(dir.path())
            .arg("--holder")
            .arg(binary("HUMATA2_HOLDER"))
            .arg("--tools")
            .arg(&tools)
            .arg("--ssh-config")
            .arg(dir.path().join("ssh_config"))
            .arg("--skills-home")
            .arg(dir.path().join("home"))
            .env("PATH", format!("{}:/usr/bin:/bin", machine.ssh_dir().display()));
        if let Some(remote) = remote {
            command
                .arg("--agents")
                .arg(&remote.host_agents)
                .arg("--default-agent")
                .arg(AGENT)
                .arg("--remote-template")
                .arg(&remote.template);
        }
        let child = command.spawn().expect("start humatad2");
        let host = Self {
            child,
            socket,
            dir,
        };
        let deadline = Instant::now() + Duration::from_secs(30);
        while tokio::net::UnixStream::connect(&host.socket).await.is_err() {
            assert!(Instant::now() < deadline, "humatad2 never started listening");
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        host
    }

    /// The app's clients on the live daemon, with the real machines client,
    /// and a handle on that client to read a machine's detail.
    async fn clients(&self) -> (Clients, Arc<dyn Machines>) {
        self.clients_on(LiveOptions::new(self.socket.clone())).await
    }

    /// As `clients`, starting Claude sessions with the scripted agent.
    async fn clients_with_agent(&self) -> (Clients, Arc<dyn Machines>) {
        self.clients_on(LiveOptions::new(self.socket.clone()).with_agent(Agent::Claude, AGENT))
            .await
    }

    /// The host's folder: the providers file, the fake login and the
    /// providers' marker files live under it.
    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    /// Saves the providers file the daemon reads (`<state>/providers.toml`,
    /// beside `hosts.json`). Read at each use, so the dialog sees it when it
    /// next opens.
    fn save_providers(&self, toml: &str) {
        std::fs::write(self.path("state/providers.toml"), toml).expect("write the providers file");
    }

    async fn clients_on(&self, options: LiveOptions) -> (Clients, Arc<dyn Machines>) {
        let daemon = LiveDaemon::connect(options)
            .await
            .expect("the app connects to the daemon");
        // Built before `daemon` moves into the terminals client, as the app does.
        let file_manager = humata2_clients_live::LiveFileManager::new(&daemon);
        let machines: Arc<dyn Machines> = Arc::new(LiveMachines::new(&daemon));
        let views_daemon = daemon.clone();
        let clients = Clients {
            daemon: Arc::new(daemon.clone()),
            desk: Arc::new(daemon.clone()),
            comments: Arc::new(LiveComments::new(&daemon)),
            history: Arc::new(humata2_clients_live::history::LiveHistory::new(&daemon)),
            marks: Arc::new(humata2_clients_live::marks::LiveMarks::new(&daemon)),
            pull_requests: Arc::new(humata2_clients_live::LivePullRequests::new(&daemon)),
            review_chat: Arc::new(humata2_clients_live::LiveReviewChat::new(&daemon)),
            todo: Arc::new(humata2_clients_live::todo::LiveTodo::new(&daemon)),
            terminals: Arc::new(daemon),
            clock: Arc::new(SystemClock::new()),
            clipboard: Arc::new(humata2_clients::FakeClipboard::default()),
            reveal: Arc::new(humata2_clients::FakeReveal::default()),
            files: Arc::new(humata2_clients::FakeFileSystem::default()),
            file_manager: Arc::new(file_manager),
            keychain: Arc::new(humata2_clients::FakeKeychain::default()),
            machines: machines.clone(),
            storage: Arc::new(MemoryStorage::new()),
            views: Arc::new(humata2_clients_live::LiveViews::new(&views_daemon)),
            local_daemon: Arc::new(humata2_clients::FakeLocalDaemon::default()),
            notifier: Arc::new(humata2_clients::NoopNotifier),
        };
        (clients, machines)
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The app, headless, on the clients it was given.
struct App<'r> {
    runtime: &'r Runtime,
    cx: HeadlessAppContext,
    window: AnyWindowHandle,
    ui: Rc<Ui>,
    daemon: Arc<dyn Daemon>,
    /// The thread that opened the app: the one gpui wakes when work comes
    /// in from another thread, and the one the waits park (D442).
    thread: std::thread::ThreadId,
}

impl<'r> App<'r> {
    fn open(runtime: &'r Runtime, clients: Clients) -> Self {
        let daemon = clients.daemon.clone();
        {
            let daemon = daemon.clone();
            // Every event from the host wakes a wait: what it looks at may
            // be the host's, not the app's (D442).
            let waiter = std::thread::current();
            runtime.spawn(async move {
                let Ok(mut events) = daemon.events().await else {
                    return;
                };
                while let Some(Ok(_)) = events.recv().await {
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
            thread: std::thread::current().id(),
        };
        app.settle();
        app
    }

    /// Runs what the last step started, lets the sockets move, and draws a
    /// frame as the app would.
    fn settle(&mut self) {
        for _ in 0..3 {
            self.runtime.block_on(async {
                for _ in 0..16 {
                    tokio::task::yield_now().await;
                }
                tokio::time::sleep(Duration::from_millis(2)).await;
            });
            self.cx.run_until_parked();
        }
        self.cx
            .update_window(self.window, |_, window, cx| {
                humata2_engine::frame::natural_frame(window, cx);
            })
            .expect("the window stays open");
    }

    /// Non-sleeping drain + frame for `wait_until`, which already parks
    /// between iterations.
    fn drain_and_frame(&mut self) {
        for _ in 0..2 {
            self.runtime.block_on(async {
                for _ in 0..16 {
                    tokio::task::yield_now().await;
                }
            });
            self.cx.run_until_parked();
        }
        self.cx
            .update_window(self.window, |_, window, cx| {
                humata2_engine::frame::natural_frame(window, cx);
            })
            .expect("the window stays open");
    }

    fn play(&mut self, action: &Action) {
        GpuiDriver::with_frames(&mut self.cx, self.window, FrameMode::Natural)
            .perform(action)
            .unwrap_or_else(|error| panic!("{action} failed: {error}"));
        self.settle();
    }

    fn click(&mut self, role: Role, label: &str) {
        self.play(&Action::Click(Target::new(role, label)));
    }

    fn type_text(&mut self, text: &str) {
        self.play(&Action::Type(text.to_owned()));
    }

    /// Sends an action the driver has no control for (opening a page or a
    /// sheet from a menu the headless window can't reach).
    fn send(&mut self, action: AppAction) {
        self.cx.update(|cx| self.ui.send(action, cx));
        self.settle();
    }

    /// What the window shows, as text.
    fn view(&mut self) -> String {
        GpuiDriver::with_frames(&mut self.cx, self.window, FrameMode::Natural)
            .view()
            .expect("the view can be read")
    }

    /// Whether the view shows a control with this role and accessible name.
    fn shows(&mut self, role: Role, label: &str) -> bool {
        self.view().contains(&format!("{role:?} {label:?}"))
    }

    /// The machine the app lists under `name`, as (id, online).
    fn machine_named(&self, name: &str) -> Option<(MachineId, bool)> {
        self.ui.read(|state| {
            state
                .workspace
                .machines
                .iter()
                .find(|machine| machine.name == name)
                .map(|machine| (machine.id.clone(), machine.online))
        })
    }

    /// Settles until `done` holds or the deadline passes. Between looks the
    /// thread parks until something wakes it (D442): gpui, when another
    /// thread hands the app work; an event from the host. What `done`
    /// looks at must come with one of them.
    fn wait_until(&mut self, what: &str, mut done: impl FnMut(&mut Self) -> bool) {
        assert_eq!(
            std::thread::current().id(),
            self.thread,
            "waits run on the thread that opened the app, which is the one woken"
        );
        let until = Instant::now() + DEADLINE;
        loop {
            self.drain_and_frame();
            if done(self) {
                return;
            }
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                let why = self.diagnose();
                panic!("timed out waiting for {what}\n{why}");
            }
            std::thread::park_timeout(left.min(Duration::from_millis(5)));
        }
    }

    /// What the app and the daemon hold now, for a timeout.
    fn diagnose(&mut self) -> String {
        let mut text = self.ui.read(|state| {
            let mut text = format!(
                "route {:?}\nload {:?}\noverlays {:?}\ntoasts {:?}\nadd dialog {:?}\n",
                state.route, state.load, state.overlays, state.toasts, state.add_machine
            );
            for machine in &state.workspace.machines {
                text.push_str(&format!(
                    "app machine {} {:?} online {}\n",
                    machine.id.as_str(),
                    machine.name,
                    machine.online
                ));
            }
            text
        });
        let daemon = self.daemon.clone();
        match self.runtime.block_on(daemon.load()) {
            Ok(workspace) => {
                for machine in &workspace.machines {
                    text.push_str(&format!(
                        "daemon machine {} {:?} online {}\n",
                        machine.id.as_str(),
                        machine.name,
                        machine.online
                    ));
                }
            }
            Err(error) => text.push_str(&format!("daemon load failed: {error}\n")),
        }
        text.push_str(&format!("view:\n{}\n", self.view()));
        text
    }

    /// The machines the daemon lists now (ListMachines, through the client),
    /// as (name, online).
    fn daemon_machines(&self) -> Vec<(String, bool)> {
        let daemon = self.daemon.clone();
        self.runtime
            .block_on(daemon.load())
            .map(|workspace| {
                workspace
                    .machines
                    .iter()
                    .map(|machine| (machine.name.clone(), machine.online))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn dialog_open(&self) -> bool {
        self.ui.read(|state| state.add_machine.open)
    }

    /// Opens the add-machine dialog from the Hosts page, as a person does,
    /// and types the host.
    fn start_add(&mut self) {
        self.send(AppAction::Switch(Route::Hosts));
        self.click(Role::Button, "Add a machine\u{2026}");
        assert!(self.dialog_open(), "the dialog opens");
        self.wait_until("the dialog's choices to load", |app| {
            !app.ui.read(|state| state.add_machine.loading)
        });
        self.type_text(HOST);
        // What was typed must reach the dialog before Add, or the add goes
        // to no host.
        self.wait_until("the typed host to reach the dialog", |app| {
            app.ui.read(|state| state.add_machine.typed == HOST)
        });
    }

    /// Adds box-1 through the dialog: Add, the host key card, Confirm, and
    /// the machine online in the app.
    fn add_box_1(&mut self, machine: &FakeMachine) -> MachineId {
        self.start_add();
        self.click(Role::Button, "Add");
        self.wait_until("the host key card", |app| {
            app.ui.read(|state| state.add_machine.host_key.is_some())
        });
        assert!(
            machine.installed("humatad2").is_none(),
            "nothing is installed on the machine before the key is confirmed"
        );
        self.click(Role::Button, "Confirm");
        self.wait_until("the dialog to close on the added machine", |app| {
            !app.dialog_open()
        });
        self.wait_until("box-1 to be listed online in the app", |app| {
            app.machine_named(NAME).is_some_and(|(_, online)| online)
        });
        self.machine_named(NAME).expect("listed after the wait").0
    }

    /// Waits for the host's peer link to box-1: it reports a daemon version.
    fn wait_linked(&mut self, machines: &Arc<dyn Machines>, id: &MachineId) {
        let machines = machines.clone();
        let id = id.clone();
        self.wait_until("the peer link to box-1", |app| {
            app.runtime
                .block_on(machines.detail(&id))
                .is_ok_and(|detail| !detail.daemon_version.is_empty())
        });
    }
}

/// Flow 05 step 9 / macro case 05 step 12: Add a machine, type an SSH host,
/// Add; the dialog shows the new host's key and nothing is installed until
/// Confirm; after it, box-1 is listed in the app and by the daemon.
#[test]
fn adding_a_machine_over_ssh_shows_its_host_key_and_lists_it_once_confirmed() {
    let runtime = runtime();
    let machine = FakeMachine::new();
    let host = runtime.block_on(Host::start(&machine));
    let (clients, _machines) = runtime.block_on(host.clients());
    let mut app = App::open(&runtime, clients);

    app.start_add();
    app.click(Role::Button, "Add");
    app.wait_until("the host key card", |app| {
        app.ui.read(|state| state.add_machine.host_key.is_some())
    });
    let fingerprint = app
        .ui
        .read(|state| state.add_machine.host_key.clone())
        .expect("the key waits for a confirm");
    assert!(app.shows(Role::Group, "Host key"), "the card is on screen");
    let view = app.view();
    assert!(
        view.contains(&fingerprint),
        "the card shows the fingerprint {fingerprint:?}:\n{view}"
    );
    assert!(app.dialog_open(), "the dialog waits for the person");
    assert!(
        machine.installed("humatad2").is_none(),
        "nothing is installed before the key is confirmed"
    );
    assert!(
        !app.daemon_machines().iter().any(|(name, _)| name == NAME),
        "the machine is recorded only when the add finishes"
    );

    app.click(Role::Button, "Confirm");
    app.wait_until("the dialog to close on the added machine", |app| !app.dialog_open());
    app.wait_until("box-1 to be listed online in the app", |app| {
        app.machine_named(NAME).is_some_and(|(_, online)| online)
    });
    let (id, _) = app.machine_named(NAME).expect("listed after the wait");

    assert_eq!(
        app.ui.read(|state| state.hosts.selected.clone()),
        Some(id),
        "the new machine is selected on the Hosts page"
    );
    assert_eq!(app.ui.read(|state| state.route.clone()), Route::Hosts);
    assert!(
        app.daemon_machines().iter().any(|(name, online)| name == NAME && *online),
        "the daemon lists box-1 online"
    );
    assert!(
        machine.installed("humatad2").is_some(),
        "the host put the daemon on the machine"
    );
    drop(app);
    drop(host);
}

/// Macro case 03 step 6, the picker only: with box-1 added and in a channel,
/// the New session sheet lists it and picking it makes it the session's
/// machine. Starting a session on box-1 is not tried: the daemon on the fake
/// machine has no agent configured, so the start can't be proven live.
#[test]
fn the_new_session_picker_lists_an_added_machine_in_its_channel() {
    let runtime = runtime();
    let machine = FakeMachine::new();
    // Making a channel with box-1 in it sets the channel up on box-1, so
    // box-1's link relays to a daemon of its own there (D146).
    let remote = runtime.block_on(Remote::start(&machine));
    let host = runtime.block_on(Host::start_with(&machine, Some(&remote)));
    let (clients, machines) = runtime.block_on(host.clients());
    let mut app = App::open(&runtime, clients);

    let box_1 = app.add_box_1(&machine);
    app.wait_linked(&machines, &box_1);
    let mac = app
        .ui
        .read(|state| {
            state
                .workspace
                .machines
                .iter()
                .find(|m| m.id != box_1)
                .map(|m| m.id.clone())
        })
        .expect("This Mac is listed");
    // A channel's machines are fixed when it is made, so oncall is made with
    // box-1 in it, through the client (the New channel sheet is the next test).
    let daemon = app.daemon.clone();
    let channel: ChannelId = app
        .runtime
        .block_on(daemon.create_channel("oncall", Vec::new(), vec![mac.clone(), box_1.clone()]))
        .expect("make the channel");
    app.wait_until("the channel to reach the app", |app| {
        app.ui.read(|state| state.workspace.channel(&channel).is_some())
    });

    app.send(AppAction::Navigate(Route::Channel(channel.clone())));
    // The sheet opens from the channel page; the sidebar and Home both have
    // a New session button, so the action is sent.
    app.send(AppAction::StartNewSession);
    assert!(
        app.ui
            .read(|state| matches!(state.overlays.last(), Some(Overlay::NewSession(_)))),
        "New session opens the sheet"
    );
    let current = app
        .ui
        .read(|state| match state.overlays.last() {
            Some(Overlay::NewSession(sheet)) => sheet.machine.clone(),
            _ => None,
        })
        .expect("the sheet picks a machine");
    let current_name = app
        .ui
        .read(|state| state.workspace.machine(&current).map(|m| m.name.clone()))
        .expect("the picked machine is listed");
    app.click(Role::Button, &current_name);
    assert!(
        app.shows(Role::Button, NAME),
        "the picker lists box-1 among the channel's online machines"
    );
    app.click(Role::Button, NAME);
    app.wait_until("box-1 to become the session's machine", |app| {
        app.ui.read(|state| match state.overlays.last() {
            Some(Overlay::NewSession(sheet)) => sheet.machine.as_ref() == Some(&box_1),
            _ => false,
        })
    });
    drop(app);
    drop(host);
    drop(remote);
}

/// Macro case 02 step 8: an online box-1 ticks in the New channel sheet; once
/// its link is down it is shown offline and a click leaves it unticked.
/// The link is dropped by turning the fake machine's ssh off and killing the
/// machine-side daemon the link runs.
#[test]
fn an_offline_machine_cannot_be_ticked_in_the_new_channel_sheet() {
    let runtime = runtime();
    let machine = FakeMachine::new();
    let host = runtime.block_on(Host::start(&machine));
    let (clients, machines) = runtime.block_on(host.clients());
    let mut app = App::open(&runtime, clients);

    let box_1 = app.add_box_1(&machine);
    app.wait_linked(&machines, &box_1);

    app.send(AppAction::Switch(Route::Home));
    app.send(AppAction::StartNewChannel);
    assert!(
        app.ui
            .read(|state| matches!(state.overlays.last(), Some(Overlay::NewChannel(_)))),
        "New channel opens the sheet"
    );
    let ticked = |app: &App| -> Vec<MachineId> {
        app.ui.read(|state| match state.overlays.last() {
            Some(Overlay::NewChannel(sheet)) => sheet.machines.clone(),
            _ => Vec::new(),
        })
    };
    app.click(Role::CheckBox, NAME);
    assert!(ticked(&app).contains(&box_1), "an online machine ticks");
    app.click(Role::CheckBox, NAME);
    assert!(!ticked(&app).contains(&box_1), "and unticks");

    // The machine goes away: new ssh calls fail, and the running link dies.
    machine.set("down");
    let _ = Command::new("pkill")
        .arg("-f")
        .arg(machine.home().join(".humata2/state").display().to_string())
        .status();
    app.wait_until("box-1 to show offline", |app| {
        app.machine_named(NAME).is_some_and(|(_, online)| !online)
    });
    assert!(
        app.daemon_machines().iter().any(|(name, online)| name == NAME && !*online),
        "the daemon lists box-1 offline too"
    );

    app.click(Role::CheckBox, NAME);
    assert!(
        !ticked(&app).contains(&box_1),
        "an offline machine does not tick"
    );
    drop(app);
    drop(host);
}

// ---- The provider, login, failure and agent paths ---------------------------------

impl App<'_> {
    /// Opens the add-machine dialog from the Hosts page and waits for its
    /// choices, without typing a host.
    fn open_add(&mut self) {
        self.send(AppAction::Switch(Route::Hosts));
        self.click(Role::Button, "Add a machine\u{2026}");
        assert!(self.dialog_open(), "the dialog opens");
        self.wait_until("the dialog's choices to load", |app| {
            !app.ui.read(|state| state.add_machine.loading)
        });
    }

    /// Clicks a text field, types into it, and waits until the dialog holds
    /// what was typed (`reached`), so nothing is added without it.
    fn fill(&mut self, label: &str, text: &str, reached: impl Fn(&App) -> bool) {
        self.click(Role::TextInput, label);
        self.type_text(text);
        self.wait_until(&format!("{label} to reach the dialog"), |app| reached(app));
    }

    /// What the login terminal printed so far, as text.
    fn login_text(&self) -> String {
        self.ui.read(|state| {
            state
                .add_machine
                .login
                .as_ref()
                .map(|login| String::from_utf8_lossy(&login.bytes).into_owned())
                .unwrap_or_default()
        })
    }

    fn add_machine_text(&self) -> String {
        self.ui.read(|state| format!("{:?}", state.add_machine.choice))
    }

    /// The step the red card names, once an add failed.
    fn failed_step(&self) -> Option<StepKind> {
        self.ui
            .read(|state| state.add_machine.failure.as_ref().map(|failure| failure.step))
    }

    /// The step list, as (step, status).
    fn steps(&self) -> Vec<(StepKind, StepStatus)> {
        self.ui.read(|state| {
            state
                .add_machine
                .steps
                .iter()
                .map(|step| (step.kind, step.status))
                .collect()
        })
    }
}

fn executable(path: &Path, text: &str) {
    std::fs::write(path, text).expect("write a script");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("make it runnable");
}

/// A login command and home for a provider: without a terminal it asks for a
/// code and fails (the non-interactive try); with one it reads the code,
/// then runs the shell the daemon asked for. A login caches a token, as
/// corp's tool does, so later connects need no person (Keep corp's cached
/// token).
fn fake_login(dir: &Path) -> PathBuf {
    let home = dir.join("machine-home");
    let fakes = home.join(".test-bin");
    std::fs::create_dir_all(&fakes).expect("make the login's home");
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
             if [ -e \"$HOME/.corp-token\" ]; then exec sh -c \"$1\"; fi\n\
             if [ ! -t 0 ]; then printf 'Verification code: '; exit 1; fi\n\
             printf 'Verification code: '; read code\n\
             [ \"$code\" = 123456 ] || {{ echo denied; exit 3; }}\n\
             : > \"$HOME/.corp-token\"\n\
             exec sh -c \"$1\"\n",
            home = home.display(),
            fakes = fakes.display()
        ),
    );
    login
}

/// Flow 05 step 9, the provider path: the Host pop-up lists the providers,
/// picking one shows its params (the typed one and a default), and Add makes
/// the machine through the provider: the provider's create command got the
/// params, and box-1 is listed online and selected.
#[test]
fn the_popup_lists_a_provider_and_it_adds_a_machine_with_its_params() {
    let runtime = runtime();
    let machine = FakeMachine::new();
    let host = runtime.block_on(Host::start(&machine));
    let dir = host.path("");
    std::fs::write(
        dir.join("identity.json"),
        format!(
            r#"{{"name":"box-1","ssh":{{"host":"box-1.internal","user":"me","port":22,"host_key":"{HOST_KEY}"}},"os":"linux","arch":"x86_64"}}"#
        ),
    )
    .expect("write the identity");
    host.save_providers(
        &"[providers.fake]\nlabel = \"Fake box\"\n\
          params = [{ name = \"zone\", prompt = \"Zone\" }, { name = \"size\", prompt = \"Size\", default = \"small\" }]\n\
          create = 'printf %s/%s \"$HUMATA_ZONE\" \"$HUMATA_SIZE\" > __DIR__/created'\n\
          identity = 'cat __DIR__/identity.json'\n"
            .replace("__DIR__", &dir.display().to_string()),
    );
    let (clients, _machines) = runtime.block_on(host.clients());
    let mut app = App::open(&runtime, clients);

    app.open_add();
    app.click(Role::Button, "Host");
    assert!(app.shows(Role::ListBoxOption, "Fake box"), "the pop-up lists the provider");
    app.click(Role::ListBoxOption, "Fake box");
    assert!(app.shows(Role::TextInput, "Zone"), "the provider's params show");
    assert!(app.shows(Role::TextInput, "Size"), "the provider's params show");
    app.fill("Zone", "eu-1", |app| app.add_machine_text().contains("eu-1"));
    app.fill("Machine name", NAME, |app| app.ui.read(|state| state.add_machine.name == NAME));
    app.click(Role::Button, "Add");

    app.wait_until("the dialog to close on the added machine", |app| !app.dialog_open());
    app.wait_until("box-1 to be listed online in the app", |app| {
        app.machine_named(NAME).is_some_and(|(_, online)| online)
    });
    let (id, _) = app.machine_named(NAME).expect("listed after the wait");
    assert_eq!(
        std::fs::read_to_string(dir.join("created")).expect("the provider's create ran"),
        "eu-1/small",
        "the typed param and the default reached the provider"
    );
    assert_eq!(app.ui.read(|state| state.hosts.selected.clone()), Some(id));
    assert!(
        machine.installed("humatad2").is_some(),
        "the machine the provider made got the daemon"
    );
    drop(app);
    drop(host);
}

/// Flow 05 step 9, the login path: a provider whose connect asks for a code
/// offers Open interactively; the terminal shows the prompt, the keys typed
/// reach it (a wrong code would be denied), Done, I'm logged in ends the
/// login, the unattended offer follows, and Make it unattended closes the
/// dialog on the listed machine.
#[test]
fn a_login_by_hand_in_the_dialog_leads_to_the_unattended_offer_and_the_machine() {
    let runtime = runtime();
    let machine = FakeMachine::new();
    let host = runtime.block_on(Host::start(&machine));
    let login = fake_login(host.dir.path());
    host.save_providers(&format!(
        "[providers.corp]\nlabel = \"Corp\"\ncreate = \"true\"\nconnect = \"{} {{cmd}}\"\n",
        login.display()
    ));
    let (clients, _machines) = runtime.block_on(host.clients());
    let mut app = App::open(&runtime, clients);

    app.open_add();
    app.click(Role::Button, "Host");
    app.click(Role::ListBoxOption, "Corp");
    app.fill("Machine name", NAME, |app| app.ui.read(|state| state.add_machine.name == NAME));
    app.click(Role::Button, "Add");
    app.wait_until("the add to ask for a login", |app| {
        app.ui.read(|state| state.add_machine.login_offer.is_some())
    });
    assert!(app.shows(Role::Button, "Open interactively"));
    assert!(
        !app.daemon_machines().iter().any(|(name, _)| name == NAME),
        "nothing is recorded until the login is done"
    );

    app.click(Role::Button, "Open interactively");
    app.wait_until("the terminal to show the code prompt", |app| {
        app.login_text().contains("Verification code")
    });
    assert!(app.shows(Role::Log, "Login terminal"));
    // Focus the terminal as a person does, then type the code and Enter.
    app.click(Role::Log, "Login terminal");
    app.type_text("123456");
    app.play(&Action::Key("enter".to_owned()));
    app.wait_until("the machine's shell prompt in the terminal", |app| {
        app.login_text().contains("$ ")
    });

    app.click(Role::Button, "Done, I'm logged in");
    app.wait_until("the unattended offer", |app| {
        app.ui.read(|state| state.add_machine.unattended.is_some())
    });
    assert!(app.shows(Role::RadioButton, "Add a Humata key to authorized_keys"));
    assert!(app.shows(Role::ListItem, "Log in: done"), "the login is its own step");

    app.click(Role::RadioButton, "Keep corp's cached token");
    app.click(Role::Button, "Make it unattended");
    app.wait_until("the dialog to close on the added machine", |app| !app.dialog_open());
    app.wait_until("box-1 to be listed in the app", |app| app.machine_named(NAME).is_some());
    let (id, _) = app.machine_named(NAME).expect("listed after the wait");
    assert_eq!(app.ui.read(|state| state.hosts.selected.clone()), Some(id));
    assert!(
        app.daemon_machines().iter().any(|(name, _)| name == NAME),
        "the daemon lists box-1"
    );
    drop(app);
    drop(host);
}

/// Flow 05 step 9, a failed add: with the machine's `~/.humata2` read-only the
/// add confirms the host key, then fails at Install with a card and Try
/// again; nothing is listed. Once the folder is writable again, Try again
/// finishes the add without another Add click (the reducer only retries the
/// operation it holds), and box-1 is listed online.
#[test]
fn an_add_over_ssh_fails_at_install_and_try_again_finishes_it() {
    let runtime = runtime();
    let machine = FakeMachine::new();
    let blocked = machine.home().join(".humata2");
    std::fs::create_dir_all(&blocked).expect("make the machine's folder");
    std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o555))
        .expect("make it read-only");
    let host = runtime.block_on(Host::start(&machine));
    let (clients, _machines) = runtime.block_on(host.clients());
    let mut app = App::open(&runtime, clients);

    app.start_add();
    app.click(Role::Button, "Add");
    app.wait_until("the host key card", |app| {
        app.ui.read(|state| state.add_machine.host_key.is_some())
    });
    app.click(Role::Button, "Confirm");
    app.wait_until("the add to fail", |app| app.failed_step().is_some());

    assert_eq!(
        app.failed_step(),
        Some(StepKind::Install),
        "the add fails at Install: {:?}, steps {:?}",
        app.ui.read(|state| format!("{:?}", state.add_machine.failure)),
        app.steps()
    );
    assert!(
        app.steps().contains(&(StepKind::Install, StepStatus::Failed)),
        "the failed step is marked in the list"
    );
    assert!(app.dialog_open(), "a failure keeps the dialog open");
    assert!(app.shows(Role::Button, "Try again"));
    assert!(
        !app.daemon_machines().iter().any(|(name, _)| name == NAME),
        "a failed add records no machine"
    );
    assert!(
        app.ui.read(|state| state.add_machine.operation.is_some()),
        "the add is an operation that can be retried"
    );

    // The person fixes it on the machine, then Try again.
    std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o755))
        .expect("make it writable again");
    app.click(Role::Button, "Try again");
    app.wait_until("the dialog to close on the added machine", |app| !app.dialog_open());
    app.wait_until("box-1 to be listed online in the app", |app| {
        app.machine_named(NAME).is_some_and(|(_, online)| online)
    });
    assert_eq!(
        app.daemon_machines().iter().filter(|(name, _)| name == NAME).count(),
        1,
        "box-1 is listed once"
    );
    assert!(machine.installed("humatad2").is_some());
    drop(app);
    drop(host);
}

// ---- A session on box-1 -----------------------------------------------------------

/// Stands in for an agent: answers every line typed with `got: <line>`, and
/// keeps each line in its conversation file. (As in live_flows/world.rs.)
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

/// Writes an agents file naming one scripted agent that keeps its
/// conversations in `conv`.
fn agents_file(dir: &Path, name: &str, conv: &Path) -> PathBuf {
    std::fs::create_dir_all(conv).expect("make the conversations folder");
    let script = dir.join(format!("{name}.sh"));
    std::fs::write(&script, agent_script(conv)).expect("write the agent");
    let spec = serde_json::json!([{
        "name": AGENT,
        "program": "/bin/sh",
        "start_args": [script, "--session-id", "{conversation}"],
        "resume_args": [script, "--resume", "{conversation}"],
        "conversation_file": format!("{}/{{conversation}}.jsonl", conv.display()),
    }]);
    let file = dir.join(format!("{name}.json"));
    std::fs::write(&file, spec.to_string()).expect("write the agents file");
    file
}

fn git(args: &[&str]) {
    let output = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The fake machine's own `humatad2` (machine role, a socket daemon of its
/// own with a scripted agent), started under the machine's home before the
/// host adds it, as daemon/humatad/tests/remote.rs does. The bundle's link
/// relays to it. Its holders and agents die with it.
struct Remote {
    child: Child,
    dir: tempfile::TempDir,
    socket: PathBuf,
    state: PathBuf,
    host_agents: PathBuf,
    template: String,
}

impl Remote {
    async fn start(machine: &FakeMachine) -> Self {
        let dir = tempfile::Builder::new()
            .prefix("h2lr")
            .tempdir_in("/tmp")
            .expect("make a temp folder");
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))
            .expect("restrict the temp folder");
        let root = dir.path();
        let host_agents = agents_file(root, "host-agent", &root.join("host-conv"));
        let box_agents = agents_file(root, "box-agent", &root.join("box-conv"));
        let remotes = root.join("remotes");
        std::fs::create_dir_all(&remotes).expect("make the remotes folder");
        let template = format!("file://{}/{{owner}}/{{name}}.git", remotes.display());
        // The machine's `~/.humata2` is a short folder: a holder's socket
        // must fit a socket address, and the fake machine's home is long.
        let short = root.join("h");
        std::fs::create_dir(&short).expect("make the machine's state parent");
        std::os::unix::fs::symlink(&short, machine.home().join(".humata2"))
            .expect("link the machine's .humata2");
        let socket = root.join("m.sock");
        let state = short.join("state");
        let child = Command::new(binary("HUMATAD2_RELEASE"))
            .args(["serve", "--role", "machine", "--socket"])
            .arg(&socket)
            .arg("--state")
            .arg(&state)
            .arg("--root")
            .arg(machine.home())
            .arg("--holder")
            .arg(binary("HUMATA2_HOLDER"))
            .arg("--shell")
            .arg("/bin/sh")
            .arg("--shell-arg=-i")
            .arg("--channels")
            .arg(machine.home().join("Humata"))
            .arg("--agents")
            .arg(&box_agents)
            .arg("--default-agent")
            .arg(AGENT)
            .arg("--remote-template")
            .arg(&template)
            .env("HOME", machine.home())
            .spawn()
            .expect("start the machine's humatad2");
        let remote = Self {
            child,
            dir,
            socket,
            state,
            host_agents,
            template,
        };
        let deadline = Instant::now() + Duration::from_secs(30);
        while tokio::net::UnixStream::connect(&remote.socket).await.is_err() {
            assert!(Instant::now() < deadline, "the machine's humatad2 never started listening");
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        remote
    }

    /// A bare remote `owner/name` on `main` with one commit; returns
    /// `owner/name`, the name a channel clones it by on both machines.
    fn repo(&self, owner: &str, name: &str) -> String {
        let root = self.dir.path();
        let bare = root.join("remotes").join(owner).join(format!("{name}.git"));
        std::fs::create_dir_all(&bare).expect("make the remote folder");
        let bare_text = bare.display().to_string();
        git(&["init", "--bare", "--initial-branch=main", &bare_text]);
        let work = root.join(format!("scratch-{owner}-{name}"));
        std::fs::create_dir_all(&work).expect("make the scratch folder");
        let work_text = work.display().to_string();
        git(&["init", "--initial-branch=main", &work_text]);
        std::fs::write(work.join("README.md"), format!("{owner}/{name}\n")).expect("write the readme");
        git(&["-C", &work_text, "add", "."]);
        git(&["-C", &work_text, "commit", "-m", "first"]);
        git(&["-C", &work_text, "push", &bare_text, "main"]);
        format!("{owner}/{name}")
    }

    /// Every conversation line the machine's agent kept, joined.
    fn box_conversations(&self) -> String {
        conversations(&self.dir.path().join("box-conv"))
    }

    /// Every conversation line the host's agent kept, joined: none, as the
    /// host runs nothing of box-1's sessions.
    fn host_conversations(&self) -> String {
        conversations(&self.dir.path().join("host-conv"))
    }
}

fn conversations(folder: &Path) -> String {
    std::fs::read_dir(folder)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
                .collect()
        })
        .unwrap_or_default()
}

impl Drop for Remote {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        // The holders and agents it started carry this folder in their
        // command lines.
        let _ = Command::new("pkill").arg("-f").arg(self.dir.path()).status();
    }
}

/// Macro case 03 steps 6 and 7: with box-1 added and in the channel, pick it
/// in the New session sheet, give a title and a first message, Start
/// session: the session is listed on box-1, opens, and its terminal, relayed
/// through the host, shows the agent's answer. The agent that ran is the
/// machine's: its conversation holds the message, the host's holds nothing.
#[test]
fn a_session_started_on_box_1_from_the_sheet_runs_there() {
    let runtime = runtime();
    let machine = FakeMachine::new();
    let remote = runtime.block_on(Remote::start(&machine));
    let repo = remote.repo("owner", "ledger");
    let host = runtime.block_on(Host::start_with(&machine, Some(&remote)));
    let (clients, machines) = runtime.block_on(host.clients_with_agent());
    let mut app = App::open(&runtime, clients);

    let box_1 = app.add_box_1(&machine);
    app.wait_linked(&machines, &box_1);
    let mac = app
        .ui
        .read(|state| {
            state
                .workspace
                .machines
                .iter()
                .find(|m| m.id != box_1)
                .map(|m| m.id.clone())
        })
        .expect("This Mac is listed");
    // The channel clones the repository on both machines before it answers.
    let daemon = app.daemon.clone();
    let channel: ChannelId = app
        .runtime
        .block_on(daemon.create_channel("oncall", vec![repo], vec![mac, box_1.clone()]))
        .expect("make the channel");
    app.wait_until("the channel to reach the app", |app| {
        app.ui.read(|state| state.workspace.channel(&channel).is_some())
    });

    app.send(AppAction::Navigate(Route::Channel(channel.clone())));
    app.send(AppAction::StartNewSession);
    let current = app
        .ui
        .read(|state| match state.overlays.last() {
            Some(Overlay::NewSession(sheet)) => sheet.machine.clone(),
            _ => None,
        })
        .expect("the sheet picks a machine");
    let current_name = app
        .ui
        .read(|state| state.workspace.machine(&current).map(|m| m.name.clone()))
        .expect("the picked machine is listed");
    app.click(Role::Button, &current_name);
    app.click(Role::Button, NAME);
    let on_box = |app: &App| {
        app.ui.read(|state| match state.overlays.last() {
            Some(Overlay::NewSession(sheet)) => sheet.machine.as_ref() == Some(&box_1),
            _ => false,
        })
    };
    app.wait_until("box-1 to become the session's machine", |app| on_box(app));

    app.fill_sheet("Title", "Dig into errors", |sheet| sheet.title == "Dig into errors");
    app.click(Role::MultilineTextInput, "First message");
    app.type_text("dig into the 02:10 errors");
    app.wait_until("the typed first message to reach the sheet", |app| {
        app.ui.read(|state| match state.overlays.last() {
            Some(Overlay::NewSession(sheet)) => sheet.first_message == "dig into the 02:10 errors",
            _ => false,
        })
    });
    app.click(Role::Button, "Start session");

    app.wait_until("the session to be listed", |app| {
        app.session_titled("Dig into errors").is_some()
    });
    let session = app.session_titled("Dig into errors").expect("listed after the wait");
    assert_eq!(session.machine, box_1, "the session is recorded on box-1");
    app.wait_until("the session page to show", |app| {
        app.ui.read(|state| state.route == Route::Session(session.id.clone()))
    });
    app.wait_until("the agent's answer in its terminal", |app| {
        app.view().contains("got: dig into the 02:10 errors")
    });
    let live = app
        .session_titled("Dig into errors")
        .map(|session| !matches!(session.status, Status::Exited { .. } | Status::Offline))
        .unwrap_or(false);
    assert!(live, "the session shows as running");
    assert!(
        remote.box_conversations().contains("dig into the 02:10 errors"),
        "the message reached the agent on the machine"
    );
    assert!(
        remote.host_conversations().is_empty(),
        "the host ran no agent of its own"
    );
    drop(app);
    drop(host);
    drop(remote);
}

impl App<'_> {
    /// The session the app lists under `title`.
    fn session_titled(&self, title: &str) -> Option<humata2_clients::model::Session> {
        self.ui.read(|state| {
            state
                .workspace
                .sessions
                .iter()
                .find(|session| session.title == title)
                .cloned()
        })
    }

    /// Clicks a field of the New session sheet, types into it and waits for
    /// the sheet to hold it.
    fn fill_sheet(&mut self, label: &str, text: &str, reached: impl Fn(&humata2_app::NewSession) -> bool) {
        self.click(Role::TextInput, label);
        self.type_text(text);
        self.wait_until(&format!("{label} to reach the sheet"), |app| {
            app.ui.read(|state| match state.overlays.last() {
                Some(Overlay::NewSession(sheet)) => reached(sheet),
                _ => false,
            })
        });
    }
}
