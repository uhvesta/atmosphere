//! A resume whose agent exits as soon as it starts fails, and ends the
//! holder it started (D784): a failed op's own holder is its debris, not a
//! session to keep. Nothing is left running for the session, and the
//! machine lists no holder for its tab.

mod common;

use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

use common::{Daemon, World, agent_tab, create_channel, finished, folder, start_session, stored_tabs};
use humata2_grpc::humata::v1 as pb;

/// How long the stopped agent's holder gets to go.
const DEADLINE: Duration = Duration::from_secs(30);

/// The test agent from now on: a resume prints a line and exits at once.
const EXITS_ON_RESUME: &str = r#"case "$1" in
  --resume) echo "agent exits at once"; exit 0 ;;
esac
exec sleep 1000
"#;

fn answers(socket: &Path) -> bool {
    UnixStream::connect(socket).is_ok()
}

async fn running_holders(daemon: &Daemon) -> Vec<pb::RunningHolder> {
    pb::machine_service_client::MachineServiceClient::new(daemon.channel().await)
        .list_running_holders(pb::ListRunningHoldersRequest { machine: None })
        .await
        .expect("the machine lists its holders")
        .into_inner()
        .holders
}

#[tokio::test(flavor = "multi_thread")]
async fn a_resume_whose_agent_exits_at_once_fails_and_leaves_no_holder() {
    let dir = folder();
    let world = World::new(dir.path());
    let args = world.args();
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let daemon = Daemon::start_with_env(common::Build::Admin, dir, &refs, &[]).await;
    world.remote("acme", "api");
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    let (id, _) = start_session(&daemon, &channel, "Fix login").await;
    common::wait_running(&daemon, &id).await;
    let tab = agent_tab(&daemon, &id).await;
    let socket = stored_tabs(&daemon.state)
        .into_iter()
        .find(|record| record.id == tab.value)
        .expect("the agent's tab is kept")
        .holder_socket;

    daemon
        .session_service()
        .await
        .stop_session(pb::StopSessionRequest { idempotency_key: None, id: Some(id.clone()), for_move: false })
        .await
        .expect("the session stops");
    let deadline = Instant::now() + DEADLINE;
    while answers(&socket) {
        assert!(Instant::now() < deadline, "the stopped agent's holder still runs");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    std::fs::write(world.root.join("agent.sh"), EXITS_ON_RESUME).unwrap();
    let asked = daemon
        .session_service()
        .await
        .resume_session(pb::ResumeSessionRequest {
            idempotency_key: None,
            id: Some(id.clone()),
            ..Default::default()
        })
        .await
        .expect("the resume is taken")
        .into_inner();
    let done = finished(&daemon, &asked).await;

    assert_eq!(done.state, pb::OperationState::Failed as i32, "{done:?}");
    assert!(!answers(&socket), "the failed resume left its holder running");
    let left: Vec<_> = running_holders(&daemon)
        .await
        .into_iter()
        .filter(|row| !row.stopped)
        .filter(|row| row.tab.as_ref() == Some(&tab) || row.session.as_ref() == Some(&id))
        .collect();
    assert!(left.is_empty(), "the machine still lists a holder for the session's tab: {left:?}");
}

/// The test agent for a restore: a resume stays up a few seconds, so the
/// listing can catch its holder, then exits without its conversation. It
/// keeps printing, so its output never settles and the check never passes.
const EXITS_SOON_ON_RESUME: &str = r#"case "$1" in
  --resume) for i in 1 2 3 4 5 6 7 8 9 10 11 12; do echo "working $i"; sleep 0.3; done; exit 0 ;;
esac
exec sleep 1000
"#;

/// Found live in VERIFY 0.2.3: while a restore's resume was still checking
/// the agent, the machine listed its holder as one no session names, and
/// saved a tab with no session for it. A holder a resume started is the
/// session's while the resume runs, and goes when the resume fails.
#[tokio::test(flavor = "multi_thread")]
async fn a_restores_holder_is_listed_as_the_sessions_and_goes_when_the_resume_fails() {
    let dir = folder();
    let world = World::new(dir.path());
    let args = world.args();
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let daemon = Daemon::start_with_env(common::Build::Admin, dir, &refs, &[]).await;
    world.remote("acme", "api");
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    let (id, _) = start_session(&daemon, &channel, "Fix login").await;
    common::wait_running(&daemon, &id).await;

    let plan = daemon.session_service()
        .await
        .plan_archive(pb::PlanArchiveRequest { id: Some(id.clone()) })
        .await
        .expect("the archive is planned")
        .into_inner();
    let archiving = daemon.session_service()
        .await
        .archive_session(pb::ArchiveSessionRequest {
            idempotency_key: None,
            id: Some(id.clone()),
            choice: pb::ArchiveChoice::Archive as i32,
            plan: plan.token,
            end: Some(true),
        })
        .await
        .expect("the archive is taken")
        .into_inner();
    let archived = finished(&daemon, &archiving).await;
    assert_eq!(archived.state, pb::OperationState::Succeeded as i32, "{archived:?}");

    std::fs::write(world.root.join("agent.sh"), EXITS_SOON_ON_RESUME).unwrap();
    let restoring = daemon.session_service()
        .await
        .restore_session(pb::RestoreSessionRequest {
            idempotency_key: None,
            id: Some(id.clone()),
            machine: None,
            stopped: false,
        })
        .await
        .expect("the restore is taken")
        .into_inner();
    let deadline = Instant::now() + DEADLINE;
    let seen = loop {
        let running: Vec<_> = running_holders(&daemon).await.into_iter().filter(|row| !row.stopped).collect();
        if let Some(row) = running.into_iter().next() {
            break row;
        }
        assert!(Instant::now() < deadline, "the restore's resume never started a holder");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(seen.session.as_ref(), Some(&id), "the resume's holder is listed as no session's: {seen:?}");

    let done = finished(&daemon, &restoring).await;
    assert_eq!(done.state, pb::OperationState::Failed as i32, "{done:?}");
    let left: Vec<_> = running_holders(&daemon).await.into_iter().filter(|row| !row.stopped).collect();
    assert!(left.is_empty(), "the failed restore left a holder running: {left:?}");
    let sessionless: Vec<_> = stored_tabs(&daemon.state)
        .into_iter()
        .filter(|record| record.session.is_none())
        .collect();
    assert!(sessionless.is_empty(), "the listing saved a tab with no session for the resume's holder: {sessionless:?}");
}
