//! `events.jsonl` reads back as the report that was written, and a file
//! that was cut short, edited or written by a newer Humata is refused.

mod support;

use std::fs;

use humata2_report::{
    DaemonError, Form, Gesture, Holds, LogLine, Pasted, ReadError, Rpc, Secrets, read_events,
    scrub, snapshot, write,
};
use support::{Driver, add_button, context, draft_field, item, terminal, token_field};

/// A recording with every kind of event in it.
fn everything() -> humata2_report::Captured {
    let mut driver = Driver::start();
    driver.type_into(draft_field(), "milk");
    driver.key(draft_field(), "enter");
    driver.type_into(draft_field(), "eggs");
    driver.click(add_button());
    driver.act(Gesture::Scroll {
        target: item("milk"),
        dx: 0,
        dy: -40,
    });
    driver.act(Gesture::Drag {
        from: item("milk"),
        to: item("eggs"),
    });
    driver.act(Gesture::Paste {
        target: Some(draft_field()),
        content: Pasted::Text("bread".to_owned()),
    });
    driver.act(Gesture::Paste {
        target: Some(terminal()),
        content: Pasted::Image {
            path: "/home/me/.humata/paste/s-1/1.png".to_owned(),
            bytes: 2048,
        },
    });
    driver.click(token_field());
    driver.recording.rpc(Rpc {
        at_ms: driver.now + 5,
        machine: "studio".to_owned(),
        method: "humata.v1.SessionService/List".to_owned(),
        status: "ok".to_owned(),
        duration_ms: 3,
        request: Some("{}".to_owned()),
        response: Some("{\"sessions\":[\"s-1\"]}".to_owned()),
        holds: Holds::Plain,
    });
    driver.recording.daemon_error(DaemonError {
        at_ms: driver.now + 6,
        machine: "studio".to_owned(),
        method: Some("humata.v1.TerminalService/Attach".to_owned()),
        code: "unavailable".to_owned(),
        message: "the holder for s-1 exited".to_owned(),
    });
    driver.recording.log(LogLine {
        at_ms: driver.now + 7,
        machine: "studio".to_owned(),
        line: "restarting holder for s-1".to_owned(),
    });
    driver.stop()
}

#[test]
fn a_written_recording_reads_back_as_it_was() {
    let reports = tempfile::tempdir().expect("a temporary folder");
    let scrubbed = scrub(everything(), Form::default(), &Secrets::new()).expect("scrubs");

    let dir = write(reports.path(), &scrubbed).expect("writes");
    let text = fs::read_to_string(dir.join("events.jsonl")).expect("events.jsonl is there");

    assert_eq!(&read_events(&text).expect("reads back"), scrubbed.captured());
}

#[test]
fn a_written_snapshot_reads_back_as_it_was() {
    let reports = tempfile::tempdir().expect("a temporary folder");
    let mut toy = support::Toy::default();
    toy.play(&humata2_report::FlowAction::Type("milk".to_owned()));
    let scrubbed = scrub(snapshot(context(), toy.trees()), Form::default(), &Secrets::new())
        .expect("scrubs");

    let dir = write(reports.path(), &scrubbed).expect("writes");
    let text = fs::read_to_string(dir.join("events.jsonl")).expect("events.jsonl is there");

    let read = read_events(&text).expect("reads back");
    assert_eq!(&read, scrubbed.captured());
    assert_eq!(read.end(), &toy.trees());
}

fn written() -> String {
    let scrubbed = scrub(everything(), Form::default(), &Secrets::new()).expect("scrubs");
    let reports = tempfile::tempdir().expect("a temporary folder");
    let dir = write(reports.path(), &scrubbed).expect("writes");
    fs::read_to_string(dir.join("events.jsonl")).expect("events.jsonl is there")
}

#[test]
fn a_file_cut_short_is_refused() {
    let text = written();
    let lines: Vec<&str> = text.lines().collect();
    let cut = lines[..lines.len() - 1].join("\n");

    assert!(matches!(read_events(&cut), Err(ReadError::Missing { .. })));
}

#[test]
fn a_line_that_isnt_an_event_is_refused() {
    let text = written();
    let mut lines: Vec<&str> = text.lines().collect();
    lines.insert(3, "{\"kind\":\"interaction\",\"gesture\":\"wave\"}");

    assert!(matches!(
        read_events(&lines.join("\n")),
        Err(ReadError::Json { line: 4, .. })
    ));
}

#[test]
fn events_after_the_end_are_refused() {
    let text = written();
    let lines: Vec<&str> = text.lines().collect();
    let event = lines[2];
    let moved = format!("{text}{event}\n");

    assert!(matches!(read_events(&moved), Err(ReadError::OutOfPlace { .. })));
}

#[test]
fn a_report_from_a_newer_humata_is_refused() {
    let text = written();
    let newer = text.replacen("\"schema\":1", "\"schema\":999", 1);
    assert_ne!(newer, text, "the context line names its schema");

    assert!(matches!(read_events(&newer), Err(ReadError::Schema { found: 999, .. })));
}
