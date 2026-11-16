//! The capability matrix (TESTING.md §5.5) at the boundary: one table of
//! tiers, each run as an agent through MCP and through the CLI inside the
//! agent's identity. Only the class of outcome is checked: allowed, held
//! for the user, refused at once, or the user's alone.

mod common;

use std::path::Path;
use std::process::Stdio;

use common::{
    Attach, Daemon, WAIT, approve, binary, create_channel, first_pending, humata_as_agent,
    machine_id, pending_requests, shell_token, start_session, start_world,
};
use common::{as_script, create_script_token, humata_as_script, start_world_with};
use humata2_grpc::humata::v1 as pb;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tonic::Request;
use tonic::metadata::MetadataValue;

const OWNER: &str = "o";
const REPO: &str = "r";

#[derive(Clone, Copy, Debug)]
enum Tier {
    NoRule,
    DefaultAllow,
    AskMe,
    Never,
    UserOnly,
}

/// What a call came to, whichever way it was made.
#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Applied,
    /// Held in the Inbox as this request.
    Pending(String),
    /// Refused at once.
    Denied,
    UserOnly,
    Other(String),
}

/// One row: a tier and the outcome its representative call has.
const TABLE: &[(Tier, &str)] = &[
    (Tier::NoRule, "applied"),
    (Tier::AskMe, "pending"),
    (Tier::Never, "denied"),
    (Tier::UserOnly, "user_only"),
];

/// A daemon with a channel and two sessions, and the first one's agent.
struct World {
    daemon: Daemon,
    _world: common::World,
    channel: pb::ChannelId,
    me: pb::SessionId,
    other: pb::SessionId,
    token: String,
    _attach: Attach,
}

impl World {
    async fn start() -> Self {
        let (daemon, world) = start_world().await;
        world.remote(OWNER, REPO);
        let (channel, setup) = create_channel(&daemon, "matrix", &[(OWNER, REPO)]).await;
        assert_eq!(
            setup.state,
            pb::OperationState::Succeeded as i32,
            "{setup:?}"
        );
        let (me, _) = start_session(&daemon, &channel, "me").await;
        let (other, _) = start_session(&daemon, &channel, "other").await;
        let (token, attach) = shell_token(&daemon, &me).await;
        let channel = channel.id.unwrap_or_default();
        Self {
            daemon,
            _world: world,
            channel,
            me,
            other,
            token,
            _attach: attach,
        }
    }

    /// The user sets the channel's rule for reading other channels. The
    /// default is Allow (D127.4), so the "Ask me" tier sets Ask first.
    async fn ask_on_global_read(&self) {
        let mut access =
            pb::access_service_client::AccessServiceClient::new(self.daemon.channel().await);
        access
            .set_channel_rule(pb::SetChannelRuleRequest {
                channel: Some(self.channel.clone()),
                capability: pb::Capability::GlobalRead as i32,
                decision: pb::Decision::Ask as i32,
                ..Default::default()
            })
            .await
            .unwrap();
    }

    /// Everything in the Inbox, answered or not.
    async fn inbox_len(&self) -> usize {
        self.daemon
            .inbox_service()
            .await
            .list_requests(pb::ListRequestsRequest {
                include_answered: true,
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
            .requests
            .len()
    }
}

fn as_agent<T>(message: T, token: &str) -> Request<T> {
    let mut request = Request::new(message);
    request.metadata_mut().insert(
        "humata-session-token",
        MetadataValue::try_from(token).unwrap(),
    );
    request
}

// ---------------------------------------------------------------------------
// MCP

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

    async fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
            .await;
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
        self.request(
            "tools/call",
            json!({ "name": tool, "arguments": arguments }),
        )
        .await
    }

