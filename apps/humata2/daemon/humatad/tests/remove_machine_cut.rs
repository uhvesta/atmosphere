//! Taking a machine out of a channel, cut at each step (SAFE, CH-8, LD-10):
//! the host dies after the removal was written down, after each session was
//! handled, after all of them were, and after the machine was dropped. At
//! its next start the removal is finished: every session the user chose to
//! move or archive is moved or archived once, and the machine is out of the
//! channel. Nothing is left half done.
//!
//! Waits are on the host's feeds and on its process ending at the stop
//! point; nothing sleeps.

use humatad_test_common as common;
use humatad_test_common::remote_rig;

use std::time::Duration;

use humata2_grpc::humata::v1 as pb;
use remote_rig::{Rig, is_live};
use tonic::transport::Channel;

const REPO: (&str, &str) = ("acme", "api");
const LONG: Duration = Duration::from_secs(120);

/// Every step of a removal a cut can land on, as `HUMATA2_STOP_AT` names it.
const POINTS: &[&str] = &[
    "remove_machine.intent",
    "remove_machine.session_handled@1",
    "remove_machine.session_handled@2",
    "remove_machine.sessions_done",
    "remove_machine.dropped",
];

async fn channels(rig: &Rig) -> pb::channel_service_client::ChannelServiceClient<Channel> {
    pb::channel_service_client::ChannelServiceClient::new(rig.channel().await)
}

async fn machines_of(rig: &Rig, channel: &pb::Channel) -> Vec<pb::MachineId> {
    channels(rig)
        .await
        .get_channel(pb::GetChannelRequest { id: channel.id.clone() })
        .await
        .unwrap()
        .into_inner()
        .machines
}

/// Waits until `gone` is no longer a machine of `channel`. The channel is
/// read, and read again each time the Channels feed tells of a change; the
/// feed is opened before the first read, so no change falls between.
async fn wait_out_of_channel(rig: &Rig, channel: &pb::Channel, gone: &pb::MachineId) {
    let mut feed = pb::event_service_client::EventServiceClient::new(rig.channel().await)
        .subscribe(pb::SubscribeRequest {
            topics: vec![pb::Topic::Channels as i32],
            ..Default::default()
        })
        .await
        .expect("the host's Channels feed opens")
        .into_inner();
    let deadline = tokio::time::Instant::now() + LONG;
    loop {
        let now = machines_of(rig, channel).await;
        if !now.contains(gone) {
            return;
        }
        match tokio::time::timeout_at(deadline, feed.message()).await {
            Ok(Ok(Some(_))) => {}
            other => panic!("{gone:?} never left the channel ({other:?}); the channel still has {now:?}"),
        }
    }
}

fn notes(rig: &Rig) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(rig.world.channels.join(".removing-machine"))
        .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default()
}

fn not_moving(session: &pb::Session) -> bool {
    session.status != pb::SessionStatus::Moving as i32
}

/// Two sessions on box-1 in a channel on this Mac, box-1 and box-2; box-1 is
/// taken out under a host that dies at `point`, then the host starts again.
/// The user chose to move the second to box-2 and to archive the first, in
/// that order.
#[tokio::test(flavor = "multi_thread")]
async fn a_removal_cut_at_any_step_is_finished_at_the_next_start() {
    for &point in POINTS {
        let mut rig = Rig::start(&["box-1", "box-2"], &[REPO]).await;
        let (one, two) = (Rig::box_id("box-1"), Rig::box_id("box-2"));
        let channel = rig.create_channel("Payments", &[REPO], &[rig.host_id().await, one.clone(), two.clone()]).await;
        let first = rig.start_on(&channel, &one, "Cache warmup").await;
        let second = rig.start_on(&channel, &one, "Ledger work").await;
        rig.wait_session(&first, "live", is_live).await;
        rig.wait_session(&second, "live", is_live).await;

        // The plan lives in the host's memory, so the stop-point host makes its own.
        rig.restart_host(&[("HUMATA2_STOP_AT", point)]).await;
        let plan = channels(&rig)
            .await
            .plan_remove_machine(pb::PlanRemoveChannelMachineRequest {
                channel: channel.id.clone(),
                machine: Some(one.clone()),
            })
            .await
            .expect("the removal is planned")
            .into_inner();
        let dispositions = vec![
            pb::SessionDisposition {
                session: Some(second.clone()),
                disposition: Some(pb::session_disposition::Disposition::MoveTo(two.clone())),
            },
            pb::SessionDisposition {
                session: Some(first.clone()),
                disposition: Some(pb::session_disposition::Disposition::Archive(pb::ArchiveChoice::Archive as i32)),
            },
        ];
        // The host kills itself at the stop point, so the operation never ends.
        let _ = channels(&rig)
            .await
            .remove_machine(pb::RemoveChannelMachineRequest {
                idempotency_key: None,
                channel: channel.id.clone(),
                machine: Some(one.clone()),
                dispositions,
                plan: plan.token.clone(),
            })
            .await;
        rig.host_stopped().await;
        assert_eq!(notes(&rig).len(), 1, "cut at {point}: the removal was written down before any session was touched");

        rig.restart_host(&[]).await;

        wait_out_of_channel(&rig, &channel, &one).await;
        let moved = rig
            .wait_session(&second, "live on box-2", |s| s.machine.as_ref() == Some(&two) && is_live(s) && not_moving(s))
            .await;
        assert!(moved.archive.is_none(), "cut at {point}: {moved:?}");
        let archived = rig.wait_session(&first, "archived", |s| s.archive.is_some()).await;
        assert!(archived.archive.is_some(), "cut at {point}");
        assert!(machines_of(&rig, &channel).await.contains(&two), "cut at {point}: box-2 stays");
        assert!(rig.the("box-1").tabs().is_empty(), "cut at {point}: an agent still runs on box-1");
        assert_eq!(rig.the("box-2").tabs().len(), 1, "cut at {point}: the moved agent runs once on box-2");
    }
}
