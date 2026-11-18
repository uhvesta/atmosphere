use std::fs;
use std::path::PathBuf;

use humata2_store::{InboxRecord, MigrateRecord, SessionRecord, Store, StoreError};

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

    store
        .put_inbox_request(&request("rq_2", "se_a", 20, 200_000))
        .unwrap();
    store
        .put_inbox_request(&request("rq_1", "se_b", 10, 10))
        .unwrap();
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
    store
        .put_inbox_request(&request("rq_1", "se_a", 1, 200_000))
        .unwrap();
    store
        .put_inbox_request(&request("rq_2", "se_b", 2, 10))
        .unwrap();
    store.remove_session("se_a").unwrap();

    assert_eq!(
        store.inbox_requests().unwrap(),
        vec![request("rq_2", "se_b", 2, 10)]
    );
    assert_eq!(
        fs::read_dir(dir.path().join("blobs")).unwrap().count(),
        0,
        "the request's file outlived its session"
    );
}

/// A Migrate item for `thing` on `machine`, pending or withdrawn.
fn migrate(id: &str, thing: i32, machine: &str, pending: bool, len: usize) -> MigrateRecord {
    MigrateRecord {
        id: id.to_owned(),
        thing,
        machine: machine.to_owned(),
        pending,
        entry: (0..len).map(|n| (n % 251) as u8).collect(),
        created_unix_ms: 1,
        updated_unix_ms: 1,
    }
}

fn pending_ids(store: &Store) -> Vec<String> {
    store
        .pending_migrate_requests()
        .unwrap()
        .into_iter()
        .map(|request| request.id)
        .collect()
}

#[test]
fn a_pending_migrate_item_survives_reopening_and_a_rewrite_replaces_it() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store
        .put_migrate_request(&migrate("rq_m1", 1, "ma_gpu", true, 200_000))
        .unwrap();
    let changed = MigrateRecord {
        updated_unix_ms: 2,
        ..migrate("rq_m1", 1, "ma_gpu", true, 40)
    };
    store.put_migrate_request(&changed).unwrap();
    drop(store);

    let store = Store::open(dir.path()).unwrap();
    assert_eq!(store.pending_migrate_requests().unwrap(), vec![changed]);
    assert_eq!(
        store.inbox_requests().unwrap(),
        Vec::new(),
        "a Migrate item is no session's request"
    );
}

#[test]
fn a_second_pending_migrate_item_for_the_same_thing_and_machine_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store
        .put_migrate_request(&migrate("rq_m1", 1, "ma_gpu", true, 10))
        .unwrap();

    let refused = store.put_migrate_request(&migrate("rq_m2", 1, "ma_gpu", true, 10));
    assert!(
        matches!(refused, Err(StoreError::MigrateTaken { thing: 1, ref machine }) if machine == "ma_gpu"),
        "a second pending item for the same thing and machine was saved"
    );

    store
        .put_migrate_request(&migrate("rq_m3", 2, "ma_gpu", true, 10))
        .unwrap();
    store
        .put_migrate_request(&migrate("rq_m4", 1, "ma_box", true, 10))
        .unwrap();
    assert_eq!(pending_ids(&store), vec!["rq_m1", "rq_m3", "rq_m4"]);
}

#[test]
fn a_withdrawn_migrate_item_stays_out_of_the_pending_list_and_blocks_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store
        .put_migrate_request(&migrate("rq_m1", 1, "ma_gpu", true, 10))
        .unwrap();
    store
        .put_migrate_request(&migrate("rq_m1", 1, "ma_gpu", false, 10))
        .unwrap();
    drop(store);

    let store = Store::open(dir.path()).unwrap();
    assert_eq!(pending_ids(&store), Vec::<String>::new());
    store
        .put_migrate_request(&migrate("rq_m2", 1, "ma_gpu", true, 10))
        .unwrap();
    assert_eq!(pending_ids(&store), vec!["rq_m2"]);
}

#[test]
fn a_migrate_item_outlives_the_sessions_whose_requests_go_with_them() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.put_session(&session("se_a")).unwrap();
    store
        .put_inbox_request(&request("rq_1", "se_a", 1, 10))
        .unwrap();
    store
        .put_migrate_request(&migrate("rq_m1", 1, "ma_gpu", true, 10))
        .unwrap();
    store.remove_session("se_a").unwrap();

    assert_eq!(store.inbox_requests().unwrap(), Vec::new());
    assert_eq!(pending_ids(&store), vec!["rq_m1"]);
}
