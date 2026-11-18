//! Notes and questions between sessions (TP-5) through a real `humatad2`:
//! a note is typed into the target's terminal once its turn is over, as
//! one message naming the sender; an agent's note asks the user first; a
//! question is typed with its id, and the asked session's answer (or its
//! no) is what the asking call returns, or it expires. Only the asked
//! session answers, and a session whose agent isn't running gets nothing.

mod common;

use std::time::{Duration, Instant};

use common::{
    Attach, Build, Daemon, WAIT, World, approve, create_channel, error_of, first_pending, folder,
    get_session, shell_token, start_session, wait_status,
};
use humata2_grpc::humata::v1 as pb;
use pb::ask_session_response::Outcome;
use tonic::Code;

fn as_agent<T>(message: T, token: &str) -> tonic::Request<T> {
    let mut request = tonic::Request::new(message);
    request.metadata_mut().insert(
        "humata-session-token",
        tonic::metadata::MetadataValue::try_from(token).unwrap(),
    );
    request
}

struct Scene {
    daemon: Daemon,
    world: World,
    asker: pb::SessionId,
    target: pb::SessionId,
    /// The target's agent, as its hooks speak for it.
    target_token: String,
    /// The asker's agent.
    asker_token: String,
    reported: i64,
    _tabs: (Attach, Attach),
}

impl Scene {
    /// Two sessions whose agents' turns come from hooks; the target's agent
    /// has said its turn is over.
    async fn new() -> Self {
        let dir = folder();
        let world = World::new(dir.path());
        let agents = serde_json::json!([{
            "name": common::AGENT_NAME,
            "program": "/bin/sh",
            "start_args": [world.root.join("agent.sh"), "--session-id", "{conversation}"],
            "resume_args": [world.root.join("agent.sh"), "--resume", "{conversation}"],
            "conversation_file": format!("{}/{{conversation}}.jsonl", world.conv.display()),
            "turn_end": "hooks",
        }]);
        std::fs::write(&world.agents, agents.to_string()).unwrap();
        world.remote("acme", "api");
        let args = world.args();
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let daemon = Daemon::start_with(Build::Admin, dir, &refs).await;
        let (channel, setup) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
        assert_eq!(setup.state, pb::OperationState::Succeeded as i32, "{:?}", setup.error);
        let (asker, started) = start_session(&daemon, &channel, "Asker").await;
        assert_eq!(started.state, pb::OperationState::Succeeded as i32, "{:?}", started.error);
        let (target, started) = start_session(&daemon, &channel, "Target").await;
        assert_eq!(started.state, pb::OperationState::Succeeded as i32, "{:?}", started.error);
        let (asker_token, asker_tab) = shell_token(&daemon, &asker).await;
        let (target_token, target_tab) = shell_token(&daemon, &target).await;
        let mut scene = Self {
            daemon,
            world,
            asker,
            target,
            target_token,
            asker_token,
            reported: 0,
            _tabs: (asker_tab, target_tab),
        };
        scene.turn(pb::TurnState::Idle).await;
        wait_status(&scene.daemon, &scene.target, pb::SessionStatus::Idle).await;
        scene
    }

    /// The target's agent reports its turn.
    async fn turn(&mut self, state: pb::TurnState) {
        self.reported += 1_000;
        let report = pb::ReportTurnRequest {
            state: state as i32,
            reported_at_unix_ns: self.reported,
        };
        self.daemon
            .session_service()
            .await
            .report_turn(as_agent(report, &self.target_token))
            .await
            .unwrap();
    }

