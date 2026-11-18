//! The session page's Files panel lists the session's files. A list read
//! before the clones were on disk came back empty; opening Files reads it
//! again, and a list that has files is not read twice.
use std::sync::Arc;

use humata2_app::{App, AppAction as A, AppState, Route};
use humata2_clients::model::SessionId;
use humata2_clients::{Clients, Fakes, fixture};
use humata2_session::{Panel, SessionAction};
use tca::{TestClock, TestStore};

fn file_requests(fakes: &Fakes) -> usize {
    fakes
        .daemon
        .calls()
        .iter()
        .filter(|call| call.starts_with("find_files"))
        .count()
}

/// The app on a session's page, its first file list read, and that
/// session: one the design workspace gives files.
fn on_a_session_page() -> (TestStore<App>, Fakes, SessionId) {
    let (clients, fakes) = Clients::fake(fixture::design(), Arc::new(TestClock::new()));
    let session = fakes
        .daemon
        .workspace()
        .session_files
        .iter()
        .find(|(_, files)| !files.is_empty())
        .map(|(id, _)| id.clone())
        .expect("the design workspace has a session with files");
    let state = AppState::launch(&clients);
    let mut store = TestStore::new(state, App::new(clients)).non_exhaustive();
    store.send(A::Loaded(Ok(fakes.daemon.workspace())), |_| {});
    store.settle();
    store.send(A::Switch(Route::Session(session.clone())), |_| {});
    store.settle();
    (store, fakes, session)
}

fn listed(store: &TestStore<App>, session: &SessionId) -> usize {
    store
        .state()
        .workspace
        .session_files
        .get(session)
        .map_or(0, Vec::len)
}

#[test]
fn opening_files_reads_a_list_that_came_back_empty_again() {
    let (mut store, fakes, session) = on_a_session_page();
    store.send(
        A::SessionFiles {
            session: session.clone(),
            result: Ok(Vec::new()),
        },
        |_| {},
    );
    assert_eq!(listed(&store, &session), 0);
    let before = file_requests(&fakes);

    store.send(A::Session(SessionAction::SetPanel(Some(Panel::Files))), |_| {});
    store.receive_matching(
        "the session's files, read again",
        |a| matches!(a, A::SessionFiles { session: s, result: Ok(files) } if *s == session && !files.is_empty()),
        |_| {},
    );
    assert_eq!(file_requests(&fakes), before + 1, "the list is read again");
    assert!(listed(&store, &session) > 0, "the panel has the session's files");
    store.skip_in_flight();
}

#[test]
fn opening_files_with_its_list_read_reads_nothing() {
    let (mut store, fakes, session) = on_a_session_page();
    assert!(listed(&store, &session) > 0);
    let before = file_requests(&fakes);

    store.send(A::Session(SessionAction::SetPanel(Some(Panel::Files))), |_| {});
    store.settle();
    assert_eq!(file_requests(&fakes), before);
    store.skip_in_flight();
}
