//! SE-26, DK-5: the Desk's lists and their order.
use std::sync::Arc;

use humata2_clients::model::SessionId;
use humata2_clients::{FakeDaemon, Workspace, fixture};
use humata2_desk::{Desk, DeskAction as A, DeskState, Sort};
use tca::{TestClock, TestStore};

fn store() -> (TestStore<Desk>, FakeDaemon) {
    let mut workspace = fixture::design();
    fixture::desk(&mut workspace);
    let fake = FakeDaemon::new(workspace.clone(), Arc::new(TestClock::new()));
    let state = DeskState {
        workspace: Arc::new(workspace),
        ..DeskState::default()
    };
    let store = TestStore::new(state, Desk::new(Arc::new(fake.clone()))).non_exhaustive();
    (store, fake)
}

fn open_ids(state: &DeskState) -> Vec<String> {
    state
        .open()
        .iter()
        .map(|row| row.session.id.to_string())
        .collect()
}

#[test]
fn open_sessions_come_most_recently_used_first() {
    let (store, _) = store();
    assert_eq!(
        open_ids(store.state()),
        ["desk-plan-q4", "desk-brin-rfc", "desk-tidy-infra"]
    );
}

#[test]
fn using_a_session_moves_it_to_the_top() {
    let (mut store, fake) = store();
    fake.use_desk_session(&SessionId::new("desk-tidy-infra"));
    store.send(A::Show(Arc::new(fake.workspace())), |_| {});
    assert_eq!(
        open_ids(store.state()).first().map(String::as_str),
        Some("desk-tidy-infra")
    );
}

#[test]
fn sorting_by_title_ignores_case() {
    let (mut store, _) = store();
    store.send(A::SetSort(Sort::Title), |_| {});
    // BRIN rollout RFC, Plan Q4 board, Tidy infra items.
    assert_eq!(
        open_ids(store.state()),
        ["desk-brin-rfc", "desk-plan-q4", "desk-tidy-infra"]
    );
}

#[test]
fn closed_sessions_never_show_in_open_and_come_last_used_first() {
    let (store, _) = store();
    assert!(!open_ids(store.state()).contains(&"desk-release-checklist".to_owned()));
    let closed: Vec<String> = store
        .state()
        .closed()
        .iter()
        .map(|row| row.session.id.to_string())
        .collect();
    assert_eq!(closed, ["desk-release-checklist"]);
}

#[test]
fn documents_skip_a_session_that_is_gone() {
    let (mut store, _) = store();
    let names = |state: &DeskState| -> Vec<String> {
        state
            .documents()
            .iter()
            .map(|(document, _)| document.path.clone())
            .collect()
    };
    assert_eq!(names(store.state()), ["rfc-brin.md", "q4-plan.md"]);

    let mut workspace: Workspace = (*store.state().workspace).clone();
    workspace
        .sessions
        .retain(|session| session.id.as_str() != "desk-plan-q4");
    store.send(A::Show(Arc::new(workspace)), |_| {});
    assert_eq!(names(store.state()), ["rfc-brin.md"]);
}
