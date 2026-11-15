//! A stream that produces faster than the reducer reduces waits for it,
//! and what the UI sends is never held back or lost.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tca::{EFFECT_QUEUE, Effect, EventStream, LocalStore, Reducer, Store, StreamEnd};
use tokio::task::LocalSet;

/// A source with `total` items ready at once, counting how many were read.
struct Ready {
    next: u64,
    total: u64,
    read: Arc<AtomicU64>,
}

impl EventStream for Ready {
    type Item = u64;
    type Error = ();

    fn next(&mut self) -> Pin<Box<dyn Future<Output = Option<Result<u64, ()>>> + Send + '_>> {
        let item = (self.next < self.total).then(|| {
            self.next += 1;
            self.read.fetch_add(1, Ordering::SeqCst);
            Ok(self.next)
        });
        Box::pin(async move { item })
    }
}

#[derive(Debug, Default)]
struct Feed {
    items: u64,
    taps: u64,
    ended: bool,
}

enum Action {
    Follow { total: u64, read: Arc<AtomicU64> },
    Item,
    Ended,
    Tap,
}

struct Following;

impl Reducer for Following {
    type State = Feed;
    type Action = Action;

    fn reduce(&self, state: &mut Feed, action: Action) -> Effect<Action> {
        match action {
            Action::Follow { total, read } => Effect::stream(
                async move {
                    Ok::<_, ()>(Ready {
                        next: 0,
                        total,
                        read,
                    })
                },
                |_| Action::Item,
                |end: StreamEnd<()>| {
                    assert_eq!(end, StreamEnd::Finished);
                    Action::Ended
                },
            ),
            Action::Item => {
                state.items += 1;
                Effect::none()
            }
            Action::Ended => {
                state.ended = true;
                Effect::none()
            }
            Action::Tap => {
                state.taps += 1;
                Effect::none()
            }
        }
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap()
}

const TOTAL: u64 = EFFECT_QUEUE as u64 * 10;

#[test]
fn a_stream_reads_no_further_ahead_of_the_reducer_than_the_queue_holds() {
    let runtime = runtime();
    let local = LocalSet::new();
    local.block_on(&runtime, async {
        let store = LocalStore::new(Feed::default(), Following).unwrap();
        let read = Arc::new(AtomicU64::new(0));
        store.send(Action::Follow {
            total: TOTAL,
            read: read.clone(),
        });
        // Nothing reduces what the stream sends yet, so it can only fill
        // the queue.
        for _ in 0..TOTAL {
            tokio::task::yield_now().await;
        }
        let ahead = read.load(Ordering::SeqCst);
        assert!(ahead > 0, "the stream never started");
        assert!(
            ahead <= EFFECT_QUEUE as u64 + 1,
            "the stream read {ahead} items with nothing reduced"
        );

        // Once something reduces, the stream goes on to the end.
        tokio::task::spawn_local(store.run());
        tokio::time::timeout(Duration::from_secs(10), async {
            while !store.with_state(|s| s.ended) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(store.with_state(|s| s.items), TOTAL);
        assert_eq!(read.load(Ordering::SeqCst), TOTAL);
    });
}

#[test]
fn what_the_ui_sends_is_never_held_back_by_a_full_queue() {
    let runtime = runtime();
    let local = LocalSet::new();
    local.block_on(&runtime, async {
        let store = LocalStore::new(Feed::default(), Following).unwrap();
        let read = Arc::new(AtomicU64::new(0));
        store.send(Action::Follow {
            total: TOTAL,
            read: read.clone(),
        });
        for _ in 0..TOTAL {
            tokio::task::yield_now().await;
        }
        let taps = EFFECT_QUEUE as u64 * 3;
        for _ in 0..taps {
            store.send(Action::Tap);
        }
        assert_eq!(store.with_state(|s| s.taps), taps);
    });
}

#[test]
fn a_store_on_its_own_thread_reduces_every_item_of_a_fast_stream_and_every_send() {
    let store = Store::new(Feed::default(), Following).unwrap();
    let read = Arc::new(AtomicU64::new(0));
    store.send(Action::Follow {
        total: TOTAL,
        read: read.clone(),
    });
    let taps = EFFECT_QUEUE as u64 * 3;
    for _ in 0..taps {
        store.send(Action::Tap);
    }
    let (tell, told) = std::sync::mpsc::channel();
    let _observing = store.observe(
        |s| (s.ended, s.items, s.taps),
        move |now| {
            let _ = tell.send(*now);
        },
    );
    loop {
        let (ended, items, seen_taps) = told.recv_timeout(Duration::from_secs(10)).unwrap();
        if ended && seen_taps == taps {
            assert_eq!(items, TOTAL);
            break;
        }
    }
}
