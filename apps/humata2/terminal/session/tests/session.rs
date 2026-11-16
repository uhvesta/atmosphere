use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};
use terminal_session::{Endpoint, Record, RecoveryHint, SessionSpec, SessionStore, Update};

#[test]
fn missing_program_keeps_atomic_recovery_metadata_and_retry_never_launches_again() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::Builder::new()
        .prefix("resume-reservation-")
        .tempdir_in("/tmp")
        .unwrap();
    secure(dir.path());
    let hint = RecoveryHint {
        provider: "fixture-provider".into(),
        session_id: "saved-conversation-1".into(),
        working_directory: "/tmp".into(),
        state_paths: vec!["/tmp/provider-state".into()],
    };
    let attributes = BTreeMap::from([
        (
            "terminal.resume.source".into(),
            "original-terminal-uuid".into(),
        ),
        ("terminal.resume.profile".into(), "trusted-agent".into()),
    ]);
    let mut store = SessionStore::open_memory_only(dir.path(), holder()).unwrap();
    let mut missing = spec();
    missing.program = dir.path().join("missing-agent-program");
    assert!(
        store
            .create_startup_with_metadata(
                missing,
                "resume-once",
                Some(hint.clone()),
                attributes.clone()
            )
            .is_err()
    );
    let saved = store.session(1).unwrap();
    let uuid = saved.uuid.clone();
    assert_eq!(saved.recovery.as_ref(), Some(&hint));
    assert_eq!(saved.attributes, attributes);
    drop(store);
    let before = std::fs::read(dir.path().join("workspace.json")).unwrap();
    let persisted: serde_json::Value = serde_json::from_slice(&before).unwrap();
    assert_eq!(
        persisted["sessions"][0]["recovery"],
        serde_json::to_value(&hint).unwrap()
    );
    assert_eq!(
        persisted["sessions"][0]["attributes"],
        serde_json::to_value(&attributes).unwrap()
    );

    // If retry executes any holder, this executable leaves an observable marker.
    let launcher = dir.path().join("retry-holder");
    std::fs::write(
        &launcher,
        "#!/bin/sh\nprintf attempted > \"$(dirname \"$0\")/retry-attempted\"\nexit 1\n",
    )
    .unwrap();
    std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut store = SessionStore::open_memory_only(dir.path(), launcher).unwrap();
    let mut replacement = hint.clone();
    replacement.session_id = "must-not-replace-the-reservation".into();
    assert_eq!(
        store
            .create_startup_with_metadata(spec(), "resume-once", Some(replacement), BTreeMap::new())
            .unwrap(),
        1
    );
    let saved = store.session(1).unwrap();
    assert_eq!(saved.uuid, uuid);
    assert_eq!(saved.recovery.as_ref(), Some(&hint));
    assert_eq!(saved.attributes, attributes);
    assert_eq!(store.sessions().len(), 1);
    assert!(!dir.path().join("retry-attempted").exists());
    assert_eq!(
        std::fs::read(dir.path().join("workspace.json")).unwrap(),
        before
    );
}
fn secure(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
}
fn holder() -> PathBuf {
    std::fs::canonicalize(std::env::var("HUMATA2_HOLDER").unwrap()).unwrap()
}
fn spec() -> SessionSpec {
    SessionSpec {
        title: "shell".into(),
        program: "/bin/sh".into(),
        args: vec!["-i".into()],
        cwd: "/tmp".into(),
        cols: 80,
        rows: 24,
        endpoint: Endpoint::Local,
    }
}
async fn until(reader: &mut terminal_session::Reader, needle: &str) -> Vec<u8> {
    tokio::time::timeout(Duration::from_secs(8), async {
        let mut out = Vec::new();
        loop {
            match reader.next().await.unwrap() {
                Some(Update::Record(Record::Output { bytes, .. })) => out.extend(bytes),
                Some(_) => {}
                None => panic!("terminal disconnected"),
            };
            if String::from_utf8_lossy(&out).contains(needle) {
                return out;
            }
        }
    })
    .await
    .expect("terminal output arrives")
}
#[tokio::test]
async fn two_real_terminals_move_resize_detach_and_resume_without_restarting() {
    let dir = tempfile::Builder::new()
        .prefix("tl-")
        .tempdir_in("/tmp")
        .unwrap();
    let binary = holder();
    secure(dir.path());
    let mut store = SessionStore::open(dir.path(), binary.clone()).unwrap();
    let a = store.create(spec()).unwrap();
    let b = store.create(spec()).unwrap();
    let mut first = store.attach(a).await.unwrap();
    let mut second = store.attach(b).await.unwrap();
    assert!(matches!(
        store.attach(a).await,
        Err(terminal_session::Error::Attached { .. })
    ));
    first
        .writer
        .input(b"export LAB_KEEP=survives; printf 'ID=%s\\n' $$\r")
        .await
        .unwrap();
    let initial = until(&mut first.reader, "ID=").await;
    assert!(!initial.is_empty());
    second
        .writer
        .input(b"export LAB_KEEP=other; printf 'SECOND_READY\\n'\r")
        .await
        .unwrap();
    until(&mut second.reader, "SECOND_READY").await;
    first.writer.resize(101, 31).await.unwrap();
    first.writer.input(b"stty size\r").await.unwrap();
    until(&mut first.reader, "31 101").await;
    store.move_to(a, 1, 0).unwrap();
    store.move_to(b, 1, 0).unwrap();
    assert_eq!(store.groups()[1], vec![b, a]);
    assert_eq!(store.selected()[1], Some(b));
    drop(first);
    drop(second);
    drop(store);
    let store = SessionStore::open(dir.path(), binary).unwrap();
    assert_eq!(store.groups()[1], vec![b, a]);
    assert_eq!(store.selected()[1], Some(b));
    let mut first = store.attach(a).await.unwrap();
    assert_eq!((first.cols, first.rows), (101, 31));
    assert!(first.history.iter().any(|r| matches!(
        r,
        Record::Resize {
            cols: 101,
            rows: 31
        }
    )));
    first
        .writer
        .input(b"printf 'RESTORED=%s\\n' \"$LAB_KEEP\"\r")
        .await
        .unwrap();
    until(&mut first.reader, "RESTORED=survives").await;
    let mut second = store.attach(b).await.unwrap();
    second
        .writer
        .input(b"printf 'ISOLATED=%s\\n' \"$LAB_KEEP\"\r")
        .await
        .unwrap();
    until(&mut second.reader, "ISOLATED=other").await;
    first.writer.stop().await.unwrap();
    second.writer.stop().await.unwrap();
    drop(first);
    drop(second);
    assert!(!store.history(a).unwrap().is_empty());
}
#[test]
fn workspace_has_one_owner_and_rejects_invalid_sizes_without_starting() {
    let d = tempfile::tempdir_in("/tmp").unwrap();
    secure(d.path());
    let mut s = SessionStore::open(d.path(), holder()).unwrap();
    assert!(matches!(
        SessionStore::open(d.path(), holder()),
        Err(terminal_session::Error::Locked { .. })
    ));
    let mut bad = spec();
    bad.cols = 0;
    assert!(s.create(bad).is_err());
    assert!(s.sessions().is_empty());
}
#[tokio::test]
async fn dead_process_is_not_silently_replaced_and_saved_output_is_kept() {
    let d = tempfile::tempdir_in("/tmp").unwrap();
    secure(d.path());
    let mut s = SessionStore::open(d.path(), holder()).unwrap();
    let id = s.create(spec()).unwrap();
    let mut a = s.attach(id).await.unwrap();
    a.writer.input(b"printf 'ON_DISK\\n'\r").await.unwrap();
    until(&mut a.reader, "ON_DISK").await;
    a.writer.stop().await.unwrap();
    drop(a);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(s.attach(id).await.is_err());
    assert!(s.history(id).unwrap().iter().any(|r|matches!(r,Record::Output{bytes,..} if String::from_utf8_lossy(bytes).contains("ON_DISK"))));
}
#[test]
fn partial_final_journal_record_is_repaired_but_corruption_is_reported() {
    use std::io::Write;
    let d = tempfile::tempdir_in("/tmp").unwrap();
    secure(d.path());
    let mut s = SessionStore::open(d.path(), holder()).unwrap();
    let mut bad = spec();
    bad.program = Path::new("/not/a/program").into();
    assert!(s.create(bad).is_err());
    let p = d.path().join("s1.journal");
    std::fs::write(&p, b"partial").unwrap();
    assert!(s.history(1).unwrap().is_empty());
    assert_eq!(std::fs::metadata(&p).unwrap().len(), 0);
    let mut f = std::fs::File::create(&p).unwrap();
    f.write_all(&1u32.to_le_bytes()).unwrap();
    f.write_all(&0u64.to_le_bytes()).unwrap();
    f.write_all(b"x").unwrap();
    assert!(matches!(
        s.history(1),
        Err(terminal_session::Error::Journal { .. })
    ));
}

