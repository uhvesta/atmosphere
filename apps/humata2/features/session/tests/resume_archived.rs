//! D784: a session ended for good is never resumed; only Restore brings it back.
use humata2_clients::{Clients, fixture, model::SessionId};
use humata2_session::{SessionAction as A, SessionState, Sessions};
use std::sync::Arc;
use tca::{TestClock, TestStore};

#[test]
fn resume_on_an_open_archived_session_asks_the_daemon_nothing() {
    let mut workspace = fixture::design();
    let id = SessionId::new("ledger");
    let Some(session) = workspace.sessions.iter_mut().find(|s| s.id == id) else {
        panic!("the design fixture has ledger");
    };
    session.archived = true;
    let (clients, fakes) = Clients::fake(workspace.clone(), Arc::new(TestClock::new()));
    let state = SessionState {
        workspace: Arc::new(workspace),
        open: Some(id),
        ..Default::default()
    };
    let mut store = TestStore::new(state, Sessions::new(clients));
    let before = fakes.daemon.calls().len();
    store.send(A::Resume, |_| {});
    assert_eq!(fakes.daemon.calls().len(), before, "no resume call");
    store.finish();
}
