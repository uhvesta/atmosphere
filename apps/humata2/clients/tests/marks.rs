//! The scripted marks keep pins and VIP marks the way the host does
//! (NV-20, NV-24, NV-25): one list for every client, in the person's order.

use humata2_clients::DaemonError;
use humata2_clients::marks::{FakeMarks, MarksClient, PinnedList, Thing};
use humata2_clients::model::{ChannelId, MachineId, SessionId};
use tokio::runtime::Builder;

fn block_on<T>(future: impl Future<Output = T>) -> T {
    match Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime.block_on(future),
        Err(error) => panic!("a runtime starts: {error}"),
    }
}

fn session(id: &str) -> Thing {
    Thing::Session(SessionId::new(id))
}

fn channel(id: &str) -> Thing {
    Thing::Channel(ChannelId::new(id))
}

fn machine(id: &str) -> Thing {
    Thing::Machine(MachineId::new(id))
}

fn set(marks: &FakeMarks, thing: &Thing, pinned: Option<bool>, vip: Option<bool>) -> PinnedList {
    match block_on(marks.set(thing, pinned, vip)) {
        Ok(list) => list,
        Err(error) => panic!("set works: {error}"),
    }
}

fn pin(marks: &FakeMarks, thing: &Thing) -> PinnedList {
    set(marks, thing, Some(true), None)
}

fn pinned(marks: &FakeMarks) -> PinnedList {
    match block_on(marks.pinned()) {
        Ok(list) => list,
        Err(error) => panic!("pinned works: {error}"),
    }
}

fn things(list: &PinnedList) -> Vec<Thing> {
    list.items.iter().map(|item| item.thing.clone()).collect()
}

/// Marks with a session, a channel and a machine pinned, in that order.
fn three() -> (FakeMarks, [Thing; 3]) {
    let marks = FakeMarks::new();
    let all = [session("s-1"), channel("billing"), machine("mac-studio")];
    for thing in &all {
        pin(&marks, thing);
    }
    (marks, all)
}

fn refused() -> DaemonError {
    DaemonError::Refused {
        what: "This pin".to_owned(),
        reason: "the test said so".to_owned(),
    }
}

#[test]
fn nothing_is_pinned_at_first() {
    let marks = FakeMarks::new();
    assert!(pinned(&marks).items.is_empty());
}

#[test]
fn a_new_pin_goes_at_the_end() {
    let (marks, [a, b, c]) = three();
    assert_eq!(things(&pinned(&marks)), [a.clone(), b.clone(), c.clone()]);

    let d = Thing::BoardItem("item-7".to_owned());
    let list = pin(&marks, &d);
    assert_eq!(things(&list), [a, b, c, d]);
    assert_eq!(pinned(&marks), list);
}

#[test]
fn unpinning_takes_it_out_and_keeps_the_others_in_order() {
    let (marks, [a, b, c]) = three();
    let list = set(&marks, &b, Some(false), None);
    assert_eq!(things(&list), [a.clone(), c.clone()]);

    // Pinned again, it is new: it goes at the end.
    assert_eq!(things(&pin(&marks, &b)), [a, c, b]);
}

#[test]
fn vip_is_set_apart_from_the_pin() {
    let (marks, [a, b, c]) = three();
    let list = set(&marks, &b, None, Some(true));
    assert_eq!(things(&list), [a, b, c]);
    let vips: Vec<bool> = list.items.iter().map(|item| item.vip).collect();
    assert_eq!(vips, [false, true, false]);

    let list = set(&marks, &things(&list)[1], None, Some(false));
    assert!(list.items.iter().all(|item| !item.vip));
}

#[test]
fn a_mark_left_alone_stays_as_it_was() {
    let marks = FakeMarks::new();
    let thing = Thing::PrReview(SessionId::new("review-12"));
    // VIP before it is pinned: not in the list, but remembered.
    assert!(set(&marks, &thing, None, Some(true)).items.is_empty());
    let list = pin(&marks, &thing);
    assert_eq!(things(&list), [thing.clone()]);
    assert!(list.items[0].vip);

    // Unpinned and pinned again, it is still VIP.
    set(&marks, &thing, Some(false), None);
    assert!(pin(&marks, &thing).items[0].vip);
}

#[test]
fn every_change_raises_the_revision() {
    let marks = FakeMarks::new();
    let a = session("s-1");
    let start = pinned(&marks).revision;
    let after_pin = pin(&marks, &a).revision;
    assert!(after_pin > start);
    let after_vip = set(&marks, &a, None, Some(true)).revision;
    assert!(after_vip > after_pin);
    let after_unpin = set(&marks, &a, Some(false), None).revision;
    assert!(after_unpin > after_vip);
    // Reading changes nothing.
    assert_eq!(pinned(&marks).revision, after_unpin);
}

