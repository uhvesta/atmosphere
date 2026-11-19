//! An agent's calls on another session (D306) through a real `humatad2`:
//! stopping or archiving it asks the user, deleting it is refused by its own
//! rule (Never by default) and adds nothing to the Inbox, and the rule the
//! user sets for one doesn't move the other. A plan never asks; the call
//! that acts does, so the user's yes carries out the action and a no leaves
//! the session as it was (D730).

use humatad_test_common as common;
use humatad_test_common::mcp_client;

use common::{
    create_channel, error_of, is_running, shell_token, start_session, start_world, wait_running,
    wait_status,
};
use humata2_grpc::humata::v1 as pb;
use mcp_client::Mcp;
use serde_json::{Value, json};
use tonic::Code;

fn as_agent<T>(message: T, token: &str) -> tonic::Request<T> {
    let mut request = tonic::Request::new(message);
    request.metadata_mut().insert(
        "humata-session-token",
        tonic::metadata::MetadataValue::try_from(token).unwrap(),
    );
    request
}

async fn inbox(daemon: &common::Daemon) -> Vec<pb::InboxRequest> {
    daemon
        .inbox_service()
        .await
        .list_requests(pb::ListRequestsRequest {
            include_answered: true,
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner()
        .requests
}

fn never_rule_of(refused: &tonic::Status) -> pb::Capability {
    assert_eq!(refused.code(), Code::PermissionDenied, "{refused:?}");
    match error_of(refused).kind {
        Some(pb::error::Kind::Denied(denied)) => {
            assert_eq!(denied.source(), pb::denied::Source::NeverRule, "{denied:?}");
            denied.capability()
        }
        other => panic!("expected a Never rule's denial, got {other:?}"),
    }
}

fn pending(refused: &tonic::Status) {
    match error_of(refused).kind {
        Some(pb::error::Kind::ApprovalPending(_)) => {}
        other => panic!("expected approval_pending, got {other:?}"),
    }
}

struct Scene {
    daemon: common::Daemon,
    channel: pb::Channel,
    other: pb::SessionId,
    token: String,
    _attach: common::Attach,
}

async fn scene() -> Scene {
    let (daemon, world) = start_world().await;
    world.remote("acme", "api");
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    let (mine, _) = start_session(&daemon, &channel, "Mine").await;
    let (other, _) = start_session(&daemon, &channel, "Other").await;
    // The test agent speaks once and goes quiet, so it may already read as
    // ProbablyIdle (D169) by the time the second start is done: both still run.
    wait_running(&daemon, &mine).await;
    wait_running(&daemon, &other).await;
    let (token, attach) = shell_token(&daemon, &mine).await;
    Scene { daemon, channel, other, token, _attach: attach }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_deleting_another_session_is_refused_and_asks_nothing() {
    let scene = scene().await;
    let mut sessions = scene.daemon.session_service().await;

    let plan = sessions
        .plan_delete(as_agent(pb::PlanDeleteRequest { id: Some(scene.other.clone()) }, &scene.token))
        .await
        .unwrap_err();
    let delete = sessions
        .delete_session(as_agent(
            pb::DeleteSessionRequest {
                id: Some(scene.other.clone()),
                ..Default::default()
            },
            &scene.token,
        ))
        .await
        .unwrap_err();

    assert_eq!(never_rule_of(&plan), pb::Capability::SessionDeleteOther);
    assert_eq!(never_rule_of(&delete), pb::Capability::SessionDeleteOther);
    assert!(inbox(&scene.daemon).await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_stopping_or_archiving_another_session_still_asks() {
    let scene = scene().await;
    let mut sessions = scene.daemon.session_service().await;

    let stop = sessions
        .stop_session(as_agent(
            pb::StopSessionRequest {
                idempotency_key: None,
                id: Some(scene.other.clone()),
                for_move: false,
            },
            &scene.token,
        ))
        .await
        .unwrap_err();
    let plan = sessions
        .plan_archive(as_agent(pb::PlanArchiveRequest { id: Some(scene.other.clone()) }, &scene.token))
        .await
        .expect("a plan only reads, so it never asks")
        .into_inner();
    let archive = sessions
        .archive_session(as_agent(
            pb::ArchiveSessionRequest {
                idempotency_key: None,
                id: Some(scene.other.clone()),
                choice: pb::ArchiveChoice::Archive as i32,
                plan: plan.token,
                end: None,
            },
            &scene.token,
        ))
        .await
        .unwrap_err();

    pending(&stop);
    pending(&archive);
    let held = inbox(&scene.daemon).await;
    assert!(!held.is_empty());
    for request in &held {
        assert_eq!(request.capability(), pb::Capability::SessionStopOther, "{request:?}");
    }
    let other = common::get_session(&scene.daemon, &scene.other).await;
    assert!(is_running(&other), "the other session runs on: {:?}", other.status());
}

#[tokio::test(flavor = "multi_thread")]
async fn setting_the_delete_rule_to_ask_makes_a_delete_wait_for_the_user() {
    let scene = scene().await;
    pb::access_service_client::AccessServiceClient::new(scene.daemon.channel().await)
        .set_channel_rule(pb::SetChannelRuleRequest {
            channel: scene.channel.id.clone(),
            capability: pb::Capability::SessionDeleteOther as i32,
            decision: pb::Decision::Ask as i32,
            ..Default::default()
        })
        .await
        .expect("the user sets the rule");

    let mut sessions = scene.daemon.session_service().await;
    let planned = sessions
        .plan_delete(as_agent(pb::PlanDeleteRequest { id: Some(scene.other.clone()) }, &scene.token))
        .await;
    let asked_by_plan = inbox(&scene.daemon).await.len();
    let waiting = sessions
        .delete_session(as_agent(
            pb::DeleteSessionRequest {
                id: Some(scene.other.clone()),
                ..Default::default()
            },
            &scene.token,
        ))
        .await
        .unwrap_err();

    // Deleting isn't built, so the plan answers that, but it asks nothing.
    assert!(planned.is_err_and(|refused| refused.code() != Code::PermissionDenied));
    assert_eq!(asked_by_plan, 0);
    pending(&waiting);
    let held = inbox(&scene.daemon).await;
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].capability(), pb::Capability::SessionDeleteOther);
}

/// `session_archive` on another session, as the agent's MCP client calls
/// it, with `wait_seconds` 0: its pending answer's request id.
async fn archive_through_mcp(scene: &Scene) -> (Mcp, String) {
    let mut mcp = Mcp::start(&scene.daemon.socket, &scene.token).await;
    let asked = mcp
        .call(1, "session_archive", json!({ "session": scene.other.value, "wait_seconds": 0 }))
        .await;
    assert_eq!(asked["structuredContent"]["outcome"], "pending", "{asked}");
    let id = asked["structuredContent"]["request_id"].as_str().unwrap().to_owned();
    (mcp, id)
}

async fn answer(scene: &Scene, id: &str, answer: pb::request_answer::Answer) {
    scene
        .daemon
        .inbox_service()
        .await
        .answer_request(pb::AnswerRequestRequest {
            id: Some(pb::RequestId { value: id.to_owned() }),
            answer: Some(pb::RequestAnswer { answer: Some(answer) }),
            ..Default::default()
        })
        .await
        .expect("the user answers the request");
}

async fn waited(mcp: &mut Mcp, id: &str) -> Value {
    mcp.call(2, "inbox_wait", json!({ "request_id": id })).await
}

#[tokio::test(flavor = "multi_thread")]
async fn approving_an_agents_archive_of_another_session_archives_it() {
    let scene = scene().await;
    let (mut mcp, id) = archive_through_mcp(&scene).await;
    let held = inbox(&scene.daemon).await;

    answer(&scene, &id, pb::request_answer::Answer::Approve(pb::Approve {})).await;
    let waited = waited(&mut mcp, &id).await;

    assert_eq!(held.len(), 1, "one request, the archive's: {held:?}");
    assert_eq!(waited["isError"], false, "{waited}");
    wait_status(&scene.daemon, &scene.other, pb::SessionStatus::Archived).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn denying_an_agents_archive_of_another_session_leaves_it_working() {
    let scene = scene().await;
    let (mut mcp, id) = archive_through_mcp(&scene).await;

    answer(&scene, &id, pb::request_answer::Answer::Deny(pb::Deny::default())).await;
    let waited = waited(&mut mcp, &id).await;

    assert_eq!(waited["isError"], true, "{waited}");
    let other = common::get_session(&scene.daemon, &scene.other).await;
    assert!(is_running(&other), "the other session runs on: {:?}", other.status());
}

/// DeleteSession of another session as the agent, with `key`: the id of
/// the request it waits on.
async fn delete_asked(scene: &Scene, key: &str) -> String {
    let refused = scene
        .daemon
        .session_service()
        .await
        .delete_session(as_agent(
            pb::DeleteSessionRequest {
                id: Some(scene.other.clone()),
                idempotency_key: Some(pb::IdempotencyKey { value: key.to_owned() }),
                ..Default::default()
            },
            &scene.token,
        ))
        .await
        .unwrap_err();
    match error_of(&refused).kind {
        Some(pb::error::Kind::ApprovalPending(pending)) => pending.request.unwrap().value,
        other => panic!("expected approval_pending, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn approving_a_delete_under_ask_runs_the_delete_and_denying_it_does_not() {
    let scene = scene().await;
    pb::access_service_client::AccessServiceClient::new(scene.daemon.channel().await)
        .set_channel_rule(pb::SetChannelRuleRequest {
            channel: scene.channel.id.clone(),
            capability: pb::Capability::SessionDeleteOther as i32,
            decision: pb::Decision::Ask as i32,
            ..Default::default()
        })
        .await
        .expect("the user sets the rule");
    let approved = delete_asked(&scene, "approved").await;
    let denied = delete_asked(&scene, "denied").await;
    let mut mcp = Mcp::start(&scene.daemon.socket, &scene.token).await;

    answer(&scene, &approved, pb::request_answer::Answer::Approve(pb::Approve {})).await;
    answer(&scene, &denied, pb::request_answer::Answer::Deny(pb::Deny::default())).await;
    let ran = waited(&mut mcp, &approved).await;
    let refused = waited(&mut mcp, &denied).await;

    // Deleting isn't built yet: once approved, the delete itself runs and
    // gives its own answer, not the plan's or a second request.
    assert_eq!(ran["isError"], true, "{ran}");
    assert_ne!(ran["structuredContent"]["error"]["kind"], "denied", "{ran}");
    assert_eq!(refused["isError"], true, "{refused}");
    assert_eq!(refused["structuredContent"]["error"]["kind"], "denied", "{refused}");
    assert_eq!(inbox(&scene.daemon).await.len(), 2, "no second request after the answers");
}
