//! `humatad2 serve --stdio` serves the peer link on its stdin and stdout,
//! as the Mac reaches a machine over SSH (GRPC.md §2, §8): PeerService and
//! what the Mac relays there, nothing else, and PeerService on no socket.

mod common;

use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use common::{Daemon, WAIT, channel_over};
use humata2_grpc::humata::v1 as pb;
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tonic::transport::Channel;
use tonic::{Code, Status};

fn binary(variable: &str) -> PathBuf {
    std::fs::canonicalize(PathBuf::from(std::env::var_os(variable).unwrap())).unwrap()
}

/// `serve --stdio` of the `variable` build, with its state in `dir`.
fn serve_stdio(variable: &str, dir: &Path) -> Command {
    let mut command = Command::new(binary(variable));
    command
        .args(["serve", "--stdio", "--state"])
        .arg(dir.join("state"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .kill_on_drop(true);
    command
}

/// The link of a `serve --stdio` started in a fresh folder.
async fn link(variable: &str) -> (TempDir, Child, Channel) {
    let dir = common::folder();
    let mut child = serve_stdio(variable, dir.path()).spawn().unwrap();
    let channel = channel_over(child.stdout.take().unwrap(), child.stdin.take().unwrap()).await;
    (dir, child, channel)
}

async fn exits(child: &mut Child) -> ExitStatus {
    tokio::time::timeout(WAIT, child.wait())
        .await
        .expect("serve --stdio kept running")
        .unwrap()
}

fn hello() -> pb::HelloRequest {
    pb::HelloRequest {
        expected_machine: None,
        host_api_major: 1,
        host_api_minor: 0,
        nonce: vec![7; 32],
        sync_marker: String::new(),
    }
}

/// Whether the method has a route on this endpoint: a service that is
/// registered but not built answers with a humata.v1.Error in the
/// details; a method with no route gets a bare UNIMPLEMENTED.
fn routed(status: &Status) -> bool {
    assert_eq!(status.code(), Code::Unimplemented, "{status:?}");
    !status.details().is_empty()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_sync_marker_comes_before_the_stream() {
    let dir = common::folder();
    let mut child = serve_stdio("HUMATAD2_RELEASE", dir.path())
        .args(["--sync", "7f3a"])
        .spawn()
        .unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    tokio::time::timeout(WAIT, stdout.read_line(&mut line))
        .await
        .expect("no sync marker")
        .unwrap();
    assert_eq!(line, "HUMATA_SYNC_7f3a\n");

    // Everything after the marker is the stream.
    let channel = channel_over(stdout, child.stdin.take().unwrap()).await;
    let info = pb::daemon_service_client::DaemonServiceClient::new(channel)
        .get_server_info(pb::GetServerInfoRequest {})
        .await
        .unwrap()
        .into_inner();
    assert_eq!(info.role(), pb::DaemonRole::Machine);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bad_sync_nonce_is_refused() {
    let dir = common::folder();
    let status = serve_stdio("HUMATAD2_RELEASE", dir.path())
        .args(["--sync", "not a nonce"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .unwrap();
    assert!(!status.success());
}

#[tokio::test(flavor = "multi_thread")]
async fn sync_needs_stdio() {
    let dir = common::folder();
    let status = Command::new(binary("HUMATAD2_RELEASE"))
        .args(["serve", "--sync", "7f3a", "--socket"])
        .arg(dir.path().join("d.sock"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .unwrap();
    assert!(!status.success());
}

#[tokio::test(flavor = "multi_thread")]
async fn peer_service_is_on_the_link() {
    let (_dir, _child, channel) = link("HUMATAD2_RELEASE").await;

    // Nothing of PeerService is built yet, but it is there.
    let not_built = pb::peer_service_client::PeerServiceClient::new(channel.clone())
        .hello(hello())
        .await
        .unwrap_err();
    assert!(routed(&not_built));

    // So is a service the Mac relays to the machine.
    let relayed = pb::terminal_service_client::TerminalServiceClient::new(channel)
        .list_tabs(pb::ListTabsRequest { session: None })
        .await
        .unwrap_err();
    assert!(routed(&relayed));
}

#[tokio::test(flavor = "multi_thread")]
async fn client_services_are_not_on_the_link() {
    let (_dir, _child, channel) = link("HUMATAD2_RELEASE").await;

    let inbox = pb::inbox_service_client::InboxServiceClient::new(channel.clone())
        .list_requests(pb::ListRequestsRequest::default())
        .await
        .unwrap_err();
    assert!(!routed(&inbox));

    let channels = pb::channel_service_client::ChannelServiceClient::new(channel)
        .list_channels(pb::ListChannelsRequest::default())
        .await
        .unwrap_err();
    assert!(!routed(&channels));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_link_is_never_the_user() {
    // The admin build takes a call with no credential for the user on its
    // socket; on the peer link it doesn't.
    let (_dir, _child, channel) = link("HUMATAD2").await;
    let me = pb::daemon_service_client::DaemonServiceClient::new(channel)
        .who_am_i(pb::WhoAmIRequest {})
        .await
        .unwrap()
        .into_inner();
    assert!(me.actor.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn admin_mode_is_refused_on_the_link() {
    let dir = common::folder();
    let status = serve_stdio("HUMATAD2", dir.path())
        .arg("--admin")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .unwrap();
    assert!(!status.success());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_socket_does_not_serve_peer_service() {
    let daemon = Daemon::start().await;
    let refused = pb::peer_service_client::PeerServiceClient::new(daemon.channel().await)
        .hello(hello())
        .await
        .unwrap_err();
    assert!(!routed(&refused));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_link_ends_when_stdin_closes() {
    let dir = common::folder();
    let mut child = serve_stdio("HUMATAD2_RELEASE", dir.path()).spawn().unwrap();
    let stdin = child.stdin.take().unwrap();
    // Give it time to start serving before hanging up.
    tokio::time::sleep(Duration::from_millis(200)).await;
    drop(stdin);
    assert!(exits(&mut child).await.success());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_link_ends_when_the_mac_hangs_up_after_calls() {
    let (_dir, mut child, channel) = link("HUMATAD2_RELEASE").await;
    pb::daemon_service_client::DaemonServiceClient::new(channel.clone())
        .get_server_info(pb::GetServerInfoRequest {})
        .await
        .unwrap();
    drop(channel);
    assert!(exits(&mut child).await.success());
}
