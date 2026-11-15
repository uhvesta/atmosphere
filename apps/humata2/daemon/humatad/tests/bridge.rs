//! `humatad2 bridge` carries the API over its stdin and stdout, the way SSH
//! would.

mod common;

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use common::{Daemon, error_of};
use humata2_grpc::humata::v1 as pb;
use hyper_util::rt::TokioIo;
use tokio::process::Command;
use tonic::transport::{Endpoint, Uri};

#[tokio::test(flavor = "multi_thread")]
async fn grpc_works_through_the_bridge() {
    let daemon = Daemon::start().await;
    let mut bridge = Command::new(std::fs::canonicalize(PathBuf::from(
        std::env::var_os("HUMATAD2").unwrap(),
    ))
    .unwrap())
    .arg("bridge")
    .arg("--socket")
    .arg(&daemon.socket)
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .kill_on_drop(true)
    .spawn()
    .unwrap();
    let stdio = tokio::io::join(bridge.stdout.take().unwrap(), bridge.stdin.take().unwrap());
    let stdio = Arc::new(Mutex::new(Some(stdio)));
    let channel = Endpoint::try_from("http://[::]:50051")
        .unwrap()
        .connect_with_connector(tower::service_fn(move |_: Uri| {
            let stdio = stdio.lock().unwrap().take();
            async move {
                stdio
                    .map(TokioIo::new)
                    .ok_or_else(|| std::io::Error::other("the bridge carries one connection"))
            }
        }))
        .await
        .unwrap();

    let info = pb::daemon_service_client::DaemonServiceClient::new(channel.clone())
        .get_server_info(pb::GetServerInfoRequest {})
        .await
        .unwrap()
        .into_inner();
    assert_eq!(info.api_major, 1);

    // A failure's details make it through as well.
    let refused = pb::channel_service_client::ChannelServiceClient::new(channel)
        .list_channels(pb::ListChannelsRequest::default())
        .await
        .unwrap_err();
    assert!(!error_of(&refused).message.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bridge_to_no_daemon_fails() {
    let dir = common::folder();

    let status = Command::new(std::fs::canonicalize(PathBuf::from(
        std::env::var_os("HUMATAD2").unwrap(),
    ))
    .unwrap())
    .arg("bridge")
    .arg("--socket")
    .arg(dir.path().join("none.sock"))
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .status()
    .await
    .unwrap();

    assert!(!status.success());
}
