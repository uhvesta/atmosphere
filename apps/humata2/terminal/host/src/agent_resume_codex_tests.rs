//! Real installed Codex + real Host/holder; only model responses are synthetic.
use super::*;
use futures_util::FutureExt;
use ghostty_vt::Grid;
use serde::Deserialize;
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::fs::PermissionsExt,
    panic::{AssertUnwindSafe, resume_unwind},
    path::Path,
    process::{Child, Command as ProcessCommand, Stdio},
    sync::Arc,
};
use terminal_provider::{CODEX_RESUME_V1, ResumeIdentity, ResumePolicy, ResumeRegistry};

#[derive(Debug, thiserror::Error)]
enum FixtureError {
    #[error("Real Codex Host fixture failed during {stage}: {source}")]
    Io {
        stage: &'static str,
        #[source]
        source: std::io::Error,
    },
    #[error("Real Codex fixture metadata could not be decoded: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("Real Codex fixture failed during {stage}")]
    Invariant { stage: &'static str },
    #[error("Real Codex fixture timed out during {stage}: {source}")]
    Timeout {
        stage: &'static str,
        #[source]
        source: tokio::time::error::Elapsed,
    },
    #[error("Real Codex Host operation failed: {source}")]
    Host {
        #[source]
        source: Error,
    },
    #[error("Real Codex session operation failed: {source}")]
    Session {
        #[source]
        source: terminal_session::Error,
    },
    #[error("Real Codex resume arguments failed validation: {source}")]
    Plan {
        #[source]
        source: terminal_provider::ResumeError,
    },
    #[error("Real Codex automatic recovery failed: {source}")]
    Resume {
        #[source]
        source: Arc<AgentResumeError>,
    },
}
fn io(stage: &'static str, source: std::io::Error) -> FixtureError {
    FixtureError::Io { stage, source }
}
fn require(ok: bool, stage: &'static str) -> Result<(), FixtureError> {
    if ok {
        Ok(())
    } else {
        Err(FixtureError::Invariant { stage })
    }
}

#[derive(Deserialize)]
struct Prepared {
    version: u32,
    session_id: String,
    binary: PathBuf,
    cwd: PathBuf,
    resume_args: Vec<String>,
    wrapper: PathBuf,
    pid_receipts: PathBuf,
    model_requests: PathBuf,
    expected_history: String,
}
#[derive(Deserialize)]
struct PidReceipt {
    pid: i32,
    requested_session_id: String,
}
struct Helper {
    child: Child,
    prepared: Prepared,
}
impl Helper {
    fn start() -> Result<Self, FixtureError> {
        let script = std::env::var_os("TERMINAL_CODEX_HELPER").ok_or(FixtureError::Invariant {
            stage: "missing Bazel helper runfile",
        })?;
        let mut child = ProcessCommand::new("/usr/bin/python3")
            .arg(script)
            .arg("--prepare-host")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| io("start isolated helper", e))?;
        let mut line = String::new();
        let read = BufReader::new(child.stdout.take().ok_or(FixtureError::Invariant {
            stage: "helper stdout",
        })?)
        .read_line(&mut line);
        let parsed = read
            .map_err(|e| io("read helper metadata", e))
            .and_then(|_| {
                serde_json::from_str::<Prepared>(&line)
                    .map_err(|source| FixtureError::Json { source })
            });
        let prepared = match parsed {
            Ok(value) => value,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        let helper = Self { child, prepared };
        let prepared = &helper.prepared;
        require(
            prepared.version == 1 && prepared.binary.is_file() && prepared.wrapper.is_file(),
            "unsupported helper metadata",
        )?;
        require(
            prepared.pid_receipts.parent() == prepared.wrapper.parent()
                && prepared.cwd.parent() == prepared.wrapper.parent()
                && prepared.model_requests.parent() == prepared.wrapper.parent(),
            "helper paths must share their owned directory",
        )?;
        Ok(helper)
    }
    fn receipts(&self) -> Result<Vec<PidReceipt>, FixtureError> {
        let text = match fs::read_to_string(&self.prepared.pid_receipts) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(io("read owned PID receipts", error)),
        };
        // A reader may race file creation or append; only complete JSONL records count.
        text.split_inclusive('\n')
            .filter(|line| line.ends_with('\n'))
            .map(|line| serde_json::from_str(line).map_err(|source| FixtureError::Json { source }))
            .collect()
    }
    fn close(&mut self) -> Result<(), FixtureError> {
        if let Some(mut input) = self.child.stdin.take() {
            input
                .write_all(b"stop\n")
                .map_err(|e| io("release helper", e))?;
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = self.child.try_wait().map_err(|e| io("reap helper", e))? {
                return require(status.success(), "helper exited unsuccessfully");
            }
            if Instant::now() >= deadline {
                self.child
                    .kill()
                    .map_err(|e| io("stop unresponsive owned helper", e))?;
                self.child
                    .wait()
                    .map_err(|e| io("reap stopped helper", e))?;
                return Err(FixtureError::Invariant {
                    stage: "helper did not release normally",
                });
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
impl Drop for Helper {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

fn owned_holder_parent(pid: i32, root: &Path, holder: &Path) -> Result<i32, FixtureError> {
    require(pid > 1, "refuse an invalid PID")?;
    let output = ProcessCommand::new("/bin/ps")
        .args(["-p", &pid.to_string(), "-o", "ppid="])
        .output()
        .map_err(|e| io("read owned CLI parent", e))?;
    let parent = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<i32>()
        .map_err(|_| FixtureError::Invariant {
            stage: "owned CLI parent was unavailable",
        })?;
    require(parent > 1, "refuse an unowned parent")?;
    let output = ProcessCommand::new("/bin/ps")
        .args(["-p", &parent.to_string(), "-o", "args="])
        .output()
        .map_err(|e| io("inspect owned holder", e))?;
    let args = String::from_utf8_lossy(&output.stdout);
    require(
        output.status.success()
            && args.contains(root.to_string_lossy().as_ref())
            && args.contains(holder.file_name().unwrap().to_string_lossy().as_ref()),
        "CLI parent must be this fixture's holder",
    )?;
    Ok(parent)
}
async fn next(view: &mut Subscription) -> Result<ServerFrame, FixtureError> {
    tokio::time::timeout(Duration::from_secs(8), view.receiver.recv())
        .await
        .map_err(|source| FixtureError::Timeout {
            stage: "receive Host frame",
            source,
        })?
        .map(|queued| queued.frame)
        .ok_or(FixtureError::Invariant {
            stage: "Host subscription ended",
        })
}
async fn sessions(host: &Host) -> Result<Vec<SessionInfo>, FixtureError> {
    let mut view = host
        .subscribe()
        .await
        .map_err(|source| FixtureError::Host { source })?;
    let result = match next(&mut view).await? {
        ServerFrame::Sessions { sessions, .. } => Ok(sessions),
        _ => Err(FixtureError::Invariant {
            stage: "initial Host session list",
        }),
    };
    host.leave(view.id).await;
    result
}
async fn receipt(helper: &Helper, index: usize) -> Result<PidReceipt, FixtureError> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(value) = helper.receipts()?.into_iter().nth(index) {
            return Ok(value);
        }
        require(
            Instant::now() < deadline,
            "owned CLI PID receipt never arrived",
        )?;
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
}
async fn historical_screen(
    host: &Host,
    id: u64,
    expected: &str,
    permit_trust: bool,
) -> Result<bool, FixtureError> {
    let mut view = host
        .subscribe()
        .await
        .map_err(|source| FixtureError::Host { source })?;
    let _ = next(&mut view).await?;
    host.frame(view.id, ClientFrame::Open { id })
        .await
        .map_err(|source| FixtureError::Host { source })?;
    let deadline = Instant::now() + Duration::from_secs(45);
    let mut answered_trust = false;
    let result = async {
        loop {
            host.frame(view.id, ClientFrame::SnapshotRequest { id })
                .await
                .map_err(|source| FixtureError::Host { source })?;
            let (bytes, offset) = loop {
                if let ServerFrame::Snapshot {
                    id: got,
                    bytes,
                    offset,
                    ..
                } = next(&mut view).await?
                {
                    if got == id {
                        break (bytes, offset);
                    }
                }
            };
            let mut vt =
                Terminal::from_checkpoint(&bytes).map_err(|_| FixtureError::Invariant {
                    stage: "decode real CLI display",
                })?;
            vt.sync();
            let text = (0..vt.size().rows)
                .filter_map(|y| vt.row(y))
                .flat_map(|row| row.cells.iter().map(|cell| cell.text.as_str()))
                .collect::<String>();
            host.frame(view.id, ClientFrame::Ack { id, offset })
                .await
                .map_err(|source| FixtureError::Host { source })?;
            if text.contains(expected) {
                return Ok(answered_trust);
            }
            let lower = text.to_lowercase();
            if permit_trust
                && !answered_trust
                && lower.contains("trust")
                && (lower.contains("folder") || lower.contains("directory"))
            {
                // Explicitly acknowledge only this empty, owned fixture folder.
                // No global trust, permission bypass or prompt replay is configured.
                host.frame(view.id, ClientFrame::TakeControl { id })
                    .await
                    .map_err(|source| FixtureError::Host { source })?;
                host.frame(
                    view.id,
                    ClientFrame::Input {
                        id,
                        seq: 1,
                        bytes: b"\r".to_vec(),
                    },
                )
                .await
                .map_err(|source| FixtureError::Host { source })?;
                answered_trust = true;
            }
            require(
                Instant::now() < deadline,
                "actual Codex TUI did not display its saved assistant marker",
            )?;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    .await;
    host.leave(view.id).await;
    result
}

async fn exercise(
    host: &Host,
    helper: &Helper,
    root: &Path,
    holder: &Path,
    known_holders: &mut BTreeSet<i32>,
) -> Result<serde_json::Value, FixtureError> {
    let before = sessions(host).await?;
    require(before.len() == 1, "expected one source agent")?;
    let source = &before[0];
    let mut observation = host.observe_agent_resume();
    let first = receipt(helper, 0).await?;
    require(
        first.requested_session_id == helper.prepared.session_id,
        "source CLI must request the exact seeded conversation",
    )?;
    let first_parent = owned_holder_parent(first.pid, root, holder)?;
    known_holders.insert(first_parent);
    let trusted_fixture =
        historical_screen(host, source.id, &helper.prepared.expected_history, true).await?;
    // Verify ownership immediately before signalling only this real CLI child.
    require(
        owned_holder_parent(first.pid, root, holder)? == first_parent,
        "CLI parent changed before the fixture kill",
    )?;
    if unsafe { libc::kill(first.pid, libc::SIGKILL) } != 0 {
        return Err(io(
            "kill only owned Codex child",
            std::io::Error::last_os_error(),
        ));
    }
    let launch = tokio::time::timeout(Duration::from_secs(45), async {
        loop {
            match observation.recv().await {
                Some(AgentResumeObservation::Launched(launch))
                    if launch.source_uuid == source.uuid =>
                {
                    break Ok(launch);
                }
                Some(AgentResumeObservation::Failed { error, .. }) => {
                    break Err(FixtureError::Resume { source: error });
                }
                Some(AgentResumeObservation::Lagged { .. }) => {
                    break Err(FixtureError::Invariant {
                        stage: "lost a recovery outcome",
                    });
                }
                Some(_) => {}
                None => {
                    break Err(FixtureError::Invariant {
                        stage: "recovery observer closed",
                    });
                }
            }
        }
    })
    .await
    .map_err(|source| FixtureError::Timeout {
        stage: "automatic provider launch",
        source,
    })??;
    require(
        !launch.existing
            && launch.source_uuid == source.uuid
            && launch.terminal_uuid != source.uuid
            && launch.terminal_id != source.id,
        "recovery must create one distinct process generation and retain the old terminal",
    )?;
    require(
        launch.provider == "codex" && launch.session_id == helper.prepared.session_id,
        "recovery launch must retain the structured provider ID",
    )?;
    let second = receipt(helper, 1).await?;
    require(
        second.pid != first.pid && second.requested_session_id == helper.prepared.session_id,
        "replacement CLI must have a new PID and request the same conversation",
    )?;
    let second_parent = owned_holder_parent(second.pid, root, holder)?;
    known_holders.insert(second_parent);
    require(
        second_parent != first_parent,
        "a fresh recovery generation needs a fresh owned holder",
    )?;
    historical_screen(
        host,
        launch.terminal_id,
        &helper.prepared.expected_history,
        false,
    )
    .await?;
    historical_screen(host, source.id, &helper.prepared.expected_history, false).await?;
    let after = sessions(host).await?;
    require(
        after.len() == 2
            && after
                .iter()
                .any(|s| s.uuid == source.uuid && matches!(s.state, SessionState::Exited { .. }))
            && after.iter().any(|s| {
                s.uuid == launch.terminal_uuid && matches!(s.state, SessionState::Running)
            }),
        "the old exited terminal and new live terminal must coexist",
    )?;
    require(
        after
            .iter()
            .find(|s| s.uuid == launch.terminal_uuid)
            .and_then(|s| s.attributes.get("terminal.resume.source"))
            == Some(&source.uuid),
        "the replacement terminal must retain its explicit predecessor link",
    )?;
    require(
        helper.receipts()?.len() == 2,
        "automatic recovery must not launch a third agent",
    )?;
    let counter: serde_json::Value = serde_json::from_slice(
        &fs::read(&helper.prepared.model_requests)
            .map_err(|e| io("read fixture model counter", e))?,
    )
    .map_err(|source| FixtureError::Json { source })?;
    require(
        counter["requests"].as_u64() == Some(1),
        "TUI restart must not replay a prompt or submit another model turn",
    )?;
    Ok(
        serde_json::json!({"actualCli":"Codex","model":"deterministic loopback Responses fixture","firstPid":first.pid,"recoveredPid":second.pid,"sameProviderConversationRequested":true,"realTuiHistoryRestored":true,"oldTerminalPreserved":true,"newTerminalUuid":true,"fixtureTrustAcknowledged":trusted_fixture,"originalPromptReplayed":false,"upstreamModelCalls":false}),
    )
}

async fn cleanup(
    host: &Host,
    root: &Path,
    holder: &Path,
    helper: &Helper,
    known_holders: &BTreeSet<i32>,
) -> Result<(), FixtureError> {
    // Stop live children first. Stop is explicit and cannot trigger auto-recovery.
    if let Ok(all) = sessions(host).await {
        let view = host
            .subscribe()
            .await
            .map_err(|source| FixtureError::Host { source })?;
        for session in all {
            let _ = host
                .frame(view.id, ClientFrame::Open { id: session.id })
                .await;
            let _ = host
                .frame(view.id, ClientFrame::TakeControl { id: session.id })
                .await;
            let _ = host
                .frame(view.id, ClientFrame::Stop { id: session.id })
                .await;
        }
        host.leave(view.id).await;
    }
    let shutdown = host.shutdown().await;
    // Host shutdown preserves holders. Attach through the same isolated Store
    // and stop every owned holder, including the old exited process's server.
    let reopen_error = match SessionStore::open_memory_only(root, holder.to_owned()) {
        Ok(store) => {
            for session in store.sessions() {
                let _ = store.stop(session.id).await;
            }
            None
        }
        Err(source) => Some(source),
    };
    // Bounded fallback only for still-running CLI children whose *current*
    // parent is verified against this exact private fixture state directory.
    for entry in helper.receipts()? {
        if owned_holder_parent(entry.pid, root, holder).is_ok() {
            let result = unsafe { libc::kill(entry.pid, libc::SIGKILL) };
            if result != 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH) {
                return Err(io("clean owned CLI", std::io::Error::last_os_error()));
            }
        }
    }
    for pid in known_holders {
        let output = ProcessCommand::new("/bin/ps")
            .args(["-p", &pid.to_string(), "-o", "args="])
            .output()
            .map_err(|e| io("inspect owned holder cleanup", e))?;
        let args = String::from_utf8_lossy(&output.stdout);
        if output.status.success()
            && args.contains(root.to_string_lossy().as_ref())
            && args.contains(holder.file_name().unwrap().to_string_lossy().as_ref())
        {
            if unsafe { libc::kill(*pid, libc::SIGKILL) } != 0
                && std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
            {
                return Err(io(
                    "clean still-running owned holder",
                    std::io::Error::last_os_error(),
                ));
            }
        }
    }
    shutdown.map_err(|source| FixtureError::Host { source })?;
    if let Some(source) = reopen_error {
        return Err(FixtureError::Session { source });
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires the explicitly configured installed Codex CLI; no upstream model calls"]
async fn actual_codex_host_recovers_exact_conversation_after_owned_child_sigkill() {
    let mut helper = Helper::start().expect("prepare isolated real Codex fixture");
    let directory = tempfile::Builder::new()
        .prefix("h2-real-codex-host-")
        .tempdir_in("/tmp")
        .unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let root = fs::canonicalize(directory.path()).unwrap();
    let holder = fs::canonicalize(std::env::var_os("HUMATA2_HOLDER").unwrap()).unwrap();
    let policy = ResumePolicy::Enabled {
        adapter: CODEX_RESUME_V1.into(),
        provider: "codex".into(),
        args: helper.prepared.resume_args.clone(),
    };
    let registry = Arc::new(ResumeRegistry::default());
    let initial = registry
        .plan(
            &policy,
            &ResumeIdentity {
                provider: "codex".into(),
                session_id: helper.prepared.session_id.clone(),
                working_directory: helper.prepared.cwd.to_string_lossy().into_owned(),
            },
        )
        .map_err(|source| FixtureError::Plan { source })
        .unwrap();
    let profile = Profile {
        name: "agent".into(),
        title: "Owned real Codex".into(),
        program: helper.prepared.wrapper.clone(),
        args: initial.args,
        cwd: helper.prepared.cwd.clone(),
        endpoint: Endpoint::Local,
        recovery: Some(RecoveryHint {
            provider: "codex".into(),
            session_id: helper.prepared.session_id.clone(),
            working_directory: initial.working_directory,
            state_paths: vec![],
        }),
        history: HistoryPolicy::default(),
    };
    let host = Host::start_with_agent_resume(
        Config {
            state: root.clone(),
            label: "real Codex recovery fixture".into(),
            start_profiles: vec!["agent".into()],
            default_history: HistoryPolicy::default(),
        },
        SessionStore::open_memory_only(&root, holder.clone()).unwrap(),
        vec![profile],
        AgentResumeConfig {
            registry,
            profiles: BTreeMap::from([("agent".into(), policy)]),
        },
    )
    .await
    .unwrap();
    let mut known_holders = BTreeSet::new();
    let result = AssertUnwindSafe(exercise(&host, &helper, &root, &holder, &mut known_holders))
        .catch_unwind()
        .await;
    let stopped = cleanup(&host, &root, &holder, &helper, &known_holders).await;
    let released = helper.close();
    match result {
        Ok(Ok(proof)) => {
            stopped.expect("stop all owned Codex/holder processes before releasing fixture state");
            released.expect("release isolated model fixture");
            if let Some(output) = std::env::var_os("TEST_UNDECLARED_OUTPUTS_DIR") {
                fs::write(
                    PathBuf::from(output).join("real-codex-host-resume-proof.json"),
                    serde_json::to_vec_pretty(&proof).unwrap(),
                )
                .unwrap();
            }
            println!("{proof}");
        }
        Ok(Err(error)) => panic!("{error}"),
        Err(panic) => resume_unwind(panic),
    }
}
