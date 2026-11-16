//! Explicit opt-in integration test against a dedicated SSH test host.
use std::{path::PathBuf, time::Duration};
use terminal_session::{Endpoint, Record, SessionSpec, SessionStore, Update};
async fn until(r: &mut terminal_session::Reader, text: &str) -> Vec<u8> {
    tokio::time::timeout(Duration::from_secs(12), async {
        let mut out = Vec::new();
        loop {
            if let Some(Update::Record(Record::Output { bytes, .. })) = r.next().await.unwrap() {
                out.extend(bytes);
            }
            if String::from_utf8_lossy(&out).contains(text) {
                return out;
            }
        }
    })
    .await
    .unwrap()
}
#[tokio::test]
async fn ssh_terminals_keep_their_process_after_transport_and_app_restart() {
    let config = PathBuf::from(
        std::env::var("TERMINAL_LAB_SSH_CONFIG")
            .expect("set TERMINAL_LAB_SSH_CONFIG to a trusted test host config"),
    );
    let root = format!("/var/tmp/terminal-lab-{}", std::process::id());
    let endpoint = Endpoint::Ssh {
        config,
        destination: std::env::var("TERMINAL_LAB_SSH_HOST").unwrap_or("lima-h2-terminal-c1".into()),
        holder: "/tmp/h2-terminal-holder".into(),
        root,
    };
    let dir = tempfile::Builder::new()
        .prefix("tls-")
        .tempdir_in("/tmp")
        .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let mut store = SessionStore::open(dir.path(), PathBuf::new()).unwrap();
    let spec = SessionSpec {
        title: "SSH".into(),
        program: "/bin/sh".into(),
        args: vec!["-i".into()],
        cwd: "/tmp".into(),
        cols: 80,
        rows: 24,
        endpoint,
    };
    let id = store.create(spec.clone()).unwrap();
    let other = store.create(spec).unwrap();
    let mut a = store.attach(id).await.unwrap();
    a.writer
        .input(b"stty -echo; export SSH_KEEP=still_here; printf 'REMOTE_PID=%s\\nEND_PID\\n' $$\r")
        .await
        .unwrap();
    let first = until(&mut a.reader, "END_PID\r\n").await;
    let first = String::from_utf8_lossy(&first);
    let pid = first
        .lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix("REMOTE_PID=")
                .filter(|n| n.chars().all(|c| c.is_ascii_digit()))
        })
        .expect("remote shell pid")
        .to_owned();
    a.writer.resize(93, 29).await.unwrap();
    a.writer.input(b"stty size\r").await.unwrap();
    until(&mut a.reader, "29 93").await;
    store.move_to(id, 1, 0).unwrap();
    a.writer
        .input(b"printf 'BEFORE_OFFLINE\\n'; sleep 0.2; printf 'WHILE_OFFLINE\\n'\r")
        .await
        .unwrap();
    until(&mut a.reader, "BEFORE_OFFLINE\r\n").await;
    drop(a);
    drop(store);
    tokio::time::sleep(Duration::from_millis(400)).await;
    let store = SessionStore::open(dir.path(), PathBuf::new()).unwrap();
    assert_eq!(store.groups()[1], vec![id]);
    let mut a = store.attach(id).await.unwrap();
    assert_eq!((a.cols, a.rows), (93, 29));
    assert!(!a.history.is_empty());
    until(&mut a.reader, "WHILE_OFFLINE\r\n").await;
    a.writer
        .input(b"printf 'REATTACHED=%s:%s\\n' \"$SSH_KEEP\" $$\r")
        .await
        .unwrap();
    until(&mut a.reader, &format!("REATTACHED=still_here:{pid}")).await;
    let mut b = store.attach(other).await.unwrap();
    b.writer
        .input(b"printf 'OTHER=%s\\n' \"${SSH_KEEP-unset}\"\r")
        .await
        .unwrap();
    until(&mut b.reader, "OTHER=unset").await;
    a.writer.stop().await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while a.reader.next().await.unwrap().is_some() {}
    })
    .await
    .unwrap();
    b.writer.stop().await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while b.reader.next().await.unwrap().is_some() {}
    })
    .await
    .unwrap();
}

