//! Session panel content is read through the existing clients, scoped to
//! the selected session/channel, and updated by the same daemon events.
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, Bounds, HeadlessAppContext, WindowBounds, WindowOptions, point, px, size,
};
use humata2_app::layout::PaneId;
use humata2_app::{AppAction, PaneAction, Route};
use humata2_clients::model::{Answer, ItemStatus, SessionId};
use humata2_clients::{Clients, Fakes, Workspace, fixture};
use humata2_engine::Resources;
use humata2_session::{Panel, SessionAction};
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
    fakes: Fakes,
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

/// RV-2, RV-3: selecting a file in the open session shows its own diff.
#[test]
fn changes_list_files_and_keep_selection_with_each_session() {
    let mut app = App::new(fixture::standard());
    app.open("s-auth", Panel::Changes);
    assert!(app.contains("change-file acme/api/src/client.rs"));
    assert!(!app.contains("change-file acme/web/src/auth/SignIn.tsx"));
    app.click("change-file acme/api/src/retry.rs");
    assert!(app.contains("change-code acme/api/src/retry.rs"));
    app.open("s-login", Panel::Changes);
    assert!(app.contains("change-code acme/web/src/auth/SignIn.tsx"));
    app.open("s-auth", Panel::Changes);
    assert!(app.contains("change-code acme/api/src/retry.rs"));
}

/// IN-3: a session's Inbox includes its forks; answering uses the daemon client.
#[test]
fn inbox_answers_the_selected_sessions_request_through_the_client() {
    let workspace = fixture::standard();
    let request = workspace
        .requests
        .iter()
        .find(|r| r.session == SessionId::new("s-auth") && r.answer.is_none())
        .expect("a pending auth request")
        .id
        .clone();
    let mut app = App::new(workspace);
    app.open("s-auth", Panel::Inbox);
    assert!(app.contains(&format!("request {request}")));
    app.click(&format!("allow {request}"));
    assert_eq!(
        app.fakes
            .daemon
            .workspace()
            .requests
            .iter()
            .find(|r| r.id == request)
            .expect("request kept")
            .answer,
        Some(Answer::Approved)
    );
    assert!(!app.contains(&format!("request {request}")));
}

/// BD-1: compact Board groups the open session's channel, then expands into lanes.
#[test]
fn board_lists_the_channels_items_and_expands_without_losing_the_session() {
    let workspace = fixture::standard();
    let session = workspace
        .session(&SessionId::new("s-auth"))
        .expect("auth session");
    let channel = session.channel.clone();
    let item = workspace
        .items
        .iter()
        .find(|item| {
            item.scope == humata2_clients::model::Scope::Channel(channel.clone())
                && item.status == ItemStatus::InProgress
        })
        .expect("a channel item")
        .id
        .clone();
    let mut app = App::new(workspace);
    app.open("s-auth", Panel::Board);
    assert!(app.contains(&format!("board-item {item}")));
    app.click("expand-panel");
    assert!(app.contains("panel-full-screen"));
    assert!(!app.contains("terminal"));
    app.click("expand-panel");
    assert!(app.contains("terminal"));
    assert!(app.contains(&format!("board-item {item}")));
}

/// D76: keep reviewable pixels of the implemented panel sections, in both appearances.
#[test]
fn the_panels_draw_their_client_data() {
    let snapshots = snapshots!().expect("snapshots beside this test");
    for (name, panel, full) in [
        ("changes", Panel::Changes, false),
        ("inbox", Panel::Inbox, false),
        ("board", Panel::Board, false),
        ("journal", Panel::Journal, false),
        ("files", Panel::Files, false),
        ("info", Panel::Info, false),
        ("changes-full", Panel::Changes, true),
        ("board-full", Panel::Board, true),
    ] {
        for (look, mode) in [
            ("light", gpui_kit::component::theme::ThemeMode::Light),
            ("dark", gpui_kit::component::theme::ThemeMode::Dark),
        ] {
            let mut app = App::new(fixture::standard());
            app.cx
                .update(|cx| gpui_kit::component::theme::Theme::change(mode, None, cx));
            app.open(
                if panel == Panel::Info {
                    "s-login"
                } else {
                    "s-auth"
                },
                panel,
            );
            if panel == Panel::Files {
                app.send(AppAction::Session(SessionAction::SelectFile {
                    repo: "acme/api".into(),
                    path: "src/client.rs".into(),
                }));
            }
            if full {
                app.send(AppAction::Session(SessionAction::ToggleWiden));
            }
            app.draw();
            let tree = app
                .cx
                .update_window(app.window, |_, window, _| view::capture(window))
                .expect("window open")
                .expect("view captured");
            snapshots.assert_text(&format!("{name}-{look}"), &tree);
            let image = app
                .cx
                .capture_screenshot(app.window)
                .expect("headless window draws");
            let bounds = app
                .cx
                .update_window(app.window, |_, window, _| {
                    window
                        .find(if full {
                            "panel-full-screen"
                        } else {
                            "side-panel"
                        })
                        .bounds()
                })
                .expect("the panel is drawn");
            let scale = image.width() as f32 / 1280.;
            let image = image::imageops::crop_imm(
                &image,
                (f32::from(bounds.origin.x) * scale).round() as u32,
                (f32::from(bounds.origin.y) * scale).round() as u32,
                (f32::from(bounds.size.width) * scale).round() as u32,
                (f32::from(bounds.size.height) * scale).round() as u32,
            )
            .to_image();
            snapshots.assert_image(&format!("{name}-{look}"), &image, Tolerance::default());
        }
    }
}

