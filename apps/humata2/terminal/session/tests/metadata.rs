use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::{
    fs::OpenOptions,
    io::Write,
    os::unix::{
        ffi::OsStrExt,
        fs::{OpenOptionsExt, PermissionsExt, symlink},
    },
    path::{Path, PathBuf},
    sync::mpsc,
    time::Duration,
};
use terminal_session::{Endpoint, NameSource, SessionSpec, SessionStore};

fn state() -> tempfile::TempDir {
    let dir = tempfile::tempdir_in("/tmp").unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    dir
}
fn workspace(count: usize) -> Value {
    let sessions=(1..=count).map(|id|json!({"id":id,"uuid":format!("{id:08x}-0000-4000-8000-{id:012x}"),"automatic_title":"fixture","spec":{"title":"fixture","program":"/bin/sh","cwd":"/tmp","cols":80,"rows":24,"endpoint":"Local"},"recovery":null})).collect::<Vec<_>>();
    json!({"namespace":"0123456789abcdef0123456789abcdef","version":3,"next":count+1,"sessions":sessions,"groups":[(1..=count).collect::<Vec<_>>(),[]],"selected":[if count>0{json!(1)}else{Value::Null},null]})
}
fn write(root: &Path, value: &Value) -> Vec<u8> {
    let bytes = serde_json::to_vec(value).unwrap();
    std::fs::write(root.join("workspace.json"), &bytes).unwrap();
    bytes
}
fn open(root: &Path) -> Result<SessionStore, terminal_session::Error> {
    // Capacity failures must happen before any holder could be launched.
    SessionStore::open_memory_only(root, PathBuf::from("/nonexistent/metadata-test-holder"))
}
fn spec() -> SessionSpec {
    SessionSpec {
        title: "new".into(),
        program: "/bin/sh".into(),
        args: vec![],
        cwd: "/tmp".into(),
        cols: 80,
        rows: 24,
        endpoint: Endpoint::Local,
    }
}

#[test]
fn oversized_but_valid_workspace_json_is_rejected_without_rewriting_it() {
    let dir = state();
    let mut bytes = serde_json::to_vec(&workspace(0)).unwrap();
    bytes.resize((16 << 20) + 1, b' ');
    std::fs::write(dir.path().join("workspace.json"), &bytes).unwrap();
    assert!(
        open(dir.path()).is_err(),
        "valid trailing whitespace must not bypass the metadata size budget"
    );
    assert_eq!(
        std::fs::metadata(dir.path().join("workspace.json"))
            .unwrap()
            .len(),
        bytes.len() as u64
    );
}

#[test]
fn duplicate_session_ids_are_rejected() {
    let dir = state();
    let mut value = workspace(2);
    value["sessions"][1]["id"] = json!(1);
    write(dir.path(), &value);
    assert!(open(dir.path()).is_err());
}

#[test]
fn unknown_and_repeated_group_references_are_rejected() {
    for groups in [json!([[1, 99], []]), json!([[1, 1], []]), json!([[1], [1]])] {
        let dir = state();
        let mut value = workspace(1);
        value["groups"] = groups;
        write(dir.path(), &value);
        assert!(open(dir.path()).is_err());
    }
}

#[test]
fn selected_session_must_belong_to_its_group_but_legacy_none_is_valid() {
    let dir = state();
    let mut value = workspace(2);
    value["groups"] = json!([[1], [2]]);
    value["selected"] = json!([2, null]);
    write(dir.path(), &value);
    assert!(open(dir.path()).is_err());
    value["selected"] = json!([null, null]);
    write(dir.path(), &value);
    assert!(open(dir.path()).is_ok());
}

#[test]
fn workspace_metadata_rejects_a_symlink_to_an_otherwise_valid_file() {
    let dir = state();
    let outside = dir.path().join("unrelated.json");
    std::fs::write(&outside, serde_json::to_vec(&workspace(0)).unwrap()).unwrap();
    symlink(outside, dir.path().join("workspace.json")).unwrap();
    assert!(open(dir.path()).is_err());
}

