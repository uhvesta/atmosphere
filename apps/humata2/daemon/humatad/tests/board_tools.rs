//! The board tools of `humatad2 mcp` (BD-1..BD-16, D155): the agent of a
//! session makes, lists, changes, moves, reads the history of and deletes
//! items on its session board; the Global board asks the user first.

mod common;

use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use common::{Attach, Daemon, WAIT, binary, folder, shell_token, state_folder};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{ChannelRecord, SessionRecord, Store};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tonic::transport::Channel;

const SESSION: &str = "se_board";
const OTHER: &str = "se_board_other";
const CHANNEL: &str = "ch_board";

struct Fixture {
    daemon: Daemon,
    token: String,
    _attach: Attach,
}

impl Fixture {
    async fn start() -> Self {
        let dir = folder();
        let store = Store::open(state_folder(dir.path()).join("store")).unwrap();
        store
            .put_channel(&ChannelRecord {
                id: CHANNEL.to_owned(),
                name: "Payments".to_owned(),
                folder: "payments".to_owned(),
                branch_prefix: None,
                default_agent: "zsh".to_owned(),
                repos: Vec::new(),
                machines: Vec::new(),
                revision: 1,
                created_unix_ms: 0,
            })
            .unwrap();
        for (id, title) in [(SESSION, "board"), (OTHER, "other")] {
            store
                .put_session(&SessionRecord {
                    id: id.to_owned(),
                    channel: CHANNEL.to_owned(),
                    machine: String::new(),
                    title: title.to_owned(),
                    agent: "zsh".to_owned(),
                    folder: dir.path().to_owned(),
                    created_unix_ms: 0,
                })
                .unwrap();
        }
        drop(store);
        let daemon = Daemon::start_in(dir).await;
        let (token, attach) = shell_token(&daemon, &pb::SessionId { value: SESSION.to_owned() }).await;
        Self { daemon, token, _attach: attach }
    }

    async fn mcp(&self) -> Mcp {
        Mcp::start(&self.daemon.socket, &self.token).await
    }

    async fn boards(&self) -> pb::board_service_client::BoardServiceClient<Channel> {
        pb::board_service_client::BoardServiceClient::new(self.daemon.channel().await)
    }

    /// What the user sees on a board.
    async fn user_sees(&self, board: pb::board_ref::Board) -> Vec<pb::BoardItem> {
        self.boards()
            .await
            .list_items(pb::ListItemsRequest {
                view: Some(pb::BoardView {
                    view: Some(pb::board_view::View::Board(pb::BoardRef { board: Some(board) })),
                }),
                columns: Vec::new(),
                page: None,
            })
            .await
            .unwrap()
            .into_inner()
            .items
    }
}

/// One `humatad2 mcp` process, spoken to as an agent's client would.
struct Mcp {
    _child: Child,
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
    next_id: u64,
}

impl Mcp {
    async fn start(socket: &Path, token: &str) -> Self {
        let mut child = Command::new(binary("HUMATAD2"))
            .arg("mcp")
            .arg("--socket")
            .arg(socket)
            .env("HUMATA_SESSION_TOKEN", token)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap()).lines();
        let mut mcp = Self { _child: child, stdin, stdout, next_id: 0 };
        let id = mcp
            .begin(
                "initialize",
                json!({
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": { "name": "test", "version": "0" },
                }),
            )
            .await;
        mcp.finish(id).await;
        mcp.send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).await;
        mcp
    }

    async fn send(&mut self, message: Value) {
        let mut line = message.to_string();
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).await.unwrap();
        self.stdin.flush().await.unwrap();
    }

    async fn begin(&mut self, method: &str, params: Value) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })).await;
        id
    }

    /// The response to request `id`, skipping notifications.
    async fn finish(&mut self, id: u64) -> Value {
        loop {
            let line = tokio::time::timeout(WAIT, self.stdout.next_line())
                .await
                .expect("humatad2 mcp went quiet")
                .unwrap()
                .expect("humatad2 mcp closed its output");
            let message: Value = serde_json::from_str(&line).unwrap();
            if message.get("method").is_none() && message["id"] == json!(id) {
                assert!(message.get("error").is_none(), "{message}");
                return message["result"].clone();
            }
        }
    }

    async fn call(&mut self, tool: &str, arguments: Value) -> Value {
        let id = self
            .begin("tools/call", json!({ "name": tool, "arguments": arguments }))
            .await;
        self.finish(id).await
    }
}

