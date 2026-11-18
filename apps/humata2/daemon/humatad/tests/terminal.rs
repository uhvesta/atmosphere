//! Terminal tabs through `humatad2`'s gRPC API: a real shell in a real
//! holder, typed into and read back over the socket.

mod common;

use std::path::Path;
use std::time::Duration;

use common::{
    Attach, Build, Daemon, World, agent_tab, binary, create_channel, folder, invalid_field, not_found, open,
    start_session, state_folder, stored_tabs, wait_status,
};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{SessionRecord, Store};
use pb::attach_client_message::Message as ClientMessage;
use pb::terminal_state::Writable;
use rustix::process::{Pid, Signal, kill_process};

async fn machine_shell(daemon: &Daemon) -> pb::TabId {
    daemon
        .terminals()
        .await
        .open_machine_shell(pb::OpenMachineShellRequest::default())
        .await
        .unwrap()
        .into_inner()
        .id
        .unwrap()
}

async fn list(daemon: &Daemon) -> Vec<pb::Tab> {
    daemon
        .terminals()
        .await
        .list_tabs(pb::ListTabsRequest::default())
        .await
        .unwrap()
        .into_inner()
        .tabs
}

// Quoting splits the word, so the shell's echo of the command line
// doesn't count as its output.
const SAY_HELLO: &str = "echo hel''lo\n";
const SAY_WORLD: &str = "echo wor''ld\n";

