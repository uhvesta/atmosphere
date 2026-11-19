//! DK-15, MV-16, MV-17: a Desk session on another machine moves into a
//! channel on that machine. It lands in the channel's `sessions/<name>`
//! there by a rename (D801 E): its agent keeps running, started once and
//! never again, the Desk folder it left is a link to the new one, and its
//! conversation is still whole. The host keeps no folder of it, and one
//! agent is live.
//!
//! Every wait is on the host's feeds (the rig's `wait_session`, `ended`).

mod common;
#[path = "common/remote_rig.rs"]
mod remote_rig;

use std::collections::HashSet;
use std::path::PathBuf;

use common::{holder_answers, stored_tabs};
use humata2_grpc::humata::v1 as pb;
use humata2_store::TabKind;
use remote_rig::{Rig, is_live};

const REPO: (&str, &str) = ("acme", "api");

/// What is said to the session before it moves.
const SAID: [&str; 3] = ["sketch the Q4 plan", "add the payments milestones", "it belongs with payments now"];

async fn desks(rig: &Rig) -> pb::desk_service_client::DeskServiceClient<tonic::transport::Channel> {
    pb::desk_service_client::DeskServiceClient::new(rig.channel().await)
}

fn key(value: &str) -> Option<pb::IdempotencyKey> {
    Some(pb::IdempotencyKey { value: value.to_owned() })
}

/// How many agents of `id` are live on all the rig's machines together.
fn live_agents(rig: &Rig, id: &pb::SessionId) -> usize {
    let mut tabs = stored_tabs(&rig.host.state);
    tabs.extend(rig.the("box-1").tabs());
    tabs.extend(rig.the("box-2").tabs());
    tabs.iter()
        .filter(|tab| tab.session.as_deref() == Some(id.value.as_str()) && tab.kind == TabKind::Agent && holder_answers(tab))
        .map(|tab| tab.holder_socket.clone())
        .collect::<HashSet<_>>()
        .len()
}

/// Says `line` to the session's agent and waits for its answer.
async fn say(rig: &Rig, id: &pb::SessionId, line: &str) {
    let tab = common::agent_tab(&rig.host, id).await;
    let open = pb::AttachOpen { session: Some(id.clone()), ..common::open(&tab, 0) };
    let mut attach = common::Attach::start(&rig.host, open).await;
    attach.wait_for("agent up").await;
    attach.type_text(&format!("{line}\n")).await;
    attach.wait_for(&format!("got: {line}")).await;
}

/// What `id`'s conversation on box-2 holds, and the folder its agent
/// works in there.
fn talk_on_box_2(rig: &Rig, id: &pb::SessionId) -> (PathBuf, String) {
    let agent = rig.the("box-2").store().agent(&id.value).unwrap().expect("box-2 has the session's agent");
    (agent.launch.folder.clone(), std::fs::read_to_string(&agent.launch.conversation_file).unwrap_or_default())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_desk_session_on_another_machine_moves_into_a_channel_there_and_keeps_running() {
    let rig = Rig::start(&["box-1", "box-2"], &[REPO]).await;
    let box_2 = Rig::box_id("box-2");
    let payments = rig.create_channel("Payments", &[REPO], &[box_2.clone()]).await;
    let started = desks(&rig)
        .await
        .start_desk_session(pb::StartDeskSessionRequest {
            idempotency_key: key("desk-plan-q4"),
            title: "Plan Q4".to_owned(),
            machine: Some(box_2.clone()),
            access: Some(pb::DeskAccess { boards: true, channels_and_machines: false }),
            ..Default::default()
        })
        .await
        .expect("the Desk session starts")
        .into_inner();
    if let Some(operation) = &started.operation {
        let ended = rig.ended(operation).await;
        assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    }
    let id = started.session.expect("a start names its session");
    rig.wait_session(&id, "live", is_live).await;
    let old = PathBuf::from(started.desk.and_then(|desk| desk.folder).expect("a start says where it works").path);
    for line in SAID {
        say(&rig, &id, line).await;
    }

    let plan = desks(&rig)
        .await
        .plan_desk_move(pb::PlanDeskMoveRequest {
            session: Some(id.clone()),
            channel: payments.id.clone(),
            branch: String::new(),
        })
        .await
        .expect("the move is planned")
        .into_inner();
    let moving = desks(&rig)
        .await
        .move_desk_session_to_channel(pb::MoveDeskSessionToChannelRequest {
            idempotency_key: key("move-plan-q4"),
            session: Some(id.clone()),
            channel: payments.id.clone(),
            plan: plan.token.clone(),
            // Its agent's turn end is only Probably idle (D318, D326): the
            // user moves it anyway.
            confirm_running: true,
            ..Default::default()
        })
        .await
        .expect("the move starts")
        .into_inner();
    let ended = rig.ended(&moving).await;

    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    let now = rig
        .wait_session(&id, "live in Payments on box-2", |session| {
            session.channel == payments.id
                && session.machine.as_ref() == Some(&box_2)
                && is_live(session)
                && session.status != pb::SessionStatus::Moving as i32
        })
        .await;
    assert_eq!(now.machine.as_ref(), Some(&box_2));
    let sessions = rig.the("box-2").channels().join(&payments.folder).join("sessions");
    let new = PathBuf::from(&plan.folder);
    assert_eq!(new.parent(), Some(sessions.as_path()), "it lands among the channel's sessions on box-2");
    let record = rig.the("box-2").store().session(&id.value).unwrap().expect("box-2 holds the session");
    assert_eq!(record.folder, new);
    assert!(rig.the("box-2").store().desk(&id.value).unwrap().is_none(), "it left box-2's Desk");
    assert_eq!(std::fs::read_link(&old).ok(), Some(new.clone()), "its Desk folder on box-2 is a link to the new one");
    let (works_in, said) = talk_on_box_2(&rig, &id);
    assert_eq!(
        std::fs::canonicalize(&works_in).ok(),
        std::fs::canonicalize(&new).ok(),
        "its agent, started under the old name, works in the new folder"
    );
    for line in SAID {
        assert!(said.lines().any(|kept| kept.contains(line)), "it holds {line:?}: {said:?}");
    }
    assert_eq!(live_agents(&rig, &id), 1, "one agent is live");
    assert!(
        !rig.world.channels.join(&payments.folder).join("sessions").join(new.file_name().unwrap_or_default()).exists(),
        "the host keeps no folder of it"
    );
}
