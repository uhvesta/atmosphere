//! A running agent's turn (SE-21, SE-22, D169) through a real `humatad2`:
//! the agent runs the hook commands the daemon wrote for Claude Code, from
//! its own terminal, and they report what turn it is in; output that goes
//! quiet makes it Probably idle when no hook has spoken; only the session's
//! own agent may report; and a plain shell has no turns.

mod common;

use common::{
    Attach, Build, Daemon, WAIT, World, agent_tab, binary, create_channel, create_script_token,
    folder, get_session, open, shell_token, start_session, wait_status,
};
use humata2_grpc::humata::v1 as pb;
use pb::SessionStatus as Status;
use tonic::{Code, Streaming};

/// The test agent: every line typed to it is answered, except
/// `run:<command>`, which it runs, as Claude Code runs a hook's command.
fn agent_script(world: &World) -> String {
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
    )
}

/// A world whose agent is the one above, its turn ends coming from
/// `turn_end` (`hooks`, `output` or `none`).
fn world_with_agent(dir: &std::path::Path, turn_end: &str) -> World {
    let world = World::new(dir);
    let script = dir.join("turn-agent.sh");
    std::fs::write(&script, agent_script(&world)).unwrap();
    let agents = serde_json::json!([{
        "name": common::AGENT_NAME,
        "program": "/bin/sh",
        "start_args": [script, "--session-id", "{conversation}"],
        "resume_args": [script, "--resume", "{conversation}"],
        "conversation_file": format!("{}/{{conversation}}.jsonl", world.conv.display()),
        "turn_end": turn_end,
    }]);
    std::fs::write(&world.agents, agents.to_string()).unwrap();
    world
}

struct Fixture {
    daemon: Daemon,
    channel: pb::Channel,
    session: pb::SessionId,
}

impl Fixture {
    async fn new(turn_end: &str) -> Self {
        let dir = folder();
        let world = world_with_agent(dir.path(), turn_end);
        world.remote("acme", "api");
        let mut args = world.args();
        args.extend(["--humata".to_owned(), binary("HUMATA").display().to_string()]);
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let daemon = Daemon::start_with(Build::Admin, dir, &refs).await;
        let (channel, setup) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
        assert_eq!(setup.state, pb::OperationState::Succeeded as i32, "{:?}", setup.error);
        let (session, operation) = start_session(&daemon, &channel, "Fix login").await;
        assert_eq!(operation.state, pb::OperationState::Succeeded as i32, "{:?}", operation.error);
        Self {
            daemon,
            channel,
            session,
        }
    }

    async fn attach(&self) -> Attach {
        let tab = agent_tab(&self.daemon, &self.session).await;
        Attach::start(&self.daemon, open(&tab, 0)).await
    }

    async fn status(&self) -> Status {
        get_session(&self.daemon, &self.session).await.status()
    }

