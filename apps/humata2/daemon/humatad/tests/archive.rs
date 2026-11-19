//! Archiving, Undo and Restore on this machine (MV-7..MV-9, MV-15, LD-2,
//! LD-4, FK-13, D110, D170) through a real `humatad2`. Needs FORK's seams;
//! moves up from tests/pending with them.

use humatad_test_common as common;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use common::{
    Attach, Daemon, World, agent_tab, create_channel, finished, folder, get_session, git,
    last_start, open, start_session,
};
use humata2_grpc::humata::v1 as pb;
use pb::SessionStatus as Status;

/// How long the clones wait after an archive, in admin test builds only.
const UNDO_WINDOW: &str = "HUMATA2_ARCHIVE_UNDO_MS";
/// Long enough that no test sees it close by itself.
const NEVER: &str = "600000";

struct Fixture {
    daemon: Daemon,
    world: World,
    channel: pb::Channel,
}

impl Fixture {
    /// A daemon whose undo window is `window` ms (for this start only).
    async fn new(window: &str) -> Self {
        Self::with_env(&[(UNDO_WINDOW, window)]).await
    }

    async fn with_env(env: &[(&str, &str)]) -> Self {
        let dir = folder();
        let world = World::new(dir.path());
        let args = world.args();
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let daemon = Daemon::start_with_env(common::Build::Admin, dir, &refs, env).await;
        world.remote("acme", "api");
        let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
        Self {
            daemon,
            world,
            channel,
        }
    }

    async fn session(&self, title: &str) -> pb::Session {
        let (id, _) = start_session(&self.daemon, &self.channel, title).await;
        let session = common::wait_running(&self.daemon, &id).await;
        say(&self.daemon, &id, "before the archive").await;
        session
    }

    async fn plan(&self, id: &pb::SessionId) -> pb::ArchivePlan {
        self.daemon
            .session_service()
            .await
            .plan_archive(pb::PlanArchiveRequest {
                id: Some(id.clone()),
            })
            .await
            .unwrap()
            .into_inner()
    }

    async fn archive(&self, id: &pb::SessionId, choice: pb::ArchiveChoice) -> pb::Operation {
        let plan = self.plan(id).await;
        let started = self
            .daemon
            .session_service()
            .await
            .archive_session(pb::ArchiveSessionRequest {
                idempotency_key: None,
                id: Some(id.clone()),
                choice: choice as i32,
                plan: plan.token,
                end: Some(true),
            })
            .await
            .unwrap()
            .into_inner();
        finished(&self.daemon, &started).await
    }

    async fn restore(&self, id: &pb::SessionId) -> pb::Operation {
        self.restore_as(id, false).await
    }

    /// Restores it; `stopped` leaves it stopped, as the close dialog's Undo
    /// of End for good does (D784).
    async fn restore_as(&self, id: &pb::SessionId, stopped: bool) -> pb::Operation {
        let started = self
            .daemon
            .session_service()
            .await
            .restore_session(pb::RestoreSessionRequest {
                idempotency_key: None,
                id: Some(id.clone()),
                machine: None,
                stopped,
            })
            .await
            .unwrap()
            .into_inner();
        finished(&self.daemon, &started).await
    }

    async fn listed(&self, include_archived: bool) -> Vec<pb::SessionId> {
        self.daemon
            .session_service()
            .await
            .list_sessions(pb::ListSessionsRequest {
                channel: None,
                machine: None,
                include_archived,
                page: None,
            })
            .await
            .unwrap()
            .into_inner()
            .sessions
            .into_iter()
            .filter_map(|s| s.id)
            .collect()
    }

