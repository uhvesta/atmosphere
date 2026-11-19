//! The `humata codex` wrapper (D795) with a fake `codex`: it runs Codex's
//! app-server on a private socket, attaches the TUI to it with `--remote`,
//! links the two to Humata, passes the TUI's exit status through and ends
//! the app-server with the TUI. When the app-server can't be had, it runs
//! plain Codex with Humata's hooks instead.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use humata2_codex::{AskKind, Humata, HumataError, Turn, Wrapper, WrapperError, run_wrapper};
use serde_json::{Value, json};
use tokio::net::UnixListener;
use tokio_tungstenite::tungstenite::Message;

const THREAD: &str = "019b2c4d-7e8f-7a01-b234-56789abcdef0";
const HOOKS: &str = "{ Stop = [] }";
const WAIT: Duration = Duration::from_secs(20);

/// A Humata that hears the conversation and turns, and never decides.
#[derive(Default)]
struct Quiet {
    conversations: Mutex<Vec<String>>,
}

impl Humata for Quiet {
    async fn turn(&self, _turn: Turn) {}

    async fn conversation(&self, thread: String) {
        if let Ok(mut conversations) = self.conversations.lock() {
            conversations.push(thread);
        }
    }

    async fn ask(&self, _kind: AskKind, _payload: Vec<u8>) -> Result<Vec<u8>, HumataError> {
        Ok(Vec::new())
    }
}

/// A fake `codex`. Each run appends its arguments, one line, to `runs`.
/// `app-server` writes its pid to `serve.pid` and then runs until killed,
/// or exits 3 at once when `serve_fails`; the TUI waits for `quit` in
/// `dir` and exits 7.
fn fake_codex(dir: &Path, serve_fails: bool) -> PathBuf {
    let program = dir.join("codex");
    let serve = if serve_fails {
        "exit 3".to_owned()
    } else {
        format!("echo $$ > '{}/serve.pid'; exec sleep 600", dir.display())
    };
    let script = format!(
        "#!/bin/sh\n\
         echo \"$*\" >> '{dir}/runs'\n\
         if [ \"$1\" = app-server ]; then {serve}; fi\n\
         while [ ! -e '{dir}/quit' ]; do sleep 0.05; done\n\
         exit 7\n",
        dir = dir.display(),
    );
    std::fs::write(&program, script).expect("write the fake codex");
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755))
        .expect("make it runnable");
    program
}

fn wrapper(program: PathBuf, dir: &Path, socket_root: &Path) -> Wrapper {
    Wrapper {
        program,
        hooks: Some(HOOKS.to_owned()),
        args: [
            "-c",
            "mcp_servers.humata={ command = \"humata\" }",
            "--model",
            "gpt-5-mini",
        ]
        .map(Into::into)
        .into(),
        folder: dir.to_path_buf(),
        socket_root: socket_root.to_path_buf(),
        retry: Duration::from_millis(100),
        wait_for_socket: Duration::from_secs(5),
    }
}

