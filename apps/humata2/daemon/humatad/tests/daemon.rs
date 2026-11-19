//! DaemonService, and what services not built yet answer.

use humatad_test_common as common;

use common::{Daemon, error_of};
use humata2_grpc::humata::v1 as pb;
use tonic::Code;

async fn server_info(daemon: &Daemon) -> pb::ServerInfo {
    pb::daemon_service_client::DaemonServiceClient::new(daemon.channel().await)
        .get_server_info(pb::GetServerInfoRequest {})
        .await
        .unwrap()
        .into_inner()
}

#[tokio::test(flavor = "multi_thread")]
async fn server_info_names_the_api_and_a_machine_that_stays_the_same() {
    let mut daemon = Daemon::start().await;
    let before = server_info(&daemon).await;

    daemon.restart().await;
    let after = server_info(&daemon).await;

    assert_eq!(before.api_major, 1);
    assert_eq!(before.role(), pb::DaemonRole::Host);
    assert!(
        before
            .machine
            .as_ref()
            .is_some_and(|machine| !machine.value.is_empty())
    );
    assert_eq!(before.machine, after.machine);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_service_not_built_yet_says_so_with_an_error() {
    let daemon = Daemon::start().await;

    let refused = pb::activity_service_client::ActivityServiceClient::new(daemon.channel().await)
        .list_activity(pb::ListActivityRequest::default())
        .await
        .unwrap_err();

    assert_eq!(refused.code(), Code::Unimplemented);
    let error = error_of(&refused);
    assert!(!error.message.is_empty());
    assert!(!error.fix.is_empty());
}
