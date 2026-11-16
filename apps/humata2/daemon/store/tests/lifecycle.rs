//! Needs the `fork_origins` and `archives` migration, `mod lifecycle` with
//! its `pub use`, and remove_session's cascade (FORK seam script).

use std::path::PathBuf;

use humata2_store::{ArchiveRow, ArchiveState, ForkOrigin, SessionRecord, Store};

fn session(id: &str) -> SessionRecord {
    SessionRecord {
        id: id.to_owned(),
        channel: "ch_1".to_owned(),
        machine: "ma_1".to_owned(),
        title: "Fix the build".to_owned(),
        agent: "claude".to_owned(),
        folder: PathBuf::from(format!("/work/{id}")),
        created_unix_ms: 1,
    }
}

fn origin(session: &str, parent: &str, at: i64) -> ForkOrigin {
    ForkOrigin {
        session_id: session.to_owned(),
        parent: parent.to_owned(),
        kind: 1,
        before_turn: 0,
        created_unix_ms: at,
    }
}

fn archived(session: &str, state: ArchiveState) -> ArchiveRow {
    ArchiveRow {
        session_id: session.to_owned(),
        archived_unix_ms: 10,
        state,
        summary: vec![1, 2, 3],
        free_after_unix_ms: 10_010,
    }
}

#[test]
fn fork_origins_survive_reopening() {
    let dir = tempfile::tempdir().unwrap();
    {
        let store = Store::open(dir.path()).unwrap();
        for id in ["se_p", "se_a", "se_b"] {
            store.put_session(&session(id)).unwrap();
        }
        store.put_fork_origin(&origin("se_b", "se_a", 3)).unwrap();
        store.put_fork_origin(&origin("se_a", "se_p", 2)).unwrap();
    }
    let store = Store::open(dir.path()).unwrap();
    assert_eq!(store.fork_origin("se_a").unwrap(), Some(origin("se_a", "se_p", 2)));
    assert_eq!(store.fork_origin("se_p").unwrap(), None);
    assert_eq!(
        store.fork_origins().unwrap(),
        vec![origin("se_a", "se_p", 2), origin("se_b", "se_a", 3)]
    );
}

#[test]
fn an_archive_moves_through_its_states_and_is_forgotten_on_restore() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.put_session(&session("se_1")).unwrap();
    store.put_archive(&archived("se_1", ArchiveState::Archiving)).unwrap();
    store.put_archive(&archived("se_1", ArchiveState::Archived)).unwrap();
    drop(store);
    let store = Store::open(dir.path()).unwrap();
    assert_eq!(
        store.archive("se_1").unwrap(),
        Some(archived("se_1", ArchiveState::Archived))
    );
    store.put_archive(&archived("se_1", ArchiveState::Freed)).unwrap();
    assert_eq!(store.archives().unwrap(), vec![archived("se_1", ArchiveState::Freed)]);
    store.remove_archive("se_1").unwrap();
    assert_eq!(store.archive("se_1").unwrap(), None);
    assert!(store.archives().unwrap().is_empty());
}

#[test]
fn removing_a_session_forgets_its_origin_and_archive_but_not_its_forks() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    for id in ["se_p", "se_f"] {
        store.put_session(&session(id)).unwrap();
    }
    store.put_fork_origin(&origin("se_p", "se_gone", 1)).unwrap();
    store.put_fork_origin(&origin("se_f", "se_p", 2)).unwrap();
    store.put_archive(&archived("se_p", ArchiveState::Freed)).unwrap();
    store.remove_session("se_p").unwrap();
    assert_eq!(store.fork_origin("se_p").unwrap(), None);
    assert_eq!(store.archive("se_p").unwrap(), None);
    // FK-13: the fork keeps naming where it came from.
    assert_eq!(store.fork_origin("se_f").unwrap(), Some(origin("se_f", "se_p", 2)));
}

#[test]
fn ids_that_are_not_ids_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    assert!(store.fork_origin("").is_err());
    assert!(store.archive("").is_err());
}