#[test]
fn a_pin_shows_its_title_and_event() {
    let marks = FakeMarks::new();
    let a = session("s-1");
    marks.set_title(&a, "Ledger rollup");
    marks.set_event(&a, "needs input");
    let list = pin(&marks, &a);
    assert_eq!(list.items[0].title, "Ledger rollup");
    assert_eq!(list.items[0].event, "needs input");

    marks.set_event(&a, "");
    assert_eq!(pinned(&marks).items[0].event, "");
}

#[test]
fn a_pinned_things_new_event_raises_the_revision_and_is_watched() {
    let marks = FakeMarks::new();
    let a = session("s-1");
    let b = session("s-2");
    pin(&marks, &a);
    let before = pinned(&marks).revision;
    let mut changes = match block_on(marks.watch()) {
        Ok(changes) => changes,
        Err(error) => panic!("watch works: {error}"),
    };

    marks.set_event(&a, "finished");

    let told = changes.try_recv().ok();
    let now = pinned(&marks);
    assert!(now.revision > before, "revision {} after {before}", now.revision);
    assert_eq!(told, Some(now.clone()));
    assert_eq!(now.items[0].event, "finished");

    // The same event again, or one on a thing that isn't pinned, is no change.
    marks.set_event(&a, "finished");
    marks.set_event(&b, "needs input");
    assert!(changes.try_recv().is_err());
    assert_eq!(pinned(&marks).revision, now.revision);
}

#[test]
fn clones_share_the_marks() {
    let app = FakeMarks::new();
    let cli = app.clone();
    let a = session("s-1");
    let list = pin(&cli, &a);
    assert_eq!(pinned(&app), list);
    assert_eq!(things(&pinned(&app)), [a]);
}

#[test]
fn watch_delivers_another_clients_change() {
    let app = FakeMarks::new();
    let cli = app.clone();
    let mut changes = match block_on(app.watch()) {
        Ok(changes) => changes,
        Err(error) => panic!("watch works: {error}"),
    };

    let pinned = pin(&cli, &session("s-1"));
    assert_eq!(changes.try_recv().ok(), Some(pinned));

    let vip = set(&cli, &session("s-1"), None, Some(true));
    assert_eq!(changes.try_recv().ok(), Some(vip));
    assert!(changes.try_recv().is_err(), "one list a change");
}

#[test]
fn watch_delivers_this_clients_own_changes_and_reorders_too() {
    let (marks, [a, b, c]) = three();
    let mut changes = match block_on(marks.watch()) {
        Ok(changes) => changes,
        Err(error) => panic!("watch works: {error}"),
    };
    // What happened before the watch isn't told again.
    assert!(changes.try_recv().is_err());

    let revision = pinned(&marks).revision;
    let order = [c, a, b];
    let list = match block_on(marks.reorder(&order, revision)) {
        Ok(list) => list,
        Err(error) => panic!("reorder works: {error}"),
    };
    assert_eq!(changes.try_recv().ok(), Some(list));
}

#[test]
fn reorder_puts_the_pins_in_the_order_given() {
    let (marks, [a, b, c]) = three();
    let before = pinned(&marks);
    let order = [c, a, b];
    let list = match block_on(marks.reorder(&order, before.revision)) {
        Ok(list) => list,
        Err(error) => panic!("reorder works: {error}"),
    };
    assert_eq!(things(&list), order);
    assert!(list.revision > before.revision);
    assert_eq!(pinned(&marks), list);
}

#[test]
fn a_reorder_made_from_an_older_list_is_refused_and_changes_nothing() {
    let (app, [a, b, c]) = three();
    let cli = app.clone();
    let seen = pinned(&app);
    // The other client changes a mark after this one read the list.
    let host = set(&cli, &a, None, Some(true));

    let result = block_on(app.reorder(&[c, b, a], seen.revision));
    assert!(result.is_err());
    assert_eq!(pinned(&app), host);
}

#[test]
fn a_reorder_must_name_each_pin_once() {
    let (marks, [a, b, c]) = three();
    let before = pinned(&marks);
    let revision = before.revision;

    let missing = [a.clone(), b.clone()];
    assert!(block_on(marks.reorder(&missing, revision)).is_err());
    let twice = [a.clone(), b.clone(), b.clone()];
    assert!(block_on(marks.reorder(&twice, revision)).is_err());
    let stranger = [a, b, c, session("not-pinned")];
    assert!(block_on(marks.reorder(&stranger, revision)).is_err());
    assert_eq!(pinned(&marks), before);
}

#[test]
fn fail_next_fails_one_call_and_changes_nothing() {
    let (marks, [a, _, _]) = three();
    let before = pinned(&marks);
    marks.fail_next(refused());
    assert_eq!(block_on(marks.set(&a, Some(false), None)), Err(refused()));
    assert_eq!(pinned(&marks), before);
}
