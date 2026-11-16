//! SPEC flows 26.2 (start a session, on This Mac) and 26.8 (Mac reboot,
//! the local part) against a real `humatad2` with a test agent that answers
//! every line with `got: <line>`. The app is headless: no window is shown.
//!
//! Each test makes its own daemon in its own temp folder.
#![cfg(target_os = "macos")]

#[path = "live_flows/files.rs"]
mod files;
#[path = "live_flows/world.rs"]
mod world;

use std::collections::{BTreeMap, VecDeque};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui_kit::{
    AnyWindowHandle, Bounds, HeadlessAppContext, Role, WindowBounds, WindowOptions, point, px, size,
};
use humata2_app::{AppAction, NewSession, Overlay, Route};
use humata2_clients::model::{
    Agent, ChannelId, Recovery, Session, SessionId, SessionOptions, Status,
};
use humata2_clients::{Clients, Daemon, Event};
use humata2_engine::Resources;
use humata2_flow::{Action, Driver as _, FrameMode, GpuiDriver, Target};
use humata2_ui::bridge::Ui;
use humata2_ui::record::Recorder;
use tokio::runtime::Runtime;

use world::World;

const DEADLINE: Duration = Duration::from_secs(60);

fn runtime() -> Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("a runtime starts")
}

/// What the client pushed to the app, kept to explain a timeout.
#[derive(Default)]
struct Heard {
    /// The last few events, summed up.
    recent: VecDeque<String>,
    /// Terminal bytes by (session, tab): how many, and the last of them.
    bytes: BTreeMap<(String, usize), (usize, Vec<u8>)>,
}

const RECENT: usize = 40;
const TAIL: usize = 400;

impl Heard {
    fn hear(&mut self, event: &Event) {
        let summary = match event {
            Event::TerminalData {
                session,
                tab,
                offset,
                bytes,
            } => {
                let (count, tail) = self
                    .bytes
                    .entry((session.as_str().to_owned(), *tab))
                    .or_default();
                *count += bytes.len();
                tail.extend_from_slice(bytes);
                let extra = tail.len().saturating_sub(TAIL);
                tail.drain(..extra);
                format!(
                    "TerminalData {} tab {tab} at {offset}: {} bytes",
                    session.as_str(),
                    bytes.len()
                )
            }
            Event::Session(session) => format!(
                "Session {} {:?} status {:?} recovery {:?}",
                session.id.as_str(),
                session.title,
                session.status,
                session.recovery
            ),
            other => format!("{other:?}").chars().take(200).collect(),
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
        for ((session, tab), (count, tail)) in &self.bytes {
            text.push_str(&format!(
                "terminal {session} tab {tab}: {count} bytes, ending {:?}\n",
                String::from_utf8_lossy(tail)
            ));
        }
        text
    }
}

/// The app, headless, on the clients it was given.
struct App<'r> {
    runtime: &'r Runtime,
    cx: HeadlessAppContext,
    window: AnyWindowHandle,
    ui: Rc<Ui>,
    daemon: Arc<dyn Daemon>,
    heard: Arc<Mutex<Heard>>,
}

