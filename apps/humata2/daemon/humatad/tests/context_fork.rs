//! Context-only and before-turn forks (FK-2, FK-3, FK-7, FK-9; D181, D185)
//! through a real `humatad2`. The parent's conversation is written as Claude
//! JSONL by the test, and the world's agent declares that format. Moves up
//! from tests/pending with FORK's context-fork batch.

use humatad_test_common as common;

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
    /// Whether the agent keeps conversations in a folder named for the
    /// session's folder, as Claude does.
    keyed: bool,
}

/// How an agent that keys its conversations by folder names that folder.
fn dashed(folder: &std::path::Path) -> String {
    folder
        .display()
        .to_string()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Makes the world's agent keep each conversation at
/// `<conv>/<session folder, dashed>/<id>.jsonl`, the layout Claude has: a
/// session given a new folder has its conversation at a new path.
fn key_conversations_by_folder(world: &World) {
    let script = world.root.join("agent.sh");
    std::fs::write(
        &script,
        format!(
            r#"CONV_DIR='{}'
mode=$1; id=$2; key=$3
mkdir -p "$CONV_DIR/$key"
trap 'echo "$mode $id $key exited $?" >> "$CONV_DIR.log"' EXIT
echo "$mode $id $key started in $(pwd)" >> "$CONV_DIR.log"
f="$CONV_DIR/$key/$id.jsonl"
case "$mode" in
  --session-id) : > "$f" ;;
esac
# Reads keys itself, as agent CLIs do, so typing never waits out the
# holder's keys limit (see common/mod.rs).
stty -icanon 2>/dev/null
echo "agent up $mode"
while IFS= read -r line; do echo "got: $line"; printf '%s\n' "$line" >> "$f"; done
"#,
            world.conv.display()
        ),
    )
    .unwrap();
    let mut agents: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&world.agents).unwrap()).unwrap();
    agents[0]["start_args"] = serde_json::json!([script, "--session-id", "{conversation}", "{folder_dashed}"]);
    agents[0]["resume_args"] = serde_json::json!([script, "--resume", "{conversation}", "{folder_dashed}"]);
    agents[0]["conversation_file"] =
        format!("{}/{{folder_dashed}}/{{conversation}}.jsonl", world.conv.display()).into();
    std::fs::write(&world.agents, agents.to_string()).unwrap();
}

impl Fixture {
    /// A working session on acme/api whose conversation is [`transcript`].
    /// `format` names the agent's conversation format, or none.
    async fn new(format: Option<&str>) -> Self {
        Self::build(format, false).await
    }

    /// Like [`Fixture::new`], with the agent keeping its conversations by folder.
    async fn keyed() -> Self {
        Self::build(Some("claude-jsonl"), true).await
    }