/// The lines the fake codex logged, once some line satisfies `found`.
async fn runs_until(dir: &Path, found: impl Fn(&str) -> bool) -> Vec<String> {
    let runs = dir.join("runs");
    tokio::time::timeout(WAIT, async {
        loop {
            let text = std::fs::read_to_string(&runs).unwrap_or_default();
            if text.lines().any(&found) {
                return text.lines().map(str::to_owned).collect();
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the fake codex ran as expected")
}

/// The socket path an `app-server --listen unix://PATH` run was given.
fn listen_path(line: &str) -> PathBuf {
    let at = line.find("unix://").expect("the app-server listens on a unix socket");
    let rest = &line[at + "unix://".len()..];
    PathBuf::from(rest.split_whitespace().next().expect("a path"))
}

fn alive(pid: &str) -> bool {
    std::process::Command::new("kill")
        .args(["-0", pid.trim()])
        .status()
        .is_ok_and(|status| status.success())
}

/// Plays the app-server on `socket` for one connection: answers the link's
/// hello and its look for the loaded threads with `THREAD`.
async fn serve_one(socket: PathBuf) {
    let listener = UnixListener::bind(&socket).expect("bind where the wrapper said");
    let (stream, _) = listener.accept().await.expect("the link connects");
    let mut ws = tokio_tungstenite::accept_async(stream).await.expect("a websocket handshake");
    loop {
        let Some(Ok(message)) = ws.next().await else {
            return;
        };
        let Message::Text(text) = message else {
            continue;
        };
        let frame: Value = serde_json::from_str(&text).expect("the link sends JSON");
        let result = match frame["method"].as_str() {
            Some("initialize") => json!({"userAgent": "codex/0.169.0"}),
            Some("thread/loaded/list") => json!({"data": [THREAD], "nextCursor": null}),
            Some("thread/resume") => json!({"thread": {"id": THREAD}}),
            _ => continue,
        };
        let reply = json!({"jsonrpc": "2.0", "id": frame["id"], "result": result});
        if ws.send(Message::text(reply.to_string())).await.is_err() {
            return;
        }
    }
}

#[tokio::test]
async fn the_tui_runs_on_a_private_app_server_and_its_thread_is_named() {
    let dir = tempfile::tempdir().expect("a folder");
    let root = tempfile::tempdir().expect("a socket root");
    let program = fake_codex(dir.path(), false);
    let humata = Arc::new(Quiet::default());
    let ran = tokio::spawn(run_wrapper(wrapper(program, dir.path(), root.path()), Arc::clone(&humata)));

    let runs = runs_until(dir.path(), |line| line.starts_with("app-server")).await;
    let serve = runs.iter().find(|line| line.starts_with("app-server")).expect("an app-server run");
    let socket = listen_path(serve);
    assert!(socket.starts_with(root.path()), "the socket is under the root it was given");
    let folder = socket.parent().expect("a folder for the socket");
    let mode = std::fs::metadata(folder).expect("the folder exists").permissions().mode();
    assert_eq!(mode & 0o777, 0o700, "only the owner reaches the socket");
    assert!(serve.contains("-c mcp_servers.humata="), "the app-server gets the config overrides");
    assert!(!serve.contains("--model"), "the TUI picks the model, not the app-server");
    assert!(!serve.contains("hooks="), "hooks would double the app-server's asks");

    tokio::spawn(serve_one(socket.clone()));
    let runs = runs_until(dir.path(), |line| line.starts_with("--remote")).await;
    let tui = runs.iter().find(|line| line.starts_with("--remote")).expect("a TUI run");
    assert!(tui.starts_with(&format!("--remote unix://{}", socket.display())));
    assert!(tui.contains("--model gpt-5-mini"), "the TUI gets the session's args");
    assert!(!tui.contains("hooks="));

    tokio::time::timeout(WAIT, async {
        while humata.conversations.lock().map(|c| c.is_empty()).unwrap_or(true) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the thread is named");
    assert_eq!(humata.conversations.lock().expect("lock")[0], THREAD);

    std::fs::write(dir.path().join("quit"), "").expect("quit the TUI");
    let code = tokio::time::timeout(WAIT, ran)
        .await
        .expect("the wrapper ends with the TUI")
        .expect("the wrapper task runs")
        .expect("the wrapper runs");
    assert_eq!(code, 7, "the TUI's exit status is the wrapper's");
    let pid = std::fs::read_to_string(dir.path().join("serve.pid")).expect("the app-server's pid");
    assert!(!alive(&pid), "the app-server ends with the TUI");
    assert!(!folder.exists(), "the socket's folder is cleaned up");
}

#[tokio::test]
async fn without_an_app_server_plain_codex_runs_with_humatas_hooks() {
    let dir = tempfile::tempdir().expect("a folder");
    let root = tempfile::tempdir().expect("a socket root");
    let program = fake_codex(dir.path(), true);
    let humata = Arc::new(Quiet::default());
    let ran = tokio::spawn(run_wrapper(wrapper(program, dir.path(), root.path()), Arc::clone(&humata)));

    let runs = runs_until(dir.path(), |line| line.starts_with("-c hooks=")).await;
    let plain = runs.iter().find(|line| line.starts_with("-c hooks=")).expect("a plain run");
    assert!(plain.starts_with(&format!("-c hooks={HOOKS}")));
    assert!(plain.contains("--model gpt-5-mini"), "plain Codex gets the session's args");
    assert!(!plain.contains("--remote"));

    std::fs::write(dir.path().join("quit"), "").expect("quit the TUI");
    let code = tokio::time::timeout(WAIT, ran)
        .await
        .expect("the wrapper ends with codex")
        .expect("the wrapper task runs")
        .expect("the wrapper runs");
    assert_eq!(code, 7);
    assert!(humata.conversations.lock().expect("lock").is_empty());
    let left: Vec<_> = std::fs::read_dir(root.path()).expect("the root").collect();
    assert!(left.is_empty(), "no socket folder is left behind");
}

#[tokio::test]
async fn a_codex_not_on_path_says_so() {
    let dir = tempfile::tempdir().expect("a folder");
    let root = tempfile::tempdir().expect("a socket root");
    let missing = dir.path().join("no-codex-here");
    let ran = run_wrapper(wrapper(missing, dir.path(), root.path()), Arc::new(Quiet::default())).await;
    assert!(matches!(ran, Err(WrapperError::NotInstalled { .. })), "got {ran:?}");
}
