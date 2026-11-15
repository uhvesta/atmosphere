//! A recording that reaches its limit keeps its newest events and its end,
//! folds the oldest into its start, and says how many it folded.

mod support;

use std::fs;

use humata2_report::{Captured, Form, Limits, Secrets, Trees, read_events, scrub, write};
use support::{Driver, add_button, draft_field};

/// A recording of adding items, and what the app went through.
struct Ran {
    captured: Captured,
    /// Every set of trees the app had, oldest first.
    history: Vec<Trees>,
    /// When the last item was added.
    last_at: u64,
}

/// Adds `count` items, each typed and then added.
fn adds(limits: Limits, count: usize) -> Ran {
    let mut driver = Driver::with_limits(limits);
    for n in 0..count {
        driver.type_into(draft_field(), &format!("item {n}"));
        driver.click(add_button());
    }
    let history = driver.history.clone();
    let last_at = driver.now;
    Ran {
        captured: driver.stop(),
        history,
        last_at,
    }
}

fn by_count() -> Limits {
    Limits {
        max_events: 25,
        max_bytes: usize::MAX,
    }
}

#[test]
fn a_full_recording_keeps_its_newest_events_and_its_end() {
    let ran = adds(by_count(), 30);

    let captured = &ran.captured;
    assert!(captured.events().len() <= 25);
    assert!(captured.dropped() > 0);
    assert_eq!(Some(captured.end()), ran.history.last());
    let last = captured.events().last().expect("events are kept");
    assert!(last.at_ms() >= ran.last_at, "the newest events are the ones kept");
}

#[test]
fn a_full_recordings_start_is_a_moment_the_app_was_in() {
    let ran = adds(by_count(), 30);

    let start = ran.captured.start();
    assert_ne!(start, &ran.history[0], "the start moved forward");
    assert!(ran.history.iter().any(|trees| trees.state == start.state));
    assert!(ran.history.iter().any(|trees| trees.view == start.view));
    assert!(ran.history.iter().any(|trees| trees.layout == start.layout));
}

#[test]
fn a_limit_smaller_than_any_event_keeps_only_the_end() {
    let limits = Limits {
        max_events: usize::MAX,
        max_bytes: 1,
    };

    let ran = adds(limits, 3);

    assert!(ran.captured.events().is_empty());
    assert_eq!(Some(ran.captured.start()), ran.history.last());
    assert_eq!(Some(ran.captured.end()), ran.history.last());
}

#[test]
fn a_full_recording_saves_reads_back_and_says_it_was_full() {
    let ran = adds(by_count(), 30);
    let dropped = ran.captured.dropped();
    let scrubbed = scrub(ran.captured, Form::default(), &Secrets::new()).expect("scrubs");
    let reports = tempfile::tempdir().expect("a temporary folder");

    let dir = write(reports.path(), &scrubbed).expect("writes");

    let text = fs::read_to_string(dir.join("events.jsonl")).expect("events.jsonl");
    let read = read_events(&text).expect("reads back");
    assert_eq!(read.dropped(), dropped);
    assert_eq!(read.flow_script().dropped, dropped);
}