#[test]
fn workspace_metadata_rejects_a_fifo_without_waiting_for_eof() {
    let dir = state();
    let path = dir.path().join("workspace.json");
    let cpath = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    // SAFETY: cpath names an owned temporary file and stays live for mkfifo.
    assert_eq!(unsafe { libc::mkfifo(cpath.as_ptr(), 0o600) }, 0);
    let mut writer = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(&path)
        .unwrap();
    writer
        .write_all(&serde_json::to_vec(&workspace(0)).unwrap())
        .unwrap();
    let root = dir.path().to_owned();
    let (tx, rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let _ = tx.send(open(&root).is_err());
    });
    let early = rx.recv_timeout(Duration::from_millis(500));
    // Unblock the pre-fix reader before asserting, leaving no worker behind.
    drop(writer);
    worker.join().unwrap();
    assert!(
        matches!(early, Ok(true)),
        "non-regular metadata blocked or was accepted: {early:?}"
    );
}

#[test]
fn creating_a_257th_session_preserves_metadata_and_does_not_attempt_launch() {
    let dir = state();
    write(dir.path(), &workspace(257));
    assert!(open(dir.path()).is_err());
    let before = write(dir.path(), &workspace(256));
    let mut store = open(dir.path()).unwrap();
    let mut files_before = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    files_before.sort();
    assert!(store.create(spec()).is_err());
    let mut files_after = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    files_after.sort();
    assert_eq!(
        files_after, files_before,
        "capacity rejection must not create holder logs, sockets or journals"
    );
    assert_eq!(
        store.sessions().len(),
        256,
        "capacity must be checked before mutating the session table"
    );
    assert_eq!(
        std::fs::read(dir.path().join("workspace.json")).unwrap(),
        before
    );
}

#[test]
fn oversized_metadata_save_rolls_back_the_memory_and_preserves_the_file() {
    let dir = state();
    let before = write(dir.path(), &workspace(1));
    let mut store = open(dir.path()).unwrap();
    let giant = PathBuf::from(format!("/{}", "x".repeat(16 << 20)));
    assert!(store.set_cwd(1, giant).is_err());
    assert_eq!(store.session(1).unwrap().spec.cwd, Path::new("/tmp"));
    assert_eq!(
        std::fs::read(dir.path().join("workspace.json")).unwrap(),
        before
    );
}

#[test]
fn exhausted_identifier_space_does_not_write_an_unreopenable_manifest() {
    let dir = state();
    let mut value = workspace(1);
    value["next"] = json!(u64::MAX - 1);
    let before = write(dir.path(), &value);
    let mut store = open(dir.path()).unwrap();
    assert!(store.create(spec()).is_err());
    assert_eq!(store.sessions().len(), 1);
    assert_eq!(
        std::fs::read(dir.path().join("workspace.json")).unwrap(),
        before
    );
}

#[test]
fn a_failed_startup_launch_keeps_one_durable_idempotent_reservation() {
    let dir = state();
    write(dir.path(), &workspace(0));
    let mut store = open(dir.path()).unwrap();
    assert!(store.create_startup(spec(), "launch-once").is_err());
    assert_eq!(store.sessions().len(), 1);
    assert_eq!(
        store.sessions()[0].startup_profile.as_deref(),
        Some("launch-once")
    );
    drop(store);
    let mut store = open(dir.path()).unwrap();
    let before = std::fs::read(dir.path().join("workspace.json")).unwrap();
    assert_eq!(store.create_startup(spec(), "launch-once").unwrap(), 1);
    assert_eq!(store.sessions().len(), 1);
    assert_eq!(
        std::fs::read(dir.path().join("workspace.json")).unwrap(),
        before
    );
    // Explicit New still reserves a distinct instance, even after a failed
    // launch. It must not accidentally become startup-key deduplication.
    assert!(store.create(spec()).is_err());
    assert_eq!(store.sessions().len(), 2);
    assert!(store.sessions()[1].startup_profile.is_none());
}

