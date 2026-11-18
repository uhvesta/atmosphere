//! D793: the Desk in the sidebar on every page, capped and foldable. The
//! rows under Desk are the session you are on and the ones used last, up to
//! the cap, drawn in the order they were started; a "more" row counts the
//! rest. The Desk folds like a channel, and the fold is saved.

use std::sync::Arc;

use humata2_clients::model::SessionId;
use humata2_clients::{Workspace, fixture};
use humata2_sidebar::{
    Delegate, Folding, Move, RowKey, Sidebar, SidebarAction as A, SidebarState,
};
use tca::TestStore;

fn id(text: &str) -> SessionId {
    SessionId::new(text)
}

/// The design fixture with the Desk's three open sessions, started in the
/// order Plan Q4, BRIN RFC, Tidy infra, and used last in that order too.
fn with_desk() -> Workspace {
    let mut workspace = fixture::design();
    fixture::desk(&mut workspace);
    workspace
}

/// `workspace` with `session` used after every other Desk session.
fn used_last(mut workspace: Workspace, session: &str) -> Workspace {
    if let Some(record) = workspace.desk.sessions.get_mut(&id(session)) {
        record.last_used.order = 99;
    }
    workspace
}

/// A store over `workspace` capped at `limit` Desk rows.
fn store(workspace: Workspace, limit: Option<usize>) -> TestStore<Sidebar> {
    let mut store =
        TestStore::new(SidebarState::new(Arc::new(workspace)), Sidebar).non_exhaustive();
    store.send(A::SetDeskLimit(limit), |_| {});
    store
}

