//! What the review pane shows, as view (roles, labels, text), layout and
//! image snapshots: a review across two repos with code, markdown, images
//! and a binary; a prepared review's groups and notes; markdown rendered;
//! and each image mode drawn. Everything runs headless; no window is shown.
//!
//! `bazel run //apps/humata2/review:snapshots_record` records them; read
//! every file it writes in `tests/__snapshots__/snapshots/` before keeping it.

use gpui_kit::prelude::*;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, Entity, TestAppContext};
use humata2_review_harness::{Fakes, open_pane, pr_review, prepared, small_review};
use humata2_review_view::ReviewPane;
use humata2_review_view::model::{
    AgentState, CommentBook, FileKey, ImageMode, ImageView, Review, ReviewMemory, ReviewSnapshot,
    SessionId,
};
use humata2_snapshot::{layout, snapshots, view};

const WIDTH: f32 = 1280.;
const HEIGHT: f32 = 900.;

fn session() -> SessionId {
    SessionId::new("s1")
}

/// Saves `memory` for the session, as if the pane had been open before.
fn remembered(fakes: &Fakes, memory: ReviewMemory) {
    let snapshot = ReviewSnapshot {
        revision: 1,
        book: CommentBook::new(session()),
        memory,
        composer: None,
    };
    if let Err(error) = fakes.store().save(&snapshot) {
        panic!("the fake store saves: {error}");
    }
}

fn show(cx: &mut TestAppContext, pane: &Entity<ReviewPane>, review: Review) {
    pane.update(cx, |pane, cx| {
        pane.show_session(session(), cx);
        pane.set_review(session(), review, cx);
        pane.set_agent_state(session(), AgentState::Idle, cx);
    });
    cx.run_until_parked();
}

fn capture(cx: &mut TestAppContext, window: AnyWindowHandle) -> (String, layout::Layout) {
    cx.executor().advance_clock(std::time::Duration::from_millis(400));
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
        let tree = view::capture(window).unwrap_or_else(|error| panic!("the view can be read: {error}"));
        (tree, layout::capture(window))
    })
    .unwrap_or_else(|error| panic!("the window is open: {error}"))
}

#[gpui_kit::test]
fn a_review_across_two_repos_shows_each_kind_of_change(cx: &mut TestAppContext) {
    let snapshots = snapshots!().unwrap_or_else(|error| panic!("{error}"));
    let fakes = Fakes::new();
    let (window, pane) = open_pane(cx, &fakes, WIDTH, HEIGHT);
    show(cx, &pane, small_review());
    let (tree, layout) = capture(cx, window);
    let saved = fakes.saved(&session()).expect("the review was saved");
    pane.read_with(cx, |pane, _| {
        assert_eq!(saved.revision, pane.session(&session()).expect("session loaded").revision());
    });
    snapshots.assert_text("small-review", &tree);
    snapshots.assert_layout("small-review", &layout);
}

#[gpui_kit::test]
fn a_prepared_review_shows_its_groups_in_order_with_its_notes(cx: &mut TestAppContext) {
    let snapshots = snapshots!().unwrap_or_else(|error| panic!("{error}"));
    let fakes = Fakes::new();
    let (window, pane) = open_pane(cx, &fakes, WIDTH, HEIGHT);
    show(cx, &pane, pr_review());
    pane.update(cx, |pane, cx| pane.open_prepared(session(), prepared(), cx));
    cx.run_until_parked();
    let (tree, _) = capture(cx, window);
    snapshots.assert_text("prepared-review", &tree);
}

#[gpui_kit::test]
fn a_markdown_file_shown_rendered_marks_what_changed(cx: &mut TestAppContext) {
    let snapshots = snapshots!().unwrap_or_else(|error| panic!("{error}"));
    let fakes = Fakes::new();
    let mut memory = ReviewMemory::default();
    memory.rendered.insert(FileKey::new("app", "README.md"));
    remembered(&fakes, memory);
    let (window, pane) = open_pane(cx, &fakes, WIDTH, HEIGHT);
    show(cx, &pane, small_review());
    let (tree, _) = capture(cx, window);
    snapshots.assert_text("markdown-rendered", &tree);
}

/// The image files of [`small_review`] alone, so they're all on screen.
#[cfg(target_os = "macos")]
fn images_only() -> Review {
    use humata2_review_view::model::Content;
    let files = small_review()
        .files()
        .iter()
        .filter(|file| matches!(file.content, Content::Image(_) | Content::Binary { .. }))
        .cloned()
        .collect();
    Review::new(files)
}

#[cfg(target_os = "macos")]
#[test]
fn each_image_mode_draws_as_recorded() {
    use gpui_kit::AppContext as _;
    use humata2_review_harness::options;

    let snapshots = snapshots!().unwrap_or_else(|error| panic!("{error}"));
    let swipe = ImageMode::Swipe { position: 300 };
    let onion = ImageMode::Onion { opacity: 500 };
    let cases = [
        ("side-by-side", ImageMode::SideBySide, false),
        ("swipe", swipe, false),
        ("onion-skin", onion, false),
        ("changed-pixels", ImageMode::SideBySide, true),
    ];
    for (name, mode, highlight) in cases {
        let fakes = Fakes::new();
        let mut memory = ReviewMemory::default();
        memory.images.insert(FileKey::new("app", "assets/logo.png"), ImageView { mode, highlight });
        remembered(&fakes, memory);
        let mut cx = humata2_snapshot::headless_context();
        let (sink, store, history) = (fakes.sink(), fakes.store(), fakes.history());
        let (window, pane) = cx
            .update(|cx| {
                gpui_kit::open_window(options(WIDTH, HEIGHT), cx, |_, cx| {
                    cx.new(|cx| ReviewPane::new(sink, store, cx).with_history(history))
                })
            })
            .unwrap_or_else(|error| panic!("the headless window opens: {error}"));
        pane.update(&mut cx, |pane, cx| {
            pane.show_session(session(), cx);
            pane.set_review(session(), images_only(), cx);
            pane.set_agent_state(session(), AgentState::Idle, cx);
        });
        cx.run_until_parked();
        cx.advance_clock(std::time::Duration::from_millis(400));
        cx.run_until_parked();
        let (image, layout) = cx
            .update_window(window, |_, window, cx| {
                window.render_frame(cx);
                window.render_frame(cx);
                (window.render_to_image(), layout::capture(window))
            })
            .unwrap_or_else(|error| panic!("the window is open: {error}"));
        let stats = pane.read_with(&cx, |pane, _| pane.stats());
        assert!(stats.images.decoded >= 3, "the images were decoded before drawing: {stats:?}");
        let image = image.unwrap_or_else(|error| panic!("Metal draws the window off screen: {error}"));
        snapshots.assert_image(name, &image, humata2_snapshot::Tolerance::default());
        snapshots.assert_layout(name, &layout);
    }
}
