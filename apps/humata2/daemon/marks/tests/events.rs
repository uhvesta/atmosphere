//! NV-21: a pinned thing shows its event (needs input, finished, failed)
//! in the Pinned section. The event is part of what the list shows, so
//! setting one on a pinned thing is a change: it raises the revision, as a
//! pin does, and a client that reorders against the older revision is
//! told to read again.

use humata2_marks::{Event, Marks, MarksError, Thing};

fn session(id: &str) -> Thing {
    Thing::Session(id.to_owned())
}

fn revision(marks: &Marks) -> u64 {
    marks.pinned().unwrap().1
}

fn event(text: &str, at_unix_ms: i64) -> Event {
    Event {
        text: text.to_owned(),
        at_unix_ms,
    }
}

fn open() -> (tempfile::TempDir, Marks) {
    let dir = tempfile::tempdir().unwrap();
    let marks = Marks::open(&dir.path().join("marks.db")).unwrap();
    (dir, marks)
}

#[test]
fn a_pinned_things_event_is_listed_with_it_and_raises_the_revision() {
    let (_dir, marks) = open();
    marks.set(&session("se_1"), Some(true), None).unwrap();
    marks.set(&session("se_2"), Some(true), None).unwrap();
    let before = revision(&marks);

    let changed = marks
        .set_event(&session("se_2"), Some(&event("needs input", 7)))
        .unwrap();

    let after = revision(&marks);
    assert!(after > before, "revision {after} after {before}");
    assert_eq!(changed, Some(after));
    let events = marks.events().unwrap();
    assert_eq!(events.get(&session("se_2")), Some(&event("needs input", 7)));
    assert_eq!(events.get(&session("se_1")), None);
}

#[test]
fn the_same_event_again_changes_nothing() {
    let (_dir, marks) = open();
    marks.set(&session("se_1"), Some(true), None).unwrap();
    marks
        .set_event(&session("se_1"), Some(&event("finished", 1)))
        .unwrap();
    let before = revision(&marks);

    let again = marks
        .set_event(&session("se_1"), Some(&event("finished", 1)))
        .unwrap();

    assert_eq!(again, None);
    assert_eq!(revision(&marks), before);
}

#[test]
fn clearing_an_event_is_a_change_and_clearing_none_is_not() {
    let (_dir, marks) = open();
    marks.set(&session("se_1"), Some(true), None).unwrap();
    assert_eq!(marks.set_event(&session("se_1"), None).unwrap(), None);
    marks
        .set_event(&session("se_1"), Some(&event("failed", 3)))
        .unwrap();
    let before = revision(&marks);

    let cleared = marks.set_event(&session("se_1"), None).unwrap();

    assert!(cleared.is_some_and(|now| now > before), "{cleared:?}");
    assert!(marks.events().unwrap().is_empty());
}

#[test]
fn a_thing_that_isnt_pinned_keeps_no_event() {
    let (_dir, marks) = open();
    // VIP only, and not marked at all.
    marks.set(&session("se_vip"), None, Some(true)).unwrap();
    let before = revision(&marks);

    for id in ["se_vip", "se_none"] {
        let changed = marks
            .set_event(&session(id), Some(&event("needs input", 1)))
            .unwrap();
        assert_eq!(changed, None, "{id}");
    }

    assert_eq!(revision(&marks), before);
    assert!(marks.events().unwrap().is_empty());
    // Not kept for later either: pinning it now shows no event yet.
    marks.set(&session("se_vip"), Some(true), None).unwrap();
    assert!(marks.events().unwrap().is_empty());
}

#[test]
fn a_thing_pinned_again_starts_with_no_event() {
    let (_dir, marks) = open();
    marks.set(&session("se_1"), Some(true), Some(true)).unwrap();
    marks
        .set_event(&session("se_1"), Some(&event("finished", 1)))
        .unwrap();

    // Unpinned while VIP keeps its row, then pinned again.
    marks.set(&session("se_1"), Some(false), None).unwrap();
    marks.set(&session("se_1"), Some(true), None).unwrap();

    assert!(marks.events().unwrap().is_empty());
}

#[test]
fn an_event_makes_a_reorder_against_the_older_list_stale() {
    let (_dir, marks) = open();
    marks.set(&session("se_1"), Some(true), None).unwrap();
    marks.set(&session("se_2"), Some(true), None).unwrap();
    let read = revision(&marks);
    marks
        .set_event(&session("se_1"), Some(&event("needs input", 1)))
        .unwrap();

    let refused = marks
        .reorder(&[session("se_2"), session("se_1")], Some(read))
        .err();

    assert!(matches!(refused, Some(MarksError::Stale { .. })), "{refused:?}");
}

#[test]
fn events_survive_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("marks.db");
    {
        let marks = Marks::open(&path).unwrap();
        marks.set(&session("se_1"), Some(true), None).unwrap();
        marks
            .set_event(&session("se_1"), Some(&event("failed", 42)))
            .unwrap();
    }

    let marks = Marks::open(&path).unwrap();

    assert_eq!(
        marks.events().unwrap().get(&session("se_1")),
        Some(&event("failed", 42))
    );
}

#[test]
fn a_file_from_before_events_opens_with_its_marks_and_takes_events() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("marks.db");
    // The marks table as it was before events were kept.
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE marks (
                 kind TEXT NOT NULL, id TEXT NOT NULL, position INTEGER,
                 vip INTEGER NOT NULL DEFAULT 0, PRIMARY KEY (kind, id)
             ) WITHOUT ROWID;
             CREATE TABLE meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL) WITHOUT ROWID;
             INSERT INTO meta (key, value) VALUES ('revision', 5);
             INSERT INTO marks (kind, id, position, vip) VALUES ('session', 'se_old', 0, 1);",
        )
        .unwrap();
    }

    let marks = Marks::open(&path).unwrap();

    let (pinned, at) = marks.pinned().unwrap();
    assert_eq!(at, 5);
    assert_eq!(pinned.len(), 1);
    assert!(pinned[0].vip);
    assert!(
        marks
            .set_event(&session("se_old"), Some(&event("finished", 1)))
            .unwrap()
            .is_some()
    );
}
