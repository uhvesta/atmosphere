//! A dialog the agent's CLI asks on its own screen, such as Copilot's folder
//! trust, through a real `humatad2` (D845): it raises no Inbox card, whatever
//! the agent's "Ask in the Inbox" switch says, and the session needs you
//! until the dialog is answered in the terminal. Humata types nothing, and
//! a first message held meanwhile is sent once after (D763).

use humatad_test_common as common;

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

/// How the test agent draws its folder trust box.
#[derive(Clone, Copy)]
enum Draw {
    /// All of it at once.
    Whole,
    /// All of it at once, but only after [`LATE_BY`], as an agent's CLI
    /// slow to start on a loaded machine draws it, after its launch.
    Late,
    /// All of it at once; once it is answered, the logo goes on animating
    /// above the agent's prompt, so its output doesn't go quiet.
    WholeThenAnimating,
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
const BOX_REST: &str = r"  '│   3. No (Esc)                                            │' \
  '│                                                          │' \
  '│ ↑/↓ to navigate · enter to select · esc to cancel        │' \
  '╰──────────────────────────────────────────────────────────╯'";

/// The test agent asks Copilot 1.0.113's folder trust on its screen, drawn
/// as `draw` says, then keeps the bytes of the line it is answered with,
/// as hex, in `trust-answer`, and answers each line after as the other
/// test agents do.
fn agent_script(world: &World, draw: Draw) -> String {
    let (before, ask, after) = match draw {
        Draw::Whole => (String::new(), format!("printf '%s\\n' \\\n{BOX_TOP} \\\n{BOX_REST}\n"), ""),
        Draw::Late => (
            format!("sleep {}\n", LATE_BY.as_secs()),
            format!("printf '%s\\n' \\\n{BOX_TOP} \\\n{BOX_REST}\n"),
            "",
        ),
        Draw::WholeThenAnimating => (
            String::new(),
            format!("printf '%s\\n' \\\n{BOX_TOP} \\\n{BOX_REST}\n"),
            "",
        ),
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

/// D775 amendment (wave 3): once the trust box is answered on the screen
/// and the agent sits at its prompt, typing to it is not typing mid-turn,
/// though its output goes on (Copilot's animated logo). Before, the box's
/// hold stayed until the output next went quiet, so the session read
/// NEEDS_INPUT and the type said the agent was busy.
#[tokio::test(flavor = "multi_thread")]
async fn typing_at_the_prompt_after_the_trust_box_was_answered_on_the_screen_is_not_busy() {
    let fix = Fixture::drawing(Draw::WholeThenAnimating).await;
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

/// D845 (the path D782's off switch took first): the trust box on the
/// screen of an agent whose "Ask in the Inbox" is off raises no card, but
/// the session needs you while the box shows and not after it is answered
/// in the terminal; its first message, held meanwhile, is sent once.
#[tokio::test(flavor = "multi_thread")]
async fn an_agent_asking_in_its_terminal_raises_no_card_but_needs_you_until_answered() {
    asks_in_its_terminal_holding_the_first_message(Draw::Whole, true).await;
}

/// D845: the switch is left ON (the default): a trust box on the agent's
/// screen still raises no card, whatever the agent's "Ask in the Inbox"
/// says, but the session needs you until it is answered in the terminal.
#[tokio::test(flavor = "multi_thread")]
async fn a_trust_box_raises_no_card_even_with_ask_in_the_inbox_on() {
    asks_in_its_terminal_holding_the_first_message(Draw::Whole, false).await;
}

/// D762: an agent slow to start draws its trust box only after the
/// session's start would send its first message. The message still waits
/// for the box to be answered: it is never typed into the box, where it
/// would answer it (the flake under load this case pins).
#[tokio::test(flavor = "multi_thread")]
async fn a_first_message_waits_for_a_trust_box_drawn_after_the_launch() {
    asks_in_its_terminal_holding_the_first_message(Draw::Late, false).await;
}

/// The trust box drawn as `draw` says, in the terminal of an agent whose
/// Ask in the Inbox is `switch_off` (or on): no card, the session needs you until the box is
/// answered there, and the held first message is sent once after.
async fn asks_in_its_terminal_holding_the_first_message(draw: Draw, switch_off: bool) {
    let dir = folder();
    let marker = dir.path().join("first-message-ran");
    let fix = Fixture::starting(
        draw,
        Start {
            asks_in_terminal: switch_off,
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
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(std::fs::read_to_string(&marker).expect("read the marker"), "sent\n", "sent more than once");
}
