//! Pinning and marking VIP: the two flags, where a new pin goes, the five
//! kinds of thing, and forgetting (NV-20, NV-25, NV-26).

use std::collections::HashSet;

use humata2_marks::{Mark, Marks, Thing};

fn open(dir: &tempfile::TempDir) -> Marks {
    Marks::open(&dir.path().join("marks.db")).unwrap()
}

fn session(id: &str) -> Thing {
    Thing::Session(id.to_owned())
}

fn pin(marks: &Marks, thing: &Thing) {
    marks.set(thing, Some(true), None).unwrap();
}

/// The pinned things, in order.
fn order(marks: &Marks) -> Vec<Thing> {
    let (pinned, _) = marks.pinned().unwrap();
    pinned.into_iter().map(|mark| mark.thing).collect()
}

fn vips(marks: &Marks) -> HashSet<Thing> {
    marks.vips().unwrap().into_iter().collect()
}

/// A thing's two flags: (pinned, vip).
fn flags(marks: &Marks, thing: &Thing) -> (bool, bool) {
    let mark = marks.get(thing).unwrap();
    (mark.pinned, mark.vip)
}

/// One thing of each kind, all with the same id.
fn five_kinds(id: &str) -> [Thing; 5] {
    [
        Thing::Session(id.to_owned()),
        Thing::Channel(id.to_owned()),
        Thing::Machine(id.to_owned()),
        Thing::PrReview(id.to_owned()),
        Thing::BoardItem(id.to_owned()),
    ]
}

#[test]
fn a_new_file_has_nothing_marked() {
    let dir = tempfile::tempdir().unwrap();
    let marks = open(&dir);
    assert_eq!(marks.pinned().unwrap(), (vec![], 0));
    assert_eq!(marks.vips().unwrap(), vec![]);
    let mark = marks.get(&session("se_1")).unwrap();
    assert_eq!(mark.thing, session("se_1"));
    assert_eq!((mark.pinned, mark.vip), (false, false));
}

#[test]
fn pinning_leaves_the_vip_mark_alone() {
    let dir = tempfile::tempdir().unwrap();
    let marks = open(&dir);
    let thing = session("se_1");

    let mark = marks.set(&thing, Some(true), None).unwrap();
    assert_eq!((mark.pinned, mark.vip), (true, false));

    marks.set(&thing, None, Some(true)).unwrap();
    let mark = marks.set(&thing, Some(false), None).unwrap();
    assert_eq!((mark.pinned, mark.vip), (false, true));
    assert_eq!(flags(&marks, &thing), (false, true));
}

#[test]
fn marking_vip_leaves_the_pin_alone() {
    let dir = tempfile::tempdir().unwrap();
    let marks = open(&dir);
    let thing = session("se_1");

    let mark = marks.set(&thing, None, Some(true)).unwrap();
    assert_eq!((mark.pinned, mark.vip), (false, true));

    marks.set(&thing, Some(true), None).unwrap();
    let mark = marks.set(&thing, None, Some(false)).unwrap();
    assert_eq!((mark.pinned, mark.vip), (true, false));
    assert_eq!(flags(&marks, &thing), (true, false));
}

#[test]
fn both_flags_can_be_set_in_one_call() {
    let dir = tempfile::tempdir().unwrap();
    let marks = open(&dir);
    let thing = session("se_1");

    let mark = marks.set(&thing, Some(true), Some(true)).unwrap();
    assert_eq!((mark.pinned, mark.vip), (true, true));
    assert_eq!(order(&marks), vec![thing.clone()]);
    assert_eq!(vips(&marks), HashSet::from([thing.clone()]));

    let mark = marks.set(&thing, Some(false), Some(false)).unwrap();
    assert_eq!((mark.pinned, mark.vip), (false, false));
    assert_eq!(order(&marks), vec![]);
    assert_eq!(vips(&marks), HashSet::new());
}

#[test]
fn set_with_neither_flag_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let marks = open(&dir);
    let thing = session("se_1");
    marks.set(&thing, Some(true), Some(true)).unwrap();

    let mark = marks.set(&thing, None, None).unwrap();
    assert_eq!((mark.pinned, mark.vip), (true, true));
    let mark = marks.set(&session("se_2"), None, None).unwrap();
    assert_eq!((mark.pinned, mark.vip), (false, false));
    assert_eq!(order(&marks), vec![thing]);
}

#[test]
fn set_returns_the_mark_that_get_then_reads() {
    let dir = tempfile::tempdir().unwrap();
    let marks = open(&dir);
    let thing = Thing::Channel("ch_1".to_owned());

    let set = marks.set(&thing, Some(true), Some(true)).unwrap();
    assert_eq!(set.thing, thing);
    assert_eq!(marks.get(&thing).unwrap(), set);
    assert_eq!(marks.pinned().unwrap(), (vec![set.clone()], set.revision));
}

