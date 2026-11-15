use std::fs;
use std::path::PathBuf;

use humata2_store::{ReviewRecord, SessionRecord, Store, StoreError};

fn session(id: &str) -> SessionRecord {
    SessionRecord {
        id: id.to_owned(),
        channel: "ch_1".to_owned(),
        machine: "ma_1".to_owned(),
        title: "Fix the build".to_owned(),
        agent: "claude".to_owned(),
        folder: PathBuf::from("/work/web"),
        created_unix_ms: 1,
    }
}

fn review(session: &str, chat_len: usize) -> ReviewRecord {
    ReviewRecord {
        session: session.to_owned(),
        chat: (0..chat_len).map(|n| (n % 251) as u8).collect(),
        shown: vec![b"api diff".to_vec(), Vec::new()],
        last_seq: 7,
        last_note: 3,
        view_reported: true,
        history: b"undo history".to_vec(),
        updated_unix_ms: 42,
    }
}

#[test]
fn a_review_survives_reopening_and_a_rewrite_replaces_it() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.put_session(&session("se_a")).unwrap();
    assert_eq!(store.review("se_a").unwrap(), None);

    store.put_review(&review("se_a", 10)).unwrap();
    let rewritten = ReviewRecord {
        last_seq: 9,
        view_reported: false,
        ..review("se_a", 200_000)
    };
    store.put_review(&rewritten).unwrap();
    drop(store);

    let store = Store::open(dir.path()).unwrap();
    assert_eq!(store.review("se_a").unwrap(), Some(rewritten));
}

#[test]
fn a_review_needs_its_session_and_goes_with_it() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let refused = store.put_review(&review("se_gone", 1));
    assert!(matches!(refused, Err(StoreError::NoSession { id }) if id == "se_gone"));

    store.put_session(&session("se_a")).unwrap();
    store.put_review(&review("se_a", 200_000)).unwrap();
    store.remove_session("se_a").unwrap();

    assert_eq!(store.review("se_a").unwrap(), None);
    assert_eq!(
        fs::read_dir(dir.path().join("blobs")).unwrap().count(),
        0,
        "the review's file outlived its session"
    );
}
