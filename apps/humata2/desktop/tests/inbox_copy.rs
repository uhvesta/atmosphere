//! IN-10, D467: an agent's cross-machine copy is one card: what it copies,
//! its size, from which machine to which, and where; Approve, Somewhere
//! else… and Deny; the real paths only behind Details. Approve answers the
//! request through the daemon client.
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, Bounds, HeadlessAppContext, WindowBounds, WindowOptions, point, px, size,
};
use humata2_app::layout::PaneId;
use humata2_app::{AppAction, PaneAction, Route};
use humata2_clients::model::{Answer, CopyCard, Preview, RequestId, SessionId};
use humata2_clients::{Clients, Fakes, Workspace, fixture};
use humata2_engine::Resources;
use humata2_session::{Panel, SessionAction};
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

/// A workspace whose waiting request in `s-auth` is a copy between two of its machines.
fn with_copy() -> (Workspace, RequestId) {
    let mut workspace = fixture::standard();
    let (from, to) = (workspace.machines[0].id.clone(), workspace.machines[1].id.clone());
    let request = workspace
        .requests
        .iter_mut()
        .find(|r| r.session == SessionId::new("s-auth") && r.answer.is_none())
        .expect("a pending auth request");
    request.preview = Preview::CrossMachineCopy(CopyCard {
        from,
        to,
        source_path: "datasets".to_owned(),
        destination_path: "/home/ml/Humata/shared".to_owned(),
        place: "ml shared".to_owned(),
        bytes: 2_100_000_000,
        files: 3,
    });
    let id = request.id.clone();
    (workspace, id)
}

#[test]
fn a_copy_card_shows_what_where_and_the_three_answers_with_paths_behind_details() {
    let (workspace, id) = with_copy();
    let mut app = App::new(workspace);
    app.open("s-auth", Panel::Inbox);
    for part in ["session", "what", "size", "from", "to", "place"] {
        assert!(app.contains(&format!("copy {part} {id}")), "the card has no {part}");
    }
    for answer in ["approve", "somewhere else", "deny"] {
        assert!(app.contains(&format!("{answer} {id}")), "the card has no {answer}");
    }
    assert!(!app.contains(&format!("details {id}")));
    app.click(&format!("toggle details {id}"));
    assert!(app.contains(&format!("details {id}")));
    app.click(&format!("toggle details {id}"));
    assert!(!app.contains(&format!("details {id}")));
}

#[test]
fn approving_a_copy_card_answers_the_request_through_the_client() {
    let (workspace, id) = with_copy();
    let mut app = App::new(workspace);
    app.open("s-auth", Panel::Inbox);
    app.click(&format!("approve {id}"));
    let answer = app
        .fakes
        .daemon
        .workspace()
        .requests
        .iter()
        .find(|r| r.id == id)
        .and_then(|r| r.answer.clone());
    assert_eq!(answer, Some(Answer::Approved));
}

#[test]
fn somewhere_else_opens_the_files_page_and_keeps_the_request_waiting() {
    let (workspace, id) = with_copy();
    let mut app = App::new(workspace);
    app.open("s-auth", Panel::Inbox);
    app.click(&format!("somewhere else {id}"));
    let answer = app
        .fakes
        .daemon
        .workspace()
        .requests
        .iter()
        .find(|r| r.id == id)
        .and_then(|r| r.answer.clone());
    assert_eq!(answer, None);
    assert!(app.contains("files-page"));
}
