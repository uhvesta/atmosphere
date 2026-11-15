//! The last good copy (PER-31): a damaged database is restored from it,
//! with what changed since listed, and never replaced when there's none.

use std::fs;
use std::path::{Path, PathBuf};

use humata2_store::{Change, SessionRecord, Store, StoreError};

fn session(id: &str, title: &str) -> SessionRecord {
    SessionRecord {
        id: id.to_owned(),
        channel: "ch_1".to_owned(),
        machine: "ma_1".to_owned(),
        title: title.to_owned(),
        agent: "claude".to_owned(),
        folder: PathBuf::from("/work"),
        created_unix_ms: 1,
    }
}

/// Overwrites the database's header, as a disk fault might.
fn damage(root: &Path) {
    let path = root.join("state.sqlite");
    let mut bytes = fs::read(&path).unwrap();
    bytes[..16].copy_from_slice(b"not a database!!");
    fs::write(&path, bytes).unwrap();
}

fn titles(store: &Store) -> Vec<(String, String)> {
    store
        .sessions()
        .unwrap()
        .into_iter()
        .map(|session| (session.id, session.title))
        .collect()
}

#[test]
fn a_healthy_store_opens_without_restoring() {
    let dir = tempfile::tempdir().unwrap();
    Store::open(dir.path()).unwrap().put_session(&session("se_a", "a")).unwrap();
    let (store, restored) = Store::open_or_restore(dir.path()).unwrap();
    assert!(restored.is_none());
    assert_eq!(titles(&store), [("se_a".to_owned(), "a".to_owned())]);
}

#[test]
fn a_damaged_database_is_restored_from_the_copy_with_what_changed_since() {
    let dir = tempfile::tempdir().unwrap();
    {
        let store = Store::open(dir.path()).unwrap();
        store.put_session(&session("se_a", "a")).unwrap();
        store.put_session(&session("se_b", "b")).unwrap();
        store.save_last_good().unwrap();
        store.put_session(&session("se_a", "a2")).unwrap();
        store.remove_session("se_b").unwrap();
        store.put_session(&session("se_c", "c")).unwrap();
    }
    damage(dir.path());

    let (store, restored) = Store::open_or_restore(dir.path()).unwrap();

    let restored = restored.expect("the damaged database wasn't restored");
    assert_eq!(
        titles(&store),
        [("se_a".to_owned(), "a".to_owned()), ("se_b".to_owned(), "b".to_owned())]
    );
    assert!(restored.damaged_kept_at.exists(), "the damaged database wasn't kept");
    assert!(restored.copy_taken_unix_ms > 0);
    let sessions: Vec<(&str, Change)> = restored
        .changes
        .iter()
        .filter(|change| change.table == "sessions")
        .map(|change| (change.key.as_str(), change.change))
        .collect();
    // The damaged header leaves no table readable, so nothing is listed
    // as changed: each table is listed as unknown instead.
    assert!(sessions.is_empty(), "{sessions:?}");
    assert!(restored.unreadable_tables.contains(&"sessions".to_owned()));

    // The copy is still there for the next time.
    drop(store);
    damage(dir.path());
    let (store, again) = Store::open_or_restore(dir.path()).unwrap();
    assert!(again.is_some());
    assert_eq!(titles(&store).len(), 2);
}

#[test]
fn a_database_damaged_inside_lists_the_rows_lost_since_the_copy() {
    let dir = tempfile::tempdir().unwrap();
    {
        let store = Store::open(dir.path()).unwrap();
        store.put_session(&session("se_a", "a")).unwrap();
        store.put_session(&session("se_b", "b")).unwrap();
        store.save_last_good().unwrap();
        store.put_session(&session("se_a", "a2")).unwrap();
        store.remove_session("se_b").unwrap();
        store.put_session(&session("se_c", "c")).unwrap();
    }
    // An index's page is overwritten: the tables still read, but SQLite's
    // quick check finds the damage.
    let path = dir.path().join("state.sqlite");
    let db = rusqlite::Connection::open(&path).unwrap();
    let page: i64 = db
        .query_row("SELECT rootpage FROM sqlite_schema WHERE name = 'sessions_by_age'", [], |row| row.get(0))
        .unwrap();
    let size: i64 = db.query_row("PRAGMA page_size", [], |row| row.get(0)).unwrap();
    drop(db);
    let mut bytes = fs::read(&path).unwrap();
    let start = usize::try_from((page - 1) * size).unwrap();
    bytes[start..start + 64].fill(0xff);
    fs::write(&path, bytes).unwrap();

    let (store, restored) = Store::open_or_restore(dir.path()).unwrap();

    let restored = restored.expect("the damaged database wasn't restored");
    assert_eq!(titles(&store).len(), 2);
    let mut sessions: Vec<(&str, Change)> = restored
        .changes
        .iter()
        .filter(|change| change.table == "sessions")
        .map(|change| (change.key.as_str(), change.change))
        .collect();
    sessions.sort_by_key(|(key, _)| key.to_owned());
    assert_eq!(
        sessions,
        [
            ("'se_a'", Change::ChangedSinceCopy),
            ("'se_b'", Change::RemovedSinceCopy),
            ("'se_c'", Change::AddedSinceCopy),
        ]
    );
}

#[test]
fn a_damaged_database_without_a_copy_is_refused_and_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    Store::open(dir.path()).unwrap().put_session(&session("se_a", "a")).unwrap();
    damage(dir.path());
    let before = fs::read(dir.path().join("state.sqlite")).unwrap();

    let opened = Store::open_or_restore(dir.path());

    assert!(matches!(opened, Err(StoreError::DamagedWithoutCopy { .. })), "{:?}", opened.err());
    assert_eq!(fs::read(dir.path().join("state.sqlite")).unwrap(), before);
}

#[test]
fn a_restored_journal_entry_still_finds_its_file() {
    let dir = tempfile::tempdir().unwrap();
    let big: Vec<u8> = (0..200_000u32).flat_map(|n| n.wrapping_mul(2_654_435_761).to_le_bytes()).collect();
    {
        let store = Store::open(dir.path()).unwrap();
        store.journal().append("write", &big).unwrap();
        store.save_last_good().unwrap();
        // Acking deletes the entry and sweeps its file, unless the copy
        // still names it.
        store.journal().ack_through(1).unwrap();
    }
    damage(dir.path());

    let (store, restored) = Store::open_or_restore(dir.path()).unwrap();

    assert!(restored.is_some());
    let entries = store.journal().entries().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].payload, big);
}
