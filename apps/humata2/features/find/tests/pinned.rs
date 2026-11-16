//! NV-20, NV-21, NV-24, NV-25: the Pinned section follows the host's pins,
//! in the person's order, whichever client changes them.
use std::sync::Arc;

use humata2_clients::DaemonError;
use humata2_clients::marks::{FakeMarks, MarksClient, PinnedList, Thing};
use humata2_clients::model::{ChannelId, MachineId, SessionId};
use humata2_find::{PinnedAction as A, PinnedDelegate, PinnedSection, PinnedState};
use tca::TestStore;
use tokio::runtime::Builder;

fn block_on<T>(future: impl Future<Output = T>) -> T {
    match Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime.block_on(future),
        Err(error) => panic!("a runtime starts: {error}"),
    }
}

fn set(marks: &FakeMarks, thing: &Thing, pinned: Option<bool>, vip: Option<bool>) -> PinnedList {
    match block_on(marks.set(thing, pinned, vip)) {
        Ok(list) => list,
        Err(error) => panic!("set works: {error}"),
    }
}

/// The pins as the host has them now.
fn host(marks: &FakeMarks) -> PinnedList {
    match block_on(marks.pinned()) {
        Ok(list) => list,
        Err(error) => panic!("pinned works: {error}"),
    }
}

fn things(list: &PinnedList) -> Vec<Thing> {
    list.items.iter().map(|item| item.thing.clone()).collect()
}

fn vips(list: &PinnedList) -> Vec<bool> {
    list.items.iter().map(|item| item.vip).collect()
}

fn board_item() -> Thing {
    Thing::BoardItem("item-7".to_owned())
}

/// The host's marks with a session, a channel and a machine pinned, in
/// that order.
fn three() -> (FakeMarks, [Thing; 3]) {
    let marks = FakeMarks::new();
    let all = [
        Thing::Session(SessionId::new("s-1")),
        Thing::Channel(ChannelId::new("billing")),
        Thing::Machine(MachineId::new("mac-studio")),
    ];
    marks.set_title(&all[0], "Ledger rollup");
    marks.set_title(&all[1], "Billing");
    marks.set_title(&all[2], "Mac Studio");
    for thing in &all {
        set(&marks, thing, Some(true), None);
    }
    (marks, all)
}

fn reducer(marks: &FakeMarks) -> PinnedSection {
    PinnedSection::new(Arc::new(marks.clone()))
}

/// A store that has the host's pins already, and doesn't follow changes.
fn loaded(marks: &FakeMarks) -> TestStore<PinnedSection> {
    let state = PinnedState {
        list: host(marks),
        ..PinnedState::default()
    };
    TestStore::new(state, reducer(marks))
}

fn refused() -> DaemonError {
    DaemonError::Refused {
        what: "This pin".to_owned(),
        reason: "the test said so".to_owned(),
    }
}

#[test]
fn start_loads_the_pins_then_follows_another_clients_change() {
    let (marks, [a, b, c]) = three();
    let pins = host(&marks);
    let mut store = TestStore::new(PinnedState::default(), reducer(&marks));

    store.send(A::Start, |_| {});
    store.receive(A::Loaded(Ok(pins.clone())), |s| s.list = pins.clone());
    assert_eq!(things(&store.state().list), [a.clone(), b.clone(), c.clone()]);
    assert_eq!(store.state().list.items[0].title, "Ledger rollup");

    // Another client of the same host, such as `humata pinned`, pins one.
    store.settle();
    let cli = marks.clone();
    let changed = set(&cli, &board_item(), Some(true), None);
    store.receive(A::Changed(changed.clone()), |s| s.list = changed.clone());
    assert_eq!(things(&store.state().list), [a.clone(), b, c.clone(), board_item()]);

    // And unpins another.
    let changed = set(&cli, &things(&changed)[1], Some(false), None);
    store.receive(A::Changed(changed.clone()), |s| s.list = changed.clone());
    assert_eq!(things(&store.state().list), [a, c, board_item()]);

    // The section follows the host until it goes away.
    store.skip_in_flight();
    store.finish();
}

#[test]
fn pinning_adds_at_the_end() {
    let (marks, [a, b, c]) = three();
    let mut store = loaded(&marks);

    store.send(
        A::Toggle {
            thing: board_item(),
            pin: true,
        },
        |_| {},
    );
    let pins = host(&marks);
    store.receive(A::Loaded(Ok(pins.clone())), |s| s.list = pins.clone());
    assert_eq!(things(&store.state().list), [a, b, c, board_item()]);
    store.finish();
}

#[test]
fn unpinning_takes_it_out() {
    let (marks, [a, b, c]) = three();
    let mut store = loaded(&marks);

    store.send(A::Toggle { thing: b, pin: false }, |_| {});
    let pins = host(&marks);
    store.receive(A::Loaded(Ok(pins.clone())), |s| s.list = pins.clone());
    assert_eq!(things(&store.state().list), [a, c]);
    store.finish();
}

