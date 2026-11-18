//! Snapshots of the gallery: its view tree (roles, labels and text), its
//! layout. Once plain, once in Source mode, and once annotated with comments
//! (one on a passage, one on the table, one on the diagram, one whose passage
//! is gone) over a diff against an older gallery. The pixels are in
//! `pixels.rs`, in their own process: these font-less contexts would otherwise
//! fix the process-wide mono font to the system UI font. Everything runs
//! headless; no window is shown.
//!
//! Record with `bazel run //apps/humata2/markdown/headless:snapshots_record`
//! and read every file it writes in `tests/__snapshots__/snapshots/`.

// `#[gpui_kit::test]` expands to paths under `gpui`.
use gpui_kit::gpui;
use gpui_kit::prelude::*;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, TestAppContext};
use humata2_markdown::ViewMode;
use humata2_markdown_headless::{GALLERY, annotate, gallery_options, init, open};
use humata2_snapshot::{layout, snapshots, view};

const WIDTH: f32 = 900.;
const HEIGHT: f32 = 1400.;

fn draw(cx: &mut TestAppContext, window: AnyWindowHandle) -> (String, humata2_snapshot::layout::Layout) {
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
    })
    .expect("the window is open");
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        let tree = view::capture(window).unwrap_or_else(|error| panic!("{error}"));
        (tree, layout::capture(window))
    })
    .expect("the window is open")
}

fn check(name: &str, tree: &str, layout: &humata2_snapshot::layout::Layout) {
    let snapshots = snapshots!().unwrap_or_else(|error| panic!("{error}"));
    snapshots.assert_text(name, tree);
    snapshots.assert_layout(name, layout);
}

/// SM-3
#[gpui_kit::test]
fn the_gallery_draws_every_kind_of_block(cx: &mut TestAppContext) {
    init(cx);
    let (window, _) = open(cx, GALLERY, gallery_options(), WIDTH, HEIGHT);
    let (tree, layout) = draw(cx, window);
    check("gallery", &tree, &layout);
}

/// SM-4
#[gpui_kit::test]
fn the_gallery_in_source_mode_shows_its_markdown(cx: &mut TestAppContext) {
    init(cx);
    let (window, pane) = open(cx, GALLERY, gallery_options(), WIDTH, HEIGHT);
    let view = pane.read_with(cx, |pane, _| pane.view.clone());
    view.update(cx, |view, cx| view.set_mode(ViewMode::Source, cx));
    let (tree, layout) = draw(cx, window);
    check("gallery-source", &tree, &layout);
}

/// SM-5, SM-7, SM-8
#[gpui_kit::test]
fn the_annotated_gallery_shows_comments_in_the_margin_and_changes_in_the_gutter(cx: &mut TestAppContext) {
    init(cx);
    let (window, pane) = open(cx, GALLERY, gallery_options(), WIDTH, HEIGHT);
    let view = pane.read_with(cx, |pane, _| pane.view.clone());
    view.update(cx, annotate);
    let (tree, layout) = draw(cx, window);
    check("gallery-annotated", &tree, &layout);
}
