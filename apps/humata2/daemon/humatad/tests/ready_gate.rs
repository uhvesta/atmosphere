//! D797: Humata's own sends to an agent with a structured ready signal wait
//! for that signal, not for its screen, through a real `humatad2`. Claude's
//! signal is a SessionStart hook of any source (it runs only once workspace
//! trust is accepted); Codex's is the wrapper's thread/started. A start's
//! first message is held until the session is heard, then sent once. Keys
//! the person types into the terminal always pass. Twenty seconds after the
//! launch the Inbox says the agent is waiting in its terminal, with the held
//! message; "Send it now" sends it by the person's choice, and the signal
//! withdraws the card and sends it. A kind with no signal keeps the screen
//! check (D762). Once a session's structured channel is heard, a question
//! on its agent's screen raises no screen card; before, it still does
//! (D798).

use humatad_test_common as common;
use humatad_test_common::trust_box;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::{
    Attach, Build, Daemon, WAIT, World, agent_tab, binary, create_channel, folder, open,
    shell_token,
};
use humata2_grpc::humata::v1 as pb;
use serde_json::Value;

/// D797: how long after its launch a session not heard from raises the
/// "waiting in its terminal" card.
const CARD_AFTER: Duration = Duration::from_secs(20);

/// Long past the time the screen check alone would have sent a first
/// message into an input that shows clear.
const SCREEN_WOULD_HAVE_SENT: Duration = Duration::from_secs(4);

/// The test agent: its screen shows a clear input from the start, as a
/// started agent does, and it runs each `run:` line it is sent.
fn agent_script(world: &World) -> String {
    format!(
        r#"CONV_DIR='{}'
mode=$1; id=$2
case "$mode" in
  --session-id) f="$CONV_DIR/$id.jsonl"; : > "$f" ;;
  --resume) f="$CONV_DIR/$id.jsonl" ;;
esac
printf '\033[H\033[2J'
echo "agent up $mode"
printf '❯ '
while IFS= read -r line; do
  case "$line" in
    run:*) eval "${{line#run:}}" ;;
    *) echo "got: $line" ;;
  esac
  printf '❯ '
done
"#,
        world.conv.display(),
    )
}

struct Fixture {
    daemon: Daemon,
    session: pb::SessionId,
    /// Where the first message, once run, writes `sent`.
    marker: PathBuf,
}

impl Fixture {
    /// A started session whose agent writes conversations in `format`, with
    /// a first message that marks it was run.
    async fn start(format: &str) -> Self {
        let dir = folder();
        let world = World::new(dir.path());
        let script = dir.path().join("ready-agent.sh");
        std::fs::write(&script, agent_script(&world)).expect("write a test file");
        let agents = serde_json::json!([{
            "name": common::AGENT_NAME,
            "program": "/bin/sh",
            "start_args": [script, "--session-id", "{conversation}"],
            "resume_args": [script, "--resume", "{conversation}"],
            "conversation_file": format!("{}/{{conversation}}.jsonl", world.conv.display()),
            "conversation_format": format,
            "turn_end": "hooks",
        }]);
        std::fs::write(&world.agents, agents.to_string()).expect("write a test file");
        world.remote("acme", "api");
        let mut args = world.args();
        args.extend(["--humata".to_owned(), binary("HUMATA").display().to_string()]);
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let marker = dir.path().join("first-message-ran");
        let daemon = Daemon::start_with(Build::Admin, dir, &refs).await;
        let (channel, setup) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
        assert_eq!(setup.state, pb::OperationState::Succeeded as i32, "{:?}", setup.error);
        let machine = common::machine_id(&daemon).await;
        let mut request =
            common::start_request(channel.id.as_ref().expect("channel id"), &machine, "Fix login");
        request.first_message = format!("run:echo sent >> '{}'", marker.display());
        let started = daemon
            .session_service()
            .await
            .start_session(request)
            .await
            .expect("start the session")
            .into_inner();
        let operation = common::finished(&daemon, &started.operation.expect("an operation")).await;
        assert_eq!(operation.state, pb::OperationState::Succeeded as i32, "{:?}", operation.error);
        Self {
            daemon,
            session: started.session.expect("a session"),
            marker,
        }
    }

