//! Actual OpenSSH processes with a strict owned CLI fixture, not remote Codex.
use super::*;
use std::{
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    process::{Command as ProcessCommand, Stdio},
    sync::Arc,
};
use terminal_provider::{CODEX_RESUME_V1, ResumePolicy, ResumeRegistry};
const SESSION: &str = "019a83ae-6368-72f5-90bc-5799b82886a1";
#[derive(Debug)]
struct LaunchRow {
    pid: u32,
    holder: u32,
    identity: String,
    cwd: String,
    os: String,
}
struct Fixture {
    directory: tempfile::TempDir,
    config: PathBuf,
    destination: String,
    remote: String,
    host: Option<Host>,
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::Builder::new()
            .prefix("h2-arssh-")
            .tempdir_in("/tmp")
            .unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let trusted = fs::canonicalize(
            std::env::var_os("TERMINAL_LAB_SSH_CONFIG").expect("set dedicated Lima OpenSSH config"),
        )
        .unwrap();
        let config = directory.path().join("fixture.ssh");
        fs::write(&config, format!("Include \"{}\"\n", trusted.display())).unwrap();
        fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
        let fixture = Self {
            remote: format!(
                "/var/tmp/{}",
                directory.path().file_name().unwrap().to_string_lossy()
            ),
            directory,
            config,
            destination: std::env::var("TERMINAL_LAB_SSH_HOST")
                .unwrap_or("lima-h2-terminal-c1".into()),
            host: None,
        };
        assert!(fixture.remote.starts_with("/var/tmp/h2-arssh-"));
        let script = format!(
            r#"#!/bin/sh
root='{root}'
case "$1" in
 boot) [ "$#" = 2 ] && [ "$2" = ORIGINAL_PROMPT_MUST_NOT_REPLAY ] || exit 71
       identity=boot; cd "$root/resume-cwd" || exit 72 ;;
 resume) [ "$#" = 2 ] && [ "$2" = '{session}' ] || exit 73; identity="$2" ;;
 *) exit 74 ;;