#[test]
fn a_new_pin_goes_at_the_end() {
    let dir = tempfile::tempdir().unwrap();
    let marks = open(&dir);
    let machine = Thing::Machine("ma_1".to_owned());
    // Not in id order, so the order kept is the order pinned.
    pin(&marks, &session("se_2"));
    pin(&marks, &machine);
    pin(&marks, &session("se_1"));
    assert_eq!(order(&marks), vec![session("se_2"), machine, session("se_1")]);
}

#[test]
fn unpinning_takes_a_thing_out_of_the_order() {
    let dir = tempfile::tempdir().unwrap();
    let marks = open(&dir);
    for id in ["se_1", "se_2", "se_3"] {
        pin(&marks, &session(id));
    }
    let mark = marks.set(&session("se_2"), Some(false), None).unwrap();
    assert!(!mark.pinned);
    assert_eq!(order(&marks), vec![session("se_1"), session("se_3")]);
}

#[test]
fn pinning_again_after_unpinning_goes_to_the_end() {
    let dir = tempfile::tempdir().unwrap();
    let marks = open(&dir);
    for id in ["se_1", "se_2", "se_3"] {
        pin(&marks, &session(id));
    }
    marks.set(&session("se_1"), Some(false), None).unwrap();
    pin(&marks, &session("se_1"));
    assert_eq!(
        order(&marks),
        vec![session("se_2"), session("se_3"), session("se_1")]
    );
}

#[test]
fn pinning_a_pinned_thing_keeps_its_place() {
    let dir = tempfile::tempdir().unwrap();
    let marks = open(&dir);
    for id in ["se_1", "se_2", "se_3"] {
        pin(&marks, &session(id));
    }
    pin(&marks, &session("se_1"));
    marks.set(&session("se_2"), Some(true), Some(true)).unwrap();
    assert_eq!(
        order(&marks),
        vec![session("se_1"), session("se_2"), session("se_3")]
    );
}

#[test]
fn a_vip_change_keeps_a_pins_place() {
    let dir = tempfile::tempdir().unwrap();
    let marks = open(&dir);
    for id in ["se_1", "se_2", "se_3"] {
        pin(&marks, &session(id));
    }
    marks.set(&session("se_1"), None, Some(true)).unwrap();
    marks.set(&session("se_1"), None, Some(false)).unwrap();
    marks.set(&session("se_2"), None, Some(true)).unwrap();
    assert_eq!(
        order(&marks),
        vec![session("se_1"), session("se_2"), session("se_3")]
    );
}

#[test]
fn a_vip_thing_that_is_not_pinned_is_in_vips_only() {
    let dir = tempfile::tempdir().unwrap();
    let marks = open(&dir);
    let vip = Thing::PrReview("se_9".to_owned());
    marks.set(&vip, None, Some(true)).unwrap();
    pin(&marks, &session("se_1"));

    assert_eq!(order(&marks), vec![session("se_1")]);
    assert_eq!(vips(&marks), HashSet::from([vip]));
}

#[test]
fn pinned_gives_each_pins_vip_flag() {
    let dir = tempfile::tempdir().unwrap();
    let marks = open(&dir);
    pin(&marks, &session("se_1"));
    marks.set(&session("se_2"), Some(true), Some(true)).unwrap();

    let (pinned, revision) = marks.pinned().unwrap();
    assert_eq!(
        pinned,
        vec![
            Mark {
                thing: session("se_1"),
                pinned: true,
                vip: false,
                revision,
            },
            Mark {
                thing: session("se_2"),
                pinned: true,
                vip: true,
                revision,
            },
        ]
    );
}

#[test]
fn vips_lists_pinned_and_unpinned_vips_and_drops_a_cleared_one() {
    let dir = tempfile::tempdir().unwrap();
    let marks = open(&dir);
    let board_item = Thing::BoardItem("bi_1".to_owned());
    marks.set(&session("se_1"), Some(true), Some(true)).unwrap();
    marks.set(&board_item, None, Some(true)).unwrap();
    marks.set(&session("se_2"), None, Some(true)).unwrap();
    assert_eq!(
        vips(&marks),
        HashSet::from([session("se_1"), board_item.clone(), session("se_2")])
    );
    assert_eq!(marks.vips().unwrap().len(), 3);

    marks.set(&session("se_2"), None, Some(false)).unwrap();
    marks.set(&session("se_1"), None, Some(false)).unwrap();
    assert_eq!(vips(&marks), HashSet::from([board_item]));
    assert_eq!(order(&marks), vec![session("se_1")]);
}

