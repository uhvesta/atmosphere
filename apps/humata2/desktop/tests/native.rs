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
    /// Writes a review comment on a line and keeps it as a draft.
    fn write_comment(&mut self, session: &str, repo: &str, line: u32, body: &str) {
        use humata2_clients::comments::{Anchor, Side};
        use humata2_review::comments::CommentsAction as C;
        let id = SessionId::new(session);
        let anchor = Anchor {
            repo: repo.into(),
            path: "src/client.rs".into(),
            side: Side::New,
            first: line,
            last: line,
            quoted: Vec::new(),
            base: None,
            detached: false,
            outdated: false,
        };
        self.send(AppAction::Comments(C::OpenBox(id.clone(), anchor)));
        self.send(AppAction::Comments(C::Typed(id.clone(), body.into())));
        self.send(AppAction::Comments(C::Close(id)));
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
    /// Folds the two channels the frames draw folded (pr-reviews and web).
    fn fold_quiet_channels(&mut self) {
        for id in ["pr-reviews", "web"] {
            self.send(AppAction::Sidebar(SidebarAction::Toggle(RowKey::Channel(
                humata2_clients::model::ChannelId::new(id),
            ))));
        }
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

/// The snapshot folder beside this file. Tests under `native/` take it from
/// here, because `snapshots!()` names the folder after the file that calls it.
fn folder() -> Snapshots {
    snapshots!().expect("snapshot folder")
}

// SEAM: archiving with unsubmitted PR drafts (D513).
#[path = "native/archive_drafts.rs"]
mod archive_drafts;
#[path = "native/board.rs"]
mod board;
#[path = "native/channel.rs"]
mod channel;
#[path = "native/channel_desk.rs"]
mod channel_desk;
#[path = "native/channel_desk_reopen.rs"]
mod channel_desk_reopen;
#[path = "native/close_running.rs"]
mod close_running;
#[path = "native/desk.rs"]
mod desk;
#[path = "native/desk_events.rs"]
mod desk_events;
#[path = "native/doc_versions.rs"]
mod doc_versions;
#[path = "native/document.rs"]
mod document;
#[path = "native/editor.rs"]
mod editor;
#[path = "native/editor_comments.rs"]
mod editor_comments;
#[path = "native/fork_move.rs"]
mod fork_move;
// AGENTSET (D725): Settings › Agents switches.
#[path = "native/agent_switches.rs"]
mod agent_switches;
#[path = "native/inbox_terminal.rs"]
mod inbox_terminal;
#[path = "native/hosts_settings.rs"]
mod hosts_settings;
// PROVIDERS: Settings › Machines › Providers, frames A–E.
#[path = "native/providers_settings.rs"]
mod providers_settings;
#[path = "native/lists.rs"]
mod lists;
#[path = "native/machines.rs"]
mod machines;
#[path = "native/marks.rs"]
mod marks;
#[path = "native/new_channel.rs"]
mod new_channel;
#[path = "native/overlays.rs"]
mod overlays;
#[path = "native/queue.rs"]
mod queue;
#[path = "native/review.rs"]
mod review;
#[path = "native/review_switches.rs"]
mod review_switches;
#[path = "native/pr_review.rs"]
mod pr_review;
#[path = "native/pr_session.rs"]
mod pr_session;
// SEAM: the review box's Ask, Draft for GitHub and Make it a draft (D631).
#[path = "native/pr_ask.rs"]
mod pr_ask;
#[path = "native/push.rs"]
mod push;
#[path = "native/session.rs"]
mod session;
#[path = "native/session_menu.rs"]
mod session_menu;
#[path = "native/shell.rs"]
mod shell;
#[path = "native/stop.rs"]
mod stop;
#[path = "native/targets.rs"]
mod targets;
#[path = "native/terminal.rs"]
mod terminal;
#[path = "native/views.rs"]
mod views;
#[path = "native/channel_desk_target.rs"]
mod channel_desk_target;
#[path = "native/desk_target.rs"]
mod desk_target;
#[path = "native/diff_view.rs"]
mod diff_view;
#[path = "native/diff_big.rs"]
mod diff_big;
#[path = "native/diff_full_screen.rs"]
mod diff_full_screen;
#[path = "native/diff_stress.rs"]
mod diff_stress;
#[path = "native/stack.rs"]
mod stack;
#[path = "native/review_ui.rs"]
mod review_ui;
