use std::sync::{Arc, Mutex};

use tca::{
    Effect, PresentationAction, PresentationState, Reducer, StreamEnd, TestStore, if_presented,
};
use tokio::sync::mpsc;

#[derive(Clone, Debug, PartialEq)]
struct Event {
    seq: u64,
}

/// The connection dropped.
#[derive(Clone, Debug, PartialEq)]
struct Dropped;

type Events = mpsc::UnboundedReceiver<Result<Event, Dropped>>;

/// A server's event stream: it sends the events after the cursor it's
/// opened from, and may drop partway or stay open for more.
#[derive(Clone, Default)]
struct FakeFeed {
    inner: Arc<Mutex<FeedInner>>,
}

#[derive(Default)]
struct FeedInner {
    events: Vec<u64>,
    /// Drops the next stream once past this sequence.
    drop_after: Option<u64>,
    /// Keeps streams open once their events are sent, for `push`.
    hold_open: bool,
    /// The cursor each stream was opened from.
    opened: Vec<Option<u64>>,
    live: Option<mpsc::UnboundedSender<Result<Event, Dropped>>>,
}

impl FakeFeed {
    fn new(events: impl IntoIterator<Item = u64>) -> Self {
        let feed = Self::default();
        feed.inner.lock().unwrap().events = events.into_iter().collect();
        feed
    }

    fn dropping_after(self, seq: u64) -> Self {
        self.inner.lock().unwrap().drop_after = Some(seq);
        self
    }

    fn held_open(self) -> Self {
        self.inner.lock().unwrap().hold_open = true;
        self
    }

    fn open(&self, after: Option<u64>) -> Result<Events, Dropped> {
        let mut inner = self.inner.lock().unwrap();
        inner.opened.push(after);
        let (send, events) = mpsc::unbounded_channel();
        let due: Vec<u64> = inner
            .events
            .iter()
            .copied()
            .filter(|seq| after.is_none_or(|after| *seq > after))
            .collect();
        for seq in due {
            if inner.drop_after.is_some_and(|last| seq > last) {
                inner.drop_after = None;
                send.send(Err(Dropped)).unwrap();
                return Ok(events);
            }
            send.send(Ok(Event { seq })).unwrap();
        }
        if inner.hold_open {
            inner.live = Some(send);
        }
        Ok(events)
    }

    /// Sends one more event on the stream held open; false if whoever
    /// followed it has stopped.
    fn push(&self, seq: u64) -> bool {
        let inner = self.inner.lock().unwrap();
        inner
            .live
            .as_ref()
            .is_some_and(|live| live.send(Ok(Event { seq })).is_ok())
    }