#[test]
fn crash_client_fixture() {
    let Some(root) = std::env::var_os("TERMINAL_CRASH_FIXTURE") else {
        return;
    };
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let root = PathBuf::from(root);
        let mut store = SessionStore::open(&root, holder()).unwrap();
        let id = store.create(spec()).unwrap();
        let mut a = store.attach(id).await.unwrap();
        let command = format!(
            "stty -echo; export CRASH_KEEP=kept; printf '%s' $$ > '{}'; printf 'READY_CRASH\\n'\r",
            root.join("shell.pid").display()
        );
        a.writer.input(command.as_bytes()).await.unwrap();
        until(&mut a.reader, "READY_CRASH\r\n").await;
        std::fs::write(root.join("client.ready"), b"ready").unwrap();
        std::future::pending::<()>().await;
    });
}

#[test]
fn killing_the_client_process_preserves_the_shell_and_synced_history() {
    let root = tempfile::Builder::new()
        .prefix("tcr-")
        .tempdir_in("/tmp")
        .unwrap();
    secure(root.path());
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "crash_client_fixture", "--nocapture"])
        .env("TERMINAL_CRASH_FIXTURE", root.path())
        .env("HUMATA2_HOLDER", holder())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(12);
    while !root.path().join("client.ready").exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "client starts its PTY"
        );
        assert!(child.try_wait().unwrap().is_none());
        std::thread::sleep(Duration::from_millis(20));
    }
    let pid = std::fs::read_to_string(root.path().join("shell.pid")).unwrap();
    child.kill().unwrap();
    child.wait().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let store = SessionStore::open(root.path(), holder()).unwrap();
        let mut a = store.attach(1).await.unwrap();
        assert!(!a.history.is_empty());
        a.writer
            .input(b"printf 'CRASH_RESUME=%s:%s\\n' \"$CRASH_KEEP\" $$\r")
            .await
            .unwrap();
        until(&mut a.reader, &format!("CRASH_RESUME=kept:{pid}")).await;
        a.writer.stop().await.unwrap();
    });
}

