use std::fs;
use std::path::PathBuf;

use humata2_store::{SessionRecord, Store, StoreError, ViewRecord};

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

fn view(id: &str, session: &str, prepared_unix_ms: i64, len: usize) -> ViewRecord {
    ViewRecord {
        id: id.to_owned(),
        session: session.to_owned(),
        view: (0..len).map(|n| (n % 251) as u8).collect(),
        keys: vec!["key-1".to_owned()],
        prepared_unix_ms,
        updated_unix_ms: prepared_unix_ms,
    }
}

#[test]
fn views_survive_reopening_newest_first_and_a_rewrite_replaces_one() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.put_session(&session("se_a")).unwrap();
    store.put_session(&session("se_b")).unwrap();
    assert_eq!(store.views("se_a").unwrap(), Vec::new());
    assert_eq!(store.view("vw_1").unwrap(), None);

    store.put_view(&view("vw_1", "se_a", 10, 10)).unwrap();
    store.put_view(&view("vw_2", "se_a", 20, 200_000)).unwrap();
    store.put_view(&view("vw_3", "se_b", 30, 10)).unwrap();
    let revised = ViewRecord {
        keys: vec!["key-1".to_owned(), "key-2".to_owned()],
        updated_unix_ms: 40,
        ..view("vw_1", "se_a", 10, 20)
    };
    store.put_view(&revised).unwrap();
    drop(store);

    let store = Store::open(dir.path()).unwrap();
    assert_eq!(store.view("vw_1").unwrap(), Some(revised.clone()));
    assert_eq!(
        store.views("se_a").unwrap(),
        vec![view("vw_2", "se_a", 20, 200_000), revised]
    );
}

#[test]
fn a_view_needs_its_session_and_goes_with_it() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let refused = store.put_view(&view("vw_1", "se_gone", 1, 1));
    assert!(matches!(refused, Err(StoreError::NoSession { id }) if id == "se_gone"));

    store.put_session(&session("se_a")).unwrap();
    store.put_view(&view("vw_1", "se_a", 1, 200_000)).unwrap();
    store.remove_session("se_a").unwrap();

    assert_eq!(store.view("vw_1").unwrap(), None);
    assert_eq!(
        fs::read_dir(dir.path().join("blobs")).unwrap().count(),
        0,
        "the view's file outlived its session"
    );
}
