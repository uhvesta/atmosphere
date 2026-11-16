//! Reordering the pinned things (NV-25): the order must be exactly the
//! pinned set, and a caller that read an older list is told so.

use humata2_marks::{Marks, MarksError, Thing};

fn open(dir: &tempfile::TempDir) -> Marks {
    Marks::open(&dir.path().join("marks.db")).unwrap()
}

fn session(id: &str) -> Thing {
    Thing::Session(id.to_owned())
}

/// A file with these sessions pinned, in this order.
fn pinned(dir: &tempfile::TempDir, ids: &[&str]) -> Marks {
    let marks = open(dir);
    for id in ids {
        marks.set(&session(id), Some(true), None).unwrap();
    }
    marks
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
fn reorder_puts_the_pins_in_the_order_given() {
    let dir = tempfile::tempdir().unwrap();
    let marks = pinned(&dir, &["se_1", "se_2", "se_3", "se_4"]);
    let new_order = vec![session("se_3"), session("se_1"), session("se_4"), session("se_2")];

    marks.reorder(&new_order, None).unwrap();

    assert_eq!(order(&marks), new_order);
}

#[test]
fn reorder_returns_the_list_that_pinned_then_reads() {
    let dir = tempfile::tempdir().unwrap();
    let marks = pinned(&dir, &["se_1", "se_2", "se_3"]);
    marks.set(&session("se_2"), None, Some(true)).unwrap();

    let returned = marks
        .reorder(&[session("se_3"), session("se_2"), session("se_1")], None)
        .unwrap();

    assert_eq!(returned, marks.pinned().unwrap());
    let vip: Vec<bool> = returned.0.iter().map(|mark| mark.vip).collect();
    assert_eq!(vip, vec![false, true, false]);
}

#[test]
fn reorder_keeps_kinds_apart() {
    let dir = tempfile::tempdir().unwrap();
    let marks = open(&dir);
    let things = [
        Thing::Session("x".to_owned()),
        Thing::Channel("x".to_owned()),
        Thing::Machine("x".to_owned()),
        Thing::PrReview("x".to_owned()),
        Thing::BoardItem("x".to_owned()),
    ];
    for thing in &things {
        marks.set(thing, Some(true), None).unwrap();
    }
    let new_order = vec![
        things[3].clone(),
        things[0].clone(),
        things[4].clone(),
        things[1].clone(),
        things[2].clone(),
    ];

    marks.reorder(&new_order, None).unwrap();

    assert_eq!(order(&marks), new_order);
}

#[test]
fn a_pin_added_after_a_reorder_goes_at_the_end() {
    let dir = tempfile::tempdir().unwrap();
    let marks = pinned(&dir, &["se_1", "se_2", "se_3"]);
    marks
        .reorder(&[session("se_3"), session("se_1"), session("se_2")], None)
        .unwrap();

    marks.set(&session("se_0"), Some(true), None).unwrap();

    assert_eq!(
        order(&marks),
        vec![session("se_3"), session("se_1"), session("se_2"), session("se_0")]
    );
}

#[test]
fn reordering_nothing_when_nothing_is_pinned_is_fine() {
    let dir = tempfile::tempdir().unwrap();
    let marks = open(&dir);
    marks.set(&session("se_vip"), None, Some(true)).unwrap();

    let (list, _) = marks.reorder(&[], None).unwrap();

    assert_eq!(list, vec![]);
}

#[test]
fn an_order_that_leaves_a_pin_out_is_a_mismatch() {
    let dir = tempfile::tempdir().unwrap();
    let marks = pinned(&dir, &["se_1", "se_2", "se_3"]);
    let before = marks.pinned().unwrap();

    let error = marks
        .reorder(&[session("se_3"), session("se_1")], None)
        .unwrap_err();

    match error {
        MarksError::OrderMismatch {
            missing,
            not_pinned,
            repeated,
        } => {
            assert_eq!(missing, vec![session("se_2")]);
            assert_eq!(not_pinned, vec![]);
            assert_eq!(repeated, vec![]);
        }
        other => panic!("expected OrderMismatch, got {other:?}"),
    }
    assert_eq!(marks.pinned().unwrap(), before);
}

#[test]
fn an_order_with_a_thing_that_is_not_pinned_is_a_mismatch() {
    let dir = tempfile::tempdir().unwrap();
    let marks = pinned(&dir, &["se_1", "se_2"]);
    let before = marks.pinned().unwrap();

    let error = marks
        .reorder(&[session("se_2"), session("se_9"), session("se_1")], None)
        .unwrap_err();

    match error {
        MarksError::OrderMismatch {
            missing,
            not_pinned,
            repeated,
        } => {
            assert_eq!(missing, vec![]);
            assert_eq!(not_pinned, vec![session("se_9")]);
            assert_eq!(repeated, vec![]);
        }
        other => panic!("expected OrderMismatch, got {other:?}"),
    }
    assert_eq!(marks.pinned().unwrap(), before);
}

#[test]
fn a_vip_that_is_not_pinned_has_no_place_in_the_order() {
    let dir = tempfile::tempdir().unwrap();
    let marks = pinned(&dir, &["se_1", "se_2"]);
    marks.set(&session("se_vip"), None, Some(true)).unwrap();

    let error = marks
        .reorder(&[session("se_vip"), session("se_2"), session("se_1")], None)
        .unwrap_err();

    assert!(
        matches!(&error, MarksError::OrderMismatch { not_pinned, .. } if *not_pinned == [session("se_vip")]),
        "{error:?}"
    );
    assert_eq!(order(&marks), vec![session("se_1"), session("se_2")]);
}

#[test]
fn the_same_id_under_another_kind_does_not_stand_in_for_a_pin() {
    let dir = tempfile::tempdir().unwrap();
    let marks = pinned(&dir, &["x", "se_2"]);
    let channel = Thing::Channel("x".to_owned());

    let error = marks
        .reorder(&[session("se_2"), channel.clone()], None)
        .unwrap_err();

    match error {
        MarksError::OrderMismatch {
            missing,
            not_pinned,
            ..
        } => {
            assert_eq!(missing, vec![session("x")]);
            assert_eq!(not_pinned, vec![channel]);
        }
        other => panic!("expected OrderMismatch, got {other:?}"),
    }
    assert_eq!(order(&marks), vec![session("x"), session("se_2")]);
}

#[test]
fn an_order_that_lists_a_pin_twice_is_a_mismatch() {
    let dir = tempfile::tempdir().unwrap();
    let marks = pinned(&dir, &["se_1", "se_2"]);
    let before = marks.pinned().unwrap();

    let error = marks
        .reorder(&[session("se_2"), session("se_1"), session("se_2")], None)
        .unwrap_err();

    match error {
        MarksError::OrderMismatch {
            missing,
            not_pinned,
            repeated,
        } => {
            assert_eq!(missing, vec![]);
            assert_eq!(not_pinned, vec![]);
            assert_eq!(repeated, vec![session("se_2")]);
        }
        other => panic!("expected OrderMismatch, got {other:?}"),
    }
    assert_eq!(marks.pinned().unwrap(), before);
}

#[test]
fn a_pin_listed_twice_in_place_of_another_is_a_mismatch() {
    // As long as the pinned set, so only looking at each entry shows it.
    let dir = tempfile::tempdir().unwrap();
    let marks = pinned(&dir, &["se_1", "se_2", "se_3"]);
    let before = marks.pinned().unwrap();

    let error = marks
        .reorder(&[session("se_3"), session("se_1"), session("se_3")], None)
        .unwrap_err();

    match error {
        MarksError::OrderMismatch {
            missing, repeated, ..
        } => {
            assert_eq!(missing, vec![session("se_2")]);
            assert_eq!(repeated, vec![session("se_3")]);
        }
        other => panic!("expected OrderMismatch, got {other:?}"),
    }
    assert_eq!(marks.pinned().unwrap(), before);
}

#[test]
fn reorder_with_the_revision_just_read_goes_through() {
    let dir = tempfile::tempdir().unwrap();
    let marks = pinned(&dir, &["se_1", "se_2"]);
    let seen = revision(&marks);

    marks
        .reorder(&[session("se_2"), session("se_1")], Some(seen))
        .unwrap();

    assert_eq!(order(&marks), vec![session("se_2"), session("se_1")]);
}

#[test]
fn reorder_with_an_older_revision_is_stale_and_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let marks = pinned(&dir, &["se_1", "se_2"]);
    let seen = revision(&marks);
    // Someone else marks a thing VIP: the pinned set is the same, the
    // revision is not.
    marks.set(&session("se_1"), None, Some(true)).unwrap();
    let before = marks.pinned().unwrap();

    let error = marks
        .reorder(&[session("se_2"), session("se_1")], Some(seen))
        .unwrap_err();

    assert!(
        matches!(error, MarksError::Stale { current } if current == before.1),
        "{error:?}"
    );
    assert_eq!(marks.pinned().unwrap(), before);
}

