//! Desk sessions on this machine (DK-2, DK-4, DK-5, DK-11, DK-14, D148,
//! D182) through a real `humatad2`: start in a scratch or picked folder,
//! close and reopen, keep, the two access switches, the 7-day delete and
//! Clean up. Parked until DeskService is served; moves up to `tests/` with
//! it.

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::{
    Attach, Build, Daemon, World, agent_tab, folder, get_session, git, humata_as_agent, machine_id,
    open, repo_ref, shell_token, stored_tabs, wait_status,
};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{DeskFolderKind, DeskRow, SessionDetail, SessionRecord, StartStep, Store};
use pb::SessionStatus as Status;
use rustix::process::{Pid, Signal, kill_process};
use tonic::Code;
use tonic::transport::Channel;

/// How long after closing a scratch folder goes, in admin test builds only.
const EXPIRE: &str = "HUMATA2_DESK_EXPIRE_MS";
/// Long enough that no test sees it pass by itself.
const NEVER: &str = "600000";

struct Fixture {
    daemon: Daemon,
    world: World,
}

impl Fixture {
    async fn new() -> Self {
        Self::expiring_after(NEVER).await
    }

    async fn expiring_after(ms: &str) -> Self {
        Self::with_env(&[(EXPIRE, ms)]).await
    }

    /// A daemon started with `env`, such as a fault point to stop at.
    async fn with_env(env: &[(&str, &str)]) -> Self {
        let dir = folder();
        let world = World::new(dir.path());
        let args = world.args();
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let daemon = Daemon::start_with_env(Build::Admin, dir, &refs, env).await;
        Self { daemon, world }
    }

    async fn desk(&self) -> pb::desk_service_client::DeskServiceClient<Channel> {
        pb::desk_service_client::DeskServiceClient::new(self.daemon.channel().await)
    }

    fn request(&self, title: &str) -> pb::StartDeskSessionRequest {
        pb::StartDeskSessionRequest {
            idempotency_key: None,
            title: title.to_owned(),
            agent: None,
            model: String::new(),
            machine: None,
            picked_path: String::new(),
            repos: Vec::new(),
            access: Some(pb::DeskAccess {
                boards: true,
                channels_and_machines: false,
            }),
            first_message: String::new(),
            channel: None,
            target: None,
            switches: None,
        }
    }

    async fn try_start(
        &self,
        request: pb::StartDeskSessionRequest,
    ) -> Result<pb::StartDeskSessionResponse, tonic::Status> {
        self.desk()
            .await
            .start_desk_session(request)
            .await
            .map(tonic::Response::into_inner)
    }

    /// Starts `request` and waits until its agent is up.
    async fn start_with(&self, request: pb::StartDeskSessionRequest) -> (pb::SessionId, PathBuf) {
        let started = self.try_start(request).await.unwrap();
        let id = started.session.unwrap();
        wait_status(&self.daemon, &id, Status::Working).await;
        let path = PathBuf::from(started.desk.unwrap().folder.unwrap().path);
        (id, path)
    }

    async fn start(&self, title: &str) -> (pb::SessionId, PathBuf) {
        self.start_with(self.request(title)).await
    }

    async fn listed(&self) -> Vec<pb::DeskSession> {
        self.desk()
            .await
            .list_desk_sessions(pb::ListDeskSessionsRequest::default())
            .await
            .unwrap()
            .into_inner()
            .sessions
    }

    async fn find(&self, id: &pb::SessionId) -> Option<pb::DeskSession> {
        self.listed()
            .await
            .into_iter()
            .find(|desk| desk.session.as_ref() == Some(id))
    }