impl<'r> App<'r> {
    fn open(runtime: &'r Runtime, clients: Clients) -> Self {
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

    /// What the window shows, as text.
    fn view(&mut self) -> String {
        GpuiDriver::with_frames(&mut self.cx, self.window, FrameMode::Natural)
            .view()
            .expect("the view can be read")
    }

    /// Whether the New session sheet is open and `holds` is true of it.
    fn sheet_holds(&self, holds: impl FnOnce(&NewSession) -> bool) -> bool {
        self.ui.read(|state| match state.overlays.last() {
            Some(Overlay::NewSession(sheet)) => holds(sheet),
            _ => false,
        })
    }

    fn session_titled(&self, title: &str) -> Option<Session> {
        self.ui.read(|state| {
            state
                .workspace
                .sessions
                .iter()
                .find(|session| session.title == title)
                .cloned()
        })
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

    /// What the app, the client and the daemon hold now, for a timeout.
    fn diagnose(&mut self) -> String {
        let mut text = self.ui.read(|state| {
            let mut text = format!(
                "route {:?}\nload {:?}\noverlays {:?}\ntoasts {:?}\nsession error {:?}\nrestore error {:?}\n",
                state.route,
                state.load,
                state.overlays,
                state.toasts,
                state.session.error,
                state.restore_error
            );
            for session in &state.workspace.sessions {
                text.push_str(&format!(
                    "app session {} {:?} status {:?} recovery {:?}\n",
                    session.id.as_str(),
                    session.title,
                    session.status,
                    session.recovery
                ));
            }
            for ((session, tab), bytes) in &state.session.terminal_bytes {
                text.push_str(&format!(
                    "app terminal {} tab {tab}: {} bytes\n",
                    session.as_str(),
                    bytes.len()
                ));
            }
            text
        });
        if let Ok(heard) = self.heard.lock() {
            text.push_str(&heard.describe());
        }
        let daemon = self.daemon.clone();
        match self.runtime.block_on(daemon.load()) {
            Ok(workspace) => {
                for session in &workspace.sessions {
                    text.push_str(&format!(
                        "daemon session {} {:?} status {:?} recovery {:?}\n",
                        session.id.as_str(),
                        session.title,
                        session.status,
                        session.recovery
                    ));
                }
            }
            Err(error) => text.push_str(&format!("daemon load failed: {error}\n")),
        }
        text.push_str(&format!("view:\n{}\n", self.view()));
        text
    }

    fn show_session(&mut self, id: &SessionId) {
        let id = id.clone();
        self.cx
            .update(|cx| self.ui.send(AppAction::Navigate(Route::Session(id)), cx));
        self.settle();
    }
}

async fn channel_with_repo(world: &World, clients: &Clients, name: &str) -> (ChannelId, String) {
    let mac = World::this_mac(clients)
        .await
        .expect("the workspace has This Mac");
    let repo = world.repo("owner", "ledger").expect("make a repo");
    let channel = clients
        .daemon
        .create_channel(name, vec![repo.clone()], vec![mac])
        .await
        .expect("create the channel");
    (channel, repo)
}

/// Starts a session through the daemon (not the UI) and waits until the
/// agent has taken its first message.
async fn running_session(world: &World, clients: &Clients, title: &str, first: &str) -> SessionId {
    let (channel, repo) = channel_with_repo(world, clients, "payments").await;
    let mac = World::this_mac(clients)
        .await
        .expect("the workspace has This Mac");
    let id = clients
        .daemon
        .create_session(
            &channel,
            Agent::Claude,
            &mac,
            title,
            SessionOptions {
                repos: vec![repo],
                first_message: first.to_owned(),
                ..SessionOptions::default()
            },
        )
        .await
        .expect("create the session");
    let until = Instant::now() + DEADLINE;
    while !world.conversation_text().contains(first) {
        assert!(Instant::now() < until, "the agent never got its first message");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    id
}

async fn session_now(daemon: &Arc<dyn Daemon>, id: &SessionId) -> Option<Session> {
    let workspace = daemon.load().await.ok()?;
    workspace.sessions.iter().find(|s| &s.id == id).cloned()
}

/// 26.2: New session, a title, the defaults, a first message, Start session.
#[test]
fn a_session_started_from_new_session_runs_and_answers_its_first_message() {
    let runtime = runtime();
    let (world, clients) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        channel_with_repo(&world, &clients, "payments").await;
        (world, clients)
    });
    let mut app = App::open(&runtime, clients);

    // New session opens the sheet on the first channel and This Mac. The
    // sidebar and the Home page both have a "New session" button, so this
    // takes the shortcut, as a person can.
    app.play(&Action::Key("secondary-n".to_owned()));
    assert!(
        app.ui
            .read(|state| matches!(state.overlays.last(), Some(Overlay::NewSession(_)))),
        "New session opens the sheet"
    );
    app.click(Role::TextInput, "Title");
    app.type_text("Fix the ledger");
    // What was typed must reach the sheet before Start session, or the
    // session starts under the default title and the wait below can only
    // say it timed out.
    app.wait_until("the typed title to reach the sheet", |app| {
        app.sheet_holds(|sheet| sheet.title == "Fix the ledger")
    });
    app.click(Role::MultilineTextInput, "First message");
    app.type_text("hello agent");
    app.wait_until("the typed first message to reach the sheet", |app| {
        app.sheet_holds(|sheet| sheet.first_message == "hello agent")
    });
    app.click(Role::Button, "Start session");

    app.wait_until("the session to be listed", |app| {
        app.session_titled("Fix the ledger").is_some()
    });
    let session = app
        .session_titled("Fix the ledger")
        .expect("listed after the wait");
    app.wait_until("the session page to show", |app| {
        app.ui
            .read(|state| state.route == Route::Session(session.id.clone()))
    });
    app.wait_until("the agent's answer in its terminal", |app| {
        app.view().contains("got: hello agent")
    });
    assert!(app.view().contains("Fix the ledger"));
    assert!(
        !app.ui
            .read(|state| matches!(state.overlays.last(), Some(Overlay::NewSession(_)))),
        "the sheet closes once the session starts"
    );
    drop(app);
    drop(world);
}

/// 26.8: the daemon and every holder die at once; the daemon starts again
/// on the same folder and resumes the agent in its conversation.
#[test]
fn after_a_reboot_the_session_is_running_again_in_its_conversation() {
    let runtime = runtime();
    let (mut world, id) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        let id = running_session(&world, &clients, "Fix the ledger", "before the reboot").await;
        (world, id)
    });

    let clients = runtime.block_on(async {
        world.reboot().await.expect("reboot");
        world.clients().await.expect("connect again")
    });
    let daemon = clients.daemon.clone();
    let mut app = App::open(&runtime, clients);
    app.show_session(&id);
    app.wait_until("the resumed agent in its terminal", |app| {
        app.view().contains("agent up --resume")
    });

    assert!(world.conversation_text().contains("before the reboot"));
    let session = runtime
        .block_on(session_now(&daemon, &id))
        .expect("the session is still listed");
    assert!(
        !matches!(session.status, Status::Exited { .. } | Status::Offline),
        "the session runs again, not {:?}",
        session.status
    );
    assert!(!matches!(
        session.recovery,
        Some(Recovery::Recoverable { .. })
    ));
    drop(app);
    drop(world);
}

/// 26.8: a conversation lost in the reboot can't be resumed; the session
/// says so in the model (the daemon's ResumeFailed shows as an exited
/// session needing recovery) and nothing runs in its place.
#[test]
fn a_session_whose_conversation_is_lost_cannot_resume() {
    let runtime = runtime();
    let (mut world, id) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        let id = running_session(&world, &clients, "Fix the ledger", "before the reboot").await;
        (world, id)
    });

    world.lose_conversations().expect("delete conversations");
    let clients = runtime.block_on(async {
        world.reboot().await.expect("reboot");
        world.clients().await.expect("connect again")
    });
    let mut app = App::open(&runtime, clients);
    app.wait_until("the session to show it can't resume", |app| {
        app.session_titled("Fix the ledger").is_some_and(|session| {
            matches!(session.recovery, Some(Recovery::Recoverable { .. }))
        })
    });
    let session = app
        .session_titled("Fix the ledger")
        .expect("listed after the wait");
    assert_eq!(session.id, id);
    assert!(matches!(session.status, Status::Exited { .. }));
    drop(app);
    drop(world);
}
