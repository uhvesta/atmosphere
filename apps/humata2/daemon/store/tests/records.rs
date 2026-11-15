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

#[test]
fn a_paste_needs_its_session() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let refused = store.save_paste("se_gone", "png", b"image");
    assert!(matches!(refused, Err(StoreError::NoSession { .. })));
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
