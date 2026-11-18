//! A question the agent's CLI asks on its own screen, such as Copilot's
//! folder trust, through a real `humatad2` (D767): it raises one Inbox
//! question with the question and every choice the screen offers, and the
//! session waits on you meanwhile. Humata types nothing until the person
//! answers in the Inbox; then it picks their choice. A question answered in
//! the terminal instead is withdrawn.

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::{
    Attach, Build, Daemon, WAIT, World, agent_tab, create_channel, folder, open, start_session,
};
use humata2_grpc::humata::v1 as pb;

const CHOICES: [&str; 3] = [
    "Yes",
    "Yes, and remember this folder for future sessions",
    "No",
];

/// The test agent asks Copilot 1.0.113's folder trust on its screen, then
/// keeps the bytes of the line it is answered with, as hex, in
/// `trust-answer`, and answers each line after as the other test agents do.
/// With `first`, it first draws only the start of the box, its first
/// choice alone, and stays quiet with it for longer than a quiet spell, as
/// Copilot was seen to before the rest of its box showed.
fn agent_script(world: &World, first: bool) -> String {
    let first = if first {
        r"printf '%s\n' \
  '│ Do you trust the files in this folder?                   │' \
  '│                                                          │' \
  '│ ❯ 1. Yes                                                 │'
sleep 6
printf '\033[H\033[2J'
"
    } else {
        ""
    };
    format!(
        r#"CONV_DIR='{}'
mode=$1; id=$2
case "$mode" in
  --session-id) f="$CONV_DIR/$id.jsonl"; : > "$f" ;;
  --resume) f="$CONV_DIR/$id.jsonl" ;;
esac
{first}printf '%s\n' \
  '╭──────────────────────────────────────────────────────────╮' \
  '│ Confirm folder trust                                     │' \
  '│ ╭──────────────────────────────────────────────────────╮ │' \
  '│ │ /work/api                                            │ │' \
  '│ ╰──────────────────────────────────────────────────────╯ │' \
  '│ Do you trust the files in this folder?                   │' \
  '│                                                          │' \
  '│ ❯ 1. Yes                                                 │' \
  '│   2. Yes, and remember this folder for future sessions   │' \
  '│   3. No (Esc)                                            │' \
  '╰──────────────────────────────────────────────────────────╯'
IFS= read -r answer
printf '%s' "$answer" | od -An -tx1 | tr -d ' \n' > "$CONV_DIR/trust-answer.part"
mv "$CONV_DIR/trust-answer.part" "$CONV_DIR/trust-answer"
printf '\033[H\033[2J'
echo "agent up $mode"
while IFS= read -r line; do echo "got: $line"; done
"#,
        world.conv.display(),
    )
}

struct Fixture {
    daemon: Daemon,
    session: pb::SessionId,
    answer: PathBuf,
}

impl Fixture {
    async fn new() -> Self {
        Self::drawing(false).await
    }

    async fn drawing(first: bool) -> Self {
        let dir = folder();
        let world = World::new(dir.path());
        let script = dir.path().join("trust-agent.sh");
        std::fs::write(&script, agent_script(&world, first)).expect("write a test file");
        let agents = serde_json::json!([{
            "name": common::AGENT_NAME,
            "program": "/bin/sh",
            "start_args": [script, "--session-id", "{conversation}"],
            "resume_args": [script, "--resume", "{conversation}"],
            "conversation_file": format!("{}/{{conversation}}.jsonl", world.conv.display()),
            "turn_end": "hooks",
        }]);
        std::fs::write(&world.agents, agents.to_string()).expect("write a test file");
        world.remote("acme", "api");
        let args = world.args();
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let answer = world.conv.join("trust-answer");
        let daemon = Daemon::start_with(Build::Admin, dir, &refs).await;
        let (channel, setup) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
        assert_eq!(setup.state, pb::OperationState::Succeeded as i32, "{:?}", setup.error);
        let (session, operation) = start_session(&daemon, &channel, "Fix login").await;
        assert_eq!(operation.state, pb::OperationState::Succeeded as i32, "{:?}", operation.error);
        Self {
            daemon,
            session,
            answer,
        }
    }

    async fn requests(&self) -> Vec<pb::InboxRequest> {
        pb::inbox_service_client::InboxServiceClient::new(self.daemon.channel().await)
            .list_requests(pb::ListRequestsRequest::default())
            .await
            .expect("list Inbox requests")
            .into_inner()
            .requests
    }

    /// Waits for the one pending Inbox request and returns it.
    async fn wait_pending(&self) -> pb::InboxRequest {
        self.wait_pending_where(|_| true).await
    }

