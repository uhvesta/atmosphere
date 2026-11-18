//! Forget a machine that is gone for good (LD-11, LD-12, flow 26.23's first
//! half; D195): the plan lists every session on it with the work lost, from
//! the host's last pointers; forgetting archives those sessions from the
//! host's records, marked machine_lost with what was lost, and takes the
//! machine out of every channel. The host never dials it.
//!
//! Waits are on the host's feeds (the rig's `wait_session`).

mod common;
#[path = "common/remote_rig.rs"]
mod remote_rig;

use common::{error_of, git, shell_token};
use humata2_grpc::humata::v1 as pb;
use remote_rig::{Rig, is_live};
use tonic::Code;
use tonic::transport::Channel;

const REPO: (&str, &str) = ("acme", "api");

async fn machines(rig: &Rig) -> pb::machine_service_client::MachineServiceClient<Channel> {
    pb::machine_service_client::MachineServiceClient::new(rig.channel().await)
}

/// A session on `box_name` with one dirty file the host has heard of.
async fn with_work(rig: &Rig, channel: &pb::Channel, box_name: &str, title: &str, folder: &str) -> pb::SessionId {
    let id = rig.start_on(channel, &Rig::box_id(box_name), title).await;
    rig.wait_session(&id, "live", is_live).await;
    let clone = rig.the(box_name).path(channel, &format!("sessions/{folder}/api"));
    std::fs::write(clone.join("README.md"), "changed on the box\n").unwrap();
    git(&["-C", &clone.display().to_string(), "status", "--short"]);
    rig.wait_session(&id, "showing a dirty file", |s| s.pointers.iter().any(|p| p.dirty_files == 1)).await;
    id
}

/// box-1 with work, a session on this Mac, then box-1 gone for good.
async fn gone_box() -> (Rig, pb::Channel, pb::SessionId, pb::SessionId) {
    let mut rig = Rig::start(&["box-1"], &[REPO]).await;
    let (host, one) = (rig.host_id().await, Rig::box_id("box-1"));
    let channel = rig.create_channel("Payments", &[REPO], &[host.clone(), one.clone()]).await;
    let on_box = with_work(&rig, &channel, "box-1", "On the box", "on-the-box").await;
    let on_host = rig.start_on(&channel, &host, "On this mac").await;
    rig.wait_session(&on_host, "live", is_live).await;
    rig.the_mut("box-1").kill();
    rig.link_lost("box-1").await;
    (rig, channel, on_box, on_host)
}

async fn plan(rig: &Rig) -> pb::ForgetMachinePlan {
    machines(rig)
        .await
        .plan_forget_machine(pb::PlanForgetMachineRequest { id: Some(Rig::box_id("box-1")) })
        .await
        .expect("the plan is made")
        .into_inner()
}

fn forget(plan: &pb::ForgetMachinePlan, key: &str) -> pb::ForgetMachineRequest {
    pb::ForgetMachineRequest {
        idempotency_key: Some(pb::IdempotencyKey { value: key.to_owned() }),
        id: Some(Rig::box_id("box-1")),
        plan: plan.token.clone(),
        presence: None,
    }
}

/// LD-11: the plan names box-1's session and its lost work, nothing on this
/// Mac, and changes nothing.
#[tokio::test(flavor = "multi_thread")]
async fn the_plan_lists_each_session_on_the_gone_machine_with_its_lost_work() {
    let (rig, _channel, on_box, _on_host) = gone_box().await;

    let plan = plan(&rig).await;

    let lost: Vec<_> = plan.lost.iter().filter_map(|l| l.session.clone()).collect();
    assert_eq!(lost, [on_box.clone()]);
    assert_eq!(plan.lost[0].title, "On the box");
    assert_eq!(plan.lost[0].dirty_files, 1);
    assert!(plan.lost[0].pointers_at.is_some());
    assert!(plan.token.is_some());
    assert!(rig.session(&on_box).await.archive.is_none(), "planning changes nothing");
}

/// LD-11, SL-4: forgetting archives box-1's session as machine_lost with
/// its lost work, takes box-1 out of the channel and leaves this Mac's
/// session alone, through a host restart, after which the same key still
/// answers the same.
#[tokio::test(flavor = "multi_thread")]
async fn forgetting_archives_its_sessions_as_lost_and_takes_it_out_of_every_channel() {
    let (mut rig, channel, on_box, on_host) = gone_box().await;
    let plan = plan(&rig).await;

    let done = machines(&rig).await.forget_machine(forget(&plan, "forget-1")).await.expect("box-1 is forgotten").into_inner();
    assert_eq!(done.archived, [on_box.clone()]);

    rig.restart_host(&[]).await;
    let again = machines(&rig).await.forget_machine(forget(&plan, "forget-1")).await.expect("the key answers after a restart").into_inner();
    assert_eq!(again.archived, done.archived);
    let archived = rig.session(&on_box).await;
    let summary = archived.archive.expect("box-1's session is archived");
    assert!(summary.machine_lost);
    assert_eq!(summary.lost.len(), 1);
    assert_eq!(summary.lost[0].dirty_files, 1);
    assert!(rig.session(&on_host).await.archive.is_none(), "this Mac's session is untouched");
    let shown = pb::channel_service_client::ChannelServiceClient::new(rig.channel().await)
        .get_channel(pb::GetChannelRequest { id: channel.id.clone() })
        .await
        .unwrap()
        .into_inner();
    assert!(!shown.machines.contains(&Rig::box_id("box-1")), "box-1 is still in the channel: {:?}", shown.machines);
    // LD-12: this host no longer knows box-1 at all.
    let listed = machines(&rig).await.list_machines(pb::ListMachinesRequest::default()).await.unwrap().into_inner();
    let ids: Vec<_> = listed.machines.iter().filter_map(|machine| machine.id.clone()).collect();
    assert!(!ids.contains(&Rig::box_id("box-1")), "box-1 is still a machine here: {ids:?}");
}

