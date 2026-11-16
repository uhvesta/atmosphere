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
    fakes: humata2_clients::Fakes,
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
        let (clients, fakes) = Clients::fake(workspace, Arc::new(TestClock::new()));
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
            fakes,
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
        // D128.5: frame 16 shows the selected fork itself (SE-6), so the
        // header reads Partial BRIN on posted_at and the rail has no Inbox
        // badge.
        app.open(selected, Panel::Changes);
        app.file_tab();
        if pinned {
            // Golden 15: two messages wait while Ledger needs you.
            for message in [
                "Keep both indexes until the backfill ends.",
                "Then check the read path before dropping the old index.",
            ] {
                app.send(AppAction::Session(SessionAction::Draft(message.into())));
                app.send(AppAction::Session(SessionAction::Submit));
            }
        }
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
            "07-session-board-panel",
            ThemeMode::Dark,
            Status::Working,
            Some(Panel::Board),
            None,
            false,
        ),
        (
            "08-session-journal-panel",
            ThemeMode::Light,
            Status::Working,
            Some(Panel::Journal),
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
            // The golden's PAY-3 card asks to move to Done: Ledger needs you.
            Status::NeedsYou,
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
        // An offline session's program can't redraw when the layout
        // changes, so frame 06 is laid out while Ledger runs and the link
        // drops just before the capture, as it would for the person.
        let mut start = fixture::design_session(status);
        if status == Status::Offline {
            for machine in &mut start.machines {
                if machine.lost_at.is_some() {
                    machine.online = true;
                }
            }
            for session in &mut start.sessions {
                if session.status == Status::Offline {
                    session.status = Status::Working;
                }
            }
        }
        let mut app = App::new(start);
        app.cx.update(|cx| Theme::change(mode, None, cx));
        app.send(AppAction::Navigate(Route::Session(SessionId::new(
            "ledger",
        ))));
        app.send(AppAction::Session(SessionAction::SetPanel(panel)));
        app.file_tab();
        if status == Status::NeedsYou {
            for message in [
                "Keep both indexes until the backfill ends.",
                "Then check the read path before dropping the old index.",
            ] {
                app.send(AppAction::Session(SessionAction::Draft(message.into())));
                app.send(AppAction::Session(SessionAction::Submit));
            }
        }
        if let Some(full) = full {
            app.send(AppAction::ToggleFullScreen(full));
        }
        if folded {
            app.send(AppAction::Pane(PaneAction::Toggle(PaneId::Sidebar)));
            app.send(AppAction::Pane(PaneAction::Drag(PaneId::Panel, 0.)));
        }
        // Every frame's Ledger has one sent comment and this saved draft, so
        // the Diff rail reads 2 (frames 01–13).
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
        if panel == Some(Panel::Changes) {
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
        if status == Status::Offline {
            let offline = fixture::design_session(Status::Offline);
            let machine = offline
                .machines
                .iter()
                .find(|machine| machine.lost_at.is_some())
                .expect("the offline fixture says when devbox-1 was lost")
                .clone();
            let ledger = offline
                .sessions
                .iter()
                .find(|session| session.id == SessionId::new("ledger"))
                .expect("the fixture has Ledger")
                .clone();
            app.fakes.daemon.emit(humata2_clients::Event::Machine(machine));
            app.fakes.daemon.emit(humata2_clients::Event::Session(ledger));
            app.settle();
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
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Info);
    let offset = app.ui.read(|s| {
        s.session
            .terminal_bytes
            .get(&(SessionId::new("ledger"), 1))
            .map_or(0, |b| b.len())
    });
    app.send(AppAction::Session(SessionAction::TerminalData {
        session: SessionId::new("ledger"),
        tab: 1,
        offset,
        bytes: b"\r\nshell-result: 42\r\n".to_vec(),
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
    // Frame 14 shows the agent's plan only, before the question.
    if let Some(ledger) = workspace
        .sessions
        .iter_mut()
        .find(|x| x.id == SessionId::new("ledger"))
        && let Some(end) = ledger
            .transcript
            .iter()
            .position(|line| line.text.contains("Switch reads"))
    {
        ledger.transcript.truncate(end + 1);
    }
    let mut app = App::new(workspace);
    app.open("ledger", Panel::Info);
    app.send(AppAction::Session(SessionAction::SetPanel(None)));
    app.cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
    // Golden 14's Diff rail reads 2: one sent comment and this saved draft.
    app.send(AppAction::Review(
        humata2_review::ReviewAction::StartComment(humata2_review::Anchor {
            path: "src/client.rs".into(),
            line: 43,
        }),
    ));
    app.send(AppAction::Review(humata2_review::ReviewAction::CommentText(
        "Keep insert_legacy until the backfill ends.".into(),
    )));
    app.send(AppAction::Review(humata2_review::ReviewAction::SaveComment));
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
    // Home's Needs you group lists sessions in workspace order (HM-1).
    let postgres = app.ui.read(|s| {
        s.workspace
            .sessions
            .iter()
            .filter(|x| !x.archived && matches!(x.status, Status::NeedsYou | Status::Exited { .. }))
            .position(|x| x.id.as_str() == "postgres")
    });
    let Some(postgres) = postgres else {
        panic!("the design workspace has an exited postgres session");
    };
    app.click(&format!("home-needs-action-{postgres}"));
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

#[test]
fn board_reference_images() {
    use humata2_board::{BoardAction as B, Mode};
    use humata2_clients::model::{ItemId, Scope};
    let mut app = App::new(fixture::design());
    app.send(AppAction::Switch(Route::Board(Scope::Global)));
    app.cx
        .update(|cx| Theme::change(ThemeMode::Light, None, cx));
    app.snapshot("21-board-kanban");
    app.cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
    app.click("board-card PAY-3");
    app.snapshot("22-board-item-open");
    app.click("board-close");
    app.cx
        .update(|cx| Theme::change(ThemeMode::Light, None, cx));
    app.send(AppAction::Board(B::Mode(Mode::Table)));
    app.snapshot("23-board-table");
    app.cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
    app.send(AppAction::Board(B::Mode(Mode::Dashboard)));
    app.snapshot("24-board-dashboard");
    app.click("board-tile-progress");
    assert!(app.contains("board-table INF-2"));
    assert!(!app.contains("board-table PAY-12"));
    app.click("board-clear-filter");
    app.click("board-table PAY-3");
    app.click("board-status");
    app.click("board-move-In review");
    assert_eq!(
        app.ui.read(|s| s
            .workspace
            .items
            .iter()
            .find(|i| i.id == ItemId::new("PAY-3"))
            .unwrap()
            .status),
        humata2_clients::model::ItemStatus::InReview
    );
}

#[test]
fn board_creates_in_selected_column_and_answers_the_item_request() {
    use humata2_board::BoardAction as B;
    use humata2_clients::model::{ItemId, ItemStatus as S, Scope};
    let mut app = App::new(fixture::design());
    app.send(AppAction::Switch(Route::Board(Scope::Global)));
    app.click("board-add-Backlog");
    app.send(AppAction::Board(B::NewTitle("A new milestone".into())));
    app.click("board-create-item");
    assert!(app.ui.read(|s| {
        s.workspace.items.iter().any(|i| {
            i.title == "A new milestone" && i.status == S::Backlog && i.scope == Scope::Global
        })
    }));
    app.click("board-card PAY-3");
    app.click("board-approve");
    assert_eq!(
        app.ui.read(|s| s
            .workspace
            .items
            .iter()
            .find(|i| i.id == ItemId::new("PAY-3"))
            .unwrap()
            .status),
        S::Done
    );
    assert!(!app.contains("board-approve"));
    app.send(AppAction::Switch(Route::Channel(
        humata2_clients::model::ChannelId::new("infra"),
    )));
    app.click("channel-tab-Board");
    assert!(app.contains("board-card INF-2"));
    assert!(!app.contains("board-card PAY-3"));
}

#[test]
fn hosts_and_settings_reference_images() {
    use humata2_clients::model::MachineId;
    use humata2_hosts::HostsAction as H;
    let mut app = App::new(fixture::design());
    app.cx
        .update(|cx| Theme::change(ThemeMode::Light, None, cx));
    app.send(AppAction::Hosts(H::Select(MachineId::new("devbox-1"))));
    assert!(app.contains("host-row devbox-1"));
    app.snapshot("28-hosts");
    app.send(AppAction::Switch(Route::Settings));
    app.cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
    app.snapshot("29-settings");
}

#[test]
fn host_reconnect_and_shell_use_the_selected_machine() {
    use humata2_clients::model::{Agent, MachineId};
    use humata2_hosts::HostsAction as H;
    let mut app = App::new(fixture::design());
    app.send(AppAction::Hosts(H::Select(MachineId::new("gpu-box"))));
    app.click("host-connect gpu-box");
    assert!(app.ui.read(|s| {
        s.workspace
            .machine(&MachineId::new("gpu-box"))
            .unwrap()
            .online
    }));
    app.cx
        .update_window(app.window, |_, window, cx| {
            window.resize(size(px(720.), px(760.)));
            window.bounds_changed(cx);
        })
        .expect("resize Hosts");
    app.settle();
    app.click("host-open-shell");
    assert!(app.ui.read(|s|matches!(s.overlay(),Some(humata2_app::Overlay::NewSession(sheet)) if sheet.machine==Some(MachineId::new("gpu-box"))&&sheet.agent==Agent::Shell)));
}

#[test]
fn layout_settings_keep_the_hidden_group_and_toggle_pinned_rows() {
    use humata2_settings::{Section, SettingsAction as T};
    let mut workspace = fixture::design();
    workspace
        .sessions
        .iter_mut()
        .find(|s| s.id.as_str() == "ledger")
        .unwrap()
        .pinned = true;
    let mut app = App::new(workspace);
    app.open("ledger", Panel::Changes);
    app.click("add-tab-0");
    app.click("split-terminal-0-zsh");
    assert!(app.contains("add-tab-1"));
    app.send(AppAction::Switch(Route::Settings));
    app.click("settings-panes-Fixed");
    app.click("settings-pinned");
    assert!(!app.contains("sidebar-pinned-ledger"));
    app.send(AppAction::Navigate(Route::Session(SessionId::new(
        "ledger",
    ))));
    assert_eq!(app.ui.read(|s| s.session.view().unwrap().groups.len()), 2);
    app.send(AppAction::Settings(T::SplitGroups(true)));
    assert!(app.contains("add-tab-0") && app.contains("add-tab-1"));
    app.send(AppAction::Switch(Route::Settings));
    app.click("settings-theme-Light");
    app.cx
        .update(|cx| assert!(!gpui_kit::component::ActiveTheme::theme(cx).is_dark()));
    app.click("settings-font-plus");
    assert_eq!(app.ui.read(|s| s.settings.preferences.font_size), 13);
    app.send(AppAction::Settings(T::Section(Section::General)));
    app.click("settings-wrap");
    assert!(app.ui.read(|s| s.review.wrap));
}

#[test]
fn palette_new_session_and_toast_reference_images() {
    use humata2_app::NewSessionAction as N;
    use humata2_clients::model::{ItemId, Scope};
    use humata2_palette::PaletteAction as P;
    let mut workspace = fixture::design_session(Status::Working);
    workspace.views.clear();
    let mut app = App::new(workspace);
    app.open("ledger", Panel::Changes);
    app.file_tab();
    app.send(AppAction::Review(
        humata2_review::ReviewAction::SelectFile {
            repo: "api".into(),
            path: "src/client.rs".into(),
        },
    ));
    app.cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
    app.send(AppAction::OpenPalette);
    app.send(AppAction::Palette(P::Query("ledg".into())));
    app.snapshot("30-palette");
    app.send(AppAction::Escape);
    app.send(AppAction::Switch(Route::Board(Scope::Global)));
    app.cx
        .update(|cx| Theme::change(ThemeMode::Light, None, cx));
    app.send(AppAction::StartNewForItem(ItemId::new("PAY-9")));
    app.send(AppAction::NewSession(N::ToggleRepo("acme/api".into())));
    app.snapshot("31-new-session");
    app.send(AppAction::NewSession(N::FirstMessage(
        "Work on PAY-9\nKeep tests passing".into(),
    )));
    app.click("new-session-create");
    let id = app
        .ui
        .read(|s| s.session.open.clone())
        .expect("new session opens");
    assert!(
        app.ui
            .read(|s| s.workspace.session_options[&id].repos == vec!["acme/ledger".to_owned()])
    );
    assert!(
        app.ui
            .read(|s| s.workspace.session_options[&id].model.is_none())
    );
    assert!(app.ui.read(|s| {
        s.workspace
            .items
            .iter()
            .find(|i| i.id.as_str() == "PAY-9")
            .unwrap()
            .session
            .as_ref()
            == Some(&id)
    }));
}

#[test]
fn archiving_offers_a_working_undo() {
    use humata2_clients::model::SessionId;
    let mut workspace = fixture::design();
    let mut currency = workspace
        .session(&SessionId::new("pr-482"))
        .unwrap()
        .clone();
    currency.id = SessionId::new("currency");
    currency.title = "Currency codes".into();
    currency.channel = humata2_clients::model::ChannelId::new("payments-rewrite");
    workspace.sessions.push(currency);
    let mut app = App::new(workspace);
    app.cx
        .update(|cx| Theme::change(ThemeMode::Light, None, cx));
    app.open("currency", Panel::Info);
    app.send(AppAction::Session(SessionAction::Archive));
    assert!(app.ui.read(|s| {
        s.workspace
            .session(&SessionId::new("currency"))
            .unwrap()
            .archived
    }));
    app.snapshot("32-toast");
    let toast = app
        .ui
        .read(|s| s.toasts.iter().find(|t| t.action.is_some()).unwrap().id);
    app.click(&format!("toast-undo-{toast}"));
    assert!(!app.ui.read(|s| {
        s.workspace
            .session(&SessionId::new("currency"))
            .unwrap()
            .archived
    }));
}

#[test]
fn first_run_actions_open_their_forms() {
    use humata2_hosts::HostsAction as H;
    let mut app = App::new(Workspace::default());
    app.cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
    app.snapshot("33-first-run");
    app.click("first-channel-create");
    assert!(
        app.ui
            .read(|s| matches!(s.overlay(), Some(humata2_app::Overlay::NewChannel(_)))),
        "overlay after Create: {:?}",
        app.ui.read(|s| s.overlays.clone())
    );
    assert!(app.contains("new-channel-create"));
    app.send(AppAction::Escape);
    app.click("first-machine-add");
    assert!(app.contains("host-add-connect"));
    app.send(AppAction::Hosts(H::Address("dev@newbox".into())));
    app.click("host-add-connect");
    assert!(app.ui.read(|s| {
        s.workspace
            .machines
            .iter()
            .any(|m| m.name == "newbox" && m.online)
    }));
}

#[test]
fn palette_file_results_open_the_current_sessions_clone() {
    use humata2_palette::{Command, PaletteAction as P};
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Info);
    app.send(AppAction::OpenPalette);
    app.send(AppAction::Palette(P::Query("client.rs".into())));
    let index=app.ui.read(|s|s.palette.entries.iter().position(|e|matches!(&e.command,Command::File{session,repo,path} if session.as_str()=="ledger"&&repo=="api"&&path=="src/client.rs")).unwrap());
    app.send(AppAction::Palette(P::Pick(index)));
    assert!(app.contains("session-file"));
    assert!(app.ui.read(|s| {
        s.session
            .open
            .as_ref()
            .is_some_and(|id| id.as_str() == "ledger")
    }));
}

/// SE-10/13/16: actual terminal keystrokes, find and resize survive tab switches.
#[test]
fn the_embedded_terminal_accepts_keys_and_keeps_its_scrollback() {
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Info);
    app.click("terminal-tab 1");
    app.click("terminal");
    app.cx
        .update_window(app.window, |_, window, cx| {
            for key in ["p", "w", "d", "enter"] {
                window.press(key, cx);
            }
        })
        .unwrap();
    app.settle();
    let key = (SessionId::new("ledger"), 1);
    assert!(
        app.ui
            .read(|s| s.session.terminal_bytes[&key].ends_with(b"~/humata\r\n$ "))
    );
    let wide = app
        .runtime
        .block_on(app.ui.clients().daemon.load())
        .unwrap()
        .terminal_sizes[&key];
    app.click("terminal-tab 0");
    app.click("terminal-tab 1");
    app.click("terminal");
    app.cx
        .update_window(app.window, |_, window, cx| window.press("cmd-f", cx))
        .unwrap();
    app.settle();
    assert!(app.contains("find-bar"));
    app.cx
        .update_window(app.window, |_, window, cx| window.press("escape", cx))
        .unwrap();
    app.settle();
    app.cx
        .update_window(app.window, |_, window, cx| {
            window.resize(size(px(720.), px(760.)));
            window.bounds_changed(cx);
        })
        .unwrap();
    app.settle();
    app.draw();
    app.settle();
    let narrow = app
        .runtime
        .block_on(app.ui.clients().daemon.load())
        .unwrap()
        .terminal_sizes[&key];
    assert!(narrow.0 < wide.0);
    app.cx
        .update_window(app.window, |_, window, cx| {
            window.resize(size(px(1280.), px(760.)));
            window.bounds_changed(cx);
        })
        .unwrap();
    app.settle();
    app.draw();
    app.settle();
    let restored = app
        .runtime
        .block_on(app.ui.clients().daemon.load())
        .unwrap()
        .terminal_sizes[&key];
    assert_eq!(restored, wide);
    assert!(app.contains("terminal"));
    assert!(
        app.ui
            .read(|s| s.session.terminal_bytes[&key].ends_with(b"~/humata\r\n$ "))
    );
}

/// FK-1/2: the header opens a real dialog and the selected kind creates a child.
#[test]
fn the_fork_dialog_creates_the_selected_kind_and_can_cancel() {
    use humata2_clients::model::ForkKind;
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Info);
    app.click("session-fork");
    assert!(app.contains("fork-create"));
    app.click("fork-kind-1");
    app.send(AppAction::Session(SessionAction::ForkTitle(
        "Review the migration".into(),
    )));
    app.snapshot("fork-dialog");
    app.click("fork-create");
    let id = app.ui.read(|s| s.session.open.clone()).unwrap();
    assert_ne!(id, SessionId::new("ledger"));
    assert!(app.ui.read(|s| {
        s.workspace.session(&id).is_some_and(|v| {
            v.parent.as_ref() == Some(&SessionId::new("ledger"))
                && v.fork == Some(ForkKind::Conversation)
        })
    }));
    app.click("session-fork");
    app.cx
        .update_window(app.window, |_, window, cx| window.press("escape", cx))
        .unwrap();
    app.settle();
    assert!(!app.contains("fork-create"));
    assert!(app.ui.read(|s| s.session.fork.is_none()));
    app.cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
    app.click("session-fork");
    app.snapshot("fork-dialog-dark");
    app.click("fork-cancel");
    assert!(app.ui.read(|s| s.session.fork.is_none()));
}

/// SE-1/8/10: starting a session or selecting a tab immediately directs keyboard input there.
#[test]
fn new_sessions_and_selected_tabs_receive_input_without_an_extra_click() {
    use humata2_app::NewSessionAction as N;
    use humata2_clients::model::Agent;
    let mut app = App::new(fixture::design());
    // Headless windows start inactive; focus callbacks require activation.
    app.cx
        .update_window(app.window, |_, window, _| window.activate_window())
        .unwrap();
    app.settle();
    app.open("ledger", Panel::Info);
    app.click("terminal");
    app.click("terminal-tab 1");
    app.cx
        .update_window(app.window, |_, window, cx| {
            for key in ["p", "w", "d", "enter"] {
                window.press(key, cx);
            }
        })
        .unwrap();
    app.settle();
    assert!(app.ui.read(|s| {
        s.session.terminal_bytes[&(SessionId::new("ledger"), 1)].ends_with(b"~/humata\r\n$ ")
    }));
    app.send(AppAction::Session(SessionAction::OpenTerminal {
        agent: Agent::Shell,
        split: false,
    }));
    let tab = app.ui.read(|s| s.session.view().unwrap().tab);
    app.cx
        .update_window(app.window, |_, window, cx| {
            for key in ["p", "w", "d", "enter"] {
                window.press(key, cx);
            }
        })
        .unwrap();
    app.settle();
    assert!(app.ui.read(|s| {
        s.session.terminal_bytes[&(SessionId::new("ledger"), tab)].ends_with(b"~/humata\r\n$ ")
    }));
    app.send(AppAction::StartNewSession);
    app.send(AppAction::NewSession(N::Agent(Agent::Shell)));
    app.send(AppAction::NewSession(N::Title("Fresh terminal".into())));
    app.click("new-session-create");
    let id = app.ui.read(|s| s.session.open.clone()).unwrap();
    app.cx
        .update_window(app.window, |_, window, cx| {
            for key in ["p", "w", "d", "enter"] {
                window.press(key, cx);
            }
        })
        .unwrap();
    app.settle();
    assert!(
        app.ui
            .read(|s| s.session.terminal_bytes[&(id, 0)].ends_with(b"~/humata\r\n$ "))
    );
    app.click("session-fork");
    app.click("fork-kind-2");
    app.click("fork-create");
    let fork = app.ui.read(|s| s.session.open.clone()).unwrap();
    app.cx
        .update_window(app.window, |_, window, cx| {
            for key in ["p", "w", "d", "enter"] {
                window.press(key, cx);
            }
        })
        .unwrap();
    app.settle();
    assert!(
        app.ui
            .read(|s| s.session.terminal_bytes[&(fork.clone(), 0)].ends_with(b"~/humata\r\n$ "))
    );
    app.send(AppAction::OpenPalette);
    app.cx
        .update_window(app.window, |_, window, cx| window.press("escape", cx))
        .unwrap();
    app.settle();
    app.cx
        .update_window(app.window, |_, window, cx| {
            for key in ["l", "s", "enter"] {
                window.press(key, cx);
            }
        })
        .unwrap();
    app.settle();
    assert!(
        app.ui
            .read(|s| s.session.terminal_bytes[&(fork, 0)].ends_with(b"notes.md  todo.txt\r\n$ "))
    );
}

/// SE-13: focus inside either grid determines the group that receives new tabs.
#[test]
fn clicking_a_split_terminal_focuses_that_group() {
    use humata2_clients::model::Agent;
    let mut app = App::new(fixture::design());
    // Headless windows start inactive; focus callbacks require activation.
    app.cx
        .update_window(app.window, |_, window, _| window.activate_window())
        .unwrap();
    app.settle();
    app.open("ledger", Panel::Info);
    app.send(AppAction::Session(SessionAction::OpenTerminal {
        agent: Agent::Codex,
        split: true,
    }));
    assert_eq!(app.ui.read(|s| s.session.view().unwrap().focused_group), 1);
    app.cx
        .update_window(app.window, |_, window, cx| {
            window.within("terminal-area").click("terminal", cx)
        })
        .unwrap();
    app.settle();
    assert_eq!(app.ui.read(|s| s.session.view().unwrap().focused_group), 0);
    app.click("terminal-secondary");
    assert_eq!(app.ui.read(|s| s.session.view().unwrap().focused_group), 1);
    app.send(AppAction::Session(SessionAction::OpenTerminal {
        agent: Agent::Shell,
        split: false,
    }));
    assert!(app.ui.read(|s| {
        s.session.view().unwrap().groups[1]
            .tabs
            .contains(&humata2_session::Tab::Terminal(3))
    }));
    assert_eq!(app.ui.read(|s| s.session.view().unwrap().groups.len()), 2);
}

/// ST-8: a wrapped line keeps its number aligned to its first visual row.
#[test]
fn wrapped_changes_align_the_gutter_and_switch_back_to_one_scrolling_row() {
    use humata2_review::ReviewAction as R;
    let mut workspace = fixture::design();
    let file = workspace
        .diffs
        .iter_mut()
        .find(|d| d.repo == "api")
        .unwrap()
        .files
        .iter_mut()
        .find(|f| f.path == "src/client.rs")
        .unwrap();
    file.lines
        .iter_mut()
        .find(|l| l.kind == humata2_clients::model::LineKind::Removed)
        .unwrap()
        .text = format!("        {}.await?;", "long_identifier_".repeat(8));
    let mut app = App::new(workspace);
    app.open("ledger", Panel::Changes);
    app.send(AppAction::Review(R::SelectFile {
        repo: "api".into(),
        path: "src/client.rs".into(),
    }));
    // Diffs clip by default; this flow is about wrapped lines.
    app.send(AppAction::Review(R::ToggleWrap));
    app.draw();
    app.cx
        .update_window(app.window, |_, window, _| {
            let gutter = window.find("change-gutter-2").bounds();
            let code = window.find("change-code-2-0").bounds();
            assert!((f32::from(gutter.origin.y - code.origin.y)).abs() < 0.5);
            assert!(window.find("change-line-2").bounds().size.height > gutter.size.height);
        })
        .unwrap();
    app.snapshot("changes-wrapped-long-line");
    app.send(AppAction::Review(R::ToggleWrap));
    app.draw();
    app.cx
        .update_window(app.window, |_, window, _| {
            let line = window.find("change-line-2").bounds();
            let gutter = window.find("change-gutter-2").bounds();
            assert!((f32::from(line.size.height - gutter.size.height)).abs() < 0.5);
        })
        .unwrap();
    app.send(AppAction::Review(R::ToggleWrap));
    app.cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
    app.snapshot("changes-wrapped-long-line-dark");
    app.send(AppAction::Settings(
        humata2_settings::SettingsAction::FontSize(8),
    ));
    app.send(AppAction::Pane(PaneAction::Drag(PaneId::Panel, 220.)));
    app.draw();
    app.cx
        .update_window(app.window, |_, window, _| {
            let gutter = window.find("change-gutter-2").bounds();
            let code = window.find("change-code-2-0").bounds();
            assert!((f32::from(gutter.origin.y - code.origin.y)).abs() < 0.5);
            assert!(f32::from(gutter.size.height) >= 24.5);
        })
        .unwrap();
}

/// SE-23/24: pending messages are inspectable; writers belong to one terminal.
#[test]
fn queued_messages_can_be_shown_removed_and_sent_and_presence_follows_the_tab() {
    // Other writers are named while the agent works (frame 01).
    let mut app = App::new(fixture::design_session(Status::Working));
    app.open("ledger", Panel::Inbox);
    assert!(app.contains("terminal-peers-0"));
    app.send(AppAction::Session(SessionAction::SelectTab(1)));
    assert!(!app.contains("terminal-peers-0"));
    app.send(AppAction::Session(SessionAction::SelectTab(0)));
    assert!(app.contains("terminal-peers-0"));
    for message in [
        "Keep both indexes until the backfill ends.",
        "Then check the read path before dropping the old index.",
    ] {
        app.send(AppAction::Session(SessionAction::Draft(message.into())));
        app.send(AppAction::Session(SessionAction::Submit));
    }
    assert!(app.contains("session-queue"));
    assert!(!app.contains("queued-message-0"));
    app.click("show-queued");
    assert!(app.contains("queued-message-1"));
    app.snapshot("queued-messages-expanded");
    app.cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
    app.snapshot("queued-messages-expanded-dark");
    app.click("remove-queued-0");
    assert_eq!(
        app.ui.read(|s| s
            .session
            .view()
            .unwrap()
            .queue
            .iter()
            .map(|message| message.text())
            .collect::<Vec<_>>()),
        ["Then check the read path before dropping the old index."]
    );
    app.click("send-queued-0");
    assert!(!app.contains("session-queue"));
    assert!(app.ui.read(|s| {
        s.workspace
            .session(&SessionId::new("ledger"))
            .unwrap()
            .transcript
            .iter()
            .any(|l| l.text == "Then check the read path before dropping the old index.")
    }));
    assert!(!app.ui.read(|s| {
        s.workspace
            .session(&SessionId::new("ledger"))
            .unwrap()
            .transcript
            .iter()
            .any(|l| l.text == "Keep both indexes until the backfill ends.")
    }));
}

/// SE-23: output-fallback agents keep waiting until the person presses Send now.
#[test]
fn uncertain_turn_end_keeps_the_queue_visible_until_explicit_send() {
    let mut workspace = fixture::design();
    let session = workspace
        .sessions
        .iter_mut()
        .find(|s| s.id == SessionId::new("ledger"))
        .unwrap();
    session.agent = humata2_clients::model::Agent::Codex;
    session.turn = humata2_clients::model::Turn::Uncertain;
    session.status = Status::ProbablyIdle;
    let mut app = App::new(workspace);
    app.open("ledger", Panel::Inbox);
    app.send(AppAction::Session(SessionAction::Draft(
        "Wait for me to send this.".into(),
    )));
    app.send(AppAction::Session(SessionAction::Submit));
    assert_eq!(app.ui.read(|s| s.session.view().unwrap().queue.len()), 1);
    assert!(app.contains("send-next-queued"));
    app.snapshot("queued-message-uncertain");
    app.click("send-next-queued");
    assert!(!app.contains("session-queue"));
}

/// SE-23: a transport failure leaves a visible retry and preserves the message.
#[test]
fn a_failed_queued_message_stays_visible_for_retry() {
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Inbox);
    app.send(AppAction::Session(SessionAction::Draft(
        "Retry this message intact.".into(),
    )));
    app.send(AppAction::Session(SessionAction::Submit));
    app.click("show-queued");
    app.fakes.daemon.fail_next(
        humata2_clients::Operation::Send,
        "The connection dropped. Reconnect the machine, then send again.",
    );
    app.click("send-queued-0");
    assert!(app.contains("session-error"));
    assert_eq!(
        app.ui.read(|s| s
            .session
            .view()
            .unwrap()
            .queue
            .iter()
            .map(|message| message.text())
            .collect::<Vec<_>>()),
        ["Retry this message intact."]
    );
    assert!(app.contains("send-next-queued"));
    app.snapshot("queued-message-failed");
    app.cx
        .update_window(app.window, |_, window, cx| {
            let dismiss = window.find(("session-error", 1usize)).bounds();
            let banner = window.find("session-error").bounds();
            assert!(dismiss.origin.x + dismiss.size.width <= banner.origin.x + banner.size.width);
            window.click(("session-error", 1usize), cx);
        })
        .unwrap();
    app.settle();
    assert!(!app.contains("session-error"));
    assert!(app.contains("session-queue"));
    app.click("send-next-queued");
    assert!(!app.contains("session-queue"));
    assert!(!app.contains("session-error"));
}

/// SM-9/10: a failed comment hand-over waits visibly, Remove returns the comments to drafts.
#[test]
fn review_comments_wait_in_the_session_queue_and_retry_without_losing_their_anchor() {
    use humata2_review::{Anchor, ReviewAction as R};
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Changes);
    app.send(AppAction::Review(R::SelectFile {
        repo: "api".into(),
        path: "src/client.rs".into(),
    }));
    app.send(AppAction::Review(R::StartComment(Anchor {
        path: "src/client.rs".into(),
        line: 43,
    })));
    app.send(AppAction::Review(R::CommentText(
        "Keep these writes in one transaction.".into(),
    )));
    app.send(AppAction::Review(R::SaveComment));
    // Comments deliver at once when the session takes input, so the capture
    // of comments visibly waiting comes from a failed delivery.
    app.fakes.daemon.fail_next(
        humata2_clients::Operation::SendComments,
        "The connection dropped. Reconnect and try again.",
    );
    app.click("send-comments");
    assert!(app.ui.read(|s| s.review.queued().is_empty()));
    assert_eq!(app.ui.read(|s| s.session.view().unwrap().queue.len()), 1);
    assert!(
        !app.fakes
            .daemon
            .workspace()
            .diffs
            .iter()
            .flat_map(|d| &d.comments)
            .any(|c| c.body == "Keep these writes in one transaction.")
    );
    app.snapshot("review-comments-queued");
    app.cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
    app.snapshot("review-comments-queued-dark");
    app.cx
        .update(|cx| Theme::change(ThemeMode::Light, None, cx));
    app.click("show-queued");
    app.click("remove-queued-0");
    assert!(app.ui.read(|s| s.session.view().unwrap().queue.is_empty()));
    assert_eq!(app.ui.read(|s| s.review.queued().len()), 1);
    // Removing the only failed message lifts the pause, so this send goes
    // at once; make it fail too, to show the retry state.
    app.fakes.daemon.fail_next(
        humata2_clients::Operation::SendComments,
        "The connection dropped. Reconnect and try again.",
    );
    app.click("send-comments");
    assert!(app.contains("session-error"));
    assert_eq!(app.ui.read(|s| s.session.view().unwrap().queue.len()), 1);
    app.snapshot("review-comments-retry");
    app.click("send-next-queued");
    assert!(!app.contains("session-queue"));
    let workspace = app.fakes.daemon.workspace();
    let sent: Vec<_> = workspace
        .diffs
        .iter()
        .filter(|d| d.session == SessionId::new("ledger"))
        .flat_map(|d| &d.comments)
        .filter(|c| c.body == "Keep these writes in one transaction.")
        .collect();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].sent);
    app.snapshot("review-comments-sent");
}

