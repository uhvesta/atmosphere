//! The comment tools of `humatad2 mcp` (DK-9, SM-19): the agent of a session
//! lists the comments typed into its terminal, replies to them and resolves
//! them, only its own session's, only when its channel's rules allow it.

use humatad_test_common as common;

use std::path::Path;
use std::process::{Command as StdCommand, Stdio};

use common::{Attach, Daemon, WAIT, binary, folder, shell_token, state_folder};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{ChannelRecord, SessionRecord, Store};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tonic::transport::Channel;

const SESSION: &str = "se_tools";
/// Another session of the same channel.
const OTHER: &str = "se_other";
const CHANNEL: &str = "ch_test";

const NAMES: [&str; 10] = ["one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten"];

/// A daemon that knows `SESSION` (in channel `CHANNEL`) whose folder holds
/// the clone `api` with `refund.rs` committed, one shell tab open in it, and
/// the token that tab's shell was given.
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
        for (id, title) in [(SESSION, "tools"), (OTHER, "other")] {
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
        let clone = dir.path().join("api");
        std::fs::create_dir(&clone).unwrap();
        git(&clone, &["init", "-q", "-b", "main"]);
        let text: String = NAMES.iter().map(|name| format!("line {name}\n")).collect();
        std::fs::write(clone.join("refund.rs"), text).unwrap();
        git(&clone, &["add", "."]);
        git(&clone, &["commit", "-q", "-m", "first"]);
        drop(store);
        let daemon = Daemon::start_in(dir).await;
        let (token, attach) = shell_token(&daemon, &session()).await;
        Self {
            daemon,
            token,
            _attach: attach,
        }
    }

    async fn mcp(&self) -> Mcp {
        Mcp::start(&self.daemon.socket, &self.token).await
    }

    async fn comments(&self) -> pb::comment_service_client::CommentServiceClient<Channel> {
        pb::comment_service_client::CommentServiceClient::new(self.daemon.channel().await)
    }

    /// A draft on `refund.rs`'s line `line`, as the user types it.
    async fn draft(&self, line: u32, text: &str) -> pb::Comment {
        self.comments()
            .await
            .create_comment(pb::CreateCommentRequest {
                idempotency_key: None,
                session: Some(session()),
                anchor: Some(pb::CommentAnchor {
                    file: Some(pb::RepoPath {
                        repo: Some(pb::RepoRef {
                            owner: "acme".to_owned(),
                            name: "api".to_owned(),
                        }),
                        path: "refund.rs".to_owned(),
                    }),
                    lines: Some(pb::LineRange { first: line, last: line }),
                    side: pb::DiffSide::New as i32,
                    ..pb::CommentAnchor::default()
                }),
                text: text.to_owned(),
                destination: Some(pb::CommentDestination {
                    destination: Some(pb::comment_destination::Destination::Terminal(pb::SessionTerminal {})),
                }),
            })
            .await
            .unwrap()
            .into_inner()
    }

    /// A draft that the user sent into the agent's terminal.
    async fn sent(&self, line: u32, text: &str) -> pb::Comment {
        let comment = self.draft(line, text).await;
        let response = self
            .comments()
            .await
            .send_comments(pb::SendCommentsRequest {
                idempotency_key: None,
                session: Some(session()),
                ids: vec![comment.id.clone().unwrap()],
                overall_note: String::new(),
            })
            .await
            .unwrap()
            .into_inner();
        assert!(!response.queued, "the shell tab is open, so the comment is typed at once");
        response.comments.into_iter().next().unwrap()
    }

    /// The comment as the user lists it, whatever its status.
    async fn find(&self, comment: &pb::Comment) -> pb::Comment {
        let all = [
            pb::CommentStatus::Draft,
            pb::CommentStatus::Queued,
            pb::CommentStatus::Sent,
            pb::CommentStatus::Addressed,
            pb::CommentStatus::Resolved,
        ];
        self.comments()
            .await
            .list_comments(pb::ListCommentsRequest {
                session: Some(session()),
                statuses: all.iter().map(|status| *status as i32).collect(),
                file: None,
                page: None,
                archived: false,
            })
            .await
            .unwrap()
            .into_inner()
            .comments
            .into_iter()
            .find(|found| found.id == comment.id)
            .expect("the comment is listed")
    }

    /// The user's rule for `capability` in the channel.
    async fn set_rule(&self, capability: pb::Capability, decision: pb::Decision) {
        pb::access_service_client::AccessServiceClient::new(self.daemon.channel().await)
            .set_channel_rule(pb::SetChannelRuleRequest {
                channel: Some(pb::ChannelId {
                    value: CHANNEL.to_owned(),
                }),
                capability: capability as i32,
                decision: decision as i32,
                ..Default::default()
            })
            .await
            .unwrap();
    }
}