    async fn fork(&self, parent: &pb::SessionId, title: &str) -> pb::SessionId {
        let started = self
            .daemon
            .session_service()
            .await
            .fork_session(pb::ForkSessionRequest {
                idempotency_key: None,
                parent: Some(parent.clone()),
                kind: pb::ForkKind::CodeOnly as i32,
                before_turn: 0,
                title: title.to_owned(),
                agent: None,
                model: String::new(),
                machine: None,
                branch: String::new(),
                reason: String::new(),
            })
            .await
            .unwrap()
            .into_inner();
        let operation = finished(&self.daemon, &started).await;
        match &operation.result {
            Some(pb::operation::Result::Session(id)) => id.clone(),
            _ => panic!("the fork failed: {operation:?}"),
        }
    }
}

fn clone_of(session: &pb::Session) -> PathBuf {
    PathBuf::from(&session.repos[0].path)
}

fn folder_of(session: &pb::Session) -> PathBuf {
    clone_of(session).parent().unwrap().to_owned()
}

async fn say(daemon: &Daemon, id: &pb::SessionId, line: &str) {
    let tab = agent_tab(daemon, id).await;
    let mut attach = Attach::start(daemon, open(&tab, 0)).await;
    attach.type_text(&format!("{line}\r")).await;
    attach.wait_for(&format!("got: {line}")).await;
}

fn succeeded(operation: &pb::Operation) {
    assert_eq!(
        operation.state,
        pb::OperationState::Succeeded as i32,
        "{operation:?}"
    );
}