/// SM-9/SE-23: a draft's own Send delivers only that comment; Send all takes the rest.
#[test]
fn a_saved_draft_sends_alone_and_send_all_takes_the_rest() {
    use humata2_review::{Anchor, ReviewAction as R};
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Changes);
    app.send(AppAction::Review(R::SelectFile {
        repo: "api".into(),
        path: "src/client.rs".into(),
    }));
    let bodies = ["First draft goes alone.", "Second draft waits its turn."];
    for (line, body) in [(43, bodies[0]), (42, bodies[1])] {
        app.send(AppAction::Review(R::StartComment(Anchor {
            path: "src/client.rs".into(),
            line,
        })));
        app.send(AppAction::Review(R::CommentText(body.into())));
        app.send(AppAction::Review(R::SaveComment));
    }
    assert_eq!(app.ui.read(|s| s.review.queued().len()), 2);
    let delivered = |app: &App, body: &str| {
        app.fakes
            .daemon
            .workspace()
            .diffs
            .iter()
            .filter(|d| d.session == SessionId::new("ledger"))
            .flat_map(|d| &d.comments)
            .any(|c| c.body == body && c.sent)
    };
    assert!(app.contains("send-comment-0"));
    app.click("send-comment-0");
    assert!(delivered(&app, bodies[0]));
    assert!(!delivered(&app, bodies[1]));
    assert_eq!(app.ui.read(|s| s.review.queued().len()), 1);
    assert!(app.ui.read(|s| s.session.view().unwrap().queue.is_empty()));
    app.click("send-comments");
    assert!(delivered(&app, bodies[1]));
    assert!(app.ui.read(|s| s.review.queued().is_empty()));
}

