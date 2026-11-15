//! D76: the real window shell compared with the approved session frames.
use gpui_kit::component::theme::{Theme, ThemeMode};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, Bounds, HeadlessAppContext, WindowBounds, WindowOptions, point, px, size,
};
use humata2_app::layout::PaneId;
use humata2_app::{AppAction, PaneAction, Route};
use humata2_clients::model::{SessionId, Status};
use humata2_clients::{Clients, Workspace, fixture};
use humata2_engine::Resources;
use humata2_session::{Panel, SessionAction};
use humata2_sidebar::{RowKey, SidebarAction};
use humata2_snapshot::{Tolerance, snapshots, view};
use humata2_ui::{bridge::Ui, record::Recorder};
use std::sync::Arc;
use tca::TestClock;
use tokio::runtime::Runtime;

struct App {
    runtime: Runtime,
    cx: HeadlessAppContext,
    window: AnyWindowHandle,
    ui: std::rc::Rc<Ui>,
}
impl App {
    fn new(workspace: Workspace) -> Self {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime starts");
        let mut cx = humata2_engine::headless(&Resources::new(humata2_ui::Assets))
            .expect("headless context starts");
        cx.update(humata2_ui::init);
        let (clients, _fakes) = Clients::fake(workspace, Arc::new(TestClock::new()));
        let ui = Ui::new(runtime.handle(), clients, Recorder::new());
        let options = WindowOptions {
            show: false,
            focus: false,
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                point(px(0.), px(0.)),
                size(px(1280.), px(760.)),
            ))),
            ..Default::default()
        };
        let (window, _) = cx
            .update(|cx| humata2_ui::open(cx, &ui, options))
            .expect("window opens headless");
        let mut app = Self {
            runtime,
            cx,
            window,
            ui,
        };
        app.settle();
        app.send(AppAction::Pane(PaneAction::Drag(PaneId::Panel, 340.)));
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
    }
    fn send(&mut self, action: AppAction) {
        self.cx.update(|cx| self.ui.send(action, cx));
        self.settle();
    }
    fn open(&mut self, session: &str, panel: Panel) {
        self.send(AppAction::Navigate(Route::Session(SessionId::new(session))));
        self.send(AppAction::Session(SessionAction::SetPanel(Some(panel))));
    }
    fn draw(&mut self) {
        self.cx
            .update_window(self.window, |_, window, cx| window.render_frame(cx))
            .expect("window stays open");
    }
    fn contains(&mut self, id: &str) -> bool {
        self.draw();
        self.cx
            .update_window(self.window, |_, window, _| {
                window
                    .try_find(gpui_kit::SharedString::from(id.to_owned()))
                    .is_some()
            })
            .expect("window stays open")
    }
    fn click(&mut self, id: &str) {
        self.cx
            .update_window(self.window, |_, window, cx| {
                window.click(gpui_kit::SharedString::from(id.to_owned()), cx)
            })
            .expect("control is drawn");
        self.settle();
    }
}

/// NV-2, CH-16: navigation and folding use the real sidebar and store.
#[test]
fn sidebar_navigation_and_folds_act_on_the_open_workspace() {
    let mut app = App::new(fixture::design());
    assert!(app.contains("sidebar-session-partial-brin"));
    app.click("sidebar-session-partial-brin");
    assert_eq!(
        app.ui.read(|s| s.session.open.clone()),
        Some(SessionId::new("partial-brin"))
    );
    app.click("rail-home");
    assert_eq!(app.ui.read(|s| s.route.clone()), Route::Home);
    assert!(app.ui.read(|s|s.trail.is_empty()));
    app.click("rail-inbox");
    assert_eq!(app.ui.read(|s| s.route.clone()), Route::Inbox);
    app.send(AppAction::Sidebar(SidebarAction::Toggle(RowKey::Channel(
        humata2_clients::model::ChannelId::new("payments-rewrite"),
    ))));
    assert!(!app.contains("sidebar-session-partial-brin"));
    app.click("sidebar-channel-payments-rewrite");
    assert_eq!(
        app.ui.read(|s| s.route.clone()),
        Route::Channel(humata2_clients::model::ChannelId::new("payments-rewrite"))
    );
    app.click("rail-settings");
    assert_eq!(app.ui.read(|s| s.route.clone()), Route::Settings);
}

/// N1: full-window reference images, never replacing the approved frames.
#[test]
fn the_real_shell_in_both_appearances_and_sidebar_states() {
    let snapshots = snapshots!().expect("snapshot folder");
    for (name, mode, pinned, folded, selected) in [
        (
            "01-session-working-light",
            ThemeMode::Light,
            false,
            false,
            "ledger",
        ),
        (
            "02-session-working-dark",
            ThemeMode::Dark,
            false,
            false,
            "ledger",
        ),
        ("15-sidebar-pinned", ThemeMode::Light, true, false, "ledger"),
        (
            "16-sidebar-fork-selected",
            ThemeMode::Light,
            false,
            false,
            "partial-brin",
        ),
        (
            "17-sidebar-channels-folded",
            ThemeMode::Dark,
            false,
            true,
            "ledger",
        ),
    ] {
        let mut workspace = fixture::design();
        let ledger = workspace
            .sessions
            .iter_mut()
            .find(|s| s.id.as_str() == "ledger")
            .expect("ledger fixture");
        ledger.status = if pinned {
            Status::NeedsYou
        } else {
            Status::Working
        };
        ledger.pinned = pinned;
        ledger.vip = pinned;
        let mut app = App::new(workspace);
        app.cx.update(|cx| Theme::change(mode, None, cx));
        app.open("ledger", Panel::Changes);
        app.send(AppAction::Review(
            humata2_review::ReviewAction::SelectFile {
                repo: "api".into(),
                path: "src/client.rs".into(),
            },
        ));
        app.send(AppAction::Review(
            humata2_review::ReviewAction::StartComment(humata2_review::Anchor {
                path: "src/client.rs".into(),
                line: 43,
            }),
        ));
        app.send(AppAction::Review(
            humata2_review::ReviewAction::CommentText(
                "Keep insert_legacy until the backfill ends.".into(),
            ),
        ));
        app.send(AppAction::Review(humata2_review::ReviewAction::SaveComment));
        app.send(AppAction::Sidebar(SidebarAction::SetCurrent(Some(
            RowKey::Session(SessionId::new(selected)),
        ))));
        for id in if folded {
            vec!["payments-rewrite", "infra", "pr-reviews", "web"]
        } else {
            vec!["pr-reviews", "web"]
        } {
            app.send(AppAction::Sidebar(SidebarAction::Toggle(RowKey::Channel(
                humata2_clients::model::ChannelId::new(id),
            ))));
        }
        app.draw();
        let tree = app
            .cx
            .update_window(app.window, |_, window, _| view::capture(window))
            .expect("window")
            .expect("view");
        snapshots.assert_text(&format!("{name}.view"), &tree);
        let image = app
            .cx
            .capture_screenshot(app.window)
            .expect("headless image");
        snapshots.assert_image(name, &image, Tolerance::default());
    }
}
