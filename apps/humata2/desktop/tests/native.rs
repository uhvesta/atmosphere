//! D76: the real window shell compared with the approved session frames.
use gpui_kit::component::theme::{Theme, ThemeMode};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, Bounds, HeadlessAppContext, WindowBounds, WindowOptions, point, px, size,
};
use humata2_app::layout::{FullScreen, PaneId};
use humata2_app::{AppAction, PaneAction, Route};
use humata2_clients::model::{SessionId, Status};
use humata2_clients::{Clients, Workspace, fixture};
use humata2_engine::Resources;
use humata2_session::{Panel, SessionAction};
use humata2_sidebar::{RowKey, SidebarAction};
use humata2_snapshot::{Mode, Snapshots, Tolerance, snapshots, view};
use humata2_ui::{bridge::Ui, record::Recorder};
use std::sync::Arc;
use tca::TestClock;
use tokio::runtime::Runtime;

// Recording must include even small controls that fit within the verify
// tolerance, so the image sent for visual review is the current render.
fn image_tolerance(snapshots: &Snapshots) -> Tolerance {
    if snapshots.mode() == Mode::Record {
        Tolerance::EXACT
    } else {
        Tolerance::default()
    }
}

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
    fn file_tab(&mut self) {
        self.send(AppAction::Session(SessionAction::SelectFile {
            repo: "api".into(),
            path: "src/client.rs".into(),
        }));
        self.send(AppAction::Session(SessionAction::OpenFile));
        self.send(AppAction::Session(SessionAction::SelectTab(0)));
    }
    fn draw(&mut self) {
        self.cx
            .update_window(self.window, |_, window, cx| window.render_frame(cx))
            .expect("window stays open");
    }
    fn snapshot(&mut self, name: &str) {
        self.draw();
        let snapshots = snapshots!().expect("snapshot folder");
        let tree = self
            .cx
            .update_window(self.window, |_, window, _| view::capture(window))
            .expect("window")
            .expect("view");
        snapshots.assert_text(&format!("{name}.view"), &tree);
        let image = self
            .cx
            .capture_screenshot(self.window)
            .expect("headless image");
        snapshots.assert_image(name, &image, image_tolerance(&snapshots));
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
    assert!(app.ui.read(|s| s.trail.is_empty()));
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
        let mut workspace = fixture::design_session(if pinned {
            Status::NeedsYou
        } else {
            Status::Working
        });
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
        if pinned {
            workspace
                .sessions
                .iter_mut()
                .find(|s| s.id.as_str() == "pr-482")
                .expect("PR fixture")
                .pinned = true;
        }
        let mut app = App::new(workspace);
        app.cx.update(|cx| Theme::change(mode, None, cx));
        app.open("ledger", Panel::Changes);
        app.file_tab();
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
        snapshots.assert_image(name, &image, image_tolerance(&snapshots));
    }
}

/// N1: expanding either area uses the same real header/panel controls.
#[test]
fn terminal_and_panel_full_screen_return_to_the_same_columns() {
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Board);
    let layout = app.ui.read(|s| s.layout);
    app.click("terminal-full-screen");
    assert_eq!(app.ui.read(|s| s.full_screen()), Some(FullScreen::Terminal));
    assert!(!app.contains("sidebar"));
    assert!(!app.contains("side-panel"));
    app.click("terminal-full-screen");
    assert!(app.contains("sidebar"));
    assert!(app.contains("side-panel"));
    app.click("expand-panel");
    assert_eq!(app.ui.read(|s| s.full_screen()), Some(FullScreen::Panel));
    assert!(app.contains("panel-full-screen"));
    assert!(!app.contains("terminal"));
    app.click("expand-panel");
    assert!(app.contains("terminal"));
    assert!(app.contains("side-panel"));
    assert_eq!(app.ui.read(|s| s.layout), layout);
}

