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

fn milestone_workspace() -> humata2_clients::model::Workspace {
    use humata2_clients::model::{Item, ItemDetails, ItemKind, Scope};

    let mut ws = fixture::design();
    let ch = ChannelId::new("payments-rewrite");
    let make = |id: &str, title: &str, status: ItemStatus, deps: &[&str]| Item {
        id: ItemId::new(id),
        scope: Scope::Channel(ch.clone()),
        kind: ItemKind::Task,
        title: title.to_owned(),
        status,
        depends_on: deps.iter().map(|d| ItemId::new(*d)).collect(),
        session: None,
        parent: None,
        revision: 0,
    };
    ws.items.push(make("MS-1", "Launch billing v2", ItemStatus::Todo, &["STEP-2", "STEP-3"]));
    ws.items.push(make("STEP-1", "Old schema done", ItemStatus::Done, &[]));
    ws.items.push(make("STEP-2", "Build API", ItemStatus::Todo, &["STEP-1"]));
    ws.items.push(make("STEP-3", "Wire frontend", ItemStatus::InProgress, &["STEP-2"]));
    ws.item_details.insert(
        ItemId::new("MS-1"),
        ItemDetails {
            description: "Ship the new billing pipeline.".to_owned(),
            ..ItemDetails::default()
        },
    );
    ws.item_details.insert(
        ItemId::new("STEP-2"),
        ItemDetails {
            description: "Add /v2/charge".to_owned(),
            ..ItemDetails::default()
        },
    );
    ws
}

#[test]
fn starting_a_milestone_prefills_brief_and_only_attaches_milestone() {
    let workspace = milestone_workspace();
    let (clients, fakes) =
        humata2_clients::fake_clients(workspace.clone(), Arc::new(TestClock::new()));
    let mut state = AppState::launch(&clients);
    state.workspace = Arc::new(workspace);
    state.route = Route::Channel(ChannelId::new("payments-rewrite"));
    let mut store = TestStore::new(state, App::new(clients)).non_exhaustive();

    store.send(A::StartNewForItem(ItemId::new("MS-1")), |_| {});
    let Overlay::NewSession(sheet) = store.state().overlay().expect("dialog open") else {
        panic!("expected NewSession overlay");
    };
    assert_eq!(sheet.item, Some(ItemId::new("MS-1")));
    assert!(sheet.first_message.contains("Milestone MS-1: Launch billing v2"));
    assert!(sheet.first_message.contains("Ship the new billing pipeline."));
    assert!(sheet.first_message.contains("1. STEP-2 — Build API [To do]\n   Add /v2/charge"));
    assert!(sheet.first_message.contains("2. STEP-3 — Wire frontend [In progress]"));
    assert!(!sheet.first_message.contains("STEP-1"), "Done steps must be omitted");
    assert!(sheet.first_message.contains("humata board"));

    store.send(A::NewSession(N::Create), |_| {});
    store.receive_matching(
        "the session started",
        |a| matches!(a, A::SessionCreated(Ok(_))),
        |_| {},
    );
    let Route::Session(started) = store.state().route.clone() else {
        panic!("expected session route");
    };
    store.skip_received();

    let after = fakes.daemon.workspace();
    let ms = after.items.iter().find(|i| i.id.as_str() == "MS-1").unwrap();
    assert_eq!(ms.status, ItemStatus::InProgress);
    let ms_sessions = after
        .item_details
        .get(&ms.id)
        .map(|d| d.sessions.clone())
        .unwrap_or_default();
    assert!(ms_sessions.contains(&started), "milestone lists the new session");

    for (step_id, expected_status) in [
        ("STEP-1", ItemStatus::Done),
        ("STEP-2", ItemStatus::Todo),
        ("STEP-3", ItemStatus::InProgress),
    ] {
        let step = after.items.iter().find(|i| i.id.as_str() == step_id).unwrap();
        assert_eq!(step.status, expected_status, "step {step_id} keeps its status");
        assert_ne!(step.session.as_ref(), Some(&started), "step {step_id} not attached");
        let step_sessions = after
            .item_details
            .get(&step.id)
            .map(|d| d.sessions.clone())
            .unwrap_or_default();
        assert!(!step_sessions.contains(&started), "step {step_id} does not list session");
    }

    store.skip_in_flight();
    store.finish();
}

#[test]
fn plain_task_gets_empty_first_message() {
    let workspace = fixture::design();
    let (clients, _) = humata2_clients::fake_clients(workspace.clone(), Arc::new(TestClock::new()));
    let mut state = AppState::launch(&clients);
    state.workspace = Arc::new(workspace);
    state.route = Route::Channel(ChannelId::new("payments-rewrite"));
    let mut store = TestStore::new(state, App::new(clients)).non_exhaustive();

    store.send(A::StartNewForItem(ItemId::new("PAY-7")), |_| {});
    let Overlay::NewSession(sheet) = store.state().overlay().expect("dialog open") else {
        panic!("expected NewSession overlay");
    };
    assert!(sheet.first_message.is_empty());
    store.finish();
}

#[test]
fn plain_start_on_milestone_leaves_first_message_empty() {
    let workspace = milestone_workspace();
    let (clients, _) = humata2_clients::fake_clients(workspace.clone(), Arc::new(TestClock::new()));
    let mut state = AppState::launch(&clients);
    state.workspace = Arc::new(workspace);
    state.route = Route::Channel(ChannelId::new("payments-rewrite"));
    let app = App::new(clients);

    let _ = app.start_new_for_item_plain(&mut state, ItemId::new("MS-1"));
    let Overlay::NewSession(sheet) = state.overlay().expect("dialog open") else {
        panic!("expected NewSession overlay");
    };
    assert_eq!(sheet.item, Some(ItemId::new("MS-1")));
    assert!(sheet.first_message.is_empty());
}

#[test]
fn todo_start_session_dispatches_milestone_brief() {
    let workspace = milestone_workspace();
    let (clients, _) = humata2_clients::fake_clients(workspace.clone(), Arc::new(TestClock::new()));
    let mut state = AppState::launch(&clients);
    state.workspace = Arc::new(workspace);
    state.route = Route::Channel(ChannelId::new("payments-rewrite"));
    let mut store = TestStore::new(state, App::new(clients)).non_exhaustive();

    store.send(A::Todo(humata2_todo_feature::TodoAction::StartSession("MS-1".to_owned())), |_| {});
    let Overlay::NewSession(sheet) = store.state().overlay().expect("dialog open") else {
        panic!("expected NewSession overlay");
    };
    assert_eq!(sheet.item, Some(ItemId::new("MS-1")));
    assert!(sheet.first_message.contains("Milestone MS-1: Launch billing v2"));
    store.finish();
}
