//! OpenCode's event stream, as v2.3.0 sent it (testdata/): frames split
//! across reads come out whole, heartbeats are skipped, and each event
//! says what it means for the turn and the Inbox.
use humata2_opencode::{Asked, Event, Frames};

/// A recorded fixture by name (testdata/, from OpenCode v2.3.0).
fn fixture(name: &str) -> String {
    match name {
        "events-session-created.sse" => include_str!("../testdata/events-session-created.sse"),
        "permission-asked.json" => include_str!("../testdata/permission-asked.json"),
        "permission-replied.json" => include_str!("../testdata/permission-replied.json"),
        "execution-started.json" => include_str!("../testdata/execution-started.json"),
        "execution-succeeded.json" => include_str!("../testdata/execution-succeeded.json"),
        "execution-interrupted.json" => include_str!("../testdata/execution-interrupted.json"),
        "tool-called.json" => include_str!("../testdata/tool-called.json"),
        other => panic!("no fixture {other}"),
    }
    .to_owned()
}

/// A recorded JSON event as one SSE frame.
fn frame(name: &str) -> String {
    let event: serde_json::Value = serde_json::from_str(&fixture(name)).expect("fixture is JSON");
    format!("data: {event}\n\n")
}

fn parsed(name: &str) -> Event {
    let mut frames = Frames::default();
    let data = frames.push(frame(name).as_bytes());
    assert_eq!(data.len(), 1, "{name} is one frame");
    Event::parse(&data[0]).expect("a recorded event parses")
}

#[test]
fn a_recorded_stream_gives_its_frames_and_skips_heartbeats() {
    let mut frames = Frames::default();
    let stream = format!(": heartbeat\n\n{}: heartbeat\n\n", fixture("events-session-created.sse"));
    let data = frames.push(stream.as_bytes());
    let events: Vec<Event> = data
        .iter()
        .map(|data| Event::parse(data).expect("a recorded event parses"))
        .collect();
    assert_eq!(events.first(), Some(&Event::Connected));
    assert!(events.len() >= 2);
}

#[test]
fn a_frame_split_across_reads_comes_out_once_whole() {
    let whole = frame("permission-asked.json");
    let (first, second) = whole.as_bytes().split_at(whole.len() / 2);
    let mut frames = Frames::default();
    assert!(frames.push(first).is_empty());
    let data = frames.push(second);
    assert_eq!(data.len(), 1);
    assert!(matches!(Event::parse(&data[0]), Ok(Event::Asked(_))));
}

#[test]
fn crlf_line_ends_are_frames_too() {
    let mut frames = Frames::default();
    let data = frames.push(b"data: {\"type\":\"server.connected\",\"data\":{}}\r\n\r\n");
    assert_eq!(data.len(), 1);
    assert_eq!(Event::parse(&data[0]).ok(), Some(Event::Connected));
}

#[test]
fn execution_events_start_and_end_a_sessions_run() {
    let session = "ses_e1c537029ffejTdBfVrnGEG0jm".to_owned();
    assert_eq!(parsed("execution-succeeded.json"), Event::Ended { session });
    assert!(matches!(parsed("execution-started.json"), Event::Started { .. }));
    assert!(matches!(parsed("execution-interrupted.json"), Event::Ended { .. }));
}

#[test]
fn a_permission_ask_names_its_session_action_and_what_it_runs() {
    let Event::Asked(asked) = parsed("permission-asked.json") else {
        panic!("permission.asked is an ask");
    };
    assert_eq!(
        asked,
        Asked {
            id: "per_1e3ac9e95001Sgm3zNE4P6lF7q".to_owned(),
            session: "ses_e1c537029ffejTdBfVrnGEG0jm".to_owned(),
            action: "shell".to_owned(),
            resources: vec!["echo hi".to_owned()],
        }
    );
}

#[test]
fn a_reply_names_the_ask_it_settled() {
    assert_eq!(
        parsed("permission-replied.json"),
        Event::Replied {
            session: "ses_e1c537029ffejTdBfVrnGEG0jm".to_owned(),
            request: "per_1e3ac9e95001Sgm3zNE4P6lF7q".to_owned(),
        }
    );
}

#[test]
fn events_this_link_doesnt_use_are_other_and_garbage_is_an_error() {
    assert_eq!(parsed("tool-called.json"), Event::Other);
    assert!(Event::parse("not json").is_err());
    // An ask with no id can't be answered.
    assert!(Event::parse(r#"{"type":"permission.asked","data":{"sessionID":"s"}}"#).is_err());
}