    async fn tool_names(&mut self) -> Vec<String> {
        self.request("tools/list", json!({})).await["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|tool| tool["name"].as_str().map(str::to_owned))
            .collect()
    }
}

fn mcp_outcome(result: &Value) -> Outcome {
    let body = &result["structuredContent"];
    if result["isError"] == json!(true) {
        return match body["error"]["kind"].as_str() {
            Some("denied") => Outcome::Denied,
            Some("user_only") => Outcome::UserOnly,
            other => Outcome::Other(format!("{other:?}")),
        };
    }
    match body["outcome"].as_str() {
        Some("applied") => Outcome::Applied,
        Some("pending") => {
            Outcome::Pending(body["request_id"].as_str().unwrap_or_default().to_owned())
        }
        other => Outcome::Other(format!("{other:?}")),
    }
}

/// The representative call of `tier`, made through MCP.
async fn mcp_call(mcp: &mut Mcp, world: &World, tier: Tier) -> Value {
    match tier {
        Tier::NoRule => {
            mcp.call(
                "file_write",
                json!({
                    "location": { "root": "clone", "repo": format!("{OWNER}/{REPO}"), "path": "note.txt" },
                    "text": "hello",
                }),
            )
            .await
        }
        Tier::DefaultAllow => {
            mcp.call("board_item_create", json!({ "board": "channel", "title": "an item" }))
                .await
        }
        // User-only is reached through the Ask call that makes a request
        // for the agent to try to answer.
        // wait_seconds 0: answer pending at once instead of waiting (TP-11).
        Tier::AskMe | Tier::UserOnly => {
            world.ask_on_global_read().await;
            mcp.call("channels_list", json!({ "wait_seconds": 0 })).await
        }
        Tier::Never => {
            mcp.call("session_stop", json!({ "session": world.other.value }))
                .await
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_matrix_holds_for_an_agent_over_mcp() {
    let mut failures = Vec::new();
    for &(tier, expect) in TABLE {
        let world = World::start().await;
        let mut mcp = Mcp::start(&world.daemon.socket, &world.token).await;
        let reply = mcp_call(&mut mcp, &world, tier).await;
        let got = mcp_outcome(&reply);
        match (expect, &got) {
            ("applied", Outcome::Applied) => {
                if world.inbox_len().await != 0 {
                    failures.push(format!(
                        "{tier:?}: an allowed call left a request in the Inbox"
                    ));
                }
            }
            ("pending", Outcome::Pending(request)) => {
                let waiting = pending_requests(&world.daemon).await;
                if waiting.len() != 1 || waiting[0].value != *request {
                    failures.push(format!(
                        "{tier:?}: the Inbox holds {waiting:?}, not {request}"
                    ));
                    continue;
                }
                approve(&world.daemon, &waiting[0]).await;
                let done = mcp
                    .call(
                        "inbox_wait",
                        json!({ "request_id": request, "wait_seconds": 15 }),
                    )
                    .await;
                if mcp_outcome(&done) != Outcome::Applied {
                    failures.push(format!("{tier:?}: after approval inbox_wait gave {done}"));
                }
            }
            ("denied", Outcome::Denied) => {
                if world.inbox_len().await != 0 {
                    failures.push(format!(
                        "{tier:?}: a refused call put a request in the Inbox"
                    ));
                }
            }
            ("user_only", Outcome::Pending(_)) => {
                // The agent has no answer tool, and the daemon refuses its
                // AnswerRequest; the request stays for the user.
                let names = mcp.tool_names().await;
                if names
                    .iter()
                    .any(|name| name.starts_with("inbox_answer") || name.contains("approve"))
                {
                    failures.push(format!(
                        "{tier:?}: the agent has a tool to answer requests: {names:?}"
                    ));
                }
                let id = first_pending(&world.daemon).await;
                let refused = world
                    .daemon
                    .inbox_service()
                    .await
                    .answer_request(as_agent(
                        pb::AnswerRequestRequest {
                            id: Some(id),
                            answer: Some(pb::RequestAnswer {
                                answer: Some(pb::request_answer::Answer::Approve(pb::Approve {})),
                            }),
                            ..Default::default()
                        },
                        &world.token,
                    ))
                    .await;
                let user_only = matches!(
                    refused
                        .as_ref()
                        .map_err(common::error_of)
                        .map_err(|e| e.kind),
                    Err(Some(pb::error::Kind::UserOnly(_)))
                );
                if !user_only || pending_requests(&world.daemon).await.len() != 1 {
                    failures.push(format!(
                        "{tier:?}: the agent's answer wasn't refused as user-only: {refused:?}"
                    ));
                }
            }
            _ => failures.push(format!("{tier:?}: expected {expect}, got {got:?}: {reply}")),
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

// ---------------------------------------------------------------------------
// CLI

fn cli_outcome(code: i32, json: &Value) -> Outcome {
    match code {
        0 => Outcome::Applied,
        12 => Outcome::Pending(
            json["error"]["details"]["request"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        ),
        _ => match json["error"]["kind"].as_str() {
            Some("denied") if code == 4 => Outcome::Denied,
            Some("user_only") if code == 5 => Outcome::UserOnly,
            other => Outcome::Other(format!("exit {code}, {other:?}")),
        },
    }
}

/// The representative call of `tier` as `humata` arguments.
async fn cli_args(world: &World, tier: Tier, local: &Path) -> Vec<String> {
    let args: Vec<&str> = match tier {
        Tier::NoRule => {
            let machine = machine_id(&world.daemon).await.value;
            let location = format!("{machine}:{}/{OWNER}/{REPO}/note.txt", world.me.value);
            let local = local.display().to_string();
            return ["files", "put", local.as_str(), location.as_str()]
                .map(str::to_owned)
                .to_vec();
        }
        Tier::DefaultAllow => vec!["board", "add", "an item"],
        Tier::AskMe | Tier::UserOnly => {
            world.ask_on_global_read().await;
            vec!["channels"]
        }
        Tier::Never => vec!["session", "stop", world.other.value.as_str()],
    };
    args.into_iter().map(str::to_owned).collect()
}

async fn cli(world: &World, extra: &[&str], args: &[String]) -> (i32, Value) {
    let all: Vec<&str> = extra
        .iter()
        .copied()
        .chain(args.iter().map(String::as_str))
        .collect();
    humata_as_agent(&world.daemon, &world.token, &all).await
}

#[tokio::test(flavor = "multi_thread")]
async fn the_matrix_holds_for_an_agent_over_the_cli() {
    let mut failures = Vec::new();
    for &(tier, expect) in TABLE {
        let world = World::start().await;
        let local = world.daemon.dir.path().join("local.txt");
        std::fs::write(&local, "hello").unwrap();
        let args = cli_args(&world, tier, &local).await;
        let (code, json) = cli(&world, &["--key", "k1"], &args).await;
        let got = cli_outcome(code, &json);
        match (expect, &got) {
            ("applied", Outcome::Applied) => {
                if world.inbox_len().await != 0 {
                    failures.push(format!(
                        "{tier:?}: an allowed call left a request in the Inbox"
                    ));
                }
            }
            ("pending", Outcome::Pending(request)) => {
                let waiting = pending_requests(&world.daemon).await;
                if waiting.len() != 1 || waiting[0].value != *request {
                    failures.push(format!(
                        "{tier:?}: the Inbox holds {waiting:?}, not {request}"
                    ));
                    continue;
                }
                // A read carries no idempotency key, so the same call run
                // with --wait asks again; approving the request that run
                // made lets it finish with the result.
                let socket = world.daemon.socket.clone();
                let token = world.token.clone();
                let home = world.daemon.dir.path().to_owned();
                let waiter = tokio::spawn({
                    let args = args.clone();
                    async move {
                        let output = Command::new(binary("HUMATA"))
                            .env_clear()
                            .env("PATH", "/usr/bin:/bin")
                            .env("HOME", home)
                            .env("HUMATA_SOCKET", socket)
                            .env("HUMATA_SESSION_TOKEN", token)
                            .args(["--json", "--key", "k1", "--wait", "15"])
                            .args(args)
                            .output()
                            .await
                            .unwrap();
                        output.status.code().unwrap_or(-1)
                    }
                });
                let second = pending_other_than(&world.daemon, &waiting[0]).await;
                approve(&world.daemon, &second).await;
                let waited = waiter.await.unwrap();
                if waited != 0 {
                    failures.push(format!(
                        "{tier:?}: the waiting call ended with exit {waited}"
                    ));
                }
            }
            ("denied", Outcome::Denied) => {
                if world.inbox_len().await != 0 {
                    failures.push(format!(
                        "{tier:?}: a refused call put a request in the Inbox"
                    ));
                }
            }
            ("user_only", Outcome::Pending(request)) => {
                let (code, json) =
                    humata_as_agent(&world.daemon, &world.token, &["inbox", "approve", request])
                        .await;
                if cli_outcome(code, &json) != Outcome::UserOnly
                    || pending_requests(&world.daemon).await.len() != 1
                {
                    failures.push(format!(
                        "{tier:?}: the agent's approve gave exit {code}: {json}"
                    ));
                }
            }
            _ => failures.push(format!("{tier:?}: expected {expect}, got {got:?}: {json}")),
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cli_call_that_asks_ends_pending_and_never_hangs_without_wait() {
    let world = World::start().await;
    world.ask_on_global_read().await;
    let started = std::time::Instant::now();

    let (code, json) = humata_as_agent(&world.daemon, &world.token, &["channels"]).await;

    assert_eq!(code, 12, "{json}");
    assert!(started.elapsed() < WAIT);
    assert_eq!(
        json["error"]["details"]["request"]
            .as_str()
            .map(str::to_owned),
        pending_requests(&world.daemon)
            .await
            .first()
            .map(|id| id.value.clone())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_denied_ask_ends_the_waiting_cli_call_as_denied() {
    let world = World::start().await;
    world.ask_on_global_read().await;
    let waiter = tokio::spawn({
        let socket = world.daemon.socket.clone();
        let token = world.token.clone();
        let home = world.daemon.dir.path().to_owned();
        async move {
            let output = Command::new(binary("HUMATA"))
                .env_clear()
                .env("PATH", "/usr/bin:/bin")
                .env("HOME", home)
                .env("HUMATA_SOCKET", socket)
                .env("HUMATA_SESSION_TOKEN", token)
                .args(["--json", "--wait", "15", "channels"])
                .output()
                .await
                .unwrap();
            (
                output.status.code().unwrap_or(-1),
                String::from_utf8_lossy(&output.stdout).into_owned(),
            )
        }
    });
    let id = first_pending(&world.daemon).await;
    world
        .daemon
        .inbox_service()
        .await
        .answer_request(pb::AnswerRequestRequest {
            id: Some(id),
            answer: Some(pb::RequestAnswer {
                answer: Some(pb::request_answer::Answer::Deny(pb::Deny {
                    note: String::new(),
                })),
            }),
            ..Default::default()
        })
        .await
        .unwrap();

    let (code, json) = waiter.await.unwrap();

    assert_ne!(code, 0, "{json}");
    assert_ne!(code, 12, "{json}");
}

/// The first pending request that isn't `seen`, waiting for it to appear.
async fn pending_other_than(daemon: &common::Daemon, seen: &pb::RequestId) -> pb::RequestId {
    let deadline = std::time::Instant::now() + WAIT;
    loop {
        if let Some(id) = pending_requests(daemon)
            .await
            .into_iter()
            .find(|id| id.value != seen.value)
        {
            return id;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the waiting call made no request"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

// ---------------------------------------------------------------------------

/// Default Allow: the channel board takes an agent's item with no question.
#[tokio::test(flavor = "multi_thread")]
async fn default_allow_tier_is_applied_with_nothing_in_the_inbox() {
    let world = World::start().await;
    let mut mcp = Mcp::start(&world.daemon.socket, &world.token).await;

    let over_mcp = mcp_call(&mut mcp, &world, Tier::DefaultAllow).await;
    let args = cli_args(&world, Tier::DefaultAllow, Path::new("")).await;
    let (code, json) = cli(&world, &[], &args).await;

    assert_eq!(mcp_outcome(&over_mcp), Outcome::Applied, "{over_mcp}");
    assert_eq!(cli_outcome(code, &json), Outcome::Applied, "{json}");
    assert_eq!(world.inbox_len().await, 0);
}

// ---------------------------------------------------------------------------
// Script tokens (SEC-14)

/// A daemon with channel "matrix" (two sessions) and channel "second" (one).
struct Scripts {
    daemon: Daemon,
    _world: common::World,
    first: pb::ChannelId,
    second: pb::ChannelId,
    mine: pb::SessionId,
    other: pb::SessionId,
}

impl Scripts {
    async fn start(extra: &[&str]) -> Self {
        let (daemon, world) = start_world_with(extra).await;
        world.remote(OWNER, REPO);
        world.remote(OWNER, "r2");
        let (first, setup) = create_channel(&daemon, "matrix", &[(OWNER, REPO)]).await;
        assert_eq!(setup.state, pb::OperationState::Succeeded as i32, "{setup:?}");
        let (second, setup) = create_channel(&daemon, "second", &[(OWNER, "r2")]).await;
        assert_eq!(setup.state, pb::OperationState::Succeeded as i32, "{setup:?}");
        let (mine, _) = start_session(&daemon, &first, "me").await;
        let (other, _) = start_session(&daemon, &first, "other").await;
        start_session(&daemon, &second, "elsewhere").await;
        Self {
            daemon,
            _world: world,
            first: first.id.unwrap_or_default(),
            second: second.id.unwrap_or_default(),
            mine,
            other,
        }
    }

    async fn inbox_len(&self) -> usize {
        self.daemon
            .inbox_service()
            .await
            .list_requests(pb::ListRequestsRequest {
                include_answered: true,
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
            .requests
            .len()
    }

    /// A token that reads the first channel.
    async fn token_for(&self, name: &str) -> (String, String) {
        create_script_token(
            &self.daemon,
            name,
            &[pb::Capability::ChannelRead],
            &[&self.first],
            0,
        )
        .await
        .unwrap()
    }

    /// Reading the channel through the CLI, as the script.
    async fn topology(&self, token: &str, channel: &pb::ChannelId) -> Outcome {
        let (code, json) =
            humata_as_script(&self.daemon, token, &["topology", "--channel", &channel.value]).await;
        script_outcome(code, &json)
    }
}

/// `cli_outcome`, with a refused credential told apart: it shares exit 5
/// with the user-only class, so the error kind decides.
fn script_outcome(code: i32, json: &Value) -> Outcome {
    // The daemon refuses an unusable token as an identity problem; a
    // bare Unauthenticated status is the same class.
    if matches!(
        json["error"]["kind"].as_str(),
        Some("unauthenticated" | "identity_mismatch")
    ) {
        return Outcome::Other("unauthenticated".to_owned());
    }
    cli_outcome(code, json)
}

fn unauthenticated() -> Outcome {
    Outcome::Other("unauthenticated".to_owned())
}

#[tokio::test(flavor = "multi_thread")]
async fn the_matrix_holds_for_a_script_token() {
    let world = Scripts::start(&[]).await;
    let (_, token) = create_script_token(
        &world.daemon,
        "ci",
        &[pb::Capability::GlobalRead, pb::Capability::ChannelRead],
        &[&world.first],
        0,
    )
    .await
    .unwrap();
    // A rule of Ask me doesn't hold a script: tokens skip channel rules.
    pb::access_service_client::AccessServiceClient::new(world.daemon.channel().await)
        .set_channel_rule(pb::SetChannelRuleRequest {
            channel: Some(world.first.clone()),
            capability: pb::Capability::GlobalRead as i32,
            decision: pb::Decision::Ask as i32,
            ..Default::default()
        })
        .await
        .unwrap();
    let local = world.daemon.dir.path().join("local.txt");
    std::fs::write(&local, "hello").unwrap();
    let machine = machine_id(&world.daemon).await.value;
    let location = format!("{machine}:{}/{OWNER}/{REPO}/note.txt", world.mine.value);
    let local = local.display().to_string();
    let first = world.first.value.clone();
    let mut failures = Vec::new();

    let rows: Vec<(&str, Vec<&str>, Outcome)> = vec![
        ("listed, rule Ask me", vec!["channels"], Outcome::Applied),
        (
            "listed, its channel",
            vec!["topology", "--channel", &first],
            Outcome::Applied,
        ),
        (
            "not listed",
            vec!["session", "stop", &world.other.value],
            Outcome::Denied,
        ),
        (
            "listed, other channel",
            vec!["topology", "--channel", &world.second.value],
            Outcome::Denied,
        ),
        (
            "files, no session",
            vec!["files", "put", &local, &location],
            Outcome::Denied,
        ),
        ("user-only token ls", vec!["token", "ls"], Outcome::UserOnly),
        (
            "user-only rule",
            vec!["access", "set", "global_read", "never", "--channel", &first],
            Outcome::UserOnly,
        ),
    ];
    for (row, args, expect) in rows {
        let (code, json) = humata_as_script(&world.daemon, &token, &args).await;
        let got = script_outcome(code, &json);
        if got != expect {
            failures.push(format!("{row}: expected {expect:?}, got {got:?}: {json}"));
        }
    }
    if world.inbox_len().await != 0 {
        failures.push("a script's call put a request in the Inbox".to_owned());
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_token_cannot_be_made_for_a_user_only_action() {
    let world = Scripts::start(&[]).await;
    for capability in [pb::Capability::AccessChange, pb::Capability::PeerReport] {
        let refused = create_script_token(&world.daemon, "x", &[capability], &[&world.first], 0)
            .await
            .unwrap_err();
        let (field, _) = common::invalid_field(&refused);
        assert_eq!(field, "capabilities", "{capability:?}");
    }
    let too_long = create_script_token(
        &world.daemon,
        "x",
        &[pb::Capability::ChannelRead],
        &[&world.first],
        91,
    )
    .await
    .unwrap_err();
    common::invalid_field(&too_long);
    let (_, token) = create_script_token(
        &world.daemon,
        "ok",
        &[pb::Capability::ChannelRead],
        &[&world.first],
        90,
    )
    .await
    .unwrap();

    // A script holding a valid token can't make or revoke tokens.
    let over_grpc = world
        .daemon
        .cli_auth_service()
        .await
        .create_script_token(as_script(
            pb::CreateScriptTokenRequest {
                name: "mine".to_owned(),
                capabilities: vec![pb::Capability::ChannelRead as i32],
                channels: vec![world.first.clone()],
                days: 1,
                ..Default::default()
            },
            &token,
        ))
        .await
        .unwrap_err();
    assert!(matches!(
        common::error_of(&over_grpc).kind,
        Some(pb::error::Kind::UserOnly(_))
    ));
    let first = world.first.value.clone();
    let (code, json) = humata_as_script(
        &world.daemon,
        &token,
        &[
            "token", "create", "--name", "mine", "--capability", "channel_read", "--channel",
            &first,
        ],
    )
    .await;
    assert_eq!(script_outcome(code, &json), Outcome::UserOnly, "{json}");
    let listed = world
        .daemon
        .cli_auth_service()
        .await
        .list_script_tokens(pb::ListScriptTokensRequest {})
        .await
        .unwrap()
        .into_inner();
    assert_eq!(listed.tokens.len(), 1, "{listed:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_revoked_token_stops_at_once_and_a_kept_one_survives_a_restart() {
    let mut world = Scripts::start(&[]).await;
    let (revoked_id, revoked) = world.token_for("revoked").await;
    let (kept_id, kept) = world.token_for("kept").await;
    assert_eq!(world.topology(&revoked, &world.first).await, Outcome::Applied);
    assert_eq!(world.topology(&kept, &world.first).await, Outcome::Applied);

    world
        .daemon
        .cli_auth_service()
        .await
        .revoke_script_token(pb::RevokeScriptTokenRequest {
            token_id: revoked_id,
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(
        world.topology(&revoked, &world.first).await,
        unauthenticated()
    );
    assert_eq!(world.topology(&kept, &world.first).await, Outcome::Applied);

    world.daemon.restart().await;

    assert_eq!(world.topology(&kept, &world.first).await, Outcome::Applied);
    assert_eq!(
        world.topology(&revoked, &world.first).await,
        unauthenticated()
    );
    let listed = world
        .daemon
        .cli_auth_service()
        .await
        .list_script_tokens(pb::ListScriptTokensRequest {})
        .await
        .unwrap()
        .into_inner();
    assert!(
        listed.tokens.iter().any(|entry| entry.token_id == kept_id),
        "{listed:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_expired_token_is_refused() {
    let world = Scripts::start(&["--script-token-seconds", "3"]).await;
    let (_, token) = create_script_token(
        &world.daemon,
        "short",
        &[pb::Capability::ChannelRead],
        &[&world.first],
        0,
    )
    .await
    .unwrap();
    let started = std::time::Instant::now();
    assert_eq!(world.topology(&token, &world.first).await, Outcome::Applied);
    let lifetime = std::time::Duration::from_secs(3);
    if let Some(left) = (lifetime + std::time::Duration::from_millis(500))
        .checked_sub(started.elapsed())
    {
        tokio::time::sleep(left).await;
    }
    assert_eq!(world.topology(&token, &world.first).await, unauthenticated());
}
