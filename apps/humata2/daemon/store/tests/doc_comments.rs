use std::path::PathBuf;

use humata2_store::{SessionRecord, Store, StoreError};

const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
const OTHER: &str = "fedcba9876543210fedcba9876543210fedcba98";

fn session(id: &str) -> SessionRecord {
    SessionRecord {
        id: id.to_owned(),
        channel: "desk".to_owned(),
        machine: "ma_1".to_owned(),
        title: "Plan".to_owned(),
        agent: "claude".to_owned(),
        folder: PathBuf::from("/work/plan"),
        created_unix_ms: 1,
    }
}

fn open() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.put_session(&session("se_a")).unwrap();
    (dir, store)
}

#[test]
fn a_record_is_read_back_and_a_missing_one_is_none() {
    let (_dir, store) = open();
    assert_eq!(store.doc_comments("se_a", COMMIT, "rfc.md").unwrap(), None);
    assert!(store.put_doc_comments("se_a", COMMIT, "rfc.md", b"\x01\x02", 5).unwrap());
    assert_eq!(store.doc_comments("se_a", COMMIT, "rfc.md").unwrap(), Some(vec![1, 2]));
    assert_eq!(store.doc_comments("se_a", COMMIT, "other.md").unwrap(), None, "records are per file");
    assert_eq!(store.doc_comments("se_a", OTHER, "rfc.md").unwrap(), None, "records are per commit");
}

#[test]
fn an_empty_record_is_a_record() {
    let (_dir, store) = open();
    assert!(store.put_doc_comments("se_a", COMMIT, "rfc.md", b"", 5).unwrap());
    assert_eq!(store.doc_comments("se_a", COMMIT, "rfc.md").unwrap(), Some(Vec::new()));
}

#[test]
fn a_second_put_for_the_same_key_keeps_the_first() {
    let (_dir, store) = open();
    assert!(store.put_doc_comments("se_a", COMMIT, "rfc.md", b"first", 5).unwrap());
    assert!(!store.put_doc_comments("se_a", COMMIT, "rfc.md", b"second", 6).unwrap());
    assert_eq!(store.doc_comments("se_a", COMMIT, "rfc.md").unwrap(), Some(b"first".to_vec()));
}

#[test]
fn the_commits_with_records_are_listed_per_file() {
    let (_dir, store) = open();
    store.put_doc_comments("se_a", COMMIT, "rfc.md", b"a", 5).unwrap();
    store.put_doc_comments("se_a", OTHER, "rfc.md", b"", 6).unwrap();
    store.put_doc_comments("se_a", OTHER, "notes.md", b"c", 6).unwrap();
    let mut commits = store.doc_comment_commits("se_a", "rfc.md").unwrap();
    commits.sort();
    assert_eq!(commits, vec![COMMIT.to_owned(), OTHER.to_owned()]);
    assert_eq!(store.doc_comment_commits("se_a", "notes.md").unwrap(), vec![OTHER.to_owned()]);
    assert!(store.doc_comment_commits("se_a", "none.md").unwrap().is_empty());
}

#[test]
fn records_survive_reopening_and_go_with_the_session() {
    let (dir, store) = open();
    store.put_doc_comments("se_a", COMMIT, "rfc.md", b"kept", 5).unwrap();
    drop(store);
    let store = Store::open(dir.path()).unwrap();
    assert_eq!(store.doc_comments("se_a", COMMIT, "rfc.md").unwrap(), Some(b"kept".to_vec()));
    store.remove_session("se_a").unwrap();
    assert_eq!(store.doc_comments("se_a", COMMIT, "rfc.md").unwrap(), None);
}

#[test]
fn a_record_for_an_unsaved_session_is_refused() {
    let (_dir, store) = open();
    let refused = store.put_doc_comments("se_missing", COMMIT, "rfc.md", b"x", 5).unwrap_err();
    assert!(matches!(refused, StoreError::NoSession { .. }));
}
