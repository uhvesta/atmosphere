//! SPEC flows 26.2 (start a session, on This Mac), 26.3 (review a 3-repo
//! diff, in `live_flows/review.rs`) and 26.8 (Mac reboot, the local part) against a real `humatad2` with a test agent that answers
//! every line with `got: <line>`. The app is headless: no window is shown.
//!
//! Each test makes its own daemon in its own temp folder.
#![cfg(target_os = "macos")]

#[path = "live_flows/away_box.rs"]
mod away_box;
#[path = "live_flows/board_drawer.rs"]
mod board_drawer;
#[path = "live_flows/board_conflict.rs"]
mod board_conflict;
#[path = "live_flows/board_revision.rs"]
mod board_revision;
#[path = "live_flows/channel_desk.rs"]
mod channel_desk;
#[path = "live_flows/channel_page.rs"]
mod channel_page;
#[path = "live_flows/channel_marks.rs"]
mod channel_marks;
#[path = "live_flows/continue_in.rs"]
mod continue_in;
#[path = "live_flows/desk.rs"]
mod desk;
#[path = "live_flows/desk_form.rs"]
mod desk_form;
#[path = "live_flows/desk_document.rs"]
mod desk_document;
#[path = "live_flows/document.rs"]
mod document;
#[path = "live_flows/files.rs"]
mod files;
#[path = "live_flows/find.rs"]
mod find;
#[path = "live_flows/find_pins.rs"]
mod find_pins;
#[path = "live_flows/fork_kinds.rs"]
mod fork_kinds;
#[path = "live_flows/ime.rs"]
mod ime;
#[path = "live_flows/keys.rs"]
mod keys;
#[path = "live_flows/lifecycle.rs"]
mod lifecycle;
#[path = "live_flows/link_drop.rs"]
mod link_drop;
#[path = "live_flows/navigation_keys.rs"]
mod navigation_keys;
#[path = "live_flows/new_channel.rs"]
mod new_channel;
#[path = "live_flows/new_channel_sheet.rs"]
mod new_channel_sheet;
#[path = "live_flows/queue.rs"]
mod queue;
#[path = "live_flows/repo_library.rs"]
mod repo_library;
#[path = "live_flows/session_add_repo.rs"]
mod session_add_repo;
#[path = "live_flows/channel_add_repo.rs"]
mod channel_add_repo;
#[path = "live_flows/remote.rs"]
mod remote;
#[path = "live_flows/restart.rs"]
mod restart;
#[path = "live_flows/pr_review.rs"]
mod pr_review;
#[path = "live_flows/review.rs"]
mod review;
/// REVIEWER exit: a reviewer is a tab in the session's own strip (D849).
#[path = "live_flows/reviewer.rs"]
mod reviewer;
#[path = "live_flows/stop_and_start.rs"]
mod stop_and_start;
#[path = "live_flows/tabs_and_palette.rs"]
mod tabs_and_palette;
#[path = "live_flows/targets.rs"]
mod targets;
#[path = "live_flows/views.rs"]
mod views;
#[path = "live_flows/views_remote.rs"]
mod views_remote;
#[path = "live_flows/terminals.rs"]
mod terminals;
#[path = "live_flows/shell_screen.rs"]
mod shell_screen;
#[path = "live_flows/two_viewers.rs"]
mod two_viewers;
#[path = "live_flows/typing.rs"]
mod typing;
#[path = "live_flows/leftovers.rs"]
mod leftovers;
#[path = "live_flows/world.rs"]
mod world;
#[path = "live_flows/real_vm.rs"]
mod real_vm;

use std::collections::VecDeque;
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

/// The app's shortcut for ⌘`key`: Cmd on macOS, Ctrl+Shift elsewhere (D318).
fn app_key(key: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("cmd-{key}")
    } else {
        format!("ctrl-shift-{key}")
    }
}

const DEADLINE: Duration = Duration::from_secs(60);

/// The longest one pass of a settle runs the app's queued work. The app
/// is never idle while a program floods its terminal (`yes`), so a drain
/// until parked would never end and no wait would reach its deadline.
/// Normal work drains in milliseconds; the rest is left for the next pass.
const DRAIN_BUDGET: Duration = Duration::from_secs(2);

