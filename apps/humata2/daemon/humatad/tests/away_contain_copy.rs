//! A machine that has not been sent the host's rules yet never starts an
//! agent on its own say-so (D266, SEC-6). Contain agents lives in the
//! policy copy, and a machine with no copy would read it as off. A real
//! role-machine daemon is linked by a test that plays the host, so the test
//! decides when the copy is sent.

mod common;
#[path = "common/away.rs"]
mod away;

use std::time::Duration;

use away::{Host, host_reachable_becomes};
use common::{Build, Daemon, World, folder};
use humata2_grpc::humata::v1 as pb;
use humata2_store::Store;
use tonic::Code;

/// A linked machine the host has sent nothing to yet, with a channel on it.
async fn linked_without_a_copy() -> (Daemon, Host, pb::Channel) {
    let dir = folder();
    let world = World::new(dir.path());
    let args = world.args();
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let daemon = away::machine_with(Build::Admin, dir, &refs).await;
    let host = Host::link(&daemon).await;
    host.resume(0).await;
    host_reachable_becomes(&daemon, true).await;
    // A ready clone, so a start gets as far as the host's rules.
    world.remote("acme", "api");
    let (channel, _) = common::create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    (daemon, host, channel)
}

/// A start of "Fix it" on this machine that waits at most `limit` for the
/// copy.
async fn start_within(
    daemon: &Daemon,
    channel: &pb::Channel,
    limit: Duration,
) -> Result<tonic::Response<pb::StartSessionResponse>, tonic::Status> {
    let machine = common::machine_id(daemon).await;
    let mut request = tonic::Request::new(common::start_request(channel.id.as_ref().unwrap(), &machine, "Fix it"));
    request.set_timeout(limit);
    daemon.session_service().await.start_session(request).await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_start_before_the_hosts_rules_arrive_is_refused_and_nothing_is_started() {
    let (daemon, _host, channel) = linked_without_a_copy().await;

    let refused = start_within(&daemon, &channel, Duration::from_secs(2))
        .await
        .expect_err("no agent starts before this machine knows whether to contain it");

    assert_eq!(refused.code(), Code::FailedPrecondition);
    let store = Store::open(daemon.state.join("store")).unwrap();
    assert!(store.sessions().unwrap().is_empty());
    assert!(common::stored_tabs(&daemon.state).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_start_that_waits_goes_on_once_the_hosts_rules_arrive() {
    let (daemon, mut host, channel) = linked_without_a_copy().await;
    let starting = tokio::spawn({
        let (daemon_channel, channel) = (daemon.session_service().await, channel.clone());
        let machine = common::machine_id(&daemon).await;
        async move {
            let mut request =
                tonic::Request::new(common::start_request(channel.id.as_ref().unwrap(), &machine, "Fix it"));
            request.set_timeout(Duration::from_secs(60));
            let mut sessions = daemon_channel;
            sessions.start_session(request).await
        }
    });

    // The host sends its rules, with Contain agents off: the agent may run
    // as the host says.
    host.send_policy(1, pb::PolicySnapshot::default()).await;
    host.journal_until_policy_ack(1).await;

    let started = starting
        .await
        .unwrap()
        .expect("the start went on with the host's rules")
        .into_inner();
    common::wait_running(&daemon, started.session.as_ref().unwrap()).await;
}
