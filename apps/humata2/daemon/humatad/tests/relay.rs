//! The peer link relays an allowlist of ordinary calls to the machine's
//! socket daemon (D139). A real role-machine daemon and a real
//! `serve --stdio` share one temp state folder and socket. Written against
//! the shape after the enable lines of the MACH relay report are applied:
//! `serve --stdio` takes `--socket`, the socket daemon serves
//! DaemonService.BeginRelay and CompleteRelay, and Hello offers the nonce
//! BeginRelay gave. Enabling this file is a move to `tests/relay.rs`.

use humatad_test_common as common;

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use common::{Build, Daemon};
use hmac::{Hmac, KeyInit, Mac};
use humata2_grpc::humata::v1 as pb;
use pb::peer_unproven::Reason;
use sha2::Sha256;
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::Channel;
use tonic::{Code, Status};

const KEY: [u8; 32] = [0x5a; 32];
const HOST_LABEL: &[u8] = b"humata-peer-host-v1";
const HOST_NONCE: [u8; 32] = [7; 32];

fn write_key(dir: &Path) {
    let path = common::state_folder(dir).join("enrollment-key");
    let hex: String = KEY.iter().map(|byte| format!("{byte:02x}")).collect();
    std::fs::write(&path, format!("{hex}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
}

/// The host's proof over the machine's nonce and its own.
fn host_proof(machine_nonce: &[u8]) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(&KEY).unwrap();
    mac.update(HOST_LABEL);
    mac.update(machine_nonce);
    mac.update(&HOST_NONCE);
    mac.finalize().into_bytes().to_vec()
}

/// A machine daemon of `build`, with the key seeded before it starts.
async fn machine_daemon(build: Build, role: &str) -> Daemon {
    let dir = common::folder();
    write_key(dir.path());
    Daemon::start_with(build, dir, &["--role", role]).await
}

/// `serve --stdio` of the release build over `daemon`'s state and socket.
async fn link(daemon: &Daemon) -> (Child, Channel) {
    let mut child = Command::new(common::binary("HUMATAD2_RELEASE"))
        .args(["serve", "--stdio", "--state"])
        .arg(&daemon.state)
        .arg("--socket")
        .arg(&daemon.socket)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let channel = common::channel_over(child.stdout.take().unwrap(), child.stdin.take().unwrap()).await;
    (child, channel)
}

fn hello() -> pb::HelloRequest {
    pb::HelloRequest {
        expected_machine: None,
        host_api_major: 1,
        host_api_minor: 0,
        nonce: HOST_NONCE.to_vec(),
        sync_marker: String::new(),
        chunk_codecs: Vec::new(),
        host_build: None,
    }
}

fn peer(channel: &Channel) -> pb::peer_service_client::PeerServiceClient<Channel> {
    pb::peer_service_client::PeerServiceClient::new(channel.clone())
}

fn sessions(channel: &Channel) -> pb::session_service_client::SessionServiceClient<Channel> {
    pb::session_service_client::SessionServiceClient::new(channel.clone())
}

fn daemon_service(channel: &Channel) -> pb::daemon_service_client::DaemonServiceClient<Channel> {
    pb::daemon_service_client::DaemonServiceClient::new(channel.clone())
}

/// Hello and ProveHost on `channel`.
async fn prove_host(channel: &Channel) {
    let answer = peer(channel).hello(hello()).await.unwrap().into_inner();
    peer(channel)
        .prove_host(pb::ProveHostRequest {
            proof: host_proof(&answer.machine_nonce),
        })
        .await
        .unwrap();
}

fn peer_unproven(status: &Status) -> Reason {
    assert_eq!(status.code(), Code::Unauthenticated, "{status:?}");
    match common::error_of(status).kind {
        Some(pb::error::Kind::PeerUnproven(refused)) => refused.reason(),
        other => panic!("expected peer_unproven, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn before_prove_host_a_relayed_call_is_peer_unproven() {
    let daemon = machine_daemon(Build::Release, "machine").await;
    let (_child, channel) = link(&daemon).await;
    let refused = sessions(&channel)
        .list_sessions(pb::ListSessionsRequest::default())
        .await
        .unwrap_err();
    assert_eq!(peer_unproven(&refused), Reason::HostNotProven);

    // Hello alone doesn't open it either.
    peer(&channel).hello(hello()).await.unwrap();
    let refused = sessions(&channel)
        .list_sessions(pb::ListSessionsRequest::default())
        .await
        .unwrap_err();
    assert_eq!(peer_unproven(&refused), Reason::HostNotProven);
}

#[tokio::test(flavor = "multi_thread")]
async fn after_prove_host_list_sessions_answers_what_the_socket_daemon_answers() {
    let daemon = machine_daemon(Build::Admin, "machine").await;
    let direct = sessions(&daemon.channel().await)
        .list_sessions(pb::ListSessionsRequest::default())
        .await
        .unwrap()
        .into_inner();
    let (_child, channel) = link(&daemon).await;
    prove_host(&channel).await;
    let relayed = sessions(&channel)
        .list_sessions(pb::ListSessionsRequest::default())
        .await
        .unwrap()
        .into_inner();
    assert_eq!(relayed, direct);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_method_off_the_allowlist_is_refused_on_a_proven_link() {
    let daemon = machine_daemon(Build::Admin, "machine").await;
    let (_child, channel) = link(&daemon).await;
    prove_host(&channel).await;
    let refused = sessions(&channel)
        .delete_session(pb::DeleteSessionRequest::default())
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::PermissionDenied);
}

#[tokio::test(flavor = "multi_thread")]
async fn on_the_socket_a_wrong_proof_is_refused_and_the_connection_stays_ordinary() {
    let daemon = machine_daemon(Build::Release, "machine").await;
    let channel = daemon.channel().await;
    let begun = daemon_service(&channel)
        .begin_relay(pb::BeginRelayRequest {})
        .await
        .unwrap()
        .into_inner();
    assert_eq!(begun.nonce.len(), 32);
    // A proof for some other nonce, as a replay from another connection.
    let refused = daemon_service(&channel)
        .complete_relay(pb::CompleteRelayRequest {
            host_nonce: HOST_NONCE.to_vec(),
            proof: host_proof(&[1; 32]),
        })
        .await
        .unwrap_err();
    assert_ne!(refused.code(), Code::Ok);
    // Still no credential, still refused as before.
    let unauthenticated = sessions(&channel)
        .list_sessions(pb::ListSessionsRequest::default())
        .await
        .unwrap_err();
    assert_eq!(unauthenticated.code(), Code::Unauthenticated);
}

#[tokio::test(flavor = "multi_thread")]
async fn on_the_socket_a_proven_connection_takes_only_allowlisted_calls() {
    let daemon = machine_daemon(Build::Release, "machine").await;
    // A fresh connection, so nothing else shares its state.
    let channel = common::connect(&daemon.socket).await;
    let begun = daemon_service(&channel)
        .begin_relay(pb::BeginRelayRequest {})
        .await
        .unwrap()
        .into_inner();
    daemon_service(&channel)
        .complete_relay(pb::CompleteRelayRequest {
            host_nonce: HOST_NONCE.to_vec(),
            proof: host_proof(&begun.nonce),
        })
        .await
        .unwrap();
    sessions(&channel)
        .list_sessions(pb::ListSessionsRequest::default())
        .await
        .unwrap();
    let off_list = sessions(&channel)
        .delete_session(pb::DeleteSessionRequest::default())
        .await
        .unwrap_err();
    assert_eq!(off_list.code(), Code::PermissionDenied);

    // The proof belongs to that connection: another one is still ordinary.
    let other = common::connect(&daemon.socket).await;
    let unauthenticated = sessions(&other)
        .list_sessions(pb::ListSessionsRequest::default())
        .await
        .unwrap_err();
    assert_eq!(unauthenticated.code(), Code::Unauthenticated);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_host_role_daemon_refuses_to_complete() {
    let daemon = machine_daemon(Build::Release, "host").await;
    let channel = daemon.channel().await;
    let begun = daemon_service(&channel)
        .begin_relay(pb::BeginRelayRequest {})
        .await
        .unwrap()
        .into_inner();
    let refused = daemon_service(&channel)
        .complete_relay(pb::CompleteRelayRequest {
            host_nonce: HOST_NONCE.to_vec(),
            proof: host_proof(&begun.nonce),
        })
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::PermissionDenied);
}

/// The Uplink of a proven link, and the sender that keeps the host's side
/// of it open.
async fn uplink(
    channel: &Channel,
) -> (mpsc::Sender<pb::HostToMachine>, tonic::Streaming<pb::MachineToHost>) {
    let (to_machine, outgoing) = mpsc::channel(1);
    let from_machine = peer(channel)
        .uplink(ReceiverStream::new(outgoing))
        .await
        .unwrap()
        .into_inner();
    (to_machine, from_machine)
}

async fn next(stream: &mut tonic::Streaming<pb::MachineToHost>) -> pb::machine_to_host::Message {
    tokio::time::timeout(Duration::from_secs(10), stream.message())
        .await
        .expect("the machine sent a message")
        .unwrap()
        .expect("the uplink stayed open")
        .message
        .expect("a message of some kind")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_machine_with_no_sessions_sends_resume_then_the_end_of_its_snapshot_and_nothing_more() {
    let daemon = machine_daemon(Build::Admin, "machine").await;
    let (_child, channel) = link(&daemon).await;
    prove_host(&channel).await;
    let (_keep_open, mut from_machine) = uplink(&channel).await;
    assert!(matches!(next(&mut from_machine).await, pb::machine_to_host::Message::Resume(_)));
    assert!(matches!(next(&mut from_machine).await, pb::machine_to_host::Message::Ack(_)));
    let more = tokio::time::timeout(Duration::from_millis(500), from_machine.message()).await;
    assert!(more.is_err(), "nothing else is sent while nothing changes");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_link_with_no_socket_sends_resume_and_nothing_else() {
    let dir = common::folder();
    write_key(dir.path());
    let mut child = Command::new(common::binary("HUMATAD2_RELEASE"))
        .args(["serve", "--stdio", "--state"])
        .arg(common::state_folder(dir.path()))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let channel = common::channel_over(child.stdout.take().unwrap(), child.stdin.take().unwrap()).await;
    prove_host(&channel).await;
    let (_keep_open, mut from_machine) = uplink(&channel).await;
    assert!(matches!(next(&mut from_machine).await, pb::machine_to_host::Message::Resume(_)));
    let more = tokio::time::timeout(Duration::from_millis(500), from_machine.message()).await;
    assert!(more.is_err(), "a link with no relay reports nothing");
}