#[test]
fn an_existing_startup_reservation_is_returned_even_at_workspace_capacity() {
    let dir = state();
    let mut value = workspace(256);
    value["sessions"][0]["startup_profile"] = json!("existing");
    let before = write(dir.path(), &value);
    let mut store = open(dir.path()).unwrap();
    assert_eq!(store.create_startup(spec(), "existing").unwrap(), 1);
    assert!(store.create_startup(spec(), "new-key").is_err());
    assert_eq!(store.sessions().len(), 256);
    assert_eq!(
        std::fs::read(dir.path().join("workspace.json")).unwrap(),
        before
    );
}

#[test]
fn legacy_startup_binding_is_durable_and_refuses_conflicting_identities() {
    let dir = state();
    write(dir.path(), &workspace(2));
    let mut store = open(dir.path()).unwrap();
    store.bind_startup_profile(1, "first").unwrap();
    let before = std::fs::read(dir.path().join("workspace.json")).unwrap();
    assert!(store.bind_startup_profile(2, "first").is_err());
    assert!(store.bind_startup_profile(1, "different").is_err());
    assert_eq!(
        std::fs::read(dir.path().join("workspace.json")).unwrap(),
        before
    );
    store.bind_startup_profile(1, "first").unwrap();
    drop(store);
    let mut store = open(dir.path()).unwrap();
    assert_eq!(store.create_startup(spec(), "first").unwrap(), 1);
    assert_eq!(store.sessions().len(), 2);
}

#[test]
fn startup_keys_are_bounded_unique_and_free_of_control_characters() {
    for invalid in [String::new(), "x".repeat(65), "line\nbreak".into()] {
        let dir = state();
        let mut value = workspace(1);
        value["sessions"][0]["startup_profile"] = json!(invalid);
        write(dir.path(), &value);
        assert!(open(dir.path()).is_err());
    }
    let dir = state();
    let mut value = workspace(2);
    value["sessions"][0]["startup_profile"] = json!("same");
    value["sessions"][1]["startup_profile"] = json!("same");
    write(dir.path(), &value);
    assert!(open(dir.path()).is_err());
}

#[test]
fn hardlinked_temporary_metadata_cannot_truncate_an_unrelated_file() {
    let dir = state();
    let before = write(dir.path(), &workspace(1));
    let mut store = open(dir.path()).unwrap();
    let unrelated = dir.path().join("unrelated-owned-fixture");
    std::fs::write(&unrelated, b"PRESERVE_UNRELATED_CONTENT").unwrap();
    std::fs::hard_link(&unrelated, dir.path().join("workspace.tmp")).unwrap();
    assert!(store.set_cwd(1, PathBuf::from("/new")).is_err());
    assert_eq!(
        std::fs::read(unrelated).unwrap(),
        b"PRESERVE_UNRELATED_CONTENT"
    );
    assert_eq!(
        std::fs::read(dir.path().join("workspace.json")).unwrap(),
        before
    );
    assert_eq!(store.session(1).unwrap().spec.cwd, Path::new("/tmp"));
}

