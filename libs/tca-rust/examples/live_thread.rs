//! A chat thread that follows the server's stream of new messages. The
//! last message's sequence number is kept in state as the cursor. When the
//! stream drops, the thread waits a little, longer each time, and opens a
//! new stream from that cursor, so nothing is missed and nothing is shown
//! twice.
//!
//! Patterns: `Effect::stream` and `StreamEnd`; resuming from a cursor kept
//! in state; a retry with backoff on a `TestClock`; one cancel id for the
//! stream and the wait before it, so stopping stops either.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tca::{BoxFuture, Clock, Effect, Reducer, StreamEnd, TestClock, TestStore};
use tokio::sync::mpsc;

#[derive(Clone, Debug, PartialEq)]
struct Message {
    seq: u64,
    text: String,
}

/// The connection went away partway.
#[derive(Clone, Debug, PartialEq)]
struct Dropped;

type Messages = mpsc::UnboundedReceiver<Result<Message, Dropped>>;

trait Feed: Send + Sync {
    /// The messages after `after`, then new ones as they come.
    fn open(&self, after: Option<u64>) -> BoxFuture<Result<Messages, Dropped>>;
}

/// A server with a history. It can drop a stream once past a message,
/// keeps the stream open for messages sent later, and records the cursor
/// each stream was opened from.
#[derive(Default)]
struct FakeFeed {
    history: Mutex<Vec<Message>>,
    drop_after: Mutex<Option<u64>>,
    opened: Mutex<Vec<Option<u64>>>,
    live: Mutex<Option<mpsc::UnboundedSender<Result<Message, Dropped>>>>,
}

impl FakeFeed {
    /// A new message on the stream that's open; false if none is.
    fn post(&self, seq: u64, text: &str) -> bool {
        let message = Message {
            seq,
            text: text.into(),
        };
        self.history.lock().unwrap().push(message.clone());
        match &*self.live.lock().unwrap() {
            Some(live) => live.send(Ok(message)).is_ok(),
            None => false,
        }
    }
}

