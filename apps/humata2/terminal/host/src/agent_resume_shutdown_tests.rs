//! Real launch ownership: shutdown must settle an already-started recovery worker.
use super::*;
use std::{
    ffi::CString,
    fs::{File, OpenOptions},
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::Path,
    sync::Arc,
};
use terminal_provider::{ResumeAdapter, ResumeError, ResumeIdentity, ResumePolicy, ResumeRegistry};

/// Stops every process whose command line names this test's own folder (its
/// holders, under `/private/tmp`), even when the test fails partway.
struct OwnHolders(String);
impl OwnHolders {
    fn of(dir: &std::path::Path) -> Self {
        let dir = std::fs::canonicalize(dir).unwrap();
        assert!(dir.starts_with("/private/tmp/"), "{} is not under /private/tmp", dir.display());
        Self(format!("{}/", dir.display()))
    }
}
impl Drop for OwnHolders {
    fn drop(&mut self) {
        let _ = std::process::Command::new("pkill").arg("-f").arg(&self.0).status();
    }
}

const ADAPTER: &str = "shutdown-race-fixture-v1";
const PROVIDER: &str = "shutdown-race-fixture";
const SESSION: &str = "exact-fixture-session-001";
const RESUME_ARG: &str = "--resume-shutdown-fixture";

struct FixtureAdapter;
impl ResumeAdapter for FixtureAdapter {
    fn key(&self) -> &str {
        ADAPTER
    }
    fn args(&self, identity: &ResumeIdentity, _: &[String]) -> Result<Vec<String>, ResumeError> {
        Ok(vec![RESUME_ARG.into(), identity.session_id.clone()])
    }
}

fn quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}
fn script(path: &Path, contents: &str) {
    std::fs::write(path, contents).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
}

