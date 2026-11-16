//! Context-only and before-turn forks (FK-2, FK-3, FK-7, FK-9; D181, D185)
//! through a real `humatad2`. The parent's conversation is written as Claude
//! JSONL by the test, and the world's agent declares that format. Moves up
//! from tests/pending with FORK's context-fork batch.

mod common;

use std::path::PathBuf;

use common::{
    Attach, Daemon, World, agent_tab, create_channel, finished, folder, get_session, git,
    last_start, open, start_session, wait_status,
};
use humata2_grpc::humata::v1 as pb;
use pb::SessionStatus as Status;

/// Three prompts, each with an answer; a tool call and its result, a meta
/// line and a sidechain line add no turns.
fn transcript() -> String {
    let lines = [
        r#"{"type":"user","message":{"role":"user","content":"Find the flaky test"},"timestamp":"2026-11-15T09:00:00Z"}"#,
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"It is retry_test."}]},"timestamp":"2026-11-15T09:00:05Z"}"#,
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"a.rs"}}]}}"#,
        r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"fn a() {}"}]}}"#,
        r#"{"type":"user","isMeta":true,"message":{"role":"user","content":"<command-name>/clear</command-name>"}}"#,
        r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"Make it deterministic\nwith a fake clock"}]},"timestamp":"2026-11-15T09:01:00Z"}"#,
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Done with a fake clock."}]}}"#,
        r#"{"type":"user","isSidechain":true,"message":{"role":"user","content":"side question"}}"#,
        r#"{"type":"user","message":{"role":"user","content":"Now delete the old sleep"},"timestamp":"2026-11-15T09:02:00Z"}"#,
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Deleted the sleep."}]}}"#,
    ];
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

struct Fixture {
    daemon: Daemon,
    world: World,
    parent: pb::Session,
}

impl Fixture {
    /// A working session on acme/api whose conversation is [`transcript`].
    /// `format` names the agent's conversation format, or none.
    async fn new(format: Option<&str>) -> Self {
        let dir = folder();
        let world = World::new(dir.path());
        if let Some(format) = format {
            let mut agents: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&world.agents).unwrap()).unwrap();
            agents[0]["conversation_format"] = format.into();
            std::fs::write(&world.agents, agents.to_string()).unwrap();
        }
        let args = world.args();
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let daemon = Daemon::start_with(common::Build::Admin, dir, &refs).await;
        world.remote("acme", "api");
        let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
        let (id, _) = start_session(&daemon, &channel, "Fix flaky test").await;
        let parent = common::wait_running(&daemon, &id).await;
        std::fs::write(world.conversation(&parent.conversation_id), transcript()).unwrap();
        // Written during turn 3; the code is copied as it is now (D181.2).
        std::fs::write(PathBuf::from(&parent.repos[0].path).join("clock.rs"), "fake\n").unwrap();
        Self {
            daemon,
            world,
            parent,
        }
    }

    fn parent_id(&self) -> pb::SessionId {
        self.parent.id.clone().unwrap()
    }

    fn request(&self, kind: pb::ForkKind, before_turn: u32) -> pb::ForkSessionRequest {
        pb::ForkSessionRequest {
            idempotency_key: None,
            parent: Some(self.parent_id()),
            kind: kind as i32,
            before_turn,
            title: "Side look".to_owned(),
            agent: None,
            model: String::new(),
            machine: None,
            branch: String::new(),
        }
    }

    async fn turns(&self) -> Vec<pb::Turn> {
        self.daemon
            .session_service()
            .await
            .list_turns(pb::ListTurnsRequest {
                session: Some(self.parent_id()),
                page: None,
            })
            .await
            .unwrap()
            .into_inner()
            .turns
    }

    /// Forks and waits; the fork's session once it is working.
    async fn fork(&self, request: pb::ForkSessionRequest) -> Result<pb::Session, tonic::Status> {
        let started = self
            .daemon
            .session_service()
            .await
            .fork_session(request)
            .await?
            .into_inner();
        let operation = finished(&self.daemon, &started).await;
        assert_eq!(
            operation.state,
            pb::OperationState::Succeeded as i32,
            "{operation:?}"
        );
        let Some(pb::operation::Result::Session(id)) = &operation.result else {
            panic!("the fork names no session: {operation:?}");
        };
        Ok(common::wait_running(&self.daemon, id).await)
    }

    fn conversation_of(&self, session: &pb::Session) -> String {
        std::fs::read_to_string(self.world.conversation(&session.conversation_id))
            .unwrap_or_default()
    }

    async fn give_own_copy(&self, fork: &pb::SessionId) -> pb::Operation {
        let started = self
            .daemon
            .session_service()
            .await
            .give_own_copy(pb::GiveOwnCopyRequest {
                idempotency_key: None,
                fork: Some(fork.clone()),
            })
            .await
            .unwrap()
            .into_inner();
        finished(&self.daemon, &started).await
    }
}

