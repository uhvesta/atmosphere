//! Whether the Mac is reachable from a machine (AWAY T3). A machine's socket
//! daemon counts its host reachable exactly while the peer link holds a
//! DaemonService.PeerUplink open on its proven relay connection. A real
//! role-machine daemon and a real `serve --stdio` share one temp state
//! folder and socket, as in `relay.rs`.

use humatad_test_common as common;

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use common::{Build, Daemon, server_info};
use hmac::{Hmac, KeyInit, Mac};
use humata2_grpc::humata::v1 as pb;
use sha2::Sha256;
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::Channel;
use tonic::{Code, Status, Streaming};

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

/// The Uplink of a proven link, and the sender that keeps the host's side
/// of it open.
async fn uplink(
    channel: &Channel,
) -> (mpsc::Sender<pb::HostToMachine>, Streaming<pb::MachineToHost>) {
    let (to_machine, outgoing) = mpsc::channel(1);
    let from_machine = peer(channel)
        .uplink(ReceiverStream::new(outgoing))
        .await
        .unwrap()
        .into_inner();
    (to_machine, from_machine)
}

async fn next(stream: &mut Streaming<pb::MachineToHost>) -> pb::machine_to_host::Message {
    tokio::time::timeout(Duration::from_secs(10), stream.message())
        .await
        .expect("the machine sent a message")
        .unwrap()
        .expect("the uplink stayed open")
        .message
        .expect("a message of some kind")
}

/// How the host's Uplink ended, reading whatever the machine sent before
/// that: the status it ended with, or none for a clean end.
async fn uplink_end(stream: &mut Streaming<pb::MachineToHost>) -> Option<Status> {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match stream.message().await {
                Ok(Some(_)) => {}
                Ok(None) => return None,
                Err(status) => return Some(status),
            }
        }
    })
    .await
    .expect("the uplink ended")
}

/// Waits until the machine's `host_reachable` is `want`. The event feed is
/// subscribed before the state is read, so a change between the two is
/// seen on the feed.
async fn host_reachable_becomes(daemon: &Daemon, want: bool) {
    let mut feed = pb::event_service_client::EventServiceClient::new(daemon.channel().await)
        .subscribe(pb::SubscribeRequest {
            topics: vec![pb::Topic::Machines as i32],
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    if server_info(daemon).await.host_reachable == want {
        return;
    }
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let event = feed.message().await.unwrap().expect("the event feed stayed open");
            if matches!(
                &event.payload,
                Some(pb::event::Payload::HostLink(changed)) if changed.reachable == want
            ) {
                return;
            }
        }
    })
    .await
    .expect("the machine's view of its host never changed");
    assert_eq!(server_info(daemon).await.host_reachable, want);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_machine_is_away_until_the_host_opens_its_uplink_and_reachable_once_it_has() {
    let daemon = machine_daemon(Build::Admin, "machine").await;
    assert!(!server_info(&daemon).await.host_reachable);

    let (_child, channel) = link(&daemon).await;
    prove_host(&channel).await;
    let (_keep_open, mut from_machine) = uplink(&channel).await;
    assert!(matches!(next(&mut from_machine).await, pb::machine_to_host::Message::Resume(_)));
    // Resume comes after the daemon took the uplink, so no waiting is needed.
    assert!(server_info(&daemon).await.host_reachable);
}

#[tokio::test(flavor = "multi_thread")]
async fn ending_the_uplink_makes_the_machine_away_again() {
    let daemon = machine_daemon(Build::Admin, "machine").await;
    let (_child, channel) = link(&daemon).await;
    prove_host(&channel).await;
    let (keep_open, mut from_machine) = uplink(&channel).await;
    assert!(matches!(next(&mut from_machine).await, pb::machine_to_host::Message::Resume(_)));
    assert!(server_info(&daemon).await.host_reachable);

    drop(keep_open);
    drop(from_machine);
    host_reachable_becomes(&daemon, false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_link_replaces_the_first_and_the_machine_stays_reachable() {
    let daemon = machine_daemon(Build::Admin, "machine").await;
    let (_first_child, first) = link(&daemon).await;
    prove_host(&first).await;
    let (_first_keep_open, mut first_from) = uplink(&first).await;
    assert!(matches!(next(&mut first_from).await, pb::machine_to_host::Message::Resume(_)));

    let (_second_child, second) = link(&daemon).await;
    prove_host(&second).await;
    let (_second_keep_open, mut second_from) = uplink(&second).await;
    assert!(matches!(next(&mut second_from).await, pb::machine_to_host::Message::Resume(_)));

    // The newer PeerUplink replaced the first, which ends with an error.
    let ended = uplink_end(&mut first_from)
        .await
        .expect("the replaced uplink ended with an error");
    assert_eq!(ended.code(), Code::Aborted);
    assert!(server_info(&daemon).await.host_reachable);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_link_whose_machines_daemon_is_gone_ends_so_the_host_dials_a_new_one() {
    let mut daemon = machine_daemon(Build::Admin, "machine").await;
    let (mut child, channel) = link(&daemon).await;
    prove_host(&channel).await;
    let (_keep_open, mut from_machine) = uplink(&channel).await;
    assert!(matches!(next(&mut from_machine).await, pb::machine_to_host::Message::Resume(_)));

    // A daemon that stops (a crash, a restart) takes the link's one relayed
    // connection with it, so the link can carry nothing more. The host's
    // Uplink ends though the host keeps its side open, and the link process
    // exits, so the host dials a link to the daemon that runs next.
    daemon.kill();
    uplink_end(&mut from_machine).await;
    let exited = tokio::time::timeout(Duration::from_secs(10), child.wait()).await;
    assert!(exited.is_ok(), "the link process outlived its machine's daemon");
}

#[tokio::test(flavor = "multi_thread")]
async fn only_the_proven_link_may_hold_the_uplink() {
    let daemon = machine_daemon(Build::Admin, "machine").await;
    let channel = daemon.channel().await;
    let (_keep_open, outgoing) = mpsc::channel::<pb::HostToMachine>(1);
    let refused = daemon_service(&channel)
        .peer_uplink(ReceiverStream::new(outgoing))
        .await
        .err()
        .expect("the socket daemon refused a PeerUplink from no proven link");
    assert_eq!(refused.code(), Code::PermissionDenied);
    assert!(!server_info(&daemon).await.host_reachable);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_host_daemon_is_always_reachable() {
    let daemon = machine_daemon(Build::Admin, "host").await;
    assert!(server_info(&daemon).await.host_reachable);
}
