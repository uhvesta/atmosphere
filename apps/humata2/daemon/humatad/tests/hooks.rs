//! Permission hooks through a real `humatad2` (HOOKS, D191, D192): an agent's
//! blocking hook raises one Inbox card, and approving, denying or nobody
//! answering each gives the hook what its agent reads. A hook that stops
//! holding withdraws its card, and with the daemon gone the hook prints
//! nothing and exits 0, so the agent's own prompt asks.

use humatad_test_common as common;

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use common::{
    Attach, Build, Daemon, WAIT, World, agent_tab, binary, create_channel, folder, open,
    shell_token, start_session,
};
use humata2_grpc::humata::v1 as pb;
use serde_json::Value;
use tonic::Streaming;

/// Claude's PermissionRequest payload (agent_hooks/testdata/claude).
const CLAUDE_BASH: &str = r#"{"session_id":"abc123","transcript_path":"/t.jsonl","cwd":"/work/api","permission_mode":"default","hook_event_name":"PermissionRequest","tool_name":"Bash","tool_input":{"command":"npm test -- --watch=false","description":"Run the test suite"}}"#;
/// Copilot's permissionRequest payload (agent_hooks/testdata/copilot).
const COPILOT_SHELL: &str = r#"{"sessionId":"c0p1","timestamp":1763251200000,"cwd":"/work/api","hook_event_name":"permissionRequest","toolName":"bash","toolInput":{"command":"rm -rf build","description":"Clean the build folder"}}"#;
/// Copilot's preToolUse payload for its ask-user form with two choices
/// (agent_hooks/testdata/copilot/ask_user_choices.json; D761).
const COPILOT_ASK: &str = r#"{"sessionId":"c0p1","timestamp":1794967800000,"cwd":"/work/api","toolName":"ask_user","toolArgs":{"message":"Which environment should I deploy to?","requestedSchema":{"type":"object","properties":{"target":{"type":"string","title":"Environment","enum":["staging","production"]}},"required":["target"]}}}"#;
/// Copilot's preToolUse payload for another tool: not a question.
const COPILOT_PRE_BASH: &str = r#"{"sessionId":"c0p1","timestamp":1794967810000,"cwd":"/work/api","toolName":"bash","toolArgs":{"command":"ls"}}"#;
/// Claude's PermissionRequest for Humata's own provider_import tool, whose
/// call only files an Inbox request.
const CLAUDE_PROVIDER_IMPORT: &str = r#"{"session_id":"abc123","transcript_path":"/t.jsonl","cwd":"/work/api","permission_mode":"default","hook_event_name":"PermissionRequest","tool_name":"mcp__humata__provider_import","tool_input":{"text":"[providers.box]\nlabel = \"Box\"\n","reason":"to add a box"}}"#;
/// Claude's PermissionRequest for provider_import via `humata_call` (MCPCOMPRESS).
const CLAUDE_HUMATA_CALL_PROVIDER_IMPORT: &str = r#"{"session_id":"abc123","transcript_path":"/t.jsonl","cwd":"/work/api","permission_mode":"default","hook_event_name":"PermissionRequest","tool_name":"mcp__humata__humata_call","tool_input":{"name":"provider_import","arguments":{"file_text":"[providers.box]\nlabel = \"Box\"\n","reason":"to add a box"}}}"#;
/// Copilot's permissionRequest for the same tool.
const COPILOT_PROVIDER_IMPORT: &str = r#"{"sessionId":"c0p1","timestamp":1763251200000,"cwd":"/work/api","hook_event_name":"permissionRequest","toolName":"humata/provider_import","toolInput":{"text":"[providers.box]\n"}}"#;
/// Copilot's permissionRequest for provider_import via `humata_call` (MCPCOMPRESS).
const COPILOT_HUMATA_CALL_PROVIDER_IMPORT: &str = r#"{"sessionId":"c0p1","timestamp":1763251200000,"cwd":"/work/api","hook_event_name":"permissionRequest","toolName":"humata/humata_call","toolInput":{"name":"provider_import","arguments":{"file_text":"[providers.box]\n"}}}"#;
/// Claude's Stop payload: not a permission ask.
const CLAUDE_STOP: &str = r#"{"session_id":"abc123","hook_event_name":"Stop","stop_hook_active":false}"#;

