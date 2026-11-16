//! A search box over a catalogue. It waits for typing to pause before it
//! searches, and a newer query replaces a search still in flight, so the
//! results shown are always for the last query. Clearing the box stops
//! both. Time comes from a `TestClock` in tests, so nothing sleeps.
//!
//! Patterns: `debounce` with a `TestClock`; cancelling in flight for a
//! newer query; a fake client that records what it was asked.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tca::{BoxFuture, Clock, Effect, Reducer, TestClock, TestStore};

/// How long typing must pause before a search starts.
const PAUSE: Duration = Duration::from_millis(300);
/// How long the fake catalogue takes to answer.
const LATENCY: Duration = Duration::from_secs(1);

/// Two ids, because they guard two different things: the pause before a
/// search, which each keystroke restarts, and the search itself, which
/// only a newer search replaces. With one id, each keystroke would also
/// drop a search about to answer, and the list would stay stale until the
/// person stopped typing.
const TYPING: &str = "typing";
const SEARCH: &str = "search";

trait Catalogue: Send + Sync {
    fn search(&self, query: &str) -> BoxFuture<Vec<String>>;
}

/// Answers after `LATENCY` on the test clock and keeps every query asked.
struct FakeCatalogue {
    titles: Vec<String>,
    clock: TestClock,
    asked: Mutex<Vec<String>>,
}

