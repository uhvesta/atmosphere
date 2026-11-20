//! To-do automation through a real `humatad2`: a session's change on the
//! event feed reaches the To-do list, so an agent-owned item whose agent
//! exits shows as stuck and records the run (TODO2WIRE).

use humatad_test_common as common;

use std::time::{Duration, Instant};

use common::{
    WAIT, agent_tab, create_channel, start_session, start_world, wait_running,
};
use humata2_grpc::humata::v1 as pb;
use tonic::transport::Channel;

/// Polls tightly until `ready` holds for the item, within `WAIT`.
async fn item_when(
    client: &mut pb::todo_service_client::TodoServiceClient<Channel>,
    id: &pb::BoardItemId,
    ready: impl Fn(&pb::TodoItem) -> bool,
) -> pb::TodoItem {
    let deadline = Instant::now() + WAIT;
    loop {
        let listed = client
            .list_todo(pb::ListTodoRequest::default())
            .await
            .unwrap()
            .into_inner();
        if let Some(item) = listed
            .items
            .into_iter()
            .find(|item| item.item.as_ref().and_then(|i| i.id.as_ref()) == Some(id))
            && ready(&item)
        {
            return item;
        }
        assert!(Instant::now() < deadline, "the item never reached the expected state");
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_owned_item_is_stuck_and_has_a_run_when_its_agent_exits() {
    let (daemon, world) = start_world().await;
    world.remote("acme", "api");
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    let (session, _) = start_session(&daemon, &channel, "Owner").await;
    wait_running(&daemon, &session).await;
    let tab = agent_tab(&daemon, &session).await;

    let mut client = pb::todo_service_client::TodoServiceClient::new(daemon.channel().await);
    let created = client
        .create_todo(pb::CreateTodoRequest {
            title: "Ship it".to_owned(),
            channel: channel.id.clone(),
            sessions: vec![session.clone()],
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    let id = created.item.unwrap().item.unwrap().id.unwrap();
    client
        .update_todo(pb::UpdateTodoRequest {
            id: Some(id.clone()),
            patch: Some(pb::TodoPatch {
                state: Some(pb::TodoState::InProgress as i32),
                owner: Some(pb::TodoOwner {
                    kind: pb::TodoOwnerKind::Agent as i32,
                    label: "fake".to_owned(),
                    session: Some(session.clone()),
                }),
                ..Default::default()
            }),
            ..Default::default()
        })
        .await
        .unwrap();

    // An interrupt stops the test agent (see session_events.rs).
    daemon
        .terminals()
        .await
        .r#type(pb::TypeRequest {
            idempotency_key: Some(pb::IdempotencyKey {
                value: "exit-the-agent".to_owned(),
            }),
            session: Some(session.clone()),
            tab: Some(tab),
            text: "\u{3}".to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();

    let stuck = item_when(&mut client, &id, |item| {
        item.fields.as_ref().is_some_and(|fields| fields.stuck_agent.is_some())
    })
    .await;
    let notice = stuck.fields.unwrap().stuck_agent.unwrap();
    assert_eq!(notice.session, Some(session.clone()));

    let runs = client
        .list_todo_runs(pb::ListTodoRunsRequest { id: Some(id) })
        .await
        .unwrap()
        .into_inner()
        .runs;
    assert!(
        runs.iter().any(|run| run.session.as_ref() == Some(&session)),
        "the exited turn should be a run on the item, got {runs:?}"
    );
}