/// Claude's Notification payload for a permission prompt (D798).
const CLAUDE_PERMISSION_PROMPT: &str = r#"{"session_id":"abc123","transcript_path":"/t.jsonl","cwd":"/work/api","hook_event_name":"Notification","notification_type":"permission_prompt","message":"Claude needs your permission to use Bash"}"#;
/// The same hook for a Claude that only sits idle.
const CLAUDE_IDLE_PROMPT: &str = r#"{"session_id":"abc123","transcript_path":"/t.jsonl","cwd":"/work/api","hook_event_name":"Notification","notification_type":"idle_prompt","message":"Claude is waiting for your input"}"#;

/// The test agent: every line typed to it is answered, except
/// `run:<command>`, which it runs, as an agent runs a hook's command.
fn agent_script(world: &World) -> String {
    format!(
        r#"CONV_DIR='{}'
mode=$1; id=$2
case "$mode" in
  --session-id) f="$CONV_DIR/$id.jsonl"; : > "$f" ;;
  --resume) f="$CONV_DIR/$id.jsonl" ;;
esac
# Reads keys itself, as agent CLIs do, so typing never waits out the
# holder's keys limit (see common/mod.rs).
stty -icanon 2>/dev/null
echo "agent up $mode"
while IFS= read -r line; do
  case "$line" in
    run:*) eval "${{line#run:}}" ;;
    *) echo "got: $line" ;;
  esac
done
"#,
        world.conv.display(),
    )
}

struct Fixture {
    daemon: Daemon,
    session: pb::SessionId,
}

impl Fixture {
    async fn new() -> Self {
        let dir = folder();
        let world = World::new(dir.path());
        let script = dir.path().join("hook-agent.sh");
        std::fs::write(&script, agent_script(&world)).expect("write a test file");
        let agents = serde_json::json!([{
            "name": common::AGENT_NAME,
            "program": "/bin/sh",
            "start_args": [script, "--session-id", "{conversation}"],
            "resume_args": [script, "--resume", "{conversation}"],
            "conversation_file": format!("{}/{{conversation}}.jsonl", world.conv.display()),
            "turn_end": "hooks",
        }, {
            // D782: another kind, whose own switch is set apart.
            "name": OTHER_KIND,
            "program": "/bin/sh",
            "start_args": [script, "--session-id", "{conversation}"],
            "resume_args": [script, "--resume", "{conversation}"],
            "conversation_file": format!("{}/{{conversation}}.jsonl", world.conv.display()),
            "turn_end": "hooks",
        }]);
        std::fs::write(&world.agents, agents.to_string()).expect("write a test file");
        world.remote("acme", "api");
        let mut args = world.args();
        args.extend(["--humata".to_owned(), binary("HUMATA").display().to_string()]);
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let daemon = Daemon::start_with(Build::Admin, dir, &refs).await;
        let (channel, setup) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
        assert_eq!(setup.state, pb::OperationState::Succeeded as i32, "{:?}", setup.error);
        let (session, operation) = start_session(&daemon, &channel, "Fix login").await;
        assert_eq!(operation.state, pb::OperationState::Succeeded as i32, "{:?}", operation.error);
        Self { daemon, session }
    }

    async fn attach(&self) -> Attach {
        let tab = agent_tab(&self.daemon, &self.session).await;
        Attach::start(&self.daemon, open(&tab, 0)).await
    }

    /// The Claude permission hook command for this daemon.
    ///
    /// ATM-62 removed `PermissionRequest` from the packaged `claude-hooks.json`,
    /// so tests that exercise the `humata hook claude permission` CLI directly
    /// invoke it with this daemon's socket.
    fn claude_permission_command(&self) -> String {
        format!(
            "HUMATA_SOCKET='{}' '{}' hook claude permission || true",
            self.daemon.socket.display(),
            binary("HUMATA").display()
        )
    }

    /// The agent runs Claude's permission hook with `payload` on stdin; its
    /// stdout lands in the returned file, and `<file>.done` once it exits,
    /// holding its exit code.
    async fn fire_permission(&self, attach: &mut Attach, name: &str, payload: &str) -> PathBuf {
        let input = self.daemon.dir.path().join(format!("{name}.in"));
        std::fs::write(&input, payload).expect("write a test file");
        let out = self.daemon.dir.path().join(format!("{name}.out"));
        let command = self.claude_permission_command();
        attach
            .type_text(&format!(
                "run:{{ {command}; }} < '{}' > '{}'; echo $? > '{}.done'\r",
                input.display(),
                out.display(),
                out.display()
            ))
            .await;
        out
    }

