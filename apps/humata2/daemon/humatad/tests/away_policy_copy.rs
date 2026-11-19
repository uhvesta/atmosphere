//! The policy copy on a machine (AWAY T4, AW-8). A real role-machine daemon
//! is sent a PolicySnapshot over its Uplink, as the host's peer link sends
//! it; the machine acks its revision, judges by it, keeps it across a
//! restart, and nothing on the machine changes it.

use humatad_test_common as common;
use humatad_test_common::away;

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
            host_build: None,
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
                copy: None,
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

#[tokio::test(flavor = "multi_thread")]
async fn keeping_a_copy_is_reported_on_the_machines_feed() {
    let daemon = machine_daemon().await;
    let (_child, channel) = link(&daemon).await;
    let mut feed = pb::event_service_client::EventServiceClient::new(daemon.channel().await)
        .subscribe(pb::SubscribeRequest {
            topics: vec![pb::Topic::Machines as i32],
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    let (to_machine, mut from_machine) = uplink(&channel).await;
    to_machine.send(snapshot(5, pb::AwayMode::Skip)).await.unwrap();
    assert_eq!(acked(&mut from_machine).await, 5);

    let kept = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let event = feed.message().await.unwrap().expect("the event feed stayed open");
            if let Some(pb::event::Payload::PolicyCopy(kept)) = event.payload {
                return kept.revision;
            }
        }
    })
    .await
    .expect("the machine never reported its copy kept");
    assert_eq!(kept, 5);
}

const CHANNEL: &str = "ch_away";

/// A copy holding channel `ch_away`, with session notes Never and While I'm
/// away at Skip, and new channels at Skip too.
fn snapshot_with_channel(revision: u64) -> pb::HostToMachine {
    let skip = pb::AwayRule {
        mode: pb::AwayMode::Skip as i32,
        low_risk: Vec::new(),
    };
    let Some(pb::host_to_machine::Message::Policy(mut copy)) =
        snapshot(revision, pb::AwayMode::Skip).message
    else {
        panic!("snapshot makes a policy copy")
    };
    let mut at = copy.at.take().unwrap_or_default();
    at.seconds = 1_794_657_600;
    copy.at = Some(at);
    copy.channels.push(pb::ChannelRules {
        channel: Some(pb::ChannelId { value: CHANNEL.to_owned() }),
        rules: vec![pb::Rule {
            capability: pb::Capability::SessionNote as i32,
            decision: pb::Decision::Never as i32,
            grants: Vec::new(),
        }],
        away: Some(skip),
        revision: None,
        copy: None,
    });
    pb::HostToMachine {
        message: Some(pb::host_to_machine::Message::Policy(copy)),
    }
}

/// What the user reads of channel `ch_away`'s rules.
async fn channel_rules(daemon: &Daemon) -> Result<pb::ChannelRules, tonic::Status> {
    access(&daemon.channel().await)
        .get_channel_rules(pb::GetChannelRulesRequest {
            channel: Some(pb::ChannelId { value: CHANNEL.to_owned() }),
        })
        .await
        .map(tonic::Response::into_inner)
}

async fn default_rules(daemon: &Daemon) -> Result<pb::DefaultRules, tonic::Status> {
    access(&daemon.channel().await)
        .get_default_rules(pb::GetDefaultRulesRequest {})
        .await
        .map(tonic::Response::into_inner)
}

fn is_host_unreachable(status: &tonic::Status) -> bool {
    matches!(common::error_of(status).kind, Some(pb::error::Kind::HostUnreachable(_)))
}

/// D220: the rules the machine judges by, read while the host is away and
/// again after a restart, from the copy the host sent; writes stay refused.
#[tokio::test(flavor = "multi_thread")]
async fn the_user_on_a_machine_reads_the_rules_it_judges_by_while_the_host_is_away() {
    let mut daemon = machine_daemon().await;
    let (child, channel) = link(&daemon).await;
    let (to_machine, mut from_machine) = uplink(&channel).await;
    to_machine.send(snapshot_with_channel(7)).await.unwrap();
    assert_eq!(acked(&mut from_machine).await, 7);
    drop((to_machine, from_machine, child));
    away::host_reachable_becomes(&daemon, false).await;

    for round in ["away", "away after a restart"] {
        let rules = channel_rules(&daemon)
            .await
            .unwrap_or_else(|status| panic!("{round}: the channel's rules were refused: {status:?}"));
        let note = rules
            .rules
            .iter()
            .find(|rule| rule.capability == pb::Capability::SessionNote as i32)
            .map(|rule| rule.decision);
        assert_eq!(note, Some(pb::Decision::Never as i32), "{round}");
        assert_eq!(rules.away.map(|away| away.mode), Some(pb::AwayMode::Skip as i32), "{round}");
        assert_eq!(rules.revision, None, "{round}: a copy offers no revision to write from");
        let stamp = rules.copy.expect("the rules name the copy they come from");
        assert_eq!(stamp.revision, 7, "{round}");
        assert_eq!(stamp.at.map(|at| at.seconds), Some(1_794_657_600), "{round}");

        let defaults = default_rules(&daemon)
            .await
            .unwrap_or_else(|status| panic!("{round}: the defaults were refused: {status:?}"));
        assert_eq!(defaults.away.map(|away| away.mode), Some(pb::AwayMode::Skip as i32), "{round}");
        assert_eq!(defaults.revision, None, "{round}");
        assert_eq!(defaults.copy.map(|copy| copy.revision), Some(7), "{round}");

        let changed = access(&daemon.channel().await)
            .set_default_rules(pb::SetDefaultRulesRequest {
                away: Some(pb::AwayRule {
                    mode: pb::AwayMode::Wait as i32,
                    low_risk: Vec::new(),
                }),
                ..Default::default()
            })
            .await;
        let refused = changed.expect_err("a machine changed its copy of the rules");
        assert!(is_host_unreachable(&refused), "{round}: {refused:?}");
        let set = access(&daemon.channel().await)
            .set_channel_rule(pb::SetChannelRuleRequest {
                channel: Some(pb::ChannelId { value: CHANNEL.to_owned() }),
                capability: pb::Capability::SessionNote as i32,
                decision: pb::Decision::Allow as i32,
                ..Default::default()
            })
            .await;
        assert!(set.is_err(), "{round}: a machine changed a channel rule: {set:?}");
        assert_eq!(away_mode(&daemon).await, pb::AwayMode::Skip as i32, "{round}");

        daemon.restart().await;
    }
}

/// D220: a machine the host never sent a copy judges by the defaults, and
/// says so: the defaults, with no copy named.
#[tokio::test(flavor = "multi_thread")]
async fn a_machine_never_sent_a_copy_reads_the_defaults_with_no_copy_named() {
    let daemon = machine_daemon().await;
    assert!(!server_info(&daemon).await.host_reachable);
    let defaults = default_rules(&daemon).await.expect("the defaults were refused");
    assert_eq!(defaults.away.map(|away| away.mode), Some(pb::AwayMode::Wait as i32));
    assert_eq!(defaults.copy, None);
    assert_eq!(defaults.revision, None);
}