impl Feed for Arc<FakeFeed> {
    fn open(&self, after: Option<u64>) -> BoxFuture<Result<Messages, Dropped>> {
        let feed = self.clone();
        Box::pin(async move {
            feed.opened.lock().unwrap().push(after);
            let (send, messages) = mpsc::unbounded_channel();
            let due: Vec<Message> = feed
                .history
                .lock()
                .unwrap()
                .iter()
                .filter(|m| after.is_none_or(|after| m.seq > after))
                .cloned()
                .collect();
            for message in due {
                if feed.drop_after.lock().unwrap().take_if(|last| message.seq > *last).is_some() {
                    let _ = send.send(Err(Dropped));
                    return Ok(messages);
                }
                let _ = send.send(Ok(message));
            }
            *feed.live.lock().unwrap() = Some(send);
            Ok(messages)
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
enum Connection {
    #[default]
    Off,
    Following,
    /// Waiting to try again, after this many drops in a row.
    Retrying { drops: u32 },
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Thread {
    messages: Vec<Message>,
    /// The last message reduced. A new stream opens after it.
    cursor: Option<u64>,
    connection: Connection,
}

#[derive(Debug, PartialEq)]
enum ThreadAction {
    Follow,
    Received(Message),
    Ended(StreamEnd<Dropped>),
    Stop,
}

/// The stream and the wait before a retry share this id: each replaces
/// the other, and `Stop` cancels whichever is running.
const FEED: &str = "feed";

/// One second after the first drop, doubling, at most a minute.
fn backoff(drops: u32) -> Duration {
    Duration::from_secs(1u64 << drops.saturating_sub(1).min(6)).min(Duration::from_secs(60))
}

struct Following {
    feed: Arc<dyn Feed>,
    clock: Arc<dyn Clock>,
}

impl Reducer for Following {
    type State = Thread;
    type Action = ThreadAction;

    fn reduce(&self, thread: &mut Thread, action: ThreadAction) -> Effect<ThreadAction> {
        match action {
            ThreadAction::Follow => {
                // Drops in a row are counted until a message comes.
                if thread.connection == Connection::Off {
                    thread.connection = Connection::Following;
                }
                let open = self.feed.open(thread.cursor);
                Effect::stream(open, ThreadAction::Received, ThreadAction::Ended)
                    .cancellable(FEED, true)
            }
            ThreadAction::Received(message) => {
                thread.connection = Connection::Following;
                // A server may send again what it already sent; the cursor
                // says what's been seen.
                if thread.cursor.is_some_and(|cursor| message.seq <= cursor) {
                    return Effect::none();
                }
                thread.cursor = Some(message.seq);
                thread.messages.push(message);
                Effect::none()
            }
            ThreadAction::Ended(StreamEnd::Failed(Dropped)) => {
                let drops = match thread.connection {
                    Connection::Retrying { drops } => drops + 1,
                    _ => 1,
                };
                thread.connection = Connection::Retrying { drops };
                let clock = self.clock.clone();
                Effect::future(async move {
                    clock.sleep(backoff(drops)).await;
                    ThreadAction::Follow
                })
                .cancellable(FEED, true)
            }
            ThreadAction::Ended(StreamEnd::Finished) | ThreadAction::Stop => {
                thread.connection = Connection::Off;
                Effect::cancel(FEED)
            }
        }
    }
}

fn message(seq: u64, text: &str) -> Message {
    Message {
        seq,
        text: text.into(),
    }
}

fn thread(history: &[(u64, &str)]) -> (TestStore<Following>, TestClock, Arc<FakeFeed>) {
    let clock = TestClock::new();
    let feed = Arc::new(FakeFeed::default());
    *feed.history.lock().unwrap() = history.iter().map(|(seq, text)| message(*seq, text)).collect();
    let reducer = Following {
        feed: Arc::new(feed.clone()),
        clock: Arc::new(clock.clone()),
    };
    (TestStore::new(Thread::default(), reducer), clock, feed)
}

fn received(thread: &mut Thread, seq: u64, text: &str) {
    thread.connection = Connection::Following;
    thread.cursor = Some(seq);
    thread.messages.push(message(seq, text));
}

#[test]
fn a_dropped_stream_resumes_after_the_last_message_it_had() {
    let (mut store, clock, feed) = thread(&[(1, "hi"), (2, "are you in?"), (3, "yes")]);
    *feed.drop_after.lock().unwrap() = Some(2);
    store.send(ThreadAction::Follow, |s| s.connection = Connection::Following);
    store.receive(ThreadAction::Received(message(1, "hi")), |s| received(s, 1, "hi"));
    store.receive(ThreadAction::Received(message(2, "are you in?")), |s| {
        received(s, 2, "are you in?");
    });
    store.receive(ThreadAction::Ended(StreamEnd::Failed(Dropped)), |s| {
        s.connection = Connection::Retrying { drops: 1 };
    });
    clock.advance(backoff(1));
    store.receive(ThreadAction::Follow, |_| {});
    store.receive(ThreadAction::Received(message(3, "yes")), |s| received(s, 3, "yes"));
    assert_eq!(*feed.opened.lock().unwrap(), [None, Some(2)]);
    store.send(ThreadAction::Stop, |s| s.connection = Connection::Off);
}

#[test]
fn new_messages_arrive_while_the_stream_is_open_and_stop_ends_it() {
    let (mut store, _clock, feed) = thread(&[(1, "hi")]);
    store.send(ThreadAction::Follow, |s| s.connection = Connection::Following);
    store.receive(ThreadAction::Received(message(1, "hi")), |s| received(s, 1, "hi"));
    assert!(feed.post(2, "lunch?"));
    store.receive(ThreadAction::Received(message(2, "lunch?")), |s| {
        received(s, 2, "lunch?");
    });
    store.send(ThreadAction::Stop, |s| s.connection = Connection::Off);
    // The stream went with the effect, so the server can't reach it.
    assert!(!feed.post(3, "anyone?"));
    store.finish();
}

#[test]
fn a_message_sent_again_after_a_reconnect_is_shown_once() {
    let (mut store, _clock, feed) = thread(&[(1, "hi")]);
    store.send(ThreadAction::Follow, |s| s.connection = Connection::Following);
    store.receive(ThreadAction::Received(message(1, "hi")), |s| received(s, 1, "hi"));
    feed.post(1, "hi");
    store.receive(ThreadAction::Received(message(1, "hi")), |_| {});
    store.send(ThreadAction::Stop, |s| s.connection = Connection::Off);
}

#[test]
fn stopping_while_waiting_to_retry_cancels_the_retry() {
    let (mut store, clock, feed) = thread(&[(1, "hi"), (2, "there")]);
    *feed.drop_after.lock().unwrap() = Some(1);
    store.send(ThreadAction::Follow, |s| s.connection = Connection::Following);
    store.receive(ThreadAction::Received(message(1, "hi")), |s| received(s, 1, "hi"));
    store.receive(ThreadAction::Ended(StreamEnd::Failed(Dropped)), |s| {
        s.connection = Connection::Retrying { drops: 1 };
    });
    store.send(ThreadAction::Stop, |s| s.connection = Connection::Off);
    clock.advance(Duration::from_secs(60));
    store.finish();
    assert_eq!(*feed.opened.lock().unwrap(), [None]);
}

#[test]
fn the_wait_doubles_with_each_drop_in_a_row_up_to_a_minute() {
    let waits: Vec<u64> = (1..=9).map(|drops| backoff(drops).as_secs()).collect();
    assert_eq!(waits, [1, 2, 4, 8, 16, 32, 60, 60, 60]);
}