impl Catalogue for FakeCatalogue {
    fn search(&self, query: &str) -> BoxFuture<Vec<String>> {
        self.asked.lock().unwrap().push(query.to_owned());
        let query = query.to_lowercase();
        let found: Vec<String> = self
            .titles
            .iter()
            .filter(|title| title.to_lowercase().contains(&query))
            .cloned()
            .collect();
        let wait = self.clock.sleep(LATENCY);
        Box::pin(async move {
            wait.await;
            found
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Search {
    query: String,
    /// The query a search is under way for.
    searching: Option<String>,
    results: Vec<String>,
}

#[derive(Debug, PartialEq)]
enum SearchAction {
    Typed(String),
    /// Typing paused on this query.
    Paused(String),
    Found(Vec<String>),
}

struct Searching {
    catalogue: Arc<dyn Catalogue>,
    clock: Arc<dyn Clock>,
}

impl Reducer for Searching {
    type State = Search;
    type Action = SearchAction;

    fn reduce(&self, search: &mut Search, action: SearchAction) -> Effect<SearchAction> {
        match action {
            SearchAction::Typed(query) => {
                search.query = query.clone();
                if query.trim().is_empty() {
                    search.searching = None;
                    search.results.clear();
                    return Effect::merge([Effect::cancel(TYPING), Effect::cancel(SEARCH)]);
                }
                // Each keystroke replaces the wait before it. Sending
                // `Paused` rather than searching here keeps the search,
                // and its own id, in one place.
                Effect::send(SearchAction::Paused(query)).debounce(TYPING, PAUSE, self.clock.clone())
            }
            SearchAction::Paused(query) => {
                search.searching = Some(query.clone());
                let catalogue = self.catalogue.clone();
                Effect::run(move |send| async move {
                    send.send(SearchAction::Found(catalogue.search(&query).await));
                })
                .cancellable(SEARCH, true)
            }
            SearchAction::Found(results) => {
                search.searching = None;
                search.results = results;
                Effect::none()
            }
        }
    }
}

fn search_box() -> (TestStore<Searching>, TestClock, Arc<FakeCatalogue>) {
    let clock = TestClock::new();
    let catalogue = Arc::new(FakeCatalogue {
        titles: ["Dune", "Dune Messiah", "Dubliners", "Emma"]
            .map(String::from)
            .to_vec(),
        clock: clock.clone(),
        asked: Mutex::new(Vec::new()),
    });
    let reducer = Searching {
        catalogue: catalogue.clone(),
        clock: Arc::new(clock.clone()),
    };
    (TestStore::new(Search::default(), reducer), clock, catalogue)
}

fn found(titles: &[&str]) -> Vec<String> {
    titles.iter().map(|title| (*title).to_owned()).collect()
}

#[test]
fn typing_quickly_searches_once_for_the_last_query() {
    let (mut store, clock, catalogue) = search_box();
    for query in ["d", "du", "dun", "dune"] {
        store.send(SearchAction::Typed(query.into()), |s| s.query = query.into());
        clock.advance(PAUSE / 2);
    }
    clock.advance(PAUSE);
    store.receive(SearchAction::Paused("dune".into()), |s| {
        s.searching = Some("dune".into());
    });
    clock.advance(LATENCY);
    store.receive(SearchAction::Found(found(&["Dune", "Dune Messiah"])), |s| {
        s.searching = None;
        s.results = found(&["Dune", "Dune Messiah"]);
    });
    assert_eq!(*catalogue.asked.lock().unwrap(), ["dune"]);
}

#[test]
fn a_newer_query_replaces_the_search_in_flight() {
    let (mut store, clock, catalogue) = search_box();
    store.send(SearchAction::Typed("du".into()), |s| s.query = "du".into());
    clock.advance(PAUSE);
    store.receive(SearchAction::Paused("du".into()), |s| {
        s.searching = Some("du".into());
    });
    // The search for "du" is under way when the person types more.
    clock.advance(LATENCY / 2);
    store.send(SearchAction::Typed("dune m".into()), |s| s.query = "dune m".into());
    clock.advance(PAUSE);
    store.receive(SearchAction::Paused("dune m".into()), |s| {
        s.searching = Some("dune m".into());
    });
    // Long enough for both to answer; only the newer one's answer comes.
    clock.advance(LATENCY);
    store.receive(SearchAction::Found(found(&["Dune Messiah"])), |s| {
        s.searching = None;
        s.results = found(&["Dune Messiah"]);
    });
    store.finish();
    assert_eq!(*catalogue.asked.lock().unwrap(), ["du", "dune m"]);
}

#[test]
fn a_search_about_to_answer_still_lands_while_the_person_types_on() {
    let (mut store, clock, _catalogue) = search_box();
    store.send(SearchAction::Typed("du".into()), |s| s.query = "du".into());
    clock.advance(PAUSE);
    store.receive(SearchAction::Paused("du".into()), |s| {
        s.searching = Some("du".into());
    });
    // Typing restarts the pause but leaves the search running, so its
    // answer lands before the pause ends, and the newer search follows.
    clock.advance(LATENCY - PAUSE / 3);
    store.send(SearchAction::Typed("dun".into()), |s| s.query = "dun".into());
    clock.advance(PAUSE / 3);
    store.receive(
        SearchAction::Found(found(&["Dune", "Dune Messiah", "Dubliners"])),
        |s| {
            s.searching = None;
            s.results = found(&["Dune", "Dune Messiah", "Dubliners"]);
        },
    );
    clock.advance(PAUSE);
    store.receive(SearchAction::Paused("dun".into()), |s| {
        s.searching = Some("dun".into());
    });
    clock.advance(LATENCY);
    store.receive(SearchAction::Found(found(&["Dune", "Dune Messiah"])), |s| {
        s.searching = None;
        s.results = found(&["Dune", "Dune Messiah"]);
    });
}

#[test]
fn clearing_the_box_stops_the_wait_and_the_search() {
    let (mut store, clock, catalogue) = search_box();
    store.send(SearchAction::Typed("du".into()), |s| s.query = "du".into());
    clock.advance(PAUSE);
    store.receive(SearchAction::Paused("du".into()), |s| {
        s.searching = Some("du".into());
    });
    store.send(SearchAction::Typed("dub".into()), |s| s.query = "dub".into());
    store.send(SearchAction::Typed(String::new()), |s| {
        s.query.clear();
        s.searching = None;
    });
    clock.advance(LATENCY * 2);
    store.finish();
    assert_eq!(*catalogue.asked.lock().unwrap(), ["du"]);
}
