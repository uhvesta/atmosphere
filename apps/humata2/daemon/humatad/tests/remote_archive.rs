//! Archive and restore a session that runs on another machine (MV-7..MV-9
//! on a box; D195): the box archives it and frees its clones there after
//! the undo window, the host lists it archived, and restoring can bring it
//! back on the box (Undo, in place) or on this Mac (a fresh clone of its
//! branch with the mirrored conversation).
//!
//! Boxes run the daemon as shipped, so their undo window is the real 10
//! seconds. Waits are on the host's feeds; a freed folder is waited for on
//! disk.

mod common;
#[path = "common/remote_rig.rs"]
mod remote_rig;

use std::path::Path;
use std::time::Duration;

use common::git;
use humata2_grpc::humata::v1 as pb;
use remote_rig::{LONG, Rig, is_live};
use tonic::transport::Channel;

const REPO: (&str, &str) = ("acme", "api");
const CLONE: &str = "sessions/cache-warmup/api";

async fn sessions(rig: &Rig) -> pb::session_service_client::SessionServiceClient<Channel> {
    pb::session_service_client::SessionServiceClient::new(rig.channel().await)
}

async fn say(rig: &Rig, id: &pb::SessionId, line: &str) {
    let tab = common::agent_tab(&rig.host, id).await;
    let open = pb::AttachOpen {
        session: Some(id.clone()),
        ..common::open(&tab, 0)
    };
    let mut attach = common::Attach::start(&rig.host, open).await;
    attach.wait_for("agent up").await;
    attach.type_text(&format!("{line}\n")).await;
    attach.wait_for(&format!("got: {line}")).await;
}

async fn archive(rig: &Rig, id: &pb::SessionId) -> pb::Operation {
    let plan = sessions(rig).await.plan_archive(pb::PlanArchiveRequest { id: Some(id.clone()) }).await.expect("the archive is planned").into_inner();
    let started = sessions(rig)
        .await
        .archive_session(pb::ArchiveSessionRequest {
            idempotency_key: None,
            id: Some(id.clone()),
            choice: pb::ArchiveChoice::Archive as i32,
            plan: plan.token,
        })
        .await
        .expect("the archive starts")
        .into_inner();
    rig.ended(&started).await
}

async fn restore(rig: &Rig, id: &pb::SessionId, on: Option<pb::MachineId>) -> pb::Operation {
    let started = sessions(rig)
        .await
        .restore_session(pb::RestoreSessionRequest { idempotency_key: None, id: Some(id.clone()), machine: on })
        .await
        .expect("the restore starts")
        .into_inner();
    rig.ended(&started).await
}

async fn listed(rig: &Rig) -> Vec<pb::SessionId> {
    sessions(rig)
        .await
        .list_sessions(pb::ListSessionsRequest::default())
        .await
        .unwrap()
        .into_inner()
        .sessions
        .into_iter()
        .filter_map(|session| session.id)
        .collect()
}

/// The box frees a folder by itself once the undo window ends; nothing on
/// the host's feeds tells of it.
async fn wait_gone(path: &Path) {
    let deadline = tokio::time::Instant::now() + LONG;
    while path.exists() {
        assert!(tokio::time::Instant::now() < deadline, "{} was never freed", path.display());
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// A live session on box-1 that has said something.
async fn on_box() -> (Rig, pb::Channel, pb::SessionId) {
    let rig = Rig::start(&["box-1"], &[REPO]).await;
    let (host, one) = (rig.host_id().await, Rig::box_id("box-1"));
    let channel = rig.create_channel("Payments", &[REPO], &[host, one.clone()]).await;
    let id = rig.start_on(&channel, &one, "Cache warmup").await;
    rig.wait_session(&id, "live", is_live).await;
    say(&rig, &id, "before the archive").await;
    (rig, channel, id)
}

/// MV-7 on a box: archived there, listed archived on the host, its agent
/// stopped and its clone freed on the box after the window.
#[tokio::test(flavor = "multi_thread")]
async fn archiving_on_box_1_frees_its_clones_there_and_lists_it_archived_on_the_host() {
    let (rig, channel, id) = on_box().await;

    let ended = archive(&rig, &id).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);

    let archived = rig.wait_session(&id, "archived", |s| s.archive.is_some()).await;
    assert_eq!(archived.machine, Some(Rig::box_id("box-1")));
    assert!(!listed(&rig).await.contains(&id), "an archived session leaves the plain list");
    wait_gone(&rig.the("box-1").path(&channel, CLONE)).await;
    assert!(rig.the("box-1").tabs().is_empty(), "its agent stopped on box-1");
}

/// MV-8 elsewhere: box-1's archive restored on this Mac, after its folder
/// was freed, is a fresh clone of its branch with the conversation.
#[tokio::test(flavor = "multi_thread")]
async fn restoring_box_1s_archive_on_this_mac_clones_its_branch_and_resumes() {
    let (rig, channel, id) = on_box().await;
    let branch = rig.session(&id).await.repos[0].branch.clone();
    let clone = rig.the("box-1").path(&channel, CLONE);
    git(&["-C", &clone.display().to_string(), "push", "-u", "origin", &branch]);
    let ended = archive(&rig, &id).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    wait_gone(&clone).await;

    let host = rig.host_id().await;
    let ended = restore(&rig, &id, Some(host.clone())).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);

    let back = rig.wait_session(&id, "live on this Mac", |s| s.machine.as_ref() == Some(&host) && is_live(s)).await;
    assert!(back.archive.is_none());
    assert_eq!(back.repos[0].branch, branch);
    let here = rig.host_path(&channel, CLONE);
    assert_eq!(git(&["-C", &here.display().to_string(), "rev-parse", "--abbrev-ref", "HEAD"]), branch);
    say(&rig, &id, "back on this mac").await;
    let talk = std::fs::read_to_string(rig.host_conversation(&back.conversation_id)).unwrap();
    assert!(talk.contains("before the archive"), "the conversation came back: {talk}");
}

/// MV-8 Undo: restored inside the window, it comes back on box-1 in its
/// own folder, with what the folder held.
#[tokio::test(flavor = "multi_thread")]
async fn undo_inside_the_window_restores_it_on_box_1_in_place() {
    let (rig, channel, id) = on_box().await;
    let clone = rig.the("box-1").path(&channel, CLONE);
    // Not work (MV-9 would refuse a plain archive over that), and a fresh
    // clone wouldn't have it: it shows the clone itself came back.
    std::fs::write(clone.join(".git").join("humata-test-marker"), "kept through the archive\n").unwrap();
    let ended = archive(&rig, &id).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);

    let ended = restore(&rig, &id, None).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    let back = rig.wait_session(&id, "live on box-1", |s| s.machine.as_ref() == Some(&Rig::box_id("box-1")) && is_live(s)).await;
    assert!(back.archive.is_none());
    assert_eq!(std::fs::read_to_string(clone.join(".git").join("humata-test-marker")).unwrap(), "kept through the archive\n");
}