#[test]
fn reorder_with_a_revision_from_the_future_is_stale() {
    let dir = tempfile::tempdir().unwrap();
    let marks = pinned(&dir, &["se_1", "se_2"]);
    let now = revision(&marks);

    let error = marks
        .reorder(&[session("se_2"), session("se_1")], Some(now + 1))
        .unwrap_err();

    assert!(
        matches!(error, MarksError::Stale { current } if current == now),
        "{error:?}"
    );
    assert_eq!(order(&marks), vec![session("se_1"), session("se_2")]);
}

#[test]
fn reorder_without_a_revision_skips_the_stale_check() {
    let dir = tempfile::tempdir().unwrap();
    let marks = pinned(&dir, &["se_1", "se_2"]);
    marks.set(&session("se_1"), None, Some(true)).unwrap();
    marks.set(&session("se_1"), None, Some(false)).unwrap();

    marks
        .reorder(&[session("se_2"), session("se_1")], None)
        .unwrap();

    assert_eq!(order(&marks), vec![session("se_2"), session("se_1")]);
}

#[test]
fn a_stale_caller_whose_order_no_longer_fits_is_told_it_is_stale() {
    // The caller read [se_1, se_2]; then se_3 was pinned. Its order now
    // misses se_3, but what it needs to hear is to read the list again.
    let dir = tempfile::tempdir().unwrap();
    let marks = pinned(&dir, &["se_1", "se_2"]);
    let seen = revision(&marks);
    marks.set(&session("se_3"), Some(true), None).unwrap();
    let now = revision(&marks);

    let error = marks
        .reorder(&[session("se_2"), session("se_1")], Some(seen))
        .unwrap_err();

    assert!(
        matches!(error, MarksError::Stale { current } if current == now),
        "{error:?}"
    );
    assert_eq!(
        order(&marks),
        vec![session("se_1"), session("se_2"), session("se_3")]
    );
}

#[test]
fn an_up_to_date_caller_with_a_wrong_order_gets_a_mismatch() {
    let dir = tempfile::tempdir().unwrap();
    let marks = pinned(&dir, &["se_1", "se_2"]);
    let seen = revision(&marks);

    let error = marks.reorder(&[session("se_2")], Some(seen)).unwrap_err();

    assert!(matches!(error, MarksError::OrderMismatch { .. }), "{error:?}");
    assert_eq!(revision(&marks), seen);
}