#[test]
fn existing_shared_directories_are_rejected_without_changing_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir_in("/tmp").unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(
        SessionStore::open(root.path(), holder()),
        Err(terminal_session::Error::Invalid { .. })
    ));
    assert_eq!(
        std::fs::metadata(root.path()).unwrap().permissions().mode() & 0o777,
        0o755
    );
}

#[tokio::test]
async fn resizing_notifies_the_foreground_application() {
    let dir = tempfile::Builder::new()
        .prefix("twinch-")
        .tempdir_in("/tmp")
        .unwrap();
    secure(dir.path());
    let mut store = SessionStore::open(dir.path(), holder()).unwrap();
    let mut config = spec();
    config.args = vec![
        "-c".into(),
        "trap 'printf WINCH_RECEIVED' WINCH; printf READY; while :; do read input; done".into(),
    ];
    let id = store.create(config).unwrap();
    let mut terminal = store.attach(id).await.unwrap();
    until(&mut terminal.reader, "READY").await;
    terminal.writer.resize(101, 31).await.unwrap();
    until(&mut terminal.reader, "WINCH_RECEIVED").await;
    terminal.writer.stop().await.unwrap();
    while terminal.reader.next().await.unwrap().is_some() {}
}

#[tokio::test]
async fn twelve_independent_terminals_keep_identity_and_layout_after_reopen() {
    let dir = tempfile::Builder::new()
        .prefix("tmulti-")
        .tempdir_in("/tmp")
        .unwrap();
    secure(dir.path());
    let binary = holder();
    let mut store = SessionStore::open(dir.path(), binary.clone()).unwrap();
    let mut terminals = Vec::new();
    let mut pids = std::collections::BTreeSet::new();
    for slot in 0..12 {
        let id = store.create(spec()).unwrap();
        let mut terminal = store.attach(id).await.unwrap();
        terminal.writer.input(format!("stty -echo; export SLOT={slot}; printf 'PTY_PID=%s\\nSLOT_READY_{slot}\\n' $$\r").as_bytes()).await.unwrap();
        let output = until(&mut terminal.reader, &format!("SLOT_READY_{slot}\r\n")).await;
        let output = String::from_utf8_lossy(&output);
        let pid = output
            .lines()
            .find_map(|line| {
                line.trim()
                    .strip_prefix("PTY_PID=")
                    .filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()))
            })
            .unwrap()
            .to_owned();
        assert!(pids.insert(pid.clone()));
        store.move_to(id, slot % 2, usize::MAX).unwrap();
        terminals.push((id, slot, pid, terminal));
    }
    let saved = store.groups().clone();
    let identities: Vec<_> = terminals
        .iter()
        .map(|(id, slot, pid, _)| (*id, *slot, pid.clone()))
        .collect();
    drop(terminals);
    drop(store);
    let store = SessionStore::open(dir.path(), binary).unwrap();
    assert_eq!(store.groups(), &saved);
    for (id, slot, pid) in identities {
        let mut terminal = store.attach(id).await.unwrap();
        terminal
            .writer
            .input(b"printf 'PTY_PID=%s SLOT=%s\\n' $$ \"$SLOT\"\r")
            .await
            .unwrap();
        until(
            &mut terminal.reader,
            &format!("PTY_PID={pid} SLOT={slot}\r\n"),
        )
        .await;
        terminal.writer.stop().await.unwrap();
        while terminal.reader.next().await.unwrap().is_some() {}
    }
}

