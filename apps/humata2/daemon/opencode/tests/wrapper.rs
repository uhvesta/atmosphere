//! The `humata opencode` wrapper (D201, D201a) with a fake `opencode`: it
//! starts its own server with a fresh password, attaches the TUI to the
//! conversation's session, keeps the state file, and ends the server with
//! the TUI. An OpenCode too old for the server API runs as a plain TUI.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use humata2_opencode::{
    Humata, HumataError, StateFile, Turn, Wrapper, WrapperError, run_wrapper, session_id,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const CONVERSATION: &str = "0f8a1c2e-7b3d-4e5f-9a6b-1c2d3e4f5a6b";

/// A Humata that hears turns and never decides.
#[derive(Default)]
struct Quiet {
    turns: Mutex<Vec<Turn>>,
}

impl Humata for Quiet {
    async fn turn(&self, turn: Turn) {
        if let Ok(mut turns) = self.turns.lock() {
            turns.push(turn);
        }
    }

    async fn ask(&self, _payload: Vec<u8>) -> Result<Vec<u8>, HumataError> {
        Ok(Vec::new())
    }
}

/// A loopback server answering as OpenCode's does, enough for the wrapper:
/// creating a session, the event stream, and empty lists. Gives its URL and
/// the request lines it read.
async fn fake_server() -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind loopback");
    let url = format!("http://{}", listener.local_addr().expect("an address"));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&seen);
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            tokio::spawn(answer(socket, Arc::clone(&log)));
        }
    });
    (url, seen)
}

async fn answer(mut socket: TcpStream, log: Arc<Mutex<Vec<String>>>) {
    let mut read = Vec::new();
    let mut buffer = [0u8; 4096];
    let head_end = loop {
        if let Some(at) = read.windows(4).position(|w| w == b"\r\n\r\n") {
            break at;
        }
        match socket.read(&mut buffer).await {
            Ok(0) | Err(_) => return,
            Ok(n) => read.extend_from_slice(&buffer[..n]),
        }
    };
    let head = String::from_utf8_lossy(&read[..head_end]).into_owned();
    let line = head.lines().next().unwrap_or_default().to_owned();
    if let Ok(mut log) = log.lock() {
        log.push(line.clone());
    }
    let json = |body: &str| {
        format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
            body.len()
        )
    };
    let response = if line.starts_with("GET /api/event") {
        let frame = "data: {\"type\":\"server.connected\",\"data\":{}}\n\n";
        format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n{:x}\r\n{frame}\r\n",
            frame.len()
        )
    } else if line.starts_with("POST /api/session ") {
        json(r#"{"data":{"id":"ses_x"}}"#)
    } else if line.starts_with("GET /api/session/active") {
        json(r#"{"data":{}}"#)
    } else if line.starts_with("GET /api/permission/request") {
        json(r#"{"data":[]}"#)
    } else {
        "HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\r\n".to_owned()
    };
    let _ = socket.write_all(response.as_bytes()).await;
    // Held open, as the event stream is, until the client goes.
    let _ = socket.read(&mut buffer).await;
}

/// A fake `opencode` in `dir`. `serve` prints `url` (and then runs for
/// `serve_runs`); the TUI writes what it was given and exits `tui_exit`
/// after `tui_runs`.
fn fake_opencode(dir: &Path, version: &str, url: &str, serve_runs: &str, tui: &str) -> PathBuf {
    let out = dir.display();
    let script = format!(
        r#"#!/bin/sh
case "$1" in
  --version) echo '{version}'; exit 0 ;;
  serve)
    printf '%s\n' "$@" > '{out}/serve-args'
    printf '%s' "$OPENCODE_SERVER_PASSWORD" > '{out}/serve-password'
    echo $$ > '{out}/serve-pid'
    echo 'loading config'
    echo 'server listening on {url}'
    exec {serve_runs} ;;
  *)
    printf '%s\n' "$@" > '{out}/tui-args'
    printf '%s' "${{OPENCODE_PASSWORD:-}}" > '{out}/tui-password'
    {tui} ;;
esac
"#
    );
    let path = dir.join("opencode");
    std::fs::write(&path, script).expect("write the fake");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    path
}

fn wrapper(dir: &Path, program: PathBuf) -> Wrapper {
    Wrapper {
        program,
        conversation: CONVERSATION.to_owned(),
        state: dir.join("state").join("opencode.json"),
        folder: dir.to_path_buf(),
        retry: Duration::from_millis(100),
        wait_for_url: Duration::from_secs(10),
        model: None,
    }
}

fn read(dir: &Path, name: &str) -> String {
    std::fs::read_to_string(dir.join(name)).unwrap_or_default()
}

fn alive(pid: &str) -> bool {
    std::process::Command::new("kill")
        .args(["-0", pid.trim()])
        .status()
        .is_ok_and(|status| status.success())
}