    async fn close(&self, id: &pb::SessionId) -> pb::DeskSession {
        self.desk()
            .await
            .close_desk_session(pb::CloseDeskSessionRequest {
                idempotency_key: None,
                session: Some(id.clone()),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
    }

    async fn reopen(&self, id: &pb::SessionId) -> Result<pb::Operation, tonic::Status> {
        self.desk()
            .await
            .reopen_desk_session(pb::ReopenDeskSessionRequest {
                idempotency_key: None,
                session: Some(id.clone()),
            })
            .await
            .map(tonic::Response::into_inner)
    }

    async fn keep(&self, id: &pb::SessionId) -> pb::DeskSession {
        self.desk()
            .await
            .keep_desk_folder(pb::KeepDeskFolderRequest {
                session: Some(id.clone()),
            })
            .await
            .unwrap()
            .into_inner()
    }

    async fn set_access(&self, id: &pb::SessionId, boards: bool, channels_and_machines: bool) -> pb::DeskSession {
        self.desk()
            .await
            .set_desk_access(pb::SetDeskAccessRequest {
                session: Some(id.clone()),
                access: Some(pb::DeskAccess {
                    boards,
                    channels_and_machines,
                }),
                switches: None,
            })
            .await
            .unwrap()
            .into_inner()
    }

    async fn plan(&self) -> pb::DeskCleanUpPlan {
        self.desk()
            .await
            .plan_desk_clean_up(pb::PlanDeskCleanUpRequest {})
            .await
            .unwrap()
            .into_inner()
    }

    async fn clean_up(&self, plan: &pb::DeskCleanUpPlan, delete: &[&pb::SessionId]) -> pb::clean_up_desk_response::Result {
        self.desk()
            .await
            .clean_up_desk(pb::CleanUpDeskRequest {
                idempotency_key: None,
                plan: plan.token.clone(),
                delete: delete.iter().map(|id| (*id).clone()).collect(),
            })
            .await
            .unwrap()
            .into_inner()
            .result
            .expect("clean up answers a result")
    }

    async fn trash(&self) -> Vec<pb::TrashItem> {
        pb::trash_service_client::TrashServiceClient::new(self.daemon.channel().await)
            .list_trash(pb::ListTrashRequest::default())
            .await
            .unwrap()
            .into_inner()
            .items
    }

    async fn session_ids(&self) -> Vec<pb::SessionId> {
        self.daemon
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
            .into_iter()
            .filter_map(|session| session.id)
            .collect()
    }

    /// `<channels root>/desk`.
    async fn add_repo(&self, id: &pb::SessionId, owner: &str, name: &str) -> Result<pb::DeskSession, tonic::Status> {
        self.desk()
            .await
            .add_desk_repo(pb::AddDeskRepoRequest {
                session: Some(id.clone()),
                repo: Some(repo_ref(owner, name)),
            })
            .await
            .map(tonic::Response::into_inner)
    }

    async fn remove_repo(&self, id: &pb::SessionId, owner: &str, name: &str) -> Result<pb::DeskSession, tonic::Status> {
        self.desk()
            .await
            .remove_desk_repo(pb::RemoveDeskRepoRequest {
                session: Some(id.clone()),
                repo: Some(repo_ref(owner, name)),
            })
            .await
            .map(tonic::Response::into_inner)
    }

    async fn copy(
        &self,
        id: &pb::SessionId,
        path: &str,
        channel: &pb::Channel,
    ) -> Result<pb::CopyDeskDocumentResponse, tonic::Status> {
        self.desk()
            .await
            .copy_desk_document(pb::CopyDeskDocumentRequest {
                idempotency_key: None,
                session: Some(id.clone()),
                path: path.to_owned(),
                channel: channel.id.clone(),
                machine: None,
            })
            .await
            .map(tonic::Response::into_inner)
    }

    fn desk_root(&self) -> PathBuf {
        self.world.channels.join("desk")
    }

    /// A Desk session in a folder the person picked, as an older release
    /// recorded it. No start can make one now (D451, D460), so its records
    /// are written to the store the way the old data holds them: the
    /// session, its finished start and its Desk row. It has no agent, so it
    /// can be closed, listed and cleaned up but not resumed.
    async fn old_picked_session(&self, title: &str, folder: &Path) -> pb::SessionId {
        let id = format!("se_old_{}", title.to_lowercase());
        let store = Store::open(self.daemon.state.join("store")).unwrap();
        store
            .put_session_with_detail(
                &SessionRecord {
                    id: id.clone(),
                    channel: "desk".to_owned(),
                    machine: machine_id(&self.daemon).await.value,
                    title: title.to_owned(),
                    agent: common::AGENT_NAME.to_owned(),
                    folder: folder.canonicalize().unwrap(),
                    created_unix_ms: 1,
                },
                &SessionDetail {
                    session: id.clone(),
                    repos: Vec::new(),
                    model: None,
                    first_message: None,
                    start: StartStep::Done,
                    operation: None,
                    revision: 1,
                },
            )
            .unwrap();
        store
            .put_desk(&DeskRow {
                session_id: id.clone(),
                kind: DeskFolderKind::Picked,
                boards: true,
                channels_and_machines: false,
                last_used_unix_ms: 1,
                closed_unix_ms: None,
                deleted_unix_ms: None,
                held_work: String::new(),
            })
            .unwrap();
        pb::SessionId { value: id }
    }
}

fn scratch_of(desk: &pb::DeskSession) -> Option<pb::desk_folder::Scratch> {
    match desk.folder.as_ref()?.kind.as_ref()? {
        pb::desk_folder::Kind::Scratch(scratch) => Some(scratch.clone()),
        pb::desk_folder::Kind::Picked(_) => None,
    }
}

fn unix_ms(at: &Option<humata2_grpc::Timestamp>) -> Option<i64> {
    at.as_ref().map(|at| at.seconds * 1000 + i64::from(at.nanos) / 1_000_000)
}

async fn say(daemon: &Daemon, id: &pb::SessionId, line: &str) -> Attach {
    let tab = agent_tab(daemon, id).await;
    let mut attach = Attach::start(daemon, open(&tab, 0)).await;
    attach.type_text(&format!("{line}\r")).await;
    attach.wait_for(&format!("got: {line}")).await;
    attach
}

async fn wait_until<F: std::future::Future<Output = bool>>(what: &str, mut done: impl FnMut() -> F) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !done().await {
        assert!(Instant::now() < deadline, "{what} never happened");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn exists(path: &Path) -> bool {
    path.exists()
}

// ----- Start. -----

#[tokio::test]
async fn a_scratch_session_runs_its_agent_in_a_folder_named_from_its_title() {
    let fixture = Fixture::new().await;
    let (id, path) = fixture.start("Plan Q4").await;
    assert_eq!(path, fixture.desk_root().join("plan-q4"));
    assert!(path.is_dir());
    let session = get_session(&fixture.daemon, &id).await;
    assert_eq!(session.channel.unwrap().value, "desk");
    // The agent runs there: a file it is told to write lands in the folder.
    let tab = agent_tab(&fixture.daemon, &id).await;
    let mut attach = Attach::start(&fixture.daemon, open(&tab, 0)).await;
    attach.wait_for("agent up --session-id").await;

    let desk = fixture.find(&id).await.expect("listed on the Desk");
    let scratch = scratch_of(&desk).expect("a scratch folder");
    assert!(!scratch.kept);
    assert!(desk.closed_at.is_none());
    assert!(desk.delete_at.is_none());
    let access = desk.access.unwrap();
    assert!(access.boards);
    assert!(!access.channels_and_machines);
    assert!(desk.last_used_at.is_some());
}

#[tokio::test]
async fn a_taken_name_gets_the_next_suffix() {
    let fixture = Fixture::new().await;
    let (_, first) = fixture.start("Plan Q4").await;
    let (_, second) = fixture.start("Plan Q4").await;
    // A folder someone made by hand is taken too.
    std::fs::create_dir_all(fixture.desk_root().join("plan-q4-3")).unwrap();
    let (_, third) = fixture.start("Plan Q4").await;
    assert_eq!(first.file_name().unwrap(), "plan-q4");
    assert_eq!(second.file_name().unwrap(), "plan-q4-2");
    assert_eq!(third.file_name().unwrap(), "plan-q4-4");
}

#[tokio::test]
async fn an_untitled_session_gets_the_fallback_name() {
    let fixture = Fixture::new().await;
    let (_, path) = fixture.start("").await;
    assert_eq!(path.parent().unwrap(), fixture.desk_root());
    assert!(!path.file_name().unwrap().is_empty());
}

#[tokio::test]
async fn a_retried_start_makes_one_session_and_one_folder() {
    let fixture = Fixture::new().await;
    let mut request = fixture.request("Plan Q4");
    request.idempotency_key = Some(pb::IdempotencyKey {
        value: "start-once".to_owned(),
    });
    let first = fixture.try_start(request.clone()).await.unwrap();
    let again = fixture.try_start(request).await.unwrap();
    assert_eq!(first.session, again.session);
    assert_eq!(fixture.listed().await.len(), 1);
    assert!(!fixture.desk_root().join("plan-q4-2").exists());
}

#[tokio::test]
async fn renaming_keeps_the_folder() {
    let fixture = Fixture::new().await;
    let (id, path) = fixture.start("Plan Q4").await;
    fixture
        .daemon
        .session_service()
        .await
        .rename_session(pb::RenameSessionRequest {
            id: Some(id.clone()),
            title: "Q4 planning".to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();
    let desk = fixture.find(&id).await.unwrap();
    assert_eq!(PathBuf::from(desk.folder.unwrap().path), path);
}

/// D451, D460: a start that names a picked folder is refused whatever the
/// path (a good folder, a missing one, a relative one), says which field,
/// and starts and makes nothing.
#[tokio::test]
async fn a_start_with_a_picked_folder_is_refused_and_starts_nothing() {
    let fixture = Fixture::new().await;
    let picked = fixture.world.root.join("my-notes");
    std::fs::create_dir_all(&picked).unwrap();
    std::fs::write(picked.join("todo.md"), "- ship\n").unwrap();
    for path in [
        picked.display().to_string(),
        fixture.world.root.join("missing").display().to_string(),
        "relative/path".to_owned(),
    ] {
        let mut request = fixture.request("Notes");
        request.picked_path = path.clone();
        let status = fixture.try_start(request).await.unwrap_err();
        let (field, problem) = common::invalid_field(&status);
        assert_eq!(field, "picked_path", "{path}");
        assert_eq!(problem, pb::invalid_field::Problem::NotAllowedHere, "{path}");
    }
    assert!(fixture.listed().await.is_empty());
    assert!(!fixture.desk_root().join("notes").exists());
    assert!(!picked.join(".git").exists(), "the picked folder was not touched");
}

/// D460: the same start without the folder is placed by the roots: a scratch
/// folder under the configured channels root's desk folder.
#[tokio::test]
async fn a_new_start_is_placed_under_the_configured_root() {
    let fixture = Fixture::new().await;
    let (id, path) = fixture.start("Notes").await;
    assert_eq!(path, fixture.desk_root().join("notes"));
    let desk = fixture.find(&id).await.unwrap();
    assert!(scratch_of(&desk).is_some(), "{desk:?}");
}

/// D458: a session an older release started in a picked folder is still
/// listed as a picked one, and closing it leaves the folder.
#[tokio::test]
async fn an_existing_picked_session_is_still_listed_and_closes_in_place() {
    let fixture = Fixture::new().await;
    let picked = fixture.world.root.join("my-notes");
    std::fs::create_dir_all(&picked).unwrap();
    std::fs::write(picked.join("todo.md"), "- ship\n").unwrap();
    let id = fixture.old_picked_session("Notes", &picked).await;
    let desk = fixture.find(&id).await.expect("the old session is listed");
    assert!(scratch_of(&desk).is_none(), "{desk:?}");
    assert_eq!(PathBuf::from(desk.folder.unwrap().path).canonicalize().unwrap(), picked.canonicalize().unwrap());
    let closed = fixture.close(&id).await;
    assert!(closed.closed_at.is_some(), "{closed:?}");
    assert!(picked.join("todo.md").exists());
}

#[tokio::test]
async fn another_machine_is_refused_and_starts_nothing() {
    let fixture = Fixture::new().await;
    let mut elsewhere = fixture.request("Elsewhere");
    elsewhere.machine = Some(pb::MachineId {
        value: "ma_somewhere_else".to_owned(),
    });
    let status = fixture.try_start(elsewhere).await.unwrap_err();
    let (field, problem) = common::invalid_field(&status);
    assert_eq!(field, "machine");
    assert_eq!(problem, pb::invalid_field::Problem::UnknownValue);

    // This machine named explicitly is fine.
    let mut here = fixture.request("Here");
    here.machine = Some(machine_id(&fixture.daemon).await);
    fixture.start_with(here).await;

    assert_eq!(fixture.listed().await.len(), 1);
    assert!(!fixture.desk_root().join("elsewhere").exists());
}

// ----- Repos (DK-3). -----

#[tokio::test]
async fn a_start_with_repos_clones_each_into_the_folder() {
    let fixture = Fixture::new().await;
    fixture.world.remote("acme", "api");
    fixture.world.remote("acme", "web");
    let mut request = fixture.request("With repos");
    request.repos = vec![repo_ref("acme", "web"), repo_ref("acme", "api")];
    let (id, path) = fixture.start_with(request).await;
    assert_eq!(std::fs::read_to_string(path.join("api/README.md")).unwrap(), "acme/api\n");
    assert_eq!(std::fs::read_to_string(path.join("web/README.md")).unwrap(), "acme/web\n");
    let desk = fixture.find(&id).await.unwrap();
    assert_eq!(desk.repos, vec![repo_ref("acme", "api"), repo_ref("acme", "web")]);
}

#[tokio::test]
async fn a_start_whose_clone_fails_starts_nothing_and_leaves_no_folder() {
    let fixture = Fixture::new().await;
    let mut request = fixture.request("Missing repo");
    request.repos = vec![repo_ref("acme", "nowhere")];
    let status = fixture.try_start(request).await.unwrap_err();
    assert_ne!(status.code(), Code::Ok);
    assert!(fixture.listed().await.is_empty());
    assert!(!fixture.desk_root().join("missing-repo").exists());
}

#[tokio::test]
async fn an_added_repo_is_cloned_and_listed_and_a_second_add_changes_nothing() {
    let fixture = Fixture::new().await;
    fixture.world.remote("acme", "api");
    let (id, path) = fixture.start("Plan Q4").await;
    let desk = fixture.add_repo(&id, "acme", "api").await.unwrap();
    assert_eq!(desk.repos, vec![repo_ref("acme", "api")]);
    assert!(path.join("api/README.md").exists());
    std::fs::write(path.join("api/wip.txt"), "wip\n").unwrap();
    assert!(fixture.add_repo(&id, "acme", "api").await.is_err());
    assert!(path.join("api/wip.txt").exists(), "the clone that was there stays");
}

#[tokio::test]
async fn a_closed_session_takes_no_repo() {
    let fixture = Fixture::new().await;
    fixture.world.remote("acme", "api");
    let (id, path) = fixture.start("Plan Q4").await;
    fixture.close(&id).await;
    let status = fixture.add_repo(&id, "acme", "api").await.unwrap_err();
    assert_eq!(status.code(), Code::FailedPrecondition, "{status:?}");
    assert!(!path.join("api").exists());
}

#[tokio::test]
async fn removing_a_clean_repo_puts_its_clone_in_the_trash() {
    let fixture = Fixture::new().await;
    fixture.world.remote("acme", "api");
    let (id, path) = fixture.start("Plan Q4").await;
    fixture.add_repo(&id, "acme", "api").await.unwrap();
    let desk = fixture.remove_repo(&id, "acme", "api").await.unwrap();
    assert!(desk.repos.is_empty(), "{desk:?}");
    assert!(!path.join("api").exists());
    assert!(path.exists(), "the session's folder stays");
    assert_eq!(fixture.trash().await.len(), 1);
}

#[tokio::test]
async fn removing_a_repo_with_work_is_refused_and_keeps_the_clone() {
    let fixture = Fixture::new().await;
    fixture.world.remote("acme", "api");
    let (id, path) = fixture.start("Plan Q4").await;
    fixture.add_repo(&id, "acme", "api").await.unwrap();
    std::fs::write(path.join("api/wip.txt"), "wip\n").unwrap();
    let status = fixture.remove_repo(&id, "acme", "api").await.unwrap_err();
    assert_eq!(status.code(), Code::FailedPrecondition, "{status:?}");
    assert!(path.join("api/wip.txt").exists());
    assert!(fixture.trash().await.is_empty());
}

// ----- Documents and Copy to a channel (DK-5, DK-13). -----

#[tokio::test]
async fn markdown_in_the_folder_is_listed_and_copies_into_a_channels_shared_folder() {
    let fixture = Fixture::new().await;
    let (channel, _) = common::create_channel(&fixture.daemon, "Payments", &[]).await;
    let (id, path) = fixture.start("Plan Q4").await;
    std::fs::write(path.join("rfc-brin.md"), "# Brin\n").unwrap();
    std::fs::write(path.join("notes.txt"), "not markdown\n").unwrap();
    assert_eq!(fixture.find(&id).await.unwrap().documents, vec!["rfc-brin.md".to_owned()]);

    fixture.copy(&id, "rfc-brin.md", &channel).await.unwrap();
    let shared = fixture.world.channels.join(&channel.folder).join("shared");
    assert_eq!(std::fs::read_to_string(shared.join("rfc-brin.md")).unwrap(), "# Brin\n");
    assert!(path.join("rfc-brin.md").exists(), "the Desk's copy stays");
}

#[tokio::test]
async fn a_copy_never_overwrites_the_channels_file() {
    let fixture = Fixture::new().await;
    let (channel, _) = common::create_channel(&fixture.daemon, "Payments", &[]).await;
    let (id, path) = fixture.start("Plan Q4").await;
    std::fs::write(path.join("rfc-brin.md"), "# Desk\n").unwrap();
    let shared = fixture.world.channels.join(&channel.folder).join("shared");
    std::fs::create_dir_all(&shared).unwrap();
    std::fs::write(shared.join("rfc-brin.md"), "# Channel\n").unwrap();
    assert!(fixture.copy(&id, "rfc-brin.md", &channel).await.is_err());
    assert_eq!(std::fs::read_to_string(shared.join("rfc-brin.md")).unwrap(), "# Channel\n");
}

#[tokio::test]
async fn a_copy_takes_only_a_file_inside_the_session_folder() {
    let fixture = Fixture::new().await;
    let (channel, _) = common::create_channel(&fixture.daemon, "Payments", &[]).await;
    let (id, _) = fixture.start("Plan Q4").await;
    std::fs::write(fixture.world.root.join("secret.md"), "# no\n").unwrap();
    for path in ["../secret.md", "/etc/hosts", "missing.md"] {
        assert!(fixture.copy(&id, path, &channel).await.is_err(), "{path}");
    }
    let shared = fixture.world.channels.join(&channel.folder).join("shared");
    assert!(!shared.join("secret.md").exists());
    assert!(!shared.join("hosts").exists());
}

#[tokio::test]
async fn a_channel_named_desk_can_not_take_the_desk_folder() {
    let fixture = Fixture::new().await;
    let status = fixture
        .daemon
        .channel_service()
        .await
        .create_channel(pb::CreateChannelRequest {
            idempotency_key: None,
            name: "Desk".to_owned(),
            repos: Vec::new(),
            machines: Vec::new(),
        })
        .await
        .unwrap_err();
    assert_eq!(status.code(), Code::InvalidArgument, "{status:?}");
}

// ----- Close, reopen, keep. -----

#[tokio::test]
async fn closing_stops_the_agent_and_keeps_the_folder_for_seven_days() {
    let fixture = Fixture::new().await;
    let (id, path) = fixture.start("Plan Q4").await;
    std::fs::write(path.join("draft.md"), "# Q4\n").unwrap();
    let closed = fixture.close(&id).await;
    let closed_at = unix_ms(&closed.closed_at).expect("closed has a time");
    let delete_at = unix_ms(&closed.delete_at).expect("a scratch folder has a delete time");
    assert_eq!(delete_at - closed_at, 600_000, "the test build's expiry");
    assert!(path.join("draft.md").exists());
    let session = get_session(&fixture.daemon, &id).await;
    assert_ne!(session.status, Status::Working as i32, "{session:?}");
    // Still listed, as closed.
    let listed = fixture.find(&id).await.expect("a closed session stays listed");
    assert!(listed.closed_at.is_some());
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

#[tokio::test]
async fn closing_ends_a_shell_whose_holder_is_stopped_and_does_not_wait_for_it() {
    let fixture = Fixture::new().await;
    let (id, _) = fixture.start("Plan Q4").await;
    let shell = fixture
        .daemon
        .terminals()
        .await
        .open_shell_tab(pb::OpenShellTabRequest {
            idempotency_key: None,
            session: Some(id.clone()),
            repo: None,
            folder: String::new(),
        })
        .await
        .unwrap()
        .into_inner()
        .id
        .unwrap();
    let holder = stored_tabs(&fixture.daemon.state)
        .iter()
        .find(|tab| tab.id == shell.value)
        .expect("the shell tab is stored")
        .holder_pid;
    let pid = i32::try_from(holder).ok().and_then(Pid::from_raw).expect("a holder pid");
    kill_process(pid, Signal::STOP).expect("the test can stop its own holder");
    let _resumed = Resumed(holder);

    tokio::time::timeout(CLOSE_GUARD, fixture.close(&id))
        .await
        .expect("the close finished within the guard, though the shell's holder never answered");

    assert!(!running(holder), "the shell's holder was ended by its pid");
    assert!(
        stored_tabs(&fixture.daemon.state)
            .iter()
            .all(|tab| tab.id != shell.value),
        "the closed shell is forgotten"
    );
}

#[tokio::test]
async fn a_closed_session_refuses_resume_and_new_tabs() {
    let fixture = Fixture::new().await;
    let (id, _) = fixture.start("Plan Q4").await;
    fixture.close(&id).await;
    let resume = fixture
        .daemon
        .session_service()
        .await
        .resume_session(pb::ResumeSessionRequest {
            id: Some(id.clone()),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert_eq!(resume.code(), Code::FailedPrecondition, "{resume:?}");
    let shell = fixture
        .daemon
        .terminals()
        .await
        .open_shell_tab(pb::OpenShellTabRequest {
            idempotency_key: None,
            session: Some(id.clone()),
            repo: None,
            folder: String::new(),
        })
        .await
        .unwrap_err();
    assert_eq!(shell.code(), Code::FailedPrecondition, "{shell:?}");
}

#[tokio::test]
async fn reopening_resumes_the_conversation_in_the_same_folder() {
    let fixture = Fixture::new().await;
    let (id, path) = fixture.start("Plan Q4").await;
    drop(say(&fixture.daemon, &id, "before closing").await);
    fixture.close(&id).await;
    fixture.reopen(&id).await.unwrap();
    wait_status(&fixture.daemon, &id, Status::Working).await;
    let tab = agent_tab(&fixture.daemon, &id).await;
    let mut attach = Attach::start(&fixture.daemon, open(&tab, 0)).await;
    attach.wait_for("agent up --resume").await;
    let desk = fixture.find(&id).await.unwrap();
    assert!(desk.closed_at.is_none());
    assert!(desk.delete_at.is_none());
    assert_eq!(PathBuf::from(desk.folder.unwrap().path), path);
}

#[tokio::test]
async fn reopening_is_refused_when_the_folder_is_gone() {
    let fixture = Fixture::new().await;
    let (id, path) = fixture.start("Plan Q4").await;
    fixture.close(&id).await;
    std::fs::remove_dir_all(&path).unwrap();
    let status = fixture.reopen(&id).await.unwrap_err();
    assert_eq!(status.code(), Code::FailedPrecondition, "{status:?}");
}

#[tokio::test]
async fn the_channel_actions_refuse_a_desk_session() {
    let fixture = Fixture::new().await;
    let (id, _) = fixture.start("Plan Q4").await;
    let status = fixture
        .daemon
        .session_service()
        .await
        .plan_archive(pb::PlanArchiveRequest {
            id: Some(id.clone()),
        })
        .await
        .unwrap_err();
    assert_eq!(status.code(), Code::FailedPrecondition, "{status:?}");
    let status = fixture
        .daemon
        .session_service()
        .await
        .fork_session(pb::ForkSessionRequest {
            idempotency_key: None,
            parent: Some(id.clone()),
            kind: pb::ForkKind::CodeOnly as i32,
            before_turn: 0,
            title: "fork".to_owned(),
            agent: None,
            model: String::new(),
            machine: None,
            branch: String::new(),
            reason: String::new(),
        })
        .await
        .unwrap_err();
    assert_eq!(status.code(), Code::FailedPrecondition, "{status:?}");
}

#[tokio::test]
async fn keeping_a_folder_takes_away_its_delete_time() {
    let fixture = Fixture::new().await;
    let (id, _) = fixture.start("Plan Q4").await;
    fixture.close(&id).await;
    let kept = fixture.keep(&id).await;
    assert!(scratch_of(&kept).unwrap().kept);
    assert!(kept.delete_at.is_none());
}

#[tokio::test]
async fn the_access_switches_are_kept_across_a_restart() {
    let mut fixture = Fixture::new().await;
    let (id, _) = fixture.start("Plan Q4").await;
    let changed = fixture.set_access(&id, false, true).await;
    let access = changed.access.unwrap();
    assert!(!access.boards);
    assert!(access.channels_and_machines);
    fixture.daemon.restart().await;
    let access = fixture.find(&id).await.unwrap().access.unwrap();
    assert!(!access.boards);
    assert!(access.channels_and_machines);
}

// ----- The agent's access. -----

#[tokio::test]
async fn with_the_board_switch_on_the_agent_adds_to_the_global_board_without_asking() {
    let fixture = Fixture::new().await;
    let (id, _) = fixture.start("Plan Q4").await;
    let (token, _attach) = shell_token(&fixture.daemon, &id).await;
    let (code, _) = humata_as_agent(&fixture.daemon, &token, &["board", "--global", "add", "From the desk"]).await;
    assert_eq!(code, 0);
    let items = pb::board_service_client::BoardServiceClient::new(fixture.daemon.channel().await)
        .list_items(pb::ListItemsRequest {
            view: Some(pb::BoardView {
                view: Some(pb::board_view::View::Board(pb::BoardRef {
                    board: Some(pb::board_ref::Board::Global(pb::GlobalBoard {})),
                })),
            }),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner()
        .items;
    assert!(items.iter().any(|item| item.title == "From the desk"), "{items:?}");
}

#[tokio::test]
async fn the_agent_can_keep_its_own_folder_but_not_another() {
    let fixture = Fixture::new().await;
    let (mine, _) = fixture.start("Mine").await;
    let (other, _) = fixture.start("Other").await;
    let (token, _attach) = shell_token(&fixture.daemon, &mine).await;
    let (code, _) = humata_as_agent(&fixture.daemon, &token, &["desk", "keep"]).await;
    assert_eq!(code, 0);
    assert!(scratch_of(&fixture.find(&mine).await.unwrap()).unwrap().kept);
    let (code, _) = humata_as_agent(&fixture.daemon, &token, &["desk", "keep", &other.value]).await;
    assert_ne!(code, 0);
    assert!(!scratch_of(&fixture.find(&other).await.unwrap()).unwrap().kept);
}

// ----- The 7 days. -----

#[tokio::test]
async fn a_closed_scratch_folder_goes_to_the_trash_after_its_days() {
    let fixture = Fixture::expiring_after("300").await;
    let (id, path) = fixture.start("Plan Q4").await;
    std::fs::write(path.join("draft.md"), "# Q4\n").unwrap();
    fixture.close(&id).await;
    wait_until("the folder's delete", || async { !exists(&path) }).await;
    wait_until("the session leaving the Desk", || async { fixture.find(&id).await.is_none() }).await;
    assert!(!fixture.session_ids().await.contains(&id));
    assert_eq!(fixture.trash().await.len(), 1);
}

#[tokio::test]
async fn work_not_pushed_holds_the_folder_and_says_why() {
    let fixture = Fixture::expiring_after("300").await;
    let (id, path) = fixture.start("Plan Q4").await;
    let repo = path.join("scratch-repo");
    let repo_text = repo.display().to_string();
    git(&["init", "-q", "--initial-branch=main", &repo_text]);
    std::fs::write(repo.join("wip.txt"), "wip\n").unwrap();
    fixture.close(&id).await;
    wait_until("the work being named", || async {
        fixture
            .find(&id)
            .await
            .is_some_and(|desk| !desk.held_work.is_empty())
    })
    .await;
    assert!(repo.join("wip.txt").exists());
    assert!(fixture.find(&id).await.unwrap().delete_at.is_none());
}

#[tokio::test]
async fn kept_and_open_folders_stay_past_the_days() {
    let fixture = Fixture::expiring_after("300").await;
    let (kept, kept_path) = fixture.start("Kept").await;
    let (_open, open_path) = fixture.start("Open").await;
    let (gone, gone_path) = fixture.start("Gone").await;
    fixture.close(&kept).await;
    fixture.keep(&kept).await;
    fixture.close(&gone).await;
    // The unkept one going shows the sweep ran past the other two.
    wait_until("the unkept folder's delete", || async { !exists(&gone_path) }).await;
    wait_until("the unkept session leaving", || async { fixture.find(&gone).await.is_none() }).await;
    assert!(kept_path.exists());
    assert!(open_path.exists());
    assert_eq!(fixture.listed().await.len(), 2);
}

#[tokio::test]
async fn a_picked_folder_is_never_deleted() {
    let fixture = Fixture::expiring_after("300").await;
    let picked = fixture.world.root.join("my-notes");
    std::fs::create_dir_all(&picked).unwrap();
    std::fs::write(picked.join("todo.md"), "- ship\n").unwrap();
    let picked_id = fixture.old_picked_session("Notes", &picked).await;
    let (scratch_id, scratch_path) = fixture.start("Scratch").await;
    fixture.close(&picked_id).await;
    fixture.close(&scratch_id).await;
    wait_until("the scratch folder's delete", || async { !exists(&scratch_path) }).await;
    assert!(picked.join("todo.md").exists());
    let desk = fixture.find(&picked_id).await.expect("a closed picked session stays listed");
    assert!(desk.delete_at.is_none());
}

#[tokio::test]
async fn the_days_left_are_counted_across_a_restart() {
    let mut fixture = Fixture::expiring_after("1500").await;
    let (id, path) = fixture.start("Plan Q4").await;
    fixture.close(&id).await;
    fixture.daemon.restart_env(&[(EXPIRE, "1500")]).await;
    wait_until("the folder's delete after the restart", || async { !exists(&path) }).await;
}

// ----- Clean up. -----

#[tokio::test]
async fn clean_up_lists_closed_sessions_and_deletes_the_ticked_ones() {
    let fixture = Fixture::new().await;
    let (open_id, _) = fixture.start("Open").await;
    let (old, old_path) = fixture.start("Old").await;
    let (newer, newer_path) = fixture.start("Newer").await;
    std::fs::write(old_path.join("big.bin"), vec![7u8; 4096]).unwrap();
    fixture.close(&old).await;
    fixture.close(&newer).await;
    let plan = fixture.plan().await;
    let listed: Vec<_> = plan.rows.iter().filter_map(|row| row.session.clone()).collect();
    assert_eq!(listed, vec![old.clone(), newer.clone()], "least recently used first");
    assert!(!listed.contains(&open_id));
    let old_row = &plan.rows[0];
    assert!(old_row.size_bytes.unwrap_or(0) >= 4096, "{old_row:?}");

    match fixture.clean_up(&plan, &[&old]).await {
        pb::clean_up_desk_response::Result::Deleted(deleted) => {
            assert_eq!(deleted.sessions, 1);
            assert!(deleted.freed_bytes >= 4096);
            assert_eq!(deleted.trash_items.len(), 1);
        }
        other => panic!("expected a delete: {other:?}"),
    }
    assert!(!old_path.exists());
    assert!(newer_path.exists());
    assert!(fixture.find(&old).await.is_none());
    assert!(fixture.find(&newer).await.is_some());
    assert_eq!(fixture.trash().await.len(), 1);
}

#[tokio::test]
async fn a_plan_that_no_longer_holds_deletes_nothing_and_answers_the_new_list() {
    let fixture = Fixture::new().await;
    let (first, first_path) = fixture.start("First").await;
    let (second, _) = fixture.start("Second").await;
    fixture.close(&first).await;
    let plan = fixture.plan().await;
    fixture.close(&second).await;
    match fixture.clean_up(&plan, &[&first]).await {
        pb::clean_up_desk_response::Result::Changed(changed) => {
            assert_eq!(changed.rows.len(), 2, "{changed:?}");
        }
        other => panic!("expected the changed list: {other:?}"),
    }
    assert!(first_path.exists());
    assert!(fixture.find(&first).await.is_some());
}

#[tokio::test]
async fn a_retried_clean_up_answers_what_the_first_one_did() {
    let fixture = Fixture::new().await;
    let (id, path) = fixture.start("Old").await;
    fixture.close(&id).await;
    let plan = fixture.plan().await;
    let request = pb::CleanUpDeskRequest {
        idempotency_key: Some(pb::IdempotencyKey { value: "clean-up-once".into() }),
        plan: plan.token.clone(),
        delete: vec![id.clone()],
    };
    let mut answers = Vec::new();
    for _ in 0..2 {
        let answer = fixture
            .desk()
            .await
            .clean_up_desk(request.clone())
            .await
            .unwrap()
            .into_inner()
            .result
            .expect("clean up answers a result");
        answers.push(answer);
    }
    assert!(matches!(answers[0], pb::clean_up_desk_response::Result::Deleted(_)), "{answers:?}");
    assert_eq!(answers[0], answers[1]);
    assert!(!path.exists());
    assert_eq!(fixture.trash().await.len(), 1);
}

/// D440: Clean up moves a scratch folder to the Bin before it marks the
/// row deleted, so whoever hears the row go (the Desk event that follows
/// the commit) finds the folder gone. Killed the moment that mark is
/// committed, the daemon has already moved the folder.
#[tokio::test]
async fn a_scratch_folder_is_in_the_bin_before_its_row_is_marked_deleted() {
    let scratch = folder();
    let arm = scratch.path().join("arm");
    let arm_path = arm.to_str().expect("the test folder's path is UTF-8").to_owned();
    let mut fixture = Fixture::with_env(&[
        (EXPIRE, NEVER),
        ("HUMATA2_STOP_AT", "store.desk.put.committed"),
        ("HUMATA2_FAULT_ARM", arm_path.as_str()),
    ])
    .await;
    let (id, path) = fixture.start("Old").await;
    fixture.close(&id).await;
    let plan = fixture.plan().await;
    std::fs::write(&arm, b"").expect("the stop is armed");

    // The call is cut off with the daemon that was answering it.
    let _ = fixture
        .desk()
        .await
        .clean_up_desk(pb::CleanUpDeskRequest {
            idempotency_key: None,
            plan: plan.token.clone(),
            delete: vec![id.clone()],
        })
        .await;
    fixture.daemon.stopped(Duration::from_secs(60)).await;

    assert!(!path.exists(), "the folder went before the row was marked deleted");
}

#[tokio::test]
async fn cleaning_up_a_picked_session_leaves_its_folder() {
    let fixture = Fixture::new().await;
    let picked = fixture.world.root.join("my-notes");
    std::fs::create_dir_all(&picked).unwrap();
    std::fs::write(picked.join("todo.md"), "- ship\n").unwrap();
    let id = fixture.old_picked_session("Notes", &picked).await;
    fixture.close(&id).await;
    let plan = fixture.plan().await;
    assert_eq!(plan.rows.len(), 1);
    assert!(plan.rows[0].size_bytes.is_none());
    fixture.clean_up(&plan, &[&id]).await;
    assert!(picked.join("todo.md").exists());
    assert!(fixture.find(&id).await.is_none());
    assert!(fixture.trash().await.is_empty());
}

#[tokio::test]
async fn an_open_session_can_not_be_cleaned_up() {
    let fixture = Fixture::new().await;
    let (closed, _) = fixture.start("Closed").await;
    let (open_id, open_path) = fixture.start("Open").await;
    fixture.close(&closed).await;
    let plan = fixture.plan().await;
    let status = fixture
        .desk()
        .await
        .clean_up_desk(pb::CleanUpDeskRequest {
            idempotency_key: None,
            plan: plan.token.clone(),
            delete: vec![open_id.clone()],
        })
        .await
        .unwrap_err();
    assert_eq!(status.code(), Code::FailedPrecondition, "{status:?}");
    assert!(open_path.exists());
}