fn session() -> pb::SessionId {
    pb::SessionId {
        value: SESSION.to_owned(),
    }
}

fn git(repo: &Path, args: &[&str]) {
    let output = StdCommand::new("git")
        .current_dir(repo)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .expect("run git");
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
}

fn status_of(comment: &pb::Comment) -> pb::CommentStatus {
    pb::CommentStatus::try_from(comment.status).unwrap()
}

fn id_of(comment: &pb::Comment) -> String {
    comment.id.clone().unwrap().value
}

/// Whether a reply was written by an agent.
fn by_agent(reply: &pb::CommentReply) -> bool {
    matches!(
        reply.author.as_ref().and_then(|author| author.actor.as_ref()),
        Some(pb::actor::Actor::Agent(_))
    )
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
        let mut mcp = Self {
            _child: child,
            stdin,
            stdout,
            next_id: 0,
        };
        mcp.request(
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
        self.stdin.write_all(line.as_bytes()).await.unwrap();
        self.stdin.flush().await.unwrap();
    }

    /// Sends a request and returns its response's result, skipping
    /// notifications.
    async fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })).await;
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
        self.request("tools/call", json!({ "name": tool, "arguments": arguments })).await
    }
}

/// What a successful tool call returned.
fn ok(result: &Value) -> &Value {
    assert_eq!(result["isError"], false, "{result}");
    &result["structuredContent"]["result"]
}

fn failed(result: &Value) {
    assert_eq!(result["isError"], true, "{result}");
}

