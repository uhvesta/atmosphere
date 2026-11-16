//! One state folder, one daemon: a second daemon on a state folder that a
//! live one holds is refused, and the first one keeps serving.

mod common;

use common::{Daemon, folder, serve_and_exit};
use humata2_grpc::humata::v1 as pb;

#[tokio::test(flavor = "multi_thread")]
async fn a_second_daemon_on_a_state_in_use_is_refused_and_the_first_keeps_serving() {
    let first = Daemon::start().await;
    let second = folder();

    let (status, stderr) = serve_and_exit(second.path(), &first.state).await;

    assert!(!status.success(), "a second daemon started: {stderr}");
    let pid = first.pid().expect("the first daemon runs");
    assert!(
        stderr.contains(&pid.to_string()),
        "the refusal should name the running daemon's pid {pid}: {stderr}"
    );
    assert!(
        !second.path().join("second.sock").exists(),
        "the refused daemon bound a socket"
    );
    let info = pb::daemon_service_client::DaemonServiceClient::new(first.channel().await)
        .get_server_info(pb::GetServerInfoRequest {})
        .await
        .unwrap()
        .into_inner();
    assert_eq!(info.api_major, 1);
}
