//! A Desk folder's versions (DV-1, DV-4, DV-13, D451, D452) through a real
//! `humatad2`: every Desk folder Humata makes is a git repository of its own
//! with no version yet; the clones in it are left out of its versions; a
//! version keeps the folder past its days, and keeps a channel's Desk folder
//! in place when its session closes. Listing, reading and restoring versions
//! come with the docs service (DOCV phase B).

use humatad_test_common as common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use common::{Build, Daemon, World, create_channel, folder, git, repo_ref, start_session, wait_status};
use humata2_grpc::humata::v1 as pb;
use tonic::transport::Channel;

/// How long after closing a scratch folder goes, in admin test builds only.
const EXPIRE: &str = "HUMATA2_DESK_EXPIRE_MS";

struct Fixture {
    daemon: Daemon,
    world: World,
}

impl Fixture {
    async fn expiring_after(ms: &str) -> Self {
        let dir = folder();
        let world = World::new(dir.path());
        let args = world.args();
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let daemon = Daemon::start_with_env(Build::Admin, dir, &refs, &[(EXPIRE, ms)]).await;
        Self { daemon, world }
    }

    async fn desk(&self) -> pb::desk_service_client::DeskServiceClient<Channel> {
        pb::desk_service_client::DeskServiceClient::new(self.daemon.channel().await)
    }

    async fn start_with(&self, request: pb::StartDeskSessionRequest) -> (pb::SessionId, PathBuf) {
        let started = self
            .desk()
            .await
            .start_desk_session(request)
            .await
            .unwrap()
            .into_inner();
        let id = started.session.unwrap();
        wait_status(&self.daemon, &id, pb::SessionStatus::Working).await;
        let path = PathBuf::from(started.desk.unwrap().folder.unwrap().path);
        (id, path)
    }