#[tokio::test(flavor = "multi_thread")]
async fn typed_input_comes_back_as_contiguous_output() {
    let daemon = Daemon::start().await;
    let tab = machine_shell(&daemon).await;
    let mut attach = Attach::start(&daemon, open(&tab, 0)).await;

    let seq = attach.type_text(SAY_HELLO).await;
    attach.wait_for("hello").await;
    attach
        .wait_until(|attach| attach.accepted.contains(&seq))
        .await;

    assert_eq!(attach.snapshots, 1);
    let state = attach.states.first().unwrap();
    assert_eq!(state.writable(), Writable::Yes);
    assert_eq!(state.other_writers, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn reattaching_from_the_cursor_sends_only_newer_output() {
    let daemon = Daemon::start().await;
    let tab = machine_shell(&daemon).await;
    let mut first = Attach::start(&daemon, open(&tab, 0)).await;
    first.type_text(SAY_HELLO).await;
    first.wait_for("hello").await;
    let cursor = first.cursor;
    drop(first);

    let mut again = Attach::start(&daemon, open(&tab, cursor)).await;
    again.type_text(SAY_WORLD).await;
    again.wait_for("world").await;

    assert_eq!(again.snapshots, 0);
    assert!(!again.shows("hello"));
}

fn caught_up(notice: &pb::TerminalNotice) -> bool {
    notice.kind() == pb::terminal_notice::Kind::CaughtUp
}

/// SE-16: an attach that resumes from an offset is told, once what it
/// missed has been replayed, how many lines that was.
#[tokio::test(flavor = "multi_thread")]
async fn resuming_from_an_offset_says_how_many_lines_it_caught_up() {
    let daemon = Daemon::start().await;
    let tab = machine_shell(&daemon).await;
    let mut first = Attach::start(&daemon, open(&tab, 0)).await;
    first.type_text(SAY_HELLO).await;
    first.wait_for("hello").await;
    let cursor = first.cursor;
    first.type_text("printf 'a\\nb\\nc\\n'; echo do''ne\n").await;
    first.wait_for("done").await;
    drop(first);

    let mut again = Attach::start(&daemon, open(&tab, cursor)).await;
    again
        .wait_until(|attach| attach.notices.iter().any(caught_up))
        .await;

    let said: Vec<_> = again.notices.iter().filter(|notice| caught_up(notice)).collect();
    assert_eq!(said.len(), 1);
    let replayed = usize::try_from(said[0].at_offset - cursor).unwrap();
    let lines = again.screen[..replayed]
        .iter()
        .filter(|&&byte| byte == b'\n')
        .count();
    assert_eq!(usize::try_from(said[0].caught_up_lines).unwrap(), lines);
    assert!(lines >= 4, "a, b, c and done were missed");
}

#[tokio::test(flavor = "multi_thread")]
async fn attaching_from_zero_again_replays_what_was_kept() {
    let daemon = Daemon::start().await;
    let tab = machine_shell(&daemon).await;
    let mut first = Attach::start(&daemon, open(&tab, 0)).await;
    first.type_text(SAY_HELLO).await;
    first.wait_for("hello").await;
    drop(first);

    let mut again = Attach::start(&daemon, open(&tab, 0)).await;
    again.read().await.unwrap();

    assert_eq!(again.snapshots, 1);
    assert!(again.shows("hello"));
}

/// Prints about 2 MiB of repetitive lines, then "done".
const SAY_A_LOT: &str = "yes humata-compresses-this-line | head -n 90000; echo do''ne\n";

/// An attach that decodes every codec and holds all the output it's sent.
fn decoding(tab: &pb::TabId, from_offset: u64) -> pb::AttachOpen {
    pb::AttachOpen {
        accept_codecs: vec![pb::ChunkCodec::Lz4 as i32, pb::ChunkCodec::Zstd as i32],
        window_bytes: 64 << 20,
        ..open(tab, from_offset)
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn output_comes_compressed_to_a_client_that_decodes_it() {
    let daemon = Daemon::start().await;
    let tab = machine_shell(&daemon).await;
    let mut attach = Attach::start(&daemon, decoding(&tab, 0)).await;
    attach.type_text(SAY_A_LOT).await;
    attach.wait_for("done").await;

    assert!(attach.compressed > 0);
    assert!(attach.shows("humata-compresses-this-line"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_big_snapshot_comes_in_pieces_to_a_client_that_decodes() {
    let daemon = Daemon::start().await;
    let tab = machine_shell(&daemon).await;
    let mut first = Attach::start(&daemon, decoding(&tab, 0)).await;
    first.type_text(SAY_A_LOT).await;
    first.wait_for("done").await;
    let end = first.cursor;
    drop(first);

    let mut again = Attach::start(&daemon, decoding(&tab, 0)).await;
    again.wait_until(|again| again.snapshots == 1).await;

    assert!(again.snapshot_pieces > 1);
    assert!(again.compressed > 0);
    assert!(again.cursor >= end);
    assert!(again.shows("done"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_client_that_lists_no_codecs_gets_its_snapshot_whole_and_raw() {
    let daemon = Daemon::start().await;
    let tab = machine_shell(&daemon).await;
    let mut first = Attach::start(&daemon, decoding(&tab, 0)).await;
    first.type_text(SAY_A_LOT).await;
    first.wait_for("done").await;
    drop(first);

    let mut again = Attach::start(&daemon, open(&tab, 0)).await;
    again.wait_until(|again| again.snapshots == 1).await;

    assert_eq!(again.snapshot_pieces, 1);
    assert_eq!(again.compressed, 0);
    assert!(again.shows("done"));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_size_a_writer_attaches_with_reaches_the_shell() {
    let daemon = Daemon::start().await;
    let tab = machine_shell(&daemon).await;
    let mut attach = Attach::start(
        &daemon,
        pb::AttachOpen {
            cols: 100,
            rows: 30,
            ..open(&tab, 0)
        },
    )
    .await;
    attach.type_text("stty size\n").await;
    attach.wait_for("30 100").await;

    attach
        .send(ClientMessage::Resize(pb::TerminalResize {
            cols: 90,
            rows: 20,
            ..Default::default()
        }))
        .await;
    attach.type_text("stty size\n").await;
    attach.wait_for("20 90").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn machine_shells_are_listed_with_their_offsets() {
    let daemon = Daemon::start().await;
    let tab = machine_shell(&daemon).await;
    let mut attach = Attach::start(&daemon, open(&tab, 0)).await;
    attach.type_text(SAY_HELLO).await;
    attach.wait_for("hello").await;

    // The listing gives what the daemon's link to the holder has heard,
    // which can trail output still in flight; it catches up.
    let mut tabs = list(&daemon).await;
    for _ in 0..50 {
        if tabs[0].end_offset >= attach.cursor {
            break;
        }
        tabs = list(&daemon).await;
    }

    assert_eq!(tabs.len(), 1);
    assert_eq!(tabs[0].id.as_ref(), Some(&tab));
    assert_eq!(tabs[0].kind(), pb::tab::Kind::Shell);
    assert!(tabs[0].machine.is_some());
    assert!(tabs[0].end_offset >= attach.cursor);
    assert!(tabs[0].earliest_offset <= tabs[0].end_offset);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_tab_outlives_its_daemon() {
    let mut daemon = Daemon::start().await;
    let tab = machine_shell(&daemon).await;
    let mut before = Attach::start(&daemon, open(&tab, 0)).await;
    before.type_text(SAY_HELLO).await;
    before.wait_for("hello").await;
    drop(before);

    daemon.restart().await;

    let tabs = list(&daemon).await;
    assert_eq!(tabs.len(), 1);
    let mut after = Attach::start(&daemon, open(&tab, 0)).await;
    after.type_text(SAY_WORLD).await;
    after.wait_for("world").await;
    assert!(after.shows("hello"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_closed_tab_is_gone() {
    let daemon = Daemon::start().await;
    let tab = machine_shell(&daemon).await;

    daemon
        .terminals()
        .await
        .close_tab(pb::CloseTabRequest {
            tab: Some(tab.clone()),
            ..Default::default()
        })
        .await
        .unwrap();

    assert!(list(&daemon).await.is_empty());
    let refused = Attach::try_start(&daemon, open(&tab, 0))
        .await
        .err()
        .unwrap();
    assert_eq!(not_found(&refused), pb::EntityKind::Tab);
}

/// The longest a close may take with a shell's holder stopped. The shutdown
/// gives up after its own bound and the holder is ended after a short grace,
/// so this only catches a hang.
const CLOSE_GUARD: Duration = Duration::from_secs(30);

/// Resumes a process the test stopped, when the test ends, so a failing
/// assertion leaves nothing stopped behind.
struct Resumed(u32);

impl Drop for Resumed {
    fn drop(&mut self) {
        // A pid that no longer names a process is fine.
        if let Some(pid) = i32::try_from(self.0).ok().and_then(Pid::from_raw) {
            let _ = kill_process(pid, Signal::CONT);
        }
    }
}

/// Whether process `pid` still runs. One that has ended and waits to be
/// reaped (a zombie) doesn't.
fn running(pid: u32) -> bool {
    let output = std::process::Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .expect("ps runs");
    let state = String::from_utf8_lossy(&output.stdout);
    let state = state.trim();
    !state.is_empty() && !state.starts_with('Z')
}

#[tokio::test(flavor = "multi_thread")]
async fn closing_a_shell_whose_holder_is_stopped_ends_the_holder_and_does_not_hang() {
    let daemon = Daemon::start().await;
    let tab = machine_shell(&daemon).await;
    let holder = stored_tabs(&daemon.state)
        .iter()
        .find(|stored| stored.id == tab.value)
        .expect("the shell tab is stored")
        .holder_pid;
    let pid = i32::try_from(holder).ok().and_then(Pid::from_raw).expect("a holder pid");
    kill_process(pid, Signal::STOP).expect("the test can stop its own holder");
    let _resumed = Resumed(holder);

    let close = async {
        daemon
            .terminals()
            .await
            .close_tab(pb::CloseTabRequest {
                tab: Some(tab.clone()),
                ..Default::default()
            })
            .await
    };
    tokio::time::timeout(CLOSE_GUARD, close)
        .await
        .expect("the close finished within the guard, though the shell's holder never answered")
        .unwrap();

    assert!(!running(holder), "the shell's holder was ended by its pid");
    assert!(list(&daemon).await.is_empty(), "the closed shell is forgotten");
}

fn checkpointing(tab: &pb::TabId) -> pb::AttachOpen {
    pb::AttachOpen {
        checkpoint_engine: ghostty_vt::CHECKPOINT_FORMAT.to_owned(),
        ..open(tab, 0)
    }
}

/// A tab's link can be dropped and a new one started while an attach is up:
/// here typing into the stopped holder times out, which drops it. The
/// attach's later resizes reach the new link's screen, so the next viewer is
/// told the size the writer set.
#[tokio::test(flavor = "multi_thread")]
async fn a_resize_after_the_tabs_link_was_replaced_reaches_the_new_screen() {
    let (daemon, tab) = session_shell().await;
    let mut writer = Attach::start(&daemon, checkpointing(&tab)).await;
    writer.wait_until(|writer| writer.snapshots == 1).await;
    let holder = stored_tabs(&daemon.state)
        .iter()
        .find(|stored| stored.id == tab.value)
        .expect("the shell tab is stored")
        .holder_pid;
    let pid = i32::try_from(holder).ok().and_then(Pid::from_raw).expect("a holder pid");
    kill_process(pid, Signal::STOP).expect("the test can stop its own holder");
    let resumed = Resumed(holder);
    assert!(
        type_into_session(&daemon, "true\n").await.is_err(),
        "typing into a stopped holder gave up"
    );
    drop(resumed);
    type_into_session(&daemon, "true\n")
        .await
        .expect("typing goes through a new link");

    writer
        .send(ClientMessage::Resize(pb::TerminalResize {
            cols: 97,
            rows: 31,
            ..Default::default()
        }))
        .await;
    writer
        .wait_until(|writer| writer.states.last().is_some_and(|state| state.cols == 97))
        .await;

    let mut viewer = Attach::start(
        &daemon,
        pb::AttachOpen {
            read_only: true,
            ..checkpointing(&tab)
        },
    )
    .await;
    viewer.wait_until(|viewer| !viewer.states.is_empty()).await;
    assert_eq!(
        (viewer.states[0].cols, viewer.states[0].rows),
        (97, 31),
        "the writer's resize reached the screen the tab has now"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn attaching_past_the_end_is_refused() {
    let daemon = Daemon::start().await;
    let tab = machine_shell(&daemon).await;

    let refused = Attach::try_start(&daemon, open(&tab, u64::MAX))
        .await
        .err()
        .unwrap();

    let (field, problem) = invalid_field(&refused);
    assert_eq!(field, "open.from_offset");
    assert_eq!(problem, pb::invalid_field::Problem::OutOfRange);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_read_only_attach_watches_but_cannot_type() {
    let daemon = Daemon::start().await;
    let tab = machine_shell(&daemon).await;
    let mut writer = Attach::start(&daemon, open(&tab, 0)).await;
    let mut watcher = Attach::start(
        &daemon,
        pb::AttachOpen {
            read_only: true,
            ..open(&tab, 0)
        },
    )
    .await;

    writer.type_text(SAY_HELLO).await;
    watcher.wait_for("hello").await;
    let state = watcher.states.first().unwrap();
    assert_eq!(state.writable(), Writable::ReadOnlyAttach);
    assert_eq!(state.other_writers, 1);

    watcher.type_text(SAY_WORLD).await;
    let refused = watcher.wait_for_error().await;
    let (field, problem) = invalid_field(&refused);
    assert_eq!(field, "input");
    assert_eq!(problem, pb::invalid_field::Problem::NotAllowedHere);
}

#[tokio::test(flavor = "multi_thread")]
async fn writers_hear_about_each_other() {
    let daemon = Daemon::start().await;
    let tab = machine_shell(&daemon).await;
    let mut first = Attach::start(&daemon, open(&tab, 0)).await;
    first.wait_until(|attach| !attach.states.is_empty()).await;

    let mut second = Attach::start(&daemon, open(&tab, 0)).await;
    second.wait_until(|attach| !attach.states.is_empty()).await;
    first
        .wait_until(|attach| {
            attach
                .states
                .last()
                .is_some_and(|state| state.other_writers == 1)
        })
        .await;

    assert_eq!(second.states[0].other_writers, 1);
    drop(second);
    first
        .wait_until(|attach| {
            attach
                .states
                .last()
                .is_some_and(|state| state.other_writers == 0)
        })
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_shell_that_exits_says_so() {
    let daemon = Daemon::start().await;
    let tab = machine_shell(&daemon).await;
    let mut attach = Attach::start(&daemon, open(&tab, 0)).await;

    attach.type_text("exit 3\n").await;
    attach
        .wait_until(|attach| {
            attach
                .states
                .last()
                .is_some_and(|state| state.writable() == Writable::Exited)
        })
        .await;

    let state = attach.states.last().unwrap();
    assert_eq!(state.exit_code, 3);
    assert_eq!(state.status(), pb::SessionStatus::Exited);
    attach.wait_until(|attach| !attach.notices.is_empty()).await;
    assert_eq!(attach.notices[0].kind(), pb::terminal_notice::Kind::Exited);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_tab_is_not_found() {
    let daemon = Daemon::start().await;

    let refused = Attach::try_start(
        &daemon,
        open(
            &pb::TabId {
                value: "tb_missing".to_owned(),
            },
            0,
        ),
    )
    .await
    .err()
    .unwrap();

    assert_eq!(not_found(&refused), pb::EntityKind::Tab);
}

const TYPING_SESSION: &str = "se_typing";

/// A daemon with one session and a shell tab open in it.
async fn session_shell() -> (Daemon, pb::TabId) {
    let dir = folder();
    let store = Store::open(state_folder(dir.path()).join("store")).unwrap();
    store
        .put_session(&SessionRecord {
            id: TYPING_SESSION.to_owned(),
            channel: "ch_typing".to_owned(),
            machine: String::new(),
            title: "typing".to_owned(),
            agent: "sh".to_owned(),
            folder: dir.path().to_owned(),
            created_unix_ms: 0,
        })
        .unwrap();
    drop(store);
    let daemon = Daemon::start_in(dir).await;
    let tab = daemon
        .terminals()
        .await
        .open_shell_tab(pb::OpenShellTabRequest {
            idempotency_key: None,
            session: Some(typing_session()),
            repo: None,
            folder: String::new(),
        })
        .await
        .unwrap()
        .into_inner()
        .id
        .unwrap();
    (daemon, tab)
}

fn typing_session() -> pb::SessionId {
    pb::SessionId {
        value: TYPING_SESSION.to_owned(),
    }
}

async fn type_into_session(daemon: &Daemon, text: &str) -> Result<pb::TypeResponse, tonic::Status> {
    daemon
        .terminals()
        .await
        .r#type(pb::TypeRequest {
            session: Some(typing_session()),
            text: text.to_owned(),
            ..Default::default()
        })
        .await
        .map(tonic::Response::into_inner)
}

#[tokio::test(flavor = "multi_thread")]
async fn each_text_typed_into_a_session_reaches_its_shell() {
    let (daemon, tab) = session_shell().await;
    let mut attach = Attach::start(&daemon, open(&tab, 0)).await;

    let first = type_into_session(&daemon, SAY_HELLO).await.unwrap();
    attach.wait_for("hello").await;
    let second = type_into_session(&daemon, SAY_WORLD).await.unwrap();
    attach.wait_for("world").await;

    assert!(first.typed);
    assert!(second.typed);
}

#[tokio::test(flavor = "multi_thread")]
async fn typing_says_it_submitted_only_text_that_ends_in_a_newline() {
    let (daemon, tab) = session_shell().await;
    let mut attach = Attach::start(&daemon, open(&tab, 0)).await;

    let left = type_into_session(&daemon, "echo sub").await.unwrap();
    assert!(left.typed);
    assert!(!left.submitted);
    let sent = type_into_session(&daemon, "mitted-$((2+3))\n").await.unwrap();
    assert!(sent.typed);
    assert!(sent.submitted);
    // The return ran the whole line, the text left unsent first.
    attach.wait_for("submitted-5").await;
}

/// A session whose agent turns come from Claude Code's hooks: the agent
/// runs the hook commands the daemon wrote when it sees `run:<command>`.
struct BusyFixture {
    daemon: Daemon,
    session: pb::SessionId,
}

impl BusyFixture {
    async fn new() -> Self {
        let dir = folder();
        let world = World::new(dir.path());
        let script = dir.path().join("busy-agent.sh");
        std::fs::write(
            &script,
            format!(
                r#"CONV_DIR='{}'
mode=$1; id=$2
case "$mode" in
  --session-id) f="$CONV_DIR/$id.jsonl"; : > "$f" ;;
  --resume) f="$CONV_DIR/$id.jsonl" ;;
esac
echo "agent up $mode"
while IFS= read -r line; do
  case "$line" in
    run:*) eval "${{line#run:}}" ;;
    *) echo "got: $line" ;;
  esac
  printf '%s\n' "$line" >> "$f"
done
"#,
                world.conv.display(),
            ),
        )
        .unwrap();
        let agents = serde_json::json!([{
            "name": common::AGENT_NAME,
            "program": "/bin/sh",
            "start_args": [script, "--session-id", "{conversation}"],
            "resume_args": [script, "--resume", "{conversation}"],
            "conversation_file": format!("{}/{{conversation}}.jsonl", world.conv.display()),
            "turn_end": "hooks",
        }]);
        std::fs::write(&world.agents, agents.to_string()).unwrap();
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

    /// The command Claude Code runs for `event`, as the daemon wrote it.
    fn hook_command(&self, event: &str) -> String {
        let file = self.daemon.state.join("claude-hooks.json");
        let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(file).unwrap()).unwrap();
        json["hooks"][event][0]["hooks"][0]["command"]
            .as_str()
            .unwrap_or_else(|| panic!("no hook for {event}"))
            .to_owned()
    }

    /// The agent runs `event`'s hook command, so its turn changes.
    async fn fire(&self, attach: &mut Attach, event: &str) {
        attach.type_text(&format!("run:{}\r", self.hook_command(event))).await;
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
            .unwrap()
            .into_inner()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn typing_now_says_whether_the_agent_was_mid_turn() {
    let fix = BusyFixture::new().await;
    let tab = agent_tab(&fix.daemon, &fix.session).await;
    let mut attach = Attach::start(&fix.daemon, open(&tab, 0)).await;

    fix.fire(&mut attach, "Stop").await;
    wait_status(&fix.daemon, &fix.session, pb::SessionStatus::Idle).await;
    let idle = fix.type_now("between turns\n").await;
    assert!(idle.typed);
    assert!(!idle.agent_busy);

    fix.fire(&mut attach, "UserPromptSubmit").await;
    wait_status(&fix.daemon, &fix.session, pb::SessionStatus::Working).await;
    let busy = fix.type_now("mid turn\n").await;
    assert!(busy.typed);
    assert!(busy.agent_busy);
    assert!(busy.queued.is_none());

    fix.fire(&mut attach, "Stop").await;
    wait_status(&fix.daemon, &fix.session, pb::SessionStatus::Idle).await;
    assert!(!fix.type_now("after the turn\n").await.agent_busy);
}

/// An agent whose turn ended with no hook to say so (Copilot's idle prompt
/// after its output went quiet) is PROBABLY_IDLE, and typing to it is not
/// typing mid-turn (D775).
#[tokio::test(flavor = "multi_thread")]
async fn typing_to_an_agent_at_its_prompt_with_no_idle_hook_is_not_busy() {
    let fix = BusyFixture::new().await;
    let tab = agent_tab(&fix.daemon, &fix.session).await;
    let mut attach = Attach::start(&fix.daemon, open(&tab, 0)).await;

    fix.fire(&mut attach, "UserPromptSubmit").await;
    wait_status(&fix.daemon, &fix.session, pb::SessionStatus::Working).await;
    assert!(fix.type_now("mid turn\n").await.agent_busy);

    wait_status(&fix.daemon, &fix.session, pb::SessionStatus::ProbablyIdle).await;
    let typed = fix.type_now("at the prompt\n").await;
    assert!(typed.typed);
    assert!(!typed.agent_busy);
}

/// D762: a session whose agent, as Copilot does in a folder it wasn't
/// told to trust (a fresh session, or one moved or resumed elsewhere),
/// first asks on its screen whether to trust the folder, reading one key
/// itself. Only after that does its input take lines.
async fn trust_asking_session() -> (Daemon, pb::SessionId) {
    let dir = folder();
    let world = World::new(dir.path());
    let script = dir.path().join("trust-agent.sh");
    std::fs::write(
        &script,
        format!(
            r#"CONV_DIR='{}'
mode=$1; id=$2
case "$mode" in
  --session-id) f="$CONV_DIR/$id.jsonl"; : > "$f" ;;
  --resume) f="$CONV_DIR/$id.jsonl" ;;
esac
stty raw -echo min 0 time 1
sel=1
draw() {{
  printf '\033[2J\033[HConfirm folder trust\r\n/work/fix-login\r\nDo you trust the files in this folder?\r\n'
  for i in 1 2 3; do
    case $i in 1) l='Yes' ;; 2) l='Yes, and remember this folder for future sessions' ;; 3) l='No (Esc)' ;; esac
    if [ "$i" = "$sel" ]; then printf '> %s. %s\r\n' "$i" "$l"; else printf '  %s. %s\r\n' "$i" "$l"; fi
  done
}}
draw
answer=''
while [ -z "$answer" ]; do
  c=$(dd bs=1 count=1 2>/dev/null)
  case "$c" in
    "$(printf '\r')") answer=$sel ;;
    "$(printf '\033')")
      case "$(dd bs=1 count=2 2>/dev/null)" in
        '[B') [ "$sel" -lt 3 ] && sel=$((sel + 1)); draw ;;
        '[A') [ "$sel" -gt 1 ] && sel=$((sel - 1)); draw ;;
        *) answer=no ;;
      esac ;;
    '') ;;
    *) answer="other $c" ;;
  esac
done
stty sane
printf '\033[2J\033[H'
echo "trust answered: $answer"
while IFS= read -r line; do echo "got: $line"; printf '%s\n' "$line" >> "$f"; done
"#,
            world.conv.display(),
        ),
    )
    .unwrap();
    let agents = serde_json::json!([{
        "name": common::AGENT_NAME,
        "program": "/bin/sh",
        "start_args": [script, "--session-id", "{conversation}"],
        "resume_args": [script, "--resume", "{conversation}"],
        "conversation_file": format!("{}/{{conversation}}.jsonl", world.conv.display()),
        "turn_end": "hooks",
    }]);
    std::fs::write(&world.agents, agents.to_string()).unwrap();
    world.remote("acme", "api");
    let mut args = world.args();
    args.extend(["--humata".to_owned(), binary("HUMATA").display().to_string()]);
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let daemon = Daemon::start_with(Build::Admin, dir, &refs).await;
    let (channel, setup) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    assert_eq!(setup.state, pb::OperationState::Succeeded as i32, "{:?}", setup.error);
    let (session, operation) = start_session(&daemon, &channel, "Fix login").await;
    assert_eq!(operation.state, pb::OperationState::Succeeded as i32, "{:?}", operation.error);
    (daemon, session)
}

async fn type_text(daemon: &Daemon, session: &pb::SessionId, text: &str) -> Result<pb::TypeResponse, tonic::Status> {
    daemon
        .terminals()
        .await
        .r#type(pb::TypeRequest {
            session: Some(session.clone()),
            text: text.to_owned(),
            ..Default::default()
        })
        .await
        .map(tonic::Response::into_inner)
}

#[tokio::test(flavor = "multi_thread")]
async fn text_typed_while_the_agent_asks_folder_trust_is_refused_and_never_answers_it() {
    let (daemon, session) = trust_asking_session().await;
    let tab = agent_tab(&daemon, &session).await;
    let mut attach = Attach::start(&daemon, open(&tab, 0)).await;
    attach.wait_for("Do you trust the files in this folder?").await;

    // Refused, not reported as submitted, and the question is left asked.
    let refused = type_text(&daemon, &session, "hello\n").await.unwrap_err();
    assert_eq!(refused.code(), tonic::Code::FailedPrecondition);
    let no_submit = type_text(&daemon, &session, "hello").await.unwrap_err();
    assert_eq!(no_submit.code(), tonic::Code::FailedPrecondition);

    // Enter alone is the person answering it on purpose.
    type_text(&daemon, &session, "\n").await.unwrap();
    attach.wait_for("trust answered: 1").await;

    // Once answered, the agent's input takes the text.
    let sent = type_text(&daemon, &session, "hello\n").await.unwrap();
    assert!(sent.submitted);
    attach.wait_for("got: hello").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn typing_into_a_shell_that_exited_is_refused() {
    let (daemon, tab) = session_shell().await;
    let mut attach = Attach::start(&daemon, open(&tab, 0)).await;
    type_into_session(&daemon, "exit 3\n").await.unwrap();
    attach
        .wait_until(|attach| {
            attach
                .states
                .last()
                .is_some_and(|state| state.writable() == Writable::Exited)
        })
        .await;

    let refused = type_into_session(&daemon, SAY_HELLO).await.unwrap_err();

    assert_ne!(refused.code(), tonic::Code::Ok);
}

async fn type_keyed(
    daemon: &Daemon,
    key: &str,
    text: &str,
) -> Result<pb::TypeResponse, tonic::Status> {
    daemon
        .terminals()
        .await
        .r#type(pb::TypeRequest {
            session: Some(typing_session()),
            text: text.to_owned(),
            idempotency_key: Some(pb::IdempotencyKey {
                value: key.to_owned(),
            }),
            ..Default::default()
        })
        .await
        .map(tonic::Response::into_inner)
}

/// A command that adds one `x` to `file`, so a test counts how often it ran.
fn add_a_mark(file: &Path) -> String {
    format!("printf x >> '{}'\n", file.display())
}

/// Types a command with key `key` and waits until the shell has run it, so
/// whatever was typed before it has run too.
async fn ran_up_to_now(daemon: &Daemon, tab: &pb::TabId, key: &str) {
    let mut attach = Attach::start(daemon, open(tab, 0)).await;
    type_keyed(daemon, key, &format!("echo {key}''-ran\n"))
        .await
        .unwrap();
    attach.wait_for(&format!("{key}-ran")).await;
}

/// The client lost the answer to a Type and retries it with the same key
/// after the daemon restarted: the retry answers typed and types nothing.
#[tokio::test(flavor = "multi_thread")]
async fn a_keyed_type_retried_after_a_restart_is_not_typed_again() {
    let (mut daemon, tab) = session_shell().await;
    let marks = folder();
    let file = marks.path().join("marks");

    let first = type_keyed(&daemon, "type-1", &add_a_mark(&file)).await.unwrap();
    ran_up_to_now(&daemon, &tab, "settle-1").await;
    daemon.restart().await;
    let retry = type_keyed(&daemon, "type-1", &add_a_mark(&file)).await.unwrap();
    ran_up_to_now(&daemon, &tab, "settle-2").await;

    assert!(first.typed);
    assert!(retry.typed);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "x");
}

/// The daemon dies once the shell has the text and before it notes so. A
/// retry with the same key asks the tab's holder, which typed it: the
/// retry answers typed and types nothing again (D426).
#[tokio::test(flavor = "multi_thread")]
async fn a_keyed_type_cut_short_after_the_shell_has_it_answers_typed() {
    let (mut daemon, tab) = session_shell().await;
    let marks = folder();
    let file = marks.path().join("marks");
    daemon
        .restart_env(&[("HUMATA2_STOP_AT", "terminal.type.typed")])
        .await;

    let cut = type_keyed(&daemon, "type-1", &add_a_mark(&file)).await;
    daemon.restart().await;
    let retry = type_keyed(&daemon, "type-1", &add_a_mark(&file)).await;
    ran_up_to_now(&daemon, &tab, "settle-1").await;

    assert!(cut.is_err(), "the daemon died before it answered");
    assert!(retry.expect("the holder says it typed the text").typed);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "x");
}

