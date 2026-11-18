//! `humatad2 mcp` against the real daemon: a session tab's token makes its
//! agent, the daemon checks what that agent may do, and an Inbox question
//! the agent gives up on is withdrawn.

mod common;

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use common::{Attach, Daemon, WAIT, error_of, folder, open, state_folder};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{ChannelRecord, SessionRecord, Store};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tonic::metadata::MetadataValue;
use tonic::{Code, Request};

const SESSION: &str = "se_mcp";
/// Another session of the same channel.
const OTHER: &str = "se_other";
const TOKEN_HEADER: &str = "humata-session-token";
/// 32 hex characters, as a token is, that the daemon never gave out.
const FORGED: &str = "0123456789abcdef0123456789abcdef";

fn binary(variable: &str) -> PathBuf {
    let path = std::env::var_os(variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("set {variable} to the binary's path"));
    std::fs::canonicalize(path).unwrap()
}

/// A daemon that knows `SESSION`, with one shell tab open in it, and the
/// token that tab's shell was given.
struct Fixture {
    daemon: Daemon,
    tab: pb::TabId,
    token: String,
    _attach: Attach,
}

impl Fixture {
    async fn start() -> Self {
        let dir = folder();
        let store = Store::open(state_folder(dir.path()).join("store")).unwrap();
        store
            .put_channel(&ChannelRecord {
                id: "ch_test".to_owned(),
                name: "Test".to_owned(),
                folder: "test".to_owned(),
                branch_prefix: None,
                default_agent: "zsh".to_owned(),
                repos: Vec::new(),
                machines: Vec::new(),
                revision: 1,
                created_unix_ms: 0,
            })
            .unwrap();
        for (id, title) in [(SESSION, "mcp"), (OTHER, "other")] {
            store
                .put_session(&SessionRecord {
                    id: id.to_owned(),
                    channel: "ch_test".to_owned(),
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

        let tab = daemon
            .terminals()
            .await
            .open_shell_tab(pb::OpenShellTabRequest {
                idempotency_key: None,
                session: Some(session()),
                repo: None,
                folder: String::new(),
            })
            .await
            .unwrap()
            .into_inner()
            .id
            .unwrap();

        let mut attach = Attach::start(&daemon, open(&tab, 0)).await;
        let file = daemon.dir.path().join("token");
        attach
            .type_text(&format!(
                "printf '%s' \"$HUMATA_SESSION_TOKEN\" > {}\r",
                file.display()
            ))
            .await;
        let token = read_when_written(&file).await;
        assert_eq!(token.len(), 32, "the tab's shell has a session token");

        Self {
            daemon,
            tab,
            token,
            _attach: attach,
        }
    }

    async fn mcp(&self, token: &str) -> Mcp {
        Mcp::start(&self.daemon.socket, token).await
    }
}

fn session() -> pb::SessionId {
    pb::SessionId {
        value: SESSION.to_owned(),
    }
}

/// `path`'s contents once something non-empty is written there.
async fn read_when_written(path: &Path) -> String {
    let deadline = Instant::now() + WAIT;
    loop {
        match std::fs::read_to_string(path) {
            Ok(text) if !text.is_empty() => return text,
            _ => {}
        }
        assert!(
            Instant::now() < deadline,
            "{} was never written",
            path.display()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// `message` sent as the agent holding `token`.
fn as_agent<T>(message: T, token: &str) -> Request<T> {
    let mut request = Request::new(message);
    request
        .metadata_mut()
        .insert(TOKEN_HEADER, MetadataValue::try_from(token).unwrap());
    request
}

/// One `humatad2 mcp` process, spoken to as an agent's client would.
struct Mcp {
    _child: Child,
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
    /// Notifications read while waiting for a response.
    notifications: Vec<Value>,
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
        let mut mcp = Self {
            _child: child,
            stdin,
            stdout,
            notifications: Vec::new(),
        };
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
        mcp.send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
            .await;
        mcp
    }

    async fn send(&mut self, message: Value) {
        let mut line = message.to_string();
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).await.unwrap();
        self.stdin.flush().await.unwrap();
    }

    async fn next(&mut self) -> Value {
        let line = tokio::time::timeout(WAIT, self.stdout.next_line())
            .await
            .expect("humatad2 mcp went quiet")
            .unwrap()
            .expect("humatad2 mcp closed its output");
        serde_json::from_str(&line).unwrap()
    }

    /// Sends a request and returns its response's result.
    async fn request(&mut self, id: u64, method: &str, params: Value) -> Value {
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
            .await;
        self.response(id).await
    }

    async fn response(&mut self, id: u64) -> Value {
        loop {
            let message = self.next().await;
            if message.get("method").is_some() {
                self.notifications.push(message);
            } else if message["id"] == json!(id) {
                assert!(message.get("error").is_none(), "{message}");
                return message["result"].clone();
            }
        }
    }

    async fn call(&mut self, id: u64, tool: &str, arguments: Value) -> Value {
        self.request(
            id,
            "tools/call",
            json!({ "name": tool, "arguments": arguments }),
        )
        .await
    }

    /// Reads until a notification `method` comes.
    async fn wait_for_notification(&mut self, method: &str) -> Value {
        loop {
            if let Some(index) = self
                .notifications
                .iter()
                .position(|message| message["method"] == method)
            {
                return self.notifications.remove(index);
            }
            let message = self.next().await;
            self.notifications.push(message);
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_types_into_its_own_session_through_mcp() {
    let fixture = Fixture::start().await;
    let mut mcp = fixture.mcp(&fixture.token).await;
    let typed = fixture.daemon.dir.path().join("typed");

    let tools = mcp.request(1, "tools/list", json!({})).await;
    let result = mcp
        .call(
            2,
            "session_type",
            json!({ "session": SESSION, "text": format!("printf ok > {}\r", typed.display()) }),
        )
        .await;

    let names: Vec<&str> = tools["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert!(names.contains(&"session_type"));
    assert_eq!(result["isError"], false, "{result}");
    assert_eq!(result["structuredContent"]["outcome"], "applied");
    assert_eq!(read_when_written(&typed).await, "ok");
}

#[tokio::test(flavor = "multi_thread")]
async fn session_type_presses_enter_unless_told_not_to() {
    let fixture = Fixture::start().await;
    let mut mcp = fixture.mcp(&fixture.token).await;
    let first = fixture.daemon.dir.path().join("first");
    let second = fixture.daemon.dir.path().join("second");

    let sent = mcp
        .call(1, "session_type", json!({ "session": SESSION, "text": format!("printf ok > {}", first.display()) }))
        .await;
    assert_eq!(sent["isError"], false, "{sent}");
    assert_eq!(sent["structuredContent"]["result"]["typed"], true, "{sent}");
    assert_eq!(sent["structuredContent"]["result"]["submitted"], true, "{sent}");
    // No agent mid-turn here, and an MCP caller is told so, as the CLI is.
    assert_eq!(sent["structuredContent"]["result"]["agent_busy"], false, "{sent}");
    assert_eq!(read_when_written(&first).await, "ok");

    let left = mcp
        .call(
            2,
            "session_type",
            json!({ "session": SESSION, "text": format!("printf ok > {}", second.display()), "submit": false }),
        )
        .await;
    assert_eq!(left["isError"], false, "{left}");
    assert_eq!(left["structuredContent"]["result"]["typed"], true, "{left}");
    assert_eq!(left["structuredContent"]["result"]["submitted"], false, "{left}");
    assert!(!second.exists(), "text left unsent ran");
    // Only in the input: empty text presses Enter alone, which runs it.
    let enter = mcp.call(3, "session_type", json!({ "session": SESSION, "text": "" })).await;
    assert_eq!(enter["structuredContent"]["result"]["submitted"], true, "{enter}");
    assert_eq!(read_when_written(&second).await, "ok");
}

/// A tool the daemon doesn't dispatch is left out of `tools/list`, and no
/// tool it lists answers "not built" when it is called.
#[tokio::test(flavor = "multi_thread")]
async fn tools_list_offers_only_tools_the_daemon_runs() {
    let fixture = Fixture::start().await;
    let mut mcp = fixture.mcp(&fixture.token).await;

    // journal_status takes no arguments and the daemon doesn't run it yet.
    let unbuilt = mcp.call(1, "journal_status", json!({})).await;
    assert_eq!(unbuilt["isError"], true, "{unbuilt}");
    assert_eq!(
        unbuilt["structuredContent"]["error"]["kind"], "peer_version_unsupported",
        "{unbuilt}"
    );

    let listed = mcp.request(2, "tools/list", json!({})).await;
    let tools = listed["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().filter_map(|tool| tool["name"].as_str()).collect();
    assert!(!names.contains(&"journal_status"), "{names:?}");
    assert!(names.contains(&"whoami"), "{names:?}");
    assert!(names.contains(&"topology_get"), "{names:?}");

    // Every listed tool that only reads and needs no arguments runs; none
    // of them answers "not built".
    let mut id = 10;
    for tool in tools {
        let reads = tool["annotations"]["readOnlyHint"] == true;
        let needs_nothing = tool["inputSchema"]["required"]
            .as_array()
            .is_none_or(|required| required.is_empty());
        if !(reads && needs_nothing) {
            continue;
        }
        id += 1;
        let name = tool["name"].as_str().unwrap();
        let result = mcp.call(id, name, json!({})).await;
        assert_ne!(
            result["structuredContent"]["error"]["kind"], "peer_version_unsupported",
            "{name} is listed but not built: {result}"
        );
    }
}

/// Typing at the end of the turn is refused as an invalid `when`, not as a
/// tool that isn't built, so the agent can type now instead.
#[tokio::test(flavor = "multi_thread")]
async fn typing_at_turn_end_is_refused_as_an_invalid_when() {
    let fixture = Fixture::start().await;
    let mut mcp = fixture.mcp(&fixture.token).await;

    let result = mcp
        .call(
            1,
            "session_type",
            json!({ "session": SESSION, "text": "later\r", "when": "turn_end" }),
        )
        .await;

    assert_eq!(result["isError"], true, "{result}");
    assert_eq!(result["structuredContent"]["error"]["kind"], "invalid_field", "{result}");
}

#[tokio::test(flavor = "multi_thread")]
async fn whoami_names_the_session_the_token_belongs_to() {
    let fixture = Fixture::start().await;
    let mut mcp = fixture.mcp(&fixture.token).await;

    let result = mcp.call(1, "whoami", json!({})).await;

    assert_eq!(result["isError"], false, "{result}");
    let me = &result["structuredContent"]["result"];
    assert_eq!(me["session"], SESSION);
    assert_eq!(me["channel"], "ch_test");
}

/// TP-1: the agent's channel's machines, this one first, each with the
/// channel's sessions on it.
#[tokio::test(flavor = "multi_thread")]
async fn topology_get_names_this_machine_and_the_channels_sessions_on_it() {
    let fixture = Fixture::start().await;
    let mut mcp = fixture.mcp(&fixture.token).await;
    let me = mcp.call(1, "whoami", json!({})).await;
    let here = me["structuredContent"]["result"]["machine"].clone();

    let result = mcp.call(2, "topology_get", json!({})).await;

    assert_eq!(result["isError"], false, "{result}");
    let topology = &result["structuredContent"]["result"];
    assert_eq!(topology["this_machine"], here, "{topology}");
    let machines = topology["machines"].as_array().unwrap();
    assert_eq!(machines[0]["id"], here, "{topology}");
    assert_eq!(machines[0]["reachability"], "online", "{topology}");
    let mut sessions: Vec<&str> = machines[0]["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|session| session["id"].as_str())
        .collect();
    sessions.sort_unstable();
    assert_eq!(sessions, [SESSION, OTHER], "{topology}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_token_the_daemon_never_gave_out_is_refused() {
    let fixture = Fixture::start().await;
    let mut mcp = fixture.mcp(FORGED).await;

    let result = mcp.call(1, "whoami", json!({})).await;
    let refused =
        pb::daemon_service_client::DaemonServiceClient::new(fixture.daemon.channel().await)
            .who_am_i(as_agent(pb::WhoAmIRequest {}, FORGED))
            .await
            .unwrap_err();

    assert_eq!(result["isError"], true);
    assert_eq!(
        result["structuredContent"]["error"]["kind"],
        "identity_mismatch"
    );
    assert_eq!(refused.code(), Code::Unauthenticated);
    assert!(matches!(
        error_of(&refused).kind,
        Some(pb::error::Kind::IdentityMismatch(_))
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn opening_a_shell_or_pasting_is_the_users_alone() {
    let fixture = Fixture::start().await;
    let open_shell = || pb::OpenShellTabRequest {
        idempotency_key: None,
        session: Some(session()),
        repo: None,
        folder: String::new(),
    };
    let paste = || pb::PasteImageRequest {
        idempotency_key: None,
        session: Some(session()),
        data: b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec(),
        format: pb::ImageFormat::Png as i32,
    };
    let mut terminals = fixture.daemon.terminals().await;

    let agent_shell = terminals
        .open_shell_tab(as_agent(open_shell(), &fixture.token))
        .await
        .unwrap_err();
    let agent_paste = terminals
        .paste_image(as_agent(paste(), &fixture.token))
        .await
        .unwrap_err();
    let user_shell = terminals.open_shell_tab(open_shell()).await;
    let user_paste = terminals.paste_image(paste()).await;

    for refused in [&agent_shell, &agent_paste] {
        assert_eq!(refused.code(), Code::PermissionDenied, "{refused:?}");
        assert!(matches!(
            error_of(refused).kind,
            Some(pb::error::Kind::UserOnly(_))
        ));
    }
    assert!(user_shell.is_ok(), "{user_shell:?}");
    assert!(user_paste.is_ok(), "{user_paste:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_closed_tabs_token_stops_working() {
    let fixture = Fixture::start().await;
    let mut terminals = fixture.daemon.terminals().await;

    terminals
        .close_tab(pb::CloseTabRequest {
            idempotency_key: None,
            session: Some(session()),
            tab: Some(fixture.tab.clone()),
        })
        .await
        .unwrap();
    let refused =
        pb::daemon_service_client::DaemonServiceClient::new(fixture.daemon.channel().await)
            .who_am_i(as_agent(pb::WhoAmIRequest {}, &fixture.token))
            .await
            .unwrap_err();

    assert_eq!(refused.code(), Code::Unauthenticated);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_question_the_agent_stops_waiting_on_is_withdrawn() {
    let fixture = Fixture::start().await;
    let mut mcp = fixture.mcp(&fixture.token).await;

    mcp.send(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "inbox_ask",
            "arguments": { "question": "Ship it?", "choices": ["yes", "no"] },
            "_meta": { "progressToken": "waiting" },
        },
    }))
    .await;
    let progress = mcp.wait_for_notification("notifications/progress").await;
    let mut inbox =
        pb::inbox_service_client::InboxServiceClient::new(fixture.daemon.channel().await);
    let asked = inbox
        .list_requests(pb::ListRequestsRequest::default())
        .await
        .unwrap()
        .into_inner();
    mcp.send(json!({
        "jsonrpc": "2.0",
        "method": "notifications/cancelled",
        "params": { "requestId": 1 },
    }))
    .await;

    assert_eq!(progress["params"]["progressToken"], "waiting");
    assert_eq!(asked.pending_count, 1);
    let deadline = Instant::now() + WAIT;
    loop {
        let requests = inbox
            .list_requests(pb::ListRequestsRequest {
                include_answered: true,
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
            .requests;
        if requests
            .iter()
            .any(|request| request.state() == pb::RequestState::Withdrawn)
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the question was never withdrawn"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// A clone of `api` in the session's folder with `refund.rs` committed,
/// then edited, and the diff shown once, as the app does.
async fn shown_review(fixture: &Fixture) {
    let clone = fixture.daemon.dir.path().join("api");
    std::fs::create_dir(&clone).unwrap();
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .current_dir(&clone)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "git {args:?}");
    };
    git(&["init", "-q", "-b", "main"]);
    std::fs::write(clone.join("refund.rs"), "one\ntwo\nthree\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "first"]);
    std::fs::write(clone.join("refund.rs"), "one\n2\nthree\n").unwrap();
    pb::diff_service_client::DiffServiceClient::new(fixture.daemon.channel().await)
        .get_diff(pb::GetDiffRequest {
            session: Some(session()),
            base: Some(pb::DiffBase {
                base: Some(pb::diff_base::Base::UncommittedOnly(pb::UncommittedOnly {})),
            }),
            repos: Vec::new(),
        })
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_review_chats_agent_shows_the_user_a_place_and_reads_it_back() {
    let fixture = Fixture::start().await;
    shown_review(&fixture).await;
    let mut mcp = fixture.mcp(&fixture.token).await;

    let focused = mcp
        .call(
            1,
            "review_focus",
            json!({ "repo": "api", "path": "refund.rs", "lines": { "first": 2, "last": 2 } }),
        )
        .await;
    let context = mcp.call(2, "review_context", json!({})).await;
    let outside = mcp
        .call(
            3,
            "review_focus",
            json!({ "repo": "api", "path": "other.rs" }),
        )
        .await;

    assert_eq!(focused["isError"], false, "{focused}");
    assert_eq!(context["isError"], false, "{context}");
    let seen = &context["structuredContent"]["result"];
    assert_eq!(seen["open"]["file"]["path"], "refund.rs");
    assert_eq!(seen["selected_text"], json!(["2"]));
    assert_eq!(
        outside["isError"], true,
        "a file the review doesn't show can't be focused"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_review_chats_agent_leaves_notes_and_orders_the_review() {
    let fixture = Fixture::start().await;
    shown_review(&fixture).await;
    let mut mcp = fixture.mcp(&fixture.token).await;
    let notes = |context: &Value| -> Vec<String> {
        context["structuredContent"]["result"]["notes"]
            .as_array()
            .map(|notes| {
                notes
                    .iter()
                    .filter_map(|note| note["text"].as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    };

    let kept = mcp
        .call(
            1,
            "review_note",
            json!({ "repo": "api", "path": "refund.rs", "hunk_at": 2, "text": "why 2?" }),
        )
        .await;
    let gone = mcp
        .call(2, "review_note", json!({ "repo": "api", "path": "refund.rs", "lines": { "first": 3, "last": 3 }, "text": "fine" }))
        .await;
    assert_eq!(kept["isError"], false, "{kept}");
    let id = |reply: &Value| {
        reply["structuredContent"]["result"]["note_id"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let edited = mcp
        .call(
            3,
            "review_note_edit",
            json!({ "note_id": id(&kept), "text": "why a digit?" }),
        )
        .await;
    let dropped = mcp
        .call(4, "review_note_drop", json!({ "note_id": id(&gone) }))
        .await;
    assert_eq!(edited["isError"], false, "{edited}");
    assert_eq!(
        dropped["structuredContent"]["result"]["dropped"], true,
        "{dropped}"
    );
    assert_eq!(
        notes(&mcp.call(5, "review_context", json!({})).await),
        ["why a digit?"]
    );

    let plan = json!({
        "title": "Refund first",
        "spec": { "repos": [{ "repo": "api", "base": { "rev": "HEAD" } }] },
        "order": [{ "repo": "api", "path": "refund.rs" }],
        "notes": [{ "repo": "api", "path": "refund.rs", "lines": { "first": 2, "last": 2 }, "text": "start here" }],
    });
    let draft = mcp.call(6, "review_prepare", plan.clone()).await;
    assert_eq!(
        draft["structuredContent"]["result"]["state"], "draft",
        "outside an open review a plan is a draft view: {draft}"
    );
    pb::review_chat_service_client::ReviewChatServiceClient::new(fixture.daemon.channel().await)
        .report_review_view(pb::ReportReviewViewRequest {
            session: Some(session()),
            view: Some(pb::ReviewViewState::default()),
        })
        .await
        .unwrap();
    let ordered = mcp.call(7, "review_prepare", plan).await;
    assert_eq!(
        ordered["structuredContent"]["result"]["applied"], true,
        "{ordered}"
    );
    assert!(
        notes(&mcp.call(8, "review_context", json!({})).await).contains(&"start here".to_owned())
    );

    pb::review_chat_service_client::ReviewChatServiceClient::new(fixture.daemon.channel().await)
        .set_agent_switches(pb::SetAgentSwitchesRequest {
            session: Some(session()),
            switches: Some(pb::AgentSwitches {
                focus: true,
                notes: false,
                order: true,
            }),
        })
        .await
        .unwrap();
    let off = mcp
        .call(
            9,
            "review_note",
            json!({ "repo": "api", "path": "refund.rs", "hunk_at": 2, "text": "again" }),
        )
        .await;
    assert_eq!(off["isError"], true, "the user turned notes off: {off}");
}

/// Sends an `inbox_request` for `localhost:port` and waits till the user
/// sees it, so the test can answer it.
async fn ask_for_port(
    mcp: &mut Mcp,
    inbox: &mut pb::inbox_service_client::InboxServiceClient<tonic::transport::Channel>,
    id: u64,
    port: u16,
) -> pb::RequestId {
    mcp.send(json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {
            "name": "inbox_request",
            "arguments": {
                "capability": "network_reach",
                "target": { "network": { "host": "localhost", "port": port } },
                "reason": "run the database tests",
            },
        },
    }))
    .await;
    let deadline = Instant::now() + WAIT;
    loop {
        let pending = inbox
            .list_requests(pb::ListRequestsRequest::default())
            .await
            .unwrap()
            .into_inner()
            .requests;
        if let Some(id) = pending.into_iter().find_map(|request| request.id) {
            return id;
        }
        assert!(
            Instant::now() < deadline,
            "the request never reached the Inbox"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn answered(id: &pb::RequestId, answer: pb::request_answer::Answer) -> pb::AnswerRequestRequest {
    pb::AnswerRequestRequest {
        id: Some(id.clone()),
        answer: Some(pb::RequestAnswer {
            answer: Some(answer),
        }),
        ..Default::default()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_asks_ahead_for_access_and_gets_the_grant_or_the_users_no() {
    let fixture = Fixture::start().await;
    let mut mcp = fixture.mcp(&fixture.token).await;
    let mut inbox =
        pb::inbox_service_client::InboxServiceClient::new(fixture.daemon.channel().await);

    let asked = ask_for_port(&mut mcp, &mut inbox, 1, 5432).await;
    inbox
        .answer_request(answered(
            &asked,
            pb::request_answer::Answer::Approve(pb::Approve {}),
        ))
        .await
        .unwrap();
    let approved = mcp.response(1).await;
    let again = mcp
        .call(
            2,
            "inbox_request",
            json!({
                "capability": "network_reach",
                "target": { "network": { "host": "localhost", "port": 5432 } },
                "reason": "run them again",
            }),
        )
        .await;
    let other = ask_for_port(&mut mcp, &mut inbox, 3, 6379).await;
    inbox
        .answer_request(answered(
            &other,
            pb::request_answer::Answer::Deny(pb::Deny {
                note: "not the cache".to_owned(),
            }),
        ))
        .await
        .unwrap();
    let denied = mcp.response(3).await;
    assert_eq!(approved["isError"], false, "{approved}");
    let approved = &approved["structuredContent"]["result"];
    assert_eq!(approved["decision"], "approved");
    let grant = approved["grant_id"].as_str().unwrap();
    assert!(!grant.is_empty());
    let again = &again["structuredContent"]["result"];
    assert_eq!(
        again["decision"], "approved",
        "a covered ask needs no new answer"
    );
    assert_eq!(again["grant_id"], grant);
    assert_eq!(denied["isError"], true, "{denied}");
    assert_eq!(denied["structuredContent"]["error"]["kind"], "denied");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_prepares_revises_and_offers_a_view() {
    let fixture = Fixture::start().await;
    let mut mcp = fixture.mcp(&fixture.token).await;
    let result = |reply: &Value| reply["structuredContent"]["result"].clone();
    let walkthrough = json!({
        "title": "Refund walkthrough",
        "summary": "Read the refund first",
        "content": { "review": { "steps": [
            { "repo": "api", "path": "refund.rs", "lines": { "first": 1, "last": 2 }, "note": "start here" },
        ] } },
    });

    let prepared = mcp.call(1, "view_prepare", walkthrough).await;
    assert_eq!(result(&prepared)["state"], "draft", "{prepared}");
    let id = result(&prepared)["view_id"].as_str().unwrap().to_owned();
    let board = mcp
        .call(
            2,
            "view_prepare",
            json!({ "title": "Board", "summary": "", "content": { "board": "channel" } }),
        )
        .await;
    assert_eq!(
        result(&board)["state"],
        "draft",
        "the caller's channel board is fine: {board}"
    );
    let global = mcp
        .call(
            3,
            "view_prepare",
            json!({ "title": "Board", "summary": "", "content": { "board": "global" } }),
        )
        .await;
    assert_eq!(
        global["isError"], true,
        "the global board isn't the channel's: {global}"
    );

    let revised = mcp
        .call(
            4,
            "view_revise",
            json!({ "view_id": id, "title": "Refund, step by step", "expected_revision": 1 }),
        )
        .await;
    assert_eq!(
        result(&revised)["title"],
        "Refund, step by step",
        "{revised}"
    );
    let stale = mcp
        .call(
            5,
            "view_revise",
            json!({ "view_id": id, "title": "Stale", "expected_revision": 1 }),
        )
        .await;
    assert_eq!(
        stale["isError"], true,
        "a stale revision is refused: {stale}"
    );
    let ready = mcp
        .call(6, "view_mark_ready", json!({ "view_id": id }))
        .await;
    assert_eq!(result(&ready)["state"], "waiting", "{ready}");

    let listed = mcp.call(7, "views_list", json!({})).await;
    let mut states: Vec<(String, String)> = result(&listed)["views"]
        .as_array()
        .unwrap()
        .iter()
        .map(|view| {
            (
                view["title"].as_str().unwrap().to_owned(),
                view["state"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    states.sort();
    assert_eq!(
        states,
        [
            ("Board".to_owned(), "draft".to_owned()),
            ("Refund, step by step".to_owned(), "waiting".to_owned()),
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_tab_that_outlives_a_restart_keeps_its_token() {
    let mut fixture = Fixture::start().await;

    fixture.daemon.restart().await;
    // Once the start has brought back what the last run left, the tab
    // it reattached to is the same shell with the same token.
    let mut daemon =
        pb::daemon_service_client::DaemonServiceClient::new(fixture.daemon.channel().await);
    let deadline = Instant::now() + WAIT;
    while daemon
        .get_server_info(pb::GetServerInfoRequest {})
        .await
        .unwrap()
        .into_inner()
        .last_start
        .is_none()
    {
        assert!(
            Instant::now() < deadline,
            "the daemon never finished bringing sessions back"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let me = daemon
        .who_am_i(as_agent(pb::WhoAmIRequest {}, &fixture.token))
        .await
        .unwrap()
        .into_inner();
    let Some(pb::actor::Actor::Agent(agent)) = me.actor.and_then(|actor| actor.actor) else {
        panic!("the token is the tab's agent");
    };
    assert_eq!(agent.session, Some(session()));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_lists_the_channels_once_the_user_approves() {
    let fixture = Fixture::start().await;
    let mut mcp = fixture.mcp(&fixture.token).await;
    let mut inbox =
        pb::inbox_service_client::InboxServiceClient::new(fixture.daemon.channel().await);
    // Reading other channels is allowed by default (D127.4); the user
    // sets it to Ask me first.
    pb::access_service_client::AccessServiceClient::new(fixture.daemon.channel().await)
        .set_channel_rule(pb::SetChannelRuleRequest {
            channel: Some(pb::ChannelId {
                value: "ch_test".to_owned(),
            }),
            capability: pb::Capability::GlobalRead as i32,
            decision: pb::Decision::Ask as i32,
            ..Default::default()
        })
        .await
        .unwrap();

    let asked = mcp
        .call(1, "channels_list", json!({ "wait_seconds": 0 }))
        .await;
    assert_eq!(asked["structuredContent"]["outcome"], "pending", "{asked}");
    let id = asked["structuredContent"]["request_id"]
        .as_str()
        .unwrap()
        .to_owned();
    inbox
        .answer_request(answered(
            &pb::RequestId { value: id.clone() },
            pb::request_answer::Answer::Approve(pb::Approve {}),
        ))
        .await
        .unwrap();
    let waited = mcp.call(2, "inbox_wait", json!({ "request_id": id })).await;

    assert_eq!(waited["isError"], false, "{waited}");
    let channels = waited["structuredContent"]["result"]["channels"]
        .as_array()
        .unwrap();
    assert!(
        channels
            .iter()
            .any(|channel| channel["channel_id"] == "ch_test"),
        "{waited}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_stopping_another_session_asks_the_user_first() {
    let fixture = Fixture::start().await;
    let mut mcp = fixture.mcp(&fixture.token).await;

    // Stopping another session is Ask me by default (D306).
    let asked = mcp
        .call(1, "session_stop", json!({ "session": OTHER, "wait_seconds": 0 }))
        .await;

    assert_eq!(asked["structuredContent"]["outcome"], "pending", "{asked}");
    assert!(asked["structuredContent"]["request_id"].is_string(), "{asked}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_cannot_stop_another_session_when_the_rule_is_never() {
    let fixture = Fixture::start().await;
    let mut mcp = fixture.mcp(&fixture.token).await;
    pb::access_service_client::AccessServiceClient::new(fixture.daemon.channel().await)
        .set_channel_rule(pb::SetChannelRuleRequest {
            channel: Some(pb::ChannelId {
                value: "ch_test".to_owned(),
            }),
            capability: pb::Capability::SessionStopOther as i32,
            decision: pb::Decision::Never as i32,
            ..Default::default()
        })
        .await
        .unwrap();

    let result = mcp
        .call(1, "session_stop", json!({ "session": OTHER }))
        .await;

    assert_eq!(result["isError"], true, "{result}");
    assert_eq!(
        result["structuredContent"]["error"]["kind"], "denied",
        "{result}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_approved_request_is_waited_for_with_inbox_wait() {
    let fixture = Fixture::start().await;
    let mut mcp = fixture.mcp(&fixture.token).await;
    let mut inbox =
        pb::inbox_service_client::InboxServiceClient::new(fixture.daemon.channel().await);

    let asked = mcp
        .call(
            1,
            "inbox_request",
            json!({
                "capability": "network_reach",
                "target": { "network": { "host": "localhost", "port": 5432 } },
                "reason": "run the database tests",
                "wait_seconds": 0,
            }),
        )
        .await;
    assert_eq!(asked["structuredContent"]["outcome"], "pending", "{asked}");
    let id = asked["structuredContent"]["request_id"]
        .as_str()
        .unwrap()
        .to_owned();
    inbox
        .answer_request(answered(
            &pb::RequestId { value: id.clone() },
            pb::request_answer::Answer::Approve(pb::Approve {}),
        ))
        .await
        .unwrap();
    let waited = mcp.call(2, "inbox_wait", json!({ "request_id": id })).await;

    assert_eq!(waited["isError"], false, "{waited}");
    let result = &waited["structuredContent"]["result"];
    assert_eq!(result["decision"], "approved");
    assert!(
        !result["grant_id"].as_str().unwrap_or_default().is_empty(),
        "{waited}"
    );
}

/// 26.5: forking only some repos, or with a first message, isn't built, so
/// `session_fork` refuses either as an invalid field and nothing forks.
#[tokio::test(flavor = "multi_thread")]
async fn a_fork_of_some_repos_or_with_a_first_message_is_refused_as_an_invalid_field() {
    let fixture = Fixture::start().await;
    let mut mcp = fixture.mcp(&fixture.token).await;
    let base = json!({
        "machine": "",
        "kind": "code_and_context",
        "reason": "try another approach",
    });
    let mut some_repos = base.clone();
    some_repos["repos"] = json!(["acme/api"]);
    let mut with_message = base.clone();
    with_message["first_message"] = json!("start with the tests");
    for (id, arguments) in [(1, some_repos), (2, with_message)] {
        let result = mcp.call(id, "session_fork", arguments.clone()).await;
        assert_eq!(result["isError"], true, "{arguments}: {result}");
        assert_eq!(result["structuredContent"]["error"]["kind"], "invalid_field", "{arguments}: {result}");
    }
    let sessions = pb::session_service_client::SessionServiceClient::new(fixture.daemon.channel().await)
        .list_sessions(pb::ListSessionsRequest { include_archived: true, ..Default::default() })
        .await
        .unwrap()
        .into_inner()
        .sessions;
    assert_eq!(sessions.len(), 2, "nothing forked: {sessions:?}");
}
