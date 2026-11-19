//! HOOKS (D191): a request no one can answer any more (the agent's hook
//! stopped waiting, or the agent withdrew it) leaves the Inbox: its count,
//! its list and its selection.
use humata2_app::{App, AppAction as A, AppState, Route};
use humata2_clients::{Clients, Event, fixture, model::RequestId};
use humata2_inbox::InboxAction;
use std::sync::Arc;
use tca::{TestClock, TestStore};

fn setup() -> TestStore<App> {
    let workspace = Arc::new(fixture::design());
    let (clients, _fakes) = humata2_clients::fake_clients((*workspace).clone(), Arc::new(TestClock::new()));
    let mut state = AppState::launch(&clients);
    state.workspace = workspace.clone();
    state.inbox.workspace = workspace.clone();
    state.session.workspace = workspace;
    state.route = Route::Inbox;
    TestStore::new(state, App::new(clients)).non_exhaustive()
}

#[test]
fn a_request_gone_from_the_daemon_leaves_the_inbox() {
    let mut store = setup();
    let id = RequestId::new("benchmark-command");
    store.send(A::Inbox(InboxAction::Select(id.clone())), |_| {});
    let before = store.state().workspace.waiting().count();
    assert!(
        store.state().workspace.waiting().any(|request| request.id == id),
        "the fixture's request waits before it goes"
    );

    store.send(A::Event(Event::RequestGone(id.clone())), |_| {});
    store.skip_received();

    let state = store.state();
    assert_eq!(state.workspace.waiting().count(), before - 1);
    assert!(state.workspace.requests.iter().all(|request| request.id != id));
    assert!(state.inbox.shown().iter().all(|request| request.id != id));
    assert!(state.inbox.selection().is_none_or(|request| request.id != id));
}

#[test]
fn a_request_gone_that_the_app_never_had_changes_nothing() {
    let mut store = setup();
    let before = store.state().workspace.requests.len();
    store.send(A::Event(Event::RequestGone(RequestId::new("never-listed"))), |_| {});
    store.skip_received();
    assert_eq!(store.state().workspace.requests.len(), before);
}
