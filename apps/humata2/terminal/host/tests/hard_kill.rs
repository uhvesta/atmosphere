//! Kill only subprocesses created by this fixture, through the public gateway.
use futures_util::{SinkExt, StreamExt};
use ghostty_vt::{Grid, Terminal};
use std::{
    collections::BTreeSet,
    fs,
    os::unix::{fs::PermissionsExt, process::ExitStatusExt},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use terminal_wire::{ClientFrame, PROTOCOL_VERSION, ServerFrame, SessionInfo, SessionState};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};
type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;
const SCRIPT: &str = r#"stty -echo
mkdir -p "$1/work"; cd "$1/work" || exit 31
printf '%s' "$$" > "$1/child-pid"; printf '%s' "$PPID" > "$1/holder-pid"
printf x >> "$1/launches"; export KILL_KEEP=retained; count=0
printf '\033]7;file://localhost%s\007READY PID=%s KEEP=%s CWD=%s\r\n' "$PWD" "$$" "$KILL_KEEP" "$PWD"
while IFS= read -r line; do
 case "$line" in
 mark) count=$((count+1)); printf 'MARK=%s PID=%s KEEP=%s CWD=%s\r\n' "$count" "$$" "$KILL_KEEP" "$PWD" ;;
 arm) printf 'ARMED\r\n'; sleep 1; printf '\033[32mOFFLINE_OUTPUT\033[0m\r\n' ;;
 esac
done
"#;