    fn opened(&self) -> Vec<Option<u64>> {
        self.inner.lock().unwrap().opened.clone()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Feed {
    /// The last sequence received, which the next stream resumes from.
    cursor: Option<u64>,
    received: Vec<u64>,
    connected: bool,
}

#[derive(Debug, PartialEq)]
enum FeedAction {
    Connect,
    Stop,
    Event(u64),
    Ended(StreamEnd<Dropped>),
}

struct Following {
    feed: FakeFeed,
}

impl Reducer for Following {
    type State = Feed;
    type Action = FeedAction;

    fn reduce(&self, state: &mut Feed, action: FeedAction) -> Effect<FeedAction> {
        match action {
            FeedAction::Connect => {
                state.connected = true;
                let feed = self.feed.clone();
                let after = state.cursor;
                Effect::stream(
                    async move { feed.open(after) },
                    |event: Event| FeedAction::Event(event.seq),
                    FeedAction::Ended,
                )
                .cancellable("feed", true)
            }
            FeedAction::Stop => {
                state.connected = false;
                Effect::cancel("feed")
            }
            FeedAction::Event(seq) => {
                state.received.push(seq);
                state.cursor = Some(seq);
                Effect::none()
            }
            FeedAction::Ended(_) => {
                state.connected = false;
                Effect::none()
            }
        }
    }
}

fn received(store: &mut TestStore<Following>, seqs: impl IntoIterator<Item = u64>) {
    for seq in seqs {
        store.receive(FeedAction::Event(seq), |s| {
            s.received.push(seq);
            s.cursor = Some(seq);
        });
    }
}

#[test]
fn a_stream_that_drops_resumes_from_the_last_sequence_received() {
    let feed = FakeFeed::new(1..=8).dropping_after(4);
    let mut store = TestStore::new(Feed::default(), Following { feed: feed.clone() });
    store.send(FeedAction::Connect, |s| s.connected = true);
    received(&mut store, 1..=4);
    store.receive(FeedAction::Ended(StreamEnd::Failed(Dropped)), |s| {
        s.connected = false
    });

    store.send(FeedAction::Connect, |s| s.connected = true);
    received(&mut store, 5..=8);
    store.receive(FeedAction::Ended(StreamEnd::Finished), |s| {
        s.connected = false
    });
    assert_eq!(feed.opened(), [None, Some(4)]);
    assert_eq!(store.state().received, (1..=8).collect::<Vec<_>>());
}

#[test]
fn a_stream_that_cant_be_opened_ends_failed() {
    struct Refusing;
    impl Reducer for Refusing {
        type State = Feed;
        type Action = FeedAction;
        fn reduce(&self, state: &mut Feed, action: FeedAction) -> Effect<FeedAction> {
            match action {
                FeedAction::Connect => Effect::stream(
                    async { Err::<Events, _>(Dropped) },
                    |event: Event| FeedAction::Event(event.seq),
                    FeedAction::Ended,
                ),
                FeedAction::Ended(_) => {
                    state.connected = false;
                    Effect::none()
                }
                _ => Effect::none(),
            }
        }
    }
    let mut store = TestStore::new(Feed::default(), Refusing);
    store.send(FeedAction::Connect, |_| {});
    store.receive(FeedAction::Ended(StreamEnd::Failed(Dropped)), |_| {});
}

#[test]
fn a_stopped_stream_sends_nothing_more_and_is_let_go() {
    let feed = FakeFeed::new(1..=2).held_open();
    let mut store = TestStore::new(Feed::default(), Following { feed: feed.clone() });
    store.send(FeedAction::Connect, |s| s.connected = true);
    received(&mut store, 1..=2);
    assert!(feed.push(3));
    received(&mut store, [3]);
    store.send(FeedAction::Stop, |s| s.connected = false);
    // The stream was dropped with its effect.
    assert!(!feed.push(4));
    store.finish();
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Screen {
    sheet: PresentationState<Feed>,
}

#[derive(Debug, PartialEq)]
enum ScreenAction {
    Open,
    Sheet(PresentationAction<FeedAction>),
}

struct Screening {
    following: Following,
}

impl Reducer for Screening {
    type State = Screen;
    type Action = ScreenAction;

    fn reduce(&self, screen: &mut Screen, action: ScreenAction) -> Effect<ScreenAction> {
        match action {
            ScreenAction::Open => {
                screen.sheet.present(Feed::default());
                Effect::none()
            }
            ScreenAction::Sheet(action) => {
                if_presented(&self.following, &mut screen.sheet, action).map(ScreenAction::Sheet)
            }
        }
    }
}

#[test]
fn a_stream_is_cancelled_with_the_feature_that_follows_it() {
    let feed = FakeFeed::new([1]).held_open();
    let mut store = TestStore::new(
        Screen::default(),
        Screening {
            following: Following { feed: feed.clone() },
        },
    );
    store.send(ScreenAction::Open, |s| s.sheet.present(Feed::default()));
    store.send(
        ScreenAction::Sheet(PresentationAction::Presented(FeedAction::Connect)),
        |s| {
            if let Some(feed) = s.sheet.get_mut() {
                feed.connected = true;
            }
        },
    );
    store.receive(
        ScreenAction::Sheet(PresentationAction::Presented(FeedAction::Event(1))),
        |s| {
            if let Some(feed) = s.sheet.get_mut() {
                feed.received = vec![1];
                feed.cursor = Some(1);
            }
        },
    );
    store.send(ScreenAction::Sheet(PresentationAction::Dismiss), |s| {
        s.sheet = PresentationState::none()
    });
    assert!(!feed.push(2));
    store.finish();
}
