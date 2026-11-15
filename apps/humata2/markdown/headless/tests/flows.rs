//! The Markdown view driven the way a person drives it: drags, clicks, keys
//! and the scroll wheel through the real view. Everything runs headless; no
//! window is shown.
//!
//! Each flow writes a markdown artifact of its steps to the test's outputs.
//! Snapshots a flow takes are kept in `tests/__snapshots__/flows/`; record
//! them with `bazel run //apps/humata2/markdown/headless:flows_record` and
//! read every file before keeping it.

use std::path::PathBuf;

// `#[gpui_kit::test]` expands to paths under `gpui`.
use gpui_kit::gpui;
use gpui_kit::Focusable as _;
use gpui_kit::prelude::*;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, Entity, Role, ScrollDelta, TestAppContext, point, px};
use humata2_flow::{Flow, GpuiDriver, SpecFlow, Target, artifact_root, run};
use humata2_markdown::{AnchorStatus, AnchorTarget, MarkdownEvent, MarkdownOptions};
use humata2_markdown_headless::{GALLERY, Pane, gallery_options, init, open};
use humata2_snapshot::snapshots;

const WIDTH: f32 = 900.;
const HEIGHT: f32 = 1400.;

fn play(cx: &mut TestAppContext, window: AnyWindowHandle, flow: Flow) {
    let snapshots = snapshots!().unwrap_or_else(|error| panic!("{error}"));
    let out = artifact_root(&option_env!("CARGO_TARGET_TMPDIR").map_or_else(std::env::temp_dir, PathBuf::from));
    if let Err(error) = run(&flow, &mut GpuiDriver::new(cx, window), &snapshots, &out) {
        panic!("{error}");
    }
}

fn events(cx: &mut TestAppContext, pane: &Entity<Pane>) -> Vec<MarkdownEvent> {
    pane.read_with(cx, |pane, _| pane.events.clone())
}

fn frame(cx: &mut TestAppContext, window: AnyWindowHandle) {
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
    })
    .expect("the window is open");
}

/// SM-6, SM-8, 26.20
#[gpui_kit::test]
fn selecting_across_blocks_and_commenting_anchors_the_comment_to_source_lines(cx: &mut TestAppContext) {
    init(cx);
    let (window, pane) = open(cx, GALLERY, gallery_options(), WIDTH, HEIGHT);
    play(
        cx,
        window,
        Flow::new(SpecFlow::new("26.20", "Iterate on a spec with the agent, comment on a passage"))
            .drag(
                Target::new(Role::Paragraph, "Paragraph 2"),
                Target::new(Role::Heading, "Heading 3, level 2"),
            )
            .snapshot()
            .click(Role::Button, "Comment on selection")
            .snapshot(),
    );
    let requested = events(cx, &pane)
        .into_iter()
        .find_map(|event| match event {
            MarkdownEvent::CommentRequested(selection) => Some(selection),
            _ => None,
        })
        .expect("the view asked for a comment");
    assert_eq!(requested.target, AnchorTarget::Text);
    // From the middle of the paragraph (lines 3-4) into the heading (line 6).
    assert!(requested.lines.start >= 3 && requested.lines.end <= 6, "{:?}", requested.lines);
    let source = &GALLERY[requested.bytes.clone()];
    assert_eq!(requested.source, source);
    assert!(!requested.quote.contains("**"), "the quote is what the reader saw");
    let statuses = pane.read_with(cx, |pane, cx| pane.view.read(cx).comment_statuses().to_vec());
    assert!(matches!(statuses.as_slice(), [AnchorStatus::Current { .. }]));
}

/// SM-6, SM-8, 26.20
#[gpui_kit::test]
fn commenting_on_a_table_from_the_gutter_targets_the_whole_table(cx: &mut TestAppContext) {
    init(cx);
    let (window, pane) = open(cx, GALLERY, gallery_options(), WIDTH, HEIGHT);
    play(
        cx,
        window,
        Flow::new(SpecFlow::new("26.20", "Iterate on a spec with the agent, comment on a table"))
            .click(Role::Button, "Comment on table 11")
            .snapshot(),
    );
    let comments = pane.read_with(cx, |pane, cx| pane.view.read(cx).comments().to_vec());
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].anchor.target, AnchorTarget::Block { index: 10 });
    assert!(comments[0].anchor.source.starts_with("| Block"));
}

/// SM-6
#[gpui_kit::test]
fn commenting_on_a_diagram_frames_the_diagram(cx: &mut TestAppContext) {
    init(cx);
    let (window, pane) = open(cx, GALLERY, gallery_options(), WIDTH, HEIGHT);
    play(
        cx,
        window,
        Flow::new(SpecFlow::new("26.20", "Iterate on a spec with the agent, comment on a diagram"))
            .click(Role::Button, "Comment on diagram 16")
            .snapshot(),
    );
    let comments = pane.read_with(cx, |pane, cx| pane.view.read(cx).comments().to_vec());
    assert_eq!(comments[0].anchor.target, AnchorTarget::Diagram { block: 15 });
}