/// SE-10: keyboard input follows focus; palette editing cannot type into the terminal.
#[test]
fn an_unfocused_terminal_does_not_receive_palette_keystrokes() {
    let mut app = App::new(fixture::design());
    app.cx
        .update_window(app.window, |_, window, _| window.activate_window())
        .unwrap();
    app.settle();
    app.open("ledger", Panel::Info);
    app.click("terminal");
    let key = (SessionId::new("ledger"), 0);
    let before = app.fakes.daemon.workspace().terminal_bytes[&key].clone();
    app.send(AppAction::OpenPalette);
    app.cx
        .update_window(app.window, |_, window, cx| {
            window.input("focus stays in search", cx);
            window.press("down", cx);
        })
        .unwrap();
    app.settle();
    assert_eq!(app.fakes.daemon.workspace().terminal_bytes[&key], before);
    app.cx
        .update_window(app.window, |_, window, cx| window.press("escape", cx))
        .unwrap();
    app.settle();
    app.click("terminal");
    app.cx
        .update_window(app.window, |_, window, cx| {
            for key in ["p", "w", "d", "enter"] {
                window.press(key, cx);
            }
        })
        .unwrap();
    app.settle();
    assert_ne!(app.fakes.daemon.workspace().terminal_bytes[&key], before);
}
