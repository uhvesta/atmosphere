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
    let one = rig.linked(BOX).await.id.expect("box-1 has a machine id");
    assert_eq!(one, Rig::box_id(BOX), "remote_rig names the box by its machine id");
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
    let one = rig.linked(BOX).await.id.expect("box-1 has a machine id");
    let two = rig.linked(OTHER).await.id.expect("box-2 has a machine id");
    assert_eq!(one, Rig::box_id(BOX));
    assert_eq!(two, Rig::box_id(OTHER));
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

#[tokio::test(flavor = "multi_thread")]
async fn remote_forks_staying_are_preserved_on_local_and_remote_replans() {
    let rig = Rig::start(&[BOX], &[REPO]).await;
    let host = rig.host_id().await;
    let one = rig.linked(BOX).await.id.expect("box-1 has a machine id");
    assert_eq!(one, Rig::box_id(BOX));
    let channel = rig.create_channel("Payments", &[REPO], &[host.clone(), one.clone()]).await;
    let local = rig.start_on(&channel, &host, "Local work").await;
    let remote = rig.start_on(&channel, &one, "Remote parent").await;
    rig.wait_session(&local, "live", is_live).await;
    rig.wait_session(&remote, "live", is_live).await;

    let started = sessions(&rig)
        .await
        .fork_session(pb::ForkSessionRequest {
            idempotency_key: Some(pb::IdempotencyKey { value: "fork-code-stay".to_owned() }),
            parent: Some(remote.clone()),
            kind: pb::ForkKind::CodeOnly as i32,
            title: "Staying fork".to_owned(),
            machine: Some(one.clone()),
            ..Default::default()
        })
        .await
        .expect("the code fork starts")
        .into_inner();
    let ended = rig.ended(&started).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{ended:?}");
    let Some(pb::operation::Result::Session(code_fork)) = ended.result else {
        panic!("the fork names no session: {ended:?}");
    };
    rig.wait_session(&code_fork, "live", is_live).await;

    let planned = plan(&rig, &[&local, &remote]).await;
    assert!(
        planned.forks_staying.iter().any(|work| work.session.as_ref() == Some(&code_fork)),
        "initial plan lists the remote code fork as staying: {planned:?}"
    );

    // A local change triggers a re-plan without re-querying box-1: box-1's
    // staying forks must survive adopt_remote.
    let local_clone = rig.session(&local).await.repos[0].path.clone();
    std::fs::write(std::path::Path::new(&local_clone).join("local_wip.txt"), "wip\n").unwrap();
    let answered = sessions(&rig)
        .await
        .delete_sessions(pb::DeleteSessionsRequest {
            idempotency_key: None,
            plan: planned.token.clone(),
            delete_unpushed_work: true,
            typed_count: 0,
            check_only: false,
        })
        .await
        .expect("re-plan is returned")
        .into_inner();
    let Some(pb::delete_sessions_response::Result::Changed(changed)) = answered.result else {
        panic!("expected Changed after local edit, got {answered:?}");
    };
    let after_local = changed.plan.expect("a fresh plan after local change");
    assert!(
        after_local.forks_staying.iter().any(|work| work.session.as_ref() == Some(&code_fork)),
        "re-plan after local change must preserve remote forks_staying: {after_local:?}"
    );

    // A remote change on box-1 triggers check_remote to replace box-1's plan:
    // staying forks must still be present.
    let remote_folder = rig
        .the(BOX)
        .store()
        .session(&remote.value)
        .unwrap()
        .expect("the box holds the remote session")
        .folder;
    std::fs::write(remote_folder.join("api").join("remote_wip.txt"), "wip\n").unwrap();
    let answered = sessions(&rig)
        .await
        .delete_sessions(pb::DeleteSessionsRequest {
            idempotency_key: None,
            plan: after_local.token.clone(),
            delete_unpushed_work: true,
            typed_count: 0,
            check_only: false,
        })
        .await
        .expect("re-plan is returned")
        .into_inner();
    let Some(pb::delete_sessions_response::Result::Changed(changed)) = answered.result else {
        panic!("expected Changed after remote edit, got {answered:?}");
    };
    let after_remote = changed.plan.expect("a fresh plan after remote change");
    assert!(
        after_remote.forks_staying.iter().any(|work| work.session.as_ref() == Some(&code_fork)),
        "re-plan after remote change must preserve remote forks_staying: {after_remote:?}"
    );

    delete(&rig, &after_remote).await;
    assert!(rig.the(BOX).store().session(&remote.value).unwrap().is_none());
    assert!(
        rig.the(BOX).store().session(&code_fork.value).unwrap().is_some(),
        "the staying code fork is untouched"
    );
}
