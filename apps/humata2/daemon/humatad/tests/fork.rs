//! Forking a session on this machine (FK-1..FK-5, FK-13, PER-27) through a
//! real `humatad2`. Needs FORK's seams (store lifecycle tables, the fork
//! handlers); moves up from tests/pending with them.

use humatad_test_common as common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::{
    Attach, Daemon, World, agent_tab, create_channel, finished, get_session, git, last_start,
    open, start_session, start_world,
};
use humata2_grpc::humata::v1 as pb;

struct Fixture {
    daemon: Daemon,
    world: World,
    parent: pb::Session,
}

impl Fixture {
    /// A working session "Fix login" on acme/api with a local commit, an
    /// uncommitted change and an untracked file, and one line in its
    /// conversation.
    async fn new() -> Self {
        let (daemon, world) = start_world().await;
        Self::in_world(daemon, world).await
    }

    /// As [`Fixture::new`], with a second agent, [`OTHER`], beside the test
    /// one, and the parent's conversation line written as Claude's JSONL so
    /// it can be summarized.
    async fn with_other_agent() -> Self {
        Self::with_other_agent_and(&[]).await
    }

    /// [`Fixture::with_other_agent`] with environment variables for the
    /// daemon's start.
    async fn with_other_agent_and(env: &[(&str, &str)]) -> Self {
        let dir = common::folder();
        let world = World::new(dir.path());
        let mut agents: Vec<serde_json::Value> =
            serde_json::from_slice(&std::fs::read(&world.agents).unwrap()).unwrap();
        let mut other = agents[0].clone();
        other["name"] = serde_json::Value::from(OTHER);
        agents.push(other);
        std::fs::write(&world.agents, serde_json::Value::from(agents).to_string()).unwrap();
        let args = world.args();
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let daemon = Daemon::start_with_env(common::Build::Admin, dir, &refs, env).await;
        Self::in_world(daemon, world).await
    }

    async fn in_world(daemon: Daemon, world: World) -> Self {
        world.remote("acme", "api");
        let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
        let (id, _) = start_session(&daemon, &channel, "Fix login").await;
        let parent = common::wait_running(&daemon, &id).await;
        let fixture = Self {
            daemon,
            world,
            parent,
        };
        let clone = fixture.clone_of(&fixture.parent);
        let text = clone.display().to_string();
        std::fs::write(clone.join("login.rs"), "fn login() {}\n").unwrap();
        git(&["-C", &text, "add", "login.rs"]);
        git(&["-C", &text, "commit", "-m", "login"]);
        std::fs::write(clone.join("README.md"), "acme/api\nchanged\n").unwrap();
        std::fs::write(clone.join("notes.txt"), "untracked\n").unwrap();
        say(&fixture.daemon, &fixture.parent_id(), &said_as_jsonl("remember the retry")).await;
        fixture
    }

    fn parent_id(&self) -> pb::SessionId {
        self.parent.id.clone().unwrap()
    }

    fn clone_of(&self, session: &pb::Session) -> PathBuf {
        PathBuf::from(&session.repos[0].path)
    }

    fn request(&self, kind: pb::ForkKind, title: &str) -> pb::ForkSessionRequest {
        pb::ForkSessionRequest {
            idempotency_key: None,
            parent: Some(self.parent_id()),
            kind: kind as i32,
            before_turn: 0,
            title: title.to_owned(),
            agent: None,
            model: String::new(),
            machine: None,
            branch: String::new(),
            reason: String::new(),
        }
    }

    /// Forks the parent and waits for the fork's operation.
    async fn fork(&self, kind: pb::ForkKind, title: &str) -> (pb::Operation, Option<pb::Session>) {
        fork_of(&self.daemon, self.request(kind, title)).await
    }

    async fn sessions(&self) -> Vec<pb::Session> {
        list(&self.daemon).await
    }
}

async fn fork_of(
    daemon: &Daemon,
    request: pb::ForkSessionRequest,
) -> (pb::Operation, Option<pb::Session>) {
    let started = daemon
        .session_service()
        .await
        .fork_session(request)
        .await
        .unwrap()
        .into_inner();
    let operation = finished(daemon, &started).await;
    let session = match &operation.result {
        Some(pb::operation::Result::Session(id)) => Some(get_session(daemon, id).await),
        _ => None,
    };
    (operation, session)
}

