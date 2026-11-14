use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use humata_core::{
    Body, Chat, ChatAction, ChatFeature, ChatItem, ChatItems, Daemon, DaemonError, Entry,
    EventKind, PermissionAnswer, Queued, Reading, Session, Streaming, ToolOutcome, Transcript,
};
use tca::{BoxFuture, IdentifiedVec, TestClock, TestStore};
use tokio::sync::mpsc;

const RETRY: Duration = Duration::from_secs(1);

type Items = mpsc::UnboundedSender<Result<ChatItem, DaemonError>>;

/// Hands each watch the next channel a test made, and answers sends as
/// scripted; once those run out, a send never answers.
#[derive(Default)]
struct FakeDaemon {
    watches: Mutex<VecDeque<ChatItems>>,
    watched: Mutex<Vec<String>>,
    sends: Mutex<VecDeque<Result<(), DaemonError>>>,
    sent: Mutex<Vec<(String, String)>>,
    aborted: Mutex<Vec<String>>,
}

impl FakeDaemon {
    /// The channel the next watch reads, to tell it items.
    fn expect_watch(&self) -> Items {
        let (items, received) = mpsc::unbounded_channel();
        self.watches.lock().unwrap().push_back(received);
        items
    }

    fn answer_sends(&self, answers: impl IntoIterator<Item = Result<(), DaemonError>>) {
        self.sends.lock().unwrap().extend(answers);
    }
}

impl Daemon for FakeDaemon {
    fn sessions(&self) -> BoxFuture<Result<Vec<Session>, DaemonError>> {
        Box::pin(std::future::pending())
    }

    fn watch(&self, session: String) -> ChatItems {
        self.watched.lock().unwrap().push(session);
        self.watches
            .lock()
            .unwrap()
            .pop_front()
            .expect("the test made a channel for each watch")
    }

    fn send_message(
        &self,
        session: String,
        text: String,
        _client_id: String,
    ) -> BoxFuture<Result<(), DaemonError>> {
        self.sent.lock().unwrap().push((session, text));
        let answer = self.sends.lock().unwrap().pop_front();
        Box::pin(async move {
            match answer {
                Some(answer) => answer,
                None => std::future::pending().await,
            }
        })
    }

    fn abort(&self, session: String) -> BoxFuture<Result<(), DaemonError>> {
        self.aborted.lock().unwrap().push(session);
        Box::pin(async { Ok(()) })
    }
}

fn store(daemon: &Arc<FakeDaemon>, clock: &TestClock) -> TestStore<ChatFeature> {
    TestStore::new(
        Chat::default(),
        ChatFeature::new(daemon.clone(), Arc::new(clock.clone()), RETRY),
    )
}

fn event(id: &str, kind: EventKind) -> ChatItem {
    ChatItem::Event {
        id: id.into(),
        kind,
    }
}

fn asked(text: &str) -> EventKind {
    EventKind::Asked {
        text: text.into(),
        images: 0,
    }
}

fn replied(text: &str) -> EventKind {
    EventKind::Replied { text: text.into() }
}

fn delta(text: &str) -> EventKind {
    EventKind::Delta { text: text.into() }
}

fn rows(entries: impl IntoIterator<Item = Entry>) -> IdentifiedVec<Entry> {
    entries.into_iter().collect()
}

fn asked_row(id: &str, text: &str) -> Entry {
    Entry::new(
        id,
        Body::Asked {
            text: text.into(),
            images: 0,
        },
    )
}

fn replied_row(id: &str, text: &str) -> Entry {
    Entry::new(id, Body::Replied { text: text.into() })
}

/// Opens `session` with `items` as its watch, through the history being
/// read with nothing in it.
fn open(store: &mut TestStore<ChatFeature>, daemon: &FakeDaemon, session: &str) -> Items {
    let items = daemon.expect_watch();
    store.send(ChatAction::Open(session.into()), |chat| {
        *chat = Chat {
            session: Some(session.into()),
            reading: Some(Reading::default()),
            ..Chat::default()
        };
    });
    items.send(Ok(ChatItem::HistoryLoaded)).unwrap();
    store.receive(ChatAction::Watched(ChatItem::HistoryLoaded), |chat| {
        chat.reading = None;
    });
    items
}

