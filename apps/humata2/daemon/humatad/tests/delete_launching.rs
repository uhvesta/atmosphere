//! D784: deleting a channel ends every agent of its sessions, one saved as
//! still launching or being resumed included, even when no tab records its
//! holder yet: the holder is found by the agent's own socket.

mod common;

use std::path::Path;
use std::time::{Duration, Instant};

use common::{Daemon, World, create_channel, finished, folder, start_session, state_folder, wait_running};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{AgentState, Store};

fn answers(socket: &Path) -> bool {
    std::os::unix::net::UnixStream::connect(socket).is_ok()
}

async fn deleting_ends_an_agent_saved_as(state: AgentState) {
    let dir = folder();
    let world = World::new(dir.path());
    let args = world.args();
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let daemon = Daemon::start_with(common::Build::Admin, dir, &refs).await;
    world.remote("acme", "api");
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    let (id, _) = start_session(&daemon, &channel, "Fix login").await;
    wait_running(&daemon, &id).await;

    // The agent is saved as launching, and no tab names its holder.
    let store = Store::open(state_folder(daemon.dir.path()).join("store")).unwrap();
    let mut agent = store.agent(&id.value).unwrap().expect("the agent is recorded");
    let socket = agent.launch.holder_socket.clone();
    assert!(answers(&socket), "the agent's holder runs before the delete");
    agent.state = state;
    store.put_agent(&agent).unwrap();
    store.remove_tab(&agent.tab).unwrap();

    let mut channels = daemon.channel_service().await;
    let plan = channels
        .plan_delete_channel(pb::PlanDeleteChannelRequest { channel: channel.id.clone() })
        .await
        .unwrap()
        .into_inner();
    let deleted = channels
        .delete_channel(pb::DeleteChannelRequest {
            idempotency_key: None,
            channel: plan.channel.clone(),
            plan: plan.token.clone(),
            presence: None,
            delete_unpushed_work: true,
            end_live: None,
        })
        .await
        .unwrap()
        .into_inner();
    let operation = finished(&daemon, deleted.operation.as_ref().unwrap()).await;
    assert_eq!(operation.state, pb::OperationState::Succeeded as i32, "{operation:?}");

    let deadline = Instant::now() + Duration::from_secs(30);
    while answers(&socket) {
        assert!(Instant::now() < deadline, "the deleted channel's agent still runs");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn deleting_a_channel_ends_an_agent_still_launching() {
    deleting_ends_an_agent_saved_as(AgentState::Launching).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn deleting_a_channel_ends_an_agent_being_resumed() {
    deleting_ends_an_agent_saved_as(AgentState::Resuming).await;
}
