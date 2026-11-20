//! Select mode: ticks, ranges, select all, forks that go along, leaving.
use std::collections::BTreeSet;
use std::sync::Arc;

use humata2_clients::model::{ChannelId, MachineId, SessionId};
use humata2_clients::{FakeDaemon, fixture};
use humata2_mass_delete::{
    Listing, MassDelete, MassDeleteAction as A, MassDeleteState, Row, Surface,
};
use tca::{TestClock, TestStore};

fn id(name: &str) -> SessionId {
    SessionId::new(name)
}

fn row(name: &str, channel: &str, machine: &str) -> Row {
    Row {
        id: id(name),
        channel: ChannelId::new(channel),
        machine: MachineId::new(machine),
        context_fork_of: None,
        archived: false,
        live: true,
        desk: false,
    }
}

fn listing() -> Listing {
    let mut fork = row("fork", "web", "m1");
    fork.context_fork_of = Some(id("a2"));
    let mut archived = row("old", "web", "m1");
    archived.archived = true;
    archived.live = false;
    let mut desk = row("desk", "desk", "m1");
    desk.desk = true;
    Listing {
        rows: vec![
            row("a1", "web", "m1"),
            row("a2", "web", "m1"),
            fork,
            row("a3", "web", "m1"),
            archived,
            desk,
            row("o1", "ops", "m2"),
            row("b1", "api", "m1"),
        ],
        offline: BTreeSet::from([MachineId::new("m2")]),
        ..Listing::default()
    }
}

fn store() -> TestStore<MassDelete> {
    let fake = FakeDaemon::new(fixture::standard(), Arc::new(TestClock::new()));
    let state = MassDeleteState {
        listing: listing(),
        ..MassDeleteState::default()
    };
    TestStore::new(state, MassDelete::new(Arc::new(fake))).non_exhaustive()
}

fn entered() -> TestStore<MassDelete> {
    let mut store = store();
    store.send(A::Enter { surface: Surface::Sidebar, first: id("a1") }, |_| {});
    store
}

fn visible() -> Vec<SessionId> {
    ["a1", "a2", "fork", "a3", "b1"].map(id).to_vec()
}

fn picked(store: &TestStore<MassDelete>) -> Vec<String> {
    store.state().picked.iter().map(|s| s.as_str().to_owned()).collect()
}

#[test]
fn entering_ticks_the_first_row_and_toggling_ticks_and_unticks() {
    let mut store = entered();
    assert!(store.state().selecting());
    assert_eq!(picked(&store), ["a1"]);

    store.send(A::Toggle(id("a3")), |_| {});
    assert_eq!(picked(&store), ["a1", "a3"]);
    store.send(A::Toggle(id("a1")), |_| {});
    assert_eq!(picked(&store), ["a3"]);
    assert_eq!(store.state().selected_count(), 1);
}

#[test]
fn a_shift_range_ticks_everything_between_the_anchor_and_the_row() {
    let mut store = entered();
    store.send(A::RangeTo { to: id("a3"), visible: visible() }, |_| {});

    // The fork between them goes along; it is not a pick.
    assert_eq!(picked(&store), ["a1", "a2", "a3"]);
    assert!(store.state().is_along(&id("fork")));
}

#[test]
fn a_range_works_upward_too() {
    let mut store = store();
    store.send(A::Enter { surface: Surface::Home, first: id("a3") }, |_| {});
    store.send(A::RangeTo { to: id("a1"), visible: visible() }, |_| {});
    assert_eq!(picked(&store), ["a1", "a2", "a3"]);
}

#[test]
fn select_all_visible_takes_live_sessions_on_reachable_machines_only() {
    let mut store = entered();
    let mut shown = visible();
    shown.extend(["old", "desk", "o1"].map(id));
    store.send(A::SelectAllVisible(shown), |_| {});

    assert_eq!(picked(&store), ["a1", "a2", "a3", "b1"]);
}

#[test]
fn a_context_fork_goes_along_and_cannot_be_cleared() {
    let mut store = entered();
    store.send(A::Toggle(id("a2")), |_| {});
    let state = store.state();
    assert!(state.is_along(&id("fork")));
    assert_eq!(
        (state.selected_count(), state.picked_count(), state.fork_count()),
        (3, 2, 1)
    );

    store.send(A::Toggle(id("fork")), |_| {});
    assert!(store.state().is_ticked(&id("fork")));
    assert_eq!(store.state().selected_count(), 3);

    store.send(A::Toggle(id("a2")), |_| {});
    assert!(!store.state().is_ticked(&id("fork")));
}

#[test]
fn desk_sessions_cannot_be_picked() {
    let mut store = entered();
    store.send(A::Toggle(id("desk")), |_| {});
    assert_eq!(picked(&store), ["a1"]);
}

#[test]
fn a_channel_and_a_machine_tick_their_live_sessions() {
    let mut store = entered();
    store.send(A::SelectChannel(ChannelId::new("api")), |_| {});
    assert_eq!(picked(&store), ["a1", "b1"]);

    store.send(A::SelectMachine(MachineId::new("m1")), |_| {});
    assert_eq!(picked(&store), ["a1", "a2", "a3", "b1"]);
}

#[test]
fn an_offline_machine_cannot_be_ticked_as_a_whole() {
    let mut store = entered();
    assert!(!store.state().can_pick_machine(&MachineId::new("m2")));
    store.send(A::SelectMachine(MachineId::new("m2")), |_| {});
    assert_eq!(picked(&store), ["a1"]);
}

#[test]
fn select_nothing_clears_the_ticks_and_stays_in_select_mode() {
    let mut store = entered();
    store.send(A::SelectNothing, |_| {});
    assert!(picked(&store).is_empty());
    assert!(store.state().selecting());
}

#[test]
fn escape_exits_select_mode() {
    let mut store = entered();
    store.send(A::Escape, |_| {});
    assert!(!store.state().selecting());
    assert!(picked(&store).is_empty());
}

#[test]
fn going_to_another_surface_exits_but_the_same_one_does_not() {
    let mut store = entered();
    store.send(A::SurfaceChanged(Surface::Sidebar), |_| {});
    assert!(store.state().selecting());

    store.send(A::SurfaceChanged(Surface::Machine(MachineId::new("m1"))), |_| {});
    assert!(!store.state().selecting());
}

#[test]
fn a_refreshed_listing_drops_ticks_on_sessions_that_are_gone() {
    let mut store = entered();
    store.send(A::Toggle(id("a3")), |_| {});
    let mut listing = listing();
    listing.rows.retain(|row| row.id != id("a3"));
    store.send(A::Show(listing), |_| {});
    assert_eq!(picked(&store), ["a1"]);
}

#[test]
fn entering_empty_ticks_nothing_and_a_row_can_then_be_ticked() {
    let mut store = store();
    store.send(A::EnterEmpty(Surface::Home), |_| {});
    assert!(store.state().selecting());
    assert!(picked(&store).is_empty());

    store.send(A::Toggle(id("a2")), |_| {});
    assert_eq!(picked(&store), ["a2"]);
}