#[tokio::test]
async fn metadata_write_failure_preserves_the_saved_and_in_memory_layout() {
    let dir = tempfile::Builder::new()
        .prefix("tdisk-")
        .tempdir_in("/tmp")
        .unwrap();
    secure(dir.path());
    let mut store = SessionStore::open(dir.path(), holder()).unwrap();
    let id = store.create(spec()).unwrap();
    let before = store.groups().clone();
    let manifest = std::fs::read(dir.path().join("workspace.json")).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o500)).unwrap();
    let result = store.move_to(id, 1, 0);
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(matches!(result, Err(terminal_session::Error::Disk { .. })));
    assert_eq!(store.groups(), &before);
    assert_eq!(
        std::fs::read(dir.path().join("workspace.json")).unwrap(),
        manifest
    );
    store.stop(id).await.unwrap();
}

#[tokio::test]
async fn host_memory_mode_keeps_output_and_launch_secrets_off_disk() {
    let dir = tempfile::Builder::new()
        .prefix("tprivate-")
        .tempdir_in("/tmp")
        .unwrap();
    secure(dir.path());
    let binary = holder();
    let mut store = SessionStore::open_memory_only(dir.path(), binary.clone()).unwrap();
    let mut launch = spec();
    launch.args=vec!["-c".into(),"stty -echo; printf 'OUTPUT_SECRET_SENTINEL\\n'; while IFS= read -r line; do printf 'MEMORY_ACK:%s\\n' \"$line\"; done".into()];
    let id = store.create(launch).unwrap();
    let recovery = terminal_session::RecoveryHint {
        provider: "fixture-agent".into(),
        session_id: "conversation-123".into(),
        working_directory: "/tmp".into(),
        state_paths: vec!["/tmp/fixture-agent-state".into()],
    };
    store.set_recovery(id, Some(recovery.clone())).unwrap();
    let mut terminal = store.attach_live_from(id, 0).await.unwrap();
    until(&mut terminal.reader, "OUTPUT_SECRET_SENTINEL\r\n").await;
    terminal
        .writer
        .input(b"INPUT_SECRET_SENTINEL\r")
        .await
        .unwrap();
    until(&mut terminal.reader, "MEMORY_ACK:INPUT_SECRET_SENTINEL\r\n").await;
    for entry in std::fs::read_dir(dir.path()).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_file() {
            let bytes = std::fs::read(entry.path()).unwrap();
            let text = String::from_utf8_lossy(&bytes);
            assert!(!text.contains("OUTPUT_SECRET_SENTINEL"));
            assert!(!text.contains("INPUT_SECRET_SENTINEL"));
        }
    }
    assert!(!dir.path().join(format!("s{id}.journal")).exists());
    assert!(!dir.path().join(format!("s{id}.scrollback")).exists());
    drop(terminal);
    drop(store);
    let store = SessionStore::open_memory_only(dir.path(), binary).unwrap();
    assert_eq!(store.session(id).unwrap().recovery, Some(recovery));
    assert!(store.session(id).unwrap().spec.args.is_empty());
    let mut terminal = store.attach_live_from(id, 0).await.unwrap();
    terminal.writer.input(b"REOPENED\r").await.unwrap();
    until(&mut terminal.reader, "MEMORY_ACK:REOPENED\r\n").await;
    terminal.writer.stop().await.unwrap();
    while terminal.reader.next().await.unwrap().is_some() {}
}

