//! A review of 300 files and 20,000 lines across three repos: the pane
//! mounts only what's on screen, reuses diff lines it already prepared, and
//! decodes no image that isn't shown. Counters are checked; times are only
//! printed, labelled headless, since a headless frame isn't a frame on a
//! display. For release timings:
//!
//!   bazel run -c opt //apps/humata2/review:large -- --nocapture

use std::time::{Duration, Instant};

use gpui_kit::prelude::*;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, Entity, ScrollDelta, TestAppContext, point, px};
use humata2_review_harness::{Fakes, large_review, open_pane};
use humata2_review_view::model::{AgentState, SessionId};
use humata2_review_view::{MountStats, ReviewPane};

const WIDTH: f32 = 1280.;
const HEIGHT: f32 = 900.;
const FILES: usize = 300;
const LINES: usize = 20_000;
/// Rows a 900-pixel body can show, with room for rows a few pixels tall.
const ON_SCREEN: usize = 120;

fn stats(cx: &mut TestAppContext, pane: &Entity<ReviewPane>) -> MountStats {
    pane.read_with(cx, |pane, _| pane.stats())
}

fn frame(cx: &mut TestAppContext, window: AnyWindowHandle, scroll: Option<f32>) -> Duration {
    let start = Instant::now();
    cx.update_window(window, |_, window, cx| {
        if let Some(by) = scroll {
            window.scroll("body", ScrollDelta::Pixels(point(px(0.), px(-by))), cx);
        }
        window.render_frame(cx);
    })
    .unwrap_or_else(|error| panic!("the window is open: {error}"));
    cx.run_until_parked();
    start.elapsed()
}

#[gpui_kit::test]
fn a_large_review_mounts_only_what_is_on_screen(cx: &mut TestAppContext) {
    let fakes = Fakes::new();
    let (window, pane) = open_pane(cx, &fakes, WIDTH, HEIGHT);
    let session = SessionId::new("large");
    let review = large_review(FILES, LINES);

    let start = Instant::now();
    pane.update(cx, |pane, cx| {
        pane.show_session(session.clone(), cx);
        pane.set_review(session.clone(), review, cx);
        pane.set_agent_state(session.clone(), AgentState::Idle, cx);
    });
    cx.run_until_parked();
    let load = start.elapsed();
    let first = frame(cx, window, None);

    let opened = stats(cx, &pane);
    assert!(opened.rows >= LINES * 9 / 10, "every diff line is a row: {opened:?}");
    assert!(opened.distinct > 0 && opened.distinct <= ON_SCREEN, "only the first screen mounts: {opened:?}");

    // Draw the same screen again: its lines are reused, none prepared again.
    frame(cx, window, None);
    let again = stats(cx, &pane);
    let list_per_frame = again.list_mounted - opened.list_mounted;
    assert!(list_per_frame <= ON_SCREEN, "the file list mounts a screen of its {FILES} rows: {again:?}");
    assert_eq!(again.lines_built, opened.lines_built, "nothing new was prepared: {again:?}");
    assert!(again.lines_reused > opened.lines_reused, "the lines on screen were reused: {again:?}");

    // Scroll through 200 screens' worth, one small step at a time.
    let mut scrolls = Vec::new();
    for _ in 0..200 {
        scrolls.push(frame(cx, window, Some(HEIGHT)));
    }
    let scrolled = stats(cx, &pane);
    let mounted_per_frame = (scrolled.mounted - again.mounted) / scrolls.len();
    assert!(mounted_per_frame <= ON_SCREEN, "each frame mounts a screen of rows: {scrolled:?}");
    assert!(scrolled.distinct < scrolled.rows, "rows never on screen never mounted: {scrolled:?}");
    assert!(
        scrolled.lines_built <= scrolled.distinct,
        "a line is prepared once while it's kept: {scrolled:?}"
    );
    let images_shown = scrolled.images.decoded;
    assert!(images_shown <= 2 * (FILES / 25 + 1), "only images scrolled past are decoded: {scrolled:?}");

    scrolls.sort();
    let median = scrolls[scrolls.len() / 2];
    let worst = scrolls[scrolls.len() - 1];
    println!(
        "headless, {FILES} files, {LINES} lines: load {load:?}, first frame {first:?}, \
         scroll frame median {median:?}, worst {worst:?}; {scrolled:?}"
    );
}