/// IN-3: descendants are included, other sessions stay outside this Inbox.
#[test]
fn the_session_inbox_includes_forks_and_denies_only_the_request_picked() {
    use humata2_clients::model::RequestId;
    let mut workspace = fixture::standard();
    let mut fork = workspace
        .session(&SessionId::new("s-auth"))
        .expect("auth session")
        .clone();
    fork.id = SessionId::new("s-auth-child");
    fork.parent = Some(SessionId::new("s-auth"));
    workspace.sessions.push(fork.clone());
    let mut request = workspace
        .requests
        .iter()
        .find(|r| r.session == SessionId::new("s-auth"))
        .expect("request")
        .clone();
    request.id = RequestId::new("r-child");
    request.session = fork.id;
    workspace.requests.push(request);
    let mut app = App::new(workspace);
    app.open("s-auth", Panel::Inbox);
    assert!(app.contains("request r-child"));
    assert!(!app.contains("request r-edit"));
    app.click("request r-child");
    app.click("deny r-child");
    let workspace = app.fakes.daemon.workspace();
    assert_eq!(
        workspace
            .requests
            .iter()
            .find(|r| r.id == RequestId::new("r-child"))
            .expect("child request")
            .answer,
        Some(Answer::Denied { reply: None })
    );
    assert_eq!(
        workspace
            .requests
            .iter()
            .find(|r| r.id == RequestId::new("r-test"))
            .expect("root request")
            .answer,
        None
    );
}

/// NV-12: the divider width excludes the new strip; dragging has no 64px jump.
#[test]
fn resizing_the_panel_keeps_the_strip_outside_its_width() {
    let mut app = App::new(fixture::standard());
    app.open("s-auth", Panel::Changes);
    app.draw();
    app.cx
        .update_window(app.window, |_, window, cx| {
            let start = window.find("panel-divider").bounds().center();
            window.drag(start, point(start.x - px(40.), start.y), cx);
        })
        .expect("divider is drawn");
    app.settle();
    let width = app.ui.read(|state| state.layout.panel.width);
    assert!(
        (width - 380.).abs() <= 2.,
        "width after a 40px drag: {width}"
    );
}

/// RV-5: the panel sends only the open session's queued drafts via the client.
#[test]
fn sending_comments_from_the_panel_delivers_the_open_sessions_queue() {
    use humata2_review::{Anchor, ReviewAction};
    let mut app = App::new(fixture::standard());
    app.open("s-auth", Panel::Changes);
    app.send(AppAction::Review(ReviewAction::StartComment(Anchor {
        path: "src/client.rs".into(),
        line: 42,
    })));
    app.send(AppAction::Review(ReviewAction::CommentText(
        "Surface a second auth failure".into(),
    )));
    app.send(AppAction::Review(ReviewAction::SaveComment));
    assert!(app.contains("send-comments"));
    app.click("send-comments");
    assert!(app.ui.read(|state| state.review.queued().is_empty()));
    let workspace = app.fakes.daemon.workspace();
    let delivered: Vec<_> = workspace
        .diffs
        .iter()
        .filter(|d| d.session == SessionId::new("s-auth"))
        .flat_map(|d| &d.comments)
        .filter(|c| c.body == "Surface a second auth failure")
        .collect();
    assert_eq!(delivered.len(), 1);
    assert!(delivered[0].sent);
    assert!(!app.contains("send-comments"));
}

/// IN-6: a question cannot accidentally use the permission approval button.
#[test]
fn questions_do_not_offer_a_permission_approval() {
    use humata2_clients::model::RequestKind;
    let mut workspace = fixture::standard();
    let request = workspace
        .requests
        .iter_mut()
        .find(|r| r.session == SessionId::new("s-auth"))
        .expect("auth request");
    request.kind = RequestKind::Question;
    let request_id = request.id.clone();
    let mut app = App::new(workspace);
    app.open("s-auth", Panel::Inbox);
    assert!(app.contains(&format!("request {request_id}")));
    assert!(!app.contains(&format!("allow {request_id}")));
    assert_eq!(
        app.fakes
            .daemon
            .workspace()
            .requests
            .iter()
            .find(|r| r.id == request_id)
            .expect("request kept")
            .answer,
        None
    );
}