#[tokio::test]
async fn owned_live_attacher_resumes_without_store_borrow_input_replay_or_process_replacement() {
    let dir = tempfile::Builder::new()
        .prefix("tattacher-")
        .tempdir_in("/tmp")
        .unwrap();
    secure(dir.path());
    let mut store = SessionStore::open_memory_only(dir.path(), holder()).unwrap();
    let mut launch = spec();
    launch.args = vec!["-c".into(), "stty -echo; count=0; while IFS= read -r line; do count=$((count+1)); printf 'COUNT_%s:%s\\n' \"$count\" \"$line\"; done".into()];
    let id = store.create(launch).unwrap();
    let prepared = store.prepare_live_attachment(id).unwrap();
    let task_capability = prepared.clone();
    let task = tokio::spawn(async move { task_capability.attach_from(0).await });
    // An owned connection future neither borrows nor locks the global workspace.
    store.move_to(id, 1, 0).unwrap();
    let mut first = task.await.unwrap().unwrap();
    assert!(matches!(
        prepared.attach_from(0).await,
        Err(terminal_session::Error::Attached { .. })
    ));
    first.writer.input(b"FIRST\r").await.unwrap();
    let offset = tokio::time::timeout(Duration::from_secs(8), async {
        let mut output = Vec::new();
        loop {
            if let Some(Update::Record(Record::Output { offset, bytes })) =
                first.reader.next().await.unwrap()
            {
                let end = offset + bytes.len() as u64;
                output.extend(bytes);
                if String::from_utf8_lossy(&output).contains("COUNT_1:FIRST\r\n") {
                    break end;
                }
            }
        }
    })
    .await
    .unwrap();
    drop(first);
    drop(store);
    let mut resumed = prepared.attach_from(offset).await.unwrap();
    assert!(resumed.history.is_empty());
    resumed.writer.input(b"SECOND\r").await.unwrap();
    let output = until(&mut resumed.reader, "COUNT_2:SECOND\r\n").await;
    let text = String::from_utf8_lossy(&output);
    assert!(
        !text.contains("COUNT_1:FIRST"),
        "reconnect must start at the authoritative output offset"
    );
    resumed.writer.stop().await.unwrap();
    while resumed.reader.next().await.unwrap().is_some() {}
    drop(resumed);
    assert!(
        prepared.attach_from(offset).await.is_err(),
        "a dead holder must not be replaced by attachment"
    );
    assert!(!dir.path().join(format!("s{id}.journal")).exists());
}

#[test]
fn prepared_live_attachment_rejects_a_journaled_store_without_mutating_metadata() {
    let dir = tempfile::tempdir_in("/tmp").unwrap();
    secure(dir.path());
    let store = SessionStore::open(dir.path(), holder()).unwrap();
    assert!(matches!(
        store.prepare_live_attachment(1),
        Err(terminal_session::Error::Invalid { .. })
    ));
    assert!(store.sessions().is_empty());
}

#[test]
fn old_launch_arguments_are_removed_during_metadata_migration() {
    let dir = tempfile::Builder::new()
        .prefix("tmigrate-")
        .tempdir_in("/tmp")
        .unwrap();
    secure(dir.path());
    let binary = holder();
    let mut store = SessionStore::open(dir.path(), binary.clone()).unwrap();
    let id = store.create(spec()).unwrap();
    drop(store);
    let manifest = dir.path().join("workspace.json");
    let mut saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
    saved["version"] = 1.into();
    saved["sessions"][0]["spec"]["args"] = serde_json::json!(["ARGUMENT_SECRET_SENTINEL"]);
    std::fs::write(&manifest, serde_json::to_vec(&saved).unwrap()).unwrap();
    let store = SessionStore::open(dir.path(), binary).unwrap();
    let rewritten = std::fs::read_to_string(&manifest).unwrap();
    assert!(!rewritten.contains("ARGUMENT_SECRET_SENTINEL"));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(store.stop(id)).unwrap();
}
