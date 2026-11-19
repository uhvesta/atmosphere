//! A question the agent's CLI asks on its own screen, such as Copilot's
//! folder trust, through a real `humatad2` (D767): it raises one Inbox
//! question with the question and every choice the screen offers, and the
//! session waits on you meanwhile. Humata types nothing until the person
//! answers in the Inbox; then it picks their choice. A question answered in
//! the terminal instead is withdrawn. An agent whose "Ask in the Inbox" is
//! off (D782) raises no card; the session still needs you while it asks.

use humatad_test_common as common;
use humatad_test_common::trust_box;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::{
    Attach, Build, Daemon, WAIT, World, agent_tab, create_channel, folder, open,
};
use humata2_grpc::humata::v1 as pb;

/// How long after its launch [`Draw::Late`] draws the box: past the start
/// a session's first message is sent at, and well inside the time a
/// starting agent gets to read keys itself (five seconds).
const LATE_BY: Duration = Duration::from_secs(1);

const CHOICES: [&str; 3] = [
    "Yes",
    "Yes, and remember this folder for future sessions",
    "No",
];

/// How the test agent draws its folder trust box.
#[derive(Clone, Copy)]
enum Draw {
    /// All of it at once.
    Whole,
    /// All of it at once, but only after [`LATE_BY`], as an agent's CLI
    /// slow to start on a loaded machine draws it, after its launch.
    Late,
    /// First only the start of the box, its first choice alone, quiet with
    /// it for longer than a quiet spell, then the screen cleared and the
    /// whole box drawn, as Copilot was seen to before the rest showed.
    FirstRowFirst,
    /// As VERIFY saw Copilot 1.0.113 do it on a remote machine (wave 3):
    /// the box down to its second choice, quiet for longer than a quiet
    /// spell, then the No row and the rest of the box, while Copilot's
    /// logo above it goes on animating (redrawn every half second), so the
    /// agent's output never goes quiet again. The box is the captured one,
    /// narrowed to fit the test's screen, with a made-up folder.
    InStepsWhileAnimating,
    /// All of it at once; once it is answered, the logo goes on animating
    /// above the agent's prompt, so its output doesn't go quiet.
    WholeThenAnimating,
    /// As VERIFY 0.2.3 saw Copilot draw it on a headless 80x24 pty: the
    /// box too short for its list, so only `1. Yes` shows above a footer
    /// that says the list scrolls (D785).
    Clipped,
    /// D786: [`trust_box::agent`]: once the test lets it, the box clipped
    /// while the pty has fewer than `whole_from` rows and whole from there,
    /// drawn again at a new size when it `redraws`.
    Sized { whole_from: u16, redraws: bool },
}

/// The rows of Copilot 1.0.113's folder trust box, as VERIFY captured it,
/// narrowed: down to the second choice, and from the third to the end.
const BOX_TOP: &str = r"  '╭──────────────────────────────────────────────────────────╮' \
  '│ Confirm folder trust                                     │' \
  '│ ╭──────────────────────────────────────────────────────╮ │' \
  '│ │ /work/api                                            │ │' \
  '│ ╰──────────────────────────────────────────────────────╯ │' \
  '│ Do you trust the files in this folder?                   │' \
  '│                                                          │' \
  '│ ❯ 1. Yes                                                 │' \
  '│   2. Yes, and remember this folder for future sessions   │'";
/// The same box as Copilot draws it on a screen too short for its list
/// (VERIFY 0.2.3, e2e023 c6c), narrowed and with a made-up folder.
const BOX_CLIPPED: &str = r"  '╭──────────────────────────────────────────────────────────╮' \
  '│ Confirm folder trust                                     │' \
  '│ ╭──────────────────────────────────────────────────────╮ │' \
  '│ │ /work/api                                            │ │' \
  '│ ╰──────────────────────────────────────────────────────╯ │' \
  '│ Do you trust the files in this folder?                   │' \
  '│                                                          │' \
  '│ ❯ 1. Yes                                                 │' \
  '│                                                          │' \
  '│ ↑/↓ to navigate · pgup/pgdn to scroll · enter to select  │' \
  '╰──────────────────────────────────────────────────────────╯'";
