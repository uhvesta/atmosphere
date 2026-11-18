//! A session's terminals draw through the reusable replica, fed by each
//! tab's link (task 5 of TERM). Headless on the fakes; nothing is shown.
//! The screen, the fitted grid and typing are checked as a person meets them:
//! by what the window shows and what the fake program received.

use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::{AnyWindowHandle, Bounds, HeadlessAppContext, WindowBounds, WindowOptions, point, px, size};
use humata2_app::{AppAction, Route};
use humata2_clients::model::{Agent, SessionId, Status};
use humata2_clients::{Clients, Workspace, fixture};
use humata2_engine::Resources;
use humata2_flow::{Action, Driver as _, FrameMode, GpuiDriver};
use humata2_session::SessionAction;
use humata2_ui::bridge::Ui;
use humata2_ui::record::Recorder;
use tca::TestClock;
use tokio::runtime::Runtime;

struct App {
    runtime: Runtime,
    cx: HeadlessAppContext,
    window: AnyWindowHandle,
    ui: Rc<Ui>,
    fakes: humata2_clients::Fakes,
}

impl App {
    fn new(workspace: Workspace) -> Self {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a current-thread runtime starts");
        let mut cx = humata2_engine::headless(&Resources::new(humata2_ui::Assets))
            .expect("the headless context starts");
        cx.update(humata2_ui::init);
        let (clients, fakes) = Clients::fake(workspace, Arc::new(TestClock::new()));
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
        let mut app = Self { runtime, cx, window, ui, fakes };
        // Headless windows start inactive; focus follows only an active one.
        app.cx
            .update_window(app.window, |_, window, _| window.activate_window())
            .expect("the window is open");
        app.settle();
        app
    }

    fn settle(&mut self) {
        for _ in 0..10 {
            self.runtime.block_on(async {
                for _ in 0..32 {
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

    fn send(&mut self, action: AppAction) {
        self.cx.update(|cx| self.ui.send(action, cx));
        self.settle();
    }

    fn open(&mut self, session: &str) {
        self.send(AppAction::Navigate(Route::Session(SessionId::new(session))));
    }

    fn play(&mut self, action: &Action) {
        GpuiDriver::with_frames(&mut self.cx, self.window, FrameMode::Natural)
            .perform(action)
            .expect("the step can be played");
        self.settle();
    }

    /// Types `text`, then Enter, into whatever has keyboard focus.
    fn type_line(&mut self, text: &str) {
        self.play(&Action::Type(text.to_owned()));
        self.play(&Action::Key("enter".to_owned()));
    }

    /// The window's accessible tree, as text.
    fn view(&mut self) -> String {
        GpuiDriver::with_frames(&mut self.cx, self.window, FrameMode::Natural)
            .view()
            .expect("the view can be read")
    }

    /// What the daemon's fake program has printed so far on a tab.
    fn printed(&self, session: &str, tab: usize) -> Vec<u8> {
        self.fakes
            .daemon
            .workspace()
            .terminal_bytes
            .get(&(SessionId::new(session), tab))
            .cloned()
            .unwrap_or_default()
    }
}

/// The rows the terminal shows, as its accessible description gives them.
fn shown_rows(view: &str) -> Vec<String> {
    let marker = "Log \"Terminal output\" description=\"";
    let start = view.find(marker).expect("the view has a terminal") + marker.len();
    let rest = &view[start..];
    let end = rest.find("\"\n").expect("the description ends");
    rest[..end].split("\\n").map(str::to_owned).collect()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|window| window == needle)
}

/// SE-1: a session's agent terminal shows its screen, drawn by the replica.
#[test]
fn a_session_tab_shows_its_screen_through_the_replica() {
    // A working agent: its screen ends in its progress line.
    let mut app = App::new(fixture::design_session(Status::Working));
    app.open("ledger");
    let view = app.view();
    assert!(view.contains("#terminal-replica"), "the replica draws the tab");
    assert!(
        shown_rows(&view).iter().any(|row| row.contains("Backfilling entries")),
        "the fixture's screen is on show: {view}"
    );
}

/// The grid is the size the view reported, and the agent has drawn its input
/// box on the last row of it, as a program does when its terminal is resized.
#[test]
fn the_grid_has_the_fitted_size_and_the_agent_redraws_to_it() {
    // A working agent draws its input box; one waiting on a menu doesn't.
    let mut app = App::new(fixture::design_session(Status::Working));
    app.open("ledger");
    let (_, rows) = app
        .fakes
        .daemon
        .workspace()
        .terminal_sizes
        .get(&(SessionId::new("ledger"), 0))
        .copied()
        .expect("the view reported the size of its grid");
    let shown = shown_rows(&app.view());
    assert_eq!(shown.len(), usize::from(rows), "the grid has the rows the view asked for");
    assert!(
        shown.last().is_some_and(|row| row.contains("accept edits")),
        "the hint is on the last row: {shown:?}"
    );
}

/// SE-10: what is typed in a shell tab reaches that tab's program over its
/// link, and what the program prints comes back and is drawn.
#[test]
fn typing_in_a_new_shell_reaches_its_program_and_the_answer_is_drawn() {
    let mut app = App::new(fixture::design());
    app.open("ledger");
    app.send(AppAction::Session(SessionAction::OpenTerminal {
        agent: Agent::Shell,
        split: false,
    }));
    let tab = app.ui.read(|s| s.session.view().expect("the session is open").tab);
    let before = app.printed("ledger", tab);
    app.type_line("pwd");
    let after = app.printed("ledger", tab);
    assert!(contains(&after[before.len()..], b"~/humata"), "the program received what was typed");
    assert!(
        shown_rows(&app.view()).iter().any(|row| row.contains("~/humata")),
        "the answer is drawn"
    );
}

/// A program that has exited takes no input: nothing typed reaches it.
#[test]
fn typing_into_an_exited_program_sends_nothing() {
    let mut app = App::new(fixture::design_session(Status::Exited { code: 0 }));
    app.open("ledger");
    let before = app.printed("ledger", 0);
    app.type_line("pwd");
    assert_eq!(app.printed("ledger", 0), before);
}
