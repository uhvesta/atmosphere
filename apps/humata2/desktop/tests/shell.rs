//! NV-1, HM-1: the window on the standard fakes as it first opens, on Home,
//! drawn headless (nothing is shown), kept as view and image snapshots.

use std::sync::Arc;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, Bounds, HeadlessAppContext, WindowBounds, WindowOptions, point, px, size};
use humata2_clients::{Clients, fixture};
use humata2_engine::Resources;
use humata2_app::AppAction;
use humata2_layout::{Axis, LayoutAction};
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
    let (clients, _fakes) = Clients::fake(fixture::standard(), Arc::new(TestClock::new()));
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

/// The close buttons the panes' headers show.
fn pane_headers(tree: &str) -> usize {
    tree.lines().filter(|line| line.trim_start().starts_with("Button \"Close ")).count()
}

#[test]
fn splitting_draws_two_panes_and_closing_one_goes_back() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a current-thread runtime starts");
    let mut cx = humata2_engine::headless(&Resources::new(humata2_ui::Assets)).expect("the headless context starts");
    cx.update(humata2_ui::init);
    let (clients, _fakes) = Clients::fake(fixture::standard(), Arc::new(TestClock::new()));
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
    assert_eq!(pane_headers(&capture(&mut cx)), 0, "one pane draws as the plain screen");

    cx.update(|cx| ui.send(AppAction::Panes(LayoutAction::SplitFocused(Axis::Horizontal)), cx));
    settle(&runtime, &cx);
    assert_eq!(pane_headers(&capture(&mut cx)), 2, "a split shows both panes");

    cx.update(|cx| ui.send(AppAction::Panes(LayoutAction::CloseFocused), cx));
    settle(&runtime, &cx);
    assert_eq!(pane_headers(&capture(&mut cx)), 0, "closing one goes back to the plain screen");
}