    /// The command Claude Code runs for `event`, as the daemon wrote it.
    fn hook_command(&self, event: &str) -> String {
        let file = self.daemon.state.join("claude-hooks.json");
        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(file).unwrap()).unwrap();
        json["hooks"][event][0]["hooks"][0]["command"]
            .as_str()
            .unwrap_or_else(|| panic!("no hook for {event}"))
            .to_owned()
    }

    /// The agent runs `event`'s hook command.
    async fn fire(&self, attach: &mut Attach, event: &str) {
        attach
            .type_text(&format!("run:{}\r", self.hook_command(event)))
            .await;
    }

    async fn events(&self) -> Streaming<pb::Event> {
        pb::event_service_client::EventServiceClient::new(self.daemon.channel().await)
            .subscribe(pb::SubscribeRequest {
                topics: vec![pb::Topic::Sessions as i32],
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
    }

    /// The next time the feed gives this session, as it shows then.
    async fn next_shown(&self, events: &mut Streaming<pb::Event>) -> pb::Session {
        tokio::time::timeout(WAIT, async {
            loop {
                let event = events.message().await.unwrap().expect("the feed ended");
                if let Some(pb::event::Payload::Session(session)) = event.payload
                    && session.id == Some(self.session.clone())
                {
                    return session;
                }
            }
        })
        .await
        .expect("the session never came on the feed")
    }

    /// The next status the feed gives this session.
    async fn next_status(&self, events: &mut Streaming<pb::Event>) -> Status {
        self.next_shown(events).await.status()
    }

    /// Waits until nothing else about the session is still on its way to
    /// the feed, once a hook has said its turn, so the next event is the
    /// next change. A feed event shows the session as it is when told, so a
    /// late one (the output fallback's, a read mark's, the clones' first
    /// pointers) would show the hook's state again. With a hook heard,
    /// output changes no status; the agent's watch and the read marks each
    /// tell in order; the clones' pointers are told when they change, and
    /// nothing changes them here after the first look.
    async fn settle(&self, attach: &mut Attach, events: &mut Streaming<pb::Event>) {
        if get_session(&self.daemon, &self.session).await.pointers.is_empty() {
            while self.next_shown(events).await.pointers.is_empty() {}
        }
        let mut marks = pb::mark_service_client::MarkServiceClient::new(self.daemon.channel().await);
        // A view clears the read mark (and is gone once dropped)...
        let viewing = marks
            .view_session(pb::ViewSessionRequest {
                session: Some(self.session.clone()),
            })
            .await
            .unwrap()
            .into_inner();
        drop(viewing);
        // ...so output marks it unread again: the watch got that far, past
        // every status it told before.
        tokio::time::timeout(WAIT, async {
            for line in 0.. {
                attach.type_text(&format!("hello {line}\r")).await;
                attach.wait_for(&format!("got: hello {line}")).await;
                if get_session(&self.daemon, &self.session).await.unread {
                    return;
                }
            }
        })
        .await
        .expect("the agent's output never marked the session unread");
        // A pin is told after the read marks before it.
        marks
            .set_marks(pb::SetMarksRequest {
                idempotency_key: None,
                entity: Some(pb::EntityRef {
                    entity: Some(pb::entity_ref::Entity::Session(self.session.clone())),
                }),
                pinned: Some(true),
                vip: None,
            })
            .await
            .unwrap();
        while !self.next_shown(events).await.pinned {}
    }
}

fn report(state: pb::TurnState, at: i64) -> pb::ReportTurnRequest {
    pb::ReportTurnRequest {
        state: state as i32,
        reported_at_unix_ns: at,
    }
}

fn as_agent<T>(message: T, token: &str) -> tonic::Request<T> {
    let mut request = tonic::Request::new(message);
    request.metadata_mut().insert(
        "humata-session-token",
        tonic::metadata::MetadataValue::try_from(token).unwrap(),
    );
    request
}

#[tokio::test(flavor = "multi_thread")]
async fn the_hooks_move_a_session_between_idle_working_and_needing_input() {
    let fix = Fixture::new("hooks").await;
    let mut attach = fix.attach().await;
    fix.fire(&mut attach, "Stop").await;
    let idle = wait_status(&fix.daemon, &fix.session, Status::Idle).await;
    assert_eq!(idle.turn_end(), pb::TurnEndSource::Hooks);

    // Output doesn't undo what a hook said: only the next hook does.
    attach.type_text("hello\r").await;
    attach.wait_for("got: hello").await;
    assert_eq!(fix.status().await, Status::Idle);

    fix.fire(&mut attach, "UserPromptSubmit").await;
    wait_status(&fix.daemon, &fix.session, Status::Working).await;
    fix.fire(&mut attach, "PermissionRequest").await;
    wait_status(&fix.daemon, &fix.session, Status::NeedsInput).await;
    fix.fire(&mut attach, "PermissionDenied").await;
    wait_status(&fix.daemon, &fix.session, Status::Working).await;
    fix.fire(&mut attach, "Elicitation").await;
    wait_status(&fix.daemon, &fix.session, Status::NeedsInput).await;
    fix.fire(&mut attach, "ElicitationResult").await;
    wait_status(&fix.daemon, &fix.session, Status::Working).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn every_change_is_on_the_feed_and_a_repeat_is_not() {
    let fix = Fixture::new("hooks").await;
    let mut events = fix.events().await;
    let mut attach = fix.attach().await;
    fix.fire(&mut attach, "Stop").await;
    // Until the hook is heard the output fallback may move the status
    // (quiet, then the typed line's echo); once it said idle, nothing does.
    while fix.next_status(&mut events).await != Status::Idle {}
    fix.settle(&mut attach, &mut events).await;
    // The same state again says nothing; the next change is the next event.
    fix.fire(&mut attach, "Stop").await;
    fix.fire(&mut attach, "UserPromptSubmit").await;
    assert_eq!(fix.next_status(&mut events).await, Status::Working);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_agents_tab_shows_the_same_turn_as_its_session() {
    let fix = Fixture::new("hooks").await;
    let mut tab = fix.attach().await;
    let mut typing = fix.attach().await;
    fix.fire(&mut typing, "Stop").await;
    // The change is pushed to the tab; it isn't asked for.
    tab.wait_until(|attach| attach.states.last().is_some_and(|state| state.status() == Status::Idle))
        .await;
    wait_status(&fix.daemon, &fix.session, Status::Idle).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_with_no_hook_that_goes_quiet_is_probably_idle_until_it_writes_again() {
    let fix = Fixture::new("output").await;
    let probably = wait_status(&fix.daemon, &fix.session, Status::ProbablyIdle).await;
    assert_eq!(probably.turn_end(), pb::TurnEndSource::OutputFallback);

    let mut attach = fix.attach().await;
    attach.type_text("again\r").await;
    wait_status(&fix.daemon, &fix.session, Status::Working).await;
    // And quiet again once it stops.
    wait_status(&fix.daemon, &fix.session, Status::ProbablyIdle).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_with_no_turns_stays_running_unknown_turn() {
    let fix = Fixture::new("none").await;
    let shown = wait_status(&fix.daemon, &fix.session, Status::RunningUnknownTurn).await;
    assert_eq!(shown.turn_end(), pb::TurnEndSource::None);
    let mut attach = fix.attach().await;
    fix.fire(&mut attach, "Stop").await;
    attach.type_text("after\r").await;
    attach.wait_for("got: after").await;
    assert_eq!(fix.status().await, Status::RunningUnknownTurn);
    assert!(attach.states.iter().all(|state| state.status() == Status::RunningUnknownTurn));
}

#[tokio::test(flavor = "multi_thread")]
async fn only_the_sessions_own_agent_can_report_its_turn() {
    let fix = Fixture::new("hooks").await;
    // The user (the admin daemon takes a call with no credential for them).
    let refused = fix
        .daemon
        .session_service()
        .await
        .report_turn(report(pb::TurnState::Idle, 1))
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::InvalidArgument, "{refused:?}");
    // A script with a token.
    let (_, secret) = create_script_token(
        &fix.daemon,
        "ci",
        &[pb::Capability::ChannelRead],
        &[&fix.channel.id.clone().unwrap()],
        0,
    )
    .await
    .unwrap();
    let mut request = tonic::Request::new(report(pb::TurnState::Idle, 1));
    request.metadata_mut().insert(
        "humata-script-token",
        tonic::metadata::MetadataValue::try_from(secret.as_str()).unwrap(),
    );
    let refused = fix
        .daemon
        .session_service()
        .await
        .report_turn(request)
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::PermissionDenied, "{refused:?}");
    // Neither changed anything.
    assert_ne!(fix.status().await, Status::Idle);

    // The session's own agent can, and repeating it is fine.
    let (token, _tab) = shell_token(&fix.daemon, &fix.session).await;
    for _ in 0..2 {
        fix.daemon
            .session_service()
            .await
            .report_turn(as_agent(report(pb::TurnState::Idle, 2_000), &token))
            .await
            .unwrap();
    }
    wait_status(&fix.daemon, &fix.session, Status::Idle).await;
    // A report made before that one, arriving after it, is dropped.
    fix.daemon
        .session_service()
        .await
        .report_turn(as_agent(report(pb::TurnState::Working, 1_000), &token))
        .await
        .unwrap();
    assert_eq!(fix.status().await, Status::Idle);
    // A state nobody can mean is refused.
    let unknown = fix
        .daemon
        .session_service()
        .await
        .report_turn(as_agent(report(pb::TurnState::Unspecified, 3_000), &token))
        .await
        .unwrap_err();
    assert_eq!(unknown.code(), Code::InvalidArgument);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_hook_survives_a_daemon_restart() {
    let mut fix = Fixture::new("hooks").await;
    let mut attach = fix.attach().await;
    fix.fire(&mut attach, "Stop").await;
    wait_status(&fix.daemon, &fix.session, Status::Idle).await;
    // The line after the hook was read, so the hook's command has ended.
    attach.type_text("after\r").await;
    attach.wait_for("got: after").await;
    drop(attach);

    // The holder, and so the agent, outlive the daemon.
    fix.daemon.restart().await;
    wait_status(&fix.daemon, &fix.session, Status::Idle).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_resume_forgets_what_the_hooks_said() {
    let fix = Fixture::new("hooks").await;
    let store = || humata2_store::Store::open(fix.daemon.state.join("store")).unwrap();
    let mut attach = fix.attach().await;
    fix.fire(&mut attach, "Stop").await;
    wait_status(&fix.daemon, &fix.session, Status::Idle).await;
    assert!(store().hook_turn(&fix.session.value).unwrap().is_some());
    drop(attach);

    let mut sessions = fix.daemon.session_service().await;
    sessions
        .stop_session(pb::StopSessionRequest {
            idempotency_key: None,
            id: Some(fix.session.clone()),
        })
        .await
        .unwrap();
    let operation = sessions
        .resume_session(pb::ResumeSessionRequest {
            idempotency_key: None,
            id: Some(fix.session.clone()),
            mode: pb::resume_session_request::Mode::Resume as i32,
        })
        .await
        .unwrap()
        .into_inner();
    common::finished(&fix.daemon, &operation).await;
    assert_eq!(store().hook_turn(&fix.session.value).unwrap(), None);
    // The resumed agent is Working or, with no hook, quiet: never the old Idle.
    let shown = get_session(&fix.daemon, &fix.session).await.status();
    assert_ne!(shown, Status::Idle);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_shell_tab_of_the_session_has_no_turns_even_as_its_agent_reports() {
    let fix = Fixture::new("hooks").await;
    let (_token, mut shell) = shell_token(&fix.daemon, &fix.session).await;
    // A command in the session's shell is the session's agent to the daemon.
    shell
        .type_text(&format!("{}\r", fix.hook_command("Stop")))
        .await;
    wait_status(&fix.daemon, &fix.session, Status::Idle).await;
    shell.type_text("echo don''e\r").await;
    shell.wait_for("done").await;
    assert!(!shell.states.is_empty());
    assert!(
        shell
            .states
            .iter()
            .all(|state| state.status() == Status::RunningUnknownTurn),
        "{:?}",
        shell.states
    );
}