#[tokio::test]
async fn the_turns_are_the_prompts_of_the_conversation() {
    let fixture = Fixture::new(Some("claude-jsonl")).await;
    let turns = fixture.turns().await;
    let listed: Vec<(u32, &str)> = turns
        .iter()
        .map(|turn| (turn.index, turn.user_text.as_str()))
        .collect();
    assert_eq!(
        listed,
        vec![
            (1, "Find the flaky test"),
            (2, "Make it deterministic"),
            (3, "Now delete the old sleep"),
        ]
    );
    assert!(turns[0].at.is_some());
}

#[tokio::test]
async fn a_fork_before_a_turn_carries_the_conversation_up_to_it_and_the_code_as_now() {
    let fixture = Fixture::new(Some("claude-jsonl")).await;
    let fork = fixture
        .fork(fixture.request(pb::ForkKind::CodeAndContext, 3))
        .await
        .unwrap();

    let carried = fixture.conversation_of(&fork);
    assert!(carried.contains("Find the flaky test"), "{carried}");
    assert!(carried.contains("Done with a fake clock."), "{carried}");
    assert!(!carried.contains("Now delete the old sleep"), "{carried}");
    assert!(!carried.contains("Deleted the sleep."), "{carried}");
    assert!(fixture.conversation_of(&fixture.parent).contains("Deleted the sleep."));
    let clone = PathBuf::from(&fork.repos[0].path);
    assert_eq!(std::fs::read_to_string(clone.join("clock.rs")).unwrap(), "fake\n");
    assert_eq!(fork.fork_origin.as_ref().map(|o| o.before_turn), Some(3));
}

#[tokio::test]
async fn a_fork_before_a_turn_that_is_not_there_is_refused() {
    let fixture = Fixture::new(Some("claude-jsonl")).await;
    for before_turn in [1, 4, 9] {
        let status = fixture
            .fork(fixture.request(pb::ForkKind::CodeAndContext, before_turn))
            .await
            .unwrap_err();
        assert_eq!(status.code(), tonic::Code::InvalidArgument, "{before_turn}: {status:?}");
    }
}

#[tokio::test]
async fn an_agent_without_a_conversation_format_has_no_turns_to_fork_at() {
    let fixture = Fixture::new(None).await;
    assert!(fixture.turns().await.is_empty());
    let status = fixture
        .fork(fixture.request(pb::ForkKind::CodeAndContext, 2))
        .await
        .unwrap_err();
    assert_eq!(status.code(), tonic::Code::FailedPrecondition, "{status:?}");
    // From now still works.
    fixture
        .fork(fixture.request(pb::ForkKind::CodeAndContext, 0))
        .await
        .unwrap();
}

#[tokio::test]
async fn a_context_only_fork_carries_the_conversation_and_works_in_the_parents_code() {
    let fixture = Fixture::new(Some("claude-jsonl")).await;
    let parent_clone = PathBuf::from(&fixture.parent.repos[0].path);

    let fork = fixture
        .fork(fixture.request(pb::ForkKind::ContextOnly, 0))
        .await
        .unwrap();

    assert_eq!(
        fork.fork_origin.as_ref().map(|o| o.kind),
        Some(pb::ForkKind::ContextOnly as i32)
    );
    assert_ne!(fork.conversation_id, fixture.parent.conversation_id);
    assert!(fixture.conversation_of(&fork).contains("Deleted the sleep."));
    // It shares the parent's clone, on the parent's branch (D185).
    assert_eq!(fork.repos.len(), 1, "{fork:?}");
    assert_eq!(PathBuf::from(&fork.repos[0].path), parent_clone);
    assert_eq!(fork.repos[0].branch, fixture.parent.repos[0].branch);

    // A shell in a clone it doesn't have is refused.
    let unknown = fixture
        .daemon
        .terminals()
        .await
        .open_shell_tab(pb::OpenShellTabRequest {
            idempotency_key: None,
            session: fork.id.clone(),
            repo: Some(common::repo_ref("acme", "nope")),
        })
        .await
        .unwrap_err();
    assert_eq!(unknown.code(), tonic::Code::InvalidArgument);

    // A shell in the fork's api clone edits the parent's file in place.
    let tab = fixture
        .daemon
        .terminals()
        .await
        .open_shell_tab(pb::OpenShellTabRequest {
            idempotency_key: None,
            session: fork.id.clone(),
            repo: Some(common::repo_ref("acme", "api")),
        })
        .await
        .unwrap()
        .into_inner()
        .id
        .unwrap();
    let mut shell = Attach::start(&fixture.daemon, open(&tab, 0)).await;
    shell
        .type_text("printf 'from the fork\\n' >> clock.rs && echo edit-$((40+2))-done\r")
        .await;
    shell.wait_for("edit-42-done").await;
    assert_eq!(
        std::fs::read_to_string(parent_clone.join("clock.rs")).unwrap(),
        "fake\nfrom the fork\n"
    );
    // The parent keeps its clone.
    let parent = get_session(&fixture.daemon, &fixture.parent_id()).await;
    assert_eq!(parent.repos, fixture.parent.repos);
}