esac
stty -echo
printf '%s|%s|%s|%s|%s\n' "$$" "$PPID" "$identity" "$PWD" "$(uname -s)" >> "$root/launches"
printf '\033]7;file://localhost%s\007AGENT_SESSION=%s PID=%s CWD=%s\r\n' "$PWD" "$identity" "$$" "$PWD"
while IFS= read -r line; do printf 'FIXTURE_INPUT=%s\r\n' "$line"; done
"#,
            root = fixture.remote,
            session = SESSION
        );
        let mut child=fixture.command(&format!("umask 077; mkdir -p '{r}/launch-cwd' '{r}/resume-cwd'; cat >'{r}/agent'; chmod 700 '{r}/agent'",r=fixture.remote)).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(script.as_bytes())
            .unwrap();
        assert!(child.wait().unwrap().success());
        fixture
    }
    fn command(&self, script: &str) -> ProcessCommand {
        let mut c = ProcessCommand::new("ssh");
        c.arg("-F")
            .arg(&self.config)
            .args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=5"])
            .arg(&self.destination)
            .arg(script);
        c
    }
    fn run(&self, script: &str) -> Vec<u8> {
        let out = self.command(script).output().unwrap();
        assert!(
            out.status.success(),
            "owned remote fixture operation failed"
        );
        out.stdout
    }
    fn endpoint(&self) -> Endpoint {
        Endpoint::Ssh {
            config: self.config.clone(),
            destination: self.destination.clone(),
            holder: std::env::var_os("TERMINAL_LAB_SSH_HOLDER")
                .map(PathBuf::from)
                .unwrap_or("/tmp/h2-terminal-holder".into()),
            root: format!("{}/holders", self.remote),
        }
    }
    async fn start(&mut self) -> Host {
        let holder = fs::canonicalize(std::env::var_os("HUMATA2_HOLDER").unwrap()).unwrap();
        let host = Host::start_with_agent_resume(
            Config {
                state: self.directory.path().into(),
                label: "SSH recovery fixture".into(),
                start_profiles: vec!["agent".into()],
                default_history: HistoryPolicy::default(),
            },
            SessionStore::open_memory_only(self.directory.path(), holder).unwrap(),
            vec![Profile {
                name: "agent".into(),
                title: "SSH strict CLI fixture".into(),
                program: format!("{}/agent", self.remote).into(),
                args: vec!["boot".into(), "ORIGINAL_PROMPT_MUST_NOT_REPLAY".into()],
                cwd: format!("{}/launch-cwd", self.remote).into(),
                endpoint: self.endpoint(),
                recovery: Some(RecoveryHint {
                    provider: "codex".into(),
                    session_id: SESSION.into(),
                    working_directory: format!("{}/resume-cwd", self.remote),
                    state_paths: vec![],
                }),
                history: HistoryPolicy::default(),
            }],
            AgentResumeConfig {
                registry: Arc::new(ResumeRegistry::default()),
                profiles: BTreeMap::from([(
                    "agent".into(),
                    ResumePolicy::Enabled {
                        adapter: CODEX_RESUME_V1.into(),
                        provider: "codex".into(),
                        args: vec![],
                    },
                )]),
            },
        )
        .await
        .unwrap();
        self.host = Some(host.clone());
        host
    }
    fn rows(&self) -> Vec<LaunchRow> {
        String::from_utf8(self.run(&format!(
            "cat '{}/launches' 2>/dev/null || true",
            self.remote
        )))
        .unwrap()
        .lines()
        .map(|line| {
            let f = line.split('|').collect::<Vec<_>>();
            assert_eq!(f.len(), 5);
            LaunchRow {
                pid: f[0].parse().unwrap(),
                holder: f[1].parse().unwrap(),
                identity: f[2].into(),
                cwd: f[3].into(),
                os: f[4].into(),
            }
        })
        .collect()
    }
    async fn launched(&self, count: usize) -> Vec<LaunchRow> {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let rows = self.rows();
            if rows.len() >= count {
                return rows;
            }
            assert!(Instant::now() < deadline, "remote fixture did not start");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
    fn kill_owned_remote(&self, pid: u32) {
        self.run(&format!("case $(tr '\\000' ' ' </proc/{pid}/cmdline) in *'{root}'*) kill -KILL {pid};; *) exit 81;; esac",root=self.remote));
    }
    fn confirm_child_dead(&self, pid: u32) {
        // A zombie cannot execute. Kill only a live process retaining this fixture's argv.
        self.run(&format!(
            r#"if [ -d /proc/{pid} ]; then
 case $(tr '\000' ' ' </proc/{pid}/cmdline) in
 *'{root}'*) kill -KILL {pid} ;;
 '') [ "$(awk '{{print $3}}' /proc/{pid}/stat)" = Z ] || exit 82 ;;
 *) exit 83 ;;
 esac
fi
for n in 1 2 3 4 5 6 7 8 9 10; do
 [ ! -d /proc/{pid} ] && exit 0
 [ "$(awk '{{print $3}}' /proc/{pid}/stat)" = Z ] && exit 0
 sleep 0.05
done
exit 84
"#,
            root = self.remote
        ));
    }
    fn kill_tunnel(&self) {
        let out = ProcessCommand::new("ps")
            .args(["-axo", "pid=,ppid="])
            .output()
            .unwrap();
        let pairs = String::from_utf8(out.stdout)
            .unwrap()
            .lines()
            .filter_map(|line| {
                let mut f = line.split_whitespace();
                Some((
                    f.next()?.parse::<u32>().ok()?,
                    f.next()?.parse::<u32>().ok()?,
                ))
            })
            .collect::<Vec<_>>();
        let mut owned = BTreeSet::from([std::process::id()]);
        loop {
            let before = owned.len();
            for (pid, parent) in &pairs {
                if owned.contains(parent) {
                    owned.insert(*pid);
                }
            }
            if owned.len() == before {
                break;
            }
        }
        let mut killed = 0;
        for pid in owned.into_iter().filter(|pid| *pid != std::process::id()) {
            let out = ProcessCommand::new("ps")
                .args(["-ww", "-p", &pid.to_string(), "-o", "command="])
                .output()
                .unwrap();
            let text = String::from_utf8_lossy(&out.stdout);
            if text.contains(&self.config.display().to_string())
                && text.contains("-L")
                && text.contains("exec /bin/cat")
            {
                // SAFETY: descendant ownership and this fixture's SSH config were verified.
                assert_eq!(unsafe { libc::kill(pid as i32, libc::SIGKILL) }, 0);
                killed += 1;
            }
        }
        assert_eq!(killed, 1);
    }
    async fn no_resume(&self, observer: &mut AgentResumeObserver, count: usize) {
        assert!(
            tokio::time::timeout(Duration::from_secs(2), observer.recv())
                .await
                .is_err(),
            "transport uncertainty must not relaunch an enabled agent"
        );
        assert_eq!(self.rows().len(), count);
    }
    async fn verify_replacement(
        &self,
        host: &Host,
        original: &TerminalHandle,
        event: &AgentResumeLaunch,
    ) {
        assert_eq!(event.source_uuid, original.uuid());
        assert_ne!(event.terminal_uuid, original.uuid());
        assert_eq!(event.session_id, SESSION);
        assert_eq!(event.provider, "codex");
        assert!(!event.existing);
        let rows = self.launched(2).await;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].identity, "boot");
        assert_eq!(rows[1].identity, SESSION);
        assert_ne!(rows[0].pid, rows[1].pid);
        assert_ne!(rows[0].holder, rows[1].holder);
        assert_eq!(rows[1].cwd, format!("{}/resume-cwd", self.remote));
        assert_eq!(rows[1].os, "Linux");
        let resumed = host.terminal(&event.terminal_uuid).await.unwrap();
        let info = wait_state(&resumed, |s| matches!(s, SessionState::Running)).await;
        assert_eq!(info.cwd, rows[1].cwd);
        assert_eq!(
            info.attributes
                .get("terminal.resume.source")
                .map(String::as_str),
            Some(original.uuid())
        );
        let saved: serde_json::Value = serde_json::from_slice(
            &fs::read(self.directory.path().join("workspace.json")).unwrap(),
        )
        .unwrap();
        let records = saved["sessions"].as_array().unwrap();
        let before = records
            .iter()
            .find(|r| r["uuid"] == original.uuid())
            .unwrap();
        let after = records
            .iter()
            .find(|r| r["uuid"] == event.terminal_uuid)
            .unwrap();
        assert_eq!(before["spec"]["endpoint"], after["spec"]["endpoint"]);
        assert_eq!(after["recovery"]["session_id"], SESSION);
        assert_eq!(after["recovery"]["working_directory"], rows[1].cwd);
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(host) = self.host.take() {
            // Await retirement before cleanup so a killed fixture cannot trigger recovery.
            let runtime = tokio::runtime::Handle::current();
            let _ = std::thread::spawn(move || {
                runtime.block_on(async move {
                    let _ = tokio::time::timeout(Duration::from_secs(35), host.shutdown()).await;
                })
            })
            .join();
        }
        let _=self.command(&format!(r#"for f in '{r}'/holders/*/*.pid; do
 [ -f "$f" ] || continue
 p=$(cat "$f"); case $(tr '\000' ' ' </proc/$p/cmdline 2>/dev/null) in *'{r}'*) kill -KILL $p 2>/dev/null;; esac
done
if [ -f '{r}/launches' ]; then
 while IFS='|' read -r p rest; do
  case $(tr '\000' ' ' </proc/$p/cmdline 2>/dev/null) in *'{r}'*) kill -KILL $p 2>/dev/null;; esac
 done <'{r}/launches'
