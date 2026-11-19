//! Changing a machine's Humata folder as a person does it, headless on the
//! fakes (SPEC PL-1, FC-10, ST-5): Settings › Machines, Change…, type the
//! folder, Plan, then choose. Nothing changes before the choice. No window
//! is shown.

use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::{
    AnyWindowHandle, Bounds, HeadlessAppContext, Role, WindowBounds, WindowOptions, point, px, size,
};
use humata2_app::{AppAction, Route};
use humata2_clients::machines::{Call, FakeMachines};
use humata2_clients::model::{MachineId, RootPlan};
use humata2_clients::{Clients, fixture};
use humata2_engine::Resources;
use humata2_flow::{Action, Driver as _, FrameMode, GpuiDriver, Target};
use humata2_ui::bridge::Ui;
use humata2_ui::record::Recorder;
use tca::TestClock;
use tokio::runtime::Runtime;

struct App {
    runtime: Runtime,
    cx: HeadlessAppContext,
    window: AnyWindowHandle,
    ui: Rc<Ui>,
    fake: FakeMachines,
}

fn mac() -> MachineId {
    MachineId::new("this-mac")
}

impl App {
    /// The app with This Mac as its only machine, so Change… is one button.
    fn new(plan: RootPlan) -> Self {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a current-thread runtime starts");
        let mut cx = humata2_engine::headless(&Resources::new(humata2_ui::Assets))
            .expect("the headless context starts");
        cx.update(humata2_ui::init);
        let mut workspace = fixture::standard();
        workspace.machines.retain(|machine| machine.id == mac());
        let (clients, fakes) = humata2_clients::fake_clients(workspace, Arc::new(TestClock::new()));
        fakes.machines.set_root_plan(Ok(plan));
        let fake = fakes.machines.clone();
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
        let mut app = Self { runtime, cx, window, ui, fake };
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

    fn play(&mut self, action: &Action) {
        GpuiDriver::with_frames(&mut self.cx, self.window, FrameMode::Natural)
            .perform(action)
            .expect("the step can be played");
        self.settle();
    }

    fn click(&mut self, role: Role, label: &str) {
        self.play(&Action::Click(Target::new(role, label)));
    }

    fn send(&mut self, action: AppAction) {
        self.cx.update(|cx| self.ui.send(action, cx));
        self.settle();
    }

    fn shows(&mut self, role: Role, label: &str) -> bool {
        let view = GpuiDriver::with_frames(&mut self.cx, self.window, FrameMode::Natural)
            .view()
            .expect("the view can be read");
        view.contains(&format!("{role:?} {label:?}"))
    }

    fn root_on_the_row(&self) -> String {
        self.ui.read(|s| s.workspace.machine(&mac()).map(|m| m.root.clone()).unwrap_or_default())
    }

    fn sheet_text(&self) -> Option<String> {
        self.ui.read(|s| s.hosts.root_change.as_ref().map(|sheet| sheet.text.clone()))
    }

    fn changes(&self) -> Vec<(String, bool)> {
        self.fake
            .calls()
            .into_iter()
            .filter_map(|call| match call {
                Call::ChangeRoot(_, root, moved) => Some((root, moved)),
                _ => None,
            })
            .collect()
    }

    fn open_machines(&mut self) {
        self.send(AppAction::Switch(Route::Settings));
        self.click(Role::Tab, "Machines");
    }
}

fn plan(running: &[&str]) -> RootPlan {
    let workspace = fixture::standard();
    RootPlan {
        machine: mac(),
        token: "pl_1".to_owned(),
        old_root: "~/Humata".to_owned(),
        new_root: "~/Humata-work".to_owned(),
        channels: workspace.channels.iter().take(2).map(|channel| channel.id.clone()).collect(),
        running: running.iter().map(|id| humata2_clients::model::SessionId::new(*id)).collect(),
    }
}

#[test]
fn change_then_plan_then_new_channels_only_shows_the_new_root_on_the_row() {
    let mut app = App::new(plan(&[]));
    app.open_machines();
    assert_eq!(app.root_on_the_row(), "~/Humata");

    app.click(Role::Button, "Change\u{2026}");
    assert_eq!(app.sheet_text(), Some("~/Humata".to_owned()), "the sheet starts with the current root");
    assert!(app.changes().is_empty());

    app.play(&Action::Type("-work".to_owned()));
    assert_eq!(app.sheet_text(), Some("~/Humata-work".to_owned()), "the typed path reaches the sheet");

    app.click(Role::Button, "Plan");
    assert!(app.shows(Role::Button, "Use for new channels only"), "the plan offers its choices");
    assert!(app.changes().is_empty(), "nothing changes before a choice is made");
    assert_eq!(app.root_on_the_row(), "~/Humata");

    app.click(Role::Button, "Use for new channels only");
    assert_eq!(app.changes(), vec![("~/Humata-work".to_owned(), false)]);
    assert_eq!(app.sheet_text(), None, "the sheet closes when the change is done");
    assert_eq!(app.root_on_the_row(), "~/Humata-work");
}

#[test]
fn running_sessions_still_allow_new_channels_only() {
    let mut app = App::new(plan(&["ledger"]));
    app.open_machines();
    app.click(Role::Button, "Change\u{2026}");
    app.play(&Action::Type("-work".to_owned()));
    app.click(Role::Button, "Plan");

    assert!(app.shows(Role::Button, "Move them"), "the choices are drawn");
    assert!(app.shows(Role::Button, "Use for new channels only"));
    assert!(app.changes().is_empty(), "nothing moves before a choice");
    assert_eq!(app.root_on_the_row(), "~/Humata");

    app.click(Role::Button, "Use for new channels only");
    assert_eq!(app.changes(), vec![("~/Humata-work".to_owned(), false)]);
}