/// An open read/write FIFO keeps releasing the owned launcher nonblocking,
/// including during assertion unwinding; it never touches another process.
struct ReleaseGate(File);
impl ReleaseGate {
    fn release(&mut self) {
        self.0.write_all(b"release\n").unwrap();
    }
}
impl Drop for ReleaseGate {
    fn drop(&mut self) {
        let _ = self.0.write_all(b"release\n");
    }
}
async fn wait_file(path: &Path) {
    use crate::agent_resume_tests::{LAUNCH_WAIT, launch_evidence};
    tokio::time::timeout(LAUNCH_WAIT, async {
        while !path.is_file() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        let root = path.parent().unwrap_or(path);
        panic!(
            "{} did not appear within {LAUNCH_WAIT:?}; {}",
            path.display(),
            launch_evidence(root)
        )
    });
}
fn attempts(path: &Path) -> usize {
    std::fs::read_to_string(path)
        .map(|text| text.lines().count())
        .unwrap_or(0)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_waits_for_paused_resume_launch_and_releases_its_durable_store() {
    let dir = tempfile::Builder::new()
        .prefix("h2-resume-stop-")
        .tempdir_in("/tmp")
        .unwrap();
    let _holders = OwnHolders::of(dir.path());
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let real_holder = std::fs::canonicalize(std::env::var_os("HUMATA2_HOLDER").unwrap()).unwrap();
    let wrapper = dir.path().join("holder-wrapper");
    let agent = dir.path().join("fixture-agent");
    let fifo = dir.path().join("release.fifo");
    let started = dir.path().join("worker-entered");
    let count = dir.path().join("launch-attempts");
    let child_pid = dir.path().join("resumed-agent.pid");
    let fifo_c = CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
    // SAFETY: the CString remains valid for this syscall, with an owned private path.
    assert_eq!(unsafe { libc::mkfifo(fifo_c.as_ptr(), 0o600) }, 0);
    let mut gate = ReleaseGate(
        OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&fifo)
            .unwrap(),
    );
    script(
        &wrapper,
        &format!(
            "#!/bin/sh\nset -eu\nfor arg; do\n if [ \"$arg\" = '{RESUME_ARG}' ]; then\n  printf 'attempt\\n' >> {}\n  printf 'entered\\n' > {}\n  IFS= read -r permit < {}\n fi\ndone\nexec {} \"$@\"\n",
            quote(&count),
            quote(&started),
            quote(&fifo),
            quote(&real_holder),
        ),
    );
    script(
        &agent,
        &format!(
            "#!/bin/sh\nset -eu\ncase \"$1\" in\n --source) exit 23;;\n '{RESUME_ARG}') [ \"$2\" = '{SESSION}' ] || exit 42; printf '%s\\n' \"$$\" > {}; while :; do sleep 30; done;;\n *) exit 43;;\nesac\n",
            quote(&child_pid),
        ),
    );
    let hint = RecoveryHint {
        provider: PROVIDER.into(),
        session_id: SESSION.into(),
        working_directory: dir.path().to_string_lossy().into_owned(),
        state_paths: Vec::new(),
    };
    let profile = Profile {
        name: "fixture".into(),
        title: "Shutdown race fixture".into(),
        program: agent,
        args: vec!["--source".into()],
        cwd: dir.path().into(),
        endpoint: Endpoint::Local,
        recovery: Some(hint.clone()),
        history: HistoryPolicy::Off,
    };
    let mut registry = ResumeRegistry::default();
    registry.register(Box::new(FixtureAdapter)).unwrap();
    let resume = AgentResumeConfig {
        registry: Arc::new(registry),
        profiles: BTreeMap::from([(
            "fixture".into(),
            ResumePolicy::Enabled {
                adapter: ADAPTER.into(),
                provider: PROVIDER.into(),
                args: Vec::new(),
            },
        )]),
    };
    let host = Host::start_with_agent_resume(
        Config {
            state: dir.path().into(),
            label: "shutdown test".into(),
            start_profiles: vec!["fixture".into()],
            default_history: HistoryPolicy::Off,
        },
        SessionStore::open_memory_only(dir.path(), wrapper.clone()).unwrap(),
        vec![profile],
        resume,
    )
    .await
    .unwrap();
    wait_file(&started).await;
    // This marker is written by the replacement HOLDER before READY, so the
    // launch worker is genuinely in flight, not merely an idle agent process.
    assert_eq!(attempts(&count), 1);
    let mut shutdown = Box::pin(host.shutdown());
    let early = tokio::time::timeout(Duration::from_millis(200), &mut shutdown).await;
    let waited_for_worker = early.is_err();
    gate.release();
    let stopped = match early {
        Ok(result) => result,
        Err(_) => tokio::time::timeout(Duration::from_secs(8), &mut shutdown)
            .await
            .unwrap(),
    };
    let attempts_at_return = attempts(&count);
    // Opening immediately is the lock barrier assertion; do not wait for a
    // lingering worker to drop SessionStore before attempting this operation.
    let reopened = SessionStore::open_memory_only(dir.path(), wrapper.clone());
    let lock_released_at_return = reopened.is_ok();
    wait_file(&child_pid).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let attempts_after_return = attempts(&count);
    // Preserve the immediate-open failure as an assertion, but still reacquire
    // this private fixture for cleanup if the regression leaves a worker behind.
    let store = match reopened {
        Ok(store) => store,
        Err(_) => tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                match SessionStore::open_memory_only(dir.path(), wrapper.clone()) {
                    Ok(store) => break store,
                    Err(_) => tokio::time::sleep(Duration::from_millis(10)).await,
                }
            }
        })
        .await
        .unwrap(),
    };
    let saved = store.sessions().to_vec();
    let resumed = saved
        .iter()
        .find(|session| {
            session
                .startup_profile
                .as_deref()
                .is_some_and(|key| key.starts_with("agent-resume:"))
        })
        .cloned();
    let settings = Recovery::open(dir.path().join("recovery"))
        .unwrap()
        .settings()
        .unwrap();
    // Clean up only this test's holder/PTYs before reporting assertion results.
    for session in &saved {
        let _ = store.stop(session.id).await;
    }
    drop(store);
    stopped.unwrap();
    assert!(
        waited_for_worker,
        "shutdown returned while its recovery launch was still blocked"
    );
    assert!(
        lock_released_at_return,
        "shutdown returned before releasing its worker's durable store lock"
    );
    assert_eq!(attempts_at_return, 1);
    assert_eq!(
        attempts_after_return, 1,
        "a recovery launch occurred after shutdown returned"
    );
    assert_eq!(
        saved.len(),
        2,
        "one source and one durable replacement must remain inspectable"
    );
    let resumed = resumed.expect("shutdown lost the durable replacement reservation");
    assert_eq!(resumed.recovery, Some(hint.clone()));
    let source = saved
        .iter()
        .find(|session| session.id != resumed.id)
        .unwrap();
    assert_eq!(
        resumed.attributes.get("terminal.resume.source"),
        Some(&source.uuid)
    );
    let setting = settings
        .get(&resumed.id)
        .expect("the settled replacement lost host settings");
    assert_eq!(setting.profile.as_deref(), Some("fixture"));
    assert_eq!(setting.recovery, Some(hint));
}

fn stop_policy() -> AgentResumeConfig {
    let mut registry = ResumeRegistry::default();
    registry.register(Box::new(FixtureAdapter)).unwrap();
    AgentResumeConfig {
        registry: Arc::new(registry),
        profiles: BTreeMap::from([(
            "fixture".into(),
            ResumePolicy::Enabled {
                adapter: ADAPTER.into(),
                provider: PROVIDER.into(),
                args: Vec::new(),
            },
        )]),
    }
}

