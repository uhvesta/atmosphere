//! Behavioral regressions at the persistent host/VT trust boundary.
use super::*;
use std::{
    fs::OpenOptions,
    io::{Read, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt, symlink},
    sync::mpsc as std_mpsc,
};

fn private_state() -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .prefix("h2-adversarial-")
        .tempdir_in("/tmp")
        .unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    dir
}

async fn start_query_host(root: &std::path::Path) -> Host {
    start_query_host_result(root).await.unwrap()
}

async fn start_query_host_result(root: &std::path::Path) -> Result<Host, Error> {
    let holder = std::fs::canonicalize(std::env::var_os("HUMATA2_HOLDER").unwrap()).unwrap();
    let store = SessionStore::open_memory_only(root, holder).unwrap();
    let profile = Profile {
        name: "query".into(),
        title: "Adversarial query fixture".into(),
        program: "/bin/sh".into(),
        args: vec![
            "-c".into(),
            r#"
printf '%s' "$$" > "$1/pid"
while [ ! -e "$1/trigger" ]; do sleep 0.01; done
stty -icanon -echo min 1 time 0
printf '\033[6n'
dd bs=1 count=6 of="$1/first" 2>/dev/null
printf 'FIRST_READY\r\n'
dd bs=1 count=6 of="$1/second" 2>/dev/null
printf 'SECOND_READY\r\n'
while [ ! -e "$1/fresh-trigger" ]; do sleep 0.01; done
printf '\033[6n'
dd bs=1 count=6 of="$1/fresh" 2>/dev/null
printf 'FRESH_READY\r\n'
exec /bin/sh -i
"#
            .into(),
            "adversarial-query".into(),
            root.to_string_lossy().into_owned(),
        ],
        cwd: root.into(),
        endpoint: Endpoint::Local,
        recovery: None,
        history: HistoryPolicy::Recent {
            max_lines: 10000,
            max_bytes: 4 << 20,
            interval_seconds: 3600,
        },
    };
    Host::start(
        Config {
            state: root.into(),
            label: "adversarial".into(),
            start_profiles: vec!["query".into()],
            default_history: HistoryPolicy::default(),
        },
        store,
        vec![profile],
    )
    .await
}

async fn receive(view: &mut Subscription) -> ServerFrame {
    tokio::time::timeout(Duration::from_secs(8), view.receiver.recv())
        .await
        .unwrap()
        .unwrap()
        .frame
}

async fn open(host: &Host) -> Subscription {
    let mut view = host.subscribe().await.unwrap();
    let _ = receive(&mut view).await;
    host.frame(view.id, ClientFrame::Open { id: 1 })
        .await
        .unwrap();
    view
}

async fn output_contains(view: &mut Subscription, expected: &str) {
    let mut output = String::new();
    loop {
        if let ServerFrame::Output { bytes, .. } = receive(view).await {
            output.push_str(&String::from_utf8_lossy(&bytes));
            if output.contains(expected) {
                return;
            }
        }
    }
}