#[test]
fn legacy_identity_migration_preserves_routes_and_assigns_durable_unique_uuids() {
    for version in [1, 2] {
        let dir = state();
        let mut value = workspace(2);
        value["version"] = json!(version);
        for session in value["sessions"].as_array_mut().unwrap() {
            session.as_object_mut().unwrap().remove("uuid");
            session.as_object_mut().unwrap().remove("automatic_title");
            session["spec"]["args"] = json!(["LEGACY_ARGUMENT_SECRET"]);
        }
        write(dir.path(), &value);
        let store = open(dir.path()).unwrap();
        let uuids = store
            .sessions()
            .iter()
            .map(|session| session.uuid.clone())
            .collect::<Vec<_>>();
        assert_ne!(uuids[0], uuids[1]);
        for (index, uuid) in uuids.iter().enumerate() {
            assert_eq!(uuid.len(), 36);
            assert_eq!(store.session_by_uuid(uuid).unwrap().id, index as u64 + 1);
        }
        drop(store);
        let bytes = std::fs::read(dir.path().join("workspace.json")).unwrap();
        let migrated: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(migrated["version"], json!(3));
        assert_eq!(migrated["namespace"], value["namespace"]);
        assert_eq!(migrated["groups"], value["groups"]);
        assert!(!String::from_utf8_lossy(&bytes).contains("LEGACY_ARGUMENT_SECRET"));
        let store = open(dir.path()).unwrap();
        assert_eq!(
            store
                .sessions()
                .iter()
                .map(|session| session.uuid.clone())
                .collect::<Vec<_>>(),
            uuids
        );
        assert_eq!(
            std::fs::read(dir.path().join("workspace.json")).unwrap(),
            bytes
        );
    }
}

#[test]
fn automatic_and_explicit_names_keep_identity_and_have_deliberate_override_semantics() {
    let dir = state();
    write(dir.path(), &workspace(1));
    let mut store = open(dir.path()).unwrap();
    let uuid = store.session(1).unwrap().uuid.clone();
    store.set_automatic_title(1, "Runtime one".into()).unwrap();
    store
        .set_name_override(1, "My terminal".into(), NameSource::Manual)
        .unwrap();
    store.set_automatic_title(1, "Runtime two".into()).unwrap();
    assert_eq!(store.session(1).unwrap().display_title(), "My terminal");
    assert_eq!(store.session(1).unwrap().automatic_title, "Runtime two");
    store
        .set_name_override(1, "Agent chose a label".into(), NameSource::Programmatic)
        .unwrap();
    drop(store);
    let mut store = open(dir.path()).unwrap();
    let session = store.session_by_uuid(&uuid).unwrap();
    assert_eq!(session.display_title(), "Agent chose a label");
    assert_eq!(
        session.name_override.as_ref().unwrap().source,
        NameSource::Programmatic
    );
    store
        .set_automatic_title(1, "Runtime three".into())
        .unwrap();
    assert_eq!(
        store.session(1).unwrap().display_title(),
        "Agent chose a label"
    );
    store.clear_name_override(1).unwrap();
    assert_eq!(store.session(1).unwrap().display_title(), "Runtime three");
    assert_eq!(store.session(1).unwrap().uuid, uuid);
    assert_eq!(store.session(1).unwrap().spec.title, "fixture");
    for invalid in [String::new(), "x".repeat(513), "line\nbreak".into()] {
        assert!(store.set_automatic_title(1, invalid.clone()).is_err());
        assert!(
            store
                .set_name_override(1, invalid, NameSource::Manual)
                .is_err()
        );
        assert_eq!(store.session(1).unwrap().display_title(), "Runtime three");
    }
    assert!(matches!(
        store.session_by_uuid("00000000-0000-4000-8000-ffffffffffff"),
        Err(terminal_session::Error::MissingUuid { .. })
    ));
    assert!(store.session_by_uuid("1").is_err());
}

#[test]
fn explicit_attributes_persist_and_invalid_updates_preserve_saved_and_live_metadata() {
    let dir = state();
    write(dir.path(), &workspace(1));
    let mut store = open(dir.path()).unwrap();
    let attributes = BTreeMap::from([
        ("project".into(), "payments".into()),
        ("empty-flag".into(), String::new()),
    ]);
    store.set_attributes(1, attributes.clone()).unwrap();
    let before = std::fs::read(dir.path().join("workspace.json")).unwrap();
    let too_many = (0..33)
        .map(|i| (format!("key{i}"), "value".into()))
        .collect();
    let too_large_total = (0..9)
        .map(|i| (format!("key{i}"), "x".repeat(4096)))
        .collect();
    for invalid in [
        too_many,
        too_large_total,
        BTreeMap::from([("x".repeat(65), "v".into())]),
        BTreeMap::from([("k".into(), "v".repeat(4097))]),
        BTreeMap::from([("bad\nkey".into(), "v".into())]),
    ] {
        assert!(store.set_attributes(1, invalid).is_err());
        assert_eq!(store.session(1).unwrap().attributes, attributes);
        assert_eq!(
            std::fs::read(dir.path().join("workspace.json")).unwrap(),
            before
        );
    }
    drop(store);
    let store = open(dir.path()).unwrap();
    assert_eq!(store.session(1).unwrap().attributes, attributes);
}