#[test]
fn the_same_id_under_another_kind_is_another_thing() {
    let dir = tempfile::tempdir().unwrap();
    let marks = open(&dir);
    let session = Thing::Session("x".to_owned());
    let channel = Thing::Channel("x".to_owned());

    marks.set(&session, Some(true), None).unwrap();
    marks.set(&channel, None, Some(true)).unwrap();

    assert_eq!(flags(&marks, &session), (true, false));
    assert_eq!(flags(&marks, &channel), (false, true));
    assert_eq!(order(&marks), vec![session]);
    assert_eq!(vips(&marks), HashSet::from([channel]));
}

#[test]
fn marking_one_kind_marks_none_of_the_other_four() {
    for marked in five_kinds("x") {
        let dir = tempfile::tempdir().unwrap();
        let marks = open(&dir);
        marks.set(&marked, Some(true), Some(true)).unwrap();
        for thing in five_kinds("x") {
            let expected = thing == marked;
            assert_eq!(
                flags(&marks, &thing),
                (expected, expected),
                "{thing:?} after marking {marked:?}"
            );
        }
        assert_eq!(order(&marks), vec![marked.clone()]);
        assert_eq!(vips(&marks), HashSet::from([marked]));
    }
}

#[test]
fn all_five_kinds_with_one_id_are_pinned_side_by_side() {
    let dir = tempfile::tempdir().unwrap();
    let marks = open(&dir);
    let things = five_kinds("x");
    for thing in things.iter().rev() {
        pin(&marks, thing);
    }
    let pinned_last_first: Vec<Thing> = things.iter().rev().cloned().collect();
    assert_eq!(order(&marks), pinned_last_first);

    // Unpinning and forgetting reach one kind only.
    marks.set(&things[1], Some(false), None).unwrap();
    marks.forget(&things[3]).unwrap();
    assert_eq!(
        order(&marks),
        vec![things[4].clone(), things[2].clone(), things[0].clone()]
    );
}

#[test]
fn forgetting_clears_the_pin_and_the_vip_mark() {
    let dir = tempfile::tempdir().unwrap();
    let marks = open(&dir);
    let thing = session("se_1");
    marks.set(&thing, Some(true), Some(true)).unwrap();

    marks.forget(&thing).unwrap();

    assert_eq!(flags(&marks, &thing), (false, false));
    assert_eq!(order(&marks), vec![]);
    assert_eq!(vips(&marks), HashSet::new());
}

#[test]
fn forgetting_a_vip_that_is_not_pinned_clears_it() {
    let dir = tempfile::tempdir().unwrap();
    let marks = open(&dir);
    let thing = Thing::Machine("ma_1".to_owned());
    marks.set(&thing, None, Some(true)).unwrap();

    marks.forget(&thing).unwrap();

    assert_eq!(flags(&marks, &thing), (false, false));
    assert_eq!(vips(&marks), HashSet::new());
}

#[test]
fn forgetting_twice_or_a_thing_never_marked_is_fine() {
    let dir = tempfile::tempdir().unwrap();
    let marks = open(&dir);
    pin(&marks, &session("se_1"));
    pin(&marks, &session("se_2"));

    marks.forget(&session("se_2")).unwrap();
    marks.forget(&session("se_2")).unwrap();
    marks.forget(&Thing::BoardItem("never".to_owned())).unwrap();

    assert_eq!(order(&marks), vec![session("se_1")]);
}

#[test]
fn forgetting_one_pin_keeps_the_others_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let marks = open(&dir);
    for id in ["se_3", "se_1", "se_4", "se_2"] {
        pin(&marks, &session(id));
    }
    marks.set(&session("se_2"), None, Some(true)).unwrap();

    marks.forget(&session("se_1")).unwrap();

    assert_eq!(
        order(&marks),
        vec![session("se_3"), session("se_4"), session("se_2")]
    );
    assert_eq!(vips(&marks), HashSet::from([session("se_2")]));
}

#[test]
fn a_forgotten_thing_pinned_again_goes_to_the_end_without_its_old_vip_mark() {
    let dir = tempfile::tempdir().unwrap();
    let marks = open(&dir);
    marks.set(&session("se_1"), Some(true), Some(true)).unwrap();
    pin(&marks, &session("se_2"));
    marks.forget(&session("se_1")).unwrap();

    let mark = marks.set(&session("se_1"), Some(true), None).unwrap();

    assert_eq!((mark.pinned, mark.vip), (true, false));
    assert_eq!(order(&marks), vec![session("se_2"), session("se_1")]);
}

#[test]
fn clones_share_the_marks_across_threads() {
    let dir = tempfile::tempdir().unwrap();
    let marks = open(&dir);
    pin(&marks, &session("se_1"));

    let clone = marks.clone();
    std::thread::spawn(move || pin(&clone, &session("se_2")))
        .join()
        .unwrap();
    // A shared reference works from another thread too.
    std::thread::scope(|scope| {
        scope.spawn(|| pin(&marks, &session("se_3")));
    });

    assert_eq!(
        order(&marks),
        vec![session("se_1"), session("se_2"), session("se_3")]
    );
}