/// Tells the watch `item` and checks the chat changes as `expect` says.
fn tell(
    store: &mut TestStore<ChatFeature>,
    items: &Items,
    item: ChatItem,
    expect: impl FnOnce(&mut Chat),
) {
    items.send(Ok(item.clone())).unwrap();
    store.receive(ChatAction::Watched(item), expect);
}

/// What the history being read holds so far.
fn read(chat: &mut Chat) -> &mut Transcript {
    &mut chat.reading.as_mut().unwrap().transcript
}

#[test]
fn the_history_shows_once_read_then_each_event_as_it_happens() {
    let daemon = Arc::new(FakeDaemon::default());
    let clock = TestClock::new();
    let mut store = store(&daemon, &clock);
    let items = daemon.expect_watch();

    store.send(ChatAction::Open("s".into()), |chat| {
        chat.session = Some("s".into());
        chat.reading = Some(Reading::default());
    });
    tell(
        &mut store,
        &items,
        ChatItem::HistoryLoading { read: 1, total: 2 },
        |chat| {
            let reading = chat.reading.as_mut().unwrap();
            reading.read = 1;
            reading.total = 2;
        },
    );
    tell(&mut store, &items, event("1", asked("hi")), |chat| {
        read(chat).entries = rows([asked_row("1", "hi")]);
    });
    tell(&mut store, &items, event("2", replied("hello")), |chat| {
        read(chat).entries = rows([asked_row("1", "hi"), replied_row("2", "hello")]);
    });
    tell(&mut store, &items, ChatItem::HistoryLoaded, |chat| {
        chat.reading = None;
        chat.shown.entries = rows([asked_row("1", "hi"), replied_row("2", "hello")]);
    });

    tell(&mut store, &items, event("3", asked("more")), |chat| {
        chat.shown.entries.push(asked_row("3", "more"));
    });
    tell(
        &mut store,
        &items,
        event("4", EventKind::TurnStarted),
        |chat| {
            chat.shown.working = true;
        },
    );
    tell(&mut store, &items, event("5", delta("Sure")), |chat| {
        chat.shown.streaming = Some(Streaming {
            id: "5".into(),
            text: "Sure".into(),
        });
    });
    tell(&mut store, &items, event("6", delta(", here")), |chat| {
        chat.shown
            .streaming
            .as_mut()
            .unwrap()
            .text
            .push_str(", here");
    });
    tell(
        &mut store,
        &items,
        event("7", replied("Sure, here.")),
        |chat| {
            chat.shown.streaming = None;
            chat.shown.entries.push(replied_row("7", "Sure, here."));
        },
    );
    tell(&mut store, &items, event("8", EventKind::Idle), |chat| {
        chat.shown.working = false;
    });

    store.send(ChatAction::Close, |chat| *chat = Chat::default());
    store.finish();
    assert_eq!(*daemon.watched.lock().unwrap(), ["s"]);
}

