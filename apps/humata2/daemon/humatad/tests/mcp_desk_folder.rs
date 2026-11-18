//! D382: a Desk session's agent reads its own folder through the MCP files
//! tools, with `root: "session_folder"`. Another session's agent is refused
//! by the policy, as it would be for any session's clone.

mod common;

use std::path::PathBuf;
use std::process::Stdio;

use common::{Build, Daemon, World, WAIT, folder, shell_token, wait_status};
use humata2_grpc::humata::v1 as pb;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

fn binary(variable: &str) -> PathBuf {
    let path = std::env::var_os(variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("set {variable} to the binary's path"));
    std::fs::canonicalize(path).expect("the binary exists")
}

async fn daemon() -> Daemon {
    let dir = folder();
    let world = World::new(dir.path());
    let args = world.args();
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    Daemon::start_with_env(Build::Admin, dir, &refs, &[]).await
}

/// Starts a plain Desk session and waits until its agent is up; returns it
/// and its folder.
async fn desk_session(daemon: &Daemon, title: &str) -> (pb::SessionId, PathBuf) {
    let started = pb::desk_service_client::DeskServiceClient::new(daemon.channel().await)
        .start_desk_session(pb::StartDeskSessionRequest {
            title: title.to_owned(),
            access: Some(pb::DeskAccess {
                boards: true,
                channels_and_machines: false,
            }),
            ..Default::default()
        })
        .await
        .expect("start a desk session")
        .into_inner();
    let id = started.session.expect("the session id");
    wait_status(daemon, &id, pb::SessionStatus::Working).await;
    let path = started.desk.and_then(|desk| desk.folder).expect("its folder").path;
    (id, PathBuf::from(path))
}

/// One `humatad2 mcp` process, spoken to as an agent's client would.
struct Mcp {
    _child: Child,
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
}

impl Mcp {
    async fn start(daemon: &Daemon, token: &str) -> Self {
        let mut child = Command::new(binary("HUMATAD2"))
            .arg("mcp")
            .arg("--socket")
            .arg(&daemon.socket)
            .env("HUMATA_SESSION_TOKEN", token)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .expect("start humatad2 mcp");
        let stdin = child.stdin.take().expect("a stdin");
        let stdout = BufReader::new(child.stdout.take().expect("a stdout")).lines();
        let mut mcp = Self { _child: child, stdin, stdout };
        mcp.request(
            0,
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "test", "version": "0" },
            }),
        )
        .await;
        mcp.send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).await;
        mcp
    }

    async fn send(&mut self, message: Value) {
        let mut line = message.to_string();
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).await.expect("write to mcp");
        self.stdin.flush().await.expect("flush to mcp");
    }

    async fn request(&mut self, id: u64, method: &str, params: Value) -> Value {
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
            .await;
        loop {
            let line = tokio::time::timeout(WAIT, self.stdout.next_line())
                .await
                .expect("humatad2 mcp went quiet")
                .expect("read from mcp")
                .expect("humatad2 mcp closed its output");
            let message: Value = serde_json::from_str(&line).expect("mcp speaks JSON");
            if message.get("method").is_none() && message["id"] == json!(id) {
                assert!(message.get("error").is_none(), "{message}");
                return message["result"].clone();
            }
        }
    }

    async fn call(&mut self, id: u64, tool: &str, arguments: Value) -> Value {
        self.request(id, "tools/call", json!({ "name": tool, "arguments": arguments }))
            .await
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_desk_sessions_agent_reads_and_lists_its_own_folder_and_another_agent_is_refused() {
    let daemon = daemon().await;
    let (mine, folder) = desk_session(&daemon, "Mine").await;
    let (other, _) = desk_session(&daemon, "Other").await;
    std::fs::write(folder.join("rfc.md"), "the document").expect("write the document");
    let (mine_token, _mine_shell) = shell_token(&daemon, &mine).await;
    let (other_token, _other_shell) = shell_token(&daemon, &other).await;
    let location = json!({ "root": "session_folder", "path": "rfc.md" });
    let named = json!({ "root": "session_folder", "session": mine.value, "path": "rfc.md" });
    let mut owner = Mcp::start(&daemon, &mine_token).await;
    let mut stranger = Mcp::start(&daemon, &other_token).await;

    // The default is the caller's own session.
    let own = owner.call(1, "file_read", json!({ "location": location })).await;
    let listed = owner
        .call(
            2,
            "files_list",
            json!({ "location": { "root": "session_folder", "path": "" } }),
        )
        .await;
    let refused = stranger.call(1, "file_read", json!({ "location": named })).await;

    assert_eq!(own["structuredContent"]["result"]["text"], "the document", "{own}");
    assert!(listed["structuredContent"].to_string().contains("rfc.md"), "{listed}");
    assert_eq!(refused["isError"], true, "{refused}");
    assert_eq!(refused["structuredContent"]["error"]["kind"], "denied", "{refused}");
}
