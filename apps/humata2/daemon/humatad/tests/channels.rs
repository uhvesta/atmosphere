//! ChannelService through a real `humatad2` (CH-1, CH-2, CH-5, FC-1): a new
//! channel gets its folders and a main clone of each repository, a repository
//! that can't be cloned leaves nothing half made, the CLI does the same, and
//! a restarted daemon still has the channel.

mod common;

use common::{World, create_channel, git, humata, repo_ref, start_world, wait_for_path};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{MainClone, Store};

fn id_of(channel: &pb::Channel) -> pb::ChannelId {
    channel.id.clone().unwrap()
}

async fn list(daemon: &common::Daemon) -> Vec<pb::Channel> {
    daemon
        .channel_service()
        .await
        .list_channels(pb::ListChannelsRequest::default())
        .await
        .unwrap()
        .into_inner()
        .channels
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_channel_gets_its_folders_and_a_main_clone() {
    let (daemon, world) = start_world().await;
    let remote = world.remote("acme", "api");

    let (channel, setup) = create_channel(&daemon, "Payments Rewrite", &[("acme", "api")]).await;

    assert_eq!(
        setup.state,
        pb::OperationState::Succeeded as i32,
        "{:?}",
        setup.error
    );
    assert_eq!(channel.folder, "payments-rewrite");
    let folder = world.channels.join(&channel.folder);
    for sub in ["main", "sessions", "shared"] {
        assert!(folder.join(sub).is_dir(), "{sub} is missing");
    }
    let clone = folder.join("main/api");
    assert!(clone.join(".git").exists());
    assert_eq!(
        git(&["-C", &clone.display().to_string(), "rev-parse", "HEAD"]),
        World::remote_head(&remote)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_channel_is_found_again_and_renaming_keeps_its_folder() {
    let (daemon, world) = start_world().await;
    world.remote("acme", "api");
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;

    let got = daemon
        .channel_service()
        .await
        .get_channel(pb::GetChannelRequest {
            id: Some(id_of(&channel)),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(got.id, channel.id);
    assert_eq!(got.repos, vec![repo_ref("acme", "api")]);
    assert!(
        list(&daemon)
            .await
            .iter()
            .any(|found| found.id == channel.id)
    );

    let renamed = daemon
        .channel_service()
        .await
        .rename_channel(pb::RenameChannelRequest {
            idempotency_key: None,
            id: Some(id_of(&channel)),
            name: "Billing".to_owned(),
            expected_revision: None,
        })
        .await
        .unwrap()
        .into_inner();

    assert_eq!(renamed.name, "Billing");
    assert_eq!(renamed.folder, channel.folder);
    assert!(
        world
            .channels
            .join(&channel.folder)
            .join("main/api")
            .is_dir()
    );
    let listed = list(&daemon).await;
    assert_eq!(
        listed
            .iter()
            .find(|found| found.id == channel.id)
            .unwrap()
            .name,
        "Billing"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_repository_that_does_not_exist_fails_the_setup_and_leaves_no_half_clone() {
    let (mut daemon, world) = start_world().await;

    let (channel, setup) = create_channel(&daemon, "Ghosts", &[("acme", "ghost")]).await;

    assert_eq!(setup.state, pb::OperationState::Failed as i32);
    assert!(setup.error.is_some());
    let step = setup
        .steps
        .iter()
        .find(|step| step.key == "clone_main:acme/ghost")
        .unwrap();
    assert_eq!(step.state, pb::StepState::Failed as i32);
    let main = world.channels.join(&channel.folder).join("main");
    assert!(main.is_dir());
    let left: Vec<_> = std::fs::read_dir(&main)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert!(left.is_empty(), "a clone was left in main/: {left:?}");
    // The channel is kept, with the repository marked as failed.
    daemon.kill();
    let record = Store::open(daemon.state.join("store"))
        .unwrap()
        .channel(&id_of(&channel).value)
        .unwrap()
        .unwrap();
    assert!(
        matches!(record.repos[0].main, MainClone::Failed { .. }),
        "{:?}",
        record.repos[0].main
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_cli_creates_the_same_channel() {
    let (daemon, world) = start_world().await;
    let remote = world.remote("acme", "api");

    let (code, json) = humata(
        &daemon,
        &["channel", "create", "CLI Channel", "--repo", "acme/api"],
    );

    assert_eq!(code, 0, "{json}");
    assert_eq!(json["ok"], true, "{json}");
    let channels = list(&daemon).await;
    let channel = channels
        .iter()
        .find(|channel| channel.name == "CLI Channel")
        .unwrap();
    assert_eq!(channel.folder, "cli-channel");
    let clone = world.channels.join("cli-channel/main/api");
    wait_for_path(&clone.join(".git")).await;
    assert_eq!(
        git(&["-C", &clone.display().to_string(), "rev-parse", "HEAD"]),
        World::remote_head(&remote)
    );
    for sub in ["sessions", "shared"] {
        assert!(world.channels.join("cli-channel").join(sub).is_dir());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_restarted_daemon_still_lists_the_channel() {
    let (mut daemon, world) = start_world().await;
    world.remote("acme", "api");
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;

    daemon.restart().await;

    let listed = list(&daemon).await;
    let found = listed.iter().find(|found| found.id == channel.id).unwrap();
    assert_eq!(found.folder, channel.folder);
    assert_eq!(found.repos, vec![repo_ref("acme", "api")]);
}