#[test]
fn a_tool_and_a_permission_get_their_answers_on_their_own_rows() {
    let daemon = Arc::new(FakeDaemon::default());
    let clock = TestClock::new();
    let mut store = store(&daemon, &clock);
    let items = open(&mut store, &daemon, "s");

    let started = |call: &str| EventKind::ToolStarted {
        call_id: call.into(),
        tool: "shell".into(),
        arguments: "{}".into(),
    };
    let running = |call: &str| Body::Tool {
        call_id: call.into(),
        tool: "shell".into(),
        arguments: "{}".into(),
        outcome: None,
    };
    tell(&mut store, &items, event("1", started("a")), |chat| {
        chat.shown.entries.push(Entry::new("1", running("a")));
    });
    tell(&mut store, &items, event("2", started("b")), |chat| {
        chat.shown.entries.push(Entry::new("2", running("b")));
    });
    let completed = EventKind::ToolCompleted {
        call_id: "a".into(),
        success: true,
        output: "ok".into(),
    };
    tell(&mut store, &items, event("3", completed), |chat| {
        chat.shown.entries.update(&"1".into(), |entry| {
            entry.body = Arc::new(Body::Tool {
                call_id: "a".into(),
                tool: "shell".into(),
                arguments: "{}".into(),
                outcome: Some(ToolOutcome {
                    success: true,
                    output: "ok".into(),
                }),
            });
        });
    });
    let requested = EventKind::PermissionRequested {
        request_id: "p".into(),
        summary: "rm -rf build".into(),
    };
    tell(&mut store, &items, event("4", requested), |chat| {
        chat.shown.entries.push(Entry::new(
            "4",
            Body::Permission {
                request_id: "p".into(),
                summary: "rm -rf build".into(),
                answer: None,
            },
        ));
    });
    let answered = EventKind::PermissionCompleted {
        request_id: "p".into(),
        approved: false,
        automatic: false,
    };
    tell(&mut store, &items, event("5", answered), |chat| {
        chat.shown.entries.update(&"4".into(), |entry| {
            entry.body = Arc::new(Body::Permission {
                request_id: "p".into(),
                summary: "rm -rf build".into(),
                answer: Some(PermissionAnswer {
                    approved: false,
                    automatic: false,
                }),
            });
        });
    });

    store.send(ChatAction::Close, |chat| *chat = Chat::default());
    store.finish();
}

#[test]
fn stopping_keeps_what_streamed_and_says_it_was_stopped() {
    let daemon = Arc::new(FakeDaemon::default());
    let clock = TestClock::new();
    let mut store = store(&daemon, &clock);
    let items = open(&mut store, &daemon, "s");

    // Nothing runs, so there's nothing to stop.
    store.send(ChatAction::Stop, |_| {});
    tell(
        &mut store,
        &items,
        event("1", EventKind::TurnStarted),
        |chat| {
            chat.shown.working = true;
        },
    );
    tell(&mut store, &items, event("2", delta("Half a")), |chat| {
        chat.shown.streaming = Some(Streaming {
            id: "2".into(),
            text: "Half a".into(),
        });
    });
    store.send(ChatAction::Stop, |chat| chat.stopping = true);
    // A second Stop while stopping asks nothing more.
    store.send(ChatAction::Stop, |_| {});
    let ended = EventKind::TurnEnded { aborted: true };
    tell(&mut store, &items, event("3", ended), |chat| {
        chat.stopping = false;
        chat.shown.working = false;
        chat.shown.streaming = None;
        chat.shown.entries = rows([replied_row("2", "Half a"), Entry::new("3", Body::Stopped)]);
    });

    store.send(ChatAction::Close, |chat| *chat = Chat::default());
    store.finish();
    assert_eq!(*daemon.aborted.lock().unwrap(), ["s"]);
}

#[test]
fn a_message_that_couldnt_be_sent_is_offered_back_with_why() {
    let daemon = Arc::new(FakeDaemon::default());
    let clock = TestClock::new();
    let mut store = store(&daemon, &clock);
    let _items = open(&mut store, &daemon, "s");
    daemon.answer_sends([Err(DaemonError::Unexpected), Ok(())]);

    // Blank messages aren't sent.
    store.send(ChatAction::Send("  \n".into()), |_| {});
    store.send(ChatAction::Send("first".into()), |chat| chat.sending = true);
    let error = DaemonError::Unexpected.to_string();
    store.receive(
        ChatAction::SendFailed {
            text: "first".into(),
            error: error.clone(),
        },
        |chat| {
            chat.sending = false;
            chat.error = Some(error.clone());
            chat.unsent = Some("first".into());
        },
    );
    store.send(ChatAction::TookUnsent, |chat| chat.unsent = None);
    store.send(ChatAction::Send("first again".into()), |chat| {
        chat.sending = true;
    });
    store.receive(ChatAction::Sent, |chat| {
        chat.sending = false;
        chat.error = None;
    });

    store.send(ChatAction::Close, |chat| *chat = Chat::default());
    store.finish();
    assert_eq!(
        *daemon.sent.lock().unwrap(),
        [
            ("s".to_string(), "first".to_string()),
            ("s".to_string(), "first again".to_string())
        ]
    );
}

