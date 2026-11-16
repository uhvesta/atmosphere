//! `humata opencode` (D201, D201a) with a fake `opencode` and a fake
//! daemon: it reports the agent's turns and passes each permission ask to
//! the Inbox as the session's agent, and the Inbox's answer reaches
//! OpenCode's server for the ask's own session.

mod support;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use humata2_cli::{exit, rpc};
use humata2_proto::humata::v1 as pb;
use support::{FakeConsole, FakeDaemon, run};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const ALLOW: &[u8] = br#"{"decision":"once"}"#;

/// One request the fake server read: its request line and body.
type Seen = Arc<Mutex<Vec<(String, Vec<u8>)>>>;

/// A loopback server that streams a run with one ask, as OpenCode's does,
/// and writes `replied` in `dir` once the ask's reply is posted.
async fn fake_server(dir: PathBuf) -> (String, Seen) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let url = format!("http://{}", listener.local_addr().expect("an address"));
    let seen: Seen = Arc::default();
    let log = Arc::clone(&seen);
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            tokio::spawn(answer(socket, Arc::clone(&log), dir.clone()));
        }
    });
    (url, seen)
}

async fn answer(mut socket: TcpStream, log: Seen, dir: PathBuf) {
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
    let length = head
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.trim().eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = read[head_end + 4..].to_vec();
    while body.len() < length {
        match socket.read(&mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(n) => body.extend_from_slice(&buffer[..n]),
        }
    }
    if let Ok(mut log) = log.lock() {
        log.push((line.clone(), body));
    }
    let json = |status: &str, body: &str| {
        format!(
            "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
            body.len()
        )
    };
    let response = if line.starts_with("GET /api/event") {
        let frames = [
            r#"{"type":"server.connected","data":{}}"#,
            r#"{"type":"session.execution.started","data":{"sessionID":"ses_child"}}"#,
            r#"{"type":"permission.asked","data":{"id":"per_1","sessionID":"ses_child","action":"shell","resources":["echo hi"]}}"#,
        ];
        let mut stream = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n".to_owned();
        for frame in frames {
            let frame = format!("data: {frame}\n\n");
            stream.push_str(&format!("{:x}\r\n{frame}\r\n", frame.len()));
        }
        stream
    } else if line.starts_with("POST /api/session ") {
        json("200 OK", r#"{"data":{"id":"ses_x"}}"#)
    } else if line.starts_with("GET /api/session/active") {
        json("200 OK", r#"{"data":{}}"#)
    } else if line.starts_with("GET /api/permission/request") {
        json("200 OK", r#"{"data":[]}"#)
    } else if line.starts_with("POST /api/session/ses_child/permission/per_1/reply") {
        let _ = std::fs::write(dir.join("replied"), "");
        "HTTP/1.1 204 No Content\r\n\r\n".to_owned()
    } else {
        "HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\r\n".to_owned()
    };
    let _ = socket.write_all(response.as_bytes()).await;
    let _ = socket.read(&mut buffer).await;
}

/// A fake `opencode` whose server is at `url` and whose TUI waits for the
/// reply to be posted (or 20 s) and exits 0.
fn fake_opencode(dir: &Path, url: &str) -> PathBuf {
    let out = dir.display();
    let script = format!(
        r#"#!/bin/sh
case "$1" in
  --version) echo 'opencode v2.3.0' ;;
  serve) echo 'server listening on {url}'; exec sleep 600 ;;
  *) i=0; while [ ! -e '{out}/replied' ] && [ $i -lt 200 ]; do sleep 0.1; i=$((i+1)); done ;;
esac
"#
    );
    let path = dir.join("opencode");
    std::fs::write(&path, script).expect("write the fake");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    path
}

#[tokio::test(flavor = "multi_thread")]
async fn an_ask_goes_to_the_inbox_and_the_answer_back_to_opencode() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let (url, seen) = fake_server(dir.path().to_path_buf()).await;
    let program = fake_opencode(dir.path(), &url);
    let state = dir.path().join("state").join("opencode.json");
    let daemon = FakeDaemon::new();
    daemon.stream(
        rpc::session::HOOK,
        &[
            &pb::HookReply {
                keepalive: true,
                stdout: Vec::new(),
            },
            &pb::HookReply {
                keepalive: false,
                stdout: ALLOW.to_vec(),
            },
        ],
    );
    let console = FakeConsole::terminal();
    let program = program.to_string_lossy().into_owned();
    let state = state.to_string_lossy().into_owned();
    let words = [
        "opencode",
        "--conversation",
        "0f8a1c2e-7b3d-4e5f-9a6b-1c2d3e4f5a6b",
        "--state",
        state.as_str(),
        "--program",
        program.as_str(),
    ];
    assert_eq!(run(&daemon, &console, &words).await, exit::OK);

    let hook: pb::HookRequest = daemon.only(rpc::session::HOOK);
    assert_eq!(
        (hook.agent.as_str(), hook.event.as_str()),
        ("opencode", "permission")
    );
    let payload: serde_json::Value = serde_json::from_slice(&hook.payload).expect("JSON");
    assert_eq!(payload["id"], "per_1");
    let turns: Vec<pb::ReportTurnRequest> = daemon.sent(rpc::session::REPORT_TURN);
    assert!(
        turns
            .iter()
            .any(|turn| turn.state == pb::TurnState::NeedsInput as i32),
        "{turns:?}"
    );
    let posted = seen.lock().map(|seen| seen.clone()).unwrap_or_default();
    let reply = posted
        .iter()
        .find(|(line, _)| line.contains("/permission/per_1/reply"))
        .expect("the answer was posted to OpenCode");
    assert_eq!(reply.1, ALLOW);
}