const BOX_REST: &str = r"  '│   3. No (Esc)                                            │' \
  '│                                                          │' \
  '│ ↑/↓ to navigate · enter to select · esc to cancel        │' \
  '╰──────────────────────────────────────────────────────────╯'";

/// The test agent asks Copilot 1.0.113's folder trust on its screen, drawn
/// as `draw` says, then keeps the bytes of the line it is answered with,
/// as hex, in `trust-answer`, and answers each line after as the other
/// test agents do.
fn agent_script(world: &World, draw: Draw) -> String {
    if let Draw::Sized { whole_from, redraws } = draw {
        return trust_box::agent(&world.conv, whole_from, redraws);
    }
    let (before, ask, after) = match draw {
        Draw::Whole => (String::new(), format!("printf '%s\\n' \\\n{BOX_TOP} \\\n{BOX_REST}\n"), ""),
        Draw::Late => (
            format!("sleep {}\n", LATE_BY.as_secs()),
            format!("printf '%s\\n' \\\n{BOX_TOP} \\\n{BOX_REST}\n"),
            "",
        ),
        Draw::FirstRowFirst => (
            r"printf '%s\n' \
  '│ Do you trust the files in this folder?                   │' \
  '│                                                          │' \
  '│ ❯ 1. Yes                                                 │'
sleep 6
printf '\033[H\033[2J'
"
            .to_owned(),
            format!("printf '%s\\n' \\\n{BOX_TOP} \\\n{BOX_REST}\n"),
            "",
        ),
        // The logo is row 1, the box rows 2 to 14; each frame is drawn on
        // row 1 and the cursor put back below the box.
        Draw::InStepsWhileAnimating => (
            String::new(),
            format!(
                r#"printf '\033[H\033[2J%s\n' '  ╭─╮╭─╮  Copilot uses AI.'
printf '%s\n' \
{BOX_TOP}
sleep 6
printf '%s\n' \
{BOX_REST}
( while :; do
    for frame in '╭─╮╭─╮' '╭──╮╭╮'; do
      printf '\033[1;3H%s\033[15;1H' "$frame"; sleep 0.5
    done
  done ) &
logo=$!
"#
            ),
            "kill $logo 2>/dev/null\n",
        ),
        Draw::WholeThenAnimating => (
            String::new(),
            format!("printf '%s\\n' \\\n{BOX_TOP} \\\n{BOX_REST}\n"),
            "",
        ),
        Draw::Clipped => (String::new(), format!("printf '%s\\n' \\\n{BOX_CLIPPED}\n"), ""),
        Draw::Sized { .. } => unreachable!("drawn by trust_box::agent"),
    };
    // Drawn on row 3 with the cursor saved and put back, so it leaves the
    // lines the agent prints alone.
    let animate = match draw {
        Draw::WholeThenAnimating => {
            r#"( while :; do
    for frame in '╭─╮╭─╮' '╭──╮╭╮'; do
      printf '\033%s\033[3;3H%s\033%s' 7 "$frame" 8 || exit; sleep 0.5
    done
  done ) &
"#
        }
        _ => "",
    };
    format!(
        r#"CONV_DIR='{}'
mode=$1; id=$2
case "$mode" in
  --session-id) f="$CONV_DIR/$id.jsonl"; : > "$f" ;;
  --resume) f="$CONV_DIR/$id.jsonl" ;;
esac
{before}{ask}IFS= read -r answer
{after}printf '%s' "$answer" | od -An -tx1 | tr -d ' \n' > "$CONV_DIR/trust-answer.part"
mv "$CONV_DIR/trust-answer.part" "$CONV_DIR/trust-answer"
printf '\033[H\033[2J'
echo "agent up $mode"
{animate}while IFS= read -r line; do
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
    answer: PathBuf,
}

/// How the fixture's session starts.
#[derive(Default)]
struct Start {
    /// D782: the agent's "Ask in the Inbox" is off before it starts.
    asks_in_terminal: bool,
    /// Its first message, held until its input is free (D762).
    first_message: String,
}

impl Fixture {
    async fn new() -> Self {
        Self::drawing(Draw::Whole).await
    }

    async fn drawing(draw: Draw) -> Self {
        Self::starting(draw, Start::default()).await
    }