async fn snapshot_contains(host: &Host, view: &mut Subscription, expected: &str) {
    use ghostty_vt::Grid;
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        host.frame(view.id, ClientFrame::SnapshotRequest { id: 1 })
            .await
            .unwrap();
        if let ServerFrame::Snapshot { bytes, .. } = receive(view).await {
            let mut vt = Terminal::from_checkpoint(&bytes).unwrap();
            vt.sync();
            let text = (0..vt.size().rows)
                .filter_map(|y| vt.row(y))
                .flat_map(|row| row.cells.iter().map(|cell| cell.text.as_str()))
                .collect::<String>();
            if text.contains(expected) {
                return;
            }
        }
        assert!(
            Instant::now() < deadline,
            "the replay did not reconstruct its expected display"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn resume_does_not_answer_old_dsr_twice_but_answers_new_queries() {
    let dir = private_state();
    let host = start_query_host(dir.path()).await;
    // Persist a verified empty screen while the same process waits for input.
    host.shutdown().await.unwrap();
    let host = start_query_host(dir.path()).await;
    let mut view = open(&host).await;
    std::fs::write(dir.path().join("trigger"), b"go").unwrap();
    output_contains(&mut view, "FIRST_READY").await;
    let first = std::fs::read(dir.path().join("first")).unwrap();
    let recovery = Recovery::open(dir.path().join("recovery")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    while recovery.reply_cursor(1).unwrap().pending {
        assert!(
            Instant::now() < deadline,
            "the acknowledged first reply never settled its recovery guard"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let (reply, stopped) = oneshot::channel();
    host.tx.send(Command::Crash { reply }).await.unwrap();
    stopped.await.unwrap();
    drop(view);

    let host = start_query_host(dir.path()).await;
    let mut view = open(&host).await;
    snapshot_contains(&host, &mut view, "FIRST_READY").await;
    host.frame(
        view.id,
        ClientFrame::Input {
            id: 1,
            seq: 1,
            bytes: b"ABCDEF".to_vec(),
        },
    )
    .await
    .unwrap();
    std::fs::write(dir.path().join("fresh-trigger"), b"go").unwrap();
    output_contains(&mut view, "FRESH_READY").await;
    let second = std::fs::read(dir.path().join("second")).unwrap();
    let fresh = std::fs::read(dir.path().join("fresh")).unwrap();
    // Stop the owned fixture before reporting the intentional pre-fix failure.
    host.frame(view.id, ClientFrame::Stop { id: 1 })
        .await
        .unwrap();
    loop {
        if matches!(receive(&mut view).await, ServerFrame::Exited { id: 1, .. }) {
            break;
        }
    }
    host.shutdown().await.unwrap();
    assert_eq!(first, b"\x1b[1;1R");
    assert_eq!(
        second, b"ABCDEF",
        "the host injected a duplicate historical DSR response into the running program"
    );
    assert!(
        fresh.starts_with(b"\x1b[") && fresh.ends_with(b"R"),
        "fresh post-reattachment terminal queries must still be answered: {fresh:?}"
    );
}

#[tokio::test]
async fn resume_answers_a_query_emitted_while_the_host_was_down() {
    let dir = private_state();
    let host = start_query_host(dir.path()).await;
    host.shutdown().await.unwrap();
    let host = start_query_host(dir.path()).await;
    let (reply, stopped) = oneshot::channel();
    host.tx.send(Command::Crash { reply }).await.unwrap();
    stopped.await.unwrap();
    std::fs::write(dir.path().join("trigger"), b"go").unwrap();

    // Confirm that the question is retained BEFORE the replacement host
    // attaches. It is replay, but no previous host ever answered it.
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let holder = std::fs::canonicalize(std::env::var_os("HUMATA2_HOLDER").unwrap()).unwrap();
        let store = SessionStore::open_memory_only(dir.path(), holder).unwrap();
        let attached = store.attach_live_from(1, 0).await.unwrap();
        let retained = attached.replay_end;
        drop(attached);
        drop(store);
        if retained >= 4 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the owned holder never retained its unanswered query"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let host = start_query_host(dir.path()).await;
    let view = open(&host).await;
    let deadline = Instant::now() + Duration::from_secs(2);
    let first = loop {
        let bytes = std::fs::read(dir.path().join("first")).unwrap_or_default();
        if bytes.len() >= 6 || Instant::now() >= deadline {
            break bytes;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    let duplicate = std::fs::read(dir.path().join("second")).unwrap_or_default();
    // Use the original holder control for cleanup even if a defensive host
    // implementation disabled viewer input during uncertain recovery.
    let _ = host.shutdown().await;
    drop(view);
    let holder = std::fs::canonicalize(std::env::var_os("HUMATA2_HOLDER").unwrap()).unwrap();
    let store = SessionStore::open_memory_only(dir.path(), holder).unwrap();
    let mut attached = store.attach_live_from(1, 0).await.unwrap();
    attached.writer.stop().await.unwrap();
    loop {
        let update = tokio::time::timeout(Duration::from_secs(8), attached.reader.next())
            .await
            .unwrap()
            .unwrap();
        if matches!(update, Some(Update::Exited(_)) | None) {
            break;
        }
    }
    assert_eq!(
        first, b"\x1b[1;1R",
        "replay cutoffs must not suppress an unanswered query emitted while the host was stopped"
    );
    assert!(
        duplicate.is_empty(),
        "the previously unanswered query must be answered exactly once: {duplicate:?}"
    );
}

#[tokio::test]
async fn uncertain_reply_delivery_preserves_display_and_disables_live_input() {
    let dir = private_state();
    let host = start_query_host(dir.path()).await;
    host.shutdown().await.unwrap();
    let recovery = Recovery::open(dir.path().join("recovery")).unwrap();
    recovery
        .save_reply_cursor(
            1,
            recovery::ReplyCursor {
                answered_through: 0,
                pending: true,
            },
        )
        .unwrap();
    let host = start_query_host(dir.path()).await;
    let mut view = open(&host).await;
    let live = loop {
        if let ServerFrame::Snapshot { live, .. } = receive(&mut view).await {
            break live;
        }
    };
    host.frame(
        view.id,
        ClientFrame::Input {
            id: 1,
            seq: 1,
            bytes: b"ABCDEF".to_vec(),
        },
    )
    .await
    .unwrap();
    let rejected = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            match receive(&mut view).await {
                ServerFrame::Error { code, .. } if code == "not_running" => break true,
                ServerFrame::InputWritten { .. } => break false,
                _ => {}
            }
        }
    })
    .await
    .unwrap_or(false);
    let _ = host.shutdown().await;
    drop(view);
    let holder = std::fs::canonicalize(std::env::var_os("HUMATA2_HOLDER").unwrap()).unwrap();
    let store = SessionStore::open_memory_only(dir.path(), holder).unwrap();
    let mut attached = store.attach_live_from(1, 0).await.unwrap();
    attached.writer.stop().await.unwrap();
    loop {
        let update = tokio::time::timeout(Duration::from_secs(8), attached.reader.next())
            .await
            .unwrap()
            .unwrap();
        if matches!(update, Some(Update::Exited(_)) | None) {
            break;
        }
    }
    assert!(
        !live,
        "uncertain historical reply delivery must not be presented as a writable resumed process"
    );
    assert!(
        rejected,
        "viewer input must be rejected while automatic reply delivery is uncertain"
    );
}

#[tokio::test]
async fn retry_after_startup_settings_failure_keeps_the_original_process_only() {
    let dir = private_state();
    let recovery = dir.path().join("recovery");
    std::fs::create_dir(&recovery).unwrap();
    std::fs::set_permissions(&recovery, std::fs::Permissions::from_mode(0o500)).unwrap();
    let failed = match start_query_host_result(dir.path()).await {
        Ok(host) => {
            let _ = host.shutdown().await;
            false
        }
        Err(_) => true,
    };
    std::fs::set_permissions(&recovery, std::fs::Permissions::from_mode(0o700)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let original_pid = loop {
        let pid = std::fs::read(dir.path().join("pid")).unwrap_or_default();
        if !pid.is_empty() || Instant::now() >= deadline {
            break pid;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    let host = start_query_host(dir.path()).await;
    let mut view = host.subscribe().await.unwrap();
    let sessions = loop {
        if let ServerFrame::Sessions { sessions, .. } = receive(&mut view).await {
            break sessions;
        }
    };
    let retried_pid = std::fs::read(dir.path().join("pid")).unwrap_or_default();
    let _ = host.shutdown().await;
    drop(view);
    let holder = std::fs::canonicalize(std::env::var_os("HUMATA2_HOLDER").unwrap()).unwrap();
    let store = SessionStore::open_memory_only(dir.path(), holder).unwrap();
    let ids = store.sessions().iter().map(|s| s.id).collect::<Vec<_>>();
    for id in ids {
        if let Ok(mut attached) = store.attach_live_from(id, 0).await {
            attached.writer.stop().await.unwrap();
            loop {
                let update = tokio::time::timeout(Duration::from_secs(8), attached.reader.next())
                    .await
                    .unwrap()
                    .unwrap();
                if matches!(update, Some(Update::Exited(_)) | None) {
                    break;
                }
            }
        }
    }
    assert!(
        failed,
        "the fixture must reject the settings write after launch"
    );
    assert!(
        !original_pid.is_empty(),
        "the first process must have launched before the settings failure"
    );
    assert_eq!(
        sessions.len(),
        1,
        "retrying startup after a settings write failure duplicated the existing holder"
    );
    assert_eq!(
        retried_pid, original_pid,
        "startup retry replaced or duplicated the original process"
    );
}

fn dense_output() -> Vec<u8> {
    let mut output = Vec::new();
    for line in 0..12_000u32 {
        output.extend_from_slice(format!("{line:05} 界e\u{301}👩\u{200d}💻").as_bytes());
        for chunk in 0..8u32 {
            output.extend_from_slice(
                format!(
                    "\x1b[38;2;{};{};{}m",
                    (line * 17 + chunk * 31) % 256,
                    (line * 43 + chunk * 7) % 256,
                    (line * 73 + chunk * 19) % 256
                )
                .as_bytes(),
            );
            for column in 0..11u32 {
                output.push(b'a' + ((line * 11 + chunk * 17 + column * 7) % 26) as u8);
            }
        }
        output.extend_from_slice(b"Z\x1b[0m\r\n");
    }
    output
}

async fn start_dense_host(root: &std::path::Path) -> Host {
    let holder = std::fs::canonicalize(std::env::var_os("HUMATA2_HOLDER").unwrap()).unwrap();
    let store = SessionStore::open_memory_only(root, holder).unwrap();
    let profile=Profile{name:"dense".into(),title:"Dense history".into(),program:"/bin/sh".into(),args:vec!["-c".into(),r#"while [ ! -e "$1/dense-trigger" ]; do sleep 0.01; done; stty -icanon -echo min 1 time 0; cat "$1/dense-output"; printf '\r\nDENSE_READY\r\n\033[5n'; dd bs=1 count=4 of="$1/dense-ack" 2>/dev/null; printf ready > "$1/dense-sent"; exec /bin/sh -i"#.into(),"dense-fixture".into(),root.to_string_lossy().into_owned()],cwd:root.into(),endpoint:Endpoint::Local,recovery:None,history:HistoryPolicy::Recent{max_lines:10000,max_bytes:4<<20,interval_seconds:3600}};
    Host::start(
        Config {
            state: root.into(),
            label: "dense-history".into(),
            start_profiles: vec!["dense".into()],
            default_history: HistoryPolicy::default(),
        },
        store,
        vec![profile],
    )
    .await
    .unwrap()
}

fn snapshot_text(frame: &ServerFrame) -> Option<(String, usize)> {
    use ghostty_vt::Grid;
    let ServerFrame::Snapshot { bytes, .. } = frame else {
        return None;
    };
    let mut terminal = Terminal::from_checkpoint(bytes).ok()?;
    terminal.sync();
    let text = (0..terminal.size().rows)
        .filter_map(|y| terminal.row(y))
        .flat_map(|row| row.cells.iter().map(|cell| cell.text.as_str()))
        .collect::<String>();
    Some((text, terminal.scrollback_rows()))
}

async fn dense_snapshot(host: &Host, view: &mut Subscription) -> Option<ServerFrame> {
    tokio::time::timeout(Duration::from_secs(15), async {
        let mut tail = Vec::new();
        loop {
            let frame = view.receiver.recv().await?.frame;
            match &frame {
                ServerFrame::Snapshot { .. } => {
                    if snapshot_text(&frame)?.0.contains("DENSE_READY") {
                        return Some(frame);
                    }
                }
                ServerFrame::Output { id, offset, bytes } => {
                    host.frame(
                        view.id,
                        ClientFrame::Ack {
                            id: *id,
                            offset: *offset + bytes.len() as u64,
                        },
                    )
                    .await
                    .ok()?;
                    tail.extend_from_slice(bytes);
                    if tail.len() > 256 {
                        tail.drain(..tail.len() - 256);
                    }
                    if String::from_utf8_lossy(&tail).contains("DENSE_READY") {
                        host.frame(view.id, ClientFrame::SnapshotRequest { id: 1 })
                            .await
                            .ok()?;
                    }
                }
                ServerFrame::ResyncRequired { .. } => {
                    host.frame(view.id, ClientFrame::SnapshotRequest { id: 1 })
                        .await
                        .ok()?;
                }
                ServerFrame::Error { code, .. } if code == "checkpoint_limit" => return None,
                _ => {}
            }
        }
    })
    .await
    .ok()
    .flatten()
}

#[tokio::test]
async fn dense_ten_thousand_line_history_fits_wire_budget_and_survives_host_restart() {
    let dir = private_state();
    std::fs::write(dir.path().join("dense-output"), dense_output()).unwrap();
    let host = start_dense_host(dir.path()).await;
    let start = Instant::now();
    std::fs::write(dir.path().join("dense-trigger"), b"go").unwrap();
    // A status reply after the burst proves the authoritative VT consumed it
    // before the viewer attaches. Slow-viewer eviction has separate coverage.
    let deadline = Instant::now() + Duration::from_secs(15);
    while !dir.path().join("dense-sent").exists() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let processed = dir.path().join("dense-sent").exists();
    let mut view = open(&host).await;
    let first = dense_snapshot(&host, &mut view).await;
    let feed_elapsed = start.elapsed();
    let save_ok = host.shutdown().await.is_ok();
    drop(view);
    let disk_bytes = std::fs::metadata(dir.path().join("recovery/s1.checkpoint"))
        .map(|m| m.len())
        .ok();
    let recovery = Recovery::open(dir.path().join("recovery")).unwrap();
    let mut settings = recovery.settings().unwrap();
    settings.get_mut(&1).unwrap().history = HistoryPolicy::Recent {
        max_lines: 10000,
        max_bytes: 64 << 10,
        interval_seconds: 3600,
    };
    recovery.save_settings(&settings).unwrap();
    let host = start_dense_host(dir.path()).await;
    let mut view = open(&host).await;
    let second = dense_snapshot(&host, &mut view).await;
    let _ = host.shutdown().await;
    drop(view);
    let holder = std::fs::canonicalize(std::env::var_os("HUMATA2_HOLDER").unwrap()).unwrap();
    let store = SessionStore::open_memory_only(dir.path(), holder).unwrap();
    let sessions = store.sessions().len();
    let mut attached = store.attach_live_from(1, u64::MAX).await.unwrap();
    attached.writer.stop().await.unwrap();
    loop {
        let update = tokio::time::timeout(Duration::from_secs(8), attached.reader.next())
            .await
            .unwrap()
            .unwrap();
        if matches!(update, Some(Update::Exited(_)) | None) {
            break;
        }
    }
    // Assertions follow fixture shutdown so a failing budget/recovery check
    // cannot leave its real shell alive after the test.
    let first = first.expect("the host must offer a checkpoint after dense output");
    let second = second.expect("the dense saved screen must be available after restart");
    let (_, before_rows) = snapshot_text(&first).unwrap();
    let (_, after_rows) = snapshot_text(&second).unwrap();
    assert!(
        before_rows >= 9500 && after_rows >= 9500,
        "dense history was evicted before its configured10k line budget: {before_rows}->{after_rows}"
    );
    assert!(save_ok);
    assert!(
        processed,
        "the authoritative host did not consume and answer after the dense output burst"
    );
    assert_eq!(sessions, 1, "restart must reattach the same owned session");
    let encoded = serde_json::to_vec(&first).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    let compressed = value["bytes"]["encoded_bytes"]
        .as_u64()
        .expect("versioned snapshot compression envelope") as usize;
    use base64::Engine;
    let actual = base64::engine::general_purpose::STANDARD
        .decode(value["bytes"]["data_base64"].as_str().unwrap())
        .unwrap();
    assert_eq!(compressed, actual.len());
    assert!(compressed <= terminal_wire::MAX_SNAPSHOT_BYTES);
    assert!(disk_bytes.is_some_and(|bytes| bytes <= 4 << 20));
    assert!(
        disk_bytes.is_some_and(|bytes| bytes > 64 << 10),
        "the saved checkpoint must exercise recovery after lowering its write budget"
    );
    let decoded: ServerFrame = serde_json::from_slice(&encoded).unwrap();
    let ServerFrame::Snapshot { bytes: decoded, .. } = decoded else {
        panic!("snapshot wire roundtrip changed frame type");
    };
    let ServerFrame::Snapshot {
        bytes: original, ..
    } = &first
    else {
        unreachable!();
    };
    assert_eq!(&decoded, original);
    eprintln!(
        "host-dense-history: rows={before_rows}->{after_rows} raw_bytes={} wire_compressed_bytes={compressed} disk_bytes={} feed_snapshot_ms={}",
        original.len(),
        disk_bytes.unwrap(),
        feed_elapsed.as_millis()
    );
}

fn small_checkpoint() -> Checkpoint {
    Checkpoint {
        bytes: vec![1; 100],
        offset: 0,
        cols: 80,
        rows: 24,
        exited: None,
        resize_epoch: 0,
    }
}

fn fifo_read_rejected_quickly(
    path: &std::path::Path,
    bytes: &[u8],
    operation: impl FnOnce() -> bool + Send + 'static,
) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let cpath = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    // SAFETY: the path is an owned temporary filename and cpath stays live.
    assert_eq!(unsafe { libc::mkfifo(cpath.as_ptr(), 0o600) }, 0);
    let mut writer = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .unwrap();
    writer.write_all(bytes).unwrap();
    let (tx, rx) = std_mpsc::channel();
    let worker = std::thread::spawn(move || {
        let _ = tx.send(operation());
    });
    let early = rx.recv_timeout(Duration::from_millis(500));
    drop(writer);
    worker.join().unwrap();
    matches!(early, Ok(true))
}

#[test]
fn recovery_settings_reject_a_valid_json_symlink() {
    let dir = private_state();
    let recovery = Recovery::open(dir.path().join("recovery")).unwrap();
    let outside = dir.path().join("unrelated-settings");
    std::fs::write(&outside, b"{}").unwrap();
    symlink(outside, dir.path().join("recovery/sessions.json")).unwrap();
    assert!(recovery.settings().is_err());
}

#[test]
fn recovery_checkpoint_rejects_a_valid_archive_symlink() {
    let dir = private_state();
    let recovery = Recovery::open(dir.path().join("recovery")).unwrap();
    recovery.save(1, small_checkpoint(), 4 << 20).unwrap();
    let path = dir.path().join("recovery/s1.checkpoint");
    let outside = dir.path().join("unrelated-checkpoint");
    std::fs::rename(&path, &outside).unwrap();
    symlink(outside, path).unwrap();
    assert!(recovery.load(1, 4 << 20).is_err());
}

#[test]
fn recovery_settings_reject_a_fifo_without_waiting_for_eof() {
    let dir = private_state();
    let recovery = Recovery::open(dir.path().join("recovery")).unwrap();
    assert!(fifo_read_rejected_quickly(
        &dir.path().join("recovery/sessions.json"),
        b"{}",
        move || recovery.settings().is_err()
    ));
}

#[test]
fn recovery_checkpoint_rejects_a_fifo_without_waiting_for_eof() {
    let dir = private_state();
    let recovery = Recovery::open(dir.path().join("recovery")).unwrap();
    recovery.save(1, small_checkpoint(), 4 << 20).unwrap();
    let path = dir.path().join("recovery/s1.checkpoint");
    let bytes = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    assert!(fifo_read_rejected_quickly(&path, &bytes, move || recovery
        .load(1, 4 << 20)
        .is_err()));
}

#[test]
fn recovery_temporary_fifo_cannot_receive_metadata_bytes() {
    use std::os::unix::ffi::OsStrExt;
    let dir = private_state();
    let recovery = Recovery::open(dir.path().join("recovery")).unwrap();
    let path = dir.path().join("recovery/sessions.json.tmp");
    let cpath = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    // SAFETY: cpath is an owned temporary filename.
    assert_eq!(unsafe { libc::mkfifo(cpath.as_ptr(), 0o600) }, 0);
    let mut pipe = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(&path)
        .unwrap();
    let failed = recovery.save_settings(&BTreeMap::new()).is_err();
    let mut bytes = [0u8; 64];
    let written = match pipe.read(&mut bytes) {
        Ok(len) => len,
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => 0,
        Err(error) => panic!("read owned FIFO: {error}"),
    };
    assert!(failed);
    assert_eq!(
        written, 0,
        "a temporary FIFO received recovery metadata before file-type rejection"
    );
}

#[test]
fn recovery_temporary_hardlink_cannot_truncate_an_unrelated_file() {
    let dir = private_state();
    let recovery = Recovery::open(dir.path().join("recovery")).unwrap();
    let unrelated = dir.path().join("unrelated-owned-file");
    std::fs::write(&unrelated, b"PRESERVE_UNRELATED_CONTENT").unwrap();
    std::fs::hard_link(&unrelated, dir.path().join("recovery/sessions.json.tmp")).unwrap();
    let failed = recovery.save_settings(&BTreeMap::new()).is_err();
    let contents = std::fs::read(unrelated).unwrap();
    assert!(failed, "the writer accepted a shared temporary inode");
    assert_eq!(contents, b"PRESERVE_UNRELATED_CONTENT");
}

#[test]
fn resize_guard_requires_exactly_eight_bytes() {
    let dir = private_state();
    let recovery = Recovery::open(dir.path().join("recovery")).unwrap();
    let path = dir.path().join("recovery/s1.resize");
    for bytes in [vec![], vec![0; 7], vec![0; 9], vec![0; 65536]] {
        std::fs::write(&path, bytes).unwrap();
        assert!(recovery.resize_epoch(1).is_err());
    }
    std::fs::write(&path, 42u64.to_le_bytes()).unwrap();
    assert_eq!(recovery.resize_epoch(1).unwrap(), Some(42));
}

#[test]
fn resize_guard_refuses_a_symlink_even_when_target_is_a_valid_guard() {
    let dir = private_state();
    let recovery = Recovery::open(dir.path().join("recovery")).unwrap();
    let outside = dir.path().join("unrelated-state");
    std::fs::write(&outside, 42u64.to_le_bytes()).unwrap();
    symlink(outside, dir.path().join("recovery/s1.resize")).unwrap();
    assert!(
        recovery.resize_epoch(1).is_err(),
        "state reads must not follow a symlink out of the private recovery directory"
    );
}

#[test]
fn resize_guard_refuses_a_fifo_without_waiting_for_its_writer_to_close() {
    use std::os::unix::ffi::OsStrExt;
    let dir = private_state();
    let recovery = Recovery::open(dir.path().join("recovery")).unwrap();
    let path = dir.path().join("recovery/s1.resize");
    let cpath = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    // SAFETY: cpath is a live NUL-terminated owned temporary filename.
    assert_eq!(unsafe { libc::mkfifo(cpath.as_ptr(), 0o600) }, 0);
    let mut writer = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(&path)
        .unwrap();
    writer.write_all(&42u64.to_le_bytes()).unwrap();
    let (sender, receiver) = std_mpsc::channel();
    let worker = std::thread::spawn(move || {
        let _ = sender.send(recovery.resize_epoch(1));
    });
    let early = receiver.recv_timeout(Duration::from_millis(500));
    // Closing this owned writer always unblocks the old implementation, so
    // the expected pre-fix failure leaves no background thread behind.
    drop(writer);
    worker.join().unwrap();
    assert!(
        matches!(early, Ok(Err(_))),
        "a non-regular guard blocked its reader or was accepted: {early:?}"
    );
}
