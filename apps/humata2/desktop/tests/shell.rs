//! NV-1, HM-1: the window on the standard fakes as it first opens, on Home,
//! drawn headless (nothing is shown), kept as view and image snapshots.

use std::sync::Arc;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, Bounds, HeadlessAppContext, WindowBounds, WindowOptions, point, px, size};
use humata2_clients::{Clients, fixture};
use humata2_engine::Resources;
use humata2_app::layout::PaneId;
use humata2_app::{AppAction, PaneAction, Route};
use humata2_session::{Panel, SessionAction};
use humata2_snapshot::{Tolerance, snapshots, view};
use humata2_ui::bridge::Ui;
use humata2_ui::record::Recorder;
use tca::TestClock;
use tokio::runtime::Runtime;

fn settle(runtime: &Runtime, cx: &HeadlessAppContext) {
    for _ in 0..10 {
        runtime.block_on(async {
            for _ in 0..32 {
                tokio::task::yield_now().await;
            }
        });
        cx.run_until_parked();
    }
}

#[test]
fn home_on_the_standard_fakes() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a current-thread runtime starts");
    let mut cx = humata2_engine::headless(&Resources::new(humata2_ui::Assets)).expect("the headless context starts");
    cx.update(humata2_ui::init);
    let (clients, _fakes) = humata2_clients::fake_clients(fixture::standard(), Arc::new(TestClock::new()));
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
    let (window, _shell): (AnyWindowHandle, _) =
        cx.update(|cx| humata2_ui::open(cx, &ui, options)).expect("the window opens");
    settle(&runtime, &cx);
    let snapshots = snapshots!().expect("snapshots are kept beside this file");
    let tree = cx
        .update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
            view::capture(window)
        })
        .expect("the window is open")
        .expect("the view can be read");
    snapshots.assert_text("home.view", &tree);
    let image = cx.capture_screenshot(window).expect("the window draws");
    snapshots.assert_image("home", &image, Tolerance::default());
}


/// Whether the window draws the session's side panel.
fn shows_panel(tree: &str) -> bool {
    tree.lines().any(|line| line.trim_start().starts_with("Complementary "))
}

#[test]
fn a_sessions_side_panel_hides_shows_and_survives_switching_screens() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a current-thread runtime starts");
    let mut cx = humata2_engine::headless(&Resources::new(humata2_ui::Assets)).expect("the headless context starts");
    cx.update(humata2_ui::init);
    let workspace = fixture::standard();
    let session = workspace.sessions.first().expect("the fakes have a session").id.clone();
    let (clients, _fakes) = humata2_clients::fake_clients(workspace, Arc::new(TestClock::new()));
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
    let (window, _shell): (AnyWindowHandle, _) =
        cx.update(|cx| humata2_ui::open(cx, &ui, options)).expect("the window opens");
    settle(&runtime, &cx);
    let capture = |cx: &mut HeadlessAppContext| {
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            view::capture(window)
        })
        .expect("the window is open")
        .expect("the view can be read")
    };
    let send = |action: AppAction, cx: &mut HeadlessAppContext| {
        cx.update(|cx| ui.send(action, cx));
        settle(&runtime, cx);
    };
    send(AppAction::Navigate(Route::Session(session.clone())), &mut cx);
    assert!(!shows_panel(&capture(&mut cx)), "a session opens without a panel");

    send(AppAction::Session(SessionAction::SetPanel(Some(Panel::Changes))), &mut cx);
    assert!(shows_panel(&capture(&mut cx)), "opening the Changes panel shows it on the right");

    send(AppAction::Pane(PaneAction::Toggle(PaneId::Panel)), &mut cx);
    assert!(!shows_panel(&capture(&mut cx)), "⌥⌘0 hides it");
    send(AppAction::Pane(PaneAction::Toggle(PaneId::Panel)), &mut cx);
    assert!(shows_panel(&capture(&mut cx)), "and shows it again");

    send(AppAction::Navigate(Route::Home), &mut cx);
    assert!(!shows_panel(&capture(&mut cx)), "Home has no side panel");
    send(AppAction::Navigate(Route::Session(session)), &mut cx);
    assert!(shows_panel(&capture(&mut cx)), "going back to the session brings its panel back");
}
