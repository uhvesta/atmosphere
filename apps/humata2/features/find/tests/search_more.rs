//! LD-23: a search's hits come a page at a time; asking for more adds the
//! next page after the hits already shown.
use std::sync::Arc;

use humata2_clients::DaemonError;
use humata2_clients::history::{Call, FakeHistory, History, IndexedSession, SearchHit, TurnRole};
use humata2_clients::model::{ChannelId, MachineId, SessionId};
use humata2_find::{Find, FindAction as A, FindState};
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

/// "ledger" is in five turns, two to a page.
fn history() -> FakeHistory {
    FakeHistory::new().with_page_size(2).with_turns(
        &about("s-1"),
        &[
            (1, TurnRole::User, "The ledger rollup is slow."),
            (2, TurnRole::User, "Index the ledger table."),
            (3, TurnRole::User, "Export the ledger as CSV."),
            (4, TurnRole::User, "Check the ledger totals."),
            (5, TurnRole::User, "Ship the ledger fix."),
        ],
    )
}

/// What the host answers for `query` at `token`, asked outside the store.
fn page(history: &FakeHistory, query: &str, token: &str) -> (Vec<SearchHit>, String) {
    match block_on(history.search(query, token)) {
        Ok(page) => page,
        Err(error) => panic!("searching {query:?} at {token:?} works: {error}"),
    }
}

fn store(history: &FakeHistory) -> TestStore<Find> {
    TestStore::new(FindState::default(), Find::new(Arc::new(history.clone())))
}

/// Types "ledger" and takes its first page.
fn first_page(store: &mut TestStore<Find>, first: &(Vec<SearchHit>, String)) {
    store.send(A::Query("ledger".to_owned()), |s| {
        s.query = "ledger".to_owned();
        s.generation = 1;
        s.loading = true;
    });
    store.receive(
        A::Loaded { generation: 1, result: Ok(first.clone()) },
        |s| {
            s.hits = Arc::new(first.0.clone());
            s.next_page = first.1.clone();
            s.loading = false;
        },
    );
}

#[test]
fn the_first_page_keeps_the_next_pages_token() {
    let history = history();
    let first = page(&history, "ledger", "");
    assert_eq!(first.0.len(), 2);
    assert!(!first.1.is_empty());
    let mut store = store(&history);
    first_page(&mut store, &first);
    store.finish();
}

#[test]
fn more_adds_the_next_page_after_the_hits_shown() {
    let history = history();
    let first = page(&history, "ledger", "");
    let second = page(&history, "ledger", &first.1);
    let third = page(&history, "ledger", &second.1);
    assert!(third.1.is_empty(), "five hits fit in three pages of two");
    let mut store = store(&history);
    first_page(&mut store, &first);

    store.send(A::More, |s| s.loading = true);
    store.receive(
        A::MoreLoaded { generation: 1, result: Ok(second.clone()) },
        |s| {
            s.hits = Arc::new([first.0.clone(), second.0.clone()].concat());
            s.next_page = second.1.clone();
            s.loading = false;
        },
    );
    store.send(A::More, |s| s.loading = true);
    store.receive(
        A::MoreLoaded { generation: 1, result: Ok(third.clone()) },
        |s| {
            s.hits = Arc::new([first.0.clone(), second.0.clone(), third.0.clone()].concat());
            s.next_page = String::new();
            s.loading = false;
        },
    );
    assert_eq!(store.state().hits.len(), 5);
    store.finish();
}

#[test]
fn more_after_the_last_page_asks_nothing() {
    let history = FakeHistory::new().with_turns(
        &about("s-1"),
        &[(1, TurnRole::User, "The ledger rollup is slow.")],
    );
    let first = page(&history, "ledger", "");
    assert!(first.1.is_empty());
    let mut store = store(&history);
    first_page(&mut store, &first);
    let asked = history.calls().len();

    store.send(A::More, |_| {});
    store.finish();
    assert_eq!(history.calls().len(), asked);
}

#[test]
fn more_while_a_page_is_on_its_way_asks_nothing_more() {
    let history = history();
    let first = page(&history, "ledger", "");
    let second = page(&history, "ledger", &first.1);
    let mut store = store(&history);
    first_page(&mut store, &first);
    let asked = history.calls().len();

    history.hold();
    store.send(A::More, |s| s.loading = true);
    store.send(A::More, |_| {});
    history.release();
    store.receive(
        A::MoreLoaded { generation: 1, result: Ok(second.clone()) },
        |s| {
            s.hits = Arc::new([first.0.clone(), second.0.clone()].concat());
            s.next_page = second.1.clone();
            s.loading = false;
        },
    );
    store.finish();
    let more: Vec<Call> = history.calls().into_iter().skip(asked).collect();
    assert_eq!(
        more,
        vec![Call::Search { query: "ledger".to_owned(), page_token: first.1.clone() }],
    );
}

#[test]
fn a_new_query_drops_the_next_page_of_the_old_one() {
    let history = history();
    let first = page(&history, "ledger", "");
    let mut store = store(&history);
    first_page(&mut store, &first);

    history.hold();
    store.send(A::More, |s| s.loading = true);
    store.send(A::Query("l".to_owned()), |s| {
        s.query = "l".to_owned();
        s.generation = 2;
        s.hits = Arc::new(Vec::new());
        s.next_page = String::new();
        s.loading = false;
    });
    // The page asked for under generation 1 still answers, and is dropped.
    history.release();
    store.skip_received();
    assert!(store.state().next_page.is_empty());
    assert!(!store.state().loading);
    assert!(store.state().hits.is_empty());
    store.finish();
}

#[test]
fn a_failed_page_keeps_the_hits_shown_and_says_why() {
    let history = history();
    let first = page(&history, "ledger", "");
    let mut store = store(&history);
    first_page(&mut store, &first);
    let error = DaemonError::Offline { machine: MachineId::new("mac-studio") };

    store.send(
        A::MoreLoaded { generation: 1, result: Err(error.clone()) },
        |s| {
            s.error = Some(error.to_string());
            s.loading = false;
        },
    );
    assert_eq!(*store.state().hits, first.0);
    assert_eq!(store.state().next_page, first.1, "the same page can be asked for again");
    store.finish();
}
