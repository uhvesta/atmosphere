//! The revision, and what is still there after the file is closed and
//! opened again (NV-24: the marks are kept on the host).

use std::collections::HashSet;

use humata2_marks::{Marks, MarksError, Thing};

fn session(id: &str) -> Thing {
    Thing::Session(id.to_owned())
}

/// The pinned things, in order.
fn order(marks: &Marks) -> Vec<Thing> {
    let (pinned, _) = marks.pinned().unwrap();
    pinned.into_iter().map(|mark| mark.thing).collect()
}

fn revision(marks: &Marks) -> u64 {
    marks.pinned().unwrap().1
}

#[test]
fn every_change_raises_the_revision() {
    let dir = tempfile::tempdir().unwrap();
    let marks = Marks::open(&dir.path().join("marks.db")).unwrap();
    let mut last = revision(&marks);
    let mut raised = |step: &str| {
        let now = revision(&marks);
        assert!(now > last, "{step}: revision {now} after {last}");
        last = now;
    };

    marks.set(&session("se_1"), Some(true), None).unwrap();
    raised("pin");
    marks.set(&session("se_2"), Some(true), None).unwrap();
    raised("second pin");
    marks.set(&session("se_1"), None, Some(true)).unwrap();
    raised("mark a pin VIP");
    marks.set(&Thing::Machine("ma_1".to_owned()), None, Some(true)).unwrap();
    raised("mark an unpinned thing VIP");
    marks
        .reorder(&[session("se_2"), session("se_1")], None)
        .unwrap();
    raised("reorder");
    marks.set(&session("se_1"), None, Some(false)).unwrap();
    raised("clear VIP");
    marks.set(&session("se_2"), Some(false), None).unwrap();
    raised("unpin");
    marks.forget(&session("se_1")).unwrap();
    raised("forget a pin");
    marks.forget(&Thing::Machine("ma_1".to_owned())).unwrap();
    raised("forget an unpinned VIP");
}

#[test]
fn a_call_that_changes_nothing_keeps_the_revision() {
    let dir = tempfile::tempdir().unwrap();
    let marks = Marks::open(&dir.path().join("marks.db")).unwrap();
    marks.set(&session("se_1"), Some(true), Some(true)).unwrap();
    let before = revision(&marks);

    marks.set(&session("se_1"), Some(true), Some(true)).unwrap();
    marks.set(&session("se_1"), Some(true), None).unwrap();
    marks.set(&session("se_1"), None, None).unwrap();
    marks.set(&session("se_2"), Some(false), Some(false)).unwrap();
    marks.forget(&session("se_2")).unwrap();
    marks.get(&session("se_1")).unwrap();
    marks.vips().unwrap();

    assert_eq!(revision(&marks), before);
}

#[test]
fn a_refused_reorder_keeps_the_revision() {
    let dir = tempfile::tempdir().unwrap();
    let marks = Marks::open(&dir.path().join("marks.db")).unwrap();
    marks.set(&session("se_1"), Some(true), None).unwrap();
    marks.set(&session("se_2"), Some(true), None).unwrap();
    let before = revision(&marks);

    marks.reorder(&[session("se_1")], None).unwrap_err();
    marks
        .reorder(&[session("se_2"), session("se_1")], Some(before + 7))
        .unwrap_err();

    assert_eq!(revision(&marks), before);
}

#[test]
fn a_mark_carries_the_revision_of_the_write_that_returned_it() {
    let dir = tempfile::tempdir().unwrap();
    let marks = Marks::open(&dir.path().join("marks.db")).unwrap();

    let first = marks.set(&session("se_1"), Some(true), None).unwrap();
    assert_eq!(first.revision, revision(&marks));
    let second = marks.set(&session("se_2"), None, Some(true)).unwrap();
    assert_eq!(second.revision, revision(&marks));
    assert!(second.revision > first.revision);

    // A read gives the revision now, for marked and unmarked things alike.
    assert_eq!(marks.get(&session("se_1")).unwrap().revision, second.revision);
    assert_eq!(marks.get(&session("se_9")).unwrap().revision, second.revision);
    let (_, after_reorder) = marks.reorder(&[session("se_1")], None).unwrap();
    assert_eq!(after_reorder, revision(&marks));
}

#[test]
fn the_revision_survives_reopening_and_goes_on_from_there() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("marks.db");
    let before = {
        let marks = Marks::open(&path).unwrap();
        marks.set(&session("se_1"), Some(true), None).unwrap();
        marks.set(&session("se_2"), Some(true), Some(true)).unwrap();
        marks.forget(&session("se_1")).unwrap();
        revision(&marks)
    };
    assert!(before > 0);

    let marks = Marks::open(&path).unwrap();
    assert_eq!(revision(&marks), before);
    assert_eq!(marks.get(&session("se_2")).unwrap().revision, before);

    marks.set(&session("se_3"), Some(true), None).unwrap();
    assert!(revision(&marks) > before);
}