    async fn start(&self, title: &str) -> (pb::SessionId, PathBuf) {
        self.start_with(pb::StartDeskSessionRequest {
            title: title.to_owned(),
            ..Default::default()
        })
        .await
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

    async fn find(&self, id: &pb::SessionId) -> Option<pb::DeskSession> {
        self.desk()
            .await
            .list_desk_sessions(pb::ListDeskSessionsRequest::default())
            .await
            .unwrap()
            .into_inner()
            .sessions
            .into_iter()
            .find(|desk| desk.session.as_ref() == Some(id))
    }

    async fn add_repo(&self, id: &pb::SessionId, owner: &str, name: &str) {
        self.desk()
            .await
            .add_desk_repo(pb::AddDeskRepoRequest {
                session: Some(id.clone()),
                repo: Some(repo_ref(owner, name)),
            })
            .await
            .unwrap();
    }

    async fn trash_items(&self) -> Vec<pb::TrashItem> {
        pb::trash_service_client::TrashServiceClient::new(self.daemon.channel().await)
            .list_trash(pb::ListTrashRequest::default())
            .await
            .unwrap()
            .into_inner()
            .items
    }
}

/// What the agent does to save a version: a plain `git commit` in the folder.
fn agent_commit(folder: &Path, file: &str, text: &str) {
    std::fs::write(folder.join(file), text).unwrap();
    let dir = folder.display().to_string();
    git(&["-C", &dir, "add", "-A"]);
    git(&["-C", &dir, "-c", "core.hooksPath=/dev/null", "commit", "-q", "-m", "Draft"]);
}

/// Whether the folder's own repository has a commit yet. Asks git about
/// the folder alone, never a repository above it.
fn has_commit(folder: &Path) -> bool {
    Command::new("git")
        .args(["rev-parse", "--verify", "-q", "HEAD"])
        .current_dir(folder)
        .env("GIT_CEILING_DIRECTORIES", folder.parent().unwrap())
        .output()
        .unwrap()
        .status
        .success()
}

/// What `git status` sees in the folder's own repository.
fn status(folder: &Path) -> String {
    let dir = folder.display().to_string();
    git(&["-C", &dir, "status", "--porcelain", "--untracked-files=all"])
}

fn kept(desk: &pb::DeskSession) -> bool {
    matches!(
        desk.folder.as_ref().and_then(|folder| folder.kind.as_ref()),
        Some(pb::desk_folder::Kind::Scratch(scratch)) if scratch.kept
    )
}

async fn wait_until<F: std::future::Future<Output = bool>>(what: &str, mut done: impl FnMut() -> F) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !done().await {
        assert!(Instant::now() < deadline, "{what} never happened");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn a_new_desk_folder_is_a_repository_with_no_version_yet() {
    let fixture = Fixture::expiring_after("600000").await;
    let (_id, path) = fixture.start("Plan Q4").await;
    assert!(path.join(".git").is_dir(), "the folder is a repository of its own");
    assert!(!has_commit(&path), "nothing is saved until someone saves it");
}

#[tokio::test]
async fn clones_in_the_folder_are_left_out_of_its_versions() {
    let fixture = Fixture::expiring_after("600000").await;
    fixture.world.remote("acme", "api");
    fixture.world.remote("acme", "web");
    let (id, path) = fixture
        .start_with(pb::StartDeskSessionRequest {
            title: "Plan Q4".to_owned(),
            repos: vec![repo_ref("acme", "api")],
            ..Default::default()
        })
        .await;
    fixture.add_repo(&id, "acme", "web").await;
    assert!(path.join("api/README.md").exists() && path.join("web/README.md").exists());
    assert_eq!(status(&path), "", "neither clone shows in the folder's own repository");
    std::fs::write(path.join("plan.md"), "# Q4\n").unwrap();
    assert_eq!(status(&path).trim(), "?? plan.md");
}

#[tokio::test]
async fn a_saved_version_keeps_the_folder_past_its_days() {
    let fixture = Fixture::expiring_after("300").await;
    let (saved, saved_path) = fixture.start("Saved").await;
    let (unsaved, unsaved_path) = fixture.start("Unsaved").await;
    agent_commit(&saved_path, "plan.md", "# Q4\n");
    std::fs::write(unsaved_path.join("plan.md"), "# typed, never saved\n").unwrap();
    let closed = fixture.close(&saved).await;
    assert!(kept(&closed), "a folder that holds a version is kept");
    assert!(closed.delete_at.is_none());
    fixture.close(&unsaved).await;
    // The unsaved one going shows the sweep ran past the saved one.
    wait_until("the unsaved folder's delete", || async { !unsaved_path.exists() }).await;
    wait_until("the unsaved session leaving", || async { fixture.find(&unsaved).await.is_none() }).await;
    assert!(saved_path.join("plan.md").exists());
    assert!(fixture.find(&saved).await.is_some_and(|desk| kept(&desk)));
}

#[tokio::test]
async fn a_version_saved_after_close_still_keeps_the_folder() {
    let fixture = Fixture::expiring_after("1500").await;
    let (id, path) = fixture.start("Plan Q4").await;
    fixture.close(&id).await;
    agent_commit(&path, "plan.md", "# saved by hand after close\n");
    let (other, other_path) = fixture.start("Other").await;
    fixture.close(&other).await;
    wait_until("the other folder's delete", || async { !other_path.exists() }).await;
    assert!(path.join("plan.md").exists(), "the sweep looks at the repository before deleting");
    assert!(fixture.find(&id).await.is_some_and(|desk| kept(&desk)));
}

#[tokio::test]
async fn a_channel_desk_folder_with_a_version_stays_when_its_session_closes() {
    let fixture = Fixture::expiring_after("600000").await;
    fixture.world.remote("acme", "api");
    let (channel, _) = create_channel(&fixture.daemon, "Payments", &[("acme", "api")]).await;
    let (target, _) = start_session(&fixture.daemon, &channel, "Ledger fix").await;
    wait_status(&fixture.daemon, &target, pb::SessionStatus::Working).await;
    let start = |title: &str| pb::StartDeskSessionRequest {
        title: title.to_owned(),
        channel: channel.id.clone(),
        target: Some(target.clone()),
        ..Default::default()
    };
    let (saved, saved_path) = fixture.start_with(start("Ledger RFC")).await;
    let (unsaved, unsaved_path) = fixture.start_with(start("Scratch notes")).await;
    assert!(saved_path.join(".git").is_dir(), "a channel's Desk folder is a repository too");
    agent_commit(&saved_path, "rfc.md", "# RFC\n");
    fixture.close(&saved).await;
    fixture.close(&unsaved).await;
    assert!(saved_path.join("rfc.md").exists(), "a folder with a version stays where it is");
    assert!(!unsaved_path.exists(), "a folder with no version goes to the trash as before");
    let was = |id: &pb::SessionId| pb::EntityRef {
        entity: Some(pb::entity_ref::Entity::Session(id.clone())),
    };
    let items = fixture.trash_items().await;
    assert!(items.iter().all(|item| item.was != Some(was(&saved))));
    assert!(items.iter().any(|item| item.was == Some(was(&unsaved))));
}

#[tokio::test]
async fn a_channel_desk_session_closed_in_place_reopens_in_its_folder() {
    let fixture = Fixture::expiring_after("600000").await;
    fixture.world.remote("acme", "api");
    let (channel, _) = create_channel(&fixture.daemon, "Payments", &[("acme", "api")]).await;
    let (target, _) = start_session(&fixture.daemon, &channel, "Ledger fix").await;
    wait_status(&fixture.daemon, &target, pb::SessionStatus::Working).await;
    let (id, path) = fixture
        .start_with(pb::StartDeskSessionRequest {
            title: "Ledger RFC".to_owned(),
            channel: channel.id.clone(),
            target: Some(target.clone()),
            ..Default::default()
        })
        .await;
    agent_commit(&path, "rfc.md", "# RFC\n");
    let closed = fixture.close(&id).await;
    assert!(closed.closed_at.is_some());
    assert!(kept(&closed), "closed in place, its folder is kept");

    let listed = |channel: &pb::Channel, target: &pb::SessionId| {
        let request = pb::ListDeskSessionsRequest {
            channel: channel.id.clone(),
            target: Some(target.clone()),
        };
        let fixture = &fixture;
        let id = id.clone();
        async move {
            fixture
                .desk()
                .await
                .list_desk_sessions(request)
                .await
                .unwrap()
                .into_inner()
                .sessions
                .into_iter()
                .find(|desk| desk.session.as_ref() == Some(&id))
        }
    };
    let shown = listed(&channel, &target).await.expect("a closed-in-place session is still listed");
    assert!(shown.closed_at.is_some() && kept(&shown));

    fixture
        .desk()
        .await
        .reopen_desk_session(pb::ReopenDeskSessionRequest {
            idempotency_key: None,
            session: Some(id.clone()),
        })
        .await
        .unwrap();
    wait_status(&fixture.daemon, &id, pb::SessionStatus::Working).await;
    let open = listed(&channel, &target).await.expect("the reopened session is listed");
    assert!(open.closed_at.is_none());
    assert_eq!(
        open.folder.map(|folder| folder.path),
        Some(path.display().to_string()),
        "it runs in the folder it was closed in"
    );
    assert!(path.join("rfc.md").exists());
}