/// The daemon dies once it noted the key and before the shell has the
/// text. A retry with the same key asks the tab's holder, which never
/// typed it: the retry types it, once (D426).
#[tokio::test(flavor = "multi_thread")]
async fn a_keyed_type_cut_short_before_the_shell_has_it_types_on_retry() {
    let (mut daemon, tab) = session_shell().await;
    let marks = folder();
    let file = marks.path().join("marks");
    daemon
        .restart_env(&[("HUMATA2_STOP_AT", "terminal.type.held")])
        .await;

    let cut = type_keyed(&daemon, "type-1", &add_a_mark(&file)).await;
    daemon.restart().await;
    let retry = type_keyed(&daemon, "type-1", &add_a_mark(&file)).await;
    ran_up_to_now(&daemon, &tab, "settle-1").await;

    assert!(cut.is_err(), "the daemon died before it answered");
    assert!(retry.expect("the holder says nothing was typed").typed);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "x");
}

/// The user answers `id` with an approval, and the agent waits for what the
/// approved call returned: its type name and encoded response.
async fn approve_and_wait(
    daemon: &common::Daemon,
    token: &str,
    id: &pb::RequestId,
) -> pb::WaitResponse {
    let mut inbox = pb::inbox_service_client::InboxServiceClient::new(daemon.channel().await);
    inbox
        .answer_request(pb::AnswerRequestRequest {
            id: Some(id.clone()),
            answer: Some(pb::RequestAnswer {
                answer: Some(pb::request_answer::Answer::Approve(pb::Approve {})),
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    let mut stream = inbox
        .wait_for_answer(agent_request(
            pb::WaitForAnswerRequest {
                id: Some(id.clone()),
                wait_seconds: 10,
            },
            token,
        ))
        .await
        .unwrap()
        .into_inner();
    loop {
        let message = tokio::time::timeout(common::WAIT, stream.message())
            .await
            .expect("the wait ended in time")
            .unwrap()
            .expect("the wait ended with an answer");
        if !message.keepalive {
            return message;
        }
    }
}

/// The request an agent's call is waiting on, from the pending error.
fn pending_request(refused: &tonic::Status) -> pb::RequestId {
    match common::error_of(refused).kind {
        Some(pb::error::Kind::ApprovalPending(pending)) => pending.request.unwrap(),
        other => panic!("expected approval_pending, got {other:?}"),
    }
}

fn agent_request<T>(message: T, token: &str) -> tonic::Request<T> {
    let mut request = tonic::Request::new(message);
    request.metadata_mut().insert(
        "humata-session-token",
        tonic::metadata::MetadataValue::try_from(token).unwrap(),
    );
    request
}

/// Makes the channel's rule for `capability` Ask me.
async fn ask_me(daemon: &common::Daemon, channel: &pb::ChannelId, capability: pb::Capability) {
    pb::access_service_client::AccessServiceClient::new(daemon.channel().await)
        .set_channel_rule(pb::SetChannelRuleRequest {
            channel: Some(channel.clone()),
            capability: capability as i32,
            decision: pb::Decision::Ask as i32,
            ..Default::default()
        })
        .await
        .unwrap();
}

/// The session token a shell tab of `session` was given.
async fn session_token(daemon: &common::Daemon, session: &pb::SessionId) -> String {
    let tab = daemon
        .terminals()
        .await
        .open_shell_tab(pb::OpenShellTabRequest {
            idempotency_key: None,
            session: Some(session.clone()),
            repo: None,
            folder: String::new(),
        })
        .await
        .unwrap()
        .into_inner()
        .id
        .unwrap();
    let file = daemon.dir.path().join(format!("token-{}", session.value));
    let mut attach = common::Attach::start(daemon, common::open(&tab, 0)).await;
    attach
        .type_text(&format!(
            "printf '%s' \"$HUMATA_SESSION_TOKEN\" > {}\r",
            file.display()
        ))
        .await;
    let deadline = std::time::Instant::now() + common::WAIT;
    loop {
        if let Ok(text) = std::fs::read_to_string(&file)
            && !text.is_empty()
        {
            return text;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the token was never written"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agents_typing_into_another_session_that_a_rule_sends_to_ask_runs_once_approved() {
    let dir = folder();
    let store = Store::open(state_folder(dir.path()).join("store")).unwrap();
    for (id, title) in [(TYPING_SESSION, "typing"), ("se_typist", "typist")] {
        store
            .put_session(&SessionRecord {
                id: id.to_owned(),
                channel: "ch_typing".to_owned(),
                machine: String::new(),
                title: title.to_owned(),
                agent: "sh".to_owned(),
                folder: dir.path().to_owned(),
                created_unix_ms: 0,
            })
            .unwrap();
    }
    drop(store);
    let daemon = Daemon::start_in(dir).await;
    let target = daemon
        .terminals()
        .await
        .open_shell_tab(pb::OpenShellTabRequest {
            idempotency_key: None,
            session: Some(typing_session()),
            repo: None,
            folder: String::new(),
        })
        .await
        .unwrap()
        .into_inner()
        .id
        .unwrap();
    let mut watching = Attach::start(&daemon, open(&target, 0)).await;
    let token = session_token(
        &daemon,
        &pb::SessionId {
            value: "se_typist".to_owned(),
        },
    )
    .await;
    ask_me(
        &daemon,
        &pb::ChannelId {
            value: "ch_typing".to_owned(),
        },
        pb::Capability::SessionType,
    )
    .await;

    let refused = daemon
        .terminals()
        .await
        .r#type(agent_request(
            pb::TypeRequest {
                session: Some(typing_session()),
                text: SAY_HELLO.to_owned(),
                ..Default::default()
            },
            &token,
        ))
        .await
        .unwrap_err();
    let waited = approve_and_wait(&daemon, &token, &pending_request(&refused)).await;

    assert_eq!(waited.result_type, "humata.v1.TypeResponse");
    let typed = <pb::TypeResponse as prost::Message>::decode(waited.result.as_slice()).unwrap();
    assert!(typed.typed);
    watching.wait_for("hello").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_tab_is_told_the_socket_its_daemon_serves_on() {
    let dir = folder();
    let store = Store::open(state_folder(dir.path()).join("store")).unwrap();
    store
        .put_session(&SessionRecord {
            id: TYPING_SESSION.to_owned(),
            channel: "ch_typing".to_owned(),
            machine: String::new(),
            title: "typing".to_owned(),
            agent: "sh".to_owned(),
            folder: dir.path().to_owned(),
            created_unix_ms: 0,
        })
        .unwrap();
    drop(store);
    let daemon = Daemon::start_in(dir).await;
    let tab = daemon
        .terminals()
        .await
        .open_shell_tab(pb::OpenShellTabRequest {
            idempotency_key: None,
            session: Some(typing_session()),
            repo: None,
            folder: String::new(),
        })
        .await
        .unwrap()
        .into_inner()
        .id
        .unwrap();
    let file = daemon.dir.path().join("socket-seen");
    let mut attach = Attach::start(&daemon, open(&tab, 0)).await;
    attach
        .type_text(&format!(
            "printf '%s' \"$HUMATA_SOCKET\" > {}\r",
            file.display()
        ))
        .await;
    let deadline = std::time::Instant::now() + common::WAIT;
    let seen = loop {
        if let Ok(text) = std::fs::read_to_string(&file)
            && !text.is_empty()
        {
            break text;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the tab never wrote HUMATA_SOCKET"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    };

    let expected = std::path::absolute(&daemon.socket).unwrap();
    assert_eq!(std::path::PathBuf::from(seen), expected);
}

const HERE_SESSION: &str = "se_here";

/// A daemon with one session, in a folder with a subfolder `deep/open_here_sub`,
/// and no tab open yet. The session's own folder is `<dir>/work`.
async fn session_with_folders() -> Daemon {
    let dir = folder();
    let work = dir.path().join("work");
    std::fs::create_dir_all(work.join("deep").join("open_here_sub")).unwrap();
    std::fs::write(work.join("a_file.txt"), b"text").unwrap();
    let store = Store::open(state_folder(dir.path()).join("store")).unwrap();
    store
        .put_session(&SessionRecord {
            id: HERE_SESSION.to_owned(),
            channel: "ch_here".to_owned(),
            machine: String::new(),
            title: "here".to_owned(),
            agent: "sh".to_owned(),
            folder: work,
            created_unix_ms: 0,
        })
        .unwrap();
    drop(store);
    Daemon::start_in(dir).await
}

fn here_session() -> pb::SessionId {
    pb::SessionId {
        value: HERE_SESSION.to_owned(),
    }
}

async fn open_in_folder(daemon: &Daemon, folder: &Path) -> Result<pb::Tab, tonic::Status> {
    daemon
        .terminals()
        .await
        .open_shell_tab(pb::OpenShellTabRequest {
            idempotency_key: None,
            session: Some(here_session()),
            repo: None,
            folder: folder.display().to_string(),
        })
        .await
        .map(tonic::Response::into_inner)
}

async fn here_tabs(daemon: &Daemon) -> usize {
    daemon
        .terminals()
        .await
        .list_tabs(pb::ListTabsRequest {
            session: Some(here_session()),
        })
        .await
        .unwrap()
        .into_inner()
        .tabs
        .len()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_shell_tab_opened_in_a_subfolder_of_the_session_starts_there() {
    let daemon = session_with_folders().await;
    let sub = daemon.dir.path().join("work/deep/open_here_sub");

    let tab = open_in_folder(&daemon, &sub).await.unwrap();

    let mut attach = Attach::start(&daemon, open(&tab.id.unwrap(), 0)).await;
    attach.type_text("pwd\n").await;
    attach.wait_for("open_here_sub").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_folder_outside_the_sessions_places_opens_nothing() {
    let daemon = session_with_folders().await;
    let elsewhere = folder();
    let dir = daemon.dir.path();
    let work = dir.join("work");
    std::os::unix::fs::symlink(elsewhere.path(), work.join("escape")).unwrap();

    // Another folder of the machine, a folder reached through "..", and a
    // link inside the session that leads out.
    for refused in [
        elsewhere.path().to_owned(),
        work.join("deep/../../elsewhere"),
        work.join("escape"),
    ] {
        let status = open_in_folder(&daemon, &refused).await.unwrap_err();
        let (field, problem) = invalid_field(&status);
        assert_eq!(field, "folder");
        assert_eq!(problem, pb::invalid_field::Problem::NotAllowedHere);
    }
    assert_eq!(here_tabs(&daemon).await, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_folder_or_a_file_opens_nothing() {
    let daemon = session_with_folders().await;
    let work = daemon.dir.path().join("work");

    let missing = open_in_folder(&daemon, &work.join("not_there")).await.unwrap_err();
    assert_eq!(not_found(&missing), pb::EntityKind::File);
    let file = open_in_folder(&daemon, &work.join("a_file.txt")).await.unwrap_err();
    assert_eq!(invalid_field(&file).0, "folder");
    let relative = open_in_folder(&daemon, Path::new("deep")).await.unwrap_err();
    assert_eq!(invalid_field(&relative).0, "folder");

    assert_eq!(here_tabs(&daemon).await, 0);
}
