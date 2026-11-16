//! Actual disposable guest power loss; never touches the live SSH demo VM.
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
const REMOTE: &str = "/var/tmp/h2-vm-agent";
struct Fixture {
    directory: Option<tempfile::TempDir>,
    driver: PathBuf,
    home: PathBuf,
    config: Option<PathBuf>,
    host: Option<Host>,
    success: bool,
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::Builder::new()
            .prefix("h2-vm-")
            .tempdir_in("/tmp")
            .unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let home = directory.path().join("lima");
        Self {
            directory: Some(directory),
            driver: fs::canonicalize(std::env::var_os("TERMINAL_VM_DRIVER").unwrap()).unwrap(),
            home,
            config: None,
            host: None,
            success: false,
        }
    }
    fn root(&self) -> &std::path::Path {
        self.directory.as_ref().unwrap().path()
    }
    fn driver_command(&self, action: &str) -> ProcessCommand {
        let mut c = ProcessCommand::new("/bin/bash");
        c.arg(&self.driver).arg(action).env("LIMA_HOME", &self.home);
        c
    }
    fn drive(&self, action: &str, args: &[&std::ffi::OsStr]) -> Vec<u8> {
        eprintln!("Throwaway VM phase: {action}");
        let out = self.driver_command(action).args(args).output().unwrap();
        assert!(
            out.status.success(),
            "owned VM phase {action} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        out.stdout
    }
    fn provision(&mut self) {
        let holder = fs::canonicalize(
            std::env::var_os("TERMINAL_VM_LINUX_HOLDER")
                .expect("pass an already Bazel-built aarch64 Linux holder"),
        )
        .unwrap();
        let image=std::env::var_os("TERMINAL_VM_IMAGE").map(PathBuf::from).unwrap_or_else(||PathBuf::from(std::env::var_os("HOME").unwrap()).join("Library/Caches/lima/download/by-url-sha256/b4c920677ef3b13bc95061fd4e5c70d4bdc8f46220c6021d58aa054a927d5663/data"));
        self.drive("create", &[holder.as_os_str(), image.as_os_str()]);
        let info: serde_json::Value = serde_json::from_slice(&self.drive("metadata", &[])).unwrap();
        self.config = Some(info["sshConfigFile"].as_str().unwrap().into());
    }
    fn ssh(&self, script: &str) -> ProcessCommand {
        let mut c = ProcessCommand::new("ssh");
        c.arg("-F")
            .arg(self.config.as_ref().unwrap())
            .args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=5"])
            .arg("lima-powerloss")
            .arg(script);
        c
    }
    fn remote(&self, script: &str) -> Vec<u8> {
        let out = self.ssh(script).output().unwrap();
        assert!(
            out.status.success(),
            "owned guest command failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        out.stdout
    }
    fn boot(&self) -> String {
        String::from_utf8(self.remote("cat /proc/sys/kernel/random/boot_id"))
            .unwrap()
            .trim()
            .into()
    }
    fn rows(&self) -> Vec<serde_json::Value> {
        String::from_utf8(self.remote(&format!("cat '{REMOTE}/launches.jsonl'")))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
    fn state(&self) -> serde_json::Value {
        serde_json::from_slice(&self.remote(&format!("cat '{REMOTE}/conversation.json'"))).unwrap()
    }
    fn installation(&self) -> serde_json::Value {
        serde_json::from_slice(&self.remote(r#"/usr/bin/python3 - <<'PY'
from pathlib import Path
import hashlib,json
result={}
for name,path in [('holder','/var/tmp/h2-vm-holder'),('fixture_agent','/var/tmp/h2-vm-agent/agent')]:
    data=Path(path).read_bytes()
    result[name]={'bytes':len(data),'sha256':hashlib.sha256(data).hexdigest()}
print(json.dumps(result))
PY"#)).unwrap()
    }
    fn install_agent(&self) {
        let script=r#"#!/usr/bin/python3
import json,os,pathlib,sys
root=pathlib.Path('/var/tmp/h2-vm-agent')
session='SESSION_PLACEHOLDER'
boot=pathlib.Path('/proc/sys/kernel/random/boot_id').read_text().strip()
def sync_directory():
 fd=os.open(root,os.O_RDONLY|os.O_DIRECTORY)
 try: os.fsync(fd)
 finally: os.close(fd)
def save(state):
 path=root/'conversation.tmp'
 with path.open('w') as output:
  json.dump(state,output);output.flush();os.fsync(output.fileno())
 os.replace(path,root/'conversation.json');sync_directory()
if sys.argv[1:]==['boot','ORIGINAL_PROMPT_MUST_NOT_REPLAY']:
 os.chdir(root/'work')
 state={'session_id':session,'history':['durable before power loss'],'origin_boot':boot,'origin_pid':os.getpid()}
 phase='boot'
elif sys.argv[1:]==['resume',session]:
 state=json.loads((root/'conversation.json').read_text())
 assert state['session_id']==session
 assert state['history']==['durable before power loss']
 assert pathlib.Path.cwd()==root/'work'
 state['history'].append('resumed after new boot')
 phase='resume'
else: raise SystemExit(71)
save(state)
record={'session_id':session,'phase':phase,'pid':os.getpid(),'holder_pid':os.getppid(),'boot_id':boot,'cwd':os.getcwd()}
with (root/'launches.jsonl').open('a') as output:
 output.write(json.dumps(record)+'\n');output.flush();os.fsync(output.fileno())
sync_directory()
print('\x1b]7;file://localhost'+os.getcwd()+'\x07SESSION='+session+' BOOT='+boot,flush=True)
for line in sys.stdin: print('INPUT='+line.rstrip(),flush=True)
"#.replace("SESSION_PLACEHOLDER",SESSION);
        let mut child=self.ssh(&format!("umask 077; mkdir -p '{REMOTE}/work'; cat >'{REMOTE}/agent'; chmod 700 '{REMOTE}/agent'")).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(script.as_bytes())
            .unwrap();
        assert!(child.wait().unwrap().success());
        self.remote(
            r#"/usr/bin/python3 - <<'PY'
from pathlib import Path
import os
root=Path('/var/tmp/h2-vm-agent')
with (root/'agent').open('rb') as source:
    os.fsync(source.fileno())
for path in [root/'work',root,root.parent]:
    fd=os.open(path,os.O_RDONLY|os.O_DIRECTORY)
    try: os.fsync(fd)
    finally: os.close(fd)
PY"#,
        );
    }
    async fn start_host(&mut self) -> Host {
        let holder = fs::canonicalize(std::env::var_os("HUMATA2_HOLDER").unwrap()).unwrap();
        let state = self.root().join("host");
        let host = Host::start_with_agent_resume(
            Config {
                state: state.clone(),
                label: "Throwaway power-loss proof".into(),
                start_profiles: vec!["agent".into()],
                default_history: HistoryPolicy::default(),
            },
            SessionStore::open_memory_only(&state, holder).unwrap(),
            vec![Profile {
                name: "agent".into(),
                title: "Whole-VM strict fixture".into(),
                program: format!("{REMOTE}/agent").into(),
                args: vec!["boot".into(), "ORIGINAL_PROMPT_MUST_NOT_REPLAY".into()],
                cwd: "/var/tmp".into(),
                endpoint: Endpoint::Ssh {
                    config: self.config.clone().unwrap(),
                    destination: "lima-powerloss".into(),
                    holder: "/var/tmp/h2-vm-holder".into(),
                    root: format!("{REMOTE}/holders"),
                },
                recovery: Some(RecoveryHint {
                    provider: "codex".into(),
                    session_id: SESSION.into(),
                    working_directory: format!("{REMOTE}/work"),
                    state_paths: vec![format!("{REMOTE}/conversation.json")],
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
    async fn no_auto(&self, host: &Host, events: &mut AgentResumeObserver) {
        assert!(
            tokio::time::timeout(Duration::from_secs(2), events.recv())
                .await
                .is_err(),
            "machine outage must not select a replacement conversation"
        );
        assert_eq!(host.terminals().await.unwrap().len(), 1);
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(host) = self.host.take() {
            let runtime = tokio::runtime::Handle::current();
            let _ = std::thread::spawn(move || {
                runtime.block_on(async move {
                    let _ = tokio::time::timeout(Duration::from_secs(35), host.shutdown()).await;
                })
            })
            .join();
        }
        if !self.success {
            if self.home.join("powerloss/lima.yaml").is_file() {
                let _ = self
                    .driver_command("poweroff")
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
            let directory = self.directory.take().unwrap().keep();
            eprintln!(
                "Preserved failed, isolated VM proof at {}. Its fresh SSH keys are private; inspect only this LIMA_HOME, never the live fixture.",
                directory.display()
            );
        }
    }
}
async fn wait_state(
    handle: &TerminalHandle,
    predicate: impl Fn(&SessionState) -> bool,
) -> SessionInfo {
    let deadline = Instant::now() + Duration::from_secs(35);
    loop {
        let info = handle.info().await.unwrap();
        if predicate(&info.state) {
            return info;
        }
        assert!(
            Instant::now() < deadline,
            "terminal did not settle after machine power loss"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "fresh disposable macOS/VZ Lima VM, pinned cached Debian image and prebuilt Linux holder"]
async fn whole_guest_power_loss_preserves_disk_identity_and_requires_confirmed_new_boot() {
    assert_eq!(std::env::consts::OS, "macos");
    assert_eq!(std::env::consts::ARCH, "aarch64");
    let mut fixture = Fixture::new();
    fixture.provision();
    fixture.install_agent();
    let installed = fixture.installation();
    assert!(installed["holder"]["bytes"].as_u64().unwrap() > 0);
    assert!(installed["fixture_agent"]["bytes"].as_u64().unwrap() > 0);
    let old_boot = fixture.boot();
    let host = fixture.start_host().await;
    let original = host
        .terminal(&host.terminals().await.unwrap()[0].uuid)
        .await
        .unwrap();
    wait_state(&original, |s| matches!(s, SessionState::Running)).await;
    let before = fixture.rows();
    assert_eq!(before.len(), 1);
    assert_eq!(before[0]["boot_id"], old_boot);
    assert_eq!(before[0]["session_id"], SESSION);
    let durable = fixture.state();
    assert_eq!(
        durable["history"],
        serde_json::json!(["durable before power loss"])
    );
    let mut events = host.observe_agent_resume();
    fixture.drive("poweroff", &[]);
    wait_state(&original, |s| matches!(s, SessionState::Unavailable)).await;
    fixture.no_auto(&host, &mut events).await;
    fixture.drive("start", &[]);
    let new_boot = fixture.boot();
    assert_ne!(
        old_boot, new_boot,
        "proof must restart the whole guest kernel"
    );
    assert_eq!(
        fixture.state(),
        durable,
        "fsynced provider conversation/history must survive forced power loss"
    );
    assert_eq!(
        fixture.installation(),
        installed,
        "durably installed executables must survive the same power loss"
    );
    assert_eq!(fixture.rows().len(), 1);
    assert!(matches!(
        original.info().await.unwrap().state,
        SessionState::Unavailable
    ));
    fixture.no_auto(&host, &mut events).await;
    // A changed kernel boot ID confirms every process from the old boot is gone.
    let launch = host
        .resume_agent_after_confirmed_loss(original.uuid())
        .await
        .unwrap();
    let observed = tokio::time::timeout(Duration::from_secs(20), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(observed,AgentResumeObservation::Launched(ref event) if event==&launch));
    assert_ne!(launch.terminal_uuid, original.uuid());
    assert_eq!(launch.session_id, SESSION);
    assert!(!launch.existing);
    let resumed = host.terminal(&launch.terminal_uuid).await.unwrap();
    let info = wait_state(&resumed, |s| matches!(s, SessionState::Running)).await;
    let after = fixture.rows();
    assert_eq!(after.len(), 2);
    assert_eq!(after[1]["phase"], "resume");
    assert_eq!(after[1]["session_id"], SESSION);
    assert_eq!(after[1]["boot_id"], new_boot);
    // Numeric PIDs can recycle across reboot; (boot_id, PID) is process identity.
    assert_ne!(
        (&after[0]["boot_id"], &after[0]["pid"]),
        (&after[1]["boot_id"], &after[1]["pid"])
    );
    assert_eq!(info.cwd, format!("{REMOTE}/work"));
    assert_eq!(after[1]["cwd"], info.cwd);
    assert_eq!(
        resumed.recovery_hint().await.unwrap().unwrap().state_paths,
        vec![format!("{REMOTE}/conversation.json")]
    );
    let recovered = fixture.state();
    assert_eq!(
        recovered["history"],
        serde_json::json!(["durable before power loss", "resumed after new boot"])
    );
    eprintln!(
        "Whole guest proof: different boot IDs; original and resumed UUIDs differ; exact saved session/history/cwd retained."
    );
    host.shutdown().await.unwrap();
    fixture.host = None;
    fixture.drive("delete", &[]);
    let proof = serde_json::json!({
        "fault":"forced_guest_poweroff",
        "old_boot_id":old_boot,"new_boot_id":new_boot,
        "source_uuid":original.uuid(),"resumed_uuid":launch.terminal_uuid,
        "session_id":SESSION,"cwd":info.cwd,
        "old_process":before[0],"new_process":after[1],
        "installation":installed,
        "history_before":durable["history"],"history_after":recovered["history"],
        "no_automatic_launch_during_outage":true,
        "explicit_recovery_after_confirmed_new_boot":true,
        "owned_vm_deleted":true,
    });
    let output = PathBuf::from(
        std::env::var_os("TEST_UNDECLARED_OUTPUTS_DIR")
            .expect("run this manual proof through Bazel"),
    );
    fs::write(
        output.join("vm-powerloss-proof.json"),
        serde_json::to_vec_pretty(&proof).unwrap(),
    )
    .unwrap();
    eprintln!(
        "Power-loss proof: {}",
        serde_json::to_string(&proof).unwrap()
    );
    fixture.success = true;
}
