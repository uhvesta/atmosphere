//! Fault only this fixture's TCP path; never restart the VM or touch other SSH sessions.
use super::*;
use std::{
    net::SocketAddr,
    os::unix::fs::PermissionsExt,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::watch,
};
struct Active(Arc<AtomicUsize>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
struct PathProxy {
    address: SocketAddr,
    online: Arc<AtomicBool>,
    discard_downstream: Arc<AtomicBool>,
    retire: watch::Sender<u64>,
    active: Arc<AtomicUsize>,
    dials: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}
impl Drop for PathProxy {
    fn drop(&mut self) {
        self.online.store(false, Ordering::Release);
        self.retire.send_modify(|n| *n += 1);
        self.task.abort();
    }
}
impl PathProxy {
    async fn new(upstream: SocketAddr) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let online = Arc::new(AtomicBool::new(true));
        let discard_downstream = Arc::new(AtomicBool::new(false));
        let active = Arc::new(AtomicUsize::new(0));
        let dials = Arc::new(AtomicUsize::new(0));
        let (retire, _) = watch::channel(0u64);
        let on = online.clone();
        let discard = discard_downstream.clone();
        let count = active.clone();
        let changes = retire.clone();
        let accepted = dials.clone();
        let task = tokio::spawn(async move {
            while let Ok((client, _)) = listener.accept().await {
                accepted.fetch_add(1, Ordering::AcqRel);
                if !on.load(Ordering::Acquire) {
                    drop(client);
                    continue;
                }
                let mut retired = changes.subscribe();
                let discard = discard.clone();
                count.fetch_add(1, Ordering::AcqRel);
                let active = Active(count.clone());
                tokio::spawn(async move {
                    let _active = active;
                    let copy = async {
                        let Ok(server) = TcpStream::connect(upstream).await else {
                            return;
                        };
                        let (mut client_read, mut client_write) = client.into_split();
                        let (mut server_read, mut server_write) = server.into_split();
                        let up = tokio::io::copy(&mut client_read, &mut server_write);
                        let down = async {
                            let mut bytes = [0u8; 16384];
                            loop {
                                let count = server_read.read(&mut bytes).await?;
                                if count == 0 {
                                    return Ok::<(), std::io::Error>(());
                                }
                                if !discard.load(Ordering::Acquire) {
                                    client_write.write_all(&bytes[..count]).await?;
                                }
                            }
                        };
                        tokio::select! {_=up=>{},_=down=>{}}
                    };
                    tokio::select! {_=copy=>{},_=retired.changed()=>{}}
                });
            }
        });
        Self {
            address,
            online,
            discard_downstream,
            retire,
            active,
            dials,
            task,
        }
    }
    async fn cut(&self) {
        self.online.store(false, Ordering::Release);
        self.retire.send_modify(|n| *n += 1);
        tokio::time::timeout(Duration::from_secs(3), async {
            while self.active.load(Ordering::Acquire) > 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }
}
fn trusted_destination(config: &std::path::Path, destination: &str) -> SocketAddr {
    let output = std::process::Command::new("ssh")
        .arg("-G")
        .arg("-F")
        .arg(config)
        .arg(destination)
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    let value = |name: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(name))
            .unwrap()
            .trim()
            .to_owned()
    };
    let host = value("hostname ");
    let port = value("port ").parse::<u16>().unwrap();
    let address = SocketAddr::new(
        host.parse()
            .expect("dedicated fixture must resolve to an IP address"),
        port,
    );
    assert!(
        address.ip().is_loopback(),
        "fault proxy is restricted to the dedicated loopback VM SSH endpoint"
    );
    address
}
fn route(
    config: &std::path::Path,
    trusted: &std::path::Path,
    destination: &str,
    proxy: &PathProxy,
) {
    // ProxyCommand keeps the original hostname/port host-key identity. The
    // included trusted config and all personal config files remain unchanged.
    assert!(
        std::path::Path::new("/usr/bin/nc").is_file(),
        "manual fault fixture requires the system netcat"
    );
    let text = format!(
        "Host {destination}\n  ProxyCommand /usr/bin/nc 127.0.0.1 {}\nInclude \"{}\"\n",
        proxy.address.port(),
        trusted.display()
    );
    let temporary = config.with_extension("tmp");
    std::fs::write(&temporary, text).unwrap();
    std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::rename(temporary, config).unwrap();
}
async fn frame(view: &mut Subscription) -> ServerFrame {
    tokio::time::timeout(Duration::from_secs(25), view.receiver.recv())
        .await
        .unwrap()
        .unwrap()
        .frame
}
async fn visible(host: &Host, view: &mut Subscription, needle: &str) -> Vec<u8> {
    use ghostty_vt::Grid;
    let deadline = Instant::now() + Duration::from_secs(25);
    loop {
        host.frame(view.id, ClientFrame::SnapshotRequest { id: 1 })
            .await
            .unwrap();
        let bytes = loop {
            if let ServerFrame::Snapshot { bytes, .. } = frame(view).await {
                break bytes;
            }
        };
        let mut vt = Terminal::from_checkpoint(&bytes).unwrap();
        vt.sync();
        let text = (0..vt.size().rows)
            .filter_map(|y| vt.row(y))
            .flat_map(|row| row.cells.iter().map(|cell| cell.text.as_str()))
            .collect::<String>();
        if text.contains(needle) {
            return bytes;
        }
        assert!(
            Instant::now() < deadline,
            "retained terminal output was not restored"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
async fn wait_running(handle: &TerminalHandle) {
    tokio::time::timeout(Duration::from_secs(25), async {
        loop {
            if matches!(handle.info().await.unwrap().state, SessionState::Running) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
}
async fn probe_count(config: PathBuf, destination: String, path: String) -> String {
    tokio::task::spawn_blocking(move || {
        let output = std::process::Command::new("ssh")
            .arg("-F")
            .arg(config)
            .args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=5"])
            .arg(destination)
            .arg(format!("cat '{path}/count' 2>/dev/null"))
            .output()
            .unwrap();
        if output.status.success() {
            String::from_utf8(output.stdout).unwrap()
        } else {
            String::new()
        }
    })
    .await
    .unwrap()
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires the dedicated trusted Lima SSH fixture and system netcat; cuts only owned proxy paths"]
async fn ssh_outage_and_route_change_reattach_existing_holder_without_input_replay() {
    let trusted = std::fs::canonicalize(
        std::env::var_os("TERMINAL_LAB_SSH_CONFIG").expect("set trusted VM config"),
    )
    .unwrap();
    let destination =
        std::env::var("TERMINAL_LAB_SSH_HOST").unwrap_or("lima-h2-terminal-c1".into());
    let upstream = trusted_destination(&trusted, &destination);
    let first = PathProxy::new(upstream).await;
    let second = PathProxy::new(upstream).await;
    let root = tempfile::Builder::new()
        .prefix("h2-roam-")
        .tempdir_in("/tmp")
        .unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let config = root.path().join("route.config");
    route(&config, &trusted, &destination, &first);
    let remote = format!(
        "/tmp/h2-roam-{}-{}",
        std::process::id(),
        root.path().file_name().unwrap().to_string_lossy()
    );
    let script = r#"stty -echo; mkdir -p "$1"; cd "$1"; export ROAM_KEEP=retained; count=0
printf '\033]7;file://localhost%s\007READY_PID=%s\r\n' "$PWD" "$$"
while IFS= read -r line; do
 count=$((count+1)); printf '%s' "$count" > count
 printf 'COUNT=%s INPUT=%s PID=%s KEEP=%s CWD=%s\r\n' "$count" "$line" "$$" "$ROAM_KEEP" "$PWD"
 if [ "$line" = arm ]; then
  printf 'ARMED\r\n'; sleep 1; printf '\033[32mOFFLINE_OUTPUT\033[0m\r\n'; stty -icanon min 1 time 0
  printf '\033[5n'; dd bs=1 count=4 of=query-reply 2>/dev/null; stty icanon; printf 'QUERY_REPLIED\r\n'
 fi
done"#;
    let holder = std::fs::canonicalize(std::env::var_os("HUMATA2_HOLDER").unwrap()).unwrap();
    let profile = Profile {
        name: "ssh".into(),
        title: "roaming fixture".into(),
        program: "/bin/sh".into(),
        args: vec!["-c".into(), script.into(), "roaming".into(), remote.clone()],
        cwd: "/tmp".into(),
        endpoint: Endpoint::Ssh {
            config: config.clone(),
            destination: destination.clone(),
            holder: std::env::var_os("TERMINAL_LAB_SSH_HOLDER")
                .map(PathBuf::from)
                .unwrap_or("/tmp/h2-terminal-holder".into()),
            root: format!(
                "/var/tmp/h2-roam-holder-{}",
                root.path().file_name().unwrap().to_string_lossy()
            ),
        },
        recovery: None,
        history: HistoryPolicy::default(),
    };
    let host = Host::start(
        Config {
            state: root.path().into(),
            label: "owned fault proxy".into(),
            start_profiles: vec!["ssh".into()],
            default_history: HistoryPolicy::default(),
        },
        SessionStore::open_memory_only(root.path(), holder).unwrap(),
        vec![profile],
    )
    .await
    .unwrap();
    let before = host.terminals().await.unwrap().remove(0);
    let handle = host.terminal(&before.uuid).await.unwrap();
    let mut view = host.subscribe().await.unwrap();
    let _ = frame(&mut view).await;
    host.frame(view.id, ClientFrame::Open { id: 1 })
        .await
        .unwrap();
    let initial = visible(&host, &mut view, "READY_PID=").await;
    handle
        .inject_text_with_control(
            "one".into(),
            InjectionOptions {
                submit: true,
                ..InjectionOptions::raw()
            },
            ControlPolicy::TakeControl,
        )
        .await
        .unwrap()
        .written()
        .await
        .unwrap();
    visible(&host, &mut view, "COUNT=1 INPUT=one").await;
    handle
        .inject_text(
            "arm".into(),
            InjectionOptions {
                submit: true,
                ..InjectionOptions::raw()
            },
        )
        .await
        .unwrap()
        .written()
        .await
        .unwrap();
    visible(&host, &mut view, "ARMED").await;
    let (reply, ready) = tokio::sync::oneshot::channel();
    host.tx
        .send(Command::ReconnectBackoffFixture {
            delay: Duration::from_millis(10),
            reply,
        })
        .await
        .unwrap();
    ready.await.unwrap();
    let prior_dials = first.dials.load(Ordering::Acquire);
    first.cut().await;
    tokio::time::timeout(Duration::from_secs(8), async {
        while matches!(handle.info().await.unwrap().state, SessionState::Running) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert!(matches!(
        handle
            .inject_text(
                "must_not_be_replayed".into(),
                InjectionOptions {
                    submit: true,
                    ..InjectionOptions::raw()
                }
            )
            .await,
        Err(HandleError::NotRunning { .. })
    ));
    // Local metadata remains responsive while SSH retries; no global Store loan.
    // Prove persistent retry after the former suspected four-attempt boundary.
    // These are real failed TCP/SSH connection attempts, not synthetic states.
    tokio::time::timeout(Duration::from_secs(40), async {
        while first.dials.load(Ordering::Acquire) < prior_dials + 5 {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    let failed_dials = first.dials.load(Ordering::Acquire) - prior_dials;
    tokio::time::timeout(
        Duration::from_secs(1),
        handle.set_attrs(BTreeMap::from([("offline".into(), "editable".into())])),
    )
    .await
    .unwrap()
    .unwrap();
    route(&config, &trusted, &destination, &second);
    wait_running(&handle).await;
    visible(&host, &mut view, "QUERY_REPLIED").await;
    handle
        .inject_text(String::new(), InjectionOptions::raw())
        .await
        .unwrap()
        .written()
        .await
        .unwrap();
    // Deliver an input while deliberately discarding only encrypted replies.
    // It executes once, its ACK is lost, and reconnect must not replay it.
    second.discard_downstream.store(true, Ordering::Release);
    let lost = handle
        .inject_text(
            "lost_ack".into(),
            InjectionOptions {
                submit: true,
                ..InjectionOptions::raw()
            },
        )
        .await
        .unwrap();
    let lost_ticket = lost.ticket().clone();
    tokio::time::timeout(Duration::from_secs(8), async {
        while probe_count(trusted.clone(), destination.clone(), remote.clone()).await != "3" {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    host.frame(view.id, ClientFrame::TakeControl { id: 1 })
        .await
        .unwrap();
    host.frame(
        view.id,
        ClientFrame::Input {
            id: 1,
            seq: 1,
            bytes: b"viewer_lost_ack\r".to_vec(),
        },
    )
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(8), async {
        while probe_count(trusted.clone(), destination.clone(), remote.clone()).await != "4" {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    second.cut().await;
    let uncertain = lost.written().await;
    let mut viewer_uncertain = false;
    let mut readonly = false;
    while !viewer_uncertain || !readonly {
        match frame(&mut view).await {
            ServerFrame::Error { code, .. } if code == "input_uncertain" => viewer_uncertain = true,
            ServerFrame::Snapshot { live: false, .. } => readonly = true,
            _ => {}
        }
    }
    let third = PathProxy::new(upstream).await;
    route(&config, &trusted, &destination, &third);
    wait_running(&handle).await;
    handle
        .inject_text_with_control(
            "after".into(),
            InjectionOptions {
                submit: true,
                ..InjectionOptions::raw()
            },
            ControlPolicy::TakeControl,
        )
        .await
        .unwrap()
        .written()
        .await
        .unwrap();
    visible(&host, &mut view, "COUNT=5 INPUT=after").await;
    host.frame(view.id, ClientFrame::TakeControl { id: 1 })
        .await
        .unwrap();
    host.frame(
        view.id,
        ClientFrame::Input {
            id: 1,
            seq: 1,
            bytes: b"duplicate_must_not_execute\r".to_vec(),
        },
    )
    .await
    .unwrap();
    loop {
        if matches!(frame(&mut view).await,ServerFrame::Error{code,..}if code=="input_sequence") {
            break;
        }
    }
    host.frame(
        view.id,
        ClientFrame::Input {
            id: 1,
            seq: 2,
            bytes: b"viewer_after\r".to_vec(),
        },
    )
    .await
    .unwrap();
    let final_screen = visible(&host, &mut view, "COUNT=6 INPUT=viewer_after").await;
    let after = handle.info().await.unwrap();
    handle
        .stop_with_control(ControlPolicy::TakeControl)
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(8), async {
        while !matches!(
            handle.info().await.unwrap().state,
            SessionState::Exited { .. }
        ) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    host.shutdown().await.unwrap();
    // Cleanup only the unique fixture directory through trusted SSH, not profiles.
    let cleanup_config = trusted.clone();
    let cleanup_destination = destination.clone();
    let cleanup_remote = remote.clone();
    tokio::task::spawn_blocking(move ||std::process::Command::new("ssh").arg("-F").arg(cleanup_config).arg(cleanup_destination).arg(format!("rm -f '{cleanup_remote}/count' '{cleanup_remote}/query-reply'; rmdir '{cleanup_remote}'")).status().unwrap()).await.unwrap();
    assert!(failed_dials >= 5);
    assert_eq!(before.uuid, after.uuid);
    assert_eq!(before.id, after.id);
    assert_eq!(after.cwd, remote);
    assert_eq!(
        after.attributes.get("offline").map(String::as_str),
        Some("editable")
    );
    assert!(matches!(uncertain,Err(HandleError::Uncertain{ticket,..})if ticket==lost_ticket));
    use ghostty_vt::Grid;
    let text = |bytes: &[u8]| {
        let mut vt = Terminal::from_checkpoint(bytes).unwrap();
        vt.sync();
        (0..vt.size().rows)
            .filter_map(|y| vt.row(y))
            .flat_map(|r| r.cells.iter().map(|c| c.text.as_str()))
            .collect::<String>()
    };
    let original = text(&initial);
    let restored = text(&final_screen);
    let pid = original
        .split("READY_PID=")
        .nth(1)
        .unwrap()
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .unwrap();
    assert!(restored.contains(&format!("PID={pid} KEEP=retained")));
    assert!(restored.contains("OFFLINE_OUTPUT"));
    assert!(restored.contains("COUNT=3 INPUT=lost_ack"));
    assert!(restored.contains("COUNT=4 INPUT=viewer_lost_ack"));
    assert!(viewer_uncertain && readonly);
    assert!(!restored.contains("must_not_be_replayed"));
    assert!(!restored.contains("duplicate_must_not_execute"));
}
