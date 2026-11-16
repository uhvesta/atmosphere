//! The sidebar's Desk rows: the open Desk sessions in the order they were
//! started, kept out of the channels (DK-1, D148).

use std::sync::Arc;

use humata2_clients::model::SessionId;
use humata2_clients::{Workspace, fixture};
use humata2_sidebar::{Delegate, RowKey, Sidebar, SidebarAction as A, SidebarState};
use tca::TestStore;

fn with_desk() -> Workspace {
    let mut workspace = fixture::design();
    fixture::desk(&mut workspace);
    workspace
}

fn id(text: &str) -> SessionId {
    SessionId::new(text)
}

/// A sidebar that has been shown `workspace`.
fn shown(workspace: Workspace) -> TestStore<Sidebar> {
    let workspace = Arc::new(workspace);
    let mut store = TestStore::new(SidebarState::default(), Sidebar);
    let expected = SidebarState::new(workspace.clone());
    store.send(A::Show(workspace), |state| *state = expected);
    store
}

fn desk_ids(state: &SidebarState) -> Vec<SessionId> {
    state
        .desk
        .iter()
        .filter_map(|row| match &row.key {
            Some(RowKey::Session(id)) => Some(id.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn the_open_desk_sessions_are_listed_in_the_order_they_started() {
    let store = shown(with_desk());
    assert_eq!(
        desk_ids(store.state()),
        vec![id("desk-plan-q4"), id("desk-brin-rfc"), id("desk-tidy-infra")]
    );
    assert!(store.state().desk.iter().all(|row| row.depth == 1));
    store.finish();
}

#[test]
fn a_desk_session_is_under_no_channel() {
    let with = shown(with_desk());
    let without = shown(fixture::design());
    assert_eq!(with.state().rows, without.state().rows);
    assert!(desk_ids(without.state()).is_empty());
    with.finish();
    without.finish();
}

#[test]
fn a_row_does_not_move_when_its_session_is_used() {
    let mut workspace = with_desk();
    if let Some(record) = workspace.desk.sessions.get_mut(&id("desk-tidy-infra")) {
        record.last_used.order = 99;
    }
    let store = shown(workspace);
    assert_eq!(
        desk_ids(store.state()),
        vec![id("desk-plan-q4"), id("desk-brin-rfc"), id("desk-tidy-infra")]
    );
    store.finish();
}

#[test]
fn a_reopened_session_is_listed_again_where_it_was() {
    let mut workspace = with_desk();
    let closed = id("desk-release-checklist");
    if let Some(record) = workspace.desk.sessions.get_mut(&closed) {
        record.closed = None;
    }
    if let Some(session) = workspace.sessions.iter_mut().find(|s| s.id == closed) {
        session.archived = false;
    }
    let store = shown(workspace);
    assert_eq!(
        desk_ids(store.state()),
        vec![
            id("desk-plan-q4"),
            id("desk-brin-rfc"),
            id("desk-tidy-infra"),
            closed
        ]
    );
    store.finish();
}

#[test]
fn opening_a_desk_row_keeps_the_cursor_on_it() {
    let workspace = Arc::new(with_desk());
    let mut store = shown((*workspace).clone());
    let key = RowKey::Session(id("desk-brin-rfc"));
    store.send(A::Activate(key.clone()), |state| {
        state.cursor = Some(key.clone());
    });
    store.receive(A::Delegate(Delegate::OpenSession(id("desk-brin-rfc"))), |_| {});
    store.send(A::Show(workspace), |_| {});
    assert_eq!(store.state().cursor, Some(key));
    store.finish();
}

#[test]
fn a_pinned_desk_session_shows_under_pinned_and_under_the_desk() {
    let mut workspace = with_desk();
    if let Some(session) = workspace
        .sessions
        .iter_mut()
        .find(|session| session.id == id("desk-brin-rfc"))
    {
        session.pinned = true;
    }
    let store = shown(workspace);
    assert!(desk_ids(store.state()).contains(&id("desk-brin-rfc")));
    assert!(store.state().index_of(&RowKey::Pinned(id("desk-brin-rfc"))).is_some());
    store.finish();
}