/// The owner's half-screen request: resize the same live headless window,
/// navigate through its compact sidebar, then widen it again.
#[test]
fn a_half_screen_window_can_navigate_and_restore_its_columns() {
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Info);
    let saved = app.ui.read(|s| s.layout);
    app.cx
        .update_window(app.window, |_, window, cx| {
            window.resize(size(px(720.), px(760.)));
            window.bounds_changed(cx);
        })
        .expect("resize");
    app.settle();
    app.draw();
    let width = app
        .cx
        .update_window(app.window, |_, window, _| {
            f32::from(window.viewport_size().width)
        })
        .expect("viewport");
    assert_eq!(width, 720.);
    assert!(app.contains("sidebar-strip"));
    assert!(app.contains("side-panel"));
    app.click("sidebar-strip");
    assert!(app.contains("sidebar-session-brin"));
    app.click("sidebar-session-brin");
    assert_eq!(
        app.ui.read(|s| s.session.open.clone()),
        Some(SessionId::new("brin"))
    );
    assert!(!app.ui.read(|s| s.navigation_overlay));
    app.cx
        .update_window(app.window, |_, window, cx| {
            window.resize(size(px(1280.), px(760.)));
            window.bounds_changed(cx);
        })
        .expect("resize back");
    app.settle();
    app.draw();
    assert!(app.contains("sidebar"));
    assert!(!app.contains("sidebar-strip"));
    assert_eq!(app.ui.read(|s| s.layout), saved);
}

/// SE-14/16, N1: the real session layouts in their approved lifecycle states.
#[test]
fn session_lifecycle_and_full_screen_reference_images() {
    for (name, mode, status, panel, full, folded) in [
        (
            "03-session-needs-you-light",
            ThemeMode::Light,
            Status::NeedsYou,
            Some(Panel::Inbox),
            None,
            false,
        ),
        (
            "04-session-needs-you-dark",
            ThemeMode::Dark,
            Status::NeedsYou,
            Some(Panel::Inbox),
            None,
            false,
        ),
        (
            "05-session-exited",
            ThemeMode::Dark,
            Status::Exited { code: 0 },
            Some(Panel::Info),
            None,
            false,
        ),
        (
            "06-session-offline",
            ThemeMode::Light,
            Status::Offline,
            Some(Panel::Files),
            None,
            false,
        ),
        (
            "09-session-no-panel",
            ThemeMode::Dark,
            Status::Working,
            None,
            None,
            false,
        ),
        (
            "10-terminal-full-screen",
            ThemeMode::Dark,
            Status::Working,
            None,
            Some(FullScreen::Terminal),
            false,
        ),
        (
            "11-panel-full-screen-diff",
            ThemeMode::Light,
            Status::Working,
            Some(Panel::Changes),
            Some(FullScreen::Panel),
            false,
        ),
        (
            "12-panel-full-screen-board",
            ThemeMode::Dark,
            Status::Working,
            Some(Panel::Board),
            Some(FullScreen::Panel),
            false,
        ),
        (
            "13-columns-collapsed",
            ThemeMode::Dark,
            Status::Working,
            None,
            None,
            true,
        ),
    ] {
        let mut app = App::new(fixture::design_session(status));
        app.cx.update(|cx| Theme::change(mode, None, cx));
        app.send(AppAction::Navigate(Route::Session(SessionId::new(
            "ledger",
        ))));
        app.send(AppAction::Session(SessionAction::SetPanel(panel)));
        app.file_tab();
        if let Some(full) = full {
            app.send(AppAction::ToggleFullScreen(full));
        }
        if folded {
            app.send(AppAction::Pane(PaneAction::Toggle(PaneId::Sidebar)));
            app.send(AppAction::Pane(PaneAction::Drag(PaneId::Panel, 0.)));
        }
        if panel == Some(Panel::Changes) {
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
            app.send(AppAction::Review(
                humata2_review::ReviewAction::SelectFile {
                    repo: "api".into(),
                    path: "src/client.rs".into(),
                },
            ));
        }
        if panel == Some(Panel::Files) {
            app.send(AppAction::Session(SessionAction::SelectFile {
                repo: "api".into(),
                path: "src/client.rs".into(),
            }));
        }
        for id in ["pr-reviews", "web"] {
            app.send(AppAction::Sidebar(SidebarAction::Toggle(RowKey::Channel(
                humata2_clients::model::ChannelId::new(id),
            ))));
        }
        app.snapshot(name);
    }
}