/// SM-3
#[gpui_kit::test]
fn copying_a_selection_puts_its_markdown_or_its_text_on_the_clipboard(cx: &mut TestAppContext) {
    init(cx);
    let (window, _pane) = open(cx, GALLERY, gallery_options(), WIDTH, HEIGHT);
    let drag = || {
        Flow::new(SpecFlow::new("26.20", "Iterate on a spec with the agent, copy a passage")).drag(
            Target::new(Role::Heading, "Heading 1, level 1").at_start(),
            Target::new(Role::Paragraph, "Paragraph 2"),
        )
    };
    play(cx, window, drag().click(Role::Button, "Copy as Markdown"));
    let markdown = cx
        .read_from_clipboard()
        .and_then(|item| item.text())
        .expect("the copy put text on the clipboard");
    assert!(markdown.starts_with("# Gallery"), "{markdown:?}");
    assert!(markdown.contains("**strong**"), "{markdown:?}");

    play(cx, window, drag().click(Role::Button, "Copy as text"));
    let text = cx
        .read_from_clipboard()
        .and_then(|item| item.text())
        .expect("the copy put text on the clipboard");
    assert!(text.contains("Gallery") && text.contains("strong"), "{text:?}");
    assert!(!text.contains('#') && !text.contains("**"), "{text:?}");
}

/// SM-3
#[gpui_kit::test]
fn clicking_a_link_reports_it_and_opens_nothing(cx: &mut TestAppContext) {
    init(cx);
    let source = "[Open the spec](https://example.com/spec)\n\nAfter.\n";
    let (window, pane) = open(cx, source, MarkdownOptions::new("Links"), WIDTH, 400.);
    frame(cx, window);
    cx.update_window(window, |_, window, cx| {
        window.click_at(("md-block", 0usize), point(px(12.), px(8.)), cx);
    })
    .expect("the window is open");
    cx.run_until_parked();
    assert_eq!(
        events(cx, &pane).last(),
        Some(&MarkdownEvent::LinkActivated {
            url: "https://example.com/spec".into()
        })
    );
}

/// SM-3
#[gpui_kit::test]
fn the_scroll_wheel_moves_a_long_document_and_only_visible_blocks_are_drawn(cx: &mut TestAppContext) {
    init(cx);
    let source: String = (1..=2500)
        .map(|n| format!("Paragraph number {n}, line one\nline two\nline three\n\n"))
        .collect();
    let (window, pane) = open(cx, &source, MarkdownOptions::new("Long"), WIDTH, 600.);
    frame(cx, window);
    let first = |cx: &mut TestAppContext| {
        cx.update_window(window, |_, window, _| window.try_find(("md-block", 0usize)).is_some())
            .expect("the window is open")
    };
    assert!(first(cx));
    let before = pane.read_with(cx, |pane, cx| pane.view.read(cx).stats());
    assert!(before.blocks_aligned);
    assert!(before.labelled_blocks > 0 && before.labelled_blocks < 40, "{before:?}");

    cx.update_window(window, |_, window, cx| {
        window.scroll("md-view", ScrollDelta::Pixels(point(px(0.), px(-3000.))), cx);
    })
    .expect("the window is open");
    cx.run_until_parked();
    frame(cx, window);
    assert!(!first(cx), "the first paragraph scrolled away");
    let after = pane.read_with(cx, |pane, cx| pane.view.read(cx).stats());
    assert!(after.labelled_blocks > 0 && after.labelled_blocks < 40, "{after:?}");
}

/// ⌘⇧U on the Mac, Ctrl+Shift+U elsewhere.
#[cfg(target_os = "macos")]
const TOGGLE_SOURCE: &str = "cmd-shift-u";
#[cfg(not(target_os = "macos"))]
const TOGGLE_SOURCE: &str = "ctrl-shift-u";

/// SM-4
#[gpui_kit::test]
fn the_keyboard_switches_to_the_source_and_back(cx: &mut TestAppContext) {
    init(cx);
    let (window, pane) = open(cx, GALLERY, gallery_options(), WIDTH, HEIGHT);
    cx.update_window(window, |_, window, cx| {
        let handle = pane.read(cx).view.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        window.press(TOGGLE_SOURCE, cx);
    })
    .expect("the window is open");
    cx.run_until_parked();
    assert_eq!(
        events(cx, &pane).last(),
        Some(&MarkdownEvent::ModeChanged(humata2_markdown::ViewMode::Source))
    );
    cx.update_window(window, |_, window, cx| window.press(TOGGLE_SOURCE, cx))
        .expect("the window is open");
    cx.run_until_parked();
    assert_eq!(
        events(cx, &pane).last(),
        Some(&MarkdownEvent::ModeChanged(humata2_markdown::ViewMode::Rendered))
    );
}
