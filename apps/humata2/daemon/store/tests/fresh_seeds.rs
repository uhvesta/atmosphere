//! Needs the `fresh_seeds` migration and `pub use fresh_seeds::FreshSeed`.

use std::path::PathBuf;

use humata2_store::{FreshSeed, SessionRecord, Store};

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

fn seed(session: &str, conversation: &str, at: i64) -> FreshSeed {
    FreshSeed {
        session_id: session.to_owned(),
        conversation_id: conversation.to_owned(),
        seed: format!("pick up {conversation}"),
        created_unix_ms: at,
    }
}

#[test]
fn a_seed_survives_reopening_until_it_is_typed() {
    let dir = tempfile::tempdir().unwrap();
    {
        let store = Store::open(dir.path()).unwrap();
        store.put_session(&session("se_1")).unwrap();
        store.put_fresh_seed(&seed("se_1", "c1", 5)).unwrap();
    }
    let store = Store::open(dir.path()).unwrap();
    assert_eq!(store.fresh_seed("se_1").unwrap(), Some(seed("se_1", "c1", 5)));
    assert_eq!(store.fresh_seeds().unwrap(), vec![seed("se_1", "c1", 5)]);
    store.typed_fresh_seed("se_1", "c1").unwrap();
    assert_eq!(store.fresh_seed("se_1").unwrap(), None);
    assert!(store.fresh_seeds().unwrap().is_empty());
}

#[test]
fn a_newer_seed_replaces_the_old_and_outlives_its_typing() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.put_session(&session("se_1")).unwrap();
    store.put_fresh_seed(&seed("se_1", "c1", 5)).unwrap();
    store.put_fresh_seed(&seed("se_1", "c2", 6)).unwrap();
    // The first conversation's typing finishing late leaves the newer one.
    store.typed_fresh_seed("se_1", "c1").unwrap();
    assert_eq!(store.fresh_seed("se_1").unwrap(), Some(seed("se_1", "c2", 6)));
}

#[test]
fn seeds_are_listed_oldest_first_and_go_with_their_session() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.put_session(&session("se_1")).unwrap();
    store.put_session(&session("se_2")).unwrap();
    store.put_fresh_seed(&seed("se_2", "c2", 9)).unwrap();
    store.put_fresh_seed(&seed("se_1", "c1", 4)).unwrap();
    assert_eq!(
        store.fresh_seeds().unwrap(),
        vec![seed("se_1", "c1", 4), seed("se_2", "c2", 9)]
    );
    store.remove_session("se_1").unwrap();
    assert_eq!(store.fresh_seeds().unwrap(), vec![seed("se_2", "c2", 9)]);
}