fn denied(result: &Value) {
    failed(result);
    assert_eq!(result["structuredContent"]["error"]["kind"], "denied", "{result}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_finds_replies_to_and_resolves_a_comment_sent_to_its_session() {
    let fixture = Fixture::start().await;
    let sent = fixture.sent(3, "rename this").await;
    let held = fixture.draft(5, "still a draft").await;
    let mut mcp = fixture.mcp().await;

    let listed = mcp.call("comments_list", json!({})).await;
    let found = ok(&listed)["comments"].as_array().unwrap();
    assert_eq!(found.len(), 1, "only the comment that was sent: {listed}");
    assert_eq!(found[0]["id"], id_of(&sent));
    assert_eq!(found[0]["path"], "refund.rs");
    assert_eq!(found[0]["text"], "rename this");
    assert_eq!(found[0]["status"], "sent");

    let replied = mcp
        .call("comment_reply", json!({ "comment_id": id_of(&sent), "text": "looking at it" }))
        .await;
    assert_eq!(ok(&replied)["replies"], 1, "{replied}");
    let seen = fixture.find(&sent).await;
    assert_eq!(status_of(&seen), pb::CommentStatus::Sent);
    assert_eq!(seen.replies.len(), 1);
    assert_eq!(seen.replies[0].text, "looking at it");
    assert!(by_agent(&seen.replies[0]));

    let resolved = mcp
        .call("comment_resolve", json!({ "comment_id": id_of(&sent), "reply": "renamed to total" }))
        .await;
    assert_eq!(ok(&resolved)["status"], "resolved", "{resolved}");
    assert_eq!(ok(&resolved)["replies"], 2);
    let seen = fixture.find(&sent).await;
    assert_eq!(status_of(&seen), pb::CommentStatus::Resolved);
    assert!(seen.resolved.is_some());
    assert_eq!(seen.replies.len(), 2);
    assert_eq!(seen.replies[1].text, "renamed to total");

    let after = mcp.call("comments_list", json!({})).await;
    assert!(ok(&after)["comments"].as_array().unwrap().is_empty(), "{after}");

    let reopened = fixture
        .comments()
        .await
        .reopen_comment(pb::ReopenCommentRequest {
            idempotency_key: None,
            id: sent.id.clone(),
            expected_revision: None,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(status_of(&reopened), pb::CommentStatus::Draft);
    assert_eq!(reopened.replies.len(), 2, "the thread stays");
    assert_eq!(status_of(&fixture.find(&held).await), pb::CommentStatus::Draft);
}

#[tokio::test(flavor = "multi_thread")]
async fn resolving_allows_blank_reply_and_needs_a_comment_that_was_sent() {
    let fixture = Fixture::start().await;
    let sent = fixture.sent(3, "rename this").await;
    let held = fixture.draft(5, "never sent").await;
    let mut mcp = fixture.mcp().await;

    let resolved = mcp
        .call("comment_resolve", json!({ "comment_id": id_of(&sent), "reply": "  " }))
        .await;
    assert_eq!(ok(&resolved)["status"], "resolved", "{resolved}");
    assert_eq!(ok(&resolved)["replies"], 0, "{resolved}");
    let seen = fixture.find(&sent).await;
    assert_eq!(status_of(&seen), pb::CommentStatus::Resolved);
    assert!(seen.replies.is_empty());

    failed(&mcp.call("comment_reply", json!({ "comment_id": id_of(&held), "text": "hello" })).await);
    failed(&mcp.call("comment_resolve", json!({ "comment_id": id_of(&held), "reply": "done" })).await);
    let seen = fixture.find(&held).await;
    assert_eq!(status_of(&seen), pb::CommentStatus::Draft);
    assert!(seen.replies.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rule_set_to_never_denies_that_tool_only() {
    let fixture = Fixture::start().await;
    let sent = fixture.sent(3, "rename this").await;
    fixture.set_rule(pb::Capability::CommentReply, pb::Decision::Never).await;
    let mut mcp = fixture.mcp().await;

    denied(&mcp.call("comment_reply", json!({ "comment_id": id_of(&sent), "text": "hello" })).await);
    assert!(fixture.find(&sent).await.replies.is_empty());

    let resolved = mcp
        .call("comment_resolve", json!({ "comment_id": id_of(&sent), "reply": "done" }))
        .await;
    assert_eq!(ok(&resolved)["status"], "resolved", "{resolved}");
    assert_eq!(status_of(&fixture.find(&sent).await), pb::CommentStatus::Resolved);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_resolve_rule_set_to_never_leaves_replying_alone() {
    let fixture = Fixture::start().await;
    let sent = fixture.sent(3, "rename this").await;
    fixture.set_rule(pb::Capability::CommentResolve, pb::Decision::Never).await;
    let mut mcp = fixture.mcp().await;

    denied(&mcp.call("comment_resolve", json!({ "comment_id": id_of(&sent), "reply": "done" })).await);
    let seen = fixture.find(&sent).await;
    assert_eq!(status_of(&seen), pb::CommentStatus::Sent);
    assert!(seen.replies.is_empty());

    let replied = mcp
        .call("comment_reply", json!({ "comment_id": id_of(&sent), "text": "hello" }))
        .await;
    assert_eq!(ok(&replied)["replies"], 1, "{replied}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_cannot_answer_a_comment_of_another_session() {
    let fixture = Fixture::start().await;
    let sent = fixture.sent(3, "rename this").await;
    let mut mcp = fixture.mcp().await;

    let theirs = format!("{OTHER}/c-1");
    failed(&mcp.call("comment_reply", json!({ "comment_id": theirs, "text": "hello" })).await);
    failed(&mcp.call("comment_resolve", json!({ "comment_id": theirs, "reply": "done" })).await);

    let seen = fixture.find(&sent).await;
    assert_eq!(status_of(&seen), pb::CommentStatus::Sent);
    assert!(seen.replies.is_empty());
}
