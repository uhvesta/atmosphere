//! A Desk folder on another machine keeps its versions there (DV-1 to DV-8,
//! DV-4; SAFE): the host lists, reads, saves and restores them through the
//! machine's own Desk service, and closing such a Desk keeps a folder that
//! holds a version, as it does here. A host and one box on the multi-machine
//! rig; every wait is on the host's feeds.

mod common;
#[path = "common/remote_rig.rs"]
mod remote_rig;

use std::path::{Path, PathBuf};

use common::{error_of, git};
use humata2_grpc::humata::v1 as pb;
use remote_rig::{Rig, is_live};
use tonic::Status;
use tonic::transport::Channel;

const BOX: &str = "box-1";
const REPO: (&str, &str) = ("acme", "api");

async fn desks(rig: &Rig) -> pb::desk_service_client::DeskServiceClient<Channel> {
    pb::desk_service_client::DeskServiceClient::new(rig.channel().await)
}

/// A plain Desk session titled `title` on the box, live; and the folder it
/// works in there.
async fn on_box(rig: &Rig, title: &str) -> (pb::SessionId, PathBuf) {
    let started = desks(rig)
        .await
        .start_desk_session(pb::StartDeskSessionRequest {
            idempotency_key: Some(pb::IdempotencyKey { value: format!("desk-{title}") }),
            title: title.to_owned(),
            machine: Some(Rig::box_id(BOX)),
            access: Some(pb::DeskAccess { boards: true, channels_and_machines: false }),
            ..Default::default()
        })
        .await
        .expect("the Desk session starts on the box")
        .into_inner();
    if let Some(operation) = &started.operation {
        let ended = rig.ended(operation).await;
        assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    }
    let id = started.session.expect("a start names its session");
    rig.wait_session(&id, "live", is_live).await;
    let folder = started.desk.and_then(|desk| desk.folder).expect("a start says where it works").path;
    (id, PathBuf::from(folder))
}

/// What the agent does to save a version: a plain `git commit` in the folder.
fn agent_commit(folder: &Path, file: &str, text: &str, message: &str) {
    std::fs::write(folder.join(file), text).unwrap();
    let dir = folder.display().to_string();
    git(&["-C", &dir, "add", "-A"]);
    git(&["-C", &dir, "-c", "core.hooksPath=/dev/null", "commit", "-q", "-m", message]);
}

async fn list(rig: &Rig, id: &pb::SessionId, path: &str) -> Result<pb::ListDocVersionsResponse, Status> {
    desks(rig)
        .await
        .list_doc_versions(pb::ListDocVersionsRequest { session: Some(id.clone()), path: path.to_owned() })
        .await
        .map(tonic::Response::into_inner)
}

async fn shown(rig: &Rig, id: &pb::SessionId) -> pb::DeskSession {
    desks(rig)
        .await
        .list_desk_sessions(pb::ListDeskSessionsRequest::default())
        .await
        .expect("the host lists Desk sessions")
        .into_inner()
        .sessions
        .into_iter()
        .find(|desk| desk.session.as_ref() == Some(id))
        .expect("the host lists the session")
}

async fn close(rig: &Rig, id: &pb::SessionId) {
    desks(rig)
        .await
        .close_desk_session(pb::CloseDeskSessionRequest { idempotency_key: None, session: Some(id.clone()), ..Default::default() })
        .await
        .expect("the Desk session closes");
}

fn kept(desk: &pb::DeskSession) -> bool {
    matches!(
        desk.folder.as_ref().and_then(|folder| folder.kind.as_ref()),
        Some(pb::desk_folder::Kind::Scratch(pb::desk_folder::Scratch { kept: true }))
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn versions_of_a_desk_folder_on_a_box_are_listed_saved_read_and_restored_there() {
    let rig = Rig::start(&[BOX], &[REPO]).await;
    let (id, folder) = on_box(&rig, "Plan").await;
    agent_commit(&folder, "plan.md", "one\n", "First");

    let listed = list(&rig, &id, "plan.md").await.expect("the host lists the box's versions");
    assert_eq!(listed.versions.len(), 1);
    assert!(!listed.unsaved_changes);
    let first = listed.versions[0].id.clone();

    std::fs::write(folder.join("plan.md"), "two\n").unwrap();
    assert!(list(&rig, &id, "plan.md").await.unwrap().unsaved_changes, "the box says its folder has changes");
    desks(&rig)
        .await
        .save_doc_version(pb::SaveDocVersionRequest {
            idempotency_key: None,
            session: Some(id.clone()),
            message: "Second".to_owned(),
        })
        .await
        .expect("the host saves a version on the box");
    let listed = list(&rig, &id, "plan.md").await.unwrap();
    assert_eq!(listed.versions.len(), 2);
    assert!(!listed.unsaved_changes);

    let read = desks(&rig)
        .await
        .read_doc_version(pb::ReadDocVersionRequest {
            session: Some(id.clone()),
            path: "plan.md".to_owned(),
            version: first.clone(),
        })
        .await
        .expect("the host reads an old version from the box")
        .into_inner();
    assert_eq!(read.text, "one\n");

    desks(&rig)
        .await
        .restore_doc_version(pb::RestoreDocVersionRequest {
            idempotency_key: None,
            session: Some(id.clone()),
            path: "plan.md".to_owned(),
            version: first,
        })
        .await
        .expect("the box restores the old version");
    assert_eq!(std::fs::read_to_string(folder.join("plan.md")).unwrap(), "one\n");
    assert_eq!(list(&rig, &id, "plan.md").await.unwrap().versions.len(), 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_host_shows_that_a_desk_folder_on_a_box_has_versions() {
    let rig = Rig::start(&[BOX], &[REPO]).await;
    let (id, folder) = on_box(&rig, "Plan").await;
    agent_commit(&folder, "plan.md", "one\n", "First");
    let head = list(&rig, &id, "plan.md").await.unwrap().versions[0].id.clone();

    let desk = shown(&rig, &id).await;
    assert!(desk.has_versions);
    assert_eq!(desk.doc_head, head);
}

#[tokio::test(flavor = "multi_thread")]
async fn versions_of_a_box_that_is_offline_answer_machine_offline() {
    let rig = Rig::start(&[BOX], &[REPO]).await;
    let (id, _) = on_box(&rig, "Plan").await;
    rig.link_lost(BOX).await;

    let offline = list(&rig, &id, "plan.md").await.expect_err("the box is away");
    assert!(matches!(error_of(&offline).kind, Some(pb::error::Kind::MachineOffline(_))), "{offline:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn closing_a_desk_on_a_box_keeps_a_folder_that_holds_a_version() {
    let rig = Rig::start(&[BOX], &[REPO]).await;
    let (id, folder) = on_box(&rig, "Plan").await;
    agent_commit(&folder, "plan.md", "one\n", "First");
    assert!(!kept(&shown(&rig, &id).await), "an open scratch folder isn't kept");

    close(&rig, &id).await;
    let closed = shown(&rig, &id).await;
    assert!(kept(&closed), "the host's row follows the box: a version keeps the folder");
    assert!(closed.delete_at.is_none(), "a kept folder has no day it goes");
}

#[tokio::test(flavor = "multi_thread")]
async fn closing_a_desk_on_a_box_with_no_version_leaves_the_folder_to_go() {
    let rig = Rig::start(&[BOX], &[REPO]).await;
    let (id, _) = on_box(&rig, "Plan").await;

    close(&rig, &id).await;
    let closed = shown(&rig, &id).await;
    assert!(!kept(&closed));
}
