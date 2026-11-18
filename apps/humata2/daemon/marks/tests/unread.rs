//! Read marks in the marks file (CH-16, CH-17; D392): a session marked
//! unread stays so when the file is closed and opened again, until it is
//! marked read; read marks aren't part of the pinned list.

use humata2_marks::{Marks, Thing};

fn open(dir: &tempfile::TempDir) -> Marks {
    Marks::open(&dir.path().join("marks.db")).unwrap()
}

#[test]
fn an_unread_mark_is_kept_until_it_is_read() {
    let dir = tempfile::tempdir().unwrap();
    let marks = open(&dir);
    assert!(marks.set_unread("se_1", true).unwrap(), "marking it unread changes it");
    assert!(!marks.set_unread("se_1", true).unwrap(), "marking it again changes nothing");
    drop(marks);

    let marks = open(&dir);
    assert_eq!(marks.unread().unwrap(), vec!["se_1".to_owned()]);
    assert!(marks.set_unread("se_1", false).unwrap(), "reading it changes it");
    assert!(!marks.set_unread("se_1", false).unwrap(), "reading it again changes nothing");
    drop(marks);

    assert!(open(&dir).unread().unwrap().is_empty());
}

#[test]
fn read_marks_leave_the_pinned_list_alone() {
    let dir = tempfile::tempdir().unwrap();
    let marks = open(&dir);
    marks.set(&Thing::Session("se_1".to_owned()), Some(true), None).unwrap();
    let before = marks.pinned().unwrap();
    marks.set_unread("se_1", true).unwrap();
    marks.set_unread("se_2", true).unwrap();
    marks.set_unread("se_1", false).unwrap();
    assert_eq!(marks.pinned().unwrap(), before);
}

#[test]
fn forgetting_a_session_drops_its_unread_mark() {
    let dir = tempfile::tempdir().unwrap();
    let marks = open(&dir);
    marks.set_unread("se_1", true).unwrap();
    marks.set_unread("se_2", true).unwrap();
    marks.forget(&Thing::Session("se_1".to_owned())).unwrap();
    assert_eq!(marks.unread().unwrap(), vec!["se_2".to_owned()]);
}

#[test]
fn a_file_from_before_read_marks_takes_them() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("marks.db");
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE marks (
                 kind TEXT NOT NULL,
                 id TEXT NOT NULL,
                 position INTEGER,
                 vip INTEGER NOT NULL DEFAULT 0,
                 PRIMARY KEY (kind, id)
             ) WITHOUT ROWID;
             CREATE TABLE meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL) WITHOUT ROWID;
             INSERT INTO meta (key, value) VALUES ('revision', 0);",
        )
        .unwrap();
    }
    let marks = Marks::open(&path).unwrap();
    assert!(marks.unread().unwrap().is_empty());
    assert!(marks.set_unread("se_1", true).unwrap());
    assert_eq!(marks.unread().unwrap(), vec!["se_1".to_owned()]);
}
