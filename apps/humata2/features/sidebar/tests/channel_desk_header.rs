//! The keyboard cursor on a channel's Desk row (CD-2): the "Desk" row under a
//! channel is a stop of its own between the channel and its Desk sessions.
//! Down and Up walk through it, Right goes into the Desk sessions, Left comes
//! back to the channel, and Enter or a click opens the channel on its Desk
//! tab.

use std::sync::Arc;

use humata2_clients::model::ChannelId;
use humata2_clients::{Workspace, fixture};
use humata2_sidebar::{Delegate, Move, RowKey, Sidebar, SidebarAction as A, SidebarState};
use tca::TestStore;

fn payments() -> ChannelId {
    ChannelId::new("payments-rewrite")
}

fn header() -> RowKey {
    RowKey::ChannelDesk(payments())
}

fn workspace() -> Workspace {
    let mut workspace = fixture::design();
    fixture::channel_desk(&mut workspace);
    workspace
}

fn store() -> TestStore<Sidebar> {
    TestStore::new(SidebarState::new(Arc::new(workspace())), Sidebar).non_exhaustive()
}

fn first_desk_session(store: &TestStore<Sidebar>) -> RowKey {
    let Some(key) = store
        .state()
        .channel_desks
        .get(&payments())
        .and_then(|rows| rows.first())
        .and_then(|row| row.key.clone())
    else {
        panic!("the fixture has Desk sessions")
    };
    key
}

fn on_header(store: &mut TestStore<Sidebar>) {
    store.send(A::SetCurrent(Some(RowKey::Channel(payments()))), |_| {});
    store.send(A::Move(Move::Down), |_| {});
    assert_eq!(store.state().cursor, Some(header()));
}

#[test]
fn down_from_the_channel_stops_on_its_desk_row_before_the_first_desk_session() {
    let mut store = store();
    on_header(&mut store);
    let first = first_desk_session(&store);
    store.send(A::Move(Move::Down), |_| {});
    assert_eq!(store.state().cursor, Some(first));
}

#[test]
fn up_from_the_first_desk_session_stops_on_the_desk_row_then_the_channel() {
    let mut store = store();
    let first = first_desk_session(&store);
    store.send(A::SetCurrent(Some(first)), |_| {});
    store.send(A::Move(Move::Up), |_| {});
    assert_eq!(store.state().cursor, Some(header()));
    store.send(A::Move(Move::Up), |_| {});
    assert_eq!(store.state().cursor, Some(RowKey::Channel(payments())));
}

#[test]
fn enter_on_the_desk_row_opens_the_channels_desk_tab() {
    let mut store = store();
    on_header(&mut store);
    store.send(A::OpenCursor, |_| {});
    store.receive(A::Delegate(Delegate::OpenChannelDesk(payments())), |_| {});
}

#[test]
fn clicking_the_desk_row_puts_the_cursor_on_it_and_opens_the_tab() {
    let mut store = store();
    store.send(A::Activate(header()), |_| {});
    assert_eq!(store.state().cursor, Some(header()));
    store.receive(A::Delegate(Delegate::OpenChannelDesk(payments())), |_| {});
}

#[test]
fn right_goes_into_the_desk_sessions_and_left_back_to_the_channel() {
    let mut store = store();
    on_header(&mut store);
    store.send(A::Move(Move::In), |_| {});
    assert_eq!(store.state().cursor, Some(first_desk_session(&store)));
    on_header(&mut store);
    store.send(A::SetCurrent(Some(header())), |_| {});
    store.send(A::Move(Move::Out), |_| {});
    assert_eq!(store.state().cursor, Some(RowKey::Channel(payments())));
}

#[test]
fn opening_the_channel_from_its_desk_row_keeps_the_cursor_there() {
    let mut store = store();
    on_header(&mut store);
    store.send(A::SetCurrent(Some(RowKey::Channel(payments()))), |_| {});
    assert_eq!(
        store.state().cursor,
        Some(header()),
        "the page the row opened is the channel's, and the cursor stays on the row"
    );
}

#[test]
fn the_desk_row_goes_when_the_channel_has_no_open_desk_session() {
    let mut store = store();
    on_header(&mut store);
    store.send(A::Show(Arc::new(fixture::design())), |_| {});
    assert_eq!(store.state().cursor, None);
}

#[test]
fn a_folded_channels_desk_row_is_skipped() {
    let mut store = store();
    let channel = RowKey::Channel(payments());
    store.send(A::Toggle(channel.clone()), |_| {});
    store.send(A::SetCurrent(Some(channel)), |_| {});
    store.send(A::Move(Move::Down), |_| {});
    assert_ne!(store.state().cursor, Some(header()));
}