    async fn starting(draw: Draw, start: Start) -> Self {
        let dir = folder();
        let world = World::new(dir.path());
        let script = dir.path().join("trust-agent.sh");
        std::fs::write(&script, agent_script(&world, draw)).expect("write a test file");
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
        let mut args = world.args();
        // Its turn hooks run the CLI.
        args.extend(["--humata".to_owned(), common::binary("HUMATA").display().to_string()]);
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let answer = world.conv.join("trust-answer");
        let daemon = Daemon::start_with(Build::Admin, dir, &refs).await;
        let (channel, setup) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
        assert_eq!(setup.state, pb::OperationState::Succeeded as i32, "{:?}", setup.error);
        if start.asks_in_terminal {
            let shown = pb::agent_service_client::AgentServiceClient::new(daemon.channel().await)
                .set_agent_enabled(pb::SetAgentEnabledRequest {
                    agent: Some(pb::AgentId {
                        value: common::AGENT_NAME.to_owned(),
                    }),
                    asks_in_inbox: Some(false),
                    ..Default::default()
                })
                .await
                .expect("the user turns the agent's Ask in the Inbox off")
                .into_inner();
            assert!(!shown.asks_in_inbox);
        }
        let machine = common::machine_id(&daemon).await;
        let mut request = common::start_request(channel.id.as_ref().expect("channel id"), &machine, "Fix login");
        request.first_message = start.first_message;
        let started = daemon
            .session_service()
            .await
            .start_session(request)
            .await
            .expect("start the session")
            .into_inner();
        let operation = common::finished(&daemon, &started.operation.expect("an operation")).await;
        let session = started.session.expect("a session");
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

    /// The command the agent's turn hook for `event` runs, as the daemon
    /// wrote it.
    fn hook_command(&self, event: &str) -> String {
        let file = self.daemon.state.join("claude-hooks.json");
        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(file).expect("read the hooks file"))
                .expect("the hooks file is JSON");
        json["hooks"][event][0]["hooks"][0]["command"]
            .as_str()
            .unwrap_or_else(|| panic!("no hook for {event}"))
            .to_owned()
    }

    async fn type_now(&self, text: &str) -> pb::TypeResponse {
        self.daemon
            .terminals()
            .await
            .r#type(pb::TypeRequest {
                session: Some(self.session.clone()),
                text: text.to_owned(),
                ..Default::default()
            })
            .await
            .expect("type into the session")
            .into_inner()
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
    let fix = Fixture::drawing(Draw::FirstRowFirst).await;
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

/// D767 amendment (wave 3): a box that finishes drawing after its card was
/// raised, while its agent's output never goes quiet again (Copilot's
/// animated logo), ends as a card with every choice, with no one attaching
/// to the session. Before, the card kept the first read's two choices until
/// something else, such as an attach, read the screen again, so No was
/// never offered.
#[tokio::test(flavor = "multi_thread")]
async fn a_trust_box_that_finishes_drawing_while_its_agent_keeps_writing_ends_with_every_choice() {
    let fix = Fixture::drawing(Draw::InStepsWhileAnimating).await;
    // The card read off the box before its No row showed offers no choice
    // (D785): two of three, with no No, is a part of the list.
    fix.wait_pending_where(|card| question_of(card).choices.is_empty())
        .await;
    let card = fix
        .wait_pending_where(|card| question_of(card).choices == CHOICES)
        .await;
    let id = card.id.clone().expect("card has an id");
    fix.answer(&id, reply("No")).await.expect("pick No");
    // ESC [ B twice (two rows down), then the line's end.
    assert_eq!(wait_answer(&fix.answer).await, "1b5b421b5b42");
}

/// D785 (VERIFY 0.2.3): a trust box clipped to `1. Yes` by a short pty,
/// with no one attached to resize it, is raised as a card with no choice:
/// a reply of Yes is refused and nothing is typed, so the person answers
/// in the terminal. Before, the card offered Yes alone.
#[tokio::test(flavor = "multi_thread")]
async fn a_trust_box_clipped_to_its_first_choice_offers_no_choice_and_takes_no_yes() {
    let fix = Fixture::drawing(Draw::Clipped).await;
    let card = fix.wait_pending().await;
    assert!(question_of(&card).choices.is_empty(), "{:?}", question_of(&card).choices);
    assert!(card.on_screen.is_some());
    let id = card.id.clone().expect("card has an id");
    assert!(fix.answer(&id, reply("Yes")).await.is_err());
    assert!(fix.answer(&id, reply("1")).await.is_err());
    assert_eq!(fix.request(&id).await.state, pb::RequestState::Pending as i32);
    assert!(!fix.answer.exists());
}

