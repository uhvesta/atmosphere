//! A recording keeps its events in the order they happened, even when a
//! daemon's call, error or log line reaches the app after later events.

mod support;

use std::fs;

use humata2_report::{
    DaemonError, Event, Form, Holds, LogLine, Rpc, Secrets, read_events, scrub, write,
};
use support::{Driver, add_button, draft_field};

fn times(events: &[Event]) -> Vec<u64> {
    events.iter().map(Event::at_ms).collect()
}

fn is_sorted(times: &[u64]) -> bool {
    times.windows(2).all(|pair| pair[0] <= pair[1])
}

#[test]
fn late_daemon_events_take_their_place_by_time() {
    let mut driver = Driver::start();
    driver.type_into(draft_field(), "milk");
    driver.click(add_button());
    // These arrive after the click at 200 was recorded, but happened
    // before it.
    driver.recording.rpc(Rpc {
        at_ms: 150,
        machine: "studio".to_owned(),
        method: "humata.v1.SessionService/List".to_owned(),
        status: "ok".to_owned(),
        duration_ms: 3,
        request: None,
        response: None,
        holds: Holds::Plain,
    });
    driver.recording.daemon_error(DaemonError {
        at_ms: 120,
        machine: "studio".to_owned(),
        method: None,
        code: "unavailable".to_owned(),
        message: "the event stream broke".to_owned(),
    });
    driver.recording.log(LogLine {
        at_ms: 50,
        machine: "studio".to_owned(),
        line: "listening".to_owned(),
    });

    let captured = driver.stop();

    let at = times(captured.events());
    assert!(is_sorted(&at), "{at:?}");
    let from_daemons = captured
        .events()
        .iter()
        .filter(|event| matches!(event, Event::Rpc(_) | Event::DaemonError(_) | Event::Log(_)))
        .count();
    assert_eq!(from_daemons, 3);
}

#[test]
fn events_read_back_in_the_order_they_happened() {
    let mut driver = Driver::start();
    driver.type_into(draft_field(), "milk");
    driver.recording.log(LogLine {
        at_ms: 10,
        machine: "studio".to_owned(),
        line: "listening".to_owned(),
    });
    driver.click(add_button());
    let scrubbed = scrub(driver.stop(), Form::default(), &Secrets::new()).expect("scrubs");
    let reports = tempfile::tempdir().expect("a temporary folder");
    let dir = write(reports.path(), &scrubbed).expect("writes");

    let text = fs::read_to_string(dir.join("events.jsonl")).expect("events.jsonl");
    let read = read_events(&text).expect("reads back");

    assert!(matches!(read.events().first(), Some(Event::Log(line)) if line.at_ms == 10));
    assert!(is_sorted(&times(read.events())));
}
