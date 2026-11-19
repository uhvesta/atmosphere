//! D784: an agent whose holder is gone while the daemon runs is never a
//! running row that can't be attached: it reads as ended, and Resume
//! brings it back to a terminal that attaches.

use humatad_test_common as common;

use std::time::{Duration, Instant};

use common::{
    Attach, Daemon, World, agent_tab, create_channel, finished, folder, get_session, is_running,
    open, start_session, stored_tabs,
};
use humata2_grpc::humata::v1 as pb;

#[tokio::test]
async fn an_agent_whose_holder_died_reads_as_ended_and_resumes() {
    let dir = folder();
    let world = World::new(dir.path());
    let args = world.args();
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let daemon = Daemon::start_with_env(common::Build::Admin, dir, &refs, &[]).await;
    world.remote("acme", "api");
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    let (id, _) = start_session(&daemon, &channel, "Fix login").await;
    common::wait_running(&daemon, &id).await;
    let agent = agent_tab(&daemon, &id).await;
    let holder: Vec<_> = stored_tabs(&daemon.state)
        .into_iter()
        .filter(|tab| tab.id == agent.value)
        .collect();

    common::kill_holders(&holder).await;
    let deadline = Instant::now() + Duration::from_secs(30);
    while is_running(&get_session(&daemon, &id).await) {
        assert!(
            Instant::now() < deadline,
            "an agent whose holder is gone still reads as running"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let resumed = daemon
        .session_service()
        .await
        .resume_session(pb::ResumeSessionRequest {
            idempotency_key: None,
            id: Some(id.clone()),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    let done = finished(&daemon, &resumed).await;
    assert!(
        matches!(done.result, Some(pb::operation::Result::Session(_))),
        "the resume failed: {done:?}"
    );
    common::wait_running(&daemon, &id).await;
    let agent = agent_tab(&daemon, &id).await;
    assert!(
        Attach::try_start(&daemon, open(&agent, 0)).await.is_ok(),
        "the resumed agent's terminal can't be attached"
    );
}