    /// Waits for the one pending Inbox request to be one `wanted` takes.
    async fn wait_pending_where(&self, wanted: impl Fn(&pb::InboxRequest) -> bool) -> pb::InboxRequest {
        let deadline = Instant::now() + WAIT * 3;
        loop {
            let mut pending: Vec<_> = self
                .requests()
                .await
                .into_iter()
                .filter(|request| request.state == pb::RequestState::Pending as i32)
                .filter(|request| wanted(request))
                .collect();
            assert!(pending.len() <= 1, "more than one card: {pending:?}");
            if let Some(card) = pending.pop() {
                return card;
            }
            assert!(Instant::now() < deadline, "no Inbox request came");
            tokio::time::sleep(Duration::from_millis(100)).await;
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

    async fn status(&self) -> pb::SessionStatus {
        common::get_session(&self.daemon, &self.session).await.status()
    }

    async fn wait_not_needing_you(&self) {
        let deadline = Instant::now() + WAIT;
        while self.status().await == pb::SessionStatus::NeedsInput {
            assert!(Instant::now() < deadline, "the session still needs you");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn attach(&self) -> Attach {
        let tab = agent_tab(&self.daemon, &self.session).await;
        Attach::start(&self.daemon, open(&tab, 0)).await
    }
}

fn reply(text: &str) -> pb::request_answer::Answer {
    pb::request_answer::Answer::Reply(pb::Reply {
        text: text.to_owned(),
    })
}

fn question_of(request: &pb::InboxRequest) -> pb::QuestionPreview {
    match request.preview.as_ref().and_then(|preview| preview.preview.as_ref()) {
        Some(pb::request_preview::Preview::Question(question)) => question.clone(),
        other => panic!("not a question card: {other:?}"),
    }
}

/// The hex of what the agent was answered with, once it was.
async fn wait_answer(path: &Path) -> String {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Ok(hex) = std::fs::read_to_string(path) {
            return hex;
        }
        assert!(Instant::now() < deadline, "the agent was never answered");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// D767: the folder trust raises one question with every choice, nothing is
/// typed until the person answers in the Inbox, and then their choice is
/// picked: here the second, one row down from the highlighted first.
#[tokio::test(flavor = "multi_thread")]
async fn folder_trust_on_the_agents_screen_is_an_inbox_question_answered_only_by_the_person() {
    let fix = Fixture::new().await;
    let card = fix.wait_pending().await;
    assert_eq!(card.kind, pb::RequestKind::Question as i32);
    let question = question_of(&card);
    assert!(question.question.contains("Do you trust the files in this folder?"));
    assert_eq!(question.choices, CHOICES);
    common::wait_status(&fix.daemon, &fix.session, pb::SessionStatus::NeedsInput).await;

    // Humata never answers it itself.
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert!(!fix.answer.exists(), "something answered the trust question");

    let id = card.id.clone().expect("card has an id");
    assert!(fix.answer(&id, approve()).await.is_err(), "an approval was taken");
    assert!(fix.answer(&id, reply("Maybe later")).await.is_err(), "a reply that is no choice was taken");
    assert!(!fix.answer.exists(), "a refused answer typed something");
    fix.answer(&id, reply("2")).await.expect("pick the second choice");

    // ESC [ B (one row down), then the line's end.
    assert_eq!(wait_answer(&fix.answer).await, "1b5b42");
    assert_eq!(fix.request(&id).await.state, pb::RequestState::Answered as i32);
    fix.wait_not_needing_you().await;
}

/// D767: a question the person answers in the terminal instead is withdrawn,
/// and the session no longer waits on them.
#[tokio::test(flavor = "multi_thread")]
async fn folder_trust_answered_in_the_terminal_withdraws_the_question() {
    let fix = Fixture::new().await;
    let card = fix.wait_pending().await;
    let id = card.id.clone().expect("card has an id");
    common::wait_status(&fix.daemon, &fix.session, pb::SessionStatus::NeedsInput).await;

    let mut attach = fix.attach().await;
    attach.type_text("\r").await;
    assert_eq!(wait_answer(&fix.answer).await, "");
    let deadline = Instant::now() + WAIT;
    while fix.request(&id).await.state != pb::RequestState::Withdrawn as i32 {
        assert!(Instant::now() < deadline, "the question was never withdrawn");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    fix.wait_not_needing_you().await;
}

/// D767: declining in the Inbox types nothing: the question stays on the
/// agent's screen for the person.
#[tokio::test(flavor = "multi_thread")]
async fn declining_folder_trust_in_the_inbox_types_nothing() {
    let fix = Fixture::new().await;
    let card = fix.wait_pending().await;
    let id = card.id.clone().expect("card has an id");
    fix.answer(
        &id,
        pb::request_answer::Answer::Deny(pb::Deny {
            note: String::new(),
        }),
    )
    .await
    .expect("decline the question");
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert!(!fix.answer.exists(), "declining typed into the question");
    // Once declined it is not asked again for the same screen.
    let pending = fix
        .requests()
        .await
        .into_iter()
        .filter(|request| request.state == pb::RequestState::Pending as i32)
        .count();
    assert_eq!(pending, 0);
}

fn approve() -> pb::request_answer::Answer {
    pb::request_answer::Answer::Approve(pb::Approve {})
}

/// D767 amendment: a box drawn in two steps, its first choice alone at
/// first, ends as a card with every choice, and replying the last choice,
/// No, moves the highlight two rows down and presses Enter. Before, the
/// card kept the first read's list, and a reply never reached the screen.
#[tokio::test(flavor = "multi_thread")]
async fn a_trust_box_drawn_in_steps_offers_every_choice_and_no_picks_the_no_row() {
    let fix = Fixture::drawing(true).await;
    let card = fix
        .wait_pending_where(|card| question_of(card).choices == CHOICES)
        .await;
    let id = card.id.clone().expect("card has an id");
    fix.answer(&id, reply("No")).await.expect("pick No");
    // ESC [ B twice (two rows down), then the line's end.
    assert_eq!(wait_answer(&fix.answer).await, "1b5b421b5b42");
    assert_eq!(fix.request(&id).await.state, pb::RequestState::Answered as i32);
    // The card read off the half-drawn box was withdrawn, not left pending.
    let deadline = Instant::now() + WAIT;
    while fix
        .requests()
        .await
        .iter()
        .any(|request| request.state == pb::RequestState::Pending as i32)
    {
        assert!(Instant::now() < deadline, "a card is still pending");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
