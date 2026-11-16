//! LD-23, LD-24, KP-1, KP-2: searching every session's conversation from
//! the app, and opening a hit's turn.
use std::sync::Arc;

use humata2_clients::DaemonError;
use humata2_clients::history::{FakeHistory, History, IndexedSession, SearchHit, TurnRole};
use humata2_clients::model::{ChannelId, MachineId, SessionId};
use humata2_find::{Find, FindAction as A, FindDelegate, FindState};
use tca::TestStore;
use tokio::runtime::Builder;

fn block_on<T>(future: impl Future<Output = T>) -> T {
    match Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime.block_on(future),
        Err(error) => panic!("a runtime starts: {error}"),
    }
}

fn about(session: &str) -> IndexedSession {
    IndexedSession {
        session: SessionId::new(session),
        channel: ChannelId::new("billing"),
        channel_name: "Billing".to_owned(),
        title: format!("Session {session}"),
        machine: MachineId::new("mac-studio"),
        machine_name: "Mac Studio".to_owned(),
        agent: "claude".to_owned(),
        as_of: "2026-11-15T09:00:00Z".to_owned(),
    }
}

/// "ledger" is in two turns, "index" in one, "csv" in another.
fn history() -> FakeHistory {
    FakeHistory::new()
        .with_turns(
            &about("s-1"),
            &[
                (1, TurnRole::User, "Why is the ledger rollup slow?"),
                (2, TurnRole::User, "Add an index for the rollup."),
            ],
        )
        .with_turns(
            &about("s-2"),
            &[(4, TurnRole::User, "Export the ledger as CSV.")],
        )
}

/// What the host answers `query` with, asked outside the store.
fn hits(history: &FakeHistory, query: &str) -> Vec<SearchHit> {
    match block_on(history.search(query, "")) {
        Ok((hits, _)) => hits,
        Err(error) => panic!("searching {query:?} works: {error}"),
    }
}

fn store(history: &FakeHistory) -> TestStore<Find> {
    TestStore::new(FindState::default(), Find::new(Arc::new(history.clone())))
}

fn loaded(generation: u64, hits: &[SearchHit]) -> A {
    A::Loaded {
        generation,
        result: Ok((hits.to_vec(), String::new())),
    }
}

fn offline() -> DaemonError {
    DaemonError::Offline {
        machine: MachineId::new("mac-studio"),
    }
}

#[test]
fn a_query_loads_its_hits() {
    let history = history();
    let ledger = hits(&history, "ledger");
    assert_eq!(ledger.len(), 2);
    let mut store = store(&history);

    store.send(A::Query("ledger".to_owned()), |s| {
        s.query = "ledger".to_owned();
        s.generation = 1;
        s.loading = true;
    });
    store.receive(loaded(1, &ledger), |s| {
        s.hits = Arc::new(ledger.clone());
        s.loading = false;
    });
    store.finish();
}

#[test]
fn a_query_under_two_characters_clears_the_hits_and_asks_nothing() {
    let history = history();
    let ledger = hits(&history, "ledger");
    let mut store = store(&history);
    store.send(A::Query("ledger".to_owned()), |s| {
        s.query = "ledger".to_owned();
        s.generation = 1;
        s.loading = true;
    });
    store.receive(loaded(1, &ledger), |s| {
        s.hits = Arc::new(ledger.clone());
        s.loading = false;
    });
    let asked = history.calls().len();

    store.send(A::Query("l".to_owned()), |s| {
        s.query = "l".to_owned();
        s.generation = 2;
        s.hits = Arc::new(Vec::new());
    });
    store.send(A::Query(String::new()), |s| {
        s.query = String::new();
        s.generation = 3;
    });
    store.finish();
    assert_eq!(history.calls().len(), asked);
}

#[test]
fn spaces_do_not_count_towards_the_two_characters() {
    let history = history();
    let mut store = store(&history);
    store.send(A::Query("  l  ".to_owned()), |s| {
        s.query = "  l  ".to_owned();
        s.generation = 1;
    });
    store.finish();
    assert!(history.calls().is_empty());
}

#[test]
fn hits_stay_while_the_next_query_is_on_its_way() {
    let history = history();
    let ledger = hits(&history, "ledger");
    let index = hits(&history, "index");
    assert_eq!(index.len(), 1);
    let mut store = store(&history);
    store.send(A::Query("ledger".to_owned()), |s| {
        s.query = "ledger".to_owned();
        s.generation = 1;
        s.loading = true;
    });
    store.receive(loaded(1, &ledger), |s| {
        s.hits = Arc::new(ledger.clone());
        s.loading = false;
    });

    store.send(A::Query("index".to_owned()), |s| {
        s.query = "index".to_owned();
        s.generation = 2;
        s.loading = true;
    });
    store.receive(loaded(2, &index), |s| {
        s.hits = Arc::new(index.clone());
        s.loading = false;
    });
    store.finish();
}

