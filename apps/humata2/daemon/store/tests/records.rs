use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use humata2_store::{
    Pointers, RepoPointer, SCHEMA_VERSION, SessionRecord, Store, StoreError, TabKind, TabRecord,
};

fn session(id: &str, created_unix_ms: i64) -> SessionRecord {
    SessionRecord {
        id: id.to_owned(),
        channel: "ch_1".to_owned(),
        machine: "ma_1".to_owned(),
        title: format!("Session {id}"),
        agent: "claude".to_owned(),
        folder: PathBuf::from("/work/humata"),
        created_unix_ms,
    }
}

fn tab(id: &str, session: Option<&str>) -> TabRecord {
    TabRecord {
        id: id.to_owned(),
        session: session.map(str::to_owned),
        kind: TabKind::Shell,
        title: "zsh".to_owned(),
        holder_socket: PathBuf::from(format!("/tmp/{id}.sock")),
        holder_pid: 4242,
        created_unix_ms: 7,
        holder_generation: 0,
    }
}

#[test]
fn records_survive_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let pointers = Pointers {
        repos: vec![RepoPointer {
            owner: "acme".to_owned(),
            name: "web".to_owned(),
            branch: "main".to_owned(),
            sha: "0123abcd".to_owned(),
            pushed: false,
            dirty_files: vec!["src/main.rs".to_owned()],
        }],
        reported_unix_ms: 99,
    };
    {
        let store = Store::open(dir.path()).unwrap();
        store.put_session(&session("se_b", 2)).unwrap();
        store.put_session(&session("se_a", 1)).unwrap();
        store.put_tab(&tab("tb_1", Some("se_a"))).unwrap();
        store.put_tab(&tab("tb_machine", None)).unwrap();
        store.put_pointers("se_a", &pointers).unwrap();
    }

    let store = Store::open(dir.path()).unwrap();
    assert_eq!(store.session("se_a").unwrap(), Some(session("se_a", 1)));
    assert_eq!(
        store.sessions().unwrap(),
        vec![session("se_a", 1), session("se_b", 2)]
    );
    assert_eq!(store.tab("tb_1").unwrap(), Some(tab("tb_1", Some("se_a"))));
    assert_eq!(store.tabs().unwrap().len(), 2);
    assert_eq!(store.pointers("se_a").unwrap(), Some(pointers));
    assert_eq!(store.pointers("se_b").unwrap(), None);
    assert_eq!(store.session("se_missing").unwrap(), None);
}

#[test]
fn a_tab_keeps_its_holder_generation() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let mut saved = tab("tab_g", None);
    saved.holder_generation = 3;
    store.put_tab(&saved).unwrap();
    assert_eq!(store.tab("tab_g").unwrap().unwrap().holder_generation, 3);

    saved.holder_generation = 4;
    store.put_tab(&saved).unwrap();
    drop(store);
    let reopened = Store::open(dir.path()).unwrap();
    assert_eq!(reopened.tab("tab_g").unwrap().unwrap(), saved);
    assert_eq!(reopened.tabs().unwrap(), vec![saved]);
}

#[test]
fn a_tab_saved_before_generations_reads_as_generation_zero() {
    let dir = tempfile::tempdir().unwrap();
    drop(Store::open(dir.path()).unwrap());
    // A row as the older schema wrote it: no generation column given.
    rusqlite::Connection::open(dir.path().join("state.sqlite"))
        .unwrap()
        .execute(
            "INSERT INTO tabs (id, session, kind, title, holder_socket, holder_pid, created_unix_ms)
             VALUES ('tab_old', NULL, 'shell', 'zsh', ?1, 9, 1)",
            [b"/tmp/tab_old.sock".as_slice()],
        )
        .unwrap();

    let store = Store::open(dir.path()).unwrap();

    assert_eq!(store.tab("tab_old").unwrap().unwrap().holder_generation, 0);
}

#[test]
fn a_write_replaces_the_record() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.put_session(&session("se_a", 1)).unwrap();
    let renamed = SessionRecord {
        title: "Renamed".to_owned(),
        ..session("se_a", 1)
    };
    store.put_session(&renamed).unwrap();
    assert_eq!(store.session("se_a").unwrap(), Some(renamed));
    assert_eq!(store.sessions().unwrap().len(), 1);
}

#[test]
fn removing_a_session_takes_its_tabs_pointers_and_pastes() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.put_session(&session("se_a", 1)).unwrap();
    store.put_session(&session("se_b", 2)).unwrap();
    store.put_tab(&tab("tb_a", Some("se_a"))).unwrap();
    store.put_tab(&tab("tb_b", Some("se_b"))).unwrap();
    store.put_pointers("se_a", &Pointers::default()).unwrap();
    let paste = store.save_paste("se_a", "png", b"image").unwrap();
    assert!(paste.exists());

    store.remove_session("se_a").unwrap();

    assert_eq!(store.session("se_a").unwrap(), None);
    assert_eq!(store.tab("tb_a").unwrap(), None);
    assert_eq!(store.pointers("se_a").unwrap(), None);
    assert!(!paste.exists());
    assert_eq!(store.session("se_b").unwrap(), Some(session("se_b", 2)));
    assert_eq!(store.tab("tb_b").unwrap(), Some(tab("tb_b", Some("se_b"))));
}

