//! The policy copy on a machine (AWAY T4, AW-8). A real role-machine daemon
//! is sent a PolicySnapshot over its Uplink, as the host's peer link sends
//! it; the machine acks its revision, judges by it, keeps it across a
//! restart, and nothing on the machine changes it.

mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use common::{Build, Daemon};
use hmac::{Hmac, KeyInit, Mac};
use humata2_grpc::humata::v1 as pb;
use sha2::Sha256;
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::Streaming;
use tonic::transport::Channel;

const KEY: [u8; 32] = [0x5a; 32];
const HOST_LABEL: &[u8] = b"humata-peer-host-v1";
const HOST_NONCE: [u8; 32] = [7; 32];

fn write_key(dir: &Path) {
    let path = common::state_folder(dir).join("enrollment-key");
    let hex: String = KEY.iter().map(|byte| format!("{byte:02x}")).collect();
    std::fs::write(&path, format!("{hex}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
}

fn host_proof(machine_nonce: &[u8]) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(&KEY).unwrap();
    mac.update(HOST_LABEL);
    mac.update(machine_nonce);
    mac.update(&HOST_NONCE);
    mac.finalize().into_bytes().to_vec()
}

async fn machine_daemon() -> Daemon {
    let dir = common::folder();
    write_key(dir.path());
    Daemon::start_with(Build::Admin, dir, &["--role", "machine"]).await
}

/// `serve --stdio` over `daemon`'s state and socket: the link process.
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

/// A proven link's Uplink, as the host holds it, once the machine resumed.
async fn uplink(channel: &Channel) -> (mpsc::Sender<pb::HostToMachine>, Streaming<pb::MachineToHost>) {
    let mut peer = pb::peer_service_client::PeerServiceClient::new(channel.clone());
    let answer = peer
        .hello(pb::HelloRequest {
            expected_machine: None,
            host_api_major: 1,
            host_api_minor: 0,
            nonce: HOST_NONCE.to_vec(),
            sync_marker: String::new(),
            chunk_codecs: Vec::new(),
        })
        .await
        .unwrap()
        .into_inner();
    peer.prove_host(pb::ProveHostRequest {
        proof: host_proof(&answer.machine_nonce),
    })
    .await
    .unwrap();
    let (to_machine, outgoing) = mpsc::channel(4);
    let mut from_machine = peer.uplink(ReceiverStream::new(outgoing)).await.unwrap().into_inner();
    // Resume comes once the daemon holds the uplink: the host is reachable.
    let resume = tokio::time::timeout(Duration::from_secs(10), from_machine.message())
        .await
        .expect("the machine resumed")
        .unwrap()
        .and_then(|message| message.message);
    assert!(matches!(resume, Some(pb::machine_to_host::Message::Resume(_))), "{resume:?}");
    (to_machine, from_machine)
}

/// The policy revision the machine acks next, skipping its other messages.
/// An ack with no revision ends the session reports, so it is skipped too.
async fn acked(stream: &mut Streaming<pb::MachineToHost>) -> u64 {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let message = stream.message().await.unwrap().expect("the uplink stayed open");
            if let Some(pb::machine_to_host::Message::Ack(ack)) = message.message
                && ack.policy_revision != 0
            {
                return ack.policy_revision;
            }
        }
    })
    .await
    .expect("the machine acked the policy copy")
}

/// A copy whose new channels start with While I'm away at `mode`.
fn snapshot(revision: u64, mode: pb::AwayMode) -> pb::HostToMachine {
    pb::HostToMachine {
        message: Some(pb::host_to_machine::Message::Policy(pb::PolicySnapshot {
            revision,
            defaults: Some(pb::DefaultRules {
                rules: Vec::new(),
                away: Some(pb::AwayRule {
                    mode: mode as i32,
                    low_risk: Vec::new(),
                }),
                revision: None,
            }),
            ..Default::default()
        })),
    }
}

fn access(channel: &Channel) -> pb::access_service_client::AccessServiceClient<Channel> {
    pb::access_service_client::AccessServiceClient::new(channel.clone())
}

async fn away_mode(daemon: &Daemon) -> i32 {
    let rules = access(&daemon.channel().await)
        .get_default_rules(pb::GetDefaultRulesRequest {})
        .await
        .unwrap()
        .into_inner();
    rules.away.map_or(0, |away| away.mode)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_machine_takes_the_hosts_copy_and_keeps_it_across_a_restart() {
    // The rules are read while the host is reachable: reading them is the
    // user's AccessChange, which needs the Mac.
    let mut daemon = machine_daemon().await;
    let (child, channel) = link(&daemon).await;
    let (to_machine, mut from_machine) = uplink(&channel).await;
    assert_eq!(away_mode(&daemon).await, pb::AwayMode::Wait as i32);
    to_machine.send(snapshot(7, pb::AwayMode::Skip)).await.unwrap();
    assert_eq!(acked(&mut from_machine).await, 7);
    assert_eq!(away_mode(&daemon).await, pb::AwayMode::Skip as i32);

    drop((to_machine, from_machine, child));
    daemon.restart().await;
    // A new link that sends no copy: the machine judges by the one it kept.
    let (_child, channel) = link(&daemon).await;
    let (_to_machine, _from_machine) = uplink(&channel).await;
    assert_eq!(away_mode(&daemon).await, pb::AwayMode::Skip as i32);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_rules_cannot_be_changed_on_a_machine() {
    let daemon = machine_daemon().await;
    let (_child, channel) = link(&daemon).await;
    let (to_machine, mut from_machine) = uplink(&channel).await;
    to_machine.send(snapshot(3, pb::AwayMode::Skip)).await.unwrap();
    assert_eq!(acked(&mut from_machine).await, 3);

    let changed = access(&daemon.channel().await)
        .set_default_rules(pb::SetDefaultRulesRequest {
            away: Some(pb::AwayRule {
                mode: pb::AwayMode::Wait as i32,
                low_risk: Vec::new(),
            }),
            ..Default::default()
        })
        .await;
    assert!(changed.is_err(), "a machine changed its copy of the rules: {changed:?}");
    assert_eq!(away_mode(&daemon).await, pb::AwayMode::Skip as i32);
}