/// D760: removing a machine from Hosts takes it out of every channel's
/// machine list too, so it isn't shown anywhere.
#[tokio::test(flavor = "multi_thread")]
async fn removing_a_machine_takes_it_out_of_every_channel() {
    let rig = Rig::start(&["box-1"], &[REPO]).await;
    let (host, one) = (rig.host_id().await, Rig::box_id("box-1"));
    let first = rig.create_channel("Payments", &[REPO], &[host.clone(), one.clone()]).await;
    let second = rig.create_channel("Ledger", &[REPO], &[host.clone(), one.clone()]).await;

    let plan = machines(&rig)
        .await
        .plan_remove_machine(pb::PlanRemoveMachineRequest { id: Some(one.clone()) })
        .await
        .expect("the removal is planned")
        .into_inner();
    let started = machines(&rig)
        .await
        .remove_machine(pb::RemoveMachineRequest { id: Some(one.clone()), plan: plan.token, ..Default::default() })
        .await
        .expect("the removal starts")
        .into_inner();
    let ended = rig.ended(&started).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);

    for channel in channels_of(&rig, [&first.id, &second.id]).await {
        assert!(!channel.machines.contains(&one), "box-1 is still in {}: {:?}", channel.name, channel.machines);
        assert!(channel.machines.contains(&host), "this Mac stays in {}", channel.name);
    }
}

/// A removal that fails (the machine refuses every dial, so nothing on it
/// can be uninstalled) leaves the machine listed and in every channel it
/// was in (D760).
#[tokio::test(flavor = "multi_thread")]
async fn a_removal_that_fails_leaves_the_machine_listed_and_in_its_channels() {
    let mut rig = Rig::start(&["box-1"], &[REPO]).await;
    let (host, one) = (rig.host_id().await, Rig::box_id("box-1"));
    let first = rig.create_channel("Payments", &[REPO], &[host.clone(), one.clone()]).await;
    let second = rig.create_channel("Ledger", &[REPO], &[host.clone(), one.clone()]).await;
    rig.the_mut("box-1").kill();
    rig.link_lost("box-1").await;
    rig.the("box-1").refuse_dials();

    let plan = machines(&rig)
        .await
        .plan_remove_machine(pb::PlanRemoveMachineRequest { id: Some(one.clone()) })
        .await
        .expect("the removal is planned")
        .into_inner();
    let started = machines(&rig)
        .await
        .remove_machine(pb::RemoveMachineRequest { id: Some(one.clone()), plan: plan.token, ..Default::default() })
        .await
        .expect("the removal starts")
        .into_inner();
    let ended = rig.ended(&started).await;
    assert_eq!(ended.state, pb::OperationState::Failed as i32, "a machine that refuses every dial can't be removed");

    let listed = machines(&rig)
        .await
        .list_machines(pb::ListMachinesRequest::default())
        .await
        .unwrap()
        .into_inner()
        .machines;
    assert!(listed.iter().any(|machine| machine.name == "box-1"), "box-1 is still listed: {listed:?}");
    for channel in channels_of(&rig, [&first.id, &second.id]).await {
        assert!(channel.machines.contains(&one), "box-1 left {} though its removal failed", channel.name);
        assert!(channel.machines.contains(&host), "this Mac stays in {}", channel.name);
    }
}

/// The channels `ids`, as the host lists them now.
async fn channels_of<const N: usize>(rig: &Rig, ids: [&Option<pb::ChannelId>; N]) -> Vec<pb::Channel> {
    let listed = pb::channel_service_client::ChannelServiceClient::new(rig.channel().await)
        .list_channels(pb::ListChannelsRequest::default())
        .await
        .unwrap()
        .into_inner()
        .channels;
    ids.iter()
        .map(|id| listed.iter().find(|channel| &channel.id == *id).cloned().expect("the channel is still listed"))
        .collect()
}

