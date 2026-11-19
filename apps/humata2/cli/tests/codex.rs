//! `humata codex` (D795) with a fake `codex` and a fake daemon: the
//! app-server's thread is the conversation, its turns are reported, an
//! approval goes to the Inbox as the session's agent and the Inbox's answer
//! reaches the app-server, and the command exits as the TUI does.

mod support;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use humata2_cli::rpc;
use humata2_proto::humata::v1 as pb;
use serde_json::{Value, json};
use support::{FakeConsole, FakeDaemon, run};
use tokio::net::UnixListener;
use tokio_tungstenite::tungstenite::Message;

const THREAD: &str = "019b2c4d-7e8f-7a01-b234-56789abcdef0";
const ALLOW: &[u8] =
    br#"{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}}"#;
const WAIT: Duration = Duration::from_secs(20);

/// A fake `codex`: `app-server` logs its arguments and waits; the TUI waits
/// for `answered` in `dir` (or 20 s) and exits 7.
fn fake_codex(dir: &Path) -> PathBuf {
    let script = format!(
        "#!/bin/sh\n\
         if [ \"$1\" = app-server ]; then echo \"$*\" > '{dir}/serve'; exec sleep 600; fi\n\
         i=0; while [ ! -e '{dir}/answered' ] && [ $i -lt 200 ]; do sleep 0.1; i=$((i+1)); done\n\
         exit 7\n",
        dir = dir.display(),
    );
    let path = dir.join("codex");
    std::fs::write(&path, script).expect("write the fake");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    path
}

/// Plays the app-server on the socket the fake `codex` was told to listen
/// on: names the thread, runs turns, asks for one approval once `go` exists
/// and records its answer in `answered`.
async fn app_server(dir: PathBuf) {
    let line = tokio::time::timeout(WAIT, async {
        loop {
            let text = std::fs::read_to_string(dir.join("serve")).unwrap_or_default();
            if text.contains("unix://") {
                return text;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the app-server was started");
    let at = line.find("unix://").expect("a unix socket") + "unix://".len();
    let socket = line[at..].split_whitespace().next().expect("a path").to_owned();
    let listener = UnixListener::bind(&socket).expect("bind where the wrapper said");
    let (stream, _) = listener.accept().await.expect("the link connects");
    let mut ws = tokio_tungstenite::accept_async(stream).await.expect("a handshake");
    let turn = |method: &str, status: &str| {
        json!({"jsonrpc":"2.0","method":method,"params":{"threadId":THREAD,
            "turn":{"id":"7","status":status,"items":[]}}})
    };
    let approval = json!({"jsonrpc":"2.0","id":41,"method":"item/commandExecution/requestApproval",
        "params":{"threadId":THREAD,"turnId":"7","itemId":"i1","startedAtMs":1,
        "command":"cargo publish","cwd":"/work","reason":"Check"}});
    while let Some(Ok(message)) = ws.next().await {
        let Message::Text(text) = message else { continue };
        let frame: Value = serde_json::from_str(&text).expect("JSON");
        if frame["method"].is_null() && frame["id"] == 41 {
            let _ = std::fs::write(dir.join("answered"), frame.to_string());
            continue;
        }
        let result = match frame["method"].as_str() {
            Some("initialize") => json!({"userAgent": "codex/0.169.0"}),
            Some("thread/loaded/list") => json!({"data": [THREAD], "nextCursor": null}),
            Some("thread/resume") => json!({"thread": {"id": THREAD}}),
            _ => continue,
        };
        let reply = json!({"jsonrpc":"2.0","id":frame["id"],"result":result});
        let _ = ws.send(Message::text(reply.to_string())).await;
        if frame["method"] == "thread/resume" {
            for message in [
                turn("turn/started", "inProgress"),
                turn("turn/completed", "completed"),
                turn("turn/started", "inProgress"),
                turn("turn/completed", "interrupted"),
            ] {
                let _ = ws.send(Message::text(message.to_string())).await;
            }
            while !dir.join("go").exists() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            let _ = ws.send(Message::text(approval.to_string())).await;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_thread_turns_and_an_approval_reach_humata_and_the_answer_goes_back() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let program = fake_codex(dir.path());
    tokio::spawn(app_server(dir.path().to_path_buf()));
    let daemon = FakeDaemon::new();
    let console = FakeConsole::terminal();
    let program = program.to_string_lossy().into_owned();
    let words = [
        "codex",
        "--program",
        program.as_str(),
        "--hooks",
        "{ Stop = [] }",
        "--",
        "--model",
        "m",
    ];
    // The fake daemon's stream is spent by the first hook call, which names
    // the thread; the approval's answer is queued once that has been sent.
    let arm = async {
        while daemon.sent::<pb::HookRequest>(rpc::session::HOOK).is_empty() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        daemon.stream(
            rpc::session::HOOK,
            &[&pb::HookReply {
                keepalive: false,
                stdout: ALLOW.to_vec(),
            }],
        );
        let _ = std::fs::write(dir.path().join("go"), "");
    };
    let (code, ()) = tokio::join!(run(&daemon, &console, &words), arm);
    assert_eq!(code, 7);

    let hooks: Vec<pb::HookRequest> = daemon.sent(rpc::session::HOOK);
    let find = |event: &str| hooks.iter().find(|hook| hook.event == event);
    let conversation = find("conversation").expect("the thread was named");
    assert_eq!(conversation.agent, "codex");
    let payload: Value = serde_json::from_slice(&conversation.payload).expect("JSON");
    assert_eq!(payload["session_id"], THREAD);
    let permission = find("permission").expect("the approval was raised");
    assert_eq!(permission.agent, "codex");

    let states: Vec<i32> = daemon
        .sent::<pb::ReportTurnRequest>(rpc::session::REPORT_TURN)
        .iter()
        .map(|turn| turn.state)
        .collect();
    let (working, idle) = (pb::TurnState::Working as i32, pb::TurnState::Idle as i32);
    assert_eq!(states, [working, idle, working, idle]);

    let answered = std::fs::read_to_string(dir.path().join("answered")).expect("an answer");
    let answered: Value = serde_json::from_str(&answered).expect("JSON");
    assert_eq!(answered["id"], 41);
    assert_eq!(answered["result"], json!({"decision": "accept"}));
}
