//! A recording can be rewound to a mark: the steps pushed after it go, the
//! daemon calls stay, and the trees are what they were at the mark.

mod support;

use humata2_report::{Event, Holds, Limits, Rpc};
use support::{Driver, add_button, draft_field};

fn call(at_ms: u64) -> Rpc {
    Rpc {
        at_ms,
        machine: "studio".to_owned(),
        method: "humata.v1.SessionService/Get".to_owned(),
        status: "ok".to_owned(),
        duration_ms: 2,
        request: None,
        response: None,
        holds: Holds::Plain,
    }
}

fn interactions(events: &[Event]) -> usize {
    events.iter().filter(|event| matches!(event, Event::Interaction(_))).count()
}

fn calls(events: &[Event]) -> usize {
    events.iter().filter(|event| matches!(event, Event::Rpc(_))).count()
}

#[test]
fn rewinding_drops_the_steps_after_the_mark_but_keeps_the_calls() {
    let mut driver = Driver::start();
    driver.type_into(draft_field(), "milk");
    let at_mark = driver.history.len() - 1;
    let mark = driver.recording.mark();
    driver.click(add_button());
    driver.recording.rpc(call(150));
    driver.type_into(draft_field(), "eggs");
    driver.recording.rpc(call(50));

    assert!(driver.recording.rewind(mark));

    let expected = driver.history[at_mark].clone();
    let captured = driver.stop();
    assert_eq!(interactions(captured.events()), 1);
    assert_eq!(calls(captured.events()), 2);
    assert_eq!(*captured.end(), expected);
}

#[test]
fn rewinding_after_the_recording_folded_past_the_mark_does_nothing() {
    let limits = Limits {
        max_events: 6,
        max_bytes: usize::MAX,
    };
    let mut driver = Driver::with_limits(limits);
    driver.type_into(draft_field(), "milk");
    let mark = driver.recording.mark();
    for _ in 0..10 {
        driver.type_into(draft_field(), "x");
    }
    let before = driver.recording.dropped();

    assert!(!driver.recording.rewind(mark));

    assert_eq!(driver.recording.dropped(), before);
    assert!(interactions(driver.stop().events()) > 1);
}

#[test]
fn a_mark_from_another_recording_does_nothing() {
    let first = Driver::start();
    let mark = first.recording.mark();
    let mut second = Driver::start();
    second.click(add_button());

    assert!(!second.recording.rewind(mark));

    assert_eq!(interactions(second.stop().events()), 1);
}
