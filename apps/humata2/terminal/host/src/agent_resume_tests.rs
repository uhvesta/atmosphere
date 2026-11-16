//! Recovery dispatch tests use a strict owned CLI fixture, not a model provider.
use super::*;
use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command as ProcessCommand};
use terminal_provider::{CODEX_RESUME_V1, ResumePolicy, ResumeRegistry};
const SESSION: &str = "019a83ae-6368-72f5-90bc-5799b82886a1";
const FORK: &str = "019a83ae-6368-72f5-90bc-5799b82886a2";
struct Fixture {
    directory: tempfile::TempDir,
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::Builder::new()
            .prefix("h2-ar-")
            .tempdir_in("/tmp")
            .unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let path = directory.path().join("agent");
        fs::write(
            &path,
            format!(
                r#"#!/bin/sh
cd '{}' || exit 71
case "$1" in
 boot) marker=boot ;;
 resume) [ "$#" = 2 ] || exit 72; case "$2" in '{}'|'{}') ;; *) exit 72;; esac; marker="$2" ;;
 *) exit 73 ;;
esac
printf '%s %s\n' "$$" "$marker" >> launches
printf 'AGENT_SESSION=%s PID=%s CWD=%s\n' "$marker" "$$" "$PWD"
exec /bin/sh -i
"#,
                directory.path().display(),
                SESSION,
                FORK
            ),
        )
        .unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        Self { directory }
    }
    fn root(&self) -> &Path {
        self.directory.path()
    }
    fn profile(&self, hint: bool) -> Profile {
        Profile {
            name: "agent".into(),
            title: "Fixture agent".into(),
            program: self.root().join("agent"),
            args: vec!["boot".into(), "ORIGINAL_PROMPT_MUST_NOT_REPLAY".into()],
            cwd: self.root().into(),
            endpoint: Endpoint::Local,
            recovery: hint.then(|| RecoveryHint {
                provider: "codex".into(),
                session_id: SESSION.into(),
                working_directory: self.root().display().to_string(),
                state_paths: vec![],
            }),
            history: HistoryPolicy::default(),
        }
    }
    fn config(&self, enabled: bool) -> AgentResumeConfig {
        AgentResumeConfig {
            registry: std::sync::Arc::new(ResumeRegistry::default()),
            profiles: if enabled {
                BTreeMap::from([(
                    "agent".into(),
                    ResumePolicy::Enabled {
                        adapter: CODEX_RESUME_V1.into(),
                        provider: "codex".into(),
                        args: vec![],
                    },
                )])
            } else {
                BTreeMap::new()
            },
        }
    }
    async fn start(&self, enabled: bool, hint: bool) -> Host {
        let holder = fs::canonicalize(std::env::var_os("HUMATA2_HOLDER").unwrap()).unwrap();
        Host::start_with_agent_resume(
            Config {
                state: self.root().into(),
                label: "Agent recovery fixture".into(),
                start_profiles: vec!["agent".into()],
                default_history: HistoryPolicy::default(),
            },
            SessionStore::open_memory_only(self.root(), holder).unwrap(),
            vec![self.profile(hint)],
            self.config(enabled),
        )
        .await
        .unwrap()
    }
    async fn launched(&self, count: usize) -> Vec<String> {
        tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                let rows = fs::read_to_string(self.root().join("launches"))
                    .unwrap_or_default()
                    .lines()
                    .map(str::to_owned)
                    .collect::<Vec<_>>();
                if rows.len() >= count {
                    return rows;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        // The holder argv must still identify this unique fixture before kill.
        for file in fs::read_dir(self.root()).into_iter().flatten().flatten() {
            if file.path().extension().is_some_and(|ext| ext == "pid") {
                if let Ok(pid) = fs::read_to_string(file.path()) {
                    let pid = pid.trim();
                    let output = ProcessCommand::new("ps")
                        .args(["-ww", "-p", pid, "-o", "command="])
                        .output();
                    if output.is_ok_and(|out| {
                        String::from_utf8_lossy(&out.stdout)
                            .contains(&self.root().display().to_string())
                    }) {
                        let _ = ProcessCommand::new("kill").args(["-KILL", pid]).status();
                    }
                }
            }
        }
    }
}
async fn source(host: &Host) -> TerminalHandle {
    let uuid = host.terminals().await.unwrap()[0].uuid.clone();
    host.terminal(&uuid).await.unwrap()
}
async fn input(terminal: &TerminalHandle, text: &str) {
    let _ = terminal
        .inject_text(text.into(), InjectionOptions::raw())
        .await
        .unwrap();
}
async fn exit_seen(terminal: &TerminalHandle) {
    tokio::time::timeout(Duration::from_secs(8), async {
        while !matches!(
            terminal.info().await.unwrap().state,
            SessionState::Exited { .. }
        ) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}
async fn launch_event(observer: &mut AgentResumeObserver) -> AgentResumeLaunch {
    match tokio::time::timeout(Duration::from_secs(10), observer.recv())
        .await
        .unwrap()
        .unwrap()
    {
        AgentResumeObservation::Launched(launch) => launch,
        other => panic!("Expected a successful owned recovery launch: {other:?}"),
    }
}

#[tokio::test]
async fn abnormal_agent_exit_resumes_exact_saved_identity_once_across_host_restart() {
    let fixture = Fixture::new();
    let host = fixture.start(true, false).await;
    fixture.launched(1).await;
    let original = source(&host).await;
    let uuid = original.uuid().to_owned();
    let context = terminal_provider::TrustedContext {
        provider: "codex".into(),
        schema: terminal_provider::CODEX_HOOKS_V1.into(),
        hook_event: None,
        state_paths: vec![],
    };
    let binding = host.bind_provider(original.route(), context).await.unwrap();
    binding
        .ingest(terminal_provider::ProviderEvent {
            provider: "codex".into(),
            schema: terminal_provider::CODEX_HOOKS_V1.into(),
            session_id: SESSION.into(),
            cwd: Some(fixture.root().display().to_string()),
            signal: terminal_provider::ProviderSignal::SessionStarted { source: None },
        })
        .await
        .unwrap();
    let mut observer = host.observe_agent_resume();
    input(&original, "kill -9 $$\r").await;
    let resumed = launch_event(&mut observer).await;
    assert_eq!(resumed.source_uuid, uuid);
    assert_ne!(resumed.terminal_uuid, uuid);
    assert_eq!(resumed.session_id, SESSION);
    assert!(!resumed.existing);
    let launches = fixture.launched(2).await;
    assert!(launches[0].ends_with(" boot"));
    assert!(launches[1].ends_with(SESSION));
    assert_ne!(
        launches[0].split_whitespace().next(),
        launches[1].split_whitespace().next()
    );
    let info = host
        .terminal(&resumed.terminal_uuid)
        .await
        .unwrap()
        .info()
        .await
        .unwrap();
    assert_eq!(info.cwd, fixture.root().display().to_string());
    assert_eq!(info.attributes.get("terminal.resume.source"), Some(&uuid));
    let bytes = fs::read(fixture.root().join("workspace.json")).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("ORIGINAL_PROMPT_MUST_NOT_REPLAY"));
    host.shutdown().await.unwrap();
    let host = fixture.start(true, false).await;
    assert_eq!(host.terminals().await.unwrap().len(), 2);
    let existing = host.resume_agent_after_confirmed_loss(&uuid).await.unwrap();
    assert!(existing.existing);
    assert_eq!(existing.terminal_uuid, resumed.terminal_uuid);
    assert_eq!(fixture.launched(2).await.len(), 2);
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn missing_provider_identity_is_reported_without_starting_a_fresh_conversation() {
    let fixture = Fixture::new();
    let host = fixture.start(true, false).await;
    fixture.launched(1).await;
    let original = source(&host).await;
    let mut observer = host.observe_agent_resume();
    input(&original, "kill -9 $$\r").await;
    let event = tokio::time::timeout(Duration::from_secs(8), observer.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(event, AgentResumeObservation::Failed { error, .. } if matches!(error.as_ref(), AgentResumeError::MissingIdentity { .. }))
    );
    let error = host
        .resume_agent_after_confirmed_loss(original.uuid())
        .await
        .unwrap_err();
    assert!(matches!(
        error.as_ref(),
        AgentResumeError::MissingIdentity { .. }
    ));
    assert_eq!(fixture.launched(1).await.len(), 1);
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn ordinary_exit_and_explicit_stop_do_not_trigger_agent_recovery() {
    for stop in [false, true] {
        let fixture = Fixture::new();
        let host = fixture.start(true, true).await;
        fixture.launched(1).await;
        let original = source(&host).await;
        let mut observer = host.observe_agent_resume();
        if stop {
            original.stop().await.unwrap();
        } else {
            input(&original, "exit 0\r").await;
        }
        exit_seen(&original).await;
        assert!(
            tokio::time::timeout(Duration::from_millis(1300), observer.recv())
                .await
                .is_err()
        );
        assert_eq!(host.terminals().await.unwrap().len(), 1);
        assert_eq!(fixture.launched(1).await.len(), 1);
        host.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn default_policy_keeps_abnormally_exited_process_as_history() {
    let fixture = Fixture::new();
    let host = fixture.start(false, true).await;
    fixture.launched(1).await;
    let original = source(&host).await;
    let mut observer = host.observe_agent_resume();
    input(&original, "kill -9 $$\r").await;
    exit_seen(&original).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(1300), observer.recv())
            .await
            .is_err()
    );
    assert_eq!(host.terminals().await.unwrap().len(), 1);
    assert_eq!(fixture.launched(1).await.len(), 1);
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn live_agent_cannot_be_replaced_even_with_explicit_loss_request() {
    let fixture = Fixture::new();
    let host = fixture.start(true, true).await;
    let original = source(&host).await;
    let error = host
        .resume_agent_after_confirmed_loss(original.uuid())
        .await
        .unwrap_err();
    assert!(matches!(error.as_ref(), AgentResumeError::Live { .. }));
    assert_eq!(host.terminals().await.unwrap().len(), 1);
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn recovery_child_retains_history_off_when_crash_precedes_host_settings_save() {
    let fixture = Fixture::new();
    let holder = fs::canonicalize(std::env::var_os("HUMATA2_HOLDER").unwrap()).unwrap();
    let mut profile = fixture.profile(true);
    profile.history = HistoryPolicy::Off;
    let host = Host::start_with_agent_resume(
        Config {
            state: fixture.root().into(),
            label: "history-off crash window".into(),
            start_profiles: vec!["agent".into()],
            default_history: HistoryPolicy::default(),
        },
        SessionStore::open_memory_only(fixture.root(), holder).unwrap(),
        vec![profile],
        fixture.config(true),
    )
    .await
    .unwrap();
    fixture.launched(1).await;
    let original = source(&host).await;
    let mut observer = host.observe_agent_resume();
    input(&original, "kill -9 $$\r").await;
    let child = launch_event(&mut observer).await;
    fixture.launched(2).await;
    host.shutdown().await.unwrap();
    let recovery = Recovery::open(fixture.root().join("recovery")).unwrap();
    let mut settings = recovery.settings().unwrap();
    // Exact state after an atomic reservation/launch but before Completed
    // saved Host settings. Parent settings and the child's durable metadata
    // already exist; original argv is deliberately absent from the descriptor.
    assert!(matches!(
        settings.remove(&child.terminal_id).unwrap().history,
        HistoryPolicy::Off
    ));
    recovery.save_settings(&settings).unwrap();
    let host = fixture.start(true, true).await;
    assert_eq!(host.terminals().await.unwrap().len(), 2);
    let existing = host
        .resume_agent_after_confirmed_loss(original.uuid())
        .await
        .unwrap();
    assert_eq!(existing.terminal_uuid, child.terminal_uuid);
    host.shutdown().await.unwrap();
    assert!(
        !fixture
            .root()
            .join(format!("recovery/s{}.checkpoint", child.terminal_id))
            .exists()
    );
    assert_eq!(fixture.launched(2).await.len(), 2);
}

#[tokio::test]
async fn a_failed_resume_is_not_retried_in_an_automatic_crash_loop() {
    let fixture = Fixture::new();
    let path = fixture.root().join("agent");
    let script = fs::read_to_string(&path).unwrap().replace(
        "exec /bin/sh -i",
        "if [ \"$marker\" != boot ]; then exit 77; fi\nexec /bin/sh -i",
    );
    fs::write(path, script).unwrap();
    let host = fixture.start(true, true).await;
    fixture.launched(1).await;
    let original = source(&host).await;
    let mut observer = host.observe_agent_resume();
    input(&original, "kill -9 $$\r").await;
    let child = launch_event(&mut observer).await;
    let terminal = host.terminal(&child.terminal_uuid).await.unwrap();
    exit_seen(&terminal).await;
    assert!(matches!(
        terminal.info().await.unwrap().state,
        SessionState::Exited { code: Some(77) }
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(1300), observer.recv())
            .await
            .is_err()
    );
    assert_eq!(host.terminals().await.unwrap().len(), 2);
    assert_eq!(fixture.launched(2).await.len(), 2);
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn foreground_fork_id_survives_restart_and_late_parent_events_cannot_restore_parent() {
    let fixture = Fixture::new();
    let host = fixture.start(true, true).await;
    fixture.launched(1).await;
    let original = source(&host).await;
    let context = terminal_provider::TrustedContext {
        provider: "codex".into(),
        schema: terminal_provider::CODEX_HOOKS_V1.into(),
        hook_event: None,
        state_paths: vec![],
    };
    let parent = host
        .bind_provider(original.route(), context.clone())
        .await
        .unwrap();
    let started = |session: &str| terminal_provider::ProviderEvent {
        provider: "codex".into(),
        schema: terminal_provider::CODEX_HOOKS_V1.into(),
        session_id: session.into(),
        cwd: Some(fixture.root().display().to_string()),
        signal: terminal_provider::ProviderSignal::SessionStarted {
            source: Some("fork".into()),
        },
    };
    assert!(
        parent
            .activate_fork("not-the-current-parent".into(), started(FORK), vec![])
            .await
            .is_err()
    );
    let child = parent
        .activate_fork(
            SESSION.into(),
            started(FORK),
            vec![fixture.root().join("fork-state.json").display().to_string()],
        )
        .await
        .unwrap();
    assert!(parent.ingest(started(SESSION)).await.is_err());
    assert!(child.ingest(started(SESSION)).await.is_err());
    let hint = original.recovery_hint().await.unwrap().unwrap();
    assert_eq!(hint.session_id, FORK);
    assert_eq!(
        hint.state_paths,
        vec![fixture.root().join("fork-state.json").display().to_string()]
    );
    let mut stale = started(SESSION);
    stale.signal = terminal_provider::ProviderSignal::TurnFinished;
    assert!(child.ingest(stale).await.is_err());
    host.shutdown().await.unwrap();
    // Simulate the narrow crash window after the authoritative workspace ID
    // write but before its companion settings write reached durable storage.
    let recovery = Recovery::open(fixture.root().join("recovery")).unwrap();
    let mut settings = recovery.settings().unwrap();
    settings
        .get_mut(&original.route())
        .unwrap()
        .foreground_session = None;
    settings
        .get_mut(&original.route())
        .unwrap()
        .recovery
        .as_mut()
        .unwrap()
        .session_id = SESSION.into();
    recovery.save_settings(&settings).unwrap();
    let host = fixture.start(true, true).await;
    let original = host.terminal(original.uuid()).await.unwrap();
    let rebound = host.bind_provider(original.route(), context).await.unwrap();
    assert!(rebound.ingest(started(SESSION)).await.is_err());
    let mut observer = host.observe_agent_resume();
    input(&original, "kill -9 $$\r").await;
    let launched = launch_event(&mut observer).await;
    assert_eq!(launched.session_id, FORK);
    assert!(fixture.launched(2).await[1].ends_with(FORK));
    assert_eq!(host.terminals().await.unwrap().len(), 2);
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn failed_fork_companion_save_keeps_committed_child_in_memory_and_revokes_parent() {
    let fixture = Fixture::new();
    let host = fixture.start(true, true).await;
    fixture.launched(1).await;
    let original = source(&host).await;
    let context = terminal_provider::TrustedContext {
        provider: "codex".into(),
        schema: terminal_provider::CODEX_HOOKS_V1.into(),
        hook_event: None,
        state_paths: vec![],
    };
    let parent = host
        .bind_provider(original.route(), context.clone())
        .await
        .unwrap();
    let started = |session: &str| terminal_provider::ProviderEvent {
        provider: "codex".into(),
        schema: terminal_provider::CODEX_HOOKS_V1.into(),
        session_id: session.into(),
        cwd: Some(fixture.root().display().to_string()),
        signal: terminal_provider::ProviderSignal::SessionStarted {
            source: Some("fork".into()),
        },
    };
    // A directory blocks only the companion settings temporary file. The
    // authoritative workspace write is still possible and must stay selected.
    let blocked = fixture.root().join("recovery/sessions.json.tmp");
    fs::create_dir(&blocked).unwrap();
    let error = parent
        .activate_fork(SESSION.into(), started(FORK), vec![])
        .await
        .err()
        .unwrap();
    assert!(matches!(error, Error::RecoverySettingsCommitted { .. }));
    assert_eq!(
        original.recovery_hint().await.unwrap().unwrap().session_id,
        FORK
    );
    let workspace: serde_json::Value =
        serde_json::from_slice(&fs::read(fixture.root().join("workspace.json")).unwrap()).unwrap();
    assert!(workspace.to_string().contains(FORK));
    assert!(parent.ingest(started(SESSION)).await.is_err());
    fs::remove_dir(blocked).unwrap();
    let rebound = host.bind_provider(original.route(), context).await.unwrap();
    assert!(rebound.ingest(started(SESSION)).await.is_err());
    let mut observer = host.observe_agent_resume();
    input(&original, "kill -9 $$\r").await;
    let launched = launch_event(&mut observer).await;
    assert_eq!(launched.session_id, FORK);
    assert!(fixture.launched(2).await[1].ends_with(FORK));
    host.shutdown().await.unwrap();
}