#[test]
fn crashed_ssh_frontend_fixture() {
    let Some(root) = std::env::var_os("TERMINAL_SSH_CRASH_ROOT") else {
        return;
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let store = SessionStore::open(std::path::Path::new(&root), PathBuf::new()).unwrap();
    runtime.block_on(async {
        let mut terminal = store.attach(1).await.unwrap();
        std::fs::write(std::path::Path::new(&root).join("client-ready"), b"ready").unwrap();
        while terminal.reader.next().await.unwrap().is_some() {}
    });
}
fn tunnel_process(root: &std::path::Path) -> Option<u32> {
    let output = std::process::Command::new("ps")
        .args(["-axo", "pid=,command="])
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| {
            let line = line.trim();
            let (pid, command) = line.split_once(char::is_whitespace)?;
            (command.contains("ssh -F ") && command.contains(&root.to_string_lossy().to_string()))
                .then(|| pid.parse().ok())
                .flatten()
        })
}
fn process_running(pid: u32, root: &std::path::Path) -> bool {
    let output = std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "stat=,command="])
        .output()
        .unwrap();
    let state = String::from_utf8_lossy(&output.stdout);
    !state.trim().is_empty()
        && !state.trim().starts_with('Z')
        && state.contains(&root.to_string_lossy().to_string())
}
#[test]
fn killing_the_ssh_frontend_reaps_its_tunnel_and_keeps_the_remote_shell() {
    let config = PathBuf::from(
        std::env::var("TERMINAL_LAB_SSH_CONFIG").expect("set TERMINAL_LAB_SSH_CONFIG"),
    );
    let root = tempfile::Builder::new()
        .prefix("tsshcrash-")
        .tempdir_in("/tmp")
        .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut store = SessionStore::open(root.path(), PathBuf::new()).unwrap();
    let id = store
        .create(SessionSpec {
            title: "crash recovery".into(),
            program: "/bin/sh".into(),
            args: vec!["-i".into()],
            cwd: "/tmp".into(),
            cols: 80,
            rows: 24,
            endpoint: Endpoint::Ssh {
                config,
                destination: "lima-h2-terminal-c1".into(),
                holder: "/tmp/h2-terminal-holder".into(),
                root: format!("/var/tmp/ssh-crash-{}", std::process::id()),
            },
        })
        .unwrap();
    let identity = runtime.block_on(async {
        let mut terminal = store.attach(id).await.unwrap();
        terminal
            .writer
            .input(b"stty -echo; export CRASH_TOKEN=kept; printf 'IDENTITY=%s\\nEND_ID\\n' $$\r")
            .await
            .unwrap();
        let bytes = until(&mut terminal.reader, "END_ID\r\n").await;
        let output = String::from_utf8_lossy(&bytes);
        output
            .lines()
            .find_map(|line| {
                line.trim()
                    .strip_prefix("IDENTITY=")
                    .filter(|s| s.chars().all(|c| c.is_ascii_digit()))
            })
            .unwrap()
            .to_owned()
    });
    drop(store);
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "crashed_ssh_frontend_fixture", "--nocapture"])
        .env("TERMINAL_SSH_CRASH_ROOT", root.path())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while !root.path().join("client-ready").exists() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    let tunnel = tunnel_process(root.path()).expect("the child has an owned SSH tunnel");
    child.kill().unwrap();
    child.wait().unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(4);
    while process_running(tunnel, root.path()) && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    let orphan = process_running(tunnel, root.path());
    let store = SessionStore::open(root.path(), PathBuf::new()).unwrap();
    runtime.block_on(async {
        let mut terminal = store.attach(id).await.unwrap();
        terminal
            .writer
            .input(b"printf 'AFTER_CRASH=%s:%s\\n' \"$CRASH_TOKEN\" $$\r")
            .await
            .unwrap();
        until(
            &mut terminal.reader,
            &format!("AFTER_CRASH=kept:{identity}\r\n"),
        )
        .await;
        terminal.writer.stop().await.unwrap();
        while terminal.reader.next().await.unwrap().is_some() {}
    });
    if orphan {
        let _ = std::process::Command::new("/bin/kill")
            .args(["-TERM", &tunnel.to_string()])
            .status();
    }
    assert!(
        !orphan,
        "the SSH transport should terminate when its frontend is killed; the remote shell must remain resumable"
    );
}
