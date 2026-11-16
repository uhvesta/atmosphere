//! Real first-start crash before the holder's READY handshake.
use futures_util::{SinkExt, StreamExt};
use std::{
    fs,
    os::unix::{fs::PermissionsExt, process::ExitStatusExt},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use terminal_wire::{ClientFrame, PROTOCOL_VERSION, ServerFrame, SessionState};
use tokio_tungstenite::{connect_async, tungstenite::Message};
const SESSION: &str = "019a83ae-6368-72f5-90bc-5799b82886a1";

struct OwnedHost {
    child: Option<Child>,
    root: PathBuf,
}
impl Drop for OwnedHost {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        // A failing regression must not leave a mistakenly launched real holder
        // or program alive after its owning host was killed.
        for path in [
            self.root.join("state/s1.pid"),
            self.root.join("program-pid"),
        ] {
            if let Ok(pid) = fs::read_to_string(path) {
                if let Ok(pid) = pid.trim().parse::<u32>() {
                    kill_matching_pid(pid, &self.root, false);
                }
            }
        }
        if let Ok(pid) = fs::read_to_string(self.root.join("paused-pid")) {
            if let Ok(pid) = pid.parse::<u32>() {
                kill_paused_wrapper(pid, &self.root, false);
            }
        }
    }
}
fn kill_paused_wrapper(pid: u32, root: &Path, required: bool) {
    kill_matching_pid(pid, &root.join("holder-wrapper"), required);
}
fn kill_matching_pid(pid: u32, marker: &Path, required: bool) {
    let out = Command::new("ps")
        .args(["-ww", "-p", &pid.to_string(), "-o", "command="])
        .output()
        .unwrap();
    let owned = out.status.success()
        && String::from_utf8_lossy(&out.stdout).contains(&marker.display().to_string());
    if required {
        assert!(owned, "paused holder no longer belongs to this fixture");
    }
    if owned {
        // SAFETY: this exact PID's argv still names the unique owned wrapper.
        let killed = unsafe { libc::kill(pid as i32, libc::SIGKILL) };
        if required {
            assert_eq!(killed, 0);
        }
    }
}
fn start(root: &Path, holder: &Path, address: std::net::SocketAddr) -> Child {
    let binary = fs::canonicalize(std::env::var_os("TERMINAL_HOST_BINARY").unwrap()).unwrap();
    let log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("host.log"))
        .unwrap();
    Command::new(binary)
        .arg("--state")
        .arg(root.join("state"))
        .arg("--holder")
        .arg(holder)
        .arg("--static-dir")
        .arg(root.join("assets"))
        .arg("--listen")
        .arg(address.to_string())
        .arg("--profiles")
        .arg(root.join("profiles.json"))
        .args(["--start", "startup"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log)
        .spawn()
        .unwrap()
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sigkill_before_holder_ready_preserves_exact_startup_recovery_reservation() {
    let directory = tempfile::Builder::new()
        .prefix("h2-first-")
        .tempdir_in("/tmp")
        .unwrap();
    let root = directory.path();
    fs::set_permissions(root, fs::Permissions::from_mode(0o700)).unwrap();
    fs::create_dir(root.join("assets")).unwrap();
    fs::write(root.join("assets/index.html"), "isolated startup fixture").unwrap();
    fs::create_dir(root.join("work")).unwrap();
    let real_holder = fs::canonicalize(std::env::var_os("HUMATA2_HOLDER").unwrap()).unwrap();
    fs::write(
        root.join("real-holder"),
        real_holder.as_os_str().as_encoded_bytes(),
    )
    .unwrap();
    let wrapper = root.join("holder-wrapper");
    fs::write(
        &wrapper,
        r#"#!/bin/sh
root=$(dirname "$0")
printf '%s' "$$" > "$root/paused-pid"
while [ ! -f "$root/release" ]; do sleep 0.05; done
IFS= read -r holder < "$root/real-holder"
exec "$holder" "$@"
"#,
    )
    .unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();
    let hint = serde_json::json!({"provider":"codex","session_id":SESSION,"working_directory":root.join("work"),"state_paths":[root.join("explicit-provider-state")]});
    let profile = serde_json::json!([{"name":"startup","title":"First-start crash fixture","program":"/bin/sh","args":["-c","printf '%s' $$ > \"$1/program-pid\"; printf executed > \"$1/program-started\"; sleep 60","startup",root],"cwd":root.join("work"),"endpoint":"Local","recovery":hint,"history":{"mode":"off"}}]);
    fs::write(
        root.join("profiles.json"),
        serde_json::to_vec(&profile).unwrap(),
    )
    .unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let mut owned = OwnedHost {
        child: Some(start(root, &wrapper, address)),
        root: root.into(),
    };
    let deadline = Instant::now() + Duration::from_secs(8);
    let paused = loop {
        if let Ok(value) = fs::read_to_string(root.join("paused-pid")) {
            break value.parse::<u32>().unwrap();
        }
        assert!(
            owned.child.as_mut().unwrap().try_wait().unwrap().is_none(),
            "owned host exited before paused holder"
        );
        assert!(
            Instant::now() < deadline,
            "holder did not reach the owned pre-READY pause"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    // This is an observable blocked launch, not an assertion about function order.
    assert!(!root.join("release").exists());
    assert!(!root.join("program-started").exists());
    let bytes = fs::read(root.join("state/workspace.json")).unwrap();
    let saved: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let sessions = saved["sessions"].as_array().unwrap();
    assert_eq!(sessions.len(), 1);
    let uuid = sessions[0]["uuid"].as_str().unwrap().to_owned();
    assert!(!uuid.is_empty());
    assert_eq!(sessions[0]["startup_profile"], "startup");
    assert_eq!(sessions[0]["recovery"], hint);
    let mut child = owned.child.take().unwrap();
    child.kill().unwrap();
    assert_eq!(child.wait().unwrap().signal(), Some(libc::SIGKILL));
    // No paused wrapper may later escape into a real holder after the test.
    kill_paused_wrapper(paused, root, true);
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let out = Command::new("ps")
            .args(["-p", &paused.to_string(), "-o", "stat="])
            .output()
            .unwrap();
        if !out.status.success()
            || String::from_utf8_lossy(&out.stdout)
                .trim_start()
                .starts_with('Z')
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "owned paused holder remained executable"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    owned.child = Some(start(root, &real_holder, address));
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut socket = loop {
        if let Ok((socket, _)) = connect_async(format!("ws://{address}/ws")).await {
            break socket;
        }
        assert!(
            owned.child.as_mut().unwrap().try_wait().unwrap().is_none(),
            "owned restarted host exited"
        );
        assert!(
            Instant::now() < deadline,
            "owned host did not reopen its durable reservation"
        );
        tokio::time::sleep(Duration::from_millis(40)).await;
    };
    let token = fs::read_to_string(root.join("state/browser.token")).unwrap();
    socket
        .send(Message::Text(
            serde_json::to_string(&ClientFrame::Authenticate {
                protocol: PROTOCOL_VERSION,
                token,
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    let message = tokio::time::timeout(Duration::from_secs(5), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let sessions = match serde_json::from_str::<ServerFrame>(message.to_text().unwrap()).unwrap() {
        ServerFrame::Sessions { sessions, .. } => sessions,
        _ => panic!("expected authenticated session descriptors"),
    };
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].uuid, uuid);
    assert!(matches!(sessions[0].state, SessionState::Unavailable));
    assert!(sessions[0].recovery_available);
    let reopened: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("state/workspace.json")).unwrap()).unwrap();
    assert_eq!(reopened["sessions"][0]["uuid"], uuid);
    assert_eq!(reopened["sessions"][0]["recovery"], hint);
    assert!(
        !root.join("program-started").exists(),
        "restart must not execute a second startup program"
    );
    assert!(
        !root.join("state/s1.pid").exists(),
        "restart must not spawn a replacement real holder"
    );
}
