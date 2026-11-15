//! Headless timings of the Markdown view, for a release build:
//!
//!     bazel test -c opt //apps/humata2/markdown/headless:timing --test_output=streamed
//!
//! They print how long parsing, streaming, diffing and drawing a long
//! document take under the headless test platform, which lays out with test
//! fonts and paints nothing, so they are not app frame times. They assert
//! nothing about time: a slow machine is not a failure.

use std::time::{Duration, Instant};

// `#[gpui_kit::test]` expands to paths under `gpui`.
use gpui_kit::gpui;
use gpui_kit::prelude::*;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, Entity, ScrollDelta, TestAppContext, point, px};
use humata2_markdown::{Document, MarkdownOptions, MarkdownView, diff};
use humata2_markdown_headless::{Pane, init, open};

/// About ten thousand lines: headings, paragraphs, lists, code and tables.
fn long_document() -> String {
    (1..=1000)
        .map(|n| {
            format!(
                "## Section {n}\n\nParagraph {n} with **strong** and `code`,\nwrapping onto a second line.\n\n- item one\n- item two\n\n```rust\nfn section_{n}() {{}}\n```\n\n| a | b |\n| - | - |\n| {n} | x |\n\n"
            )
        })
        .collect()
}

fn report(what: &str, runs: &[Duration]) {
    let mut sorted = runs.to_vec();
    sorted.sort();
    let median = sorted.get(sorted.len() / 2).copied().unwrap_or_default();
    let worst = sorted.last().copied().unwrap_or_default();
    println!("headless timing: {what}: median {median:?}, worst {worst:?} over {} runs", runs.len());
}

fn time(runs: usize, mut f: impl FnMut()) -> Vec<Duration> {
    (0..runs)
        .map(|_| {
            let start = Instant::now();
            f();
            start.elapsed()
        })
        .collect()
}

/// SM-3
#[::std::prelude::v1::test]
fn parsing_streaming_and_diffing_a_long_document() {
    let source = long_document();
    println!("headless timing: document of {} lines, {} bytes", source.lines().count(), source.len());
    report("parse whole", &time(10, || drop(Document::new(&source))));

    let mut streamed = Document::default();
    let mut appends = Vec::new();
    for piece in source.as_bytes().chunks(64) {
        let piece = std::str::from_utf8(piece).unwrap_or_default();
        let start = Instant::now();
        streamed.append(piece);
        appends.push(start.elapsed());
    }
    report("append 64 bytes", &appends);

    let old = Document::new(&source);
    let new = Document::new(&source.replace("Paragraph 500 ", "Paragraph five hundred "));
    report("diff one changed block", &time(10, || drop(diff(&old, &new))));
}

fn frame(cx: &mut TestAppContext, window: AnyWindowHandle) -> Duration {
    let start = Instant::now();
    cx.update_window(window, |_, window, cx| window.render_frame(cx))
        .expect("the window is open");
    start.elapsed()
}

fn view(cx: &mut TestAppContext, pane: &Entity<Pane>) -> Entity<MarkdownView> {
    pane.read_with(cx, |pane, _| pane.view.clone())
}

/// SM-3
#[gpui_kit::test]
fn drawing_scrolling_and_streaming_into_a_long_document(cx: &mut TestAppContext) {
    init(cx);
    let source = long_document();
    let start = Instant::now();
    let (window, pane) = open(cx, &source, MarkdownOptions::new("Long"), 900., 800.);
    let first = frame(cx, window);
    println!("headless timing: open and first frame: {:?} (first frame {first:?})", start.elapsed());

    let mut scrolls = Vec::new();
    for _ in 0..60 {
        cx.update_window(window, |_, window, cx| {
            window.scroll("md-view", ScrollDelta::Pixels(point(px(0.), px(-400.))), cx);
        })
        .expect("the window is open");
        cx.run_until_parked();
        scrolls.push(frame(cx, window));
    }
    report("frame after a 400 px scroll", &scrolls);

    let markdown = view(cx, &pane);
    let mut appends = Vec::new();
    for n in 0..200 {
        let start = Instant::now();
        markdown.update(cx, |view, cx| view.append(&format!("Streamed word {n} "), cx));
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| window.render_frame(cx))
            .expect("the window is open");
        appends.push(start.elapsed());
    }
    report("append and frame", &appends);
    let stats = markdown.read_with(cx, |view, _| view.stats());
    println!("headless timing: counters after streaming: {stats:?}");
}
