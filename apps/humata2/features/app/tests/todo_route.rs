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

#[test]
fn an_items_channel_opens_from_the_list() {
    let (mut store, _) = store();
    let channel = store.state().workspace.channels[0].id.clone();
    store.send(A::Switch(Route::Todo), |_| {});
    store.send(A::Todo(TodoAction::OpenChannel(channel.as_str().to_owned())), |_| {});
    assert_eq!(store.state().route, Route::Channel(channel));
    store.skip_received();
    store.skip_in_flight();
    store.finish();
}

fn loaded() -> (TestStore<App>, humata2_clients::model::SessionId) {
    let (mut store, session) = store();
    store.send(A::Switch(Route::Todo), |_| {});
    store.receive_matching(
        "the list loaded",
        |a| matches!(a, A::Todo(TodoAction::Loaded(..))),
        |_| {},
    );
    (store, session)
}

fn sheet(store: &TestStore<App>) -> Option<humata2_app::NewSession> {
    store.state().overlays.iter().rev().find_map(|o| match o {
        humata2_app::Overlay::NewSession(sheet) => Some(sheet.clone()),
        _ => None,
    })
}

#[test]
fn opening_a_session_loads_the_list_for_its_panel() {
    let (mut store, session) = store();
    assert!(!store.state().todo.loaded);
    store.send(A::Switch(Route::Session(session)), |_| {});
    store.receive_matching(
        "the list loaded",
        |a| matches!(a, A::Todo(TodoAction::Loaded(..))),
        |_| {},
    );
    assert!(store.state().todo.loaded);
    store.skip_received();
    store.skip_in_flight();
    store.finish();
}

#[test]
fn do_next_opens_new_session_filled_from_the_top_item() {
    let (mut store, _) = loaded();
    let top = store.state().todo.ranked().do_next.expect("a top item");
    store.send(A::Todo(TodoAction::DoNext), |_| {});
    let entry = store.state().todo.items.iter().find(|e| e.todo.key == top).cloned();
    let entry = entry.expect("the top item is listed");
    let sheet = sheet(&store).expect("the sheet is open");
    assert_eq!(sheet.title, entry.todo.title);
    assert!(sheet.first_message.starts_with(&entry.todo.title));
    assert_eq!(sheet.item.as_ref().map(|i| i.as_str()), Some(top.as_str()));
    store.skip_received();
    store.skip_in_flight();
    store.finish();
}

#[test]
fn skip_moves_the_sheet_to_the_next_item_and_closing_it_ends_do_next() {
    let (mut store, _) = loaded();
    store.send(A::Todo(TodoAction::DoNext), |_| {});
    let first = sheet(&store).expect("open").title;
    store.send(A::Todo(TodoAction::SkipDoNext), |_| {});
    let second = sheet(&store).expect("still open").title;
    assert_ne!(first, second);
    assert_eq!(
        store.state().overlays.iter().filter(|o| matches!(o, humata2_app::Overlay::NewSession(_))).count(),
        1
    );
    store.send(A::Escape, |_| {});
    assert!(sheet(&store).is_none());
    assert_eq!(store.state().todo.do_next, None);
    store.skip_received();
    store.skip_in_flight();
    store.finish();
}

#[test]
fn do_next_with_nothing_to_do_opens_no_sheet() {
    let (mut store, _) = store();
    store.send(A::Todo(TodoAction::DoNext), |_| {});
    assert!(sheet(&store).is_none());
    store.skip_received();
    store.skip_in_flight();
    store.finish();
}

#[test]
fn start_the_review_opens_the_pull_request_review_for_the_items_pulls() {
    let (mut store, _) = loaded();
    store.send(A::Todo(TodoAction::StartReview("REV-1".to_owned())), |_| {});
    assert!(store.state().pr_dialog.open);
    store.skip_received();
    store.skip_in_flight();
    store.finish();
}