/// What a successful tool call returned.
fn ok(result: &Value) -> &Value {
    assert_eq!(result["isError"], false, "{result}");
    &result["structuredContent"]["result"]
}

fn error_kind(result: &Value) -> &str {
    assert_eq!(result["isError"], true, "{result}");
    result["structuredContent"]["error"]["kind"].as_str().unwrap_or_default()
}

fn id_of(item: &Value) -> String {
    item["id"].as_str().unwrap().to_owned()
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_works_its_session_board_from_making_an_item_to_deleting_it() {
    let fixture = Fixture::start().await;
    let mut mcp = fixture.mcp().await;

    let made = mcp
        .call("board_item_create", json!({ "board": "session", "title": "Refund path", "column": "to_do" }))
        .await;
    let made = ok(&made).clone();
    let first = id_of(&made);
    assert_eq!(made["column"], "to_do");
    assert_eq!(made["title"], "Refund path");
    assert_eq!(made["sessions"], json!([]));
    let second = id_of(ok(
        &mcp.call("board_item_create", json!({ "board": "session", "title": "Webhooks", "column": "backlog" })).await,
    ));
    assert_ne!(first, second);

    let listed = mcp.call("board_items_list", json!({ "board": "session" })).await;
    let ids: Vec<String> = ok(&listed)["items"].as_array().unwrap().iter().map(id_of).collect();
    assert!(ids.contains(&first) && ids.contains(&second), "{listed}");
    let only = mcp.call("board_items_list", json!({ "board": "session", "columns": ["to_do"] })).await;
    let in_column: Vec<String> = ok(&only)["items"].as_array().unwrap().iter().map(id_of).collect();
    assert_eq!(in_column, vec![first.clone()]);

    // Sessions and dependencies go in and come out as lists.
    let updated = mcp
        .call(
            "board_item_update",
            json!({
                "id": first,
                "description": "refunds over the limit",
                "sessions_add": [SESSION, OTHER],
                "depends_on_add": [second],
            }),
        )
        .await;
    let updated = ok(&updated);
    assert_eq!(updated["description"], "refunds over the limit");
    assert_eq!(updated["sessions"], json!([SESSION, OTHER]));
    assert_eq!(updated["depends_on"][0]["id"], second);
    assert_eq!(updated["depends_on"][0]["done"], false);
    let trimmed = mcp
        .call("board_item_update", json!({ "id": first, "sessions_remove": [OTHER] }))
        .await;
    assert_eq!(ok(&trimmed)["sessions"], json!([SESSION]));
    assert_eq!(ok(&trimmed)["depends_on"].as_array().unwrap().len(), 1);

    // The dependency done shows as done, with when.
    let moved = mcp.call("board_item_move", json!({ "id": second, "column": "done" })).await;
    assert_eq!(ok(&moved)["column"], "done");
    assert!(ok(&moved)["done_at"].is_string(), "{moved}");
    let got = mcp.call("board_item_get", json!({ "id": first })).await;
    assert_eq!(ok(&got)["depends_on"][0]["done"], true);

    let history = mcp.call("board_item_history", json!({ "id": first })).await;
    let entries = ok(&history)["entries"].as_array().unwrap();
    assert!(entries.len() >= 3, "made, changed, and sessions changed: {history}");
    assert_eq!(entries[0]["kind"], "created");
    assert!(entries.iter().any(|entry| entry["kind"] == "sessions"), "{history}");
    assert!(entries.iter().all(|entry| entry["lost_conflict"] == false));

    let deleted = mcp.call("board_item_delete", json!({ "id": first })).await;
    assert_eq!(ok(&deleted)["deleted"], true);
    let gone = mcp.call("board_item_get", json!({ "id": first })).await;
    assert_eq!(error_kind(&gone), "not_found");
    let left = fixture
        .user_sees(pb::board_ref::Board::Session(pb::SessionId { value: SESSION.to_owned() }))
        .await;
    assert_eq!(left.len(), 1);
}

/// VF-21: an item made with no board and no column is on the session
/// board, in To do: the same place `humata board add` puts it.
#[tokio::test(flavor = "multi_thread")]
async fn an_item_made_with_no_board_or_column_lands_on_the_session_board_in_to_do() {
    let fixture = Fixture::start().await;
    let mut mcp = fixture.mcp().await;
    let made = ok(&mcp.call("board_item_create", json!({ "title": "Plain" })).await).clone();
    assert_eq!(made["board"], json!({ "session": SESSION }));
    assert_eq!(made["column"], "to_do");
    let on_board = fixture
        .user_sees(pb::board_ref::Board::Session(pb::SessionId { value: SESSION.to_owned() }))
        .await;
    assert_eq!(on_board.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dependency_that_would_close_a_loop_is_refused() {
    let fixture = Fixture::start().await;
    let mut mcp = fixture.mcp().await;
    let a = id_of(ok(&mcp.call("board_item_create", json!({ "board": "session", "title": "A" })).await));
    let b = id_of(ok(
        &mcp.call("board_item_create", json!({ "board": "session", "title": "B", "depends_on": [a.clone()] }))
            .await,
    ));

    let looped = mcp.call("board_item_update", json!({ "id": a, "depends_on_add": [b] })).await;
    assert_eq!(error_kind(&looped), "dependency_loop");
    let itself = mcp.call("board_item_update", json!({ "id": a, "depends_on_add": [a] })).await;
    assert_eq!(error_kind(&itself), "dependency_loop");

    let unchanged = mcp.call("board_item_get", json!({ "id": a })).await;
    assert_eq!(ok(&unchanged)["depends_on"], json!([]));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stale_revision_conflicts_only_on_a_field_that_changed() {
    let fixture = Fixture::start().await;
    let mut mcp = fixture.mcp().await;
    let made = mcp.call("board_item_create", json!({ "board": "session", "title": "Old" })).await;
    let made = ok(&made).clone();
    let id = id_of(&made);
    let revision = made["revision"].as_u64().unwrap();
    mcp.call("board_item_update", json!({ "id": id, "title": "New" })).await;

    let stale = mcp
        .call("board_item_update", json!({ "id": id, "title": "Mine", "expected_revision": revision }))
        .await;
    assert_eq!(error_kind(&stale), "revision_conflict");
    // Another field than the one that changed: no conflict.
    let fine = mcp
        .call(
            "board_item_update",
            json!({ "id": id, "description": "ok", "expected_revision": revision }),
        )
        .await;
    assert_eq!(ok(&fine)["description"], "ok");
}

#[tokio::test(flavor = "multi_thread")]
async fn creating_on_the_global_board_waits_for_the_user() {
    let fixture = Fixture::start().await;
    let mut mcp = fixture.mcp().await;
    let mut inbox =
        pb::inbox_service_client::InboxServiceClient::new(fixture.daemon.channel().await);

    let call = mcp
        .begin(
            "tools/call",
            json!({
                "name": "board_item_create",
                "arguments": { "board": "global", "title": "Rotate keys", "reason": "found while testing" },
            }),
        )
        .await;
    let deadline = Instant::now() + WAIT;
    let request = loop {
        let pending = inbox
            .list_requests(pb::ListRequestsRequest::default())
            .await
            .unwrap()
            .into_inner()
            .requests;
        if let Some(id) = pending.into_iter().find_map(|request| request.id) {
            break id;
        }
        assert!(Instant::now() < deadline, "the create never reached the Inbox");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert!(
        fixture.user_sees(pb::board_ref::Board::Global(pb::GlobalBoard {})).await.is_empty(),
        "nothing lands before the user answers"
    );

    inbox
        .answer_request(pb::AnswerRequestRequest {
            id: Some(request),
            answer: Some(pb::RequestAnswer {
                answer: Some(pb::request_answer::Answer::Deny(pb::Deny { note: "not now".to_owned() })),
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    let denied = mcp.finish(call).await;
    assert_eq!(denied["isError"], true, "{denied}");
    assert!(fixture.user_sees(pb::board_ref::Board::Global(pb::GlobalBoard {})).await.is_empty());
}