async fn explicit_stop_stays_stopped_after_restart(viewer_stop: bool) {
    let dir = tempfile::Builder::new()
        .prefix("h2-stop-policy-")
        .tempdir_in("/tmp")
        .unwrap();
    let _holders = OwnHolders::of(dir.path());
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let holder = std::fs::canonicalize(std::env::var_os("HUMATA2_HOLDER").unwrap()).unwrap();
    let agent = dir.path().join("agent");
    let launches = dir.path().join("launches");
    script(
        &agent,
        &format!(
            "#!/bin/sh\nset -eu\ncase \"$1\" in\n --source) ;;\n '{RESUME_ARG}') [ \"$2\" = '{SESSION}' ] || exit 42;;\n *) exit 43;;\nesac\nprintf '%s\\n' \"$1\" >> {}\nexec /bin/sh -i\n",
            quote(&launches),
        ),
    );
    let profile = Profile {
        name: "fixture".into(),
        title: "Explicit stop fixture".into(),
        program: agent,
        args: vec!["--source".into()],
        cwd: dir.path().into(),
        endpoint: Endpoint::Local,
        recovery: Some(RecoveryHint {
            provider: PROVIDER.into(),
            session_id: SESSION.into(),
            working_directory: dir.path().to_string_lossy().into_owned(),
            state_paths: Vec::new(),
        }),
        history: HistoryPolicy::default(),
    };
    let config = || Config {
        state: dir.path().into(),
        label: "Stop policy".into(),
        start_profiles: vec!["fixture".into()],
        default_history: HistoryPolicy::default(),
    };
    let host = Host::start_with_agent_resume(
        config(),
        SessionStore::open_memory_only(dir.path(), holder.clone()).unwrap(),
        vec![profile.clone()],
        stop_policy(),
    )
    .await
    .unwrap();
    wait_file(&launches).await;
    let original = host.terminals().await.unwrap().remove(0);
    let terminal = host.terminal(&original.uuid).await.unwrap();
    let _viewer = if viewer_stop {
        let mut viewer = host.subscribe().await.unwrap();
        host.frame(viewer.id, ClientFrame::Open { id: original.id })
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                if matches!(viewer.receiver.recv().await.unwrap().frame, ServerFrame::Control { id, controller: true } if id == original.id) { break; }
            }
        }).await.unwrap();
        host.frame(viewer.id, ClientFrame::Stop { id: original.id })
            .await
            .unwrap();
        Some(viewer)
    } else {
        terminal.stop().await.unwrap();
        None
    };
    // A transport may already have queued the real nonzero child exit when
    // explicit Stop is accepted. Supply that delayed report deterministically;
    // otherwise a missing holder alone would never exercise the auto-exit path.
    host.tx
        .send(Command::Output(original.id, Update::Exited(143)))
        .await
        .unwrap();
    assert!(matches!(
        terminal.info().await.unwrap().state,
        SessionState::Exited { code: Some(143) }
    ));
    host.shutdown().await.unwrap();
    let stopped_settings = Recovery::open(dir.path().join("recovery"))
        .unwrap()
        .settings()
        .unwrap();
    let durable_stop = stopped_settings.get(&original.id).unwrap().resume_blocked;
    let host = Host::start_with_agent_resume(
        config(),
        SessionStore::open_memory_only(dir.path(), holder).unwrap(),
        vec![profile],
        stop_policy(),
    )
    .await
    .unwrap();
    // Cover the immediate startup tick and at least one subsequent auto-resume
    // tick, with the saved nonzero exit still present in the restored screen.
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let after = host.terminals().await.unwrap();
    let count = attempts(&launches);
    // Cleanup also handles the regression case where a duplicate was launched.
    for info in &after {
        if matches!(info.state, SessionState::Running) {
            let _ = host
                .terminal(&info.uuid)
                .await
                .unwrap()
                .stop_with_control(ControlPolicy::TakeControl)
                .await;
        }
    }
    host.shutdown().await.unwrap();
    assert!(
        durable_stop,
        "explicit Stop did not persist its recovery opt-out"
    );
    assert_eq!(count, 1, "restart auto-resumed an explicitly stopped agent");
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].uuid, original.uuid);
    assert!(matches!(
        after[0].state,
        SessionState::Exited { code: Some(143) }
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explicit_native_stop_is_not_undone_by_automatic_recovery_after_restart() {
    explicit_stop_stays_stopped_after_restart(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explicit_viewer_stop_is_not_undone_by_automatic_recovery_after_restart() {
    explicit_stop_stays_stopped_after_restart(true).await;
}