    /// The pending Inbox requests of this session.
    async fn pending(&self) -> Vec<pb::InboxRequest> {
        let listed = pb::inbox_service_client::InboxServiceClient::new(self.daemon.channel().await)
            .list_requests(pb::ListRequestsRequest::default())
            .await
            .expect("list Inbox requests")
            .into_inner();
        listed
            .requests
            .into_iter()
            .filter(|request| request.state == pb::RequestState::Pending as i32)
            .collect()
    }

    /// Waits for `count` pending requests and returns them.
    async fn wait_pending(&self, count: usize) -> Vec<pb::InboxRequest> {
        let deadline = Instant::now() + WAIT;
        loop {
            let pending = self.pending().await;
            if pending.len() == count {
                return pending;
            }
            assert!(
                Instant::now() < deadline,
                "wanted {count} pending requests, have {}",
                pending.len()
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn request(&self, id: &pb::RequestId) -> pb::InboxRequest {
        pb::inbox_service_client::InboxServiceClient::new(self.daemon.channel().await)
            .get_request(pb::GetRequestRequest {
                id: Some(id.clone()),
            })
            .await
            .expect("get the Inbox request")
            .into_inner()
    }

    async fn answer(
        &self,
        id: &pb::RequestId,
        answer: pb::request_answer::Answer,
    ) -> Result<pb::InboxRequest, tonic::Status> {
        pb::inbox_service_client::InboxServiceClient::new(self.daemon.channel().await)
            .answer_request(pb::AnswerRequestRequest {
                id: Some(id.clone()),
                answer: Some(pb::RequestAnswer {
                    answer: Some(answer),
                }),
                ..Default::default()
            })
            .await
            .map(tonic::Response::into_inner)
    }

    /// Holds the Hook stream directly, as the session's agent.
    async fn hold(&self, token: &str, agent: &str, payload: &str) -> Streaming<pb::HookReply> {
        pb::session_service_client::SessionServiceClient::new(self.daemon.channel().await)
            .hook(as_agent(
                pb::HookRequest {
                    agent: agent.to_owned(),
                    event: "permission".to_owned(),
                    payload: payload.as_bytes().to_vec(),
                },
                token,
            ))
            .await
            .expect("open the Hook stream")
            .into_inner()
    }

    /// Holds the Hook stream for the agent's ask-user form (D761).
    async fn hold_question(&self, token: &str, payload: &str) -> Streaming<pb::HookReply> {
        pb::session_service_client::SessionServiceClient::new(self.daemon.channel().await)
            .hook(as_agent(
                pb::HookRequest {
                    agent: "copilot".to_owned(),
                    event: "question".to_owned(),
                    payload: payload.as_bytes().to_vec(),
                },
                token,
            ))
            .await
            .expect("open the Hook stream")
            .into_inner()
    }

    async fn status(&self) -> pb::SessionStatus {
        common::get_session(&self.daemon, &self.session).await.status()
    }

    /// Waits until the session no longer needs you.
    async fn wait_not_needing_you(&self) {
        let deadline = Instant::now() + WAIT;
        while self.status().await == pb::SessionStatus::NeedsInput {
            assert!(Instant::now() < deadline, "the session still needs you");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// The user sets Settings › Agents › `kind` › Ask in the Inbox (D782).
    async fn asks_in_inbox(&self, kind: &str, on: bool) {
        let shown = pb::agent_service_client::AgentServiceClient::new(self.daemon.channel().await)
            .set_agent_enabled(pb::SetAgentEnabledRequest {
                agent: Some(pb::AgentId {
                    value: kind.to_owned(),
                }),
                asks_in_inbox: Some(on),
                ..Default::default()
            })
            .await
            .expect("the user changes Settings › Agents")
            .into_inner();
        assert_eq!(shown.asks_in_inbox, on);
    }

    /// The session's agent asks the user through the Inbox (AskUser).
    async fn ask_user(&self, token: &str) -> pb::RequestState {
        pb::inbox_service_client::InboxServiceClient::new(self.daemon.channel().await)
            .ask_user(as_agent(
                pb::AskUserRequest {
                    question: "Deploy to staging?".to_owned(),
                    ..Default::default()
                },
                token,
            ))
            .await
            .expect("the agent asks the user")
            .into_inner()
            .state()
    }

    /// The session's agent asks ahead for network reach (RequestAccess).
    async fn request_access(&self, token: &str) -> pb::RequestState {
        pb::inbox_service_client::InboxServiceClient::new(self.daemon.channel().await)
            .request_access(as_agent(
                pb::RequestAccessRequest {
                    capability: pb::Capability::NetworkReach as i32,
                    target: Some(pb::GrantTarget {
                        target: Some(pb::grant_target::Target::Network(pb::NetworkTarget {
                            host: "localhost".to_owned(),
                            port: 5432,
                            machine: None,
                        })),
                    }),
                    reason: "to run the tests".to_owned(),
                    ..Default::default()
                },
                token,
            ))
            .await
            .expect("the agent asks ahead")
            .into_inner()
            .state()
    }
}

/// The second agent kind in the fixture's agents file (D782).
const OTHER_KIND: &str = "other";

fn as_agent<T>(message: T, token: &str) -> tonic::Request<T> {
    let mut request = tonic::Request::new(message);
    request.metadata_mut().insert(
        "humata-session-token",
        tonic::metadata::MetadataValue::try_from(token).expect("token is valid metadata"),
    );
    request
}

fn approve() -> pb::request_answer::Answer {
    pb::request_answer::Answer::Approve(pb::Approve {})
}

fn deny(note: &str) -> pb::request_answer::Answer {
    pb::request_answer::Answer::Deny(pb::Deny {
        note: note.to_owned(),
    })
}

/// What the hook printed, once it has exited 0.
async fn printed(out: &PathBuf) -> Vec<u8> {
    let done = PathBuf::from(format!("{}.done", out.display()));
    let deadline = Instant::now() + WAIT;
    loop {
        if let Ok(code) = std::fs::read_to_string(&done)
            && !code.trim().is_empty()
        {
            assert_eq!(code.trim(), "0", "the hook must exit 0");
            return std::fs::read(out).expect("read the hook output");
        }
        assert!(Instant::now() < deadline, "the hook never finished");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// The stdout bytes the stream ends with, skipping keepalives.
async fn last_reply(stream: &mut Streaming<pb::HookReply>) -> Vec<u8> {
    tokio::time::timeout(WAIT, async {
        while let Some(reply) = stream.message().await.expect("hook stream yields a message") {
            if !reply.keepalive {
                return reply.stdout;
            }
        }
        panic!("the stream ended with no reply");
    })
    .await
    .expect("no reply on the stream")
}

fn command_of(request: &pb::InboxRequest) -> String {
    match request.preview.as_ref().and_then(|preview| preview.preview.as_ref()) {
        Some(pb::request_preview::Preview::Command(command)) => command.command.clone(),
        other => panic!("not a command card: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_daemon_does_not_write_a_permission_request_claude_hook() {
    let fix = Fixture::new().await;
    let file = fix.daemon.state.join("claude-hooks.json");
    let text = std::fs::read_to_string(file).expect("read claude-hooks.json");
    let json: Value = serde_json::from_str(&text).expect("claude-hooks.json is JSON");
    assert!(
        json["hooks"].get("PermissionRequest").is_none(),
        "ATM-62: packaged claude-hooks.json must not register PermissionRequest"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_inbox_approval_reaches_the_waiting_hook() {
    let fix = Fixture::new().await;
    let mut attach = fix.attach().await;
    let out = fix.fire_permission(&mut attach, "approve", CLAUDE_BASH).await;
    let card = fix.wait_pending(1).await.remove(0);
    assert_eq!(card.kind, pb::RequestKind::Session as i32);
    assert_eq!(command_of(&card), "npm test -- --watch=false");
    assert!(!card.action_title.is_empty());

    fix.answer(card.id.as_ref().expect("card has an id"), approve()).await.expect("answer");
    let printed: Value = serde_json::from_slice(&printed(&out).await).expect("JSON");
    assert_eq!(printed["hookSpecificOutput"]["decision"]["behavior"], "allow");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_inbox_deny_reaches_the_hook_with_its_note() {
    let fix = Fixture::new().await;
    let mut attach = fix.attach().await;
    let out = fix.fire_permission(&mut attach, "deny", CLAUDE_BASH).await;
    let card = fix.wait_pending(1).await.remove(0);
    fix.answer(card.id.as_ref().expect("card has an id"), deny("run it against staging"))
        .await
        .expect("deny the card");
    let printed: Value = serde_json::from_slice(&printed(&out).await).expect("JSON");
    let decision = &printed["hookSpecificOutput"]["decision"];
    assert_eq!(decision["behavior"], "deny");
    assert!(decision["message"].as_str().expect("string").contains("run it against staging"));
}

#[tokio::test(flavor = "multi_thread")]
async fn each_agent_gets_the_answer_in_its_own_shape() {
    let fix = Fixture::new().await;
    let (token, _shell) = shell_token(&fix.daemon, &fix.session).await;
    let mut stream = fix.hold(&token, "copilot", COPILOT_SHELL).await;
    let card = fix.wait_pending(1).await.remove(0);
    assert_eq!(command_of(&card), "rm -rf build");
    fix.answer(card.id.as_ref().expect("card has an id"), approve()).await.expect("answer");
    let printed: Value = serde_json::from_slice(&last_reply(&mut stream).await).expect("JSON");
    assert_eq!(printed["behavior"], "allow");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_hook_that_stops_holding_withdraws_its_card_and_it_cant_be_answered() {
    let fix = Fixture::new().await;
    let (token, _shell) = shell_token(&fix.daemon, &fix.session).await;
    let stream = fix.hold(&token, "claude", CLAUDE_BASH).await;
    let card = fix.wait_pending(1).await.remove(0);
    let id = card.id.clone().expect("card has an id");
    // The agent's own timeout killed the hook: the TUI asks now.
    drop(stream);
    let deadline = Instant::now() + WAIT;
    loop {
        let now = fix.request(&id).await;
        if now.state == pb::RequestState::Withdrawn as i32 {
            assert_eq!(now.withdraw_reason, pb::WithdrawReason::CallEnded as i32);
            break;
        }
        assert!(Instant::now() < deadline, "the card was never withdrawn");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(fix.answer(&id, approve()).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_same_ask_held_twice_is_one_card_and_one_answer_reaches_both() {
    let fix = Fixture::new().await;
    let (token, _shell) = shell_token(&fix.daemon, &fix.session).await;
    let mut first = fix.hold(&token, "claude", CLAUDE_BASH).await;
    fix.wait_pending(1).await;
    let mut second = fix.hold(&token, "claude", CLAUDE_BASH).await;
    // Give a second card time to show up, if one were made.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let card = fix.wait_pending(1).await.remove(0);
    fix.answer(card.id.as_ref().expect("card has an id"), approve()).await.expect("answer");
    for stream in [&mut first, &mut second] {
        let printed: Value = serde_json::from_slice(&last_reply(stream).await).expect("JSON");
        assert_eq!(printed["hookSpecificOutput"]["decision"]["behavior"], "allow");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_payload_that_isnt_an_ask_gets_no_decision_and_no_card() {
    let fix = Fixture::new().await;
    let (token, _shell) = shell_token(&fix.daemon, &fix.session).await;
    for (agent, payload) in [
        ("claude", CLAUDE_STOP),
        ("claude", "not json"),
        ("gemini", CLAUDE_BASH),
        // Another agent's shape isn't guessed at.
        ("copilot", CLAUDE_BASH),
    ] {
        let mut stream = fix.hold(&token, agent, payload).await;
        assert!(last_reply(&mut stream).await.is_empty(), "{agent}: {payload}");
    }
    assert!(fix.pending().await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn humatas_own_tool_that_files_an_inbox_request_is_allowed_with_no_card_of_its_own() {
    let fix = Fixture::new().await;
    let (token, _shell) = shell_token(&fix.daemon, &fix.session).await;
    for payload in [CLAUDE_PROVIDER_IMPORT, CLAUDE_HUMATA_CALL_PROVIDER_IMPORT] {
        let mut claude = fix.hold(&token, "claude", payload).await;
        let printed: Value = serde_json::from_slice(&last_reply(&mut claude).await).expect("JSON");
        assert_eq!(printed["hookSpecificOutput"]["decision"]["behavior"], "allow");
    }
    for payload in [COPILOT_PROVIDER_IMPORT, COPILOT_HUMATA_CALL_PROVIDER_IMPORT] {
        let mut copilot = fix.hold(&token, "copilot", payload).await;
        let printed: Value = serde_json::from_slice(&last_reply(&mut copilot).await).expect("JSON");
        assert_eq!(printed["behavior"], "allow");
    }
    assert!(fix.pending().await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn only_the_sessions_own_agent_may_hold_a_hook() {
    let fix = Fixture::new().await;
    let refused = pb::session_service_client::SessionServiceClient::new(fix.daemon.channel().await)
        .hook(pb::HookRequest {
            agent: "claude".to_owned(),
            event: "permission".to_owned(),
            payload: CLAUDE_BASH.as_bytes().to_vec(),
        })
        .await;
    assert!(refused.is_err());
    assert!(fix.pending().await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn answer_all_leaves_permission_cards_alone() {
    let fix = Fixture::new().await;
    let (token, _shell) = shell_token(&fix.daemon, &fix.session).await;
    let _stream = fix.hold(&token, "claude", CLAUDE_BASH).await;
    let card = fix.wait_pending(1).await.remove(0);
    let _ = pb::inbox_service_client::InboxServiceClient::new(fix.daemon.channel().await)
        .answer_all(pb::AnswerAllRequest {
            ids: vec![card.id.clone().expect("card has an id")],
            approve: true,
            ..Default::default()
        })
        .await;
    assert_eq!(
        fix.request(card.id.as_ref().expect("card has an id")).await.state,
        pb::RequestState::Pending as i32
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn with_the_daemon_gone_the_hook_prints_nothing_and_exits_0() {
    let mut fix = Fixture::new().await;
    let command = fix.claude_permission_command();
    fix.daemon.kill();
    let mut child = Command::new("/bin/sh")
        .arg("-c")
        .arg(&command)
        .env("HUMATA_SESSION_TOKEN", "0123456789abcdef0123456789abcdef")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn the hook");
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(CLAUDE_BASH.as_bytes())
        .expect("write hook stdin");
    let output = child.wait_with_output().expect("hook process exits");
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty());
}

/// D782: each agent kind has its own "Ask in the Inbox". Another kind's
/// switch off leaves this one's asks in the Inbox. With this kind's off,
/// its permission hook and its ask-user form get no decision at once, so
/// its own terminal asks, and raise no card; its AskUser and RequestAccess
/// still wait in the Inbox. Turning it back on raises cards again, with no
/// restart.
#[tokio::test(flavor = "multi_thread")]
async fn an_agent_whose_asks_are_off_asks_in_its_terminal_and_the_others_still_ask_in_the_inbox() {
    let fix = Fixture::new().await;
    let (token, _shell) = shell_token(&fix.daemon, &fix.session).await;
    fix.asks_in_inbox(OTHER_KIND, false).await;
    let _first = fix.hold(&token, "claude", CLAUDE_BASH).await;
    fix.wait_pending(1).await;

    fix.asks_in_inbox(common::AGENT_NAME, false).await;
    let mut permission = fix.hold(&token, "copilot", COPILOT_SHELL).await;
    assert!(last_reply(&mut permission).await.is_empty());
    let mut question = fix.hold_question(&token, COPILOT_ASK).await;
    assert!(last_reply(&mut question).await.is_empty());
    assert_eq!(fix.pending().await.len(), 1);

    assert_eq!(fix.ask_user(&token).await, pb::RequestState::Pending);
    assert_eq!(fix.request_access(&token).await, pb::RequestState::Pending);
    fix.wait_pending(3).await;

    fix.asks_in_inbox(common::AGENT_NAME, true).await;
    let _again = fix.hold(&token, "copilot", COPILOT_SHELL).await;
    fix.wait_pending(4).await;
}

/// D782: every agent asks in the Inbox until the user says otherwise, and
/// D199's one switch reads as off.
#[tokio::test(flavor = "multi_thread")]
async fn asks_go_to_the_inbox_by_default() {
    let fix = Fixture::new().await;
    let listed = pb::agent_service_client::AgentServiceClient::new(fix.daemon.channel().await)
        .list_agents(pb::ListAgentsRequest::default())
        .await
        .expect("list the agents")
        .into_inner();
    assert!(
        listed
            .agents
            .iter()
            .filter(|agent| agent.id.as_ref().is_some_and(|id| id.value != "zsh"))
            .all(|agent| agent.asks_in_inbox)
    );
    let shown =
        pb::settings_service_client::SettingsServiceClient::new(fix.daemon.channel().await)
            .get_settings(pb::GetSettingsRequest {})
            .await
            .expect("read the settings")
            .into_inner();
    assert_eq!(shown.agents.map(|agents| agents.permissions_in_agent), Some(false));
}

/// D782: D199's one switch still works, by setting every kind's own.
#[tokio::test(flavor = "multi_thread")]
async fn the_old_one_switch_sets_every_agents_own() {
    let fix = Fixture::new().await;
    let (token, _shell) = shell_token(&fix.daemon, &fix.session).await;
    let shown =
        pb::settings_service_client::SettingsServiceClient::new(fix.daemon.channel().await)
            .update_settings(pb::UpdateSettingsRequest {
                agents: Some(pb::AgentSettings {
                    permissions_in_agent: true,
                    ..Default::default()
                }),
                ..Default::default()
            })
            .await
            .expect("the user changes Settings › Agents")
            .into_inner();
    assert_eq!(shown.agents.map(|agents| agents.permissions_in_agent), Some(true));
    let mut stream = fix.hold(&token, "claude", CLAUDE_BASH).await;
    assert!(last_reply(&mut stream).await.is_empty());
    assert!(fix.pending().await.is_empty());
}

fn reply(text: &str) -> pb::request_answer::Answer {
    pb::request_answer::Answer::Reply(pb::Reply {
        text: text.to_owned(),
        selected_choices: Vec::new(),
    })
}

fn question_of(request: &pb::InboxRequest) -> pb::QuestionPreview {
    match request.preview.as_ref().and_then(|preview| preview.preview.as_ref()) {
        Some(pb::request_preview::Preview::Question(question)) => question.clone(),
        other => panic!("not a question card: {other:?}"),
    }
}

/// D761: Copilot's ask-user form raises one Inbox question with all of it,
/// the session needs you while it waits, and the reply reaches Copilot.
#[tokio::test(flavor = "multi_thread")]
async fn copilots_ask_user_form_is_an_inbox_question_and_the_reply_reaches_it() {
    let fix = Fixture::new().await;
    let (token, _shell) = shell_token(&fix.daemon, &fix.session).await;
    assert_ne!(fix.status().await, pb::SessionStatus::NeedsInput);
    let mut stream = fix.hold_question(&token, COPILOT_ASK).await;

    let card = fix.wait_pending(1).await.remove(0);
    assert_eq!(card.kind, pb::RequestKind::Question as i32);
    let question = question_of(&card);
    assert!(question.question.contains("Which environment should I deploy to?"));
    assert_eq!(question.choices, ["staging", "production"]);
    common::wait_status(&fix.daemon, &fix.session, pb::SessionStatus::NeedsInput).await;

    let id = card.id.clone().expect("card has an id");
    // A question takes a reply, not an approval.
    assert!(fix.answer(&id, approve()).await.is_err());
    fix.answer(&id, reply("staging")).await.expect("reply to the question");
    let printed: Value = serde_json::from_slice(&last_reply(&mut stream).await).expect("JSON");
    assert_eq!(printed["permissionDecision"], "deny");
    assert!(printed["permissionDecisionReason"].as_str().expect("string").contains("staging"));
    assert_eq!(fix.request(&id).await.state, pb::RequestState::Answered as i32);
    fix.wait_not_needing_you().await;
}

/// D761: declining the question tells Copilot so and ends the wait.
#[tokio::test(flavor = "multi_thread")]
async fn declining_copilots_question_tells_it_and_ends_the_wait() {
    let fix = Fixture::new().await;
    let (token, _shell) = shell_token(&fix.daemon, &fix.session).await;
    let mut stream = fix.hold_question(&token, COPILOT_ASK).await;
    let card = fix.wait_pending(1).await.remove(0);
    fix.answer(card.id.as_ref().expect("card has an id"), deny("not today"))
        .await
        .expect("decline the question");
    let printed: Value = serde_json::from_slice(&last_reply(&mut stream).await).expect("JSON");
    assert_eq!(printed["permissionDecision"], "deny");
    assert!(printed["permissionDecisionReason"].as_str().expect("string").contains("not today"));
    fix.wait_not_needing_you().await;
}

/// D761: a question whose hook lets go is withdrawn, and the session no
/// longer needs you; another tool's preToolUse raises nothing.
#[tokio::test(flavor = "multi_thread")]
async fn a_question_whose_hook_lets_go_is_withdrawn_and_another_tool_raises_none() {
    let fix = Fixture::new().await;
    let (token, _shell) = shell_token(&fix.daemon, &fix.session).await;
    let mut other = fix.hold_question(&token, COPILOT_PRE_BASH).await;
    assert!(last_reply(&mut other).await.is_empty());
    assert!(fix.pending().await.is_empty());

    let stream = fix.hold_question(&token, COPILOT_ASK).await;
    let card = fix.wait_pending(1).await.remove(0);
    let id = card.id.clone().expect("card has an id");
    common::wait_status(&fix.daemon, &fix.session, pb::SessionStatus::NeedsInput).await;
    drop(stream);
    let deadline = Instant::now() + WAIT;
    while fix.request(&id).await.state != pb::RequestState::Withdrawn as i32 {
        assert!(Instant::now() < deadline, "the question was never withdrawn");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    fix.wait_not_needing_you().await;
}

impl Fixture {
    /// Sends the session's agent's Notification hook and returns its reply.
    async fn notify(&self, token: &str, payload: &str) -> Vec<u8> {
        let mut stream = pb::session_service_client::SessionServiceClient::new(self.daemon.channel().await)
            .hook(as_agent(
                pb::HookRequest {
                    agent: "claude".to_owned(),
                    event: "notification".to_owned(),
                    payload: payload.as_bytes().to_vec(),
                },
                token,
            ))
            .await
            .expect("the notification hook is taken")
            .into_inner();
        last_reply(&mut stream).await
    }

    /// The session's agent reports its turn, as `humata turn` does.
    async fn report_turn(&self, token: &str, state: pb::TurnState) {
        pb::session_service_client::SessionServiceClient::new(self.daemon.channel().await)
            .report_turn(as_agent(
                pb::ReportTurnRequest {
                    state: state as i32,
                    reported_at_unix_ns: 0,
                },
                token,
            ))
            .await
            .expect("the turn is reported");
    }

    async fn wait_withdrawn(&self, id: &pb::RequestId) -> pb::InboxRequest {
        let deadline = Instant::now() + WAIT;
        loop {
            let now = self.request(id).await;
            if now.state == pb::RequestState::Withdrawn as i32 {
                return now;
            }
            assert!(Instant::now() < deadline, "the card was never withdrawn");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

/// D798: a Claude Notification of a permission prompt, with no ask held
/// for the session, raises one card that says what Claude shows; the hook
/// itself ends at once with no decision.
#[tokio::test(flavor = "multi_thread")]
async fn a_claude_permission_prompt_notification_raises_one_card() {
    let fix = Fixture::new().await;
    let (token, _shell) = shell_token(&fix.daemon, &fix.session).await;
    assert!(fix.notify(&token, CLAUDE_PERMISSION_PROMPT).await.is_empty());
    let card = fix.wait_pending(1).await.remove(0);
    assert!(command_of(&card).contains("needs your permission"));
    // A second notice for the same wait is the same card.
    assert!(fix.notify(&token, CLAUDE_PERMISSION_PROMPT).await.is_empty());
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(fix.pending().await.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_idle_prompt_or_an_unreadable_notification_raises_no_card() {
    let fix = Fixture::new().await;
    let (token, _shell) = shell_token(&fix.daemon, &fix.session).await;
    for payload in [CLAUDE_IDLE_PROMPT, "not json", CLAUDE_STOP] {
        assert!(fix.notify(&token, payload).await.is_empty(), "{payload}");
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(fix.pending().await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_notification_raises_no_card_when_the_agents_asks_stay_in_its_terminal() {
    let fix = Fixture::new().await;
    let (token, _shell) = shell_token(&fix.daemon, &fix.session).await;
    fix.asks_in_inbox(common::AGENT_NAME, false).await;
    assert!(fix.notify(&token, CLAUDE_PERMISSION_PROMPT).await.is_empty());
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(fix.pending().await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_notification_card_is_withdrawn_when_the_session_reports_a_turn() {
    let fix = Fixture::new().await;
    let (token, _shell) = shell_token(&fix.daemon, &fix.session).await;
    for state in [pb::TurnState::Working, pb::TurnState::Idle] {
        fix.notify(&token, CLAUDE_PERMISSION_PROMPT).await;
        let id = fix.wait_pending(1).await.remove(0).id.expect("card has an id");
        fix.report_turn(&token, state).await;
        fix.wait_withdrawn(&id).await;
        assert!(fix.answer(&id, deny("")).await.is_err());
    }
}

/// D798: every held ask goes at a turn's end, though its hook stays; an
/// answer that comes after does nothing, and the hook gets no decision.
#[tokio::test(flavor = "multi_thread")]
async fn a_held_permission_card_is_withdrawn_as_resolved_when_the_turn_ends() {
    let fix = Fixture::new().await;
    let (token, _shell) = shell_token(&fix.daemon, &fix.session).await;
    let mut stream = fix.hold(&token, "claude", CLAUDE_BASH).await;
    let id = fix.wait_pending(1).await.remove(0).id.expect("card has an id");
    fix.report_turn(&token, pb::TurnState::Idle).await;
    let now = fix.wait_withdrawn(&id).await;
    assert_eq!(now.withdraw_reason, pb::WithdrawReason::Resolved as i32);
    assert!(fix.answer(&id, approve()).await.is_err());
    assert!(last_reply(&mut stream).await.is_empty());
}
