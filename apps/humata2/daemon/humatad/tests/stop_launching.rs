//! Stop on an agent saved as launching whose old holder is still alive
//! (SAFE): no start of this daemon is live for it, so stop ends the old
//! holder, saves the agent stopped only once that holder is gone, and the
//! agent then resumes as any stopped one. The state is made by saving the
//! launching record over a running agent, which is what a daemon stopped
//! between starting a holder and saving it running leaves. Waits are
//! bounded by `LONG`.

mod common;
#[path = "common/remote_rig.rs"]
mod remote_rig;

use humata2_grpc::humata::v1 as pb;
use humata2_store::{AgentState, TabKind};
use remote_rig::{LONG, Rig, is_live};

const REPO: (&str, &str) = ("acme", "api");

#[tokio::test(flavor = "multi_thread")]
async fn stop_on_a_launching_agent_ends_its_holder_then_the_agent_resumes() {
    let rig = Rig::start(&[], &[REPO]).await;
    let host = rig.host_id().await;
    let channel = rig.create_channel("launching", &[REPO], std::slice::from_ref(&host)).await;
    let session = rig.start_on(&channel, &host, "Left launching").await;
    rig.wait_session(&session, "running", |one| common::is_running(one)).await;

    let store = humata2_store::Store::open(rig.host.state.join("store")).unwrap();
    let mut agent = store.agent(&session.value).unwrap().expect("the agent is saved");
    agent.state = AgentState::Launching;
    store.put_agent(&agent).unwrap();
    let tab = common::stored_tabs(&rig.host.state)
        .into_iter()
        .find(|tab| tab.session.as_deref() == Some(session.value.as_str()) && matches!(tab.kind, TabKind::Agent))
        .expect("the agent's tab");
    assert!(common::holder_answers(&tab), "the old holder is alive");

    pb::session_service_client::SessionServiceClient::new(rig.channel().await)
        .stop_session(pb::StopSessionRequest { idempotency_key: None, id: Some(session.clone()), for_move: false })
        .await
        .expect("stop ends the old holder of a launching agent");

    let saved = store.agent(&session.value).unwrap().expect("still saved");
    assert_eq!(saved.state, AgentState::Stopped);
    let deadline = std::time::Instant::now() + LONG;
    while common::holder_answers(&tab) {
        assert!(std::time::Instant::now() < deadline, "the old holder never went");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    let resumed = pb::session_service_client::SessionServiceClient::new(rig.channel().await)
        .resume_session(pb::ResumeSessionRequest { id: Some(session.clone()), ..Default::default() })
        .await
        .expect("a stopped agent resumes")
        .into_inner();
    let ended = rig.ended(&resumed).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    rig.wait_session(&session, "live again", |one| is_live(one)).await;
}
