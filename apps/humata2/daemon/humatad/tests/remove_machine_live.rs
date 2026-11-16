//! Taking a machine out of a channel while sessions run on it (CH-8,
//! LD-10; D195): the plan lists each live session there with its work; the
//! removal handles them one at a time as the user chose (move to another
//! machine, or archive), and only then drops the machine. A failed move
//! keeps the machine in the channel and the session running.
//!
//! Waits are on the host's feeds (the rig's `wait_session`, `ended`).

mod common;
#[path = "common/remote_rig.rs"]
mod remote_rig;

use std::os::unix::fs::PermissionsExt;

use humata2_grpc::humata::v1 as pb;
use remote_rig::{Rig, is_live};
use tonic::transport::Channel;

const REPO: (&str, &str) = ("acme", "api");

async fn channels(rig: &Rig) -> pb::channel_service_client::ChannelServiceClient<Channel> {
    pb::channel_service_client::ChannelServiceClient::new(rig.channel().await)
}

async fn plan(rig: &Rig, channel: &pb::Channel, machine: &pb::MachineId) -> pb::RemoveChannelMachinePlan {
    channels(rig)
        .await
        .plan_remove_machine(pb::PlanRemoveChannelMachineRequest { channel: channel.id.clone(), machine: Some(machine.clone()) })
        .await
        .expect("the removal is planned")
        .into_inner()
}

async fn remove(rig: &Rig, channel: &pb::Channel, machine: &pb::MachineId, plan: &pb::RemoveChannelMachinePlan, dispositions: Vec<pb::SessionDisposition>) -> pb::Operation {
    let started = channels(rig)
        .await
        .remove_machine(pb::RemoveChannelMachineRequest {
            idempotency_key: None,
            channel: channel.id.clone(),
            machine: Some(machine.clone()),
            dispositions,
            plan: plan.token.clone(),
        })
        .await
        .expect("the removal starts")
        .into_inner();
    rig.ended(&started).await
}

fn move_to(session: &pb::SessionId, to: &pb::MachineId) -> pb::SessionDisposition {
    pb::SessionDisposition { session: Some(session.clone()), disposition: Some(pb::session_disposition::Disposition::MoveTo(to.clone())) }
}

fn archive(session: &pb::SessionId) -> pb::SessionDisposition {
    pb::SessionDisposition {
        session: Some(session.clone()),
        disposition: Some(pb::session_disposition::Disposition::Archive(pb::ArchiveChoice::ArchiveAndDeleteWork as i32)),
    }
}

async fn machines_of(rig: &Rig, channel: &pb::Channel) -> Vec<pb::MachineId> {
    channels(rig).await.get_channel(pb::GetChannelRequest { id: channel.id.clone() }).await.unwrap().into_inner().machines
}

/// Two sessions on box-1, one with an unpushed commit, in a channel on this
/// Mac, box-1 and box-2.
async fn two_on_box_1() -> (Rig, pb::Channel, pb::SessionId, pb::SessionId) {
    let rig = Rig::start(&["box-1", "box-2"], &[REPO]).await;
    let (one, two) = (Rig::box_id("box-1"), Rig::box_id("box-2"));
    let channel = rig.create_channel("Payments", &[REPO], &[rig.host_id().await, one.clone(), two]).await;
    let first = rig.start_on(&channel, &one, "Cache warmup").await;
    let second = rig.start_on(&channel, &one, "Ledger work").await;
    rig.wait_session(&first, "live", is_live).await;
    rig.wait_session(&second, "live", is_live).await;
    let clone = rig.the("box-1").path(&channel, "sessions/ledger-work/api");
    std::fs::write(clone.join("work.txt"), "unpushed\n").unwrap();
    common::git(&["-C", &clone.display().to_string(), "add", "work.txt"]);
    common::git(&["-C", &clone.display().to_string(), "commit", "-m", "unpushed work"]);
    rig.wait_session(&second, "showing its unpushed commit", |s| s.pointers.iter().any(|p| p.unpushed_commits == 1)).await;
    (rig, channel, first, second)
}

/// CH-8: box-1's live sessions are listed with their unpushed work.
#[tokio::test(flavor = "multi_thread")]
async fn removing_box_1_lists_its_live_sessions_with_their_unpushed_work() {
    let (rig, channel, first, second) = two_on_box_1().await;

    let plan = plan(&rig, &channel, &Rig::box_id("box-1")).await;

    let mut listed: Vec<_> = plan.sessions.iter().map(|s| (s.session.clone().unwrap(), s.unpushed_commits)).collect();
    listed.sort_by(|a, b| a.0.value.cmp(&b.0.value));
    let mut wanted = vec![(first, 0), (second, 1)];
    wanted.sort_by(|a, b| a.0.value.cmp(&b.0.value));
    assert_eq!(listed, wanted);
    assert!(plan.token.is_some());
}

/// LD-10: one moves to box-2, the other is archived, then box-1 leaves the
/// channel.
#[tokio::test(flavor = "multi_thread")]
async fn removing_box_1_moves_one_archives_the_other_then_drops_the_machine() {
    let (rig, channel, first, second) = two_on_box_1().await;
    let (one, two) = (Rig::box_id("box-1"), Rig::box_id("box-2"));
    let plan = plan(&rig, &channel, &one).await;

    let ended = remove(&rig, &channel, &one, &plan, vec![move_to(&second, &two), archive(&first)]).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);

    let moved = rig.wait_session(&second, "live on box-2", |s| s.machine.as_ref() == Some(&two) && is_live(s)).await;
    assert!(moved.archive.is_none());
    assert!(rig.session(&first).await.archive.is_some(), "the other is archived");
    assert!(!machines_of(&rig, &channel).await.contains(&one), "box-1 left the channel");
    assert!(rig.the("box-1").tabs().is_empty());
}

/// A move that fails keeps box-1 in the channel and the session running
/// there; what the removal already did stays done.
#[tokio::test(flavor = "multi_thread")]
async fn a_failed_move_keeps_box_1_in_the_channel_and_the_session_running() {
    let (rig, channel, first, second) = two_on_box_1().await;
    let (one, two) = (Rig::box_id("box-1"), Rig::box_id("box-2"));
    let plan = plan(&rig, &channel, &one).await;
    let sessions_on_two = rig.the("box-2").path(&channel, "sessions");
    std::fs::create_dir_all(&sessions_on_two).unwrap();
    std::fs::set_permissions(&sessions_on_two, std::fs::Permissions::from_mode(0o555)).unwrap();

    let ended = remove(&rig, &channel, &one, &plan, vec![move_to(&second, &two), archive(&first)]).await;
    std::fs::set_permissions(&sessions_on_two, std::fs::Permissions::from_mode(0o755)).unwrap();

    assert_eq!(ended.state, pb::OperationState::Failed as i32);
    let error = ended.error.expect("the failure says why");
    assert!(!error.message.is_empty() && !error.fix.is_empty(), "{error:?}");
    let stayed = rig.wait_session(&second, "live on box-1", is_live).await;
    assert_eq!(stayed.machine, Some(one.clone()));
    assert!(machines_of(&rig, &channel).await.contains(&one), "box-1 stays in the channel");
}