#[test]
fn a_slow_reply_to_an_older_query_never_replaces_newer_hits() {
    let history = history();
    let index = hits(&history, "index");
    let mut store = store(&history);

    // The first query's reply is held back; the second's comes at once.
    history.hold();
    store.send(A::Query("ledger".to_owned()), |s| {
        s.query = "ledger".to_owned();
        s.generation = 1;
        s.loading = true;
    });
    store.send(A::Query("index".to_owned()), |s| {
        s.query = "index".to_owned();
        s.generation = 2;
    });
    store.receive(loaded(2, &index), |s| {
        s.hits = Arc::new(index.clone());
        s.loading = false;
    });

    // Now the old reply arrives, if the search still waited for it.
    history.release();
    store.skip_received();
    assert_eq!(store.state().query, "index");
    assert_eq!(*store.state().hits, index);
    assert!(!store.state().loading);
    assert!(store.state().error.is_none());
    store.finish();
}

#[test]
fn a_reply_for_an_older_generation_is_dropped() {
    let history = history();
    let ledger = hits(&history, "ledger");
    let index = hits(&history, "index");
    let mut store = store(&history);
    store.send(A::Query("ledger".to_owned()), |s| {
        s.query = "ledger".to_owned();
        s.generation = 1;
        s.loading = true;
    });
    store.receive(loaded(1, &ledger), |s| {
        s.hits = Arc::new(ledger.clone());
        s.loading = false;
    });

    // Neither hits nor a failure from an older query change what shows.
    store.send(loaded(0, &index), |_| {});
    store.send(
        A::Loaded {
            generation: 0,
            result: Err(offline()),
        },
        |_| {},
    );
    store.finish();
}

#[test]
fn a_short_query_drops_the_reply_still_on_its_way() {
    let history = history();
    let mut store = store(&history);
    history.hold();
    store.send(A::Query("ledger".to_owned()), |s| {
        s.query = "ledger".to_owned();
        s.generation = 1;
        s.loading = true;
    });
    store.send(A::Query("l".to_owned()), |s| {
        s.query = "l".to_owned();
        s.generation = 2;
        s.loading = false;
    });

    history.release();
    store.skip_received();
    assert!(store.state().hits.is_empty());
    assert!(!store.state().loading);
    store.finish();
}

#[test]
fn a_failed_search_shows_and_the_next_query_clears_it() {
    let history = history();
    let ledger = hits(&history, "ledger");
    let index = hits(&history, "index");
    let mut store = store(&history);
    store.send(A::Query("ledger".to_owned()), |s| {
        s.query = "ledger".to_owned();
        s.generation = 1;
        s.loading = true;
    });
    store.receive(loaded(1, &ledger), |s| {
        s.hits = Arc::new(ledger.clone());
        s.loading = false;
    });

    history.fail_next(offline());
    store.send(A::Query("index".to_owned()), |s| {
        s.query = "index".to_owned();
        s.generation = 2;
        s.loading = true;
    });
    store.receive(
        A::Loaded {
            generation: 2,
            result: Err(offline()),
        },
        |s| {
            s.loading = false;
            s.error = Some(offline().to_string());
            // The other query's hits don't stay under this one.
            s.hits = Arc::new(Vec::new());
        },
    );

    store.send(A::Query("index".to_owned()), |s| {
        s.generation = 3;
        s.loading = true;
        s.error = None;
    });
    store.receive(loaded(3, &index), |s| {
        s.hits = Arc::new(index.clone());
        s.loading = false;
    });
    store.finish();
}

#[test]
fn a_short_query_clears_the_error_too() {
    let history = history();
    let mut store = store(&history);
    history.fail_next(offline());
    store.send(A::Query("ledger".to_owned()), |s| {
        s.query = "ledger".to_owned();
        s.generation = 1;
        s.loading = true;
    });
    store.receive(
        A::Loaded {
            generation: 1,
            result: Err(offline()),
        },
        |s| {
            s.loading = false;
            s.error = Some(offline().to_string());
        },
    );
    store.send(A::Query("l".to_owned()), |s| {
        s.query = "l".to_owned();
        s.generation = 2;
        s.error = None;
    });
    store.finish();
}

#[test]
fn picking_a_hit_opens_its_turn() {
    let history = history();
    let csv = hits(&history, "csv");
    let mut store = store(&history);
    store.send(A::Query("csv".to_owned()), |s| {
        s.query = "csv".to_owned();
        s.generation = 1;
        s.loading = true;
    });
    store.receive(loaded(1, &csv), |s| {
        s.hits = Arc::new(csv.clone());
        s.loading = false;
    });

    store.send(A::Open(0), |_| {});
    store.receive(
        A::Delegate(FindDelegate::OpenTurn {
            session: SessionId::new("s-2"),
            turn: 4,
            seq: csv[0].seq,
        }),
        |_| {},
    );
    store.finish();
}

#[test]
fn picking_past_the_last_hit_does_nothing() {
    let history = history();
    let csv = hits(&history, "csv");
    let mut store = store(&history);
    store.send(A::Query("csv".to_owned()), |s| {
        s.query = "csv".to_owned();
        s.generation = 1;
        s.loading = true;
    });
    store.receive(loaded(1, &csv), |s| {
        s.hits = Arc::new(csv.clone());
        s.loading = false;
    });
    store.send(A::Open(1), |_| {});
    store.finish();
}