/// Waits for `name` in `dir` to hold something.
async fn written(dir: &Path, name: &str) -> String {
    for _ in 0..200 {
        let text = read(dir, name);
        if !text.trim().is_empty() {
            return text;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("{name} was never written");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_tui_attaches_to_its_own_server_and_the_server_ends_with_it() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let (url, seen) = fake_server().await;
    let program = fake_opencode(dir.path(), "opencode v2.3.0", &url, "sleep 600", "sleep 1; exit 7");
    let wrapper = wrapper(dir.path(), program);
    let state = wrapper.state.clone();
    let code = run_wrapper(wrapper, Arc::new(Quiet::default())).await.expect("the TUI ran");
    // The TUI's own exit is the wrapper's.
    assert_eq!(code, 7);

    let id = session_id(CONVERSATION);
    let tui: Vec<String> = read(dir.path(), "tui-args").lines().map(str::to_owned).collect();
    assert_eq!(tui, ["--server", url.as_str(), "-s", id.as_str()]);
    let password = read(dir.path(), "serve-password");
    assert!(password.len() >= 16, "a fresh password, not {password:?}");
    assert_eq!(read(dir.path(), "tui-password"), password);
    let serve = read(dir.path(), "serve-args");
    assert!(serve.contains("127.0.0.1"));
    assert!(!serve.contains("--cors") && !serve.contains("--service"));

    let file = StateFile::read(&state).expect("the state file");
    assert_eq!((file.url.as_str(), file.password.as_str()), (url.as_str(), password.as_str()));
    let mode = std::fs::metadata(&state).expect("stat").permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    // The session was made before the TUI asked for it.
    let lines = seen.lock().map(|lines| lines.clone()).unwrap_or_default();
    assert!(lines.iter().any(|line| line.starts_with("POST /api/session ")), "{lines:?}");
    assert!(!alive(&read(dir.path(), "serve-pid")), "serve outlived the TUI");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_tui_runs_on_the_sessions_model_when_one_was_chosen() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let (url, _seen) = fake_server().await;
    let program = fake_opencode(dir.path(), "opencode v2.3.0", &url, "sleep 600", "exit 0");
    let wrapper = Wrapper {
        model: Some("anthropic/claude-haiku-5.5".to_owned()),
        ..wrapper(dir.path(), program)
    };
    run_wrapper(wrapper, Arc::new(Quiet::default())).await.expect("the TUI ran");

    let id = session_id(CONVERSATION);
    let tui: Vec<String> = read(dir.path(), "tui-args").lines().map(str::to_owned).collect();
    assert_eq!(
        tui,
        ["--server", url.as_str(), "-s", id.as_str(), "--model", "anthropic/claude-haiku-5.5"]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_fresh_password_each_start() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let (url, _seen) = fake_server().await;
    let program = fake_opencode(dir.path(), "opencode v2.3.0", &url, "sleep 600", "exit 0");
    run_wrapper(wrapper(dir.path(), program.clone()), Arc::new(Quiet::default()))
        .await
        .expect("ran");
    let first = read(dir.path(), "serve-password");
    run_wrapper(wrapper(dir.path(), program), Arc::new(Quiet::default()))
        .await
        .expect("ran again");
    assert_ne!(first, read(dir.path(), "serve-password"));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_server_dying_ends_the_tui_and_says_so() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let (url, _seen) = fake_server().await;
    let program = fake_opencode(dir.path(), "opencode v2.3.0", &url, "sleep 1", "sleep 600");
    let ran = run_wrapper(wrapper(dir.path(), program), Arc::new(Quiet::default())).await;
    assert!(matches!(ran, Err(WrapperError::ServeEnded { .. })), "{ran:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_opencode_too_old_for_the_server_runs_as_a_plain_tui() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let (url, _seen) = fake_server().await;
    let program = fake_opencode(dir.path(), "opencode v2.2.7", &url, "sleep 600", "exit 0");
    let code = run_wrapper(wrapper(dir.path(), program), Arc::new(Quiet::default()))
        .await
        .expect("the TUI ran");
    assert_eq!(code, 0);
    assert!(read(dir.path(), "serve-pid").is_empty(), "no server for an old OpenCode");
    assert!(!read(dir.path(), "tui-args").contains("--server"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_that_never_says_where_it_listens_falls_back_to_a_plain_tui() {
    let dir = tempfile::tempdir().expect("a temp dir");
    // `serve` prints no URL: a non-loopback one is no URL either.
    let program = fake_opencode(
        dir.path(),
        "opencode v2.3.0",
        "http://0.0.0.0:4096",
        "sleep 600",
        "exit 0",
    );
    let mut wrapper = wrapper(dir.path(), program);
    wrapper.wait_for_url = Duration::from_secs(1);
    let code = run_wrapper(wrapper, Arc::new(Quiet::default())).await.expect("the TUI ran");
    assert_eq!(code, 0);
    assert!(!read(dir.path(), "tui-args").contains("--server"));
    let serve = written(dir.path(), "serve-pid").await;
    assert!(!alive(&serve), "the server that gave no URL was left running");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_left_from_a_crash_is_ended_before_a_new_one_starts() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let (url, _seen) = fake_server().await;
    let program = fake_opencode(dir.path(), "opencode v2.3.0", &url, "sleep 600", "exit 0");
    let wrapper = wrapper(dir.path(), program);
    // A leftover serve, as a crashed wrapper would leave it: its state file
    // names it, and its server still answers to its password.
    let mut leftover = std::process::Command::new("sleep")
        .arg("600")
        .spawn()
        .expect("spawn a leftover");
    if let Some(folder) = wrapper.state.parent() {
        std::fs::create_dir_all(folder).expect("state folder");
    }
    StateFile {
        url: url.clone(),
        password: "old".to_owned(),
        session: session_id(CONVERSATION),
        serve_pid: leftover.id(),
    }
    .write(&wrapper.state)
    .expect("write the old state");
    run_wrapper(wrapper, Arc::new(Quiet::default())).await.expect("ran");
    let ended = tokio::task::spawn_blocking(move || leftover.wait())
        .await
        .expect("joined")
        .expect("waited");
    assert!(!ended.success(), "the leftover was ended by a signal");
}