/// SE-14: either Resume control starts the same exited agent and retains its view.
#[test]
fn resuming_an_exited_session_keeps_its_open_panel() {
    let mut app = App::new(fixture::design_session(Status::Exited { code: 0 }));
    app.open("ledger", Panel::Info);
    app.click("resume-agent");
    assert_eq!(
        app.ui.read(|s| s
            .workspace
            .session(&SessionId::new("ledger"))
            .expect("session")
            .status),
        Status::Idle
    );
    assert_eq!(app.ui.read(|s| s.panel()), Some(Panel::Info));
    assert!(!app.contains("resume-agent"));
}

/// SE-11: picking the same strip choice closes it; picking it again reopens it.
#[test]
fn clicking_the_open_panel_choice_closes_and_reopens_it() {
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Inbox);
    assert!(app.contains("panel-badge Inbox"));
    app.click("panel-tab Inbox");
    assert_eq!(app.ui.read(|s| s.panel()), None);
    assert!(!app.contains("side-panel"));
    app.click("panel-tab Inbox");
    assert_eq!(app.ui.read(|s| s.panel()), Some(Panel::Inbox));
    assert!(app.contains("side-panel"));
}

/// SE-8/10: shell tabs show their own output, and live output remains in its tab.
#[test]
fn terminal_tabs_show_only_the_selected_terminals_live_output() {
    use humata2_clients::model::{Line, Speaker};
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Info);
    app.send(AppAction::Session(SessionAction::Output {
        session: SessionId::new("ledger"),
        tab: 1,
        line: Line {
            from: Speaker::Tool,
            text: "shell-result: 42".into(),
        },
    }));
    let capture = |app: &mut App| {
        app.draw();
        app.cx
            .update_window(app.window, |_, window, _| view::capture(window))
            .expect("window")
            .expect("view")
    };
    assert!(!capture(&mut app).contains("shell-result: 42"));
    app.click("terminal-tab 1");
    assert_eq!(app.ui.read(|s| s.session.view().expect("view").tab), 1);
    assert!(capture(&mut app).contains("shell-result: 42"));
    app.click("terminal-tab 0");
    assert!(!capture(&mut app).contains("shell-result: 42"));
}

/// SE-8: Open big creates a file tab; selecting/closing it keeps the terminals.
#[test]
fn files_open_in_the_tab_row_and_close_back_to_the_terminal() {
    use humata2_session::Tab;
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Files);
    app.click("session-file api/src/client.rs");
    app.click("files-open-big");
    assert!(app.contains("session-file"));
    assert!(app.ui.read(|s|matches!(s.session.view().unwrap().selected(0),Some(Tab::File {path,..}) if path=="src/client.rs")));
    app.snapshot("file-tab-open");
    app.click("terminal-tab 0");
    assert!(app.contains("terminal"));
    app.click("file-tab-0-2");
    app.click("close-tab-0-2");
    assert!(!app.contains("session-file"));
    assert!(app.contains("terminal"));
}

/// D76: plus opens a program; splitting twice still leaves just two groups.
#[test]
fn the_plus_menu_splits_once_and_joining_keeps_the_program_running() {
    let mut workspace = fixture::design();
    workspace.views.clear();
    let mut app = App::new(workspace);
    app.open("ledger", Panel::Info);
    app.send(AppAction::Session(SessionAction::SetPanel(None)));
    app.cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
    for channel in ["pr-reviews", "web"] {
        app.send(AppAction::Sidebar(SidebarAction::Toggle(RowKey::Channel(
            humata2_clients::model::ChannelId::new(channel),
        ))));
    }
    app.click("add-tab-0");
    assert!(app.contains("new-terminal-menu"));
    app.click("split-terminal-0-codex");
    assert_eq!(app.ui.read(|s| s.session.view().unwrap().groups.len()), 2);
    assert!(app.contains("terminal-secondary"));
    app.snapshot("14-two-tab-groups");
    app.click("add-tab-1");
    app.click("split-terminal-1-codex");
    assert_eq!(app.ui.read(|s| s.session.view().unwrap().groups.len()), 2);
    let shells = app.ui.read(|s| {
        s.workspace
            .session(&SessionId::new("ledger"))
            .unwrap()
            .shells
            .len()
    });
    app.click("close-group-1");
    assert_eq!(app.ui.read(|s| s.session.view().unwrap().groups.len()), 1);
    assert_eq!(
        app.ui.read(|s| s
            .workspace
            .session(&SessionId::new("ledger"))
            .unwrap()
            .shells
            .len()),
        shells
    );
    assert!(app.contains("terminal-tab 2"));
}