#[tokio::test]
async fn giving_a_context_only_fork_its_own_copy_moves_it_onto_its_own_clones() {
    let fixture = Fixture::new(Some("claude-jsonl")).await;
    let fork = fixture
        .fork(fixture.request(pb::ForkKind::ContextOnly, 0))
        .await
        .unwrap();
    let fork_id = fork.id.clone().unwrap();
    let conversation = fixture.conversation_of(&fork);

    let operation = fixture.give_own_copy(&fork_id).await;

    assert_eq!(
        operation.state,
        pb::OperationState::Succeeded as i32,
        "{operation:?}"
    );
    assert_eq!(operation.kind, pb::OperationKind::GiveOwnCopy as i32);
    let after = get_session(&fixture.daemon, &fork_id).await;
    assert_eq!(after.repos.len(), 1, "{after:?}");
    let own = PathBuf::from(&after.repos[0].path);
    let parent_clone = PathBuf::from(&fixture.parent.repos[0].path);
    assert_ne!(own, parent_clone);
    assert_ne!(after.repos[0].branch, fixture.parent.repos[0].branch);
    assert_eq!(
        git(&["-C", &own.display().to_string(), "branch", "--show-current"]),
        after.repos[0].branch
    );
    assert_eq!(std::fs::read_to_string(own.join("clock.rs")).unwrap(), "fake\n");
    // The agent goes on in its own clones, with its conversation.
    let after = common::wait_running(&fixture.daemon, &fork_id).await;
    // It was moved, not interrupted: it isn't told to check its work.
    assert_eq!(fixture.conversation_of(&after), conversation);
    assert_eq!(after.interrupted_at, None);
    std::fs::write(own.join("mine.txt"), "fork only\n").unwrap();
    assert!(!parent_clone.join("mine.txt").exists());
    assert!(agent_tab(&fixture.daemon, &fork_id).await.value.len() > 0);
    // A second copy is refused: it has its own already.
    let again = fixture
        .daemon
        .session_service()
        .await
        .give_own_copy(pb::GiveOwnCopyRequest {
            idempotency_key: None,
            fork: Some(fork_id),
        })
        .await;
    assert!(
        again.as_ref().is_err_and(|s| s.code() == tonic::Code::FailedPrecondition),
        "{again:?}"
    );
}

/// The names in `dir`, sorted.
fn names(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

async fn wait_dead(daemon: &Daemon) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while std::os::unix::net::UnixStream::connect(&daemon.socket).is_ok() {
        assert!(
            std::time::Instant::now() < deadline,
            "the daemon never stopped at its stop point"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn an_own_copy_cut_short_leaves_no_folder_after_restart() {
    let mut fixture = Fixture::new(Some("claude-jsonl")).await;
    let fork = fixture
        .fork(fixture.request(pb::ForkKind::ContextOnly, 0))
        .await
        .unwrap();
    let fork_id = fork.id.clone().unwrap();
    let sessions = PathBuf::from(&fixture.parent.repos[0].path)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_owned();
    let before = names(&sessions);
    fixture
        .daemon
        .restart_env(&[("HUMATA2_STOP_AT", "own_copy.copy_code:api")])
        .await;
    last_start(&fixture.daemon).await;
    common::wait_running(&fixture.daemon, &fork_id).await;

    let _ = fixture
        .daemon
        .session_service()
        .await
        .give_own_copy(pb::GiveOwnCopyRequest {
            idempotency_key: None,
            fork: Some(fork_id.clone()),
        })
        .await;
    wait_dead(&fixture.daemon).await;
    fixture.daemon.restart().await;
    last_start(&fixture.daemon).await;

    assert_eq!(names(&sessions), before, "the half-made copy was cleared");
    let after = common::wait_running(&fixture.daemon, &fork_id).await;
    assert_eq!(after.repos, fork.repos, "it still works in its parent's code");
}

#[tokio::test]
async fn an_own_copy_that_fails_after_stopping_the_agent_never_leaves_it_said_working() {
    let fixture = Fixture::new(Some("claude-jsonl")).await;
    let fork = fixture
        .fork(fixture.request(pb::ForkKind::ContextOnly, 0))
        .await
        .unwrap();
    let fork_id = fork.id.clone().unwrap();
    std::fs::remove_file(fixture.world.conversation(&fork.conversation_id)).unwrap();

    let operation = fixture.give_own_copy(&fork_id).await;

    assert_eq!(operation.state, pb::OperationState::Failed as i32, "{operation:?}");
    // Its conversation is gone, so it can't start again anywhere: it says
    // so rather than say it works.
    wait_status(&fixture.daemon, &fork_id, Status::ResumeFailed).await;
}