#[test]
fn a_watch_that_ends_is_followed_again_and_reads_the_history_afresh() {
    let daemon = Arc::new(FakeDaemon::default());
    let clock = TestClock::new();
    let mut store = store(&daemon, &clock);
    let items = open(&mut store, &daemon, "s");
    tell(&mut store, &items, event("1", asked("hi")), |chat| {
        chat.shown.entries = rows([asked_row("1", "hi")]);
    });

    let again = daemon.expect_watch();
    items.send(Err(DaemonError::Unexpected)).unwrap();
    let error = DaemonError::Unexpected.to_string();
    store.receive(ChatAction::WatchEnded(Some(error.clone())), |chat| {
        chat.error = Some(error.clone());
    });
    clock.advance(RETRY);
    store.receive(ChatAction::Rewatch, |chat| {
        chat.reading = Some(Reading::default());
    });
    // What showed stays until the history is read again.
    tell(&mut store, &again, event("1", asked("hi")), |chat| {
        chat.error = None;
        read(chat).entries = rows([asked_row("1", "hi")]);
    });
    tell(&mut store, &again, ChatItem::HistoryLoaded, |chat| {
        chat.reading = None;
    });

    store.send(ChatAction::Close, |chat| *chat = Chat::default());
    store.finish();
}

#[test]
fn a_rewind_reads_the_chat_again_and_queued_messages_come_and_go() {
    let daemon = Arc::new(FakeDaemon::default());
    let clock = TestClock::new();
    let mut store = store(&daemon, &clock);
    let items = open(&mut store, &daemon, "s");

    let queued = EventKind::Queued {
        queue_id: "q".into(),
        text: "next".into(),
    };
    tell(&mut store, &items, event("1", queued), |chat| {
        chat.shown.queued = vec![Queued {
            id: "q".into(),
            text: "next".into(),
        }];
    });
    let dequeued = EventKind::Dequeued {
        queue_id: "q".into(),
    };
    tell(&mut store, &items, event("2", dequeued), |chat| {
        chat.shown.queued.clear();
    });

    let again = daemon.expect_watch();
    tell(
        &mut store,
        &items,
        event("3", EventKind::Rewritten),
        |chat| {
            chat.reading = Some(Reading::default());
        },
    );
    tell(&mut store, &again, ChatItem::HistoryLoaded, |chat| {
        chat.reading = None;
    });

    store.send(ChatAction::Close, |chat| *chat = Chat::default());
    store.finish();
    assert_eq!(*daemon.watched.lock().unwrap(), ["s", "s"]);
}

#[test]
fn opening_another_session_follows_it_instead() {
    let daemon = Arc::new(FakeDaemon::default());
    let clock = TestClock::new();
    let mut store = store(&daemon, &clock);
    let first = open(&mut store, &daemon, "a");
    tell(&mut store, &first, event("1", asked("hi")), |chat| {
        chat.shown.entries = rows([asked_row("1", "hi")]);
    });
    // Opening the one that shows changes nothing.
    store.send(ChatAction::Open("a".into()), |_| {});
    let _second = open(&mut store, &daemon, "b");

    store.send(ChatAction::Close, |chat| *chat = Chat::default());
    store.finish();
    // The first watch was let go.
    assert!(first.is_closed());
    assert_eq!(*daemon.watched.lock().unwrap(), ["a", "b"]);
}