    /// The lines typed into the target's agent so far.
    async fn typed(&self) -> Vec<String> {
        let conversation = get_session(&self.daemon, &self.target).await.conversation_id;
        std::fs::read_to_string(self.world.conversation(&conversation))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// Polls until a line typed into the target holds `text`.
    async fn typed_line(&self, text: &str) -> String {
        let deadline = Instant::now() + WAIT;
        loop {
            if let Some(line) = self.typed().await.into_iter().find(|line| line.contains(text)) {
                return line;
            }
            assert!(Instant::now() < deadline, "{text:?} was never typed into the target");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn sessions(&self) -> pb::session_service_client::SessionServiceClient<tonic::transport::Channel> {
        self.daemon.session_service().await
    }

    /// The user asks the target `question` in a task of its own.
    async fn ask(&self, question: &str, timeout_seconds: u32) -> tokio::task::JoinHandle<Result<pb::AskSessionResponse, tonic::Status>> {
        let mut sessions = self.sessions().await;
        let request = pb::AskSessionRequest {
            idempotency_key: None,
            to: Some(self.target.clone()),
            question: question.to_owned(),
            reason: String::new(),
            timeout_seconds,
        };
        tokio::spawn(async move { sessions.ask_session(request).await.map(tonic::Response::into_inner) })
    }

    /// The id typed into the target with the question holding `text`.
    async fn question_id(&self, text: &str) -> pb::QuestionId {
        let line = self.typed_line(text).await;
        let start = line.find("qu_").unwrap_or_else(|| panic!("no question id in {line:?}"));
        let value: String = line[start..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        pb::QuestionId { value }
    }

    async fn answer(
        &self,
        token: &str,
        question: &pb::QuestionId,
        answer: &str,
        decline: bool,
    ) -> Result<pb::AnswerQuestionResponse, tonic::Status> {
        let request = pb::AnswerQuestionRequest {
            idempotency_key: None,
            question: Some(question.clone()),
            answer: answer.to_owned(),
            decline,
        };
        self.sessions()
            .await
            .answer_question(as_agent(request, token))
            .await
            .map(tonic::Response::into_inner)
    }
}

fn note(to: &pb::SessionId, text: &str) -> pb::SendNoteRequest {
    pb::SendNoteRequest {
        idempotency_key: None,
        to: Some(to.clone()),
        text: text.to_owned(),
        reason: String::new(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_note_waits_for_the_targets_turn_to_end_then_is_typed_once() {
    let mut scene = Scene::new().await;
    scene.turn(pb::TurnState::Working).await;
    wait_status(&scene.daemon, &scene.target, pb::SessionStatus::Working).await;

    let mut sessions = scene.sessions().await;
    let request = note(&scene.target, "the schema changed in the api repo");
    let sending = tokio::spawn(async move { sessions.send_note(request).await });
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(!sending.is_finished(), "the note didn't wait for the turn to end");
    assert!(
        !scene.typed().await.iter().any(|line| line.contains("the schema changed")),
        "the note was typed mid-turn"
    );

    scene.turn(pb::TurnState::Idle).await;
    let sent = tokio::time::timeout(WAIT, sending).await.unwrap().unwrap().unwrap().into_inner();
    assert!(sent.typed);
    scene.typed_line("the schema changed in the api repo").await;
    let times = scene
        .typed()
        .await
        .iter()
        .filter(|line| line.contains("the schema changed"))
        .count();
    assert_eq!(times, 1, "typed as one message, once");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agents_note_asks_the_user_first_and_names_the_sending_session() {
    let scene = Scene::new().await;
    let refused = scene
        .sessions()
        .await
        .send_note(as_agent(note(&scene.target, "rebase onto the new main"), &scene.asker_token))
        .await
        .unwrap_err();
    assert!(
        matches!(error_of(&refused).kind, Some(pb::error::Kind::ApprovalPending(_))),
        "{refused:?}"
    );
    assert!(!scene.typed().await.iter().any(|line| line.contains("rebase onto")));

    approve(&scene.daemon, &first_pending(&scene.daemon).await).await;
    let line = scene.typed_line("rebase onto the new main").await;
    assert!(
        line.contains("Asker") && line.contains(&scene.asker.value),
        "the note doesn't name its sender: {line:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_question_returns_the_asked_sessions_answer() {
    let scene = Scene::new().await;
    let asking = scene.ask("which port does the api listen on", 0).await;
    let question = scene.question_id("which port does the api listen on").await;

    // Only the asked session answers it.
    let wrong = scene
        .answer(&scene.asker_token, &question, "9999", false)
        .await
        .unwrap_err();
    assert_eq!(wrong.code(), Code::PermissionDenied, "{wrong:?}");
    assert!(!asking.is_finished());

    scene.answer(&scene.target_token, &question, "8080", false).await.unwrap();
    let asked = tokio::time::timeout(WAIT, asking).await.unwrap().unwrap().unwrap();
    assert_eq!(asked.outcome(), Outcome::Answered);
    assert_eq!(asked.answer, "8080");

    // Answered once: a second answer finds nothing waiting.
    let again = scene
        .answer(&scene.target_token, &question, "8081", false)
        .await
        .unwrap_err();
    assert_eq!(again.code(), Code::NotFound, "{again:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_declined_question_returns_declined() {
    let scene = Scene::new().await;
    let asking = scene.ask("may I drop the users table", 0).await;
    let question = scene.question_id("may I drop the users table").await;
    scene.answer(&scene.target_token, &question, "", true).await.unwrap();
    let asked = tokio::time::timeout(WAIT, asking).await.unwrap().unwrap().unwrap();
    assert_eq!(asked.outcome(), Outcome::Declined);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_question_nobody_answers_expires_and_cant_be_answered_after() {
    let scene = Scene::new().await;
    let asking = scene.ask("is the build green", 2).await;
    let question = scene.question_id("is the build green").await;
    let asked = tokio::time::timeout(WAIT, asking).await.unwrap().unwrap().unwrap();
    assert_eq!(asked.outcome(), Outcome::Expired);
    let late = scene
        .answer(&scene.target_token, &question, "yes", false)
        .await
        .unwrap_err();
    assert_eq!(late.code(), Code::NotFound, "{late:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_note_to_a_stopped_session_is_refused_and_types_nothing() {
    let scene = Scene::new().await;
    scene
        .sessions()
        .await
        .stop_session(pb::StopSessionRequest {
            idempotency_key: None,
            id: Some(scene.target.clone()),
        })
        .await
        .unwrap();
    let refused = scene
        .sessions()
        .await
        .send_note(note(&scene.target, "are you there"))
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::FailedPrecondition, "{refused:?}");
    let asked = scene
        .sessions()
        .await
        .ask_session(pb::AskSessionRequest {
            to: Some(scene.target.clone()),
            question: "are you there".to_owned(),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert_eq!(asked.code(), Code::FailedPrecondition, "{asked:?}");
    assert!(!scene.typed().await.iter().any(|line| line.contains("are you there")));
}