#[test]
fn the_revision_of_a_file_emptied_again_is_not_back_at_the_start() {
    // Everything was unmarked, so no thing is kept; the count of changes is.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("marks.db");
    let before = {
        let marks = Marks::open(&path).unwrap();
        marks.set(&session("se_1"), Some(true), Some(true)).unwrap();
        marks.set(&session("se_1"), Some(false), Some(false)).unwrap();
        revision(&marks)
    };

    let marks = Marks::open(&path).unwrap();
    assert_eq!(marks.pinned().unwrap(), (vec![], before));
    assert!(before > 0);
}

#[test]
fn marks_and_their_order_survive_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("marks.db");
    let channel = Thing::Channel("ch_1".to_owned());
    let machine = Thing::Machine("ma_1".to_owned());
    let review = Thing::PrReview("se_7".to_owned());
    let board_item = Thing::BoardItem("bi_1".to_owned());
    let before = {
        let marks = Marks::open(&path).unwrap();
        marks.set(&session("se_1"), Some(true), None).unwrap();
        marks.set(&channel, Some(true), Some(true)).unwrap();
        marks.set(&machine, Some(true), None).unwrap();
        marks.set(&review, Some(true), None).unwrap();
        marks.set(&board_item, None, Some(true)).unwrap();
        // An order that is neither the order pinned nor sorted.
        marks
            .reorder(
                &[review.clone(), session("se_1"), machine.clone(), channel.clone()],
                None,
            )
            .unwrap();
        // And one unpinned, so the order kept has a gap behind it.
        marks.set(&machine, Some(false), None).unwrap();
        marks.pinned().unwrap()
    };

    let marks = Marks::open(&path).unwrap();

    assert_eq!(marks.pinned().unwrap(), before);
    assert_eq!(
        order(&marks),
        vec![review, session("se_1"), channel.clone()]
    );
    let vips: HashSet<Thing> = marks.vips().unwrap().into_iter().collect();
    assert_eq!(vips, HashSet::from([channel, board_item.clone()]));
    let unpinned_vip = marks.get(&board_item).unwrap();
    assert_eq!((unpinned_vip.pinned, unpinned_vip.vip), (false, true));
    let unpinned = marks.get(&machine).unwrap();
    assert_eq!((unpinned.pinned, unpinned.vip), (false, false));
}

#[test]
fn a_pin_added_after_reopening_goes_at_the_end() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("marks.db");
    {
        let marks = Marks::open(&path).unwrap();
        for id in ["se_1", "se_2", "se_3"] {
            marks.set(&session(id), Some(true), None).unwrap();
        }
        marks
            .reorder(&[session("se_3"), session("se_2"), session("se_1")], None)
            .unwrap();
        marks.set(&session("se_1"), Some(false), None).unwrap();
    }

    let marks = Marks::open(&path).unwrap();
    marks.set(&session("se_0"), Some(true), None).unwrap();

    assert_eq!(
        order(&marks),
        vec![session("se_3"), session("se_2"), session("se_0")]
    );
}

#[test]
fn a_forgotten_thing_stays_forgotten_after_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("marks.db");
    {
        let marks = Marks::open(&path).unwrap();
        marks.set(&session("se_1"), Some(true), Some(true)).unwrap();
        marks.set(&session("se_2"), Some(true), None).unwrap();
        marks.forget(&session("se_1")).unwrap();
    }

    let marks = Marks::open(&path).unwrap();

    assert_eq!(order(&marks), vec![session("se_2")]);
    assert_eq!(marks.vips().unwrap(), vec![]);
    let forgotten = marks.get(&session("se_1")).unwrap();
    assert_eq!((forgotten.pinned, forgotten.vip), (false, false));
}

#[test]
fn opening_twice_without_a_change_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("marks.db");
    drop(Marks::open(&path).unwrap());
    let marks = Marks::open(&path).unwrap();
    assert_eq!(marks.pinned().unwrap(), (vec![], 0));
}

#[test]
fn a_file_that_is_not_a_marks_database_is_refused_and_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("marks.db");
    let contents = b"these are somebody's notes, not a database\n".repeat(40);
    std::fs::write(&path, &contents).unwrap();

    let error = Marks::open(&path).err();

    assert!(
        matches!(error, Some(MarksError::Database { .. })),
        "{error:?}"
    );
    assert_eq!(std::fs::read(&path).unwrap(), contents);
}