fn listed(state: &SidebarState) -> Vec<SessionId> {
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
fn under_the_cap_every_open_desk_session_is_listed_in_start_order() {
    let store = store(with_desk(), Some(5));
    assert_eq!(
        listed(store.state()),
        [id("desk-plan-q4"), id("desk-brin-rfc"), id("desk-tidy-infra")]
    );
    assert_eq!(store.state().desk_more, 0);
}

#[test]
fn over_the_cap_the_ones_used_last_are_listed_in_start_order_and_the_rest_counted() {
    let store = store(used_last(with_desk(), "desk-tidy-infra"), Some(2));
    assert_eq!(
        listed(store.state()),
        [id("desk-plan-q4"), id("desk-tidy-infra")]
    );
    assert_eq!(store.state().desk_more, 1);
}

#[test]
fn the_session_you_are_on_is_listed_even_when_it_was_used_longest_ago() {
    let mut store = store(with_desk(), Some(1));
    assert_eq!(listed(store.state()), [id("desk-plan-q4")]);
    store.send(
        A::SetCurrent(Some(RowKey::Session(id("desk-tidy-infra")))),
        |_| {},
    );
    assert_eq!(listed(store.state()), [id("desk-tidy-infra")]);
    assert_eq!(store.state().desk_more, 2);
}

#[test]
fn using_a_listed_session_does_not_move_its_row() {
    let store = store(used_last(with_desk(), "desk-brin-rfc"), Some(3));
    assert_eq!(
        listed(store.state()),
        [id("desk-plan-q4"), id("desk-brin-rfc"), id("desk-tidy-infra")]
    );
}

#[test]
fn with_no_cap_every_open_desk_session_is_listed() {
    let store = store(with_desk(), None);
    assert_eq!(listed(store.state()).len(), 3);
    assert_eq!(store.state().desk_more, 0);
}

#[test]
fn a_folded_desk_lists_nothing_and_unfolding_lists_them_again() {
    let mut store = store(with_desk(), Some(5));
    store.send(A::Toggle(RowKey::Desk), |_| {});
    assert!(store.state().folding.desk);
    assert!(listed(store.state()).is_empty());
    assert_eq!(store.state().desk_more, 0);
    store.send(A::Toggle(RowKey::Desk), |_| {});
    assert_eq!(listed(store.state()).len(), 3);
}

#[test]
fn collapse_all_folds_the_desk_and_expand_all_opens_it() {
    let mut store = store(with_desk(), Some(5));
    store.send(A::CollapseAll, |_| {});
    assert!(store.state().folding.desk);
    store.send(A::ExpandAll, |_| {});
    assert!(!store.state().folding.desk);
    assert_eq!(listed(store.state()).len(), 3);
}

#[test]
fn folding_saved_before_the_desk_could_fold_reads_as_open() {
    let folding: Folding = serde_json::from_str(r#"{"channels":[],"sessions":[]}"#)
        .expect("older saved folding still reads");
    assert!(!folding.desk);
}

#[test]
fn a_restored_fold_hides_the_desk_rows() {
    let mut store = store(with_desk(), Some(5));
    let folding = Folding {
        desk: true,
        ..Folding::default()
    };
    store.send(A::Restore(folding), |_| {});
    assert!(listed(store.state()).is_empty());
}

/// The keys ↓ visits from the Desk row to the first channel.
fn walk_down(store: &mut TestStore<Sidebar>) -> Vec<RowKey> {
    store.send(A::Activate(RowKey::Desk), |_| {});
    store.skip_received();
    let mut seen = vec![RowKey::Desk];
    loop {
        store.send(A::Move(Move::Down), |_| {});
        let Some(at) = store.state().cursor.clone() else {
            break;
        };
        if matches!(at, RowKey::Channel(_)) || seen.contains(&at) {
            seen.push(at);
            break;
        }
        seen.push(at);
    }
    seen
}

#[test]
fn down_walks_the_desk_row_its_sessions_and_the_more_row_into_the_channels() {
    let mut store = store(used_last(with_desk(), "desk-tidy-infra"), Some(2));
    let seen = walk_down(&mut store);
    assert_eq!(
        seen[..4],
        [
            RowKey::Desk,
            RowKey::Session(id("desk-plan-q4")),
            RowKey::Session(id("desk-tidy-infra")),
            RowKey::DeskMore,
        ]
    );
    assert!(matches!(seen.get(4), Some(RowKey::Channel(_))));
}

#[test]
fn a_folded_desk_is_one_stop_before_the_channels() {
    let mut store = store(with_desk(), Some(5));
    store.send(A::Toggle(RowKey::Desk), |_| {});
    let seen = walk_down(&mut store);
    assert_eq!(seen.len(), 2);
    assert!(matches!(seen[1], RowKey::Channel(_)));
}

#[test]
fn left_on_a_desk_session_goes_to_the_desk_row_and_left_again_folds_it() {
    let mut store = store(with_desk(), Some(5));
    store.send(
        A::SetCurrent(Some(RowKey::Session(id("desk-brin-rfc")))),
        |_| {},
    );
    store.send(A::Move(Move::Out), |_| {});
    assert_eq!(store.state().cursor, Some(RowKey::Desk));
    store.send(A::Move(Move::Out), |_| {});
    assert!(store.state().folding.desk);
    assert_eq!(store.state().cursor, Some(RowKey::Desk));
}

#[test]
fn right_on_the_desk_row_unfolds_it_then_goes_to_its_first_session() {
    let mut store = store(with_desk(), Some(5));
    store.send(A::Toggle(RowKey::Desk), |_| {});
    store.send(A::Activate(RowKey::Desk), |_| {});
    store.skip_received();
    store.send(A::Move(Move::In), |_| {});
    assert!(!store.state().folding.desk);
    assert_eq!(store.state().cursor, Some(RowKey::Desk));
    store.send(A::Move(Move::In), |_| {});
    assert_eq!(
        store.state().cursor,
        Some(RowKey::Session(id("desk-plan-q4")))
    );
}

#[test]
fn folding_with_the_cursor_on_a_desk_session_moves_it_to_the_desk_row() {
    let mut store = store(with_desk(), Some(5));
    store.send(
        A::SetCurrent(Some(RowKey::Session(id("desk-brin-rfc")))),
        |_| {},
    );
    store.send(A::Toggle(RowKey::Desk), |_| {});
    assert_eq!(store.state().cursor, Some(RowKey::Desk));
}

#[test]
fn enter_on_the_desk_row_or_the_more_row_opens_the_desk_page() {
    for key in [RowKey::Desk, RowKey::DeskMore] {
        let mut store = store(used_last(with_desk(), "desk-tidy-infra"), Some(1));
        store.send(A::Activate(key.clone()), |_| {});
        store.receive(A::Delegate(Delegate::OpenDesk), |_| {});
        assert_eq!(store.state().cursor, Some(key));
        store.send(A::OpenCursor, |_| {});
        store.receive(A::Delegate(Delegate::OpenDesk), |_| {});
    }
}

#[test]
fn with_no_open_desk_session_the_desk_is_not_a_stop() {
    let mut store = store(fixture::design(), Some(5));
    assert_eq!(store.state().desk_open, 0);
    let first = store
        .state()
        .rows
        .iter()
        .find_map(|row| row.key.clone())
        .expect("the fixture has channels");
    store.send(A::SetCurrent(Some(first.clone())), |_| {});
    store.send(A::Move(Move::Up), |_| {});
    assert_eq!(store.state().cursor, Some(first));
}