/// VW-2/4/6: the offer waits for consent; Later returns, Open it creates a tab.
#[test]
fn prepared_views_wait_for_open_and_later_returns_on_the_next_visit() {
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Inbox);
    assert!(app.contains("offer-open"));
    assert!(!app.contains("prepared-view-content"));
    app.click("offer-later");
    assert!(!app.contains("offer-open"));
    app.click("rail-home");
    app.click("sidebar-session-ledger");
    assert!(app.contains("offer-open"));
    app.click("offer-open");
    assert!(app.contains("prepared-view-content"));
    assert!(!app.contains("offer-open"));
    app.click("terminal-tab 0");
    assert!(app.contains("terminal"));
}

/// N1: Home and wide Inbox are real pages over the clients, in both appearances.
#[test]
fn home_and_global_inbox_reference_images() {
    for (name, route, mode) in [
        ("25-home", Route::Home, ThemeMode::Light),
        ("26-inbox", Route::Inbox, ThemeMode::Light),
        ("27-inbox-dark", Route::Inbox, ThemeMode::Dark),
    ] {
        let mut workspace = fixture::design();
        for s in &mut workspace.sessions {
            s.vip = s.id.as_str() == "ledger";
            s.pinned = route == Route::Home && ["ledger", "pr-482"].contains(&s.id.as_str());
        }
        let mut app = App::new(workspace);
        app.cx.update(|cx| Theme::change(mode, None, cx));
        app.send(AppAction::Switch(route));
        for id in ["pr-reviews", "web"] {
            app.send(AppAction::Sidebar(SidebarAction::Toggle(RowKey::Channel(
                humata2_clients::model::ChannelId::new(id),
            ))));
        }
        app.snapshot(name);
    }
}

/// IN-3/7/15: selected details answer the chosen request, then history can reveal it.
#[test]
fn the_global_inbox_answers_only_the_selected_request() {
    use humata2_clients::model::{Answer, RequestId};
    let mut app = App::new(fixture::design());
    app.click("rail-inbox");
    assert_eq!(
        app.ui.read(|s| s.inbox.selected.clone()),
        Some(RequestId::new("move-pay-3"))
    );
    app.click("global-request push-brin");
    assert_eq!(
        app.ui.read(|s| s.inbox.selected.clone()),
        Some(RequestId::new("push-brin"))
    );
    app.click("deny push-brin");
    assert!(app.ui.read(|s| {
        s.workspace
            .requests
            .iter()
            .find(|r| r.id.as_str() == "push-brin")
            .unwrap()
            .answer
            == Some(Answer::Denied { reply: None })
    }));
    assert!(app.ui.read(|s| {
        s.workspace
            .requests
            .iter()
            .find(|r| r.id.as_str() == "move-pay-3")
            .unwrap()
            .answer
            .is_none()
    }));
    assert!(!app.contains("global-request push-brin"));
    app.click("inbox-history-Answered");
    assert!(app.contains("global-request push-brin"));
    assert!(!app.contains("global-request move-pay-3"));
    app.click("inbox-history-All");
    assert!(app.contains("global-request move-pay-3"));
    app.click("inbox-history-Waiting");
    assert!(!app.contains("global-request push-brin"));
}