    /// The command Claude runs for its SessionStart hook to name its
    /// conversation, as the daemon wrote it into Claude's hooks file.
    fn session_start_hook(&self) -> String {
        let file = self.daemon.state.join("claude-hooks.json");
        let text = std::fs::read_to_string(&file).expect("read Claude's hooks file");
        let hooks: Value = serde_json::from_str(&text).expect("claude-hooks.json is JSON");
        hooks["hooks"]["SessionStart"]
            .as_array()
            .expect("SessionStart hooks")
            .iter()
            .filter_map(|group| group["hooks"].as_array())
            .flatten()
            .filter_map(|hook| hook["command"].as_str())
            .find(|command| command.contains("conversation"))
            .expect("a SessionStart hook that names the conversation")
            .to_owned()
    }

    /// Claude, once its workspace trust is accepted, runs its SessionStart
    /// hook in the session; returns once the hook has exited.
    async fn claude_starts(&self, shell: &mut Attach) {
        let payload = serde_json::json!({
            "session_id": "7c1e2a90-5b3d-4f6e-9a21-0d8c4b7e3f15",
            "transcript_path": "/home/u/.claude/projects/-work-api/7c1e2a90-5b3d-4f6e-9a21-0d8c4b7e3f15.jsonl",
            "cwd": "/work/api",
            "hook_event_name": "SessionStart",
            "source": "startup",
        });
        self.run_hook(shell, &self.session_start_hook(), &payload, "claude-start").await;
    }

    /// The `humata codex` wrapper names the thread Codex's app-server
    /// announced with thread/started, as `humata hook codex conversation`.
    async fn codex_starts(&self, shell: &mut Attach) {
        let payload = serde_json::json!({
            "session_id": "019b2c4d-7e8f-7a01-b234-56789abcdef0",
            "source": "thread/started",
        });
        let command = self
            .session_start_hook()
            .replace("hook claude conversation", "hook codex conversation");
        self.run_hook(shell, &command, &payload, "codex-start").await;
    }

    /// Claude's SessionStart hook, run by the agent itself in its own
    /// terminal, as Claude runs its hooks.
    async fn agent_claude_starts(&self, agent: &mut Attach) {
        let payload = serde_json::json!({
            "session_id": "7c1e2a90-5b3d-4f6e-9a21-0d8c4b7e3f15",
            "cwd": "/work/api",
            "hook_event_name": "SessionStart",
            "source": "startup",
        });
        let input = self.daemon.dir.path().join("agent-start.in");
        std::fs::write(&input, payload.to_string()).expect("write a test file");
        let done = self.daemon.dir.path().join("agent-start.done");
        agent
            .type_text(&format!(
                "run:{{ {}; }} < '{}'; echo $? > '{}'\r",
                self.session_start_hook(),
                input.display(),
                done.display(),
            ))
            .await;
        common::wait_for_path(&done).await;
    }

    /// The agent draws Copilot's folder trust box on its screen and waits
    /// in it for an answer.
    async fn agent_draws_trust_box(&self, agent: &mut Attach) {
        let draw = self.daemon.dir.path().join("draw-trust.sh");
        std::fs::write(
            &draw,
            format!(
                "printf '%s\\n' \\\n{} \\\n{}\nIFS= read -r answer\n",
                trust_box::BOX_TOP,
                trust_box::BOX_REST
            ),
        )
        .expect("write a test file");
        agent.type_text(&format!("run:. '{}'\r", draw.display())).await;
    }

    async fn run_hook(&self, shell: &mut Attach, command: &str, payload: &Value, name: &str) {
        let input = self.daemon.dir.path().join(format!("{name}.in"));
        std::fs::write(&input, payload.to_string()).expect("write a test file");
        let done = self.daemon.dir.path().join(format!("{name}.done"));
        shell
            .type_text(&format!(
                "{{ {command}; }} < '{}'; echo $? > '{}'\r",
                input.display(),
                done.display(),
            ))
            .await;
        common::wait_for_path(&done).await;
    }

    async fn pending(&self) -> Vec<pb::InboxRequest> {
        pb::inbox_service_client::InboxServiceClient::new(self.daemon.channel().await)
            .list_requests(pb::ListRequestsRequest::default())
            .await
            .expect("list Inbox requests")
            .into_inner()
            .requests
            .into_iter()
            .filter(|request| request.state == pb::RequestState::Pending as i32)
            .collect()
    }

