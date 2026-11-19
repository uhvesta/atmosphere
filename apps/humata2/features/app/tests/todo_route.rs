//! BOARDSIMPLE: the To-do page loads its list when it opens, and an item's
//! session opens from it.
use humata2_app::{App, AppAction as A, AppState, Route};
use humata2_clients::fixture;
use humata2_todo_feature::TodoAction;
use std::sync::Arc;
use tca::{TestClock, TestStore};

fn store() -> (TestStore<App>, humata2_clients::model::SessionId) {
    let workspace = fixture::design();
    let session = workspace.sessions[0].id.clone();
    let (clients, _fakes) =
        humata2_clients::fake_clients(workspace.clone(), Arc::new(TestClock::new()));
    let mut state = AppState::launch(&clients);
    state.workspace = Arc::new(workspace);
    (TestStore::new(state, App::new(clients)).non_exhaustive(), session)
}

#[test]
fn opening_the_to_do_page_loads_the_list() {
    let (mut store, _) = store();
    store.send(A::Switch(Route::Todo), |_| {});
    store.receive_matching(
        "the list loaded",
        |a| matches!(a, A::Todo(TodoAction::Loaded(..))),
        |_| {},
    );
    assert!(!store.state().todo.items.is_empty());
    store.skip_received();
    store.skip_in_flight();
    store.finish();
}

#[test]
fn an_items_session_opens_from_the_list() {
    let (mut store, session) = store();
    store.send(A::Switch(Route::Todo), |_| {});
    store.send(A::Todo(TodoAction::OpenSession(session.as_str().to_owned())), |_| {});
    assert_eq!(store.state().route, Route::Session(session));
    store.skip_received();
    store.skip_in_flight();
    store.finish();
}
