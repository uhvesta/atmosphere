//! End-to-end test for `humatad2 serve --daemonize`:
//! - Parent exits 0 quickly and closes stdout/stderr (no hanging SSH pipe).
//! - Daemon runs detached in its own session, writes `daemon.lock`, and
//!   answers gRPC calls on its Unix socket.
//! - Stopping/killing the daemon and starting `humatad2 serve --daemonize`
//!   again brings it back cleanly, preserves prior SQLite state, and writes
//!   a restart reconciliation report.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use humata2_grpc::humata::v1 as pb;
use rustix::process::{Pid, Signal, getsid, kill_process, test_kill_process};

async fn connect(socket: &Path) -> Result<tonic::transport::Channel, tonic::transport::Error> {
    let path = socket.to_owned();
    tonic::transport::Endpoint::try_from("http://[::]:50051")
        .unwrap()
        .connect_with_connector(tower::service_fn(move |_: tonic::transport::Uri| {
            let path = path.clone();
            async move {
                tokio::net::UnixStream::connect(path)
                    .await
                    .map(hyper_util::rt::TokioIo::new)
            }
        }))
        .await
}

struct DaemonGuard {
    pid: Option<Pid>,
}

impl DaemonGuard {
    fn new() -> Self {
        Self { pid: None }
    }

    fn track(&mut self, pid: Pid) {
        self.pid = Some(pid);
    }

    fn stop(&mut self) {
        if let Some(pid) = self.pid.take() {
            let _ = kill_process(pid, Signal::TERM);
            let deadline = Instant::now() + Duration::from_secs(10);
            while Instant::now() < deadline {
                if test_kill_process(pid).is_err() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            let _ = kill_process(pid, Signal::KILL);
        }
    }
}

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        self.stop();
    }
}

fn spawn_daemonized(role: &str, socket: &Path, state: &Path, root: &Path) {
    let humatad = std::env::var_os("HUMATAD2").expect("HUMATAD2 env");
    let holder = std::env::var_os("HUMATA2_HOLDER").expect("HUMATA2_HOLDER env");
    let started = Instant::now();
    // Capture stdout and stderr via pipes (just like SSH does). If the
    // daemonized child failed to redirect stdout/stderr, `.output()` would
    // hang until the daemon exited.
    let output = Command::new(&humatad)
        .args(["serve", "--role", role, "--admin", "--daemonize", "--socket"])
        .arg(socket)
        .arg("--state")
        .arg(state)
        .arg("--root")
        .arg(root)
        .arg("--holder")
        .arg(&holder)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn humatad2 --daemonize");
    assert!(
        output.status.success(),
        "parent should exit 0, got {:?}; stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "parent took {:?} to detach and close stdio",
        started.elapsed()
    );
}

async fn wait_for_server(socket: &Path) -> pb::ServerInfo {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(channel) = connect(socket).await {
            let mut client = pb::daemon_service_client::DaemonServiceClient::new(channel);
            if let Ok(resp) = client.get_server_info(pb::GetServerInfoRequest {}).await {
                return resp.into_inner();
            }
        }
        assert!(
            Instant::now() < deadline,
            "daemonized humatad2 did not answer on {}",
            socket.display()
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

fn read_lock_pid(state: &Path) -> Pid {
    let text = std::fs::read_to_string(state.join("daemon.lock")).expect("read daemon.lock");
    let raw: i32 = text.trim().parse().expect("numeric pid in daemon.lock");
    Pid::from_raw(raw).expect("valid pid")
}

fn count_restart_reports(state: &Path) -> usize {
    let reports = state.join("restarts");
    std::fs::read_dir(&reports)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().join("restart.json").is_file())
        .count()
}

async fn wait_for_restart_reports(state: &Path, expected: usize) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while count_restart_reports(state) < expected {
        assert!(
            Instant::now() < deadline,
            "expected at least {expected} restart reports in {}, found {}",
            state.display(),
            count_restart_reports(state)
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[tokio::test]
async fn daemonize_detaches_answers_and_reconciles_state_on_restart() {
    let dir = tempfile::Builder::new()
        .prefix("h2d")
        .tempdir_in("/tmp")
        .unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let socket: PathBuf = dir.path().join("humatad.sock");
    let state: PathBuf = dir.path().join("state");
    let root: PathBuf = dir.path().join("root");
    std::fs::create_dir_all(&root).unwrap();

    let mut guard = DaemonGuard::new();

    // 1. First daemonized start.
    spawn_daemonized("host", &socket, &state, &root);
    let info1 = wait_for_server(&socket).await;
    let pid1 = read_lock_pid(&state);
    guard.track(pid1);

    // Daemon must be in its own detached session (different from caller's session).
    let my_sid = getsid(None).expect("getsid self");
    let daemon_sid = getsid(Some(pid1)).expect("getsid daemon");
    assert_ne!(daemon_sid, my_sid, "daemon must be in a detached session");
    assert!(
        state.join("logs/humatad2.daemon.log").is_file(),
        "daemonize should create its default log file"
    );

    // Create a channel so there is durable SQLite state to reconcile on restart.
    let channel = connect(&socket).await.expect("connect");
    let mut channels = pb::channel_service_client::ChannelServiceClient::new(channel);
    let created = channels
        .create_channel(pb::CreateChannelRequest {
            name: "Detached Persistence".to_owned(),
            ..Default::default()
        })
        .await
        .expect("create channel")
        .into_inner();
    let created_id = created.channel.expect("channel").id;
    wait_for_restart_reports(&state, 1).await;

    // 2. Stop the daemon (simulating a container/machine restart or kill).
    guard.stop();

    // 3. Restart via `--daemonize` on the same state and socket.
    spawn_daemonized("host", &socket, &state, &root);
    let info2 = wait_for_server(&socket).await;
    let pid2 = read_lock_pid(&state);
    guard.track(pid2);

    assert_ne!(pid1, pid2, "restarted daemon must be a new process");
    assert_eq!(
        info1.machine, info2.machine,
        "restarted daemon must preserve machine_id"
    );

    // Verify prior SQLite state survived and restart reconciliation completed.
    let channel2 = connect(&socket).await.expect("reconnect");
    let mut channels2 = pb::channel_service_client::ChannelServiceClient::new(channel2);
    let listed = channels2
        .list_channels(pb::ListChannelsRequest::default())
        .await
        .expect("list channels")
        .into_inner();
    assert!(
        listed.channels.iter().any(|c| c.id == created_id),
        "prior channel should be preserved after detached restart"
    );
    wait_for_restart_reports(&state, 2).await;

    guard.stop();
}
