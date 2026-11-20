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

// The copy an upgrade keeps before it changes the schema (D281g). An older
// build's database stands in as this build's with a lower schema number:
// the tables are the same, only the number the upgrade reads differs.

/// Sets the schema number of the closed database in `root`.
fn set_schema(root: &Path, version: i64) {
    let connection = rusqlite::Connection::open(root.join("state.sqlite")).unwrap();
    connection.pragma_update(None, "user_version", version).unwrap();
}

/// A closed store in `root` at the schema before this build's, holding
/// session `se_a` titled "old".
fn older_store(root: &Path) {
    Store::open(root).unwrap().put_session(&session("se_a", "old")).unwrap();
    set_schema(root, humata2_store::SCHEMA_VERSION - 1);
}

/// `len` bytes that don't compress, so a journal entry of them is kept in
/// a blob file rather than its row.
fn noise(len: usize) -> Vec<u8> {
    let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut bytes = Vec::with_capacity(len + 8);
    while bytes.len() < len {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        bytes.extend_from_slice(&state.to_le_bytes());
    }
    bytes.truncate(len);
    bytes
}

fn blob_files(root: &Path) -> usize {
    fs::read_dir(root.join("blobs")).unwrap().count()
}

#[test]
fn a_copy_before_an_upgrade_is_kept_only_when_the_schema_changes() {
    let dir = tempfile::tempdir().unwrap();
    let none_yet = Store::copy_before_upgrade(dir.path(), "to_2.0.5").unwrap();
    assert!(none_yet.is_none(), "copied a database that isn't there");

    Store::open(dir.path()).unwrap().put_session(&session("se_a", "old")).unwrap();
    let same = Store::copy_before_upgrade(dir.path(), "to_2.0.5").unwrap();
    assert!(same.is_none(), "copied a database already at this build's schema");

    set_schema(dir.path(), humata2_store::SCHEMA_VERSION - 1);
    let copy = Store::copy_before_upgrade(dir.path(), "to_2.0.5").unwrap().unwrap();
    assert_eq!(copy.schema, humata2_store::SCHEMA_VERSION - 1);
    assert!(copy.path.is_file());
}

#[test]
fn a_database_newer_than_this_build_is_refused_rather_than_copied() {
    let dir = tempfile::tempdir().unwrap();
    Store::open(dir.path()).unwrap();
    set_schema(dir.path(), humata2_store::SCHEMA_VERSION + 1);

    let copied = Store::copy_before_upgrade(dir.path(), "to_2.0.5");

    assert!(matches!(copied, Err(StoreError::SchemaTooNew { .. })), "{copied:?}");
}

#[test]
fn putting_the_copy_back_undoes_the_upgrade_and_keeps_what_it_replaced() {
    let dir = tempfile::tempdir().unwrap();
    older_store(dir.path());
    Store::copy_before_upgrade(dir.path(), "to_2.0.5").unwrap().unwrap();
    // The new build migrates and writes, then never reaches ready.
    set_schema(dir.path(), humata2_store::SCHEMA_VERSION);
    Store::open(dir.path()).unwrap().put_session(&session("se_a", "upgraded")).unwrap();

    let restored = Store::restore_upgrade_copy(dir.path(), "to_2.0.5").unwrap();

    assert_eq!(restored.schema, humata2_store::SCHEMA_VERSION - 1);
    assert!(restored.replaced_kept_at.is_file(), "the replaced database wasn't kept");
    set_schema(dir.path(), humata2_store::SCHEMA_VERSION);
    let store = Store::open(dir.path()).unwrap();
    assert_eq!(titles(&store), vec![("se_a".to_owned(), "old".to_owned())]);
}

#[test]
fn a_restore_puts_back_a_last_good_copy_the_older_build_can_read() {
    let dir = tempfile::tempdir().unwrap();
    older_store(dir.path());
    Store::copy_before_upgrade(dir.path(), "to_2.0.5").unwrap().unwrap();
    set_schema(dir.path(), humata2_store::SCHEMA_VERSION);
    Store::open(dir.path()).unwrap().save_last_good().unwrap();

    Store::restore_upgrade_copy(dir.path(), "to_2.0.5").unwrap();

    // The older build, finding its database damaged later, restores a copy
    // of its own schema, not the newer build's.
    let last_good = rusqlite::Connection::open(dir.path().join("last-good.sqlite")).unwrap();
    let schema: i64 = last_good.query_row("PRAGMA user_version", [], |row| row.get(0)).unwrap();
    assert_eq!(schema, humata2_store::SCHEMA_VERSION - 1);
}

#[test]
fn putting_back_a_copy_never_taken_or_dropped_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    older_store(dir.path());

    let never = Store::restore_upgrade_copy(dir.path(), "to_2.0.5");
    assert!(matches!(never, Err(StoreError::NoUpgradeCopy { .. })), "{never:?}");

    Store::copy_before_upgrade(dir.path(), "to_2.0.5").unwrap().unwrap();
    Store::drop_upgrade_copy(dir.path(), "to_2.0.5").unwrap();
    Store::drop_upgrade_copy(dir.path(), "to_2.0.5").unwrap();
    let dropped = Store::restore_upgrade_copy(dir.path(), "to_2.0.5");
    assert!(matches!(dropped, Err(StoreError::NoUpgradeCopy { .. })), "{dropped:?}");
}

#[test]
fn a_copy_name_that_could_leave_its_folder_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    older_store(dir.path());

    let copied = Store::copy_before_upgrade(dir.path(), "../state");

    assert!(matches!(copied, Err(StoreError::InvalidId { .. })), "{copied:?}");
}

#[test]
fn the_files_a_pre_upgrade_copy_names_are_kept_until_it_is_dropped() {
    let dir = tempfile::tempdir().unwrap();
    {
        let store = Store::open(dir.path()).unwrap();
        store.journal().append("write", &noise(300 << 10)).unwrap();
    }
    let with_file = blob_files(dir.path());
    assert!(with_file > 0, "the payload went in its row, not a file");
    set_schema(dir.path(), humata2_store::SCHEMA_VERSION - 1);
    Store::copy_before_upgrade(dir.path(), "to_2.0.5").unwrap().unwrap();
    set_schema(dir.path(), humata2_store::SCHEMA_VERSION);
    {
        let store = Store::open(dir.path()).unwrap();
        let last = store.journal().entries().unwrap().last().unwrap().seq;
        store.journal().ack_through(last).unwrap();
    }

    drop(Store::open(dir.path()).unwrap());
    assert_eq!(blob_files(dir.path()), with_file, "a file the copy names was swept");

    Store::drop_upgrade_copy(dir.path(), "to_2.0.5").unwrap();
    drop(Store::open(dir.path()).unwrap());
    assert!(blob_files(dir.path()) < with_file, "the dropped copy's file was kept");
}

#[test]
fn damaged_copies_are_never_pruned_while_old_upgrade_copies_are() {
    let dir = tempfile::tempdir().unwrap();
    for at in 1..=4 {
        fs::create_dir(dir.path().join(format!("damaged-{at}"))).unwrap();
        fs::create_dir(dir.path().join(format!("upgraded-{at}"))).unwrap();
    }
    drop(Store::open(dir.path()).unwrap());
    let mut left: Vec<String> = fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|entry| entry.unwrap().file_name().into_string().ok())
        .filter(|name| name.starts_with("damaged-") || name.starts_with("upgraded-"))
        .collect();
    left.sort();
    assert_eq!(
        left,
        ["damaged-1", "damaged-2", "damaged-3", "damaged-4", "upgraded-3", "upgraded-4"]
    );
}
