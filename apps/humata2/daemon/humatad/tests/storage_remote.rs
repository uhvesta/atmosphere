//! The Storage page of another machine (LD-13 to LD-15): the host asks the
//! machine, over its link, what it keeps, and the clean-ups run there. A
//! host and one box on the multi-machine rig; nothing waits on a clock.

mod common;
#[path = "common/remote_rig.rs"]
mod remote_rig;

use common::error_of;
use humata2_grpc::humata::v1 as pb;
use pb::machine_service_client::MachineServiceClient;
use remote_rig::{Rig, is_live};
use tonic::transport::Channel;
use tonic::{Code, Status};

const REPO: (&str, &str) = ("acme", "api");
const BOX: &str = "box-1";

async fn machines(rig: &Rig) -> MachineServiceClient<Channel> {
    MachineServiceClient::new(rig.host.channel().await)
}

fn box_id() -> Option<pb::MachineId> {
    Some(Rig::box_id(BOX))
}

/// A live session on the box.
async fn on_box() -> (Rig, pb::SessionId) {
    let rig = Rig::start(&[BOX], &[REPO]).await;
    let (host, one) = (rig.host_id().await, Rig::box_id(BOX));
    let channel = rig.create_channel("Payments", &[REPO], &[host, one.clone()]).await;
    let id = rig.start_on(&channel, &one, "Cache warmup").await;
    rig.wait_session(&id, "live", is_live).await;
    (rig, id)
}

/// The sessions whose clones `report` lists.
fn clone_sessions(report: &pb::StorageReport) -> Vec<String> {
    report
        .rows
        .iter()
        .filter(|row| row.kind == pb::StorageKind::SessionClones as i32)
        .filter_map(|row| match row.entity.as_ref()?.entity.as_ref()? {
            pb::entity_ref::Entity::Session(id) => Some(id.value.clone()),
            _ => None,
        })
        .collect()
}

fn is_offline(status: &Status) -> bool {
    matches!(error_of(status).kind, Some(pb::error::Kind::MachineOffline(_)))
}

fn clear_request(session: &pb::SessionId) -> pb::ClearBuildCacheRequest {
    pb::ClearBuildCacheRequest {
        idempotency_key: None,
        session: Some(session.clone()),
        path: "target".to_owned(),
        confirm_running: true,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_storage_page_of_a_box_shows_what_the_box_keeps() {
    let (rig, id) = on_box().await;

    let report = machines(&rig)
        .await
        .get_storage(pb::GetStorageRequest { id: box_id() })
        .await
        .expect("the host reads the box's storage")
        .into_inner();
    assert_eq!(report.machine, box_id());
    assert!(report.total_bytes > 0);
    assert!(clone_sessions(&report).contains(&id.value), "the box's session is in the box's report: {report:?}");

    let own = machines(&rig)
        .await
        .get_storage(pb::GetStorageRequest { id: None })
        .await
        .expect("the host reads its own storage")
        .into_inner();
    assert!(!clone_sessions(&own).contains(&id.value), "the host's own report doesn't hold the box's clones");
}

#[tokio::test(flavor = "multi_thread")]
async fn trimming_the_logs_of_a_box_runs_on_the_box() {
    let (rig, _) = on_box().await;
    machines(&rig)
        .await
        .trim_logs(pb::TrimLogsRequest { idempotency_key: None, id: box_id() })
        .await
        .expect("the box trims its own logs");
}

#[tokio::test(flavor = "multi_thread")]
async fn clearing_a_cache_of_a_session_on_a_box_is_decided_on_the_box() {
    let (rig, id) = on_box().await;
    // The box has no such cache in the session's folder: its answer is a
    // refusal of the path, not "no such session here".
    let refused = machines(&rig)
        .await
        .clear_build_cache(clear_request(&id))
        .await
        .expect_err("there is no cache called target");
    assert_eq!(refused.code(), Code::InvalidArgument);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_box_that_is_offline_answers_machine_offline_at_once() {
    let (rig, id) = on_box().await;
    rig.link_lost(BOX).await;

    let mut client = machines(&rig).await;
    let report = client.get_storage(pb::GetStorageRequest { id: box_id() }).await.expect_err("offline");
    assert!(is_offline(&report), "{report:?}");
    let trim = client
        .trim_logs(pb::TrimLogsRequest { idempotency_key: None, id: box_id() })
        .await
        .expect_err("offline");
    assert!(is_offline(&trim), "{trim:?}");
    let clear = client.clear_build_cache(clear_request(&id)).await.expect_err("offline");
    assert!(is_offline(&clear), "{clear:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_machine_the_host_doesnt_know_is_not_found() {
    let rig = Rig::start(&[BOX], &[REPO]).await;
    let unknown = machines(&rig)
        .await
        .get_storage(pb::GetStorageRequest { id: Some(Rig::box_id("nowhere")) })
        .await
        .expect_err("no such machine");
    assert_eq!(unknown.code(), Code::NotFound);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_box_answers_storage_to_the_host_and_to_nobody_else() {
    let rig = Rig::start(&[BOX], &[REPO]).await;
    // A caller at the box itself is not the host.
    let channel = rig.the(BOX).daemon().channel().await;
    let direct = MachineServiceClient::new(channel)
        .get_storage(pb::GetStorageRequest { id: None })
        .await;
    assert!(direct.is_err(), "the box's own caller isn't the host: {direct:?}");
}
