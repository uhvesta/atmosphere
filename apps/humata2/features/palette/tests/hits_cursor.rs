//! The palette's cursor runs on past its own entries over the search hits
//! the app shows under them (LD-23, LD-24, frame 30), and over a "more"
//! row when there is another page: a hit opens and the palette closes, the
//! "more" row loads the next page and the palette stays.
use std::sync::Arc;
use std::time::Duration;

use humata2_clients::fixture;
use humata2_clients::model::{MachineId, SessionId};
use humata2_palette::{Command, Delegate, Palette, PaletteAction as A, PaletteState};
use tca::TestStore;

/// Typed so that no command, channel or session matches.
const NOTHING: &str = "zzqqxxjj";

fn opened() -> TestStore<Palette> {
    let mut store = TestStore::new(PaletteState::default(), Palette)
        .non_exhaustive()
        .with_timeout(Duration::from_millis(100));
    store.send(A::Open { in_session: false }, |s| s.open = true);
    store
}

fn typed(store: &mut TestStore<Palette>, query: &str) {
    store.send(A::Query(query.to_owned()), |s| s.query = query.to_owned());
}

fn entries(store: &TestStore<Palette>) -> usize {
    store.state().entries.len()
}

#[test]
fn the_cursor_wraps_over_the_hits_and_the_more_row() {
    let mut store = opened();
    typed(&mut store, NOTHING);
    assert_eq!(entries(&store), 0);
    store.send(A::Hits { count: 2, more: true }, |s| {
        s.hits = 2;
        s.more_hits = true;
    });
    store.send(A::Move(1), |s| s.cursor = 1);
    store.send(A::Move(1), |s| s.cursor = 2);
    store.send(A::Move(1), |s| s.cursor = 0);
    store.send(A::Move(-1), |s| s.cursor = 2);
}

#[test]
fn running_a_hit_closes_the_palette_and_hands_its_place_to_the_app() {
    let mut store = opened();
    let commands = entries(&store);
    assert!(commands > 0, "an empty query lists the commands");
    store.send(A::Hits { count: 2, more: false }, |s| s.hits = 2);
    // Up from the first entry wraps to the last row: the second hit.
    store.send(A::Move(-1), |s| s.cursor = commands + 1);
    let recent = store.state().recent.clone();
    store.send(A::Run, |s| {
        s.open = false;
        s.recent = recent;
    });
    store.receive(A::Delegate(Delegate::Closed), |_| {});
    store.receive(A::Delegate(Delegate::Hit(1)), |_| {});
}

#[test]
fn clicking_a_hit_does_what_running_it_does() {
    let mut store = opened();
    typed(&mut store, NOTHING);
    store.send(A::Hits { count: 3, more: false }, |s| s.hits = 3);
    store.send(A::Pick(2), |s| s.open = false);
    store.receive(A::Delegate(Delegate::Closed), |_| {});
    store.receive(A::Delegate(Delegate::Hit(2)), |_| {});
    assert!(store.state().recent.is_empty(), "hits are not recent picks");
}

#[test]
fn running_the_more_row_asks_for_more_and_keeps_the_palette_open() {
    let mut store = opened();
    typed(&mut store, NOTHING);
    store.send(A::Hits { count: 2, more: true }, |s| {
        s.hits = 2;
        s.more_hits = true;
    });
    store.send(A::Move(-1), |s| s.cursor = 2);
    store.send(A::Run, |s| {
        s.open = true;
        s.cursor = 2;
    });
    store.receive(A::Delegate(Delegate::MoreHits), |_| {});
    let closed = store.try_receive_matching(
        "Closed",
        |action| matches!(action, A::Delegate(Delegate::Closed)),
        |_| {},
    );
    assert!(closed.is_err(), "the palette stays open for the next page");
    assert!(store.state().recent.is_empty());
}

#[test]
fn past_every_row_nothing_runs() {
    let mut store = opened();
    typed(&mut store, NOTHING);
    store.send(A::Hits { count: 1, more: false }, |s| s.hits = 1);
    store.send(A::Pick(5), |s| s.open = true);
    let sent = store.try_receive_matching(
        "a delegate",
        |action| matches!(action, A::Delegate(_)),
        |_| {},
    );
    assert!(sent.is_err());
}

#[test]
fn fewer_hits_bring_the_cursor_back_onto_a_row() {
    let mut store = opened();
    typed(&mut store, NOTHING);
    store.send(A::Hits { count: 5, more: false }, |s| s.hits = 5);
    store.send(A::Move(-1), |s| s.cursor = 4);
    store.send(A::Hits { count: 2, more: false }, |s| {
        s.hits = 2;
        s.cursor = 1;
    });
    store.send(A::Hits { count: 0, more: false }, |s| {
        s.hits = 0;
        s.cursor = 0;
    });
}

#[test]
fn hits_arriving_leave_a_cursor_that_still_fits() {
    let mut store = opened();
    assert!(entries(&store) > 2);
    store.send(A::Move(1), |s| s.cursor = 1);
    store.send(A::Hits { count: 4, more: true }, |s| {
        s.hits = 4;
        s.more_hits = true;
        s.cursor = 1;
    });
}

#[test]
fn while_asking_for_a_machine_the_hits_are_not_rows() {
    let mut workspace = fixture::design();
    for machine in &mut workspace.machines {
        if machine.id == MachineId::new("gpu-box") {
            machine.online = true;
        }
    }
    let state = PaletteState {
        workspace: Arc::new(workspace),
        context: Some(SessionId::new("rate")),
        ..Default::default()
    };
    let mut store = TestStore::new(state, Palette).non_exhaustive();
    store.send(A::Open { in_session: true }, |s| s.open = true);
    typed(&mut store, "move");
    let at = store
        .state()
        .entries
        .iter()
        .position(|e| e.command == Command::MoveSession)
        .unwrap_or_else(|| panic!("Move session is offered"));
    store.send(A::Pick(at), |s| s.open = true);
    store.send(A::Hits { count: 3, more: true }, |s| {
        s.hits = 3;
        s.more_hits = true;
    });
    let machines = entries(&store);
    assert!(machines > 0, "the step lists machines");
    store.send(A::Move(-1), |s| s.cursor = machines - 1);
}
