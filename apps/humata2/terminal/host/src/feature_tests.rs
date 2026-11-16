//! Native embedding contracts, independent of a browser or app chrome.
use super::*;
use ghostty_vt::Grid;
use std::os::unix::fs::PermissionsExt;
use terminal_provider::{ProviderEvent, ProviderSignal, TrustedContext};
fn state() -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .prefix("h2-native-features-")
        .tempdir_in("/tmp")
        .unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    dir
}
fn holder() -> PathBuf {
    std::fs::canonicalize(std::env::var_os("HUMATA2_HOLDER").unwrap()).unwrap()
}
fn profile(root: &std::path::Path) -> Profile {
    Profile {
        name: "shell".into(),
        title: "Source".into(),
        program: "/bin/sh".into(),
        args: vec!["-i".into()],
        cwd: root.into(),
        endpoint: Endpoint::Local,
        recovery: None,
        history: HistoryPolicy::Off,
    }
}
async fn start(root: &std::path::Path, profiles: Vec<Profile>) -> Host {
    Host::start(
        Config {
            state: root.into(),
            label: "native feature tests".into(),
            start_profiles: vec!["shell".into()],
            default_history: HistoryPolicy::default(),
        },
        SessionStore::open_memory_only(root, holder()).unwrap(),
        profiles,
    )
    .await
    .unwrap()
}
async fn next(view: &mut Subscription) -> ServerFrame {
    tokio::time::timeout(Duration::from_secs(8), view.receiver.recv())
        .await
        .unwrap()
        .unwrap()
        .frame
}
async fn open(host: &Host, id: u64) -> Subscription {
    let mut view = host.subscribe().await.unwrap();
    let _ = next(&mut view).await;
    host.frame(view.id, ClientFrame::Open { id }).await.unwrap();
    loop {
        if matches!(
            next(&mut view).await,
            ServerFrame::Control {
                controller: true,
                ..
            }
        ) {
            break;
        }
    }
    view
}
async fn input(host: &Host, view: &Subscription, id: u64, seq: u64, text: String) {
    host.frame(
        view.id,
        ClientFrame::Input {
            id,
            seq,
            bytes: text.into_bytes(),
        },
    )
    .await
    .unwrap();
}
async fn snapshot(host: &Host, view: &mut Subscription, id: u64, needle: &str) -> Terminal {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        host.frame(view.id, ClientFrame::SnapshotRequest { id })
            .await
            .unwrap();
        let bytes = loop {
            if let ServerFrame::Snapshot { id: got, bytes, .. } = next(view).await {
                if got == id {
                    break bytes;
                }
            }
        };
        let mut vt = Terminal::from_checkpoint(&bytes).unwrap();
        vt.sync();
        let text = (0..vt.size().rows)
            .filter_map(|y| vt.row(y))
            .flat_map(|r| r.cells.iter().map(|c| c.text.as_str()))
            .collect::<String>();
        if text.contains(needle) {
            return vt;
        }
        assert!(
            Instant::now() < deadline,
            "snapshot missing expected fixture output"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
async fn stop(host: &Host, view: &mut Subscription, id: u64) {
    host.frame(view.id, ClientFrame::TakeControl { id })
        .await
        .unwrap();
    loop {
        if matches!(next(view).await,ServerFrame::Control{id:got,controller:true}if got==id) {
            break;
        }
    }
    host.frame(view.id, ClientFrame::Stop { id }).await.unwrap();
    loop {
        if matches!(next(view).await,ServerFrame::Exited{id:got,..}if got==id) {
            break;
        }
    }
}
fn context() -> TrustedContext {
    TrustedContext {
        provider: "codex".into(),
        schema: terminal_provider::CODEX_HOOKS_V1.into(),
        hook_event: None,
        state_paths: vec!["/tmp/explicit-provider-state".into()],
    }
}
fn event(id: &str, signal: ProviderSignal) -> ProviderEvent {
    ProviderEvent {
        provider: "codex".into(),
        schema: terminal_provider::CODEX_HOOKS_V1.into(),
        session_id: id.into(),
        cwd: Some("/tmp".into()),
        signal,
    }
}
#[tokio::test]
async fn trusted_provider_identity_rejects_stale_hooks_and_reports_observer_lag() {
    let dir = state();
    let host = start(dir.path(), vec![profile(dir.path())]).await;
    let mut view = open(&host, 1).await;
    let mut observer = host.observe_provider(1).await.unwrap();
    assert!(host.bind_provider(999, context()).await.is_err());
    let binding = host.bind_provider(1, context()).await.unwrap();
    assert!(
        binding
            .ingest(event("not-started", ProviderSignal::TurnFinished))
            .await
            .is_err()
    );
    binding
        .ingest(event(
            "first",
            ProviderSignal::SessionStarted {
                source: Some("startup".into()),
            },
        ))
        .await
        .unwrap();
    binding
        .ingest(event("first", ProviderSignal::TurnFinished))
        .await
        .unwrap();
    binding
        .ingest(event(
            "first",
            ProviderSignal::Notification {
                kind: "fixture".into(),
                title: None,
                message: "EPHEMERAL_PRIVATE_MESSAGE".into(),
            },
        ))
        .await
        .unwrap();
    assert!(matches!(
        observer.recv().await,
        Some(ProviderObservation::Lagged { dropped: 2 })
    ));
    assert!(matches!(
        observer.recv().await,
        Some(ProviderObservation::Event {
            terminal: 1,
            event: ProviderEvent {
                signal: ProviderSignal::SessionStarted { .. },
                ..
            }
        })
    ));
    assert!(
        binding
            .ingest(event(
                "old-session",
                ProviderSignal::Notification {
                    kind: "fixture".into(),
                    title: None,
                    message: "STALE_PRIVATE_MESSAGE".into()
                }
            ))
            .await
            .is_err()
    );
    let newer = host.bind_provider(1, context()).await.unwrap();
    assert!(
        binding
            .ingest(event(
                "second",
                ProviderSignal::SessionStarted { source: None }
            ))
            .await
            .is_err()
    );
    newer
        .ingest(event("first", ProviderSignal::TurnStarted))
        .await
        .unwrap();
    assert!(
        matches!(observer.recv().await,Some(ProviderObservation::Event{event:ProviderEvent{session_id,..},..})if session_id=="first")
    );
    assert!(
        newer
            .ingest(event(
                "first",
                ProviderSignal::Notification {
                    kind: "fixture".into(),
                    title: None,
                    message: "x".repeat(8193)
                }
            ))
            .await
            .is_err()
    );
    stop(&host, &mut view, 1).await;
    host.shutdown().await.unwrap();
    let store = SessionStore::open_memory_only(dir.path(), holder()).unwrap();
    let hint = store.sessions()[0].recovery.as_ref().unwrap();
    assert_eq!(hint.session_id, "first");
    assert_eq!(hint.state_paths, vec!["/tmp/explicit-provider-state"]);
    for file in ["workspace.json", "recovery/sessions.json"] {
        let text = std::fs::read_to_string(dir.path().join(file)).unwrap();
        assert!(!text.contains("EPHEMERAL_PRIVATE_MESSAGE"));
        assert!(!text.contains("STALE_PRIVATE_MESSAGE"));
    }
}
#[tokio::test]
async fn sibling_uses_observed_cwd_fresh_identity_and_sticky_source_history_off() {
    let dir = state();
    let child = dir.path().join("child");
    std::fs::create_dir(&child).unwrap();
    let child = std::fs::canonicalize(child).unwrap();
    let mut source = profile(dir.path());
    source.recovery = Some(RecoveryHint {
        provider: "fixture".into(),
        session_id: "do-not-copy".into(),
        working_directory: dir.path().to_string_lossy().into_owned(),
        state_paths: vec![],
    });
    let host = start(dir.path(), vec![source]).await;
    let mut first = open(&host, 1).await;
    assert!(matches!(
        host.spawn_from(1, SpawnFromOptions::default()).await,
        Err(SpawnError::CwdUnavailable { id: 1 })
    ));
    input(&host,&first,1,1,format!("cd '{}'; printf '\\033]7;file://localhost%s\\007' \"$PWD\"; printf '%s' \"$$\" > '{}'; printf 'SOURCE_AT_CHILD\\n'\n",child.display(),dir.path().join("source.pid").display())).await;
    snapshot(&host, &mut first, 1, "SOURCE_AT_CHILD").await;
    let spawned = host
        .spawn_from(1, SpawnFromOptions::default())
        .await
        .unwrap();
    assert_ne!(spawned.id, 1);
    assert_eq!(spawned.cwd, child);
    assert_eq!(spawned.cwd_source, CwdSource::Observed);
    let mut second = open(&host, spawned.id).await;
    input(
        &host,
        &second,
        spawned.id,
        1,
        format!(
            "printf '%s' \"$$\" > '{}'; printf '%s' \"$PWD\" > '{}'; printf 'SIBLING_READY\\n'\n",
            dir.path().join("sibling.pid").display(),
            dir.path().join("sibling.cwd").display()
        ),
    )
    .await;
    snapshot(&host, &mut second, spawned.id, "SIBLING_READY").await;
    input(
        &host,
        &first,
        1,
        2,
        "printf 'ORIGINAL_STILL_ALIVE\\n'\n".into(),
    )
    .await;
    snapshot(&host, &mut first, 1, "ORIGINAL_STILL_ALIVE").await;
    let pid1 = std::fs::read_to_string(dir.path().join("source.pid")).unwrap();
    let pid2 = std::fs::read_to_string(dir.path().join("sibling.pid")).unwrap();
    let settings = Recovery::open(dir.path().join("recovery"))
        .unwrap()
        .settings()
        .unwrap();
    let no_recovery = settings[&spawned.id].recovery.is_none();
    let off = matches!(settings[&spawned.id].history, HistoryPolicy::Off);
    stop(&host, &mut first, 1).await;
    stop(&host, &mut second, spawned.id).await;
    host.shutdown().await.unwrap();
    assert_ne!(pid1, pid2);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("sibling.cwd")).unwrap(),
        child.to_string_lossy()
    );
    assert!(no_recovery && off);
}
#[tokio::test]
async fn sibling_last_known_override_profile_endpoint_and_launch_failure_are_explicit() {
    let dir = state();
    let source = profile(dir.path());
    let mut other = source.clone();
    other.name = "explicit".into();
    other.endpoint = Endpoint::Ssh {
        config: "/nonexistent/config".into(),
        destination: "not-a-host".into(),
        holder: "/missing".into(),
        root: "/missing".into(),
    };
    let host = start(dir.path(), vec![source, other]).await;
    let mut first = open(&host, 1).await;
    assert!(matches!(
        host.spawn_from(999, SpawnFromOptions::default()).await,
        Err(SpawnError::Missing { id: 999 })
    ));
    assert!(matches!(
        host.spawn_from(
            1,
            SpawnFromOptions {
                profile: SpawnProfile::Named("removed".into()),
                ..Default::default()
            }
        )
        .await,
        Err(SpawnError::ProfileUnavailable { .. })
    ));
    assert!(matches!(
        host.spawn_from(
            1,
            SpawnFromOptions {
                cwd: SpawnCwd::Explicit("relative".into()),
                ..Default::default()
            }
        )
        .await,
        Err(SpawnError::InvalidCwd { .. })
    ));
    let spawned = host
        .spawn_from(
            1,
            SpawnFromOptions {
                profile: SpawnProfile::Named("explicit".into()),
                cwd: SpawnCwd::AllowLastKnown,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(spawned.cwd_source, CwdSource::LastKnown);
    assert_eq!(spawned.cwd, dir.path());
    let mut second = open(&host, spawned.id).await;
    input(
        &host,
        &second,
        spawned.id,
        1,
        "printf 'LOCAL_ENDPOINT_PRESERVED\\n'\n".into(),
    )
    .await;
    snapshot(&host, &mut second, spawned.id, "LOCAL_ENDPOINT_PRESERVED").await;
    let missing = dir.path().join("directory-does-not-exist");
    let failure = host
        .spawn_from(
            1,
            SpawnFromOptions {
                cwd: SpawnCwd::Explicit(missing),
                ..Default::default()
            },
        )
        .await;
    let failed_id = match failure {
        Err(SpawnError::Launch { id: Some(id), .. }) => id,
        other => panic!("unexpected result: {other:?}"),
    };
    stop(&host, &mut first, 1).await;
    stop(&host, &mut second, spawned.id).await;
    assert!(matches!(
        host.spawn_from(
            1,
            SpawnFromOptions {
                cwd: SpawnCwd::AllowLastKnown,
                ..Default::default()
            }
        )
        .await,
        Err(SpawnError::Unavailable { id: 1 })
    ));
    host.shutdown().await.unwrap();
    let store = SessionStore::open_memory_only(dir.path(), holder()).unwrap();
    assert_eq!(store.sessions().len(), 3);
    assert!(
        store
            .sessions()
            .iter()
            .all(|s| matches!(s.spec.endpoint, Endpoint::Local))
    );
    assert!(store.sessions().iter().any(|s| s.id == failed_id));
}
async fn output_until(view: &mut Subscription, needle: &str) -> String {
    let mut text = String::new();
    loop {
        if let ServerFrame::Output { bytes, .. } = next(view).await {
            text.push_str(&String::from_utf8_lossy(&bytes));
            if text.contains(needle) {
                return text;
            }
        }
    }
}
#[tokio::test]
#[ignore = "requires explicitly configured trusted SSH fixture and matching memory-only holder"]
async fn ssh_sibling_has_fresh_pid_same_remote_os_and_observed_directory() {
    let dir = state();
    let mut source = profile(std::path::Path::new("/tmp"));
    source.endpoint = Endpoint::Ssh {
        config: std::env::var_os("TERMINAL_LAB_SSH_CONFIG")
            .expect("set trusted SSH config")
            .into(),
        destination: std::env::var("TERMINAL_LAB_SSH_HOST").unwrap_or("lima-h2-terminal-c1".into()),
        holder: std::env::var_os("TERMINAL_LAB_SSH_HOLDER")
            .map(PathBuf::from)
            .unwrap_or("/tmp/h2-terminal-holder".into()),
        root: format!(
            "/var/tmp/host-sibling-{}-{}",
            std::process::id(),
            dir.path().file_name().unwrap().to_string_lossy()
        ),
    };
    let mut other = profile(std::path::Path::new("/tmp"));
    other.name = "fresh-shell".into();
    let host = start(dir.path(), vec![source, other]).await;
    let mut first = open(&host, 1).await;
    let cwd = format!(
        "/tmp/h2-sibling-{}-{}",
        std::process::id(),
        dir.path().file_name().unwrap().to_string_lossy()
    );
    input(&host,&first,1,1,format!("stty -echo; mkdir -p '{cwd}'; cd '{cwd}'; printf '\\033]7;file://localhost%s\\007' \"$PWD\"; printf 'SOURCE_OS=%s PID=%s CWD=%s DONE\\n' \"$(uname -s)\" \"$$\" \"$PWD\"\n")).await;
    let original = output_until(&mut first, " DONE\r\n").await;
    let spawned = host
        .spawn_from(
            1,
            SpawnFromOptions {
                profile: SpawnProfile::Named("fresh-shell".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let mut second = open(&host, spawned.id).await;
    input(&host,&second,spawned.id,1,"stty -echo; printf 'SIBLING_OS=%s PID=%s CWD=%s DONE\\n' \"$(uname -s)\" \"$$\" \"$PWD\"\n".into()).await;
    let sibling = output_until(&mut second, " DONE\r\n").await;
    input(
        &host,
        &first,
        1,
        2,
        "printf 'SOURCE_STILL_ALIVE\\n'\n".into(),
    )
    .await;
    let original_alive = output_until(&mut first, "SOURCE_STILL_ALIVE\r\n").await;
    let sibling_uuid = terminal_uuid(&host, spawned.id).await;
    let native = host.terminal(&sibling_uuid).await.unwrap();
    let native_receipt = native
        .inject_text_with_control(
            "printf 'NATIVE_SSH_DONE\\n'".into(),
            InjectionOptions {
                submit: true,
                ..InjectionOptions::raw()
            },
            ControlPolicy::TakeControl,
        )
        .await
        .unwrap();
    assert_eq!(
        native_receipt.written().await.unwrap().terminal_uuid,
        sibling_uuid
    );
    let native_output = output_until(&mut second, "NATIVE_SSH_DONE\r\n").await;
    host.frame(second.id, ClientFrame::TakeControl { id: spawned.id })
        .await
        .unwrap();
    input(
        &host,
        &second,
        spawned.id,
        2,
        "cd /tmp; printf 'LEFT_DIRECTORY\\n'\n".into(),
    )
    .await;
    let _ = output_until(&mut second, "LEFT_DIRECTORY\r\n").await;
    input(
        &host,
        &first,
        1,
        3,
        format!("cd /tmp; rmdir '{cwd}'; printf 'REMOTE_CLEANUP\\n'\n"),
    )
    .await;
    let _ = output_until(&mut first, "REMOTE_CLEANUP\r\n").await;
    stop(&host, &mut first, 1).await;
    stop(&host, &mut second, spawned.id).await;
    host.shutdown().await.unwrap();
    let pid = |s: &str| {
        s.split(" PID=")
            .skip(1)
            .filter_map(|tail| tail.split_whitespace().next())
            .find_map(|candidate| candidate.parse::<u32>().ok())
            .expect("runtime shell PID after echoed format placeholders")
    };
    assert_ne!(pid(&original), pid(&sibling));
    assert!(original.contains("SOURCE_OS=Linux"));
    assert!(sibling.contains("SIBLING_OS=Linux"));
    assert!(original.contains(&format!("CWD={cwd}")));
    assert!(sibling.contains(&format!("CWD={cwd}")));
    assert_eq!(spawned.cwd, PathBuf::from(cwd));
    assert_eq!(spawned.cwd_source, CwdSource::Observed);
    assert!(original_alive.contains("SOURCE_STILL_ALIVE"));
    assert!(native_output.contains("NATIVE_SSH_DONE"));
}
#[tokio::test]
async fn output_burst_uses_bounded_resync_without_dropping_controller() {
    let dir = state();
    let host = start(dir.path(), vec![profile(dir.path())]).await;
    let mut view = open(&host, 1).await;
    let initial = snapshot(&host, &mut view, 1, "").await;
    drop(initial);
    host.frame(view.id, ClientFrame::SnapshotRequest { id: 1 })
        .await
        .unwrap();
    let offset = loop {
        if let ServerFrame::Snapshot { offset, .. } = next(&mut view).await {
            break offset;
        }
    };
    let chunk = b"small output\r\n";
    for i in 0..256 {
        host.tx
            .send(Command::Output(
                1,
                Update::Record(Record::Output {
                    offset: offset + i * chunk.len() as u64,
                    bytes: chunk.to_vec(),
                }),
            ))
            .await
            .unwrap();
    }
    // A native API request is an ordered actor barrier after all injected output.
    let _ = host.bind_provider(1, context()).await.unwrap();
    let mut saw_resync = false;
    while let Ok(frame) = view.receiver.try_recv() {
        if matches!(frame.frame, ServerFrame::ResyncRequired { id: 1, .. }) {
            saw_resync = true;
        }
    }
    let still_connected = !view.receiver.is_closed();
    if still_connected {
        host.frame(view.id, ClientFrame::TakeControl { id: 1 })
            .await
            .unwrap();
        loop {
            if matches!(
                next(&mut view).await,
                ServerFrame::Control {
                    id: 1,
                    controller: true
                }
            ) {
                break;
            }
        }
        let _ = snapshot(&host, &mut view, 1, "small output").await;
        stop(&host, &mut view, 1).await;
    } else {
        // Directly stop only this fixture holder if the pre-fix queue evicts it.
        let mut cleanup = host.subscribe().await.unwrap();
        let _ = next(&mut cleanup).await;
        host.frame(cleanup.id, ClientFrame::Open { id: 1 })
            .await
            .unwrap();
        loop {
            if matches!(
                next(&mut cleanup).await,
                ServerFrame::Control {
                    controller: true,
                    ..
                }
            ) {
                break;
            }
        }
        stop(&host, &mut cleanup, 1).await;
    }
    host.shutdown().await.unwrap();
    assert!(
        saw_resync && still_connected,
        "bounded output bursts must resynchronize rather than disconnect the controller"
    );
}
async fn terminal_uuid(host: &Host, id: u64) -> String {
    let mut view = host.subscribe().await.unwrap();
    let uuid = match next(&mut view).await {
        ServerFrame::Sessions { sessions, .. } => {
            sessions.into_iter().find(|s| s.id == id).unwrap().uuid
        }
        _ => panic!("missing sessions"),
    };
    host.leave(view.id).await;
    uuid
}
#[tokio::test]
async fn durable_handle_names_attributes_and_automatic_title_survive_reopen() {
    let dir = state();
    let launch = profile(dir.path());
    let host = start(dir.path(), vec![launch.clone()]).await;
    let uuid = terminal_uuid(&host, 1).await;
    let handle = host.terminal(&uuid).await.unwrap();
    let manual = handle
        .set_name_with_source("Manual title".into(), NameSource::Manual)
        .await
        .unwrap();
    assert_eq!(manual.title, "Manual title");
    let first = handle
        .inject_text(
            "printf '\\033]2;Runtime title\\007'; printf 'TITLE_EVENT_DONE\\n'".into(),
            InjectionOptions {
                submit: true,
                ..InjectionOptions::raw()
            },
        )
        .await
        .unwrap();
    let ticket = first.written().await.unwrap();
    let mut view = open(&host, 1).await;
    let _ = snapshot(&host, &mut view, 1, "TITLE_EVENT_DONE").await;
    let info = handle.info().await.unwrap();
    assert_eq!(info.uuid, uuid);
    assert_eq!(info.title, "Manual title");
    assert!(info.automatic_title.ends_with("Runtime title"));
    let programmatic = handle.set_name("Programmatic title".into()).await.unwrap();
    assert_eq!(
        programmatic.name_override.unwrap().source,
        NameSource::Programmatic
    );
    let attrs = BTreeMap::from([
        ("issue".into(), "TASK-9".into()),
        ("group".into(), "work".into()),
    ]);
    handle.set_attrs(attrs.clone()).await.unwrap();
    assert!(
        handle
            .set_attrs(BTreeMap::from([("oversized".into(), "x".repeat(4097))]))
            .await
            .is_err()
    );
    assert_eq!(handle.info().await.unwrap().attributes, attrs);
    let automatic = handle.clear_name().await.unwrap();
    assert_eq!(automatic.title, automatic.automatic_title);
    handle
        .set_name_with_source("Saved manual".into(), NameSource::Manual)
        .await
        .unwrap();
    host.shutdown().await.unwrap();
    assert!(matches!(handle.info().await, Err(HandleError::Closed)));
    let host = start(dir.path(), vec![launch]).await;
    let restored = host.terminal(&uuid).await.unwrap();
    let info = restored.info().await.unwrap();
    assert_eq!(info.uuid, uuid);
    assert_eq!(info.title, "Saved manual");
    assert_eq!(info.attributes, attrs);
    assert_eq!(info.name_override.unwrap().source, NameSource::Manual);
    let fresh = restored
        .inject_text(
            "printf 'NEW_HOST_INPUT\\n'".into(),
            InjectionOptions {
                submit: true,
                ..InjectionOptions::raw()
            },
        )
        .await
        .unwrap();
    let fresh_ticket = fresh.written().await.unwrap();
    assert_ne!(ticket.connection_epoch, fresh_ticket.connection_epoch);
    assert_eq!(ticket.terminal_uuid, fresh_ticket.terminal_uuid);
    let mut view = open(&host, 1).await;
    snapshot(&host, &mut view, 1, "NEW_HOST_INPUT").await;
    stop(&host, &mut view, 1).await;
    host.shutdown().await.unwrap();
}
#[tokio::test]
async fn real_native_injection_preserves_bytes_and_adds_no_implicit_enter() {
    let dir = state();
    let mut launch = profile(dir.path());
    launch.args=vec!["-c".into(),"stty -icanon -echo min 1 time 0; printf 'RAW_READY\\n'; dd bs=1 count=5 of=\"$1/first\" 2>/dev/null; printf 'FIVE_RECEIVED\\n'; dd bs=1 count=1 of=\"$1/sixth\" 2>/dev/null; printf 'SIX_RECEIVED\\n'; exec /bin/sh -i".into(),"raw-fixture".into(),dir.path().to_string_lossy().into_owned()];
    let host = start(dir.path(), vec![launch]).await;
    let uuid = terminal_uuid(&host, 1).await;
    let handle = host.terminal(&uuid).await.unwrap();
    let mut view = open(&host, 1).await;
    snapshot(&host, &mut view, 1, "RAW_READY").await;
    assert!(matches!(
        handle.stop().await,
        Err(HandleError::Controlled { .. })
    ));
    assert!(matches!(
        handle
            .inject_text("abcde".into(), InjectionOptions::raw())
            .await,
        Err(HandleError::Controlled { .. })
    ));
    let queued = handle
        .inject_text_with_control(
            "abcde".into(),
            InjectionOptions::raw(),
            ControlPolicy::TakeControl,
        )
        .await
        .unwrap();
    let ticket = queued.ticket().clone();
    assert_eq!(queued.written().await.unwrap(), ticket);
    snapshot(&host, &mut view, 1, "FIVE_RECEIVED").await;
    let five = std::fs::read(dir.path().join("first")).unwrap();
    let sixth_before = std::fs::metadata(dir.path().join("sixth"))
        .map(|m| m.len())
        .unwrap_or(0);
    let queued = handle
        .inject_text("Z".into(), InjectionOptions::raw())
        .await
        .unwrap();
    let ticket2 = queued.written().await.unwrap();
    snapshot(&host, &mut view, 1, "SIX_RECEIVED").await;
    let sixth = std::fs::read(dir.path().join("sixth")).unwrap();
    host.frame(view.id, ClientFrame::TakeControl { id: 1 })
        .await
        .unwrap();
    handle
        .stop_with_control(ControlPolicy::TakeControl)
        .await
        .unwrap();
    loop {
        if matches!(next(&mut view).await, ServerFrame::Exited { id: 1, .. }) {
            break;
        }
    }
    assert!(matches!(
        handle.info().await.unwrap().state,
        SessionState::Exited { .. }
    ));
    host.shutdown().await.unwrap();
    assert_eq!(five, b"abcde");
    assert_eq!(sixth_before, 0);
    assert_eq!(sixth, b"Z");
    assert!(ticket2.seq > ticket.seq);
    assert_eq!(ticket2.connection_epoch, ticket.connection_epoch);
}
#[tokio::test]
async fn native_receipts_wait_for_holder_ack_and_disconnect_is_uncertain_without_resend() {
    let dir = state();
    let mut launch = profile(dir.path());
    launch.args = vec![
        "-c".into(),
        "stty -echo; printf '\\033[?2004hPASTE_READY\\n'; while :; do sleep 1; done".into(),
    ];
    let host = start(dir.path(), vec![launch]).await;
    let uuid = terminal_uuid(&host, 1).await;
    let handle = host.terminal(&uuid).await.unwrap();
    let mut view = open(&host, 1).await;
    let vt = snapshot(&host, &mut view, 1, "PASTE_READY").await;
    assert!(vt.is_bracketed_paste());
    let (reply, answer) = tokio::sync::oneshot::channel();
    host.tx
        .send(Command::Native(native::NativeCommand::WriterFixture {
            id: 1,
            reply,
        }))
        .await
        .unwrap();
    let (mut requests, original) = answer.await.unwrap().unwrap();
    let queued = handle
        .inject_text_with_control(
            "line1\nline2".into(),
            InjectionOptions::paste(),
            ControlPolicy::TakeControl,
        )
        .await
        .unwrap();
    let ticket = queued.ticket().clone();
    let bytes = match requests.recv().await.unwrap() {
        Write::Native { bytes, ticket: seq } => {
            assert_eq!(seq, ticket.seq);
            bytes
        }
        _ => panic!("expected native write"),
    };
    let mut written = tokio::spawn(queued.written());
    host.tx
        .send(Command::Native(native::NativeCommand::Sent {
            id: 1,
            ticket: ticket.seq,
            internal: 9001,
        }))
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(50), &mut written)
            .await
            .is_err()
    );
    host.tx
        .send(Command::Output(1, Update::Written(9001)))
        .await
        .unwrap();
    assert_eq!(written.await.unwrap().unwrap(), ticket);
    let second = handle
        .inject_text("not resent".into(), InjectionOptions::raw())
        .await
        .unwrap();
    let uncertain_ticket = second.ticket().clone();
    let _ = requests.recv().await.unwrap();
    host.tx.send(Command::Disconnected(1)).await.unwrap();
    let result = second.written().await;
    // The simulated disconnect deliberately removed the host writer. Clean up
    // through a fresh real attachment; EOF is also a valid stop outcome.
    drop(original);
    host.shutdown().await.unwrap();
    let store = SessionStore::open_memory_only(dir.path(), holder()).unwrap();
    let mut attached = store.attach_live_from(1, 0).await.unwrap();
    attached.writer.stop().await.unwrap();
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            if matches!(
                attached.reader.next().await.unwrap(),
                Some(Update::Exited(_)) | None
            ) {
                break;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(bytes, b"\x1b[200~line1\nline2\x1b[201~");
    assert!(
        matches!(result,Err(HandleError::Uncertain{ticket,reason:WriteUncertainty::TransportLost,..})if ticket==uncertain_ticket)
    );
    assert!(
        requests.try_recv().is_err(),
        "uncertain input was automatically replayed"
    );
}

#[tokio::test]
async fn retained_and_forward_output_gaps_revoke_live_view_before_refusing_more_input() {
    for forward in [false, true] {
        let dir = state();
        let host = start(dir.path(), vec![profile(dir.path())]).await;
        let mut view = open(&host, 1).await;
        let update = if forward {
            Update::Record(Record::Output {
                offset: 1 << 30,
                bytes: b"untrusted tail".to_vec(),
            })
        } else {
            Update::Record(Record::Gap {
                from: 0,
                to: 1 << 30,
            })
        };
        host.tx.send(Command::Output(1, update)).await.unwrap();
        let mut revoked = false;
        let mut readonly = false;
        while !revoked || !readonly {
            match next(&mut view).await {
                ServerFrame::Control {
                    id: 1,
                    controller: false,
                } => revoked = true,
                ServerFrame::Snapshot {
                    id: 1, live: false, ..
                } => readonly = true,
                _ => {}
            }
        }
        host.frame(
            view.id,
            ClientFrame::Input {
                id: 1,
                seq: 1,
                bytes: b"must_not_execute\r".to_vec(),
            },
        )
        .await
        .unwrap();
        let refused = loop {
            if let ServerFrame::Error { code, .. } = next(&mut view).await {
                if code == "read_only" {
                    break true;
                }
            }
        };
        host.shutdown().await.unwrap();
        let store = SessionStore::open_memory_only(dir.path(), holder()).unwrap();
        let mut attached = store.attach_live_from(1, 0).await.unwrap();
        attached.writer.stop().await.unwrap();
        tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                if matches!(
                    attached.reader.next().await.unwrap(),
                    Some(Update::Exited(_)) | None
                ) {
                    break;
                }
            }
        })
        .await
        .unwrap();
        assert!(revoked && readonly && refused);
    }
}
