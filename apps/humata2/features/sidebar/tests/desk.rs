//! The sidebar's Desk rows: the open Desk sessions in the order they were
//! started, kept out of the channels (DK-1, D148).

use std::sync::Arc;

use humata2_clients::model::SessionId;
use humata2_clients::{Workspace, fixture};
use humata2_sidebar::{Delegate, Move, RowKey, Sidebar, SidebarAction as A, SidebarState};
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
fn a_pinned_desk_session_still_shows_under_the_desk() {
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
    store.finish();
}

/// A store over `workspace`, its Desk rows showing or not, and where the
/// rows it draws begin and end.
fn walking(workspace: Workspace, shown: bool) -> (TestStore<Sidebar>, Landmarks) {
    let mut state = SidebarState::new(Arc::new(workspace));
    let keys = |rows: &[humata2_sidebar::Row]| -> Vec<RowKey> {
        rows.iter().filter_map(|row| row.key.clone()).collect()
    };
    let channel = keys(&state.rows)
        .into_iter()
        .find(|key| matches!(key, RowKey::Channel(_)));
    let landmarks = Landmarks {
        first_channel: channel,
        desk: keys(&state.desk),
    };
    state.desk_shown = shown;
    (TestStore::new(state, Sidebar), landmarks)
}

struct Landmarks {
    first_channel: Option<RowKey>,
    desk: Vec<RowKey>,
}

fn go(store: &mut TestStore<Sidebar>, movement: Move, to: &RowKey) {
    store.send(A::Move(movement), |state| state.cursor = Some(to.clone()));
}

#[test]
fn down_walks_from_the_first_desk_row_through_the_desk_into_the_channels() {
    let (mut store, marks) = walking(with_desk(), true);
    let (Some(first), Some(channel)) = (marks.desk.first().cloned(), marks.first_channel.clone())
    else {
        panic!("the fixture has Desk rows and channels");
    };
    assert_eq!(marks.desk.len(), 3);
    store.send(A::SetCurrent(Some(first.clone())), |state| {
        state.cursor = Some(first.clone());
        state.current = Some(first.clone());
    });
    for key in &marks.desk[1..] {
        go(&mut store, Move::Down, key);
    }
    go(&mut store, Move::Down, &channel);
    store.finish();
}

#[test]
fn up_walks_from_the_first_channel_back_through_the_desk() {
    let (mut store, marks) = walking(with_desk(), true);
    let Some(channel) = marks.first_channel.clone() else {
        panic!("the fixture has channels");
    };
    store.send(A::SetCurrent(Some(channel.clone())), |state| {
        state.cursor = Some(channel.clone());
        state.current = Some(channel.clone());
    });
    for key in marks.desk.iter().rev() {
        go(&mut store, Move::Up, key);
    }
    store.finish();
}

#[test]
fn up_from_the_first_desk_row_stays_there() {
    let (mut store, marks) = walking(with_desk(), true);
    let Some(first) = marks.desk.first().cloned() else {
        panic!("the fixture has Desk rows");
    };
    store.send(A::SetCurrent(Some(first.clone())), |state| {
        state.cursor = Some(first.clone());
        state.current = Some(first.clone());
    });
    go(&mut store, Move::Up, &first);
    store.finish();
}

#[test]
fn enter_on_a_desk_row_opens_that_session() {
    let (mut store, marks) = walking(with_desk(), true);
    let Some(key) = marks.desk.get(1).cloned() else {
        panic!("the fixture has Desk rows");
    };
    store.send(A::SetCurrent(Some(key.clone())), |state| {
        state.cursor = Some(key.clone());
        state.current = Some(key.clone());
    });
    store.send(A::OpenCursor, |_| {});
    store.receive(
        A::Delegate(Delegate::OpenSession(id("desk-brin-rfc"))),
        |_| {},
    );
    store.finish();
}

#[test]
fn with_the_desk_rows_not_showing_down_skips_them() {
    let (mut store, marks) = walking(with_desk(), false);
    let Some(channel) = marks.first_channel.clone() else {
        panic!("the fixture has channels");
    };
    store.send(A::SetCurrent(Some(channel.clone())), |state| {
        state.cursor = Some(channel.clone());
        state.current = Some(channel.clone());
    });
    let next = {
        let state = store.state();
        let rows: Vec<_> = state.rows.iter().filter_map(|row| row.key.clone()).collect();
        let at = rows.iter().position(|key| *key == channel).unwrap_or(0);
        rows.get(at + 1).cloned().unwrap_or_else(|| channel.clone())
    };
    assert!(!marks.desk.contains(&next));
    go(&mut store, Move::Down, &next);
    store.finish();
}
