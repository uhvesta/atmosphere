//! Planning the delete of a channel that another machine holds (CH-24,
//! D784; VERIFY 0.2.3). This humatad2 can't yet delete a channel that
//! another machine holds, so no plan for it may be offered. The plan is
//! refused up front, and the refusal names each live session there and
//! tells online machines from offline ones as they are. A host and one box
//! on the multi-machine rig; every wait is on the host's feeds.

mod common;
#[path = "common/remote_rig.rs"]
mod remote_rig;

use humata2_grpc::humata::v1 as pb;
use remote_rig::{Rig, is_live};
use tonic::Code;

const BOX: &str = "box-1";
const REPO: (&str, &str) = ("acme", "api");

/// A channel on this Mac and box-1, with session "Ledger work" live on box-1.
async fn live_on_box() -> (Rig, pb::Channel, pb::SessionId) {
    let rig = Rig::start(&[BOX], &[REPO]).await;
    let one = Rig::box_id(BOX);
    let channel = rig
        .create_channel("Payments", &[REPO], &[rig.host_id().await, one.clone()])
        .await;
    let session = rig.start_on(&channel, &one, "Ledger work").await;
    rig.wait_session(&session, "live", is_live).await;
    (rig, channel, session)
}

async fn plan(rig: &Rig, channel: &pb::Channel) -> Result<pb::DeleteChannelPlan, tonic::Status> {
    pb::channel_service_client::ChannelServiceClient::new(rig.channel().await)
        .plan_delete_channel(pb::PlanDeleteChannelRequest { channel: channel.id.clone() })
        .await
        .map(tonic::Response::into_inner)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_channel_with_a_live_session_on_an_online_box_gets_no_plan_apply_would_refuse() {
    let (rig, channel, session) = live_on_box().await;
    rig.linked(BOX).await;

    let refused = plan(&rig, &channel)
        .await
        .expect_err("no plan is offered for a delete this humatad2 can't apply");

    assert_eq!(refused.code(), Code::FailedPrecondition, "{refused:?}");
    let error = common::error_of(&refused);
    assert!(error.message.contains("Ledger work"), "the refusal names the live session: {error:?}");
    assert!(!error.message.contains("offline"), "box-1 is online and isn't called offline: {error:?}");
    assert!(!error.fix.is_empty(), "the refusal says what to do");
    assert!(is_live(&rig.session(&session).await), "planning changed nothing");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_channel_on_an_offline_box_gets_no_plan_and_the_refusal_says_the_box_is_offline() {
    let (rig, channel, _) = live_on_box().await;
    rig.link_lost(BOX).await;

    let refused = plan(&rig, &channel)
        .await
        .expect_err("no plan is offered for a delete this humatad2 can't apply");

    assert_eq!(refused.code(), Code::FailedPrecondition, "{refused:?}");
    let error = common::error_of(&refused);
    assert!(error.message.contains("Ledger work"), "the refusal names the session the host knows of: {error:?}");
    assert!(error.message.contains("offline"), "the refusal says box-1 is offline: {error:?}");
}