    async fn build(format: Option<&str>, keyed: bool) -> Self {
        let dir = folder();
        let world = World::new(dir.path());
        if keyed {
            key_conversations_by_folder(&world);
        }
        if let Some(format) = format {
            let mut agents: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&world.agents).unwrap()).unwrap();
            agents[0]["conversation_format"] = format.into();
            agents[0]["version_args"] = serde_json::json!([]);
            std::fs::write(&world.agents, agents.to_string()).unwrap();
        }
        let args = world.args();
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let daemon = Daemon::start_with(common::Build::Admin, dir, &refs).await;
        world.remote("acme", "api");
        let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
        let (id, _) = start_session(&daemon, &channel, "Fix flaky test").await;
        let parent = common::wait_running(&daemon, &id).await;
        let file = Self::file_of(&world, keyed, &parent);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, transcript()).unwrap();
        // Written during turn 3; the code is copied as it is now (D181.2).
        std::fs::write(PathBuf::from(&parent.repos[0].path).join("clock.rs"), "fake\n").unwrap();
        Self {
            daemon,
            world,
            parent,
            keyed,
        }
    }

    /// Where the agent keeps `session`'s conversation.
    fn file_of(world: &World, keyed: bool, session: &pb::Session) -> PathBuf {
        if !keyed {
            return world.conversation(&session.conversation_id);
        }
        let folder = PathBuf::from(&session.repos[0].path);
        world
            .conv
            .join(dashed(folder.parent().unwrap()))
            .join(format!("{}.jsonl", session.conversation_id))
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
            reason: String::new(),
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
        std::fs::read_to_string(Self::file_of(&self.world, self.keyed, session)).unwrap_or_default()
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
            folder: String::new(),
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
            folder: String::new(),
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

/// What a restart after a cut must leave as it was.
struct Before {
    /// The folder the channel's session folders are in.
    sessions_folder: PathBuf,
    folder_names: Vec<String>,
    conversation_names: Vec<String>,
}

impl Before {
    fn of(fixture: &Fixture) -> Self {
        let sessions_folder = PathBuf::from(&fixture.parent.repos[0].path)
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_owned();
        Self {
            folder_names: names(&sessions_folder),
            sessions_folder,
            conversation_names: names(&fixture.world.conv),
        }
    }
}

async fn list_sessions(daemon: &Daemon) -> Vec<pb::Session> {
    daemon
        .session_service()
        .await
        .list_sessions(pb::ListSessionsRequest {
            channel: None,
            machine: None,
            include_archived: true,
            page: None,
        })
        .await
        .unwrap()
        .into_inner()
        .sessions
}

/// `session`'s conversation is what it was, as far as it went: after a
/// restart the resumed agent is told it was interrupted and writes that
/// down after it, so lines may follow.
fn assert_conversation_kept(fixture: &Fixture, session: &pb::Session, was: &str) {
    let now = fixture.conversation_of(session);
    assert!(now.starts_with(was), "{now}");
}

/// The parent keeps its clone and its conversation, as they were.
async fn assert_parent_untouched(fixture: &Fixture) {
    let parent = get_session(&fixture.daemon, &fixture.parent_id()).await;
    assert_eq!(parent.repos, fixture.parent.repos, "the parent keeps its repo");
    let clone = PathBuf::from(&parent.repos[0].path);
    assert_eq!(std::fs::read_to_string(clone.join("clock.rs")).unwrap(), "fake\n");
    assert!(fixture.conversation_of(&parent).contains("Deleted the sleep."));
}

/// Forks the parent with `request`, has the daemon die at the stop point
/// `stop`, and starts the daemon again.
async fn cut_fork_at(fixture: &mut Fixture, stop: &str, request: pb::ForkSessionRequest) {
    fixture.daemon.restart_env(&[("HUMATA2_STOP_AT", stop)]).await;
    last_start(&fixture.daemon).await;
    let _ = fixture
        .daemon
        .session_service()
        .await
        .fork_session(request)
        .await;
    wait_dead(&fixture.daemon).await;
    fixture.daemon.restart().await;
    last_start(&fixture.daemon).await;
}

/// Gives `fork` its own copy with the daemon dying at the stop point
/// `stop`, and starts the daemon again.
async fn cut_own_copy_at(fixture: &mut Fixture, fork: &pb::SessionId, stop: &str) {
    fixture.daemon.restart_env(&[("HUMATA2_STOP_AT", stop)]).await;
    last_start(&fixture.daemon).await;
    common::wait_running(&fixture.daemon, fork).await;
    let _ = fixture
        .daemon
        .session_service()
        .await
        .give_own_copy(pb::GiveOwnCopyRequest {
            idempotency_key: None,
            fork: Some(fork.clone()),
        })
        .await;
    wait_dead(&fixture.daemon).await;
    fixture.daemon.restart().await;
    last_start(&fixture.daemon).await;
}

/// After a restart, a cut context-only fork is gone and its parent's folder
/// is as it was.
async fn assert_fork_undone(fixture: &Fixture, before: &Before) {
    let sessions = list_sessions(&fixture.daemon).await;
    assert_eq!(sessions.len(), 1, "the cut fork is gone: {sessions:?}");
    assert_eq!(sessions[0].id, Some(fixture.parent_id()));
    assert_eq!(
        names(&before.sessions_folder),
        before.folder_names,
        "the fork's folder was removed"
    );
    assert_eq!(
        names(&fixture.world.conv),
        before.conversation_names,
        "the fork's conversation copy was removed"
    );
    let parent = get_session(&fixture.daemon, &fixture.parent_id()).await;
    assert!(parent.forks.is_empty(), "{parent:?}");
    assert_parent_untouched(fixture).await;
}

#[tokio::test]
async fn a_context_only_fork_cut_when_its_records_are_saved_is_undone_at_restart() {
    let mut fixture = Fixture::new(Some("claude-jsonl")).await;
    let before = Before::of(&fixture);
    let request = fixture.request(pb::ForkKind::ContextOnly, 0);
    cut_fork_at(&mut fixture, "fork.write_record", request).await;
    assert_fork_undone(&fixture, &before).await;
}

#[tokio::test]
async fn a_context_only_fork_cut_after_its_conversation_is_copied_is_undone_at_restart() {
    let mut fixture = Fixture::new(Some("claude-jsonl")).await;
    let before = Before::of(&fixture);
    let request = fixture.request(pb::ForkKind::ContextOnly, 0);
    cut_fork_at(&mut fixture, "fork.copy_conversation", request).await;
    assert_fork_undone(&fixture, &before).await;
}

/// A context-only fork cut at its own-copy `stop` still works in its
/// parent's code, with its conversation as it was and no copy left behind.
#[tokio::test]
async fn an_own_copy_cut_when_its_conversation_is_moved_stays_on_the_parents_code() {
    let mut fixture = Fixture::new(Some("claude-jsonl")).await;
    let request = fixture.request(pb::ForkKind::ContextOnly, 0);
    let fork = fixture.fork(request).await.unwrap();
    let fork_id = fork.id.clone().unwrap();
    let before = Before::of(&fixture);
    let conversation = fixture.conversation_of(&fork);

    cut_own_copy_at(&mut fixture, &fork_id, "own_copy.move_conversation").await;

    let after = common::wait_running(&fixture.daemon, &fork_id).await;
    assert_eq!(after.repos, fork.repos, "{after:?}");
    assert_eq!(
        names(&before.sessions_folder),
        before.folder_names,
        "the half-made copy was cleared"
    );
    assert_eq!(names(&fixture.world.conv), before.conversation_names);
    assert_conversation_kept(&fixture, &after, &conversation);
    assert_parent_untouched(&fixture).await;
}

/// Waits for the keyed agent to run; if it doesn't in time, says what it did
/// (its script logs each start and exit beside the conversations).
async fn running_or_explain(fixture: &Fixture, id: &pb::SessionId) -> pb::Session {
    let wait = common::wait_running(&fixture.daemon, id);
    match tokio::time::timeout(std::time::Duration::from_secs(40), wait).await {
        Ok(session) => session,
        Err(_) => {
            let log = std::fs::read_to_string(format!("{}.log", fixture.world.conv.display()))
                .unwrap_or_default();
            let session = get_session(&fixture.daemon, id).await;
            panic!(
                "{} never ran: {:?}, exit code {}, resume failure {:?}\nthe agent's starts and exits:\n{log}",
                id.value,
                session.status(),
                session.exit_code,
                session.resume_failed
            );
        }
    }
}

/// With an agent that keys conversations by folder, a cut own copy has
/// already copied the conversation to the new folder's path: the restart
/// removes that copy and any folder, and the fork goes on from its parent's.
async fn assert_cut_own_copy_leaves_nothing(stop: &str) {
    let mut fixture = Fixture::keyed().await;
    let request = fixture.request(pb::ForkKind::ContextOnly, 0);
    let fork = fixture.fork(request).await.unwrap();
    let fork_id = fork.id.clone().unwrap();
    let before = Before::of(&fixture);
    let conversation = fixture.conversation_of(&fork);
    assert!(conversation.contains("Deleted the sleep."), "{conversation}");

    cut_own_copy_at(&mut fixture, &fork_id, stop).await;

    let after = running_or_explain(&fixture, &fork_id).await;
    assert_eq!(after.repos, fork.repos, "{after:?}");
    assert_eq!(
        names(&before.sessions_folder),
        before.folder_names,
        "no folder of the half-made copy is left"
    );
    assert_eq!(
        names(&fixture.world.conv),
        before.conversation_names,
        "the copy under the new folder's name was removed"
    );
    assert_conversation_kept(&fixture, &after, &conversation);
    assert_parent_untouched(&fixture).await;
}

#[tokio::test]
async fn an_own_copy_cut_when_its_conversation_is_moved_leaves_no_copy_under_the_new_folder() {
    assert_cut_own_copy_leaves_nothing("own_copy.move_conversation").await;
}

#[tokio::test]
async fn an_own_copy_cut_once_its_folder_is_in_place_but_unrecorded_is_taken_back_at_restart() {
    assert_cut_own_copy_leaves_nothing("own_copy.renamed").await;
}

/// The fork works in its own copy of the code, on its own branch, with its
/// conversation as it was; no half-made copy is left beside the folder.
async fn assert_own_copy_kept(fixture: &Fixture, fork_id: &pb::SessionId, conversation: &str) {
    let after = common::wait_running(&fixture.daemon, fork_id).await;
    let own = PathBuf::from(&after.repos[0].path);
    let parent_clone = PathBuf::from(&fixture.parent.repos[0].path);
    assert_ne!(own, parent_clone, "{after:?}");
    assert_eq!(std::fs::read_to_string(own.join("clock.rs")).unwrap(), "fake\n");
    assert_eq!(
        git(&["-C", &own.display().to_string(), "branch", "--show-current"]),
        after.repos[0].branch
    );
    let sessions = parent_clone.parent().unwrap().parent().unwrap().to_owned();
    assert_eq!(own.parent().unwrap().parent(), Some(sessions.as_path()));
    let listed = names(&sessions);
    assert!(listed.iter().all(|name| !name.ends_with(".partial")), "{listed:?}");
    assert_conversation_kept(fixture, &after, &conversation);
    assert_parent_untouched(fixture).await;
}

#[tokio::test]
async fn an_own_copy_cut_once_moved_into_place_keeps_the_copy_after_restart() {
    let mut fixture = Fixture::new(Some("claude-jsonl")).await;
    let request = fixture.request(pb::ForkKind::ContextOnly, 0);
    let fork = fixture.fork(request).await.unwrap();
    let fork_id = fork.id.clone().unwrap();
    let conversation = fixture.conversation_of(&fork);

    cut_own_copy_at(&mut fixture, &fork_id, "own_copy.launch").await;

    assert_own_copy_kept(&fixture, &fork_id, &conversation).await;
}

#[tokio::test]
async fn an_own_copy_cut_after_its_agent_starts_again_keeps_the_copy_after_restart() {
    let mut fixture = Fixture::new(Some("claude-jsonl")).await;
    let request = fixture.request(pb::ForkKind::ContextOnly, 0);
    let fork = fixture.fork(request).await.unwrap();
    let fork_id = fork.id.clone().unwrap();
    let conversation = fixture.conversation_of(&fork);

    cut_own_copy_at(&mut fixture, &fork_id, "own_copy.verify").await;

    assert_own_copy_kept(&fixture, &fork_id, &conversation).await;
}
