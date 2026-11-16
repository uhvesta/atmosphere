//! Forking a session on this machine (FK-1..FK-5, FK-13, PER-27) through a
//! real `humatad2`. Needs FORK's seams (store lifecycle tables, the fork
//! handlers); moves up from tests/pending with them.

mod common;

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
        say(&fixture.daemon, &fixture.parent_id(), "remember the retry").await;
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
