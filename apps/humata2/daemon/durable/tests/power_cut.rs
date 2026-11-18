//! Plays back power cuts: a child writes under a simulated folder and is
//! cut, and what's left on disk must be what was synced and nothing else.
//!
//! The child is this test binary run again with `cut_child` selected and
//! the environment naming the steps; that test does nothing otherwise.

use std::fs::{self, OpenOptions};
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::Command;

use humata2_durable::{File, create_dir_all, cut, install_sqlite, remove_file, rename, sync_dir};

const STEPS: &str = "HUMATA2_DURABLE_STEPS";
const ROOT: &str = "HUMATA2_DURABLE_ROOT";

fn writable() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    options
}

fn write_only() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.write(true);
    options
}

fn creating() -> OpenOptions {
    let mut options = writable();
    options.create(true).truncate(false);
    options
}

/// One step of the child's work, by name.
fn step(root: &Path, name: &str) {
    let file = |name: &str| root.join(name);
    match name {
        "write" => {
            let mut a = File::open(&writable(), &file("a")).unwrap();
            a.write_all_at(b"NEW", 0).unwrap();
            a.append(b"MORE").unwrap();
        }
        "sync-a" => File::open(&writable(), &file("a")).unwrap().sync_data().unwrap(),
        "truncate" => File::open(&writable(), &file("a")).unwrap().set_len(1).unwrap(),
        "truncate-write-only" => File::open(&write_only(), &file("a")).unwrap().set_len(1).unwrap(),
        "new" => {
            let mut b = File::open(&creating(), &file("b")).unwrap();
            b.append(b"made").unwrap();
            b.sync_all().unwrap();
        }
        "replace" => {
            let mut next = File::open(&creating(), &file("a.next")).unwrap();
            next.append(b"replaced").unwrap();
            next.sync_all().unwrap();
            rename(&file("a.next"), &file("a")).unwrap();
        }
        "remove" => remove_file(&file("a")).unwrap(),
        "sync-root" => sync_dir(root).unwrap(),
        "folders" => {
            create_dir_all(&file("x/y"), 0o700).unwrap();
            let mut c = File::open(&creating(), &file("x/y/c")).unwrap();
            c.append(b"deep").unwrap();
            c.sync_all().unwrap();
            sync_dir(&file("x/y")).unwrap();
        }
        "sql-synced" => sql(root, "FULL", "INSERT INTO t VALUES (2)"),
        "sql-unsynced" => sql(root, "OFF", "INSERT INTO t VALUES (3)"),
        "sql-checkpoint" => sql(root, "OFF", "PRAGMA wal_checkpoint(TRUNCATE)"),
        other => panic!("no step {other}"),
    }
}

/// Runs `statement` on the database, with `synchronous` set so it syncs
/// or doesn't. The connection is left open, as a running daemon's is.
fn sql(root: &Path, synchronous: &str, statement: &str) {
    install_sqlite();
    let db = rusqlite::Connection::open(root.join("db")).unwrap();
    db.pragma_update(None, "synchronous", synchronous).unwrap();
    db.execute_batch(statement).unwrap();
    std::mem::forget(db);
}

/// The child. Runs only when a test starts it.
#[test]
fn cut_child() {
    let Ok(steps) = std::env::var(STEPS) else {
        return;
    };
    let root = std::path::PathBuf::from(std::env::var(ROOT).unwrap());
    for name in steps.split(',') {
        step(&root, name);
    }
    cut("the end of the steps");
}

/// A folder to simulate, with `a` already on disk holding `old`.
fn folder() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("root")).unwrap();
    fs::write(dir.path().join("root/a"), "old").unwrap();
    dir
}

/// Runs `steps` in a child under a simulated power cut, then cuts it.
fn run_then_cut(dir: &Path, steps: &str) {
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["cut_child", "--exact", "--nocapture", "--test-threads=1"])
        .env(STEPS, steps)
        .env(ROOT, dir.join("root"))
        .env("HUMATA2_POWER_CUT_ROOTS", dir.join("root"))
        .env("HUMATA2_POWER_CUT_SPARE", dir.join("spare"))
        .env_remove("TESTBRIDGE_TEST_ONLY")
        .status()
        .unwrap();
    assert_eq!(status.signal(), Some(9), "the child didn't stop at the cut: {status}");
}

