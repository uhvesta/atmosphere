//! The fault build of the daemon (`humatad2_faults`): a test can trace,
//! stop and power-cut it at the points of the daemon and of the crates
//! under it, which the plain admin daemon can't reach. A start whose
//! environment has `HUMATA2_STOP_AT`, `HUMATA2_FAULT_TRACE` and the like
//! gets that build by itself (tests/common).

use humatad_test_common as common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use common::{Build, Daemon, World, create_channel, folder, get_session, start_session, wait_running};
use humata2_grpc::humata::v1 as pb;
use rustix::process::{Pid, Signal};

const LONG: Duration = Duration::from_secs(60);

/// A daemon serving a world, started with `env`.
async fn daemon_with(env: &[(&str, &str)]) -> (Daemon, World) {
    let dir = folder();
    let world = World::new(dir.path());
    world.remote("acme", "api");
    let args = world.args();
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    (Daemon::start_with_env(Build::Admin, dir, &refs, env).await, world)
}

fn text(path: &Path) -> &str {
    path.to_str().expect("a UTF-8 path")
}

/// The point names written to `trace`, in order.
fn traced(trace: &Path) -> Vec<String> {
    std::fs::read_to_string(trace)
        .map(|lines| lines.lines().map(str::to_owned).collect())
        .unwrap_or_default()
}

fn fork_request(parent: &pb::SessionId) -> pb::ForkSessionRequest {
    pb::ForkSessionRequest {
        idempotency_key: None,
        parent: Some(parent.clone()),
        kind: pb::ForkKind::CodeOnly as i32,
        before_turn: 0,
        title: "Child".to_owned(),
        agent: None,
        model: String::new(),
        machine: None,
        branch: String::new(),
        reason: String::new(),
    }
}

/// Asks for a fork and doesn't care how the call ends: the daemon may die
/// under it.
async fn fork_ignoring_the_end(daemon: &Daemon, parent: &pb::SessionId) {
    let _ = daemon.session_service().await.fork_session(fork_request(parent)).await;
}

/// Asks for a session and doesn't care how the call ends.
async fn start_ignoring_the_end(daemon: &Daemon, channel: &pb::Channel, title: &str) {
    let machine = common::machine_id(daemon).await;
    let request = common::start_request(channel.id.as_ref().expect("a channel id"), &machine, title);
    let _ = daemon.session_service().await.start_session(request).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_trace_lists_the_points_of_the_daemon_and_of_its_crates_reached_once_armed() {
    let scratch = folder();
    let (trace, arm) = (scratch.path().join("trace"), scratch.path().join("arm"));
    let (daemon, _world) =
        daemon_with(&[("HUMATA2_FAULT_TRACE", text(&trace)), ("HUMATA2_FAULT_ARM", text(&arm))]).await;
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    let (id, _) = start_session(&daemon, &channel, "Fix login").await;
    wait_running(&daemon, &id).await;
    assert!(traced(&trace).is_empty(), "nothing is traced before the arm file exists");

    std::fs::write(&arm, b"").expect("the arm file is written");
    let operation = daemon.session_service().await.fork_session(fork_request(&id)).await.expect("the fork is asked for");
    common::finished(&daemon, &operation.into_inner()).await;

    let points = traced(&trace);
    assert!(points.contains(&"fork.verify".to_owned()), "a point of the daemon: {points:?}");
    assert!(points.contains(&"sessions.fork.recorded".to_owned()), "a point of the sessions crate: {points:?}");
    assert!(
        !points.contains(&"sessions.start.recorded".to_owned()),
        "the start before the arm file is not traced: {points:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stop_at_a_point_of_the_sessions_crate_kills_the_daemon_there() {
    let (mut daemon, _world) = daemon_with(&[("HUMATA2_STOP_AT", "sessions.fork.recorded")]).await;
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    let (id, _) = start_session(&daemon, &channel, "Fix login").await;
    wait_running(&daemon, &id).await;

    fork_ignoring_the_end(&daemon, &id).await;

    daemon.stopped(LONG).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn hits_before_the_arm_file_exists_do_not_count_toward_the_stop() {
    let scratch = folder();
    let arm = scratch.path().join("arm");
    let (mut daemon, _world) =
        daemon_with(&[("HUMATA2_STOP_AT", "sessions.start.recorded"), ("HUMATA2_FAULT_ARM", text(&arm))]).await;
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    // Reaches the point, unarmed: the daemon lives and the session runs.
    let (first, _) = start_session(&daemon, &channel, "First").await;
    wait_running(&daemon, &first).await;

    std::fs::write(&arm, b"").expect("the arm file is written");
    start_ignoring_the_end(&daemon, &channel, "Second").await;

    daemon.stopped(LONG).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn sigusr2_power_cuts_the_daemon_and_what_was_synced_comes_back() {
    let dir = folder();
    let state: PathBuf = common::state_folder(dir.path());
    let spare = dir.path().join("spare");
    let world = World::new(dir.path());
    world.remote("acme", "api");
    let args = world.args();
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut daemon = Daemon::start_with_env(
        Build::Admin,
        dir,
        &refs,
        &[("HUMATA2_POWER_CUT_ROOTS", text(&state)), ("HUMATA2_POWER_CUT_SPARE", text(&spare))],
    )
    .await;
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    let (id, _) = start_session(&daemon, &channel, "Fix login").await;
    wait_running(&daemon, &id).await;

    let pid = daemon.pid().and_then(|raw| i32::try_from(raw).ok()).and_then(Pid::from_raw).expect("a daemon process");
    rustix::process::kill_process(pid, Signal::USR2).expect("SIGUSR2 reaches the daemon");
    daemon.stopped(LONG).await;

    // Started again without the cut, the session is there as it was.
    daemon.restart().await;
    let session = get_session(&daemon, &id).await;
    assert_eq!(session.id, Some(id));
}
