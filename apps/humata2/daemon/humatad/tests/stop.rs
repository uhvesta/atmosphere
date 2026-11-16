//! SIGTERM stops a daemon even while an app holds an event subscription
//! open: the subscription ends cleanly, and the daemon exits.

mod common;

use common::Daemon;
use humata2_grpc::humata::v1 as pb;
use tonic::Request;

#[tokio::test(flavor = "multi_thread")]
async fn sigterm_ends_an_open_event_subscription_and_the_daemon_exits() {
    let mut daemon = Daemon::start().await;
    let mut events = pb::event_service_client::EventServiceClient::new(daemon.channel().await)
        .subscribe(Request::new(pb::SubscribeRequest::default()))
        .await
        .unwrap()
        .into_inner();

    let status = daemon.terminate().await;

    assert!(status.success(), "the daemon exited with {status}");
    assert!(
        matches!(events.message().await, Ok(None)),
        "the subscription should end cleanly when the daemon stops"
    );
}