fn read(dir: &Path, name: &str) -> Option<String> {
    fs::read_to_string(dir.join("root").join(name)).ok()
}

#[test]
fn writes_not_synced_are_gone() {
    let dir = folder();
    run_then_cut(dir.path(), "write,truncate");
    assert_eq!(read(dir.path(), "a").as_deref(), Some("old"));
}

#[test]
fn a_cut_after_truncating_through_a_write_only_handle_puts_back_what_was_there() {
    let dir = folder();
    run_then_cut(dir.path(), "truncate-write-only");
    assert_eq!(read(dir.path(), "a").as_deref(), Some("old"));
}

#[test]
fn synced_writes_stay_and_later_ones_go() {
    let dir = folder();
    run_then_cut(dir.path(), "write,sync-a,truncate");
    assert_eq!(read(dir.path(), "a").as_deref(), Some("NEWMORE"));
}

#[test]
fn a_new_file_is_gone_until_its_folder_is_synced() {
    let dir = folder();
    run_then_cut(dir.path(), "new");
    assert_eq!(read(dir.path(), "b"), None);

    let dir = folder();
    run_then_cut(dir.path(), "new,sync-root");
    assert_eq!(read(dir.path(), "b").as_deref(), Some("made"));
}

#[test]
fn a_rename_over_a_file_is_undone_until_its_folder_is_synced() {
    let dir = folder();
    run_then_cut(dir.path(), "replace");
    assert_eq!(read(dir.path(), "a").as_deref(), Some("old"));
    assert_eq!(read(dir.path(), "a.next"), None);

    let dir = folder();
    run_then_cut(dir.path(), "replace,sync-root");
    assert_eq!(read(dir.path(), "a").as_deref(), Some("replaced"));
    assert_eq!(read(dir.path(), "a.next"), None);
}

#[test]
fn a_removed_file_comes_back_until_its_folder_is_synced() {
    let dir = folder();
    run_then_cut(dir.path(), "remove");
    assert_eq!(read(dir.path(), "a").as_deref(), Some("old"));

    let dir = folder();
    run_then_cut(dir.path(), "remove,sync-root");
    assert_eq!(read(dir.path(), "a"), None);
}

#[test]
fn new_folders_are_gone_until_their_parents_are_synced() {
    let dir = folder();
    run_then_cut(dir.path(), "folders");
    assert!(!dir.path().join("root/x").exists());
}

#[test]
fn nothing_outside_the_simulated_folders_is_touched() {
    let dir = folder();
    fs::write(dir.path().join("outside"), "kept").unwrap();
    run_then_cut(dir.path(), "write");
    assert_eq!(fs::read_to_string(dir.path().join("outside")).unwrap(), "kept");
}

/// A database under the folder holding row 1, closed cleanly.
fn database(dir: &Path) {
    let db = rusqlite::Connection::open(dir.join("root/db")).unwrap();
    db.execute_batch(
        "PRAGMA journal_mode = WAL; PRAGMA synchronous = FULL;
         CREATE TABLE t (n INTEGER PRIMARY KEY); INSERT INTO t VALUES (1);",
    )
    .unwrap();
}

fn rows(dir: &Path) -> Vec<i64> {
    let db = rusqlite::Connection::open(dir.join("root/db")).unwrap();
    let check: String = db.query_row("PRAGMA integrity_check", [], |row| row.get(0)).unwrap();
    assert_eq!(check, "ok");
    let mut query = db.prepare("SELECT n FROM t ORDER BY n").unwrap();
    query.query_map([], |row| row.get(0)).unwrap().map(Result::unwrap).collect()
}

#[test]
fn a_sqlite_commit_not_synced_is_gone_and_one_synced_stays() {
    let dir = folder();
    database(dir.path());
    run_then_cut(dir.path(), "sql-synced,sql-unsynced");
    assert_eq!(rows(dir.path()), [1, 2]);
}

#[test]
fn a_sqlite_checkpoint_not_synced_leaves_the_database_whole() {
    let dir = folder();
    database(dir.path());
    run_then_cut(dir.path(), "sql-synced,sql-checkpoint,sql-unsynced");
    assert_eq!(rows(dir.path()), [1, 2]);
}