/// D76, BD-1: Backlog and In review are real client values and neither
/// disappears when the compact board becomes the expanded kanban.
#[test]
fn expanded_board_keeps_backlog_and_in_review_items() {
    use humata2_clients::model::ItemId;
    let mut workspace = fixture::standard();
    let source = workspace
        .items
        .iter()
        .find(|item| item.id == ItemId::new("A-1"))
        .expect("api item")
        .clone();
    for (id, status) in [
        ("A-backlog", ItemStatus::Backlog),
        ("A-review", ItemStatus::InReview),
    ] {
        let mut item = source.clone();
        item.id = ItemId::new(id);
        item.status = status;
        workspace.items.push(item);
    }
    let mut app = App::new(workspace);
    app.open("s-auth", Panel::Board);
    assert!(app.contains("board-item A-backlog"));
    assert!(app.contains("board-item A-review"));
    app.click("expand-panel");
    assert!(app.contains("board-item A-backlog"));
    assert!(app.contains("board-item A-review"));
    assert!(!app.contains("board-item W-2"));
}

/// Files and Info are reachable from the native strip, rather than empty panels.
#[test]
fn files_and_info_show_the_open_sessions_data() {
    let mut app = App::new(fixture::standard());
    app.open("s-auth", Panel::Changes);
    app.click("panel-tab Files");
    assert!(app.contains("session-files"));
    app.click("session-file acme/api/src/client.rs");
    app.ui.read(|state| {
        assert_eq!(
            state.session.view().expect("session").file.as_ref(),
            Some(&("acme/api".to_owned(), "src/client.rs".to_owned()))
        )
    });

    app.click("panel-tab Info");
    assert!(app.contains("session-info s-auth"));
    app.open("s-login", Panel::Info);
    assert!(app.contains("session-info s-login"));
    assert!(!app.contains("session-info s-auth"));
    app.click("session-info-fork s-passkey");
    app.ui
        .read(|state| assert_eq!(state.session.open, Some(SessionId::new("s-passkey"))));
}

#[test]
fn session_files_filter_fold_and_restore_without_showing_other_clones() {
    use humata2_clients::model::SessionFile;
    let mut workspace = fixture::standard();
    workspace
        .session_files
        .get_mut(&SessionId::new("s-login"))
        .expect("fixture listing")
        .push(SessionFile {
            repo: "private/other".into(),
            path: "secret.txt".into(),
            folder: false,
        });
    let mut app = App::new(workspace);
    app.open("s-auth", Panel::Files);
    assert!(!app.contains("session-file private/other/secret.txt"));
    assert!(app.contains("session-file acme/api/src/lib.rs"));
    app.click("session-folder acme/api/src");
    assert!(!app.contains("session-file acme/api/src/lib.rs"));
    app.click("session-file-filter");
    app.cx
        .update_window(app.window, |_, window, cx| window.input("CLIENT", cx))
        .expect("window");
    app.settle();
    app.ui
        .read(|state| assert_eq!(state.session.view().expect("view").file_filter, "CLIENT"));
    assert!(app.contains("session-file acme/api/src/client.rs"));
    assert!(!app.contains("session-file acme/api/src/lib.rs"));
    app.open("s-login", Panel::Files);
    let saved = humata2_app::saved::restore(app.fakes.storage.as_ref())
        .expect("saved state decodes")
        .expect("state saved");
    assert_eq!(
        saved.sessions[&SessionId::new("s-auth")].file_filter,
        "CLIENT"
    );
    assert!(app.contains("session-file private/other/secret.txt"));
    app.open("s-auth", Panel::Files);
    app.ui
        .read(|state| assert_eq!(state.session.view().expect("view").file_filter, "CLIENT"));
    app.send(AppAction::Session(
        SessionAction::FilterFiles(String::new()),
    ));
    assert!(!app.contains("session-file acme/api/src/client.rs"));
    app.click("session-folder acme/api/src");
    assert!(app.contains("session-file acme/api/src/lib.rs"));
    app.click("session-file acme/api/src/client.rs");
    app.click("files-open-big");
    assert!(app.contains("session-file"));
    assert!(
        app.ui
            .read(|s| matches!(s.session.view().expect("view").selected(0),
        Some(humata2_session::Tab::File {path,..}) if path=="src/client.rs"))
    );
    app.click("terminal-tab 0");
    assert!(app.contains("terminal"));
    assert!(app.contains("session-files"));
}

#[test]
fn saved_sessions_from_before_file_views_still_restore() {
    use humata2_clients::Storage as _;
    let storage = humata2_clients::MemoryStorage::new();
    storage.save(humata2_app::saved::KEY, br#"{"sessions":{"s-auth":{"tab":0,"panel":"Files","widened":false,"draft":"keep this","queue":[]}}}"#).expect("old state stored");
    let saved = humata2_app::saved::restore(&storage)
        .expect("old state decodes")
        .expect("state exists");
    let view = &saved.sessions[&SessionId::new("s-auth")];
    assert_eq!(view.panel, Some(Panel::Files));
    assert_eq!(view.draft, "keep this");
    assert!(view.file.is_none());
    assert!(view.file_filter.is_empty());
    assert!(view.folded_files.is_empty());
}
