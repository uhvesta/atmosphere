//! Deleting sessions that run on other machines (MASSDELETE, D876): the host
//! asks each online machine for the plan of its own sessions and keeps the
//! machine's token; the machine trashes its own folders and forgets its own
//! records. An offline machine's sessions are listed as left alone and
//! nothing is done to them. A host and boxes on the multi-machine rig.

use humatad_test_common::remote_rig;

use humata2_grpc::humata::v1 as pb;
use remote_rig::{Rig, is_live};
use tonic::Code;

const BOX: &str = "box-1";
const OTHER: &str = "box-2";
const REPO: (&str, &str) = ("acme", "api");

async fn sessions(rig: &Rig) -> pb::session_service_client::SessionServiceClient<tonic::transport::Channel> {
    pb::session_service_client::SessionServiceClient::new(rig.channel().await)
}

fn picked(ids: &[&pb::SessionId]) -> pb::DeleteScope {
    pb::DeleteScope {
        scope: Some(pb::delete_scope::Scope::Sessions(pb::delete_scope::Picked {
            ids: ids.iter().map(|id| (*id).clone()).collect(),
        })),
    }
}

async fn plan(rig: &Rig, ids: &[&pb::SessionId]) -> pb::DeleteSessionsPlan {
    sessions(rig)
        .await
        .plan_delete_sessions(pb::PlanDeleteSessionsRequest { scope: Some(picked(ids)) })
        .await
        .expect("the plan is made")
        .into_inner()
}

async fn delete(rig: &Rig, plan: &pb::DeleteSessionsPlan) -> pb::delete_sessions_response::Deleted {
    let answered = sessions(rig)
        .await
        .delete_sessions(pb::DeleteSessionsRequest {
            idempotency_key: None,
            plan: plan.token.clone(),
            delete_unpushed_work: true,
            typed_count: 0,
            check_only: false,
        })
        .await
        .expect("the delete is accepted")
        .into_inner();
    let Some(pb::delete_sessions_response::Result::Deleted(done)) = answered.result else {
        panic!("expected a delete, got {answered:?}");
    };
    let ended = rig.ended(done.operation.as_ref().expect("a delete has an operation")).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{ended:?}");
    done
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_on_an_online_machine_is_deleted_there_through_the_host() {
    let rig = Rig::start(&[BOX], &[REPO]).await;
    let one = Rig::box_id(BOX);
    let channel = rig.create_channel("Payments", &[REPO], &[rig.host_id().await, one.clone()]).await;
    let session = rig.start_on(&channel, &one, "Ledger work").await;
    rig.wait_session(&session, "live", is_live).await;
    let folder = rig.the(BOX).store().session(&session.value).unwrap().expect("the box holds the session").folder;

    let planned = plan(&rig, &[&session]).await;

    let listed = planned.sessions.iter().find(|s| s.session.as_ref() == Some(&session)).expect("the session is planned");
    assert_eq!(listed.machine.as_ref(), Some(&one), "{planned:?}");
    assert!(planned.left_alone.is_empty(), "an online machine is not left alone: {planned:?}");
    let done = delete(&rig, &planned).await;

    assert_eq!(done.sessions, 1);
    assert!(rig.the(BOX).store().session(&session.value).unwrap().is_none(), "the box forgot the session");
    assert!(!folder.exists(), "the box's folder went to its trash");
    let trashed = std::fs::read_dir(rig.the(BOX).trash()).map(|items| items.flatten().count()).unwrap_or(0);
    assert!(trashed > 0, "the box's trash holds the folder");
    let gone = sessions(&rig)
        .await
        .get_session(pb::GetSessionRequest { id: Some(session.clone()) })
        .await
        .unwrap_err();
    assert_eq!(gone.code(), Code::NotFound, "the host forgot it too: {gone:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_on_an_offline_machine_is_listed_as_left_alone_and_untouched() {
    let rig = Rig::start(&[BOX, OTHER], &[REPO]).await;
    let (one, two) = (Rig::box_id(BOX), Rig::box_id(OTHER));
    let channel = rig
        .create_channel("Payments", &[REPO], &[rig.host_id().await, one.clone(), two.clone()])
        .await;
    let online = rig.start_on(&channel, &one, "Ledger work").await;
    let away = rig.start_on(&channel, &two, "Audit trail").await;
    rig.wait_session(&online, "live", is_live).await;
    rig.wait_session(&away, "live", is_live).await;
    rig.link_lost(OTHER).await;

    let planned = plan(&rig, &[&online, &away]).await;

    assert!(planned.sessions.iter().any(|s| s.session.as_ref() == Some(&online)), "{planned:?}");
    assert!(!planned.sessions.iter().any(|s| s.session.as_ref() == Some(&away)), "{planned:?}");
    let left = planned.left_alone.iter().find(|l| l.machine.as_ref() == Some(&two)).expect("box-2 is left alone");
    assert!(left.sessions.iter().any(|s| s.session.as_ref() == Some(&away)), "{planned:?}");
    delete(&rig, &planned).await;

    assert!(rig.the(BOX).store().session(&online.value).unwrap().is_none());
    assert!(
        rig.the(OTHER).store().session(&away.value).unwrap().is_some(),
        "the offline machine's session is untouched"
    );
    let still = rig.session(&away).await;
    assert_eq!(still.id.as_ref(), Some(&away), "the host still lists it");
}