    /// Waits past [`CARD_AFTER`] for the one "waiting in its terminal" card.
    async fn wait_card(&self) -> pb::InboxRequest {
        let deadline = Instant::now() + CARD_AFTER + WAIT;
        loop {
            let mut pending = self.pending().await;
            assert!(pending.len() <= 1, "more than one card: {pending:?}");
            if let Some(card) = pending.pop() {
                return card;
            }
            assert!(Instant::now() < deadline, "no waiting card came");
            tokio::time::sleep(Duration::from_millis(200)).await;
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

    async fn status(&self) -> pb::SessionStatus {
        common::get_session(&self.daemon, &self.session).await.status()
    }

    fn sent(&self) -> bool {
        self.marker.exists()
    }

    /// Waits for the first message to have run, then checks it ran once.
    async fn wait_sent_once(&self) {
        let deadline = Instant::now() + WAIT;
        while !self.sent() {
            assert!(Instant::now() < deadline, "the held first message was never sent");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
        let runs = std::fs::read_to_string(&self.marker).expect("read the marker");
        assert_eq!(runs.lines().count(), 1, "the first message was sent more than once");
    }
}

/// The card's id, once it names `session`.
fn card_of(card: &pb::InboxRequest, session: &pb::SessionId) -> pb::RequestId {
    let named = card.requester.as_ref().and_then(|actor| actor.session.as_ref());
    assert_eq!(named, Some(session), "the card names another session");
    card.id.clone().expect("a request id")
}

async fn wait_path(path: &Path) {
    common::wait_for_path(path).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn claudes_first_message_waits_for_its_session_start_not_a_clear_screen() {
    let fix = Fixture::start("claude-jsonl").await;
    let (_token, mut shell) = shell_token(&fix.daemon, &fix.session).await;
    tokio::time::sleep(SCREEN_WOULD_HAVE_SENT).await;
    assert!(!fix.sent(), "the first message was sent before Claude was heard");

    fix.claude_starts(&mut shell).await;
    fix.wait_sent_once().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn codexs_first_message_waits_for_its_thread_not_a_clear_screen() {
    let fix = Fixture::start("codex-rollout").await;
    let (_token, mut shell) = shell_token(&fix.daemon, &fix.session).await;
    tokio::time::sleep(SCREEN_WOULD_HAVE_SENT).await;
    assert!(!fix.sent(), "the first message was sent before Codex was heard");

    fix.codex_starts(&mut shell).await;
    fix.wait_sent_once().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn keys_typed_into_the_terminal_pass_while_the_first_message_waits() {
    let fix = Fixture::start("claude-jsonl").await;
    let tab = agent_tab(&fix.daemon, &fix.session).await;
    let mut attach = Attach::start(&fix.daemon, open(&tab, 0)).await;
    let typed = fix.daemon.dir.path().join("typed");
    attach
        .type_text(&format!("run:echo typed > '{}'\r", typed.display()))
        .await;
    wait_path(&typed).await;
    assert!(!fix.sent(), "the person's keys released the held first message");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_not_heard_from_raises_a_waiting_card_and_send_it_now_sends_the_message() {
    let fix = Fixture::start("claude-jsonl").await;
    let card = fix.wait_card().await;
    let id = card_of(&card, &fix.session);
    assert_eq!(fix.status().await, pb::SessionStatus::NeedsInput);
    assert!(!fix.sent(), "the message was sent before the person chose to");

    pb::inbox_service_client::InboxServiceClient::new(fix.daemon.channel().await)
        .answer_request(pb::AnswerRequestRequest {
            id: Some(id.clone()),
            answer: Some(pb::RequestAnswer {
                answer: Some(pb::request_answer::Answer::Approve(pb::Approve {})),
            }),
            ..Default::default()
        })
        .await
        .expect("the person picks Send it now");
    fix.wait_sent_once().await;
    assert_ne!(fix.request(&id).await.state, pb::RequestState::Pending as i32);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_signal_after_the_card_withdraws_it_and_sends_the_message() {
    let fix = Fixture::start("claude-jsonl").await;
    let (_token, mut shell) = shell_token(&fix.daemon, &fix.session).await;
    let card = fix.wait_card().await;
    let id = card_of(&card, &fix.session);

    fix.claude_starts(&mut shell).await;
    fix.wait_sent_once().await;
    let deadline = Instant::now() + WAIT;
    while fix.request(&id).await.state == pb::RequestState::Pending as i32 {
        assert!(Instant::now() < deadline, "the waiting card was never withdrawn");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(fix.pending().await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_heard_before_the_card_is_due_raises_none() {
    let fix = Fixture::start("claude-jsonl").await;
    let (_token, mut shell) = shell_token(&fix.daemon, &fix.session).await;
    fix.claude_starts(&mut shell).await;
    fix.wait_sent_once().await;
    tokio::time::sleep(CARD_AFTER + Duration::from_secs(2)).await;
    assert!(fix.pending().await.is_empty(), "a card came for a session already heard");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_with_no_ready_signal_keeps_the_screen_check() {
    let fix = Fixture::start("none").await;
    fix.wait_sent_once().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_question_on_the_screen_of_a_session_heard_from_raises_no_screen_card() {
    let fix = Fixture::start("claude-jsonl").await;
    let tab = agent_tab(&fix.daemon, &fix.session).await;
    let mut agent = Attach::start(&fix.daemon, open(&tab, 0)).await;
    fix.agent_claude_starts(&mut agent).await;
    fix.wait_sent_once().await;

    fix.agent_draws_trust_box(&mut agent).await;
    agent.wait_for("Do you trust the files in this folder?").await;
    // Long past the time a screen card comes for an agent not heard from.
    tokio::time::sleep(SCREEN_WOULD_HAVE_SENT).await;
    assert!(fix.pending().await.is_empty(), "the screen fallback raised a card over the structured channel");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_question_on_the_screen_of_a_session_not_heard_from_still_raises_a_screen_card() {
    let fix = Fixture::start("claude-jsonl").await;
    let tab = agent_tab(&fix.daemon, &fix.session).await;
    let mut agent = Attach::start(&fix.daemon, open(&tab, 0)).await;
    fix.agent_draws_trust_box(&mut agent).await;
    let deadline = Instant::now() + WAIT * 3;
    loop {
        let questions = fix
            .pending()
            .await
            .into_iter()
            .filter(|card| {
                matches!(
                    card.preview.as_ref().and_then(|preview| preview.preview.as_ref()),
                    Some(pb::request_preview::Preview::Question(_))
                )
            })
            .count();
        if questions == 1 {
            break;
        }
        assert!(Instant::now() < deadline, "no screen card came for an agent not heard from");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// D797: a send of Humata's own (the composer, an Inbox reply and MCP's
/// session_type all type through Type) made while the first message waits
/// for the signal waits behind it, and goes after it once the session is
/// heard; it never reaches the agent first.
#[tokio::test(flavor = "multi_thread")]
async fn a_send_while_the_first_message_waits_goes_after_it_once_the_session_is_heard() {
    let fix = Fixture::start("claude-jsonl").await;
    let (_token, mut shell) = shell_token(&fix.daemon, &fix.session).await;
    let mut terminals = fix.daemon.terminals().await;
    let request = pb::TypeRequest {
        session: Some(fix.session.clone()),
        text: format!("run:echo second >> '{}'\n", fix.marker.display()),
        ..Default::default()
    };
    let send = tokio::spawn(async move { terminals.r#type(request).await });
    tokio::time::sleep(SCREEN_WOULD_HAVE_SENT).await;
    assert!(!fix.sent(), "a send reached the agent before it was heard");

    fix.claude_starts(&mut shell).await;
    send.await
        .expect("the send's task")
        .expect("the send goes through once the session is heard");
    let deadline = Instant::now() + WAIT;
    loop {
        let runs = std::fs::read_to_string(&fix.marker).unwrap_or_default();
        if runs.lines().count() >= 2 {
            assert_eq!(runs.lines().collect::<Vec<_>>(), ["sent", "second"], "out of order");
            break;
        }
        assert!(Instant::now() < deadline, "the first message and the send didn't both arrive: {runs:?}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// D798: when the Codex link drops, the screen fallback is back for the
/// session: a question drawn on its screen raises a screen card again.
#[tokio::test(flavor = "multi_thread")]
async fn a_dropped_link_brings_back_the_screen_card() {
    let fix = Fixture::start("codex-rollout").await;
    let (token, mut shell) = shell_token(&fix.daemon, &fix.session).await;
    fix.codex_starts(&mut shell).await;
    fix.wait_sent_once().await;

    let mut request = tonic::Request::new(pb::HookRequest {
        agent: "codex".to_owned(),
        event: "lost".to_owned(),
        payload: b"{}".to_vec(),
    });
    request.metadata_mut().insert(
        "humata-session-token",
        tonic::metadata::MetadataValue::try_from(token.as_str()).expect("token is valid metadata"),
    );
    let mut replies = fix
        .daemon
        .session_service()
        .await
        .hook(request)
        .await
        .expect("the link's drop is told")
        .into_inner();
    while let Some(reply) = replies.message().await.expect("the drop's reply") {
        if !reply.keepalive {
            break;
        }
    }

    let tab = agent_tab(&fix.daemon, &fix.session).await;
    let mut agent = Attach::start(&fix.daemon, open(&tab, 0)).await;
    fix.agent_draws_trust_box(&mut agent).await;
    let deadline = Instant::now() + WAIT * 3;
    while fix.pending().await.is_empty() {
        assert!(Instant::now() < deadline, "no screen card came once the link dropped");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}