#[test]
fn a_paste_is_kept_privately_under_its_session() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.put_session(&session("se_a", 1)).unwrap();

    let first = store.save_paste("se_a", "png", b"first").unwrap();
    let second = store.save_paste("se_a", "png", b"second").unwrap();

    assert_ne!(first, second);
    assert_eq!(fs::read(&first).unwrap(), b"first");
    assert_eq!(fs::read(&second).unwrap(), b"second");
    assert!(first.starts_with(dir.path()));
    let mode = fs::metadata(&first).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
}

/// A paste a call names with a key is saved once for that key, even by a
/// store opened again after a restart; the first bytes stay. Another key is
/// another paste.
#[test]
fn a_keyed_paste_is_saved_once_across_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.put_session(&session("se_a", 1)).unwrap();
    let first = store.save_keyed_paste("se_a", "key-1", "png", b"first").unwrap();
    drop(store);

    let store = Store::open(dir.path()).unwrap();
    let again = store.save_keyed_paste("se_a", "key-1", "png", b"retried").unwrap();
    let other = store.save_keyed_paste("se_a", "key-2", "png", b"other").unwrap();

    assert_eq!(again, first);
    assert_eq!(fs::read(&first).unwrap(), b"first");
    assert_ne!(other, first);
    assert_eq!(fs::read(&other).unwrap(), b"other");
    let folder = first.parent().unwrap();
    assert_eq!(fs::read_dir(folder).unwrap().count(), 2);
    let mode = fs::metadata(&first).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    assert!(matches!(
        store.save_keyed_paste("se_gone", "key-1", "png", b"image"),
        Err(StoreError::NoSession { .. })
    ));
}

#[test]
fn a_paste_needs_its_session() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let refused = store.save_paste("se_gone", "png", b"image");
    assert!(matches!(refused, Err(StoreError::NoSession { .. })));
}

#[test]
fn a_sessions_paste_folder_is_made_privately_and_goes_with_it() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.put_session(&session("se_a", 1)).unwrap();

    let folder = store.paste_folder("se_a").unwrap();
    assert!(folder.is_dir());
    assert!(folder.starts_with(dir.path()));
    let mode = fs::metadata(&folder).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700);
    let image = store.save_paste("se_a", "png", b"image").unwrap();
    assert_eq!(image.parent(), Some(folder.as_path()));
    assert!(matches!(
        store.paste_folder("se_gone"),
        Err(StoreError::NoSession { .. })
    ));

    store.remove_session("se_a").unwrap();
    assert!(!folder.exists());
}

#[test]
fn ids_that_could_leave_their_folder_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    for id in ["", "../escape", "a/b", ".hidden", &"x".repeat(129)] {
        let refused = store.put_session(&session(id, 1));
        assert!(
            matches!(refused, Err(StoreError::InvalidId { .. })),
            "{id:?} was accepted"
        );
        assert!(matches!(
            store.session(id),
            Err(StoreError::InvalidId { .. })
        ));
    }
}

#[test]
fn a_tab_or_pointers_of_an_unsaved_session_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let tab = store.put_tab(&tab("tb_1", Some("se_gone")));
    assert!(matches!(tab, Err(StoreError::NoSession { id }) if id == "se_gone"));
    let pointers = store.put_pointers("se_gone", &Pointers::default());
    assert!(matches!(pointers, Err(StoreError::NoSession { id }) if id == "se_gone"));
    assert!(store.tabs().unwrap().is_empty());
}

#[test]
fn rewriting_a_session_keeps_its_tabs() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.put_session(&session("se_a", 1)).unwrap();
    store.put_tab(&tab("tb_a", Some("se_a"))).unwrap();

    store.put_session(&session("se_a", 1)).unwrap();

    assert_eq!(store.tab("tb_a").unwrap(), Some(tab("tb_a", Some("se_a"))));
}

#[test]
fn state_from_a_newer_schema_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    drop(Store::open(dir.path()).unwrap());
    let newer = SCHEMA_VERSION + 1;
    rusqlite::Connection::open(dir.path().join("state.sqlite"))
        .unwrap()
        .pragma_update(None, "user_version", newer)
        .unwrap();

    match Store::open(dir.path()) {
        Err(StoreError::SchemaTooNew { found, supported, .. }) => {
            assert_eq!(found, newer);
            assert_eq!(supported, SCHEMA_VERSION);
        }
        Err(other) => panic!("refused for another reason: {other}"),
        Ok(_) => panic!("opened state from a newer schema"),
    }
}

#[test]
fn files_and_folders_are_private() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("state");
    let store = Store::open(&root).unwrap();
    store.put_session(&session("se_a", 1)).unwrap();
    store.journal().append("big", &vec![7; 1 << 20]).unwrap();

    let mode = |path: PathBuf| fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(root.clone()), 0o700);
    assert_eq!(mode(root.join("blobs")), 0o700);
    for entry in fs::read_dir(&root).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_file() {
            assert_eq!(mode(entry.path()), 0o600, "{:?} is readable by others", entry.path());
        }
    }
}

#[test]
fn each_committed_write_is_told_and_a_refused_one_is_not() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let told = Arc::new(AtomicUsize::new(0));
    let counting = told.clone();
    store.on_commit(move || {
        counting.fetch_add(1, Ordering::SeqCst);
    });

    store.put_session(&session("se_a", 1)).unwrap();
    store.put_tab(&tab("tb_1", Some("se_a"))).unwrap();
    assert!(store.put_tab(&tab("tb_2", Some("se_missing"))).is_err());
    store.session("se_a").unwrap();

    assert_eq!(told.load(Ordering::SeqCst), 2);
}