#[test]
fn malformed_or_duplicate_v3_uuids_are_rejected_without_repairing_identity() {
    for invalid in [
        json!("not-a-uuid"),
        json!("00000000-0000-0000-0000-000000000000"),
        json!("ABCDEFAB-0000-4000-8000-000000000001"),
        Value::Null,
    ] {
        let dir = state();
        let mut value = workspace(1);
        if invalid.is_null() {
            value["sessions"][0].as_object_mut().unwrap().remove("uuid");
        } else {
            value["sessions"][0]["uuid"] = invalid;
        }
        let before = write(dir.path(), &value);
        assert!(open(dir.path()).is_err());
        assert_eq!(
            std::fs::read(dir.path().join("workspace.json")).unwrap(),
            before
        );
    }
    let dir = state();
    let mut value = workspace(2);
    value["sessions"][1]["uuid"] = value["sessions"][0]["uuid"].clone();
    write(dir.path(), &value);
    assert!(open(dir.path()).is_err());
}

#[test]
fn failed_launch_reservation_keeps_its_uuid_and_new_instances_get_new_uuids() {
    let dir = state();
    write(dir.path(), &workspace(0));
    let mut store = open(dir.path()).unwrap();
    assert!(store.create_startup(spec(), "once").is_err());
    let uuid = store.session(1).unwrap().uuid.clone();
    drop(store);
    let mut store = open(dir.path()).unwrap();
    assert_eq!(store.create_startup(spec(), "once").unwrap(), 1);
    assert_eq!(store.session_by_uuid(&uuid).unwrap().id, 1);
    assert!(store.create(spec()).is_err());
    assert_ne!(store.session(2).unwrap().uuid, uuid);
}

#[test]
fn name_and_attributes_roll_back_when_atomic_metadata_write_fails() {
    let dir = state();
    let before = write(dir.path(), &workspace(1));
    let mut store = open(dir.path()).unwrap();
    std::fs::create_dir(dir.path().join("workspace.tmp")).unwrap();
    assert!(store.set_automatic_title(1, "new runtime".into()).is_err());
    assert!(
        store
            .set_name_override(1, "new override".into(), NameSource::Manual)
            .is_err()
    );
    assert!(
        store
            .set_attributes(1, BTreeMap::from([("project".into(), "x".into())]))
            .is_err()
    );
    assert_eq!(store.session(1).unwrap().display_title(), "fixture");
    assert!(store.session(1).unwrap().name_override.is_none());
    assert!(store.session(1).unwrap().attributes.is_empty());
    assert_eq!(
        std::fs::read(dir.path().join("workspace.json")).unwrap(),
        before
    );
}

#[test]
fn failed_migration_does_not_publish_partial_uuids_or_change_the_legacy_manifest() {
    let dir = state();
    let mut value = workspace(1);
    value["version"] = json!(2);
    value["sessions"][0].as_object_mut().unwrap().remove("uuid");
    let before = write(dir.path(), &value);
    std::fs::create_dir(dir.path().join("workspace.tmp")).unwrap();
    assert!(open(dir.path()).is_err());
    assert_eq!(
        std::fs::read(dir.path().join("workspace.json")).unwrap(),
        before
    );
    std::fs::remove_dir(dir.path().join("workspace.tmp")).unwrap();
    assert_eq!(open(dir.path()).unwrap().sessions()[0].uuid.len(), 36);
}
