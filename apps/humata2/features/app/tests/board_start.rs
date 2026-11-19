//! SE-3, BD-16: starting a session for a board item adds the session to the
//! item, and an item not yet started moves to In progress.
use humata2_app::{App, AppAction as A, AppState, NewSessionAction as N, Overlay, Route};
use humata2_clients::{
    Clients, fixture,
    model::{ChannelId, ItemId, ItemStatus, SessionId},
};
use std::sync::Arc;
use tca::{TestClock, TestStore};

/// Starts a session for `item` from its drawer and returns the new
/// session with the item as the daemon has it afterwards.
fn start_for(item: &str) -> (SessionId, ItemStatus, Vec<SessionId>, Option<SessionId>) {
    let workspace = fixture::design();
    let (clients, fakes) = humata2_clients::fake_clients(workspace.clone(), Arc::new(TestClock::new()));
    let mut state = AppState::launch(&clients);
    state.workspace = Arc::new(workspace);
    state.route = Route::Channel(ChannelId::new("payments-rewrite"));
    let mut store = TestStore::new(state, App::new(clients)).non_exhaustive();
    store.send(A::StartNewForItem(ItemId::new(item)), |_| {});
    assert!(
        matches!(store.state().overlay(), Some(Overlay::NewSession(_))),
        "the launch dialog opens for the item"
    );
    store.send(A::NewSession(N::Create), |_| {});
    let mut started = None;
    store.receive_matching(
        "the session started",
        |a| matches!(a, A::SessionCreated(Ok(_))),
        |_| {},
    );
    if let Route::Session(id) = &store.state().route {
        started = Some(id.clone());
    }
    let started = started.expect("the app opens the started session");
    store.skip_received();
    let after = fakes.daemon.workspace();
    let found = after
        .items
        .iter()
        .find(|i| i.id.as_str() == item)
        .expect("the item is still on the board");
    let sessions = after
        .item_details
        .get(&found.id)
        .map(|d| d.sessions.clone())
        .unwrap_or_default();
    let status = found.status;
    let session = found.session.clone();
    store.skip_in_flight();
    store.finish();
    (started, status, sessions, session)
}

#[test]
fn starting_a_session_for_a_to_do_item_adds_it_and_starts_the_item() {
    let (started, status, sessions, session) = start_for("PAY-7");
    assert_eq!(status, ItemStatus::InProgress);
    assert!(sessions.contains(&started), "the item lists the new session");
    assert!(session.is_some(), "the item counts as assigned");
}

#[test]
fn starting_a_session_for_an_item_in_review_keeps_its_column() {
    let (started, status, sessions, _) = start_for("PAY-5");
    assert_eq!(status, ItemStatus::InReview);
    assert!(sessions.contains(&started));
}
