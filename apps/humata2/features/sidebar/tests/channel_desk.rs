//! The keyboard cursor over a channel's own Desk rows (CD-2): they sit under
//! their channel, above its sessions, and ↑ ↓ walk through them in the order
//! they are drawn.

use std::sync::Arc;

use humata2_clients::model::ChannelId;
use humata2_clients::{Workspace, fixture};
use humata2_sidebar::{Move, RowKey, Sidebar, SidebarAction as A, SidebarState};
use tca::TestStore;

fn payments() -> ChannelId {
    ChannelId::new("payments-rewrite")
}

fn workspace() -> Workspace {
    let mut workspace = fixture::design();
    fixture::channel_desk(&mut workspace);
    workspace
}

fn store() -> TestStore<Sidebar> {
    TestStore::new(SidebarState::new(Arc::new(workspace())), Sidebar).non_exhaustive()
}

/// The channel's row, its Desk sessions, then its first session, as drawn.
fn drawn(store: &TestStore<Sidebar>) -> (RowKey, Vec<RowKey>, RowKey) {
    let state = store.state();
    let channel = RowKey::Channel(payments());
    let desks: Vec<RowKey> = state
        .channel_desks
        .get(&payments())
        .map(|rows| rows.iter().filter_map(|row| row.key.clone()).collect())
        .unwrap_or_default();
    let keys: Vec<RowKey> = state.rows.iter().filter_map(|row| row.key.clone()).collect();
    let at = keys.iter().position(|key| *key == channel).unwrap_or(0);
    let first = keys.get(at + 1).cloned().unwrap_or_else(|| channel.clone());
    (channel, desks, first)
}

fn put_cursor(store: &mut TestStore<Sidebar>, key: &RowKey) {
    store.send(A::SetCurrent(Some(key.clone())), |_| {});
}

#[test]
fn down_walks_from_the_channel_into_its_desk_sessions_then_its_own() {
    let mut store = store();
    let (channel, desks, first) = drawn(&store);
    assert_eq!(desks.len(), 2, "the fixture has two open Desk sessions");
    assert!(!desks.contains(&first), "they are not in the sessions tree");
    put_cursor(&mut store, &channel);
    let header = RowKey::ChannelDesk(payments());
    for key in std::iter::once(&header)
        .chain(desks.iter())
        .chain(std::iter::once(&first))
    {
        store.send(A::Move(Move::Down), |_| {});
        assert_eq!(store.state().cursor.as_ref(), Some(key));
    }
}

#[test]
fn up_walks_back_from_the_channels_first_session_through_its_desk_sessions() {
    let mut store = store();
    let (channel, desks, first) = drawn(&store);
    put_cursor(&mut store, &first);
    let header = RowKey::ChannelDesk(payments());
    for key in desks
        .iter()
        .rev()
        .chain(std::iter::once(&header))
        .chain(std::iter::once(&channel))
    {
        store.send(A::Move(Move::Up), |_| {});
        assert_eq!(store.state().cursor.as_ref(), Some(key));
    }
}

#[test]
fn a_folded_channels_desk_sessions_are_skipped() {
    let mut store = store();
    let (channel, desks, _first) = drawn(&store);
    store.send(A::Toggle(channel.clone()), |_| {});
    put_cursor(&mut store, &channel);
    store.send(A::Move(Move::Down), |_| {});
    let cursor = store.state().cursor.clone();
    assert!(
        cursor.as_ref().is_some_and(|key| !desks.contains(key)),
        "down from a folded channel doesn't stop on its Desk sessions"
    );
}

#[test]
fn enter_on_a_channel_desk_row_opens_that_session() {
    let mut store = store();
    let (channel, desks, _first) = drawn(&store);
    put_cursor(&mut store, &channel);
    store.send(A::Move(Move::Down), |_| {});
    store.send(A::Move(Move::Down), |_| {});
    store.send(A::OpenCursor, |_| {});
    let Some(RowKey::Session(opened)) = desks.first().cloned() else {
        panic!("the fixture has Desk sessions")
    };
    store.receive(
        A::Delegate(humata2_sidebar::Delegate::OpenSession(opened)),
        |_| {},
    );
}