/// The same key answers the same; a used plan with a new key is refused;
/// an agent can't forget a machine.
#[tokio::test(flavor = "multi_thread")]
async fn forget_is_safe_to_repeat_and_an_agent_cannot_forget() {
    let (rig, _channel, on_box, on_host) = gone_box().await;
    let plan = plan(&rig).await;

    let (token, _shell) = shell_token(&rig.host, &on_host).await;
    let mut asked = tonic::Request::new(forget(&plan, "forget-agent"));
    asked.metadata_mut().insert("humata-session-token", tonic::metadata::MetadataValue::try_from(token.as_str()).unwrap());
    let refused = machines(&rig).await.forget_machine(asked).await.expect_err("an agent can't forget a machine");
    assert_eq!(refused.code(), Code::PermissionDenied);
    assert!(matches!(error_of(&refused).kind, Some(pb::error::Kind::UserOnly(_))), "{refused:?}");
    assert!(rig.session(&on_box).await.archive.is_none());

    let first = machines(&rig).await.forget_machine(forget(&plan, "forget-1")).await.unwrap().into_inner();
    let again = machines(&rig).await.forget_machine(forget(&plan, "forget-1")).await.unwrap().into_inner();
    assert_eq!(first.archived, again.archived);
    let used = machines(&rig).await.forget_machine(forget(&plan, "forget-2")).await.expect_err("a plan is good once");
    assert_eq!(used.code(), Code::FailedPrecondition, "{used:?}");
}

/// A host killed after the sessions were archived and before the answer was
/// kept answers a retry with the same key and plan as the first call would
/// have: box-1 is forgotten and its session archived once.
#[tokio::test(flavor = "multi_thread")]
async fn a_forget_cut_short_after_archiving_is_finished_by_a_retry() {
    let (mut rig, _channel, on_box, on_host) = gone_box().await;
    // The plan lives in the host's memory, so the stop-point host makes its own.
    rig.restart_host(&[("HUMATA2_STOP_AT", "forget.archived")]).await;
    let plan = plan(&rig).await;
    let _ = machines(&rig).await.forget_machine(forget(&plan, "forget-1")).await;
    rig.host_stopped().await;
    rig.restart_host(&[]).await;

    let done = machines(&rig).await.forget_machine(forget(&plan, "forget-1")).await.expect("the retry finishes the forget").into_inner();
    assert_eq!(done.archived, [on_box.clone()]);
    let summary = rig.session(&on_box).await.archive.expect("box-1's session is archived");
    assert!(summary.machine_lost);
    assert!(rig.session(&on_host).await.archive.is_none(), "this Mac's session is untouched");
    let listed = machines(&rig).await.list_machines(pb::ListMachinesRequest::default()).await.unwrap().into_inner();
    let ids: Vec<_> = listed.machines.iter().filter_map(|machine| machine.id.clone()).collect();
    assert!(!ids.contains(&Rig::box_id("box-1")), "box-1 is still a machine here: {ids:?}");
    let again = machines(&rig).await.forget_machine(forget(&plan, "forget-1")).await.unwrap().into_inner();
    assert_eq!(again.archived, done.archived);
}

/// Every step a forget takes, as the points a cut can land on: the forget
/// kept, a session's archive row saved (the store's points, either side of
/// the commit), the sessions archived, and the machine let go of.
const FORGET_POINTS: &[&str] = &[
    "forget.begun",
    "store.archive.put.committing@1",
    "store.archive.put.committed@1",
    "forget.archived",
    "forget.machine_removed",
];

/// A forget cut at any step is finished by the host's next start with no
/// one asking again: the machine's session is archived as lost, box-1 is
/// out of the channel and the machine list, and this Mac's session is
/// untouched. The same key then answers the same.
#[tokio::test(flavor = "multi_thread")]
async fn a_forget_cut_at_any_point_is_finished_at_the_next_start() {
    for point in FORGET_POINTS {
        let (mut rig, channel, on_box, on_host) = gone_box().await;
        rig.restart_host(&[("HUMATA2_STOP_AT", point)]).await;
        let plan = plan(&rig).await;
        let _ = machines(&rig).await.forget_machine(forget(&plan, "forget-cut")).await;
        rig.host_stopped().await;
        rig.restart_host(&[]).await;

        let deadline = std::time::Instant::now() + remote_rig::LONG;
        loop {
            let listed = machines(&rig).await.list_machines(pb::ListMachinesRequest::default()).await.unwrap().into_inner();
            if !listed.machines.iter().any(|machine| machine.id.as_ref() == Some(&Rig::box_id("box-1"))) {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "cut at {point}: box-1 is still a machine here");
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        let summary = rig
            .wait_session(&on_box, "archived as lost", |s| s.archive.as_ref().is_some_and(|a| a.machine_lost))
            .await
            .archive
            .expect("box-1's session is archived");
        assert!(summary.machine_lost, "cut at {point}");
        assert!(rig.session(&on_host).await.archive.is_none(), "cut at {point}: this Mac's session is untouched");
        let shown = pb::channel_service_client::ChannelServiceClient::new(rig.channel().await)
            .get_channel(pb::GetChannelRequest { id: channel.id.clone() })
            .await
            .unwrap()
            .into_inner();
        assert!(!shown.machines.contains(&Rig::box_id("box-1")), "cut at {point}: {:?}", shown.machines);
        let again = machines(&rig).await.forget_machine(forget(&plan, "forget-cut")).await.expect("the key answers").into_inner();
        assert_eq!(again.archived, [on_box.clone()], "cut at {point}");
    }
}
