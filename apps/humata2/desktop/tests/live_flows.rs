//! SPEC flows 26.2 (start a session, on This Mac) and 26.8 (Mac reboot,
//! the local part) against a real `humatad2` with a test agent that answers
//! every line with `got: <line>`. The app is headless: no window is shown.
//!
//! Each test makes its own daemon in its own temp folder.
#![cfg(target_os = "macos")]

#[path = "live_flows/world.rs"]
mod world;

use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::{
    AnyWindowHandle, Bounds, HeadlessAppContext, Role, WindowBounds, WindowOptions, point, px, size,
};
use humata2_app::{AppAction, Overlay, Route};
use humata2_clients::model::{
    Agent, ChannelId, Recovery, Session, SessionId, SessionOptions, Status,
};
use humata2_clients::{Clients, Daemon};
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

/// The app, headless, on the clients it was given.
struct App<'r> {
    runtime: &'r Runtime,
    cx: HeadlessAppContext,
    window: AnyWindowHandle,
    ui: Rc<Ui>,
}

impl<'r> App<'r> {
    fn open(runtime: &'r Runtime, clients: Clients) -> Self {
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
            assert!(Instant::now() < until, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(100));
        }
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
    app.click(Role::MultilineTextInput, "First message");
    app.type_text("hello agent");
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