#[test]
fn vip_goes_on_and_off() {
    let (marks, [_, b, _]) = three();
    let mut store = loaded(&marks);

    store.send(
        A::Vip {
            thing: b.clone(),
            on: true,
        },
        |_| {},
    );
    let pins = host(&marks);
    store.receive(A::Loaded(Ok(pins.clone())), |s| s.list = pins.clone());
    assert_eq!(vips(&store.state().list), [false, true, false]);

    store.send(A::Vip { thing: b, on: false }, |_| {});
    let pins = host(&marks);
    store.receive(A::Loaded(Ok(pins.clone())), |s| s.list = pins.clone());
    assert_eq!(vips(&store.state().list), [false, false, false]);
    store.finish();
}

#[test]
fn a_move_reorders_at_once_and_the_host_keeps_the_order() {
    let (marks, [a, b, c]) = three();
    let mut store = loaded(&marks);
    let before = store.state().list.clone();

    // The first pin goes last: the list shows it before the host answers.
    store.send(A::Move { from: 0, to: 2 }, |s| {
        s.list.items = vec![
            before.items[1].clone(),
            before.items[2].clone(),
            before.items[0].clone(),
        ];
    });
    let pins = host(&marks);
    assert_eq!(things(&pins), [b.clone(), c.clone(), a.clone()]);
    store.receive(A::Loaded(Ok(pins.clone())), |s| s.list = pins.clone());

    // And back to the front.
    store.send(A::Move { from: 2, to: 0 }, |s| {
        s.list.items = vec![
            pins.items[2].clone(),
            pins.items[0].clone(),
            pins.items[1].clone(),
        ];
    });
    let pins = host(&marks);
    assert_eq!(things(&pins), [a, b, c]);
    store.receive(A::Loaded(Ok(pins.clone())), |s| s.list = pins.clone());
    store.finish();
}

#[test]
fn a_move_made_from_a_stale_list_reloads_to_the_hosts_order() {
    let (marks, [a, b, c]) = three();
    let mut store = loaded(&marks).non_exhaustive();
    // Another client changes the pins; this one hasn't heard yet.
    let cli = marks.clone();
    let pins = set(&cli, &a, None, Some(true));

    store.send(A::Move { from: 0, to: 2 }, |_| {});
    assert_eq!(things(&store.state().list), [b.clone(), c.clone(), a.clone()]);

    store.receive_matching("Loaded(Ok(_))", |action| matches!(action, A::Loaded(Ok(_))), |_| {});
    assert_eq!(things(&store.state().list), [a, b, c]);
    assert_eq!(store.state().list, pins);
    assert!(store.state().error.is_none());
    // The host kept its order.
    assert_eq!(host(&marks), pins);
}

#[test]
fn a_move_the_host_fails_goes_back_to_the_hosts_order() {
    let (marks, [a, b, c]) = three();
    let mut store = loaded(&marks).non_exhaustive();
    let pins = host(&marks);

    marks.fail_next(refused());
    store.send(A::Move { from: 2, to: 0 }, |_| {});
    assert_eq!(things(&store.state().list), [c.clone(), a.clone(), b.clone()]);

    store.receive_matching("Loaded(Ok(_))", |action| matches!(action, A::Loaded(Ok(_))), |_| {});
    assert_eq!(things(&store.state().list), [a, b, c]);
    assert_eq!(store.state().list, pins);
}

#[test]
fn a_move_to_where_it_is_or_past_the_end_asks_nothing() {
    let (marks, _) = three();
    let mut store = loaded(&marks);
    let asked = marks.calls().len();

    store.send(A::Move { from: 1, to: 1 }, |_| {});
    store.send(A::Move { from: 0, to: 3 }, |_| {});
    store.send(A::Move { from: 3, to: 0 }, |_| {});
    store.finish();
    assert_eq!(marks.calls().len(), asked);
}

#[test]
fn an_older_list_never_replaces_a_newer_one() {
    let (marks, _) = three();
    let mut store = loaded(&marks);
    let older = store.state().list.clone();
    let cli = marks.clone();
    let newer = set(&cli, &board_item(), Some(true), None);
    assert!(newer.revision > older.revision);

    store.send(A::Changed(newer.clone()), |s| s.list = newer.clone());
    // A reply that was on its way before the change arrives after it.
    store.send(A::Loaded(Ok(older.clone())), |_| {});
    store.send(A::Changed(older), |_| {});
    store.finish();
}

#[test]
fn a_failed_change_shows_and_the_next_list_clears_it() {
    let (marks, [a, b, c]) = three();
    let mut store = loaded(&marks);

    marks.fail_next(refused());
    store.send(
        A::Toggle {
            thing: board_item(),
            pin: true,
        },
        |_| {},
    );
    store.receive(A::Loaded(Err(refused())), |s| {
        s.error = Some(refused().to_string());
    });
    assert_eq!(things(&store.state().list), [a.clone(), b.clone(), c.clone()]);

    store.send(
        A::Toggle {
            thing: board_item(),
            pin: true,
        },
        |_| {},
    );
    let pins = host(&marks);
    store.receive(A::Loaded(Ok(pins.clone())), |s| {
        s.list = pins.clone();
        s.error = None;
    });
    assert_eq!(things(&store.state().list), [a, b, c, board_item()]);
    store.finish();
}

#[test]
fn picking_a_pin_opens_it() {
    let (marks, [_, b, _]) = three();
    let mut store = loaded(&marks);

    store.send(A::Open(1), |_| {});
    store.receive(A::Delegate(PinnedDelegate::Open(b)), |_| {});
    // Past the end, nothing.
    store.send(A::Open(3), |_| {});
    store.finish();
}