async fn wait_gone(path: &std::path::Path) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while path.exists() {
        assert!(Instant::now() < deadline, "{} was never deleted", path.display());
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn wait_dead(daemon: &Daemon) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while std::os::unix::net::UnixStream::connect(&daemon.socket).is_ok() {
        assert!(Instant::now() < deadline, "the daemon never stopped at its stop point");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn commit_and_push(session: &pb::Session) -> String {
    let clone = clone_of(session).display().to_string();
    git(&["-C", &clone, "checkout", "-q", &session.repos[0].branch]);
    std::fs::write(clone_of(session).join("pushed.txt"), "on the remote\n").unwrap();
    git(&["-C", &clone, "add", "pushed.txt"]);
    git(&["-C", &clone, "commit", "-q", "-m", "pushed"]);
    git(&["-C", &clone, "push", "-q", "origin", &session.repos[0].branch]);
    git(&["-C", &clone, "rev-parse", "HEAD"])
}

#[tokio::test]
async fn the_plan_names_unpushed_work_and_running_forks() {
    let fixture = Fixture::new(NEVER).await;
    let clean = fixture.session("Clean").await;
    let plan = fixture.plan(clean.id.as_ref().unwrap()).await;
    assert!(plan.unpushed.is_empty(), "{plan:?}");
    assert!(plan.choices.contains(&(pb::ArchiveChoice::Archive as i32)));
    assert!(!plan.choices.contains(&(pb::ArchiveChoice::ArchiveAndDeleteWork as i32)));
    assert_eq!(plan.forks_keep_running, 0);

    let dirty = fixture.session("Dirty").await;
    std::fs::write(clone_of(&dirty).join("wip.txt"), "wip\n").unwrap();
    fixture.fork(dirty.id.as_ref().unwrap(), "Its fork").await;
    let plan = fixture.plan(dirty.id.as_ref().unwrap()).await;
    assert_eq!(plan.forks_keep_running, 1);
    assert_eq!(plan.unpushed.len(), 1, "{plan:?}");
    let repo = &plan.unpushed[0];
    assert_eq!(repo.branch, dirty.repos[0].branch);
    assert_eq!(repo.dirty_files, 1);
    assert!(repo.never_pushed);
    assert!(plan.choices.contains(&(pb::ArchiveChoice::ArchiveAndDeleteWork as i32)));
    assert!(!plan.choices.contains(&(pb::ArchiveChoice::Archive as i32)));
    assert!(plan.token.is_some_and(|t| !t.value.is_empty()));
}

#[tokio::test]
async fn plain_archive_is_refused_while_there_is_unpushed_work() {
    let fixture = Fixture::new(NEVER).await;
    let session = fixture.session("Dirty").await;
    let id = session.id.clone().unwrap();
    std::fs::write(clone_of(&session).join("wip.txt"), "wip\n").unwrap();
    let plan = fixture.plan(&id).await;
    let status = fixture
        .daemon
        .session_service()
        .await
        .archive_session(pb::ArchiveSessionRequest {
            idempotency_key: None,
            id: Some(id.clone()),
            choice: pb::ArchiveChoice::Archive as i32,
            plan: plan.token,
                end: Some(true),
        })
        .await
        .unwrap_err();
    assert_eq!(status.code(), tonic::Code::FailedPrecondition);
    assert!(common::is_running(&get_session(&fixture.daemon, &id).await), "the session runs on");
    assert!(clone_of(&session).join("wip.txt").exists());
}

#[tokio::test]
async fn undo_brings_the_session_back_exactly() {
    let fixture = Fixture::new(NEVER).await;
    let session = fixture.session("Fix login").await;
    let id = session.id.clone().unwrap();
    std::fs::write(clone_of(&session).join("wip.txt"), "wip\n").unwrap();

    succeeded(&fixture.archive(&id, pb::ArchiveChoice::ArchiveAndDeleteWork).await);
    let archived = get_session(&fixture.daemon, &id).await;
    assert_eq!(archived.status, Status::Archived as i32);
    let summary = archived.archive.clone().expect("an archived session has a summary");
    assert_eq!(summary.repos.len(), 1);
    assert_eq!(summary.repos[0].branch, session.repos[0].branch);
    assert!(summary.repos[0].files.iter().any(|f| f.path == "wip.txt"));
    assert!(!fixture.listed(false).await.contains(&id));
    assert!(fixture.listed(true).await.contains(&id));
    // Inside the undo window nothing is deleted yet.
    assert!(clone_of(&session).join("wip.txt").exists());

    succeeded(&fixture.restore(&id).await);
    let back = common::wait_running(&fixture.daemon, &id).await;
    assert!(back.archive.is_none());
    assert_eq!(back.conversation_id, session.conversation_id);
    assert_eq!(back.repos[0].branch, session.repos[0].branch);
    assert_eq!(std::fs::read_to_string(clone_of(&back).join("wip.txt")).unwrap(), "wip\n");
    say(&fixture.daemon, &id, "after the undo").await;
    let said = std::fs::read_to_string(fixture.world.conversation(&back.conversation_id)).unwrap();
    assert!(said.contains("before the archive") && said.contains("after the undo"));
}

/// D784: Undo of End for good gives the session back stopped, not running,
/// and Resume then brings back the same conversation.
#[tokio::test]
async fn undo_of_end_for_good_gives_it_back_stopped_and_resume_revives_it() {
    let fixture = Fixture::new(NEVER).await;
    let session = fixture.session("Fix login").await;
    let id = session.id.clone().unwrap();

    succeeded(&fixture.archive(&id, pb::ArchiveChoice::Archive).await);
    succeeded(&fixture.restore_as(&id, true).await);
    let back = get_session(&fixture.daemon, &id).await;
    assert!(back.archive.is_none(), "an undone archive still reads as archived");
    assert_eq!(back.status, Status::Stopped as i32, "{back:?}");
    // It stays stopped: nothing resumes it behind the user's back.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(get_session(&fixture.daemon, &id).await.status, Status::Stopped as i32);

    let resumed = fixture
        .daemon
        .session_service()
        .await
        .resume_session(pb::ResumeSessionRequest {
            idempotency_key: None,
            id: Some(id.clone()),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    succeeded(&finished(&fixture.daemon, &resumed).await);
    let running = common::wait_running(&fixture.daemon, &id).await;
    assert_eq!(running.conversation_id, session.conversation_id);
    assert_eq!(running.folder, session.folder);
    say(&fixture.daemon, &id, "after the resume").await;
    let said = std::fs::read_to_string(fixture.world.conversation(&running.conversation_id)).unwrap();
    assert!(said.contains("before the archive") && said.contains("after the resume"));
}

/// End for good can't be resumed (D784): only Restore brings it back. Found
/// live in VERIFY 0.2.3, where Resume started a new agent for an archived
/// session.
#[tokio::test]
async fn an_ended_session_refuses_resume_and_starts_no_agent() {
    let fixture = Fixture::new(NEVER).await;
    let session = fixture.session("Fix login").await;
    let id = session.id.clone().unwrap();

    succeeded(&fixture.archive(&id, pb::ArchiveChoice::Archive).await);
    let refused = fixture
        .daemon
        .session_service()
        .await
        .resume_session(pb::ResumeSessionRequest {
            idempotency_key: None,
            id: Some(id.clone()),
            ..Default::default()
        })
        .await
        .expect_err("an archived session was resumed");
    assert_eq!(refused.code(), tonic::Code::FailedPrecondition, "{refused:?}");
    tokio::time::sleep(Duration::from_millis(300)).await;
    let after = get_session(&fixture.daemon, &id).await;
    assert!(after.archive.is_some(), "the refused resume unarchived it: {after:?}");
    assert_eq!(after.status, Status::Archived as i32, "an agent runs for an archived session: {after:?}");
}

#[tokio::test]
async fn the_clones_go_when_the_window_closes_and_restore_clones_what_was_pushed() {
    let fixture = Fixture::new("0").await;
    let session = fixture.session("Ship it").await;
    let id = session.id.clone().unwrap();
    let pushed = commit_and_push(&session);

    succeeded(&fixture.archive(&id, pb::ArchiveChoice::Archive).await);
    wait_gone(&folder_of(&session)).await;
    let archived = get_session(&fixture.daemon, &id).await;
    assert_eq!(archived.status, Status::Archived as i32);
    assert_eq!(archived.archive.unwrap().repos[0].last_pushed_sha, pushed);

    succeeded(&fixture.restore(&id).await);
    let back = common::wait_running(&fixture.daemon, &id).await;
    let clone = clone_of(&back).display().to_string();
    assert_eq!(git(&["-C", &clone, "rev-parse", "HEAD"]), pushed);
    assert_eq!(git(&["-C", &clone, "branch", "--show-current"]), session.repos[0].branch);
    assert_eq!(back.conversation_id, session.conversation_id);
}

#[tokio::test]
async fn restoring_a_branch_never_pushed_starts_it_from_main() {
    let fixture = Fixture::new("0").await;
    let session = fixture.session("Throwaway").await;
    let id = session.id.clone().unwrap();
    std::fs::write(clone_of(&session).join("wip.txt"), "wip\n").unwrap();

    succeeded(&fixture.archive(&id, pb::ArchiveChoice::ArchiveAndDeleteWork).await);
    wait_gone(&folder_of(&session)).await;
    succeeded(&fixture.restore(&id).await);
    let back = common::wait_running(&fixture.daemon, &id).await;
    assert!(!clone_of(&back).join("wip.txt").exists());
    let clone = clone_of(&back).display().to_string();
    assert_eq!(git(&["-C", &clone, "branch", "--show-current"]), session.repos[0].branch);
    let main = World::remote_head(&fixture.world.remotes.join("acme").join("api.git"));
    assert_eq!(git(&["-C", &clone, "rev-parse", "HEAD"]), main);
}

#[tokio::test]
async fn a_restart_inside_the_window_finishes_the_delete_when_it_closes() {
    // A window short enough to close while the test waits, long enough to
    // outlast the archive call (D430: the free is kept across the restart).
    let mut fixture = Fixture::new("5000").await;
    let session = fixture.session("Fix login").await;
    let id = session.id.clone().unwrap();
    succeeded(&fixture.archive(&id, pb::ArchiveChoice::Archive).await);
    assert!(folder_of(&session).exists());
    fixture.daemon.restart().await;
    last_start(&fixture.daemon).await;
    wait_gone(&folder_of(&session)).await;
    assert_eq!(get_session(&fixture.daemon, &id).await.status, Status::Archived as i32);
}

#[tokio::test]
async fn an_archive_cut_short_is_finished_at_restart() {
    let mut fixture = Fixture::new(NEVER).await;
    let session = fixture.session("Fix login").await;
    let id = session.id.clone().unwrap();
    fixture
        .daemon
        .restart_env(&[("HUMATA2_STOP_AT", "archive.recorded")])
        .await;
    last_start(&fixture.daemon).await;
    let plan = fixture.plan(&id).await;
    let _ = fixture
        .daemon
        .session_service()
        .await
        .archive_session(pb::ArchiveSessionRequest {
            idempotency_key: None,
            id: Some(id.clone()),
            choice: pb::ArchiveChoice::Archive as i32,
            plan: plan.token,
                end: Some(true),
        })
        .await;
    wait_dead(&fixture.daemon).await;
    fixture.daemon.restart().await;
    last_start(&fixture.daemon).await;
    // Inside its undo window, so its folder is kept.
    let archived = get_session(&fixture.daemon, &id).await;
    assert_eq!(archived.status, Status::Archived as i32);
    assert!(archived.archive.is_some());
    assert!(folder_of(&session).exists());
    // It was asked to end its terminals, and the restart that finishes it
    // keeps that choice (D801 F).
    let tabs = common::stored_tabs(&fixture.daemon.state);
    assert!(
        !tabs
            .iter()
            .filter(|t| t.session.as_deref() == Some(id.value.as_str()))
            .any(common::holder_answers),
        "a cut archive asked to end its terminals left them running"
    );
}

/// A restart inside an archive's undo window keeps the session's folder:
/// it is freed when the window closes, not when the daemon comes back.
#[tokio::test]
async fn a_restart_inside_the_undo_window_keeps_the_folder() {
    let mut fixture = Fixture::new(NEVER).await;
    let session = fixture.session("Fix login").await;
    let id = session.id.clone().unwrap();
    succeeded(&fixture.archive(&id, pb::ArchiveChoice::Archive).await);
    fixture.daemon.restart_env(&[(UNDO_WINDOW, NEVER)]).await;
    last_start(&fixture.daemon).await;
    // Long enough for a free at start to have run.
    let settle = Instant::now() + Duration::from_secs(2);
    while Instant::now() < settle {
        assert!(folder_of(&session).exists(), "the folder was freed inside its undo window");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(get_session(&fixture.daemon, &id).await.status, Status::Archived as i32);
}

/// Every step an archive takes, as the points a cut can land on: the
/// intent recorded, the agent stopped, the summary written (the store's own
/// points either side of the commit), and the folder removed and the row
/// freed. The `@n` is the nth time that store point is reached in the run.
const ARCHIVE_POINTS: &[&str] = &[
    "archive.recorded",
    "archive.agent_stopped",
    "store.archive.put.committing@2",
    "archive.archived",
    "archive.folder_removed",
    "store.archive.put.committed@3",
];

/// An archive cut before its intent is saved leaves no trace: the session
/// runs on, whole.
#[tokio::test]
async fn an_archive_cut_before_it_is_recorded_leaves_the_session_running() {
    let mut fixture = Fixture::new("0").await;
    let session = fixture.session("Fix login").await;
    let id = session.id.clone().unwrap();
    fixture
        .daemon
        .restart_env(&[(UNDO_WINDOW, "0"), ("HUMATA2_STOP_AT", "store.archive.put.committing@1")])
        .await;
    last_start(&fixture.daemon).await;
    let plan = fixture.plan(&id).await;
    let _ = fixture
        .daemon
        .session_service()
        .await
        .archive_session(pb::ArchiveSessionRequest {
            idempotency_key: None,
            id: Some(id.clone()),
            choice: pb::ArchiveChoice::Archive as i32,
            plan: plan.token,
                end: Some(true),
        })
        .await;
    wait_dead(&fixture.daemon).await;
    fixture.daemon.restart_env(&[(UNDO_WINDOW, "0")]).await;
    last_start(&fixture.daemon).await;
    let running = common::wait_running(&fixture.daemon, &id).await;
    assert!(folder_of(&session).exists());
    assert!(running.archive.is_none());
}

/// An archive cut at any later point is finished at the next start: the session
/// is archived, its folder is freed, and no agent or holder is left.
#[tokio::test]
async fn an_archive_cut_at_any_point_is_finished_at_restart() {
    for point in ARCHIVE_POINTS {
        let mut fixture = Fixture::new("0").await;
        let session = fixture.session("Fix login").await;
        let id = session.id.clone().unwrap();
        fixture
            .daemon
            .restart_env(&[(UNDO_WINDOW, "0"), ("HUMATA2_STOP_AT", point)])
            .await;
        last_start(&fixture.daemon).await;
        let plan = fixture.plan(&id).await;
        let _ = fixture
            .daemon
            .session_service()
            .await
            .archive_session(pb::ArchiveSessionRequest {
                idempotency_key: None,
                id: Some(id.clone()),
                choice: pb::ArchiveChoice::Archive as i32,
                plan: plan.token,
                end: Some(true),
            })
            .await;
        wait_dead(&fixture.daemon).await;
        fixture.daemon.restart_env(&[(UNDO_WINDOW, "0")]).await;
        last_start(&fixture.daemon).await;
        // The choice to end its terminals is kept with the archive, so the
        // restart ends them wherever it was cut (D801 F), then frees the folder.
        let still_running = false;
        wait_gone(&folder_of(&session)).await;
        let archived = get_session(&fixture.daemon, &id).await;
        assert_eq!(archived.status, Status::Archived as i32, "cut at {point}");
        assert!(archived.archive.is_some(), "cut at {point}");
        let tabs = common::stored_tabs(&fixture.daemon.state);
        assert_eq!(
            tabs.iter()
                .filter(|t| t.session.as_deref() == Some(id.value.as_str()))
                .any(common::holder_answers),
            still_running,
            "cut at {point}"
        );
        assert_eq!(folder_of(&session).exists(), still_running, "cut at {point}");
    }
}

#[tokio::test]
async fn forks_move_up_while_their_parent_is_archived_and_back_on_restore() {
    let fixture = Fixture::new(NEVER).await;
    let top = fixture.session("Top").await;
    let top_id = top.id.clone().unwrap();
    let middle = fixture.fork(&top_id, "Middle").await;
    let bottom = fixture.fork(&middle, "Bottom").await;

    succeeded(&fixture.archive(&middle, pb::ArchiveChoice::ArchiveAndDeleteWork).await);
    // FK-13: the fork keeps running, now under the archived one's parent.
    let moved = common::wait_running(&fixture.daemon, &bottom).await;
    assert_eq!(moved.fork_origin.unwrap().parent, Some(top_id.clone()));
    let top_now = get_session(&fixture.daemon, &top_id).await;
    assert!(top_now.forks.contains(&bottom));
    assert!(!top_now.forks.contains(&middle));

    succeeded(&fixture.restore(&middle).await);
    let back = get_session(&fixture.daemon, &bottom).await;
    assert_eq!(back.fork_origin.unwrap().parent, Some(middle.clone()));
    assert_eq!(get_session(&fixture.daemon, &top_id).await.forks, vec![middle]);
}

#[tokio::test]
async fn an_archived_session_cannot_be_archived_again() {
    let fixture = Fixture::new(NEVER).await;
    let session = fixture.session("Once").await;
    let id = session.id.clone().unwrap();
    succeeded(&fixture.archive(&id, pb::ArchiveChoice::Archive).await);
    let status = fixture
        .daemon
        .session_service()
        .await
        .plan_archive(pb::PlanArchiveRequest { id: Some(id) })
        .await
        .unwrap_err();
    assert_eq!(status.code(), tonic::Code::FailedPrecondition);
}