fi
rm -rf '{r}'
"#,r=self.remote)).stdout(Stdio::null()).stderr(Stdio::null()).status();
    }
}
async fn wait_state(
    handle: &TerminalHandle,
    predicate: impl Fn(&SessionState) -> bool,
) -> SessionInfo {
    let deadline = Instant::now() + Duration::from_secs(25);
    loop {
        let info = handle.info().await.unwrap();
        if predicate(&info.state) {
            return info;
        }
        assert!(Instant::now() < deadline, "terminal state did not settle");
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
}
async fn launch_event(observer: &mut AgentResumeObserver) -> AgentResumeLaunch {
    match tokio::time::timeout(Duration::from_secs(20), observer.recv())
        .await
        .unwrap()
        .unwrap()
    {
        AgentResumeObservation::Launched(event) => event,
        other => panic!("owned recovery failed: {other:?}"),
    }
}
async fn frame(view: &mut Subscription) -> ServerFrame {
    tokio::time::timeout(Duration::from_secs(25), view.receiver.recv())
        .await
        .unwrap()
        .unwrap()
        .frame
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "dedicated Lima OpenSSH; strict generic fixture, not installed remote Codex"]
async fn ssh_confirmed_child_sigkill_automatically_resumes_exact_saved_session() {
    let mut fixture = Fixture::new();
    let host = fixture.start().await;
    let rows = fixture.launched(1).await;
    let original = host
        .terminal(&host.terminals().await.unwrap()[0].uuid)
        .await
        .unwrap();
    wait_state(&original, |s| matches!(s, SessionState::Running)).await;
    let mut observer = host.observe_agent_resume();
    fixture.kill_owned_remote(rows[0].pid);
    let event = launch_event(&mut observer).await;
    fixture.verify_replacement(&host, &original, &event).await;
    assert!(matches!(
        original.info().await.unwrap().state,
        SessionState::Exited { .. }
    ));
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "dedicated Lima OpenSSH; strict generic fixture, not installed remote Codex"]
async fn ssh_enabled_resume_does_not_guess_transport_or_holder_death() {
    let mut fixture = Fixture::new();
    let host = fixture.start().await;
    let rows = fixture.launched(1).await;
    let original = host
        .terminal(&host.terminals().await.unwrap()[0].uuid)
        .await
        .unwrap();
    wait_state(&original, |s| matches!(s, SessionState::Running)).await;
    let mut observer = host.observe_agent_resume();
    let mut view = host.subscribe().await.unwrap();
    let _ = frame(&mut view).await;
    host.frame(
        view.id,
        ClientFrame::Open {
            id: original.route(),
        },
    )
    .await
    .unwrap();
    loop {
        if matches!(
            frame(&mut view).await,
            ServerFrame::Snapshot { live: true, .. }
        ) {
            break;
        }
    }
    fixture.kill_tunnel();
    loop {
        if matches!(
            frame(&mut view).await,
            ServerFrame::Snapshot { live: false, .. }
        ) {
            break;
        }
    }
    wait_state(&original, |s| matches!(s, SessionState::Running)).await;
    fixture.no_resume(&mut observer, 1).await;
    loop {
        if matches!(
            frame(&mut view).await,
            ServerFrame::Snapshot { live: true, .. }
        ) {
            break;
        }
    }
    assert_eq!(fixture.rows()[0].pid, rows[0].pid);
    fixture.kill_owned_remote(rows[0].holder);
    wait_state(&original, |s| matches!(s, SessionState::Unavailable)).await;
    fixture.no_resume(&mut observer, 1).await;
    host.frame(
        view.id,
        ClientFrame::SnapshotRequest {
            id: original.route(),
        },
    )
    .await
    .unwrap();
    loop {
        if let ServerFrame::Snapshot { live, .. } = frame(&mut view).await {
            assert!(!live);
            break;
        }
    }
    fixture.confirm_child_dead(rows[0].pid);
    let resumed = host
        .resume_agent_after_confirmed_loss(original.uuid())
        .await
        .unwrap();
    let observed = launch_event(&mut observer).await;
    assert_eq!(resumed, observed);
    fixture.verify_replacement(&host, &original, &resumed).await;
}