#[derive(Clone)]
struct Remote {
    config: PathBuf,
    destination: String,
    root: String,
}
impl Remote {
    fn run(&self, script: &str) -> std::process::Output {
        let output = Command::new("ssh")
            .arg("-F")
            .arg(&self.config)
            .args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=5"])
            .arg(&self.destination)
            .arg(script)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "owned remote fixture command failed"
        );
        output
    }
    fn read(&self, file: &str) -> String {
        String::from_utf8(self.run(&format!("cat '{}/{file}'", self.root)).stdout).unwrap()
    }
    fn kill(&self, file: &str) {
        // The command line must still identify this unique fixture before kill.
        self.run(&format!(
            "p=$(cat '{root}/{file}'); case $(tr '\\000' ' ' </proc/$p/cmdline) in *'{root}'*) kill -KILL $p;; *) exit 42;; esac",
            root = self.root
        ));
    }
}
struct Fixture {
    directory: tempfile::TempDir,
    process: Option<Child>,
    address: std::net::SocketAddr,
    remote: Option<Remote>,
    sequence: u64,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(mut child) = self.process.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(remote) = &self.remote {
            // Cleanup is restricted to generated /var/tmp fixture paths and
            // processes whose argv still contains that exact path.
            let _ = Command::new("ssh").arg("-F").arg(&remote.config)
                .args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=5"])
                .arg(&remote.destination).arg(format!(
                    "for f in child-pid holder-pid; do p=$(cat '{r}/'$f 2>/dev/null); if [ -n \"$p\" ] && [ -r /proc/$p/cmdline ]; then case $(tr '\\000' ' ' </proc/$p/cmdline) in *'{r}'*) kill -KILL $p 2>/dev/null;; esac; fi; done; rm -rf '{r}'",r=remote.root))
                .stdout(Stdio::null()).stderr(Stdio::null()).status();
        } else {
            for name in ["child-pid", "holder-pid"] {
                if let Ok(value) = fs::read_to_string(self.directory.path().join(name)) {
                    if let Ok(pid) = value.parse() {
                        kill_owned_local(pid, self.directory.path(), false);
                    }
                }
            }
        }
    }
}
fn kill_owned_local(pid: u32, marker: &Path, required: bool) {
    let output = Command::new("ps")
        .args(["-ww", "-p", &pid.to_string(), "-o", "command="])
        .output()
        .unwrap();
    let matches = output.status.success()
        && String::from_utf8_lossy(&output.stdout).contains(&marker.to_string_lossy().to_string());
    if required {
        assert!(matches, "PID no longer identifies the owned fixture");
    }
    if matches {
        // SAFETY: ownership was verified against the unique fixture argv.
        let result = unsafe { libc::kill(pid as i32, libc::SIGKILL) };
        if required {
            assert_eq!(result, 0);
        }
    }
}
impl Fixture {
    async fn new(ssh: bool) -> (Self, Socket, SessionInfo) {
        let directory = tempfile::Builder::new()
            .prefix("h2-kill-")
            .tempdir_in("/tmp")
            .unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let assets = directory.path().join("assets");
        fs::create_dir(&assets).unwrap();
        fs::write(assets.join("index.html"), "owned failure fixture").unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let remote = if ssh {
            let trusted = fs::canonicalize(
                std::env::var_os("TERMINAL_LAB_SSH_CONFIG").expect("set dedicated Lima SSH config"),
            )
            .unwrap();
            let config = directory.path().join("fixture.ssh");
            fs::write(&config, format!("Include \"{}\"\n", trusted.display())).unwrap();
            fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
            let remote = Remote {
                config,
                destination: std::env::var("TERMINAL_LAB_SSH_HOST")
                    .unwrap_or("lima-h2-terminal-c1".into()),
                root: format!(
                    "/var/tmp/{}",
                    directory.path().file_name().unwrap().to_string_lossy()
                ),
            };
            assert!(remote.root.starts_with("/var/tmp/h2-kill-"));
            remote.run(&format!("umask 077; mkdir -p '{}/work'", remote.root));
            Some(remote)
        } else {
            None
        };
        let root = remote
            .as_ref()
            .map(|r| r.root.clone())
            .unwrap_or_else(|| directory.path().display().to_string());
        let endpoint = remote.as_ref().map(|r|serde_json::json!({"Ssh":{"config":r.config,"destination":r.destination,"holder":std::env::var("TERMINAL_LAB_SSH_HOLDER").unwrap_or("/tmp/h2-terminal-holder".into()),"root":format!("{}/holders",r.root)}})).unwrap_or_else(||serde_json::json!("Local"));
        let profiles = serde_json::json!([{"name":"owned","title":"Owned kill fixture","program":"/bin/sh","args":["-c",SCRIPT,"kill-fixture",root],"cwd":if ssh {"/tmp".to_string()} else {directory.path().display().to_string()},"endpoint":endpoint,"history":{"mode":"recent","max_lines":10000,"max_bytes":4194304,"interval_seconds":1},"recovery":{"provider":"kill-fixture","session_id":"explicit-conversation-1","working_directory":format!("{root}/work"),"state_paths":[]}}]);
        fs::write(
            directory.path().join("profiles.json"),
            serde_json::to_vec(&profiles).unwrap(),
        )
        .unwrap();
        let mut fixture = Self {
            directory,
            process: None,
            address,
            remote,
            sequence: 0,
        };
        fixture.start();
        let (mut socket, info) = fixture.connect().await;
        visible(&mut socket, "READY PID=").await;
        (fixture, socket, info)
    }
    fn state(&self) -> PathBuf {
        self.directory.path().join("state")
    }
    fn start(&mut self) {
        let host = fs::canonicalize(std::env::var_os("TERMINAL_HOST_BINARY").unwrap()).unwrap();
        let holder = fs::canonicalize(std::env::var_os("HUMATA2_HOLDER").unwrap()).unwrap();
        let log = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.directory.path().join("host.log"))
            .unwrap();
        self.process = Some(
            Command::new(host)
                .arg("--state")
                .arg(self.state())
                .arg("--holder")
                .arg(holder)
                .arg("--static-dir")
                .arg(self.directory.path().join("assets"))
                .arg("--listen")
                .arg(self.address.to_string())
                .arg("--profiles")
                .arg(self.directory.path().join("profiles.json"))
                .args(["--start", "owned", "--checkpoint-seconds", "1"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(log)
                .spawn()
                .unwrap(),
        );
        self.sequence = 0;
    }
    fn kill_host(&mut self) {
        let mut child = self.process.take().unwrap();
        child.kill().unwrap();
        assert_eq!(child.wait().unwrap().signal(), Some(libc::SIGKILL));
    }
    fn read(&self, name: &str) -> String {
        self.remote
            .as_ref()
            .map(|r| r.read(name))
            .unwrap_or_else(|| fs::read_to_string(self.directory.path().join(name)).unwrap())
    }
    fn kill_program(&self, name: &str) {
        if let Some(remote) = &self.remote {
            remote.kill(name);
        } else {
            kill_owned_local(
                self.read(name).parse().unwrap(),
                self.directory.path(),
                true,
            );
        }
    }
    async fn connect(&mut self) -> (Socket, SessionInfo) {
        let deadline = Instant::now() + Duration::from_secs(25);
        let mut socket = loop {
            if let Ok((socket, _)) = connect_async(format!("ws://{}/ws", self.address)).await {
                break socket;
            }
            assert!(Instant::now() < deadline, "owned host never became ready");
            assert!(
                self.process.as_mut().unwrap().try_wait().unwrap().is_none(),
                "owned host exited during startup"
            );
            tokio::time::sleep(Duration::from_millis(40)).await;
        };
        let token = fs::read_to_string(self.state().join("browser.token")).unwrap();
        send(
            &mut socket,
            ClientFrame::Authenticate {
                protocol: PROTOCOL_VERSION,
                token,
            },
        )
        .await;
        let info = loop {
            if let ServerFrame::Sessions { sessions, .. } = receive(&mut socket).await {
                assert_eq!(sessions.len(), 1, "startup duplicated an existing terminal");
                break sessions.into_iter().next().unwrap();
            }
        };
        send(&mut socket, ClientFrame::Open { id: info.id }).await;
        (socket, info)
    }
    async fn input(&mut self, socket: &mut Socket, text: &str) {
        self.sequence += 1;
        send(
            socket,
            ClientFrame::Input {
                id: 1,
                seq: self.sequence,
                bytes: text.as_bytes().to_vec(),
            },
        )
        .await;
        loop {
            if let ServerFrame::InputWritten { seq, .. } = receive(socket).await {
                if seq == self.sequence {
                    break;
                }
            }
        }
    }
    async fn checkpoint(&self, minimum: u64) {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            if let Ok(bytes) = fs::read(self.state().join("recovery/s1.checkpoint")) {
                if bytes.len() > 8 {
                    let limits = terminal_codec::Limits {
                        max_decoded_bytes: terminal_wire::MAX_RAW_CHECKPOINT_BYTES + 4100,
                        max_encoded_bytes: terminal_wire::MAX_SNAPSHOT_BYTES,
                    };
                    if let Ok(envelope) = terminal_codec::Envelope::from_bytes(&bytes[8..], limits)
                    {
                        if let Ok(raw) =
                            terminal_codec::Registry::default().decode(&envelope, limits)
                        {
                            let n = u32::from_le_bytes(raw[..4].try_into().unwrap()) as usize;
                            let header: serde_json::Value =
                                serde_json::from_slice(&raw[4..4 + n]).unwrap();
                            if header["offset"].as_u64().unwrap() >= minimum {
                                return;
                            }
                        }
                    }
                }
            }
            assert!(
                Instant::now() < deadline,
                "host never durably checkpointed observed output"
            );
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
    }
    fn kill_tunnel(&self) {
        let host = self.process.as_ref().unwrap().id();
        // Enumerate only numeric ownership first; inspect argv only for this host's descendants.
        let output = Command::new("ps")
            .args(["-A", "-o", "pid=,ppid="])
            .output()
            .unwrap();
        let pairs: Vec<(u32, u32)> = String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .filter_map(|line| {
                let mut v = line.split_whitespace();
                Some((v.next()?.parse().ok()?, v.next()?.parse().ok()?))
            })
            .collect();
        let mut owned = BTreeSet::from([host]);
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
        let marker = self.remote.as_ref().unwrap().config.display().to_string();
        let mut killed = 0;
        for pid in owned.into_iter().filter(|pid| *pid != host) {
            let output = Command::new("ps")
                .args(["-ww", "-p", &pid.to_string(), "-o", "command="])
                .output()
                .unwrap();
            let text = String::from_utf8_lossy(&output.stdout);
            if text.contains(&marker) && text.contains("-L") && text.contains("exec /bin/cat") {
                assert_eq!(unsafe { libc::kill(pid as i32, libc::SIGKILL) }, 0);
                killed += 1;
            }
        }
        assert_eq!(
            killed, 1,
            "expected exactly one owned SSH attachment tunnel"
        );
    }
}
async fn send(socket: &mut Socket, frame: ClientFrame) {
    socket
        .send(Message::Text(serde_json::to_string(&frame).unwrap().into()))
        .await
        .unwrap();
}
async fn receive(socket: &mut Socket) -> ServerFrame {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(25), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let Message::Text(text) = message {
            let frame: ServerFrame = serde_json::from_str(&text).unwrap();
            let offset = match &frame {
                ServerFrame::Output { id, offset, bytes } => {
                    Some((*id, *offset + bytes.len() as u64))
                }
                ServerFrame::Snapshot { id, offset, .. } => Some((*id, *offset)),
                _ => None,
            };
            if let Some((id, offset)) = offset {
                send(socket, ClientFrame::Ack { id, offset }).await;
            }
            return frame;
        }
    }
}
async fn visible(socket: &mut Socket, needle: &str) -> (u64, bool) {
    let deadline = Instant::now() + Duration::from_secs(25);
    loop {
        send(socket, ClientFrame::SnapshotRequest { id: 1 }).await;
        loop {
            if let ServerFrame::Snapshot {
                bytes,
                offset,
                live,
                ..
            } = receive(socket).await
            {
                let mut terminal = Terminal::from_checkpoint(&bytes).unwrap();
                terminal.sync();
                let text = (0..terminal.size().rows)
                    .filter_map(|y| terminal.row(y))
                    .flat_map(|row| row.cells.iter().map(|c| c.text.as_str()))
                    .collect::<String>();
                if text.contains(needle) {
                    return (offset, live);
                }
                break;
            }
        }
        assert!(
            Instant::now() < deadline,
            "expected terminal state was not visible"
        );
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
}
async fn state(socket: &mut Socket, predicate: impl Fn(&SessionState) -> bool) -> SessionInfo {
    let deadline = Instant::now() + Duration::from_secs(25);
    loop {
        send(socket, ClientFrame::List).await;
        loop {
            if let ServerFrame::Sessions { sessions, .. } = receive(socket).await {
                assert_eq!(sessions.len(), 1);
                let info = sessions.into_iter().next().unwrap();
                if predicate(&info.state) {
                    return info;
                }
                break;
            }
        }
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
}
async fn host_kill(ssh: bool) {
    let (mut fixture, mut socket, original) = Fixture::new(ssh).await;
    let pid = fixture.read("child-pid");
    fixture.input(&mut socket, "mark\n").await;
    let (offset, _) = visible(&mut socket, "MARK=1").await;
    fixture.checkpoint(offset).await;
    fixture.input(&mut socket, "arm\n").await;
    visible(&mut socket, "ARMED").await;
    fixture.kill_host();
    drop(socket);
    tokio::time::sleep(Duration::from_millis(1200)).await;
    fixture.start();
    let (mut socket, restored) = fixture.connect().await;
    assert_eq!(restored.uuid, original.uuid);
    assert_eq!(fixture.read("child-pid"), pid);
    assert_eq!(fixture.read("launches"), "x");
    assert!(visible(&mut socket, "OFFLINE_OUTPUT").await.1);
    fixture.input(&mut socket, "mark\n").await;
    visible(&mut socket, "MARK=2").await;
    visible(&mut socket, "KEEP=retained").await;
    assert!(restored.recovery_available);
}
async fn holder_kill(ssh: bool) {
    let (mut fixture, mut socket, original) = Fixture::new(ssh).await;
    fixture.input(&mut socket, "mark\n").await;
    let (offset, _) = visible(&mut socket, "MARK=1").await;
    fixture.checkpoint(offset).await;
    fixture.kill_program("holder-pid");
    state(&mut socket, |s| matches!(s, SessionState::Unavailable)).await;
    assert!(!visible(&mut socket, "MARK=1").await.1);
    fixture.kill_host();
    drop(socket);
    fixture.start();
    let (mut socket, restored) = fixture.connect().await;
    assert_eq!(original.uuid, restored.uuid);
    assert!(matches!(restored.state, SessionState::Unavailable));
    assert!(!visible(&mut socket, "MARK=1").await.1);
    assert_eq!(fixture.read("launches"), "x");
}
async fn child_kill(ssh: bool) {
    let (mut fixture, mut socket, original) = Fixture::new(ssh).await;
    fixture.kill_program("child-pid");
    let exited = state(&mut socket, |s| matches!(s, SessionState::Exited { .. })).await;
    fixture.kill_host();
    drop(socket);
    fixture.start();
    let (mut socket, restored) = fixture.connect().await;
    assert_eq!(original.uuid, restored.uuid);
    assert!(matches!(
        state(&mut socket, |s| matches!(s, SessionState::Exited { .. }))
            .await
            .state,
        SessionState::Exited { .. }
    ));
    assert_eq!(fixture.read("launches"), "x");
    assert!(exited.recovery_available);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn local_host_sigkill_reattaches_same_process() {
    host_kill(false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn local_holder_sigkill_retains_read_only_checkpoint() {
    holder_kill(false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn local_child_sigkill_reports_exit_without_respawn() {
    child_kill(false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "dedicated Lima OpenSSH fixture"]
async fn ssh_host_sigkill_reattaches_same_process() {
    host_kill(true).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "dedicated Lima OpenSSH fixture"]
async fn ssh_holder_sigkill_retains_read_only_checkpoint() {
    holder_kill(true).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "dedicated Lima OpenSSH fixture"]
async fn ssh_child_sigkill_reports_exit_without_respawn() {
    child_kill(true).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "dedicated Lima OpenSSH fixture"]
async fn ssh_tunnel_sigkill_reattaches_same_remote_process() {
    let (mut fixture, mut socket, original) = Fixture::new(true).await;
    let pid = fixture.read("child-pid");
    fixture.kill_tunnel();
    loop {
        if let ServerFrame::Snapshot { live: false, .. } = receive(&mut socket).await {
            break;
        }
    }
    let restored = state(&mut socket, |s| matches!(s, SessionState::Running)).await;
    assert_eq!(restored.uuid, original.uuid);
    fixture.input(&mut socket, "mark\n").await;
    visible(&mut socket, "MARK=1").await;
    assert_eq!(fixture.read("child-pid"), pid);
    assert_eq!(fixture.read("launches"), "x");
}