/// Home actions open the right session and Resume the exited process.
#[test]
fn home_review_and_resume_act_on_their_own_rows() {
    let mut app = App::new(fixture::design());
    app.click("home-needs-action-0");
    assert_eq!(
        app.ui.read(|s| s.session.open.clone()),
        Some(SessionId::new("ledger"))
    );
    assert_eq!(app.ui.read(|s| s.panel()), Some(Panel::Inbox));
    app.click("rail-home");
    app.click("home-needs-action-1");
    assert_eq!(
        app.ui.read(|s| s.session.open.clone()),
        Some(SessionId::new("postgres"))
    );
    assert_eq!(
        app.ui.read(|s| s
            .workspace
            .session(&SessionId::new("postgres"))
            .unwrap()
            .status),
        Status::Idle
    );
}

/// N1: the real channel's three approved page bodies.
#[test]
fn channel_page_reference_images() {
    use humata2_app::channel::{Action, Tab};
    for (name, tab, mode) in [
        ("18-channel-sessions", Tab::Sessions, ThemeMode::Light),
        ("19-channel-activity", Tab::Activity, ThemeMode::Dark),
        ("20-channel-repos-machines", Tab::Repos, ThemeMode::Light),
    ] {
        let mut app = App::new(fixture::design());
        app.cx.update(|cx| Theme::change(mode, None, cx));
        app.send(AppAction::Switch(Route::Channel(
            humata2_clients::model::ChannelId::new("payments-rewrite"),
        )));
        app.send(AppAction::Channel(Action::Tab(tab)));
        for id in ["pr-reviews", "web"] {
            app.send(AppAction::Sidebar(SidebarAction::Toggle(RowKey::Channel(
                humata2_clients::model::ChannelId::new(id),
            ))));
        }
        app.snapshot(name);
    }
}

/// CH-12/13: filters/folds retain fork context; an Inbox count opens the right session.
#[test]
fn channel_sessions_filter_fold_and_open_their_own_inbox() {
    use humata2_app::channel::Action;
    let mut app = App::new(fixture::design());
    app.click("sidebar-channel-payments-rewrite");
    assert!(app.contains("channel-session-partial-brin"));
    app.click("channel-fold-ledger");
    assert!(!app.contains("channel-session-partial-brin"));
    app.click("channel-expand-all");
    assert!(app.contains("channel-session-partial-brin"));
    app.click("channel-session-filter");
    app.cx
        .update_window(app.window, |_, window, cx| window.input("PARTIAL", cx))
        .expect("filter input");
    app.settle();
    assert!(app.contains("channel-session-ledger"));
    assert!(app.contains("channel-session-partial-brin"));
    assert!(!app.contains("channel-session-refund"));
    app.send(AppAction::Channel(Action::Filter(String::new())));
    app.click("channel-inbox-ledger");
    assert_eq!(
        app.ui.read(|s| s.session.open.clone()),
        Some(SessionId::new("ledger"))
    );
    assert_eq!(app.ui.read(|s| s.panel()), Some(Panel::Inbox));
}

/// CH-9: clone cells filter the channel table and Activity links open their session.
#[test]
fn channel_clone_cells_and_activity_are_actionable() {
    use humata2_app::channel::Tab;
    let mut app = App::new(fixture::design());
    app.click("sidebar-channel-payments-rewrite");
    app.click("channel-tab-Repos and machines");
    app.click("channel-clones-acme/ledger-devbox-1");
    assert!(app.contains("channel-sessions"));
    assert!(app.ui.read(|s| {
        s.channel_pages
            .values()
            .any(|v| v.tab == Tab::Sessions && v.clones.is_some())
    }));
    assert!(!app.contains("channel-session-refund"));
    app.click("channel-clear-clones");
    assert!(app.contains("channel-session-refund"));
    app.click("channel-tab-Inbox");
    assert!(app.contains("global-request move-pay-3"));
    assert!(!app.contains("global-request benchmark-command"));
    app.click("rail-home");
    app.click("sidebar-channel-payments-rewrite");
    assert!(app.contains("global-request move-pay-3"));
    assert!(!app.contains("global-request benchmark-command"));
    app.click("channel-tab-Activity");
    app.click("channel-event-0");
    assert_eq!(
        app.ui.read(|s| s.session.open.clone()),
        Some(SessionId::new("ledger"))
    );
    assert_eq!(app.ui.read(|s| s.panel()), Some(Panel::Inbox));
}
