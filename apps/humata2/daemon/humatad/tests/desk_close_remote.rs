//! A channel's Desk session on another machine closes as one here does
//! (DV-4, D452, CH-24, SAFE): a plain close leaves a folder that holds a
//! version where it is, and deleting the channel sends that folder to the
//! machine's Bin, as the folder's life ends with the channel. A host and one
//! box on the multi-machine rig; every wait is on the host's feeds.

mod common;
#[path = "common/remote_rig.rs"]
mod remote_rig;

use std::path::{Path, PathBuf};

use common::git;
use humata2_grpc::humata::v1 as pb;
use remote_rig::{Rig, is_live};
use tonic::transport::Channel;

const BOX: &str = "box-1";
const REPO: (&str, &str) = ("acme", "api");

async fn desks(rig: &Rig) -> pb::desk_service_client::DeskServiceClient<Channel> {
    pb::desk_service_client::DeskServiceClient::new(rig.channel().await)
}

/// A channel with a Desk session on the box whose folder holds a version.
async fn versioned_on_box(rig: &Rig) -> (pb::Channel, pb::SessionId, PathBuf) {
    let host = rig.host_id().await;
    let channel = rig.create_channel("Payments", &[REPO], &[host.clone()]).await;
    let target = rig.start_on(&channel, &host, "Ledger fix").await;
    let started = desks(rig)
        .await
        .start_desk_session(pb::StartDeskSessionRequest {
            idempotency_key: Some(pb::IdempotencyKey { value: "desk-watch".to_owned() }),
            title: "Watch remotely".to_owned(),
            machine: Some(Rig::box_id(BOX)),
            channel: channel.id.clone(),
            target: Some(target),
            ..Default::default()
        })
        .await
        .expect("the channel's Desk session starts on the box")
        .into_inner();
    if let Some(operation) = &started.operation {
        let ended = rig.ended(operation).await;
        assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    }
    let id = started.session.expect("a start names its session");
    rig.wait_session(&id, "live", is_live).await;
    let folder = PathBuf::from(started.desk.and_then(|desk| desk.folder).expect("a start says where it works").path);
    commit(&folder);
    (channel, id, folder)
}

/// What the agent does to save a version: a plain `git commit` in the folder.
fn commit(folder: &Path) {
    std::fs::write(folder.join("plan.md"), "one\n").unwrap();
    let dir = folder.display().to_string();
    git(&["-C", &dir, "add", "-A"]);
    git(&["-C", &dir, "-c", "core.hooksPath=/dev/null", "commit", "-q", "-m", "First"]);
}

fn bin(rig: &Rig) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(rig.the(BOX).channels().join(".trash"))
        .map(|dir| dir.flatten().map(|entry| entry.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    names.sort();
    names
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plain_close_of_a_channel_desk_on_a_box_keeps_a_folder_that_holds_a_version() {
    let rig = Rig::start(&[BOX], &[REPO]).await;
    let (_, id, folder) = versioned_on_box(&rig).await;
    let before = bin(&rig);

    desks(&rig)
        .await
        .close_desk_session(pb::CloseDeskSessionRequest { session: Some(id), ..Default::default() })
        .await
        .expect("the host closes it on the box");

    assert!(folder.join("plan.md").is_file(), "the box keeps the versioned folder");
    assert_eq!(bin(&rig), before, "the box's Bin took nothing");
}

#[tokio::test(flavor = "multi_thread")]
async fn deleting_a_channel_trashes_its_versioned_desk_folder_on_a_box() {
    let rig = Rig::start(&[BOX], &[REPO]).await;
    let (channel, _, folder) = versioned_on_box(&rig).await;
    let before = bin(&rig);
    let mut channels = pb::channel_service_client::ChannelServiceClient::new(rig.channel().await);

    let plan = channels
        .plan_delete_channel(pb::PlanDeleteChannelRequest { channel: channel.id.clone() })
        .await
        .expect("the host plans the delete")
        .into_inner();
    let deleted = channels
        .delete_channel(pb::DeleteChannelRequest {
            idempotency_key: None,
            channel: plan.channel.clone(),
            plan: plan.token.clone(),
            presence: None,
            delete_unpushed_work: true,
        })
        .await
        .expect("the host deletes the channel")
        .into_inner();
    let ended = rig.ended(deleted.operation.as_ref().expect("a delete has an operation")).await;

    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    assert!(!folder.exists(), "the box keeps no versioned folder of a deleted channel");
    assert_ne!(bin(&rig), before, "the box's Bin took the folder");
}