async fn list(daemon: &Daemon) -> Vec<pb::Session> {
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

async fn say(daemon: &Daemon, id: &pb::SessionId, line: &str) {
    let tab = agent_tab(daemon, id).await;
    let mut attach = Attach::start(daemon, open(&tab, 0)).await;
    attach.type_text(&format!("{line}\r")).await;
    attach.wait_for(&format!("got: {line}")).await;
}

fn conversation(world: &World, session: &pb::Session) -> String {
    std::fs::read_to_string(world.conversation(&session.conversation_id)).unwrap_or_default()
}

fn branch_of(clone: &Path) -> String {
    git(&["-C", &clone.display().to_string(), "branch", "--show-current"])
}

/// The second agent a [`Fixture::with_other_agent`] daemon knows.
const OTHER: &str = "other";

/// `line` as a user message in Claude's JSONL, as the test agent writes
/// every line it reads to its conversation.
fn said_as_jsonl(line: &str) -> String {
    serde_json::json!({"type": "user", "message": {"role": "user", "content": line}}).to_string()
}

/// The file a fork that runs another agent is handed the conversation in.
fn handover_file(folder: &Path, fork: &pb::Session) -> PathBuf {
    folder
        .join(".humata")
        .join(format!("handover-{}.jsonl", fork.id.as_ref().unwrap().value))
}

/// Waits until the fork's agent has read the first line of its handover
/// message and the summary line that names what was asked.
async fn wait_handed_over(daemon: &Daemon, fork: &pb::SessionId) -> Attach {
    let tab = agent_tab(daemon, fork).await;
    let mut attach = Attach::start(daemon, open(&tab, 0)).await;
    attach.wait_for("got: [handover").await;
    attach.wait_for("remember the retry").await;
    attach
}

/// Waits until the daemon on `daemon.socket` stops answering: a stop
/// point killed it.
async fn wait_dead(daemon: &Daemon) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while std::os::unix::net::UnixStream::connect(&daemon.socket).is_ok() {
        assert!(Instant::now() < deadline, "the daemon never stopped at its stop point");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn the_preview_names_the_forks_branch_and_what_it_copies() {
    let fixture = Fixture::new().await;
    let preview = fixture
        .daemon
        .session_service()
        .await
        .preview_fork(fixture.request(pb::ForkKind::CodeAndContext, "Try a retry"))
        .await
        .unwrap()
        .into_inner();
    assert!(!preview.text.is_empty());
    assert!(preview.copy_bytes > 0);
    assert_eq!(preview.branches.len(), 1);
    let parent_branch = &fixture.parent.repos[0].branch;
    assert!(preview.branches.iter().all(|b| b.branch != *parent_branch));
    assert!(preview.branches.iter().all(|b| b.branch.contains("try-a-retry")));
    assert!(!preview.seeded_with_summary);
    // A preview makes nothing.
    assert_eq!(fixture.sessions().await.len(), 1);
}

#[tokio::test]
async fn code_and_context_copies_the_work_and_the_conversation() {
    let fixture = Fixture::new().await;
    let (operation, fork) = fixture.fork(pb::ForkKind::CodeAndContext, "Try a retry").await;
    assert_eq!(operation.state, pb::OperationState::Succeeded as i32, "{operation:?}");
    assert_eq!(operation.kind, pb::OperationKind::ForkSession as i32);
    let fork = fork.unwrap();
    let fork = common::wait_running(&fixture.daemon, fork.id.as_ref().unwrap()).await;

    let origin = fork.fork_origin.clone().unwrap();
    assert_eq!(origin.parent, Some(fixture.parent_id()));
    assert_eq!(origin.kind, pb::ForkKind::CodeAndContext as i32);
    assert_eq!(fork.channel, fixture.parent.channel);
    assert_eq!(fork.agent, fixture.parent.agent);
    let parent = get_session(&fixture.daemon, &fixture.parent_id()).await;
    assert_eq!(parent.forks, vec![fork.id.clone().unwrap()]);

    // The code: its own folder and branch, with the parent's commit,
    // uncommitted change and untracked file.
    let (from, to) = (fixture.clone_of(&parent), fixture.clone_of(&fork));
    assert_ne!(from, to);
    assert_ne!(fork.repos[0].branch, parent.repos[0].branch);
    assert_eq!(branch_of(&to), fork.repos[0].branch);
    assert_eq!(branch_of(&from), parent.repos[0].branch);
    assert_eq!(std::fs::read_to_string(to.join("login.rs")).unwrap(), "fn login() {}\n");
    assert_eq!(std::fs::read_to_string(to.join("README.md")).unwrap(), "acme/api\nchanged\n");
    assert_eq!(std::fs::read_to_string(to.join("notes.txt")).unwrap(), "untracked\n");

    // The conversation: a copy under its own id, carried on by the fork.
    assert_ne!(fork.conversation_id, parent.conversation_id);
    assert!(conversation(&fixture.world, &fork).contains("remember the retry"));
    say(&fixture.daemon, fork.id.as_ref().unwrap(), "only in the fork").await;
    assert!(conversation(&fixture.world, &fork).contains("only in the fork"));
    assert!(!conversation(&fixture.world, &parent).contains("only in the fork"));

    // Writing in the fork leaves the parent's clone alone.
    std::fs::write(to.join("fork.txt"), "x").unwrap();
    assert!(!from.join("fork.txt").exists());
}

#[tokio::test]
async fn code_only_copies_the_work_into_a_fresh_conversation() {
    let fixture = Fixture::new().await;
    let (operation, fork) = fixture.fork(pb::ForkKind::CodeOnly, "Clean slate").await;
    assert_eq!(operation.state, pb::OperationState::Succeeded as i32, "{operation:?}");
    let fork = fork.unwrap();
    let fork = common::wait_running(&fixture.daemon, fork.id.as_ref().unwrap()).await;
    assert_eq!(fork.fork_origin.as_ref().unwrap().kind, pb::ForkKind::CodeOnly as i32);
    let to = fixture.clone_of(&fork);
    assert_eq!(std::fs::read_to_string(to.join("notes.txt")).unwrap(), "untracked\n");
    assert!(!conversation(&fixture.world, &fork).contains("remember the retry"));
}

#[tokio::test]
async fn forks_nest_and_each_names_its_parent() {
    let fixture = Fixture::new().await;
    let (_, child) = fixture.fork(pb::ForkKind::CodeOnly, "Child").await;
    let child = child.unwrap();
    let mut request = fixture.request(pb::ForkKind::CodeOnly, "Grandchild");
    request.parent = child.id.clone();
    let (operation, grandchild) = fork_of(&fixture.daemon, request).await;
    assert_eq!(operation.state, pb::OperationState::Succeeded as i32, "{operation:?}");
    let grandchild = grandchild.unwrap();
    assert_eq!(grandchild.fork_origin.unwrap().parent, child.id);
    let child = get_session(&fixture.daemon, child.id.as_ref().unwrap()).await;
    assert_eq!(child.forks, vec![grandchild.id.clone().unwrap()]);
    // Three distinct branches.
    let mut branches: Vec<String> = fixture
        .sessions()
        .await
        .into_iter()
        .map(|s| s.repos[0].branch.clone())
        .collect();
    branches.sort();
    branches.dedup();
    assert_eq!(branches.len(), 3);
}

#[tokio::test]
async fn forking_a_session_that_does_not_exist_names_it() {
    let fixture = Fixture::new().await;
    let mut request = fixture.request(pb::ForkKind::CodeOnly, "Nowhere");
    request.parent = Some(pb::SessionId {
        value: "se_missing".to_owned(),
    });
    let status = fixture
        .daemon
        .session_service()
        .await
        .fork_session(request)
        .await
        .unwrap_err();
    assert_eq!(status.code(), tonic::Code::NotFound);
    assert_eq!(common::not_found(&status), pb::EntityKind::Session);
}

#[tokio::test]
async fn a_fork_cut_before_its_agent_starts_is_undone_at_restart() {
    let mut fixture = Fixture::new().await;
    fixture
        .daemon
        .restart_env(&[("HUMATA2_STOP_AT", "fork.copy_code:api")])
        .await;
    last_start(&fixture.daemon).await;
    let channel_sessions = fixture.clone_of(&fixture.parent);
    let sessions_folder = channel_sessions.parent().unwrap().parent().unwrap().to_owned();
    let before: Vec<_> = std::fs::read_dir(&sessions_folder).unwrap().collect();
    let _ = fixture
        .daemon
        .session_service()
        .await
        .fork_session(fixture.request(pb::ForkKind::CodeAndContext, "Cut short"))
        .await;
    wait_dead(&fixture.daemon).await;
    fixture.daemon.restart().await;
    last_start(&fixture.daemon).await;

    let sessions = fixture.sessions().await;
    assert_eq!(sessions.len(), 1, "the cut fork is gone: {sessions:?}");
    assert_eq!(sessions[0].id, Some(fixture.parent_id()));
    assert!(sessions[0].forks.is_empty());
    let after: Vec<_> = std::fs::read_dir(&sessions_folder).unwrap().collect();
    assert_eq!(after.len(), before.len(), "the fork's folder was removed");
    // The parent is untouched and still has its work.
    let parent = common::wait_running(&fixture.daemon, &fixture.parent_id()).await;
    assert!(fixture.clone_of(&parent).join("notes.txt").exists());
}

#[tokio::test]
async fn a_fork_cut_after_its_agent_started_is_kept_at_restart() {
    let mut fixture = Fixture::new().await;
    fixture
        .daemon
        .restart_env(&[("HUMATA2_STOP_AT", "fork.verify")])
        .await;
    last_start(&fixture.daemon).await;
    let _ = fixture
        .daemon
        .session_service()
        .await
        .fork_session(fixture.request(pb::ForkKind::CodeAndContext, "Past the point"))
        .await;
    wait_dead(&fixture.daemon).await;
    fixture.daemon.restart().await;
    last_start(&fixture.daemon).await;

    let fork = fixture
        .sessions()
        .await
        .into_iter()
        .find(|s| s.id != Some(fixture.parent_id()))
        .expect("the fork past its launch is kept");
    let fork = common::wait_running(&fixture.daemon, fork.id.as_ref().unwrap()).await;
    assert_eq!(fork.fork_origin.clone().unwrap().parent, Some(fixture.parent_id()));
    assert!(conversation(&fixture.world, &fork).contains("remember the retry"));
}

#[tokio::test]
async fn a_fork_with_another_agent_is_handed_the_conversation_and_a_summary() {
    let fixture = Fixture::with_other_agent().await;
    let mut request = fixture.request(pb::ForkKind::CodeAndContext, "Ask the other one");
    request.agent = Some(pb::AgentId { value: OTHER.to_owned() });
    let preview = fixture
        .daemon
        .session_service()
        .await
        .preview_fork(request.clone())
        .await
        .unwrap()
        .into_inner();
    assert!(preview.seeded_with_summary);

    let (operation, fork) = fork_of(&fixture.daemon, request).await;
    assert_eq!(operation.state, pb::OperationState::Succeeded as i32, "{operation:?}");
    let fork = fork.unwrap();
    assert_eq!(fork.agent.as_ref().map(|agent| agent.value.as_str()), Some(OTHER));
    let fork = common::wait_running(&fixture.daemon, fork.id.as_ref().unwrap()).await;
    wait_handed_over(&fixture.daemon, fork.id.as_ref().unwrap()).await;

    // The code is copied as for any fork; the conversation is handed over
    // as a file beside the clones, not resumed.
    let to = fixture.clone_of(&fork);
    assert_eq!(std::fs::read_to_string(to.join("notes.txt")).unwrap(), "untracked\n");
    let handover = std::fs::read_to_string(handover_file(to.parent().unwrap(), &fork)).unwrap();
    assert!(handover.contains("remember the retry"));
    let own = conversation(&fixture.world, &fork);
    assert_eq!(own.matches("[handover").count(), 1, "{own}");
    assert!(own.contains(".humata/handover-"), "the message names the file: {own}");
}

/// A line a Codex rollout holds once a prompt is done.
fn said_as_rollout(line: &str) -> String {
    serde_json::json!({"type": "event_msg", "payload": {"type": "item_completed", "turn_id": "t1",
        "item": {"type": "UserMessage", "content": [{"type": "text", "text": line}]}}})
    .to_string()
}

/// Agents-10 step 12: a parent whose conversation holds Codex rollout lines
/// hands its fork a summary read from them, not its terminal's output.
#[tokio::test]
async fn a_fork_of_a_codex_parent_is_handed_a_summary_read_from_its_rollout() {
    let fixture = Fixture::with_other_agent().await;
    say(&fixture.daemon, &fixture.parent_id(), &said_as_rollout("split the retry loop")).await;
    let mut request = fixture.request(pb::ForkKind::CodeAndContext, "Ask the other one");
    request.agent = Some(pb::AgentId { value: OTHER.to_owned() });
    let (operation, fork) = fork_of(&fixture.daemon, request).await;
    assert_eq!(operation.state, pb::OperationState::Succeeded as i32, "{operation:?}");
    let fork = common::wait_running(&fixture.daemon, fork.unwrap().id.as_ref().unwrap()).await;
    let mut attach = wait_handed_over(&fixture.daemon, fork.id.as_ref().unwrap()).await;
    attach.wait_for("split the retry loop").await;
    let own = conversation(&fixture.world, &fork);
    assert!(
        !own.contains("item_completed"),
        "the summary holds the rollout's prompts, not the terminal's output: {own}"
    );
}

#[tokio::test]
async fn a_context_only_fork_with_another_agent_hands_over_in_its_parents_folder() {
    let fixture = Fixture::with_other_agent().await;
    let mut request = fixture.request(pb::ForkKind::ContextOnly, "Read along");
    request.agent = Some(pb::AgentId { value: OTHER.to_owned() });
    let (operation, fork) = fork_of(&fixture.daemon, request).await;
    assert_eq!(operation.state, pb::OperationState::Succeeded as i32, "{operation:?}");
    let fork = fork.unwrap();
    assert_eq!(fork.agent.as_ref().map(|agent| agent.value.as_str()), Some(OTHER));
    wait_handed_over(&fixture.daemon, fork.id.as_ref().unwrap()).await;
    let folder = fixture.clone_of(&fixture.parent).parent().unwrap().to_owned();
    let handover = std::fs::read_to_string(handover_file(&folder, &fork)).unwrap();
    assert!(handover.contains("remember the retry"));
}

#[tokio::test]
async fn a_code_only_fork_with_another_agent_starts_it_with_nothing_to_read() {
    let fixture = Fixture::with_other_agent().await;
    let mut request = fixture.request(pb::ForkKind::CodeOnly, "Fresh eyes");
    request.agent = Some(pb::AgentId { value: OTHER.to_owned() });
    let preview = fixture
        .daemon
        .session_service()
        .await
        .preview_fork(request.clone())
        .await
        .unwrap()
        .into_inner();
    assert!(!preview.seeded_with_summary);
    let (operation, fork) = fork_of(&fixture.daemon, request).await;
    assert_eq!(operation.state, pb::OperationState::Succeeded as i32, "{operation:?}");
    let fork = common::wait_running(&fixture.daemon, fork.unwrap().id.as_ref().unwrap()).await;
    assert_eq!(fork.agent.as_ref().map(|agent| agent.value.as_str()), Some(OTHER));
    assert!(!handover_file(fixture.clone_of(&fork).parent().unwrap(), &fork).exists());
    assert!(!conversation(&fixture.world, &fork).contains("remember the retry"));
}

#[tokio::test]
async fn a_fork_with_an_agent_humata_does_not_know_is_refused() {
    let fixture = Fixture::with_other_agent().await;
    let mut request = fixture.request(pb::ForkKind::CodeAndContext, "Nobody");
    request.agent = Some(pb::AgentId { value: "nobody".to_owned() });
    let refused = fixture
        .daemon
        .session_service()
        .await
        .fork_session(request)
        .await
        .unwrap_err();
    assert_eq!(common::invalid_field(&refused).0, "agent");
    assert_eq!(fixture.sessions().await.len(), 1);
}

/// A machine this host has never linked is an unknown value of `machine`,
/// not a feature that isn't built, and nothing forks.
#[tokio::test]
async fn a_fork_onto_a_machine_this_host_does_not_know_is_refused_as_an_unknown_machine() {
    let fixture = Fixture::new().await;
    let mut request = fixture.request(pb::ForkKind::CodeAndContext, "Elsewhere");
    request.machine = Some(pb::MachineId { value: "ghost-box".to_owned() });
    let refused = fixture
        .daemon
        .session_service()
        .await
        .fork_session(request)
        .await
        .unwrap_err();
    let (field, problem) = common::invalid_field(&refused);
    assert_eq!(field, "machine");
    assert_eq!(problem, pb::invalid_field::Problem::UnknownValue);
    assert_eq!(fixture.sessions().await.len(), 1);
}

#[tokio::test]
async fn a_handover_cut_after_the_launch_is_typed_once_after_a_restart() {
    let mut fixture = Fixture::with_other_agent().await;
    fixture
        .daemon
        .restart_env(&[("HUMATA2_STOP_AT", "fork.verify")])
        .await;
    last_start(&fixture.daemon).await;
    let mut request = fixture.request(pb::ForkKind::CodeAndContext, "Cut at the handover");
    request.agent = Some(pb::AgentId { value: OTHER.to_owned() });
    let _ = fixture.daemon.session_service().await.fork_session(request).await;
    wait_dead(&fixture.daemon).await;
    fixture.daemon.restart().await;
    last_start(&fixture.daemon).await;

    let fork = fixture
        .sessions()
        .await
        .into_iter()
        .find(|s| s.id != Some(fixture.parent_id()))
        .expect("the fork past its launch is kept");
    let fork = common::wait_running(&fixture.daemon, fork.id.as_ref().unwrap()).await;
    wait_handed_over(&fixture.daemon, fork.id.as_ref().unwrap()).await;
    let own = conversation(&fixture.world, &fork);
    assert_eq!(own.matches("[handover").count(), 1, "{own}");
}

/// Archiving a context-only fork frees only what is its own: its handover
/// file goes, and the parent's folder and clones it worked in stay.
#[tokio::test]
async fn freeing_a_context_only_fork_removes_its_handover_file_and_keeps_its_parents_folder() {
    let fixture = Fixture::with_other_agent_and(&[("HUMATA2_ARCHIVE_UNDO_MS", "0")]).await;
    let mut request = fixture.request(pb::ForkKind::ContextOnly, "Read along");
    request.agent = Some(pb::AgentId { value: OTHER.to_owned() });
    let (operation, fork) = fork_of(&fixture.daemon, request).await;
    assert_eq!(operation.state, pb::OperationState::Succeeded as i32, "{operation:?}");
    let fork = fork.unwrap();
    let fork_id = fork.id.clone().unwrap();
    wait_handed_over(&fixture.daemon, &fork_id).await;
    let clone = fixture.clone_of(&fixture.parent);
    let handover = handover_file(clone.parent().unwrap(), &fork);
    assert!(handover.is_file(), "the fork was handed {}", handover.display());

    let service = fixture.daemon.session_service().await;
    let plan = service
        .clone()
        .plan_archive(pb::PlanArchiveRequest { id: Some(fork_id.clone()) })
        .await
        .unwrap()
        .into_inner();
    let started = service
        .clone()
        .archive_session(pb::ArchiveSessionRequest {
            idempotency_key: None,
            id: Some(fork_id.clone()),
            choice: pb::ArchiveChoice::Archive as i32,
            plan: plan.token,
            end: Some(true),
        })
        .await
        .unwrap()
        .into_inner();
    let archived = finished(&fixture.daemon, &started).await;
    assert_eq!(archived.state, pb::OperationState::Succeeded as i32, "{archived:?}");

    let deadline = Instant::now() + Duration::from_secs(60);
    while handover.exists() {
        assert!(Instant::now() < deadline, "{} was never removed", handover.display());
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(clone.join("login.rs").is_file(), "the parent's clone stays");
    assert!(clone.join("notes.txt").is_file(), "the parent's untracked work stays");
    let parent = get_session(&fixture.daemon, &fixture.parent_id()).await;
    assert_eq!(parent.repos.len(), 1, "the parent keeps its repo");
}

/// The names in `dir`, sorted.
fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// What a fork is cut against: the folders a restart after the cut must
/// leave as they were.
struct Before {
    /// The folder the channel's session folders are in.
    sessions_folder: PathBuf,
    folder_names: Vec<String>,
    conversation_names: Vec<String>,
}

impl Before {
    fn of(fixture: &Fixture) -> Self {
        let sessions_folder = fixture
            .clone_of(&fixture.parent)
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

/// After a restart, a cut fork is gone: no session, no folder, no copy of
/// its conversation, and the parent is as it was.
async fn assert_fork_undone(fixture: &Fixture, before: &Before) {
    let sessions = fixture.sessions().await;
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
    assert_eq!(parent.repos, fixture.parent.repos, "the parent keeps its repo");
    let clone = fixture.clone_of(&parent);
    assert_eq!(branch_of(&clone), parent.repos[0].branch);
    assert_eq!(std::fs::read_to_string(clone.join("login.rs")).unwrap(), "fn login() {}\n");
    assert_eq!(std::fs::read_to_string(clone.join("README.md")).unwrap(), "acme/api\nchanged\n");
    assert_eq!(std::fs::read_to_string(clone.join("notes.txt")).unwrap(), "untracked\n");
    assert!(conversation(&fixture.world, &parent).contains("remember the retry"));
}

#[tokio::test]
async fn a_fork_cut_when_its_records_are_saved_is_undone_at_restart() {
    let mut fixture = Fixture::new().await;
    let before = Before::of(&fixture);
    let request = fixture.request(pb::ForkKind::CodeAndContext, "Cut at the record");
    cut_fork_at(&mut fixture, "fork.write_record", request).await;
    assert_fork_undone(&fixture, &before).await;
}

#[tokio::test]
async fn a_fork_cut_after_its_conversation_is_copied_is_undone_at_restart() {
    let mut fixture = Fixture::new().await;
    let before = Before::of(&fixture);
    let request = fixture.request(pb::ForkKind::CodeAndContext, "Cut after the conversation");
    cut_fork_at(&mut fixture, "fork.copy_conversation", request).await;
    assert_fork_undone(&fixture, &before).await;
}

#[tokio::test]
async fn a_fork_cut_before_its_agent_launches_is_undone_at_restart() {
    let mut fixture = Fixture::new().await;
    let before = Before::of(&fixture);
    let request = fixture.request(pb::ForkKind::CodeAndContext, "Cut at the launch");
    cut_fork_at(&mut fixture, "fork.launch", request).await;
    assert_fork_undone(&fixture, &before).await;
}

#[tokio::test]
async fn a_code_only_fork_cut_before_its_agent_launches_is_undone_at_restart() {
    let mut fixture = Fixture::new().await;
    let before = Before::of(&fixture);
    let request = fixture.request(pb::ForkKind::CodeOnly, "Clean cut");
    cut_fork_at(&mut fixture, "fork.launch", request).await;
    assert_fork_undone(&fixture, &before).await;
}

#[tokio::test]
async fn a_handed_over_fork_cut_after_its_conversation_is_copied_is_undone_at_restart() {
    let mut fixture = Fixture::with_other_agent().await;
    let before = Before::of(&fixture);
    let mut request = fixture.request(pb::ForkKind::CodeAndContext, "Cut at the handover");
    request.agent = Some(pb::AgentId { value: OTHER.to_owned() });
    cut_fork_at(&mut fixture, "fork.copy_conversation", request).await;
    assert_fork_undone(&fixture, &before).await;
}