/// D767 amendment (wave 3): a trust box the person answers on the screen
/// while its card is pending leaves a card that takes no reply: a reply
/// right after is refused as withdrawn and the card ends withdrawn. Before, a reply that came before the next quiet
/// spell was taken as answered though nothing was pressed.
#[tokio::test(flavor = "multi_thread")]
async fn a_reply_after_the_box_was_answered_on_the_screen_is_withdrawn() {
    let fix = Fixture::new().await;
    let card = fix.wait_pending().await;
    let id = card.id.clone().expect("card has an id");

    let mut attach = fix.attach().await;
    attach.type_text("\r").await;
    attach.wait_for("agent up").await;
    assert_eq!(wait_answer(&fix.answer).await, "");

    let replied = fix.answer(&id, reply("Yes")).await;
    assert!(
        !matches!(&replied, Ok(request) if request.state == pb::RequestState::Answered as i32),
        "a reply to a box no longer on the screen was taken: {replied:?}"
    );
    let deadline = Instant::now() + WAIT;
    while fix.request(&id).await.state != pb::RequestState::Withdrawn as i32 {
        assert!(Instant::now() < deadline, "the card was never withdrawn");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    fix.wait_not_needing_you().await;
}

/// D775 amendment (wave 3): once the trust box is answered on the screen
/// and the agent sits at its prompt, typing to it is not typing mid-turn,
/// though its output goes on (Copilot's animated logo). Before, the box's
/// card stayed open until the output next went quiet, so the session read
/// NEEDS_INPUT and the type said the agent was busy.
#[tokio::test(flavor = "multi_thread")]
async fn typing_at_the_prompt_after_the_trust_box_was_answered_on_the_screen_is_not_busy() {
    let fix = Fixture::drawing(Draw::WholeThenAnimating).await;
    fix.wait_pending().await;
    common::wait_status(&fix.daemon, &fix.session, pb::SessionStatus::NeedsInput).await;

    let mut attach = fix.attach().await;
    attach.type_text("\r").await;
    attach.wait_for("agent up").await;
    // Its hook says its turn is over, as Copilot's does at its prompt.
    attach.type_text(&format!("run:{}\r", fix.hook_command("Stop"))).await;
    common::wait_status(&fix.daemon, &fix.session, pb::SessionStatus::Idle).await;
    let typed = fix.type_now("hello\n").await;
    assert!(typed.typed);
    assert!(!typed.agent_busy, "{typed:?}");
}

/// D782: an agent whose "Ask in the Inbox" is off raises no card for the
/// trust box on its screen, but the session needs you while the box shows
/// and not after it is answered in the terminal; its first message, held
/// meanwhile, is sent once.
#[tokio::test(flavor = "multi_thread")]
async fn an_agent_asking_in_its_terminal_raises_no_card_but_needs_you_until_answered() {
    asks_in_its_terminal_holding_the_first_message(Draw::Whole).await;
}

/// D762: an agent slow to start draws its trust box only after the
/// session's start would send its first message. The message still waits
/// for the box to be answered: it is never typed into the box, where it
/// would answer it (the flake under load this case pins).
#[tokio::test(flavor = "multi_thread")]
async fn a_first_message_waits_for_a_trust_box_drawn_after_the_launch() {
    asks_in_its_terminal_holding_the_first_message(Draw::Late).await;
}

/// The trust box drawn as `draw` says, in the terminal of an agent whose
/// Ask in the Inbox is off: no card, the session needs you until the box is
/// answered there, and the held first message is sent once after.
async fn asks_in_its_terminal_holding_the_first_message(draw: Draw) {
    let dir = folder();
    let marker = dir.path().join("first-message-ran");
    let fix = Fixture::starting(
        draw,
        Start {
            asks_in_terminal: true,
            first_message: format!("run:echo sent >> '{}'", marker.display()),
        },
    )
    .await;
    common::wait_status(&fix.daemon, &fix.session, pb::SessionStatus::NeedsInput).await;
    // Long enough for a card to have come, as the tests above see one come.
    tokio::time::sleep(Duration::from_secs(4)).await;
    let pending = fix
        .requests()
        .await
        .into_iter()
        .filter(|request| request.state == pb::RequestState::Pending as i32)
        .count();
    assert_eq!(pending, 0, "a card was raised for an agent asking in its terminal");
    assert_eq!(fix.status().await, pb::SessionStatus::NeedsInput);
    assert!(!marker.exists(), "the first message was sent over the trust box");
    assert!(!fix.answer.exists(), "something answered the trust box");

    let mut attach = fix.attach().await;
    attach.type_text("\r").await;
    assert_eq!(wait_answer(&fix.answer).await, "");
    fix.wait_not_needing_you().await;
    let deadline = Instant::now() + WAIT;
    while !marker.exists() {
        assert!(Instant::now() < deadline, "the held first message was never sent");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(std::fs::read_to_string(&marker).expect("read the marker"), "sent\n", "sent more than once");
}

fn dismiss() -> pb::request_answer::Answer {
    pb::request_answer::Answer::Deny(pb::Deny {
        note: String::new(),
    })
}

/// D781: a question read off the agent's screen names the agent's tab, by
/// its index among the session's tabs as ListTabs orders them, so the app
/// can show that screen under it.
#[tokio::test(flavor = "multi_thread")]
async fn a_screen_question_names_its_agent_tab() {
    let fix = Fixture::new().await;
    let card = fix.wait_pending().await;
    let tabs = fix
        .daemon
        .terminals()
        .await
        .list_tabs(pb::ListTabsRequest {
            session: Some(fix.session.clone()),
        })
        .await
        .expect("list the session's tabs")
        .into_inner()
        .tabs;
    let agent = tabs
        .iter()
        .position(|tab| tab.kind == pb::tab::Kind::Agent as i32)
        .expect("the session has an agent tab");
    let tab = card.on_screen.as_ref().map(|on_screen| on_screen.tab as usize);
    assert_eq!(tab, Some(agent), "{card:?}");
}

/// D781: Dismiss (a Deny with no note) while the dialog is still on the
/// screen types nothing and raises no second item, and the session goes on
/// needing you until the dialog is answered in its terminal.
#[tokio::test(flavor = "multi_thread")]
async fn dismissing_while_the_screen_asks_types_nothing_and_the_session_keeps_needing_you() {
    let fix = Fixture::new().await;
    let card = fix.wait_pending().await;
    let id = card.id.clone().expect("card has an id");
    common::wait_status(&fix.daemon, &fix.session, pb::SessionStatus::NeedsInput).await;
    let dismissed = fix.answer(&id, dismiss()).await.expect("dismiss the item");
    assert_eq!(dismissed.state, pb::RequestState::Denied as i32, "{dismissed:?}");

    // Longer than a quiet spell, so the screen has been looked at again.
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert!(!fix.answer.exists(), "dismissing typed into the question");
    let pending = fix
        .requests()
        .await
        .into_iter()
        .filter(|request| request.state == pb::RequestState::Pending as i32)
        .count();
    assert_eq!(pending, 0, "a second item was raised for the same screen");
    assert_eq!(fix.status().await, pb::SessionStatus::NeedsInput);

    // Answered in the terminal: the session no longer needs you.
    let mut attach = fix.attach().await;
    attach.type_text("\r").await;
    assert_eq!(wait_answer(&fix.answer).await, "");
    fix.wait_not_needing_you().await;
    assert_eq!(fix.request(&id).await.state, pb::RequestState::Denied as i32);
}

/// D781: an item whose dialog was answered in the terminal ends withdrawn
/// as resolved, not as a call that ended, whether Dismiss or the next look
/// at the screen finds it gone first; Dismiss types nothing.
#[tokio::test(flavor = "multi_thread")]
async fn dismissing_after_the_dialog_was_answered_in_the_terminal_withdraws_it_as_resolved() {
    let fix = Fixture::new().await;
    let card = fix.wait_pending().await;
    let id = card.id.clone().expect("card has an id");

    let mut attach = fix.attach().await;
    attach.type_text("\r").await;
    attach.wait_for("agent up").await;
    assert_eq!(wait_answer(&fix.answer).await, "");
    // Taken as the item's end if Dismiss came first; refused as withdrawn
    // if the look did. Either way nothing is pressed.
    let _ = fix.answer(&id, dismiss()).await;

    let deadline = Instant::now() + WAIT;
    loop {
        let request = fix.request(&id).await;
        if request.state == pb::RequestState::Withdrawn as i32 {
            assert_eq!(
                request.withdraw_reason,
                pb::WithdrawReason::Resolved as i32,
                "{request:?}"
            );
            break;
        }
        assert_ne!(request.state, pb::RequestState::Denied as i32, "{request:?}");
        assert!(Instant::now() < deadline, "the item was never withdrawn");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    fix.wait_not_needing_you().await;
}

// D786: every holder starts at 80x50, and a trust box a short screen cuts
// off, with no client attached to type, grows the pty's rows to 50 once.

/// Long enough for a grow or a card to have come, as the tests above see
/// one come: past a quiet spell and a look.
const SETTLE: Duration = Duration::from_secs(5);

impl Fixture {
    /// The conversations folder, where the agent of [`trust_box::agent`]
    /// keeps its `go` and `sizes`.
    fn conv(&self) -> PathBuf {
        self.answer.parent().expect("the answer is in the conversations folder").to_owned()
    }

    async fn reader(&self) -> Attach {
        let tab = agent_tab(&self.daemon, &self.session).await;
        Attach::start(&self.daemon, trust_box::reader_open(None, &tab)).await
    }

    /// The agent's screen size now, `(cols, rows)`.
    async fn size(&self) -> (u32, u32) {
        let mut reader = self.reader().await;
        let state = trust_box::wait_state(&mut reader, |_| true).await;
        (state.cols, state.rows)
    }

    /// Attaches with input at `cols`x`rows`, which sizes the pty to it, and
    /// returns the attach.
    async fn writer(&self, cols: u32, rows: u32) -> Attach {
        let tab = agent_tab(&self.daemon, &self.session).await;
        Attach::start(&self.daemon, trust_box::writer_open(None, &tab, cols, rows)).await
    }

    /// Sizes the pty to `cols`x`rows` as a client does and leaves; once no
    /// one with input is attached, lets the agent draw. Returns a reader,
    /// which hears the screen's size.
    async fn short_then_draw(&self, cols: u32, rows: u32) -> Attach {
        let writer = self.writer(cols, rows).await;
        let mut reader = self.reader().await;
        trust_box::wait_state(&mut reader, |state| {
            (state.cols, state.rows) == (cols, rows) && state.other_writers == 1
        })
        .await;
        drop(writer);
        trust_box::wait_state(&mut reader, |state| state.other_writers == 0).await;
        trust_box::go(&self.conv());
        reader
    }

    async fn pending(&self) -> Vec<pb::InboxRequest> {
        self.requests()
            .await
            .into_iter()
            .filter(|request| request.state == pb::RequestState::Pending as i32)
            .collect()
    }

    async fn unread(&self) -> bool {
        common::get_session(&self.daemon, &self.session).await.unread
    }
}

/// Copilot's trust box needs about 26 rows to show its whole list.
const COPILOT_ROWS: u16 = 30;

fn redrawing(whole_from: u16) -> Draw {
    Draw::Sized {
        whole_from,
        redraws: true,
    }
}

/// D786 (pick 1): a session no client ever attached to starts at 80x50,
/// so the trust box shows whole and its card offers every choice.
#[tokio::test(flavor = "multi_thread")]
async fn a_headless_trust_box_offers_every_choice() {
    let fix = Fixture::drawing(redrawing(COPILOT_ROWS)).await;
    trust_box::go(&fix.conv());
    let card = fix
        .wait_pending_where(|card| question_of(card).choices == CHOICES)
        .await;
    assert!(card.on_screen.is_some());
    assert_eq!(trust_box::sizes(&fix.conv()), ["50 80"]);
}

/// D786: a box a 24-row screen clips, with no writer attached, grows the
/// pty to 80x50; the agent draws it whole and exactly one card is raised,
/// with every choice. The session needs you throughout.
#[tokio::test(flavor = "multi_thread")]
async fn a_short_screen_with_no_writer_grows_to_50_rows_and_raises_one_card_with_every_choice() {
    let fix = Fixture::drawing(redrawing(COPILOT_ROWS)).await;
    drop(fix.short_then_draw(80, 24).await);
    trust_box::wait_size(&fix.conv(), "50 80").await;
    assert_eq!(fix.status().await, pb::SessionStatus::NeedsInput);
    let card = fix
        .wait_pending_where(|card| question_of(card).choices == CHOICES)
        .await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let raised = fix.requests().await;
    assert_eq!(raised.len(), 1, "more than one card was raised: {raised:?}");
    assert_eq!(raised[0].id, card.id);
    assert_eq!(trust_box::sizes(&fix.conv()), ["24 80", "50 80"]);
    assert_eq!(fix.size().await, (80, 50));
    assert_eq!(fix.status().await, pb::SessionStatus::NeedsInput);
}

/// D786: with a writer attached the screen is the writer's to size: no
/// resize, and the clipped box's card offers no choice (D785).
#[tokio::test(flavor = "multi_thread")]
async fn with_a_writer_attached_the_screen_is_not_resized_and_the_card_offers_no_choice() {
    let fix = Fixture::drawing(redrawing(COPILOT_ROWS)).await;
    let writer = fix.writer(80, 24).await;
    trust_box::go(&fix.conv());
    let card = fix.wait_pending().await;
    assert!(question_of(&card).choices.is_empty(), "{:?}", question_of(&card).choices);
    tokio::time::sleep(SETTLE).await;
    assert_eq!(trust_box::sizes(&fix.conv()), ["24 80"]);
    assert_eq!(fix.size().await, (80, 24));
    drop(writer);
}

/// D786: a read-only attach doesn't block the grow, and hears the screen's
/// new size.
#[tokio::test(flavor = "multi_thread")]
async fn a_reader_does_not_block_the_grow_and_hears_the_new_size() {
    let fix = Fixture::drawing(redrawing(COPILOT_ROWS)).await;
    let mut reader = fix.short_then_draw(80, 24).await;
    let state = trust_box::wait_state(&mut reader, |state| state.rows == 50).await;
    assert_eq!(state.cols, 80);
    fix.wait_pending_where(|card| question_of(card).choices == CHOICES)
        .await;
}

/// D786: a box still drawing (its first choice alone, no footer that says
/// it scrolls) is not cut off by the screen's height, so it is never
/// resized; the drawn box's card offers every choice.
#[tokio::test(flavor = "multi_thread")]
async fn a_box_still_drawing_is_never_resized() {
    let fix = Fixture::drawing(Draw::FirstRowFirst).await;
    // The box is drawn in full 6 s in; the screen is short before then.
    drop(fix.writer(80, 24).await);
    fix.wait_pending_where(|card| question_of(card).choices == CHOICES)
        .await;
    assert_eq!(fix.size().await, (80, 24));
}

/// D786: a box still clipped at 50 rows is resized once, then its card
/// offers no choice (D785); it is never resized again.
#[tokio::test(flavor = "multi_thread")]
async fn a_box_still_clipped_at_50_rows_is_resized_once_then_offers_no_choice() {
    let fix = Fixture::drawing(redrawing(60)).await;
    drop(fix.short_then_draw(80, 24).await);
    trust_box::wait_size(&fix.conv(), "50 80").await;
    let card = fix.wait_pending().await;
    assert!(question_of(&card).choices.is_empty(), "{:?}", question_of(&card).choices);
    tokio::time::sleep(SETTLE).await;
    assert_eq!(trust_box::sizes(&fix.conv()), ["24 80", "50 80"]);
    assert_eq!(fix.pending().await.len(), 1);
    assert_eq!(fix.size().await, (80, 50));
}

/// D786: the grow changes only the rows, keeping the columns, and never
/// shrinks a pty that already has 50 rows or more.
#[tokio::test(flavor = "multi_thread")]
async fn the_grow_keeps_the_columns_and_never_shrinks_the_pty() {
    let wide = Fixture::drawing(redrawing(COPILOT_ROWS)).await;
    drop(wide.short_then_draw(100, 24).await);
    wide.wait_pending_where(|card| question_of(card).choices == CHOICES)
        .await;
    assert_eq!(trust_box::sizes(&wide.conv()), ["24 100", "50 100"]);

    let tall = Fixture::drawing(redrawing(70)).await;
    drop(tall.short_then_draw(100, 60).await);
    let card = tall.wait_pending().await;
    assert!(question_of(&card).choices.is_empty());
    tokio::time::sleep(SETTLE).await;
    assert_eq!(trust_box::sizes(&tall.conv()), ["60 100"]);
    assert_eq!(tall.size().await, (100, 60));
}

/// D786: the grow types nothing: the agent's answer is only the Enter the
/// person's Yes presses.
#[tokio::test(flavor = "multi_thread")]
async fn the_grow_types_nothing() {
    let fix = Fixture::drawing(redrawing(COPILOT_ROWS)).await;
    drop(fix.short_then_draw(80, 24).await);
    let card = fix
        .wait_pending_where(|card| question_of(card).choices == CHOICES)
        .await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(!fix.answer.exists(), "something answered the trust question");
    let id = card.id.clone().expect("card has an id");
    fix.answer(&id, reply("Yes")).await.expect("pick Yes");
    assert_eq!(wait_answer(&fix.answer).await, "");
}

/// D786: an agent whose "Ask in the Inbox" is off (D782) is never resized:
/// its box stays clipped in its terminal, and the session needs you.
#[tokio::test(flavor = "multi_thread")]
async fn with_ask_in_the_inbox_off_the_screen_is_not_resized() {
    let fix = Fixture::starting(
        redrawing(COPILOT_ROWS),
        Start {
            asks_in_terminal: true,
            ..Start::default()
        },
    )
    .await;
    drop(fix.short_then_draw(80, 24).await);
    common::wait_status(&fix.daemon, &fix.session, pb::SessionStatus::NeedsInput).await;
    tokio::time::sleep(SETTLE).await;
    assert_eq!(trust_box::sizes(&fix.conv()), ["24 80"]);
    assert_eq!(fix.size().await, (80, 24));
    assert!(fix.pending().await.is_empty());
}

/// D786: a program that ignores the resize, and writes nothing after it,
/// still gets its card within 3 s of the grow: the box it shows is still
/// clipped, so the card offers no choice (D785).
#[tokio::test(flavor = "multi_thread")]
async fn a_program_that_ignores_the_resize_still_gets_its_card_within_3_seconds() {
    let fix = Fixture::drawing(Draw::Sized {
        whole_from: COPILOT_ROWS,
        redraws: false,
    })
    .await;
    let mut reader = fix.short_then_draw(80, 24).await;
    trust_box::wait_state(&mut reader, |state| state.rows == 50).await;
    let grown = Instant::now();
    drop(reader);
    let deadline = grown + Duration::from_secs(3);
    let card = loop {
        if let Some(card) = fix.pending().await.pop() {
            break card;
        }
        assert!(Instant::now() < deadline, "no card within 3 s of the grow");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert!(question_of(&card).choices.is_empty(), "{:?}", question_of(&card).choices);
    assert_eq!(trust_box::sizes(&fix.conv()), ["24 80"]);
}

/// D786: the repaint the grow brings is not new output (D393): a session
/// read before the grow stays read.
#[tokio::test(flavor = "multi_thread")]
async fn the_grow_is_not_unread_output() {
    // Clipped at 50 rows too, so the repaint shows the same words.
    let fix = Fixture::drawing(redrawing(60)).await;
    drop(fix.short_then_draw(80, 24).await);
    let deadline = Instant::now() + WAIT;
    while !fix.unread().await {
        assert!(Instant::now() < deadline, "the box's first draw never marked the session");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let mut viewing = pb::mark_service_client::MarkServiceClient::new(fix.daemon.channel().await)
        .view_session(pb::ViewSessionRequest {
            session: Some(fix.session.clone()),
        })
        .await
        .expect("view the session")
        .into_inner();
    viewing.message().await.expect("the view is confirmed").expect("the view is confirmed");
    drop(viewing);
    let deadline = Instant::now() + WAIT;
    while fix.unread().await {
        assert!(Instant::now() < deadline, "viewing never marked the session read");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    trust_box::wait_size(&fix.conv(), "50 80").await;
    fix.wait_pending().await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(!fix.unread().await, "the grow's repaint marked the session unread");
}
