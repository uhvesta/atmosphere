//! Desk rows (DK-2, DK-4, DK-11, D182). Needs the `desk_sessions` migration,
//! `mod desk;` and `pub use desk::{DeskFolderKind, DeskRow};` in lib.rs, and
//! the `desk_sessions` delete in `remove_session`. Moves up with them.

use std::path::PathBuf;

use humata2_store::{DeskFolderKind, DeskRow, SessionRecord, Store};

fn session(id: &str, folder: &str) -> SessionRecord {
    SessionRecord {
        id: id.to_owned(),
        channel: "desk".to_owned(),
        machine: "ma_1".to_owned(),
        title: "Plan Q4".to_owned(),
        agent: "claude".to_owned(),
        folder: PathBuf::from(folder),
        created_unix_ms: 1,
    }
}

fn scratch(id: &str) -> DeskRow {
    DeskRow {
        session_id: id.to_owned(),
        kind: DeskFolderKind::Scratch { kept: false },
        boards: true,
        channels_and_machines: false,
        last_used_unix_ms: 10,
        closed_unix_ms: None,
        deleted_unix_ms: None,
        held_work: String::new(),
    }
}

#[test]
fn a_row_survives_reopening_the_store() {
    let dir = tempfile::tempdir().unwrap();
    {
        let store = Store::open(dir.path()).unwrap();
        store.put_session(&session("se_1", "/h/desk/plan-q4")).unwrap();
        store.put_desk(&scratch("se_1")).unwrap();
    }
    let store = Store::open(dir.path()).unwrap();
    assert_eq!(store.desk("se_1").unwrap(), Some(scratch("se_1")));
    assert_eq!(store.desks().unwrap(), vec![scratch("se_1")]);
}

#[test]
fn putting_again_replaces_the_row() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.put_session(&session("se_1", "/h/desk/plan-q4")).unwrap();
    store.put_desk(&scratch("se_1")).unwrap();
    let closed = DeskRow {
        kind: DeskFolderKind::Scratch { kept: true },
        boards: false,
        channels_and_machines: true,
        last_used_unix_ms: 50,
        closed_unix_ms: Some(99),
        deleted_unix_ms: Some(120),
        held_work: "acme/api: 2 commits on port-cron never pushed".to_owned(),
        ..scratch("se_1")
    };
    store.put_desk(&closed).unwrap();
    assert_eq!(store.desk("se_1").unwrap(), Some(closed.clone()));
    assert_eq!(store.desks().unwrap(), vec![closed]);
}

#[test]
fn a_picked_folder_row_keeps_its_kind() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.put_session(&session("se_2", "/Users/me/notes")).unwrap();
    let row = DeskRow {
        kind: DeskFolderKind::Picked,
        ..scratch("se_2")
    };
    store.put_desk(&row).unwrap();
    assert_eq!(store.desk("se_2").unwrap(), Some(row));
}

#[test]
fn every_row_is_listed_deleted_ones_too() {
    // The daemon picks a free scratch name from every row, tombstones included.
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.put_session(&session("se_1", "/h/desk/plan-q4")).unwrap();
    store.put_session(&session("se_2", "/h/desk/plan-q4-2")).unwrap();
    let gone = DeskRow {
        closed_unix_ms: Some(5),
        deleted_unix_ms: Some(6),
        ..scratch("se_1")
    };
    store.put_desk(&gone).unwrap();
    store.put_desk(&scratch("se_2")).unwrap();
    let mut ids: Vec<String> = store.desks().unwrap().into_iter().map(|row| row.session_id).collect();
    ids.sort();
    assert_eq!(ids, vec!["se_1".to_owned(), "se_2".to_owned()]);
}

#[test]
fn an_unknown_session_has_no_row() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    assert_eq!(store.desk("se_9").unwrap(), None);
}

#[test]
fn removing_the_session_removes_its_desk_row() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.put_session(&session("se_1", "/h/desk/plan-q4")).unwrap();
    store.put_desk(&scratch("se_1")).unwrap();
    store.remove_session("se_1").unwrap();
    assert_eq!(store.desk("se_1").unwrap(), None);
    assert!(store.desks().unwrap().is_empty());
}

#[test]
fn a_desk_row_needs_its_session() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    assert!(store.put_desk(&scratch("se_9")).is_err());
}
