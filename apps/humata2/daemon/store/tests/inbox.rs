use std::fs;
use std::path::PathBuf;

use humata2_store::{InboxRecord, SessionRecord, Store, StoreError};

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

fn request(id: &str, session: &str, created_unix_ms: i64, len: usize) -> InboxRecord {
    InboxRecord {
        id: id.to_owned(),
        session: session.to_owned(),
        entry: (0..len).map(|n| (n % 251) as u8).collect(),
        created_unix_ms,
        updated_unix_ms: created_unix_ms,
    }
}

#[test]
fn requests_survive_reopening_oldest_first_and_a_rewrite_replaces_one() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.put_session(&session("se_a")).unwrap();
    store.put_session(&session("se_b")).unwrap();
    assert_eq!(store.inbox_requests().unwrap(), Vec::new());

    store.put_inbox_request(&request("rq_2", "se_a", 20, 200_000)).unwrap();
    store.put_inbox_request(&request("rq_1", "se_b", 10, 10)).unwrap();
    let answered = InboxRecord {
        updated_unix_ms: 30,
        ..request("rq_1", "se_b", 10, 40)
    };
    store.put_inbox_request(&answered).unwrap();
    drop(store);

    let store = Store::open(dir.path()).unwrap();
    assert_eq!(
        store.inbox_requests().unwrap(),
        vec![answered, request("rq_2", "se_a", 20, 200_000)]
    );
}

#[test]
fn a_request_needs_its_session_and_goes_with_it() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let refused = store.put_inbox_request(&request("rq_1", "se_gone", 1, 1));
    assert!(matches!(refused, Err(StoreError::NoSession { id }) if id == "se_gone"));

    store.put_session(&session("se_a")).unwrap();
    store.put_session(&session("se_b")).unwrap();
    store.put_inbox_request(&request("rq_1", "se_a", 1, 200_000)).unwrap();
    store.put_inbox_request(&request("rq_2", "se_b", 2, 10)).unwrap();
    store.remove_session("se_a").unwrap();

    assert_eq!(store.inbox_requests().unwrap(), vec![request("rq_2", "se_b", 2, 10)]);
    assert_eq!(
        fs::read_dir(dir.path().join("blobs")).unwrap().count(),
        0,
        "the request's file outlived its session"
    );
}
