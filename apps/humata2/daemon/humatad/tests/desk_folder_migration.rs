//! Desk folders made before DOCV (D458): one VIP Migrate item sets up a
//! repository in each, with no commit; folders that already have one post
//! nothing.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use common::{Build, Daemon, World, approve, folder, wait_status};
use humata2_grpc::humata::v1 as pb;

async fn start_desk_session(daemon: &Daemon) -> PathBuf {
    let started = pb::desk_service_client::DeskServiceClient::new(daemon.channel().await)
        .start_desk_session(pb::StartDeskSessionRequest {
            title: "Old notes".to_owned(),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    let id = started.session.unwrap();
    wait_status(daemon, &id, pb::SessionStatus::Working).await;
    PathBuf::from(started.desk.unwrap().folder.unwrap().path)
}

/// The Migrate items for the Desk folders that wait for the user.
async fn pending_desk_folder_items(daemon: &Daemon) -> Vec<pb::InboxRequest> {
    daemon
        .inbox_service()
        .await
        .list_requests(pb::ListRequestsRequest::default())
        .await
        .unwrap()
        .into_inner()
        .requests
        .into_iter()
        .filter(|request| request.state() == pb::RequestState::Pending)
        .filter(|request| match request.preview.as_ref().and_then(|preview| preview.preview.as_ref()) {
            Some(pb::request_preview::Preview::Migrate(card)) => {
                card.thing() == pb::MigrateThing::DeskFolders
            }
            _ => false,
        })
        .collect()
}

async fn wait_for_item(daemon: &Daemon) -> pb::InboxRequest {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(item) = pending_desk_folder_items(daemon).await.into_iter().next() {
            return item;
        }
        assert!(Instant::now() < deadline, "no Migrate item for the Desk folders reached the Inbox");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

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

async fn started() -> (Daemon, PathBuf) {
    let dir = folder();
    let world = World::new(dir.path());
    let args = world.args();
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let daemon = Daemon::start_with(Build::Admin, dir, &refs).await;
    let desk = start_desk_session(&daemon).await;
    (daemon, desk)
}

#[tokio::test]
async fn a_desk_folder_without_a_repository_gets_one_when_the_user_migrates() {
    let (mut daemon, desk) = started().await;
    daemon.kill();
    std::fs::remove_dir_all(desk.join(".git")).unwrap();
    daemon.restart().await;

    let item = wait_for_item(&daemon).await;
    assert!(item.vip, "a Migrate item is always VIP");
    assert_eq!(pending_desk_folder_items(&daemon).await.len(), 1);
    assert!(!desk.join(".git").exists(), "nothing changes before the user migrates");

    approve(&daemon, item.id.as_ref().unwrap()).await;

    let deadline = Instant::now() + Duration::from_secs(20);
    while !pending_desk_folder_items(&daemon).await.is_empty() {
        assert!(Instant::now() < deadline, "the item stayed pending after Migrate");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(desk.join(".git").is_dir(), "the folder became a repository");
    assert!(!has_commit(&desk), "migrating saves no version");
    daemon.restart().await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(pending_desk_folder_items(&daemon).await.is_empty(), "a migrated folder posts nothing");
}

#[tokio::test]
async fn desk_folders_that_already_have_a_repository_post_no_item() {
    let (mut daemon, desk) = started().await;
    assert!(desk.join(".git").is_dir());
    daemon.restart().await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(pending_desk_folder_items(&daemon).await.is_empty());
}
