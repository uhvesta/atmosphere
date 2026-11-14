use std::sync::Arc;
use std::time::Instant;

use humata_core::{
    Body, Chat, ChatUpdate, Entry, Feed, Reading, ReplyChange, Row, RowChange, Streaming,
    ToolOutcome,
};

/// What Swift keeps, built only from updates.
#[derive(Default)]
struct Mirror {
    rows: Vec<Row>,
    reply: Option<(String, String)>,
    updates: usize,
}

impl Mirror {
    fn apply(&mut self, update: ChatUpdate) {
        if update.is_empty() {
            return;
        }
        self.updates += 1;
        if update.reset {
            self.rows.clear();
            self.reply = None;
        }
        for change in update.rows {
            match change {
                RowChange::Removed { id } => self.rows.retain(|row| row.id != id),
                RowChange::Inserted { index, row } => self.rows.insert(index as usize, row),
                RowChange::Updated { index, row } => self.rows[index as usize] = row,
            }
        }
        match update.reply {
            Some(ReplyChange::Started { id, text }) => self.reply = Some((id, text)),
            Some(ReplyChange::Appended { text }) => self.reply.as_mut().unwrap().1.push_str(&text),
            Some(ReplyChange::Ended) => self.reply = None,
            None => {}
        }
    }

    /// Checks it holds what `chat` shows.
    fn matches(&self, chat: &Chat) {
        let rows: Vec<Row> = chat.shown.entries.iter().map(Row::from).collect();
        assert_eq!(self.rows, rows);
        let reply = chat
            .shown
            .streaming
            .as_ref()
            .map(|streaming| (streaming.id.to_string(), streaming.text.clone()));
        assert_eq!(self.reply, reply);
    }
}

fn said(id: usize) -> Entry {
    Entry::new(
        id.to_string(),
        Body::Asked {
            text: format!("message {id}"),
            images: 0,
        },
    )
}

fn chat(session: &str, rows: usize) -> Chat {
    let mut chat = Chat {
        session: Some(session.into()),
        ..Chat::default()
    };
    for id in 0..rows {
        chat.shown.entries.push(said(id));
    }
    chat
}

#[test]
fn the_first_update_tells_everything_and_an_unchanged_chat_tells_nothing() {
    let chat = chat("s", 3);
    let mut feed = Feed::new();
    let first = feed.next(&chat);
    assert_eq!(first.rows.len(), 3);
    assert_eq!(first.status.as_ref().unwrap().session.as_deref(), Some("s"));
    let mut mirror = Mirror::default();
    mirror.apply(first);
    mirror.matches(&chat);

    let again = feed.next(&chat);
    assert!(again.is_empty());
    // Even the very first update of an empty chat tells its status.
    assert!(Feed::new().next(&Chat::default()).status.is_some());
}

#[test]
fn a_streaming_reply_sends_only_what_it_gained() {
    let mut chat = chat("s", 1);
    let mut feed = Feed::new();
    let mut mirror = Mirror::default();
    mirror.apply(feed.next(&chat));

    chat.shown.streaming = Some(Streaming {
        id: "r".into(),
        text: "Hel".into(),
    });
    let started = feed.next(&chat);
    assert_eq!(
        started.reply,
        Some(ReplyChange::Started {
            id: "r".into(),
            text: "Hel".into()
        })
    );
    assert!(started.rows.is_empty());
    mirror.apply(started);

    chat.shown
        .streaming
        .as_mut()
        .unwrap()
        .text
        .push_str("lo, wörld");
    let appended = feed.next(&chat);
    assert_eq!(
        appended.reply,
        Some(ReplyChange::Appended {
            text: "lo, wörld".into()
        })
    );
    mirror.apply(appended);
    mirror.matches(&chat);

    chat.shown.streaming = None;
    chat.shown.entries.push(Entry::new(
        "done",
        Body::Replied {
            text: "Hello, wörld".into(),
        },
    ));
    let ended = feed.next(&chat);
    assert_eq!(ended.reply, Some(ReplyChange::Ended));
    assert_eq!(ended.rows.len(), 1);
    mirror.apply(ended);
    mirror.matches(&chat);
}

#[test]
fn a_changed_row_alone_is_sent_and_another_session_starts_over() {
    let mut chat = chat("s", 5);
    chat.shown.entries.push(Entry::new(
        "tool",
        Body::Tool {
            call_id: "c".into(),
            tool: "shell".into(),
            arguments: "{}".into(),
            outcome: None,
        },
    ));
    let mut feed = Feed::new();
    let mut mirror = Mirror::default();
    mirror.apply(feed.next(&chat));

    chat.shown.entries.update(&"tool".into(), |entry| {
        entry.body = Arc::new(Body::Tool {
            call_id: "c".into(),
            tool: "shell".into(),
            arguments: "{}".into(),
            outcome: Some(ToolOutcome {
                success: true,
                output: "done".into(),
            }),
        });
    });
    let changed = feed.next(&chat);
    assert!(matches!(
        changed.rows.as_slice(),
        [RowChange::Updated { index: 5, .. }]
    ));
    assert!(changed.status.is_none());
    mirror.apply(changed);
    mirror.matches(&chat);

    let other = Chat {
        reading: Some(Reading::default()),
        ..self::chat("t", 2)
    };
    let switched = feed.next(&other);
    assert!(switched.reset);
    assert!(switched.status.as_ref().unwrap().reading.is_some());
    mirror.apply(switched);
    mirror.matches(&other);
}

#[test]
fn updates_alike_in_content_still_differ() {
    let mut chat = chat("s", 0);
    chat.shown.streaming = Some(Streaming {
        id: "r".into(),
        text: String::new(),
    });
    let mut feed = Feed::new();
    let first = feed.next(&chat);
    chat.shown.streaming.as_mut().unwrap().text.push('a');
    let one = feed.next(&chat);
    chat.shown.streaming.as_mut().unwrap().text.push('a');
    let two = feed.next(&chat);
    assert_eq!(one.reply, two.reply);
    assert_ne!(one, two);
    assert_ne!(first, one);
}

/// A long chat with a long reply streaming in small parts, as a model
/// sends them. Its timings print with HUMATA_PERF set.
#[test]
fn a_long_reply_streaming_into_a_long_chat_stays_mirrored() {
    const ROWS: usize = 5_000;
    const DELTAS: usize = 20_000;
    let mut chat = chat("s", ROWS);
    let mut feed = Feed::new();
    let mut mirror = Mirror::default();

    let start = Instant::now();
    mirror.apply(feed.next(&chat));
    let first = start.elapsed();

    chat.shown.streaming = Some(Streaming {
        id: "r".into(),
        text: String::new(),
    });
    let start = Instant::now();
    for n in 0..DELTAS {
        chat.shown
            .streaming
            .as_mut()
            .unwrap()
            .text
            .push_str(if n % 7 == 0 { " ünïcode " } else { "token " });
        mirror.apply(feed.next(&chat));
    }
    let streaming = start.elapsed();
    mirror.matches(&chat);
    assert_eq!(mirror.updates, DELTAS + 1);

    if std::env::var_os("HUMATA_PERF").is_some() {
        eprintln!(
            "feed: first update of {ROWS} rows {first:?}; {DELTAS} deltas {streaming:?}, {:?} each",
            streaming / DELTAS as u32
        );
    }
}