/// The case's runtime. Every case makes one first, so this is where its
/// watch begins: a case still running past the limit fails there, whatever
/// it made its daemon with (`leftovers.rs`).
fn runtime() -> Runtime {
    leftovers::case_begins();
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
}

const RECENT: usize = 40;

impl Heard {
    fn hear(&mut self, event: &Event) {
        let summary = match event {
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
    /// The thread that opened the app: the one gpui wakes when work comes
    /// in from another thread, and the one the waits park (D399).
    thread: std::thread::ThreadId,
    /// The window's view, for what only it reaches (D484).
    shell: gpui_kit::Entity<humata2_ui::Shell>,
    /// How many drains stopped at [`DRAIN_BUDGET`] with work still queued,
    /// for a timeout's diagnosis.
    unfinished_drains: usize,
}

impl<'r> App<'r> {
    fn open(runtime: &'r Runtime, clients: Clients) -> Self {
        let daemon = clients.daemon.clone();
        let heard = Arc::new(Mutex::new(Heard::default()));
        {
            let daemon = daemon.clone();
            let heard = heard.clone();
            // Every event from the host wakes a wait: what it looks at may
            // be the host's, not the app's (D399).
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
        let (window, shell) = cx
            .update(|cx| humata2_ui::open(cx, &ui, options))
            .expect("the window opens");
        let mut app = Self {
            runtime,
            cx,
            window,
            ui,
            daemon,
            heard,
            thread: std::thread::current().id(),
            shell,
            unfinished_drains: 0,
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
            self.drain();
        }
        self.cx
            .update_window(self.window, |_, window, cx| {
                humata2_engine::frame::natural_frame(window, cx);
            })
            .expect("the window stays open");
    }

    /// Runs the app's queued work until none is left, as
    /// `run_until_parked` does, or until [`DRAIN_BUDGET`] passes.
    fn drain(&mut self) {
        let until = Instant::now() + DRAIN_BUDGET;
        while self.cx.background_executor().tick() {
            if Instant::now() >= until {
                self.unfinished_drains += 1;
                return;
            }
        }
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

    /// Moves the pointer over a control that may show only while hovered.
    fn hover(&mut self, role: Role, label: &str) {
        self.play(&Action::Hover(Target::new(role, label)));
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
    fn wait_until(&mut self, what: &str, done: impl FnMut(&mut Self) -> bool) {
        self.wait_within(what, DEADLINE, done);
    }

    /// Settles until `done` holds or `deadline` passes. Between looks the
    /// thread parks until something wakes it (D399): gpui, when another
    /// thread hands the app work; an event from the host; an agent's
    /// conversation write. What `done` looks at must come with one of them.
    /// It also wakes every 16 ms, a display's frame tick, so work waiting
    /// for the next frame (a terminal's flood, a batch a frame) gets one.
    fn wait_within(&mut self, what: &str, deadline: Duration, mut done: impl FnMut(&mut Self) -> bool) {
        assert_eq!(
            std::thread::current().id(),
            self.thread,
            "waits run on the thread that opened the app, which is the one woken"
        );
        let until = Instant::now() + deadline;
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
            // A display draws about every 16 ms whether or not anything
            // woke us: work waiting for the next frame (a terminal's full
            // batch) needs one.
            std::thread::park_timeout(left.min(Duration::from_millis(16)));
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
        if self.unfinished_drains > 0 {
            text.push_str(&format!(
                "{} drains stopped after {} s each with the app's work still queued: something kept it busy\n",
                self.unfinished_drains,
                DRAIN_BUDGET.as_secs()
            ));
        }
        let focused = self
            .cx
            .update_window(self.window, |_, window, cx| {
                window.focused(cx).map(|handle| format!("{handle:?}"))
            })
            .ok()
            .flatten();
        text.push_str(&format!("focused {focused:?}\nview:\n{}\n", self.view()));
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
    session_in(world, clients, &channel, repo, title, first).await
}

/// Starts a session in `channel` through the daemon and waits until the
/// agent has taken its first message.
async fn session_in(
    world: &World,
    clients: &Clients,
    channel: &ChannelId,
    repo: String,
    title: &str,
    first: &str,
) -> SessionId {
    let mac = World::this_mac(clients)
        .await
        .expect("the workspace has This Mac");
    let id = clients
        .daemon
        .create_session(
            channel,
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
    agent_has(world, first).await;
    id
}

/// Waits until an agent's conversation holds `first`, woken by each write
/// to it (D399). The daemon clones a session's repos before it launches
/// the agent, so once this returns the clones are there (D441).
async fn agent_has(world: &World, first: &str) {
    let until = tokio::time::Instant::now() + DEADLINE;
    while !world.conversation_text().contains(first) {
        let woke = tokio::time::timeout_at(until, world.wrote()).await;
        assert!(woke.is_ok(), "the agent never got its first message");
    }
}

async fn session_now(daemon: &Arc<dyn Daemon>, id: &SessionId) -> Option<Session> {
    let workspace = daemon.load().await.ok()?;
    workspace.sessions.iter().find(|s| &s.id == id).cloned()
}

/// D399: work a tokio task hands the app wakes a parked wait. Waits park
/// with no clock between looks, so this is what keeps them from sleeping
/// to their deadline.
#[test]
fn work_handed_to_the_app_from_another_thread_wakes_a_wait() {
    let runtime = runtime();
    let (_world, clients) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        (world, clients)
    });
    let mut app = App::open(&runtime, clients);
    let ran = Rc::new(std::cell::Cell::new(false));
    let (give, take) = tokio::sync::oneshot::channel::<()>();
    {
        let ran = ran.clone();
        app.cx.update(|cx| {
            cx.foreground_executor()
                .spawn(async move {
                    if take.await.is_ok() {
                        ran.set(true);
                    }
                })
                .detach();
        });
    }
    // Handed over once the wait below has parked, from a tokio worker.
    runtime.spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let _ = give.send(());
    });
    app.wait_until("the app to run the work it was handed", |_| ran.get());
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
    // The sheet picks its channel and machine from what the app has
    // loaded, so wait for the first load to bring them.
    app.wait_until("the channel and its machine to load", |app| {
        app.ui.read(|state| {
            state
                .workspace
                .channels
                .first()
                .is_some_and(|channel| state.workspace.channel_machines(&channel.id).any(|m| m.online))
        })
    });

    // New session opens the sheet on the first channel and This Mac. The
    // sidebar and the Home page both have a "New session" button, so this
    // takes the shortcut, as a person can.
    app.play(&Action::Key(app_key("n")));
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

/// The app quits and starts again against the same daemon: the session's
/// tab reattaches the holder it had (the agent did not start again), and
/// shows what the agent printed while no app was attached.
#[test]
fn after_the_app_restarts_the_session_tab_shows_what_was_printed_while_it_was_gone() {
    let runtime = runtime();
    let (world, id, clients) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        let id = running_session(&world, &clients, "Fix the ledger", "before the restart").await;
        (world, id, clients)
    });
    let mut app = App::open(&runtime, clients);
    app.show_session(&id);
    app.wait_until("the agent's first answer in its terminal", |app| {
        app.view().contains("got: before the restart")
    });
    drop(app);

    // No app is attached now: a bare client types a line the agent answers.
    let clients = runtime.block_on(async {
        let clients = world.clients().await.expect("connect again");
        clients
            .terminals
            .write(&id, 0, "while the app was gone")
            .await
            .expect("type into the tab");
        clients
    });
    let until = tokio::time::Instant::now() + DEADLINE;
    while !world.conversation_text().contains("while the app was gone") {
        let woke = runtime.block_on(async { tokio::time::timeout_at(until, world.wrote()).await });
        assert!(woke.is_ok(), "the agent never got the line typed while the app was gone");
    }

    let mut app = App::open(&runtime, clients);
    app.show_session(&id);
    app.wait_until("the answer printed while the app was gone", |app| {
        app.view().contains("got: while the app was gone")
    });
    let view = app.view();
    assert!(view.contains("got: before the restart"), "the screen kept what came before");
    assert!(
        view.contains("agent up --session-id") && !view.contains("agent up --resume"),
        "the same agent is still running, not a new one"
    );
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
