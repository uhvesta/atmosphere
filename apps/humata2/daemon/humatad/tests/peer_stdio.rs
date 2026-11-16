//! `humatad2 serve --stdio` serves the peer link on its stdin and stdout,
//! as the Mac reaches a machine over SSH (GRPC.md §2, §8): PeerService and
//! what the Mac relays there, nothing else, and PeerService on no socket.
//! The link serves nothing but the handshake until the host proves it holds
//! the machine's enrollment key.

mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use common::{Daemon, WAIT, channel_over};
use hmac::{Hmac, KeyInit, Mac};
use humata2_grpc::humata::v1 as pb;
use pb::peer_unproven::Reason;
use sha2::Sha256;
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::{Child, Command};
use tonic::transport::Channel;
use tonic::{Code, Status};

const KEY: [u8; 32] = [0x5a; 32];
const MACHINE_LABEL: &[u8] = b"humata-peer-machine-v1";
const HOST_LABEL: &[u8] = b"humata-peer-host-v1";

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

/// Writes `key` as the enrollment key of the daemon in `dir`, as hex in a
/// file of `mode`.
fn write_key(dir: &Path, key: &[u8], mode: u32) {
    let path = common::state_folder(dir).join("enrollment-key");
    let hex: String = key.iter().map(|byte| format!("{byte:02x}")).collect();
    std::fs::write(&path, format!("{hex}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
}

/// The link of a `serve --stdio` started in `dir`.
async fn link_in(variable: &str, dir: &Path) -> (Child, Channel) {
    let mut child = serve_stdio(variable, dir).spawn().unwrap();
    let channel = channel_over(child.stdout.take().unwrap(), child.stdin.take().unwrap()).await;
    (child, channel)
}

/// The link of a `serve --stdio` started in a fresh folder with no key.
async fn link(variable: &str) -> (TempDir, Child, Channel) {
    let dir = common::folder();
    let (child, channel) = link_in(variable, dir.path()).await;
    (dir, child, channel)
}

/// A link whose host has proved it holds the machine's key.
async fn proven_link(variable: &str) -> (TempDir, Child, Channel) {
    let dir = common::folder();
    write_key(dir.path(), &KEY, 0o600);
    let (child, channel) = link_in(variable, dir.path()).await;
    let answer = peer(&channel).hello(hello()).await.unwrap().into_inner();
    peer(&channel)
        .prove_host(pb::ProveHostRequest {
            proof: host_proof(&KEY, &answer),
        })
        .await
        .unwrap();
    (dir, child, channel)
}

fn peer(channel: &Channel) -> pb::peer_service_client::PeerServiceClient<Channel> {
    pb::peer_service_client::PeerServiceClient::new(channel.clone())
}

fn daemon(channel: &Channel) -> pb::daemon_service_client::DaemonServiceClient<Channel> {
    pb::daemon_service_client::DaemonServiceClient::new(channel.clone())
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
        chunk_codecs: Vec::new(),
    }
}

/// HMAC-SHA256 under `key` of `label`, then `first`, then `second`.
fn sign(key: &[u8], label: &[u8], first: &[u8], second: &[u8]) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).unwrap();
    mac.update(label);
    mac.update(first);
    mac.update(second);
    mac.finalize().into_bytes().to_vec()
}

/// The host's proof under `key` for the link Hello `answer` came from.
fn host_proof(key: &[u8], answer: &pb::HelloResponse) -> Vec<u8> {
    sign(key, HOST_LABEL, &answer.machine_nonce, &hello().nonce)
}

/// Why a call was refused with `peer_unproven`.
fn peer_unproven(status: &Status) -> Reason {
    assert_eq!(status.code(), Code::Unauthenticated, "{status:?}");
    match common::error_of(status).kind {
        Some(pb::error::Kind::PeerUnproven(refused)) => refused.reason(),
        other => panic!("expected peer_unproven, got {other:?}"),
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
    write_key(dir.path(), &KEY, 0o600);
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
    let answer = peer(&channel).hello(hello()).await.unwrap().into_inner();
    peer(&channel)
        .prove_host(pb::ProveHostRequest {
            proof: host_proof(&KEY, &answer),
        })
        .await
        .unwrap();
    let info = daemon(&channel)
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
async fn the_machine_proves_it_holds_the_key() {
    let dir = common::folder();
    write_key(dir.path(), &KEY, 0o600);
    let (_child, channel) = link_in("HUMATAD2_RELEASE", dir.path()).await;
    let answer = peer(&channel).hello(hello()).await.unwrap().into_inner();
    assert_eq!(answer.machine_nonce.len(), 32);
    assert_eq!(
        answer.proof,
        sign(&KEY, MACHINE_LABEL, &hello().nonce, &answer.machine_nonce)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn hello_lists_the_codecs_the_machine_decodes() {
    let dir = common::folder();
    write_key(dir.path(), &KEY, 0o600);
    let (_child, channel) = link_in("HUMATAD2_RELEASE", dir.path()).await;
    let answer = peer(&channel).hello(hello()).await.unwrap().into_inner();
    let mut codecs = answer.chunk_codecs.clone();
    codecs.sort_unstable();
    let all = [pb::ChunkCodec::None, pb::ChunkCodec::Lz4, pb::ChunkCodec::Zstd].map(|codec| codec as i32);
    assert_eq!(codecs, all);
}

#[tokio::test(flavor = "multi_thread")]
async fn with_compression_off_hello_lists_only_raw_chunks() {
    let dir = common::folder();
    write_key(dir.path(), &KEY, 0o600);
    std::fs::write(
        common::state_folder(dir.path()).join("compression.toml"),
        "compression = \"off\"\n",
    )
    .unwrap();
    let (_child, channel) = link_in("HUMATAD2_RELEASE", dir.path()).await;
    let answer = peer(&channel).hello(hello()).await.unwrap().into_inner();
    assert_eq!(answer.chunk_codecs, vec![pb::ChunkCodec::None as i32]);
}

#[tokio::test(flavor = "multi_thread")]
async fn each_link_gets_a_fresh_machine_nonce() {
    let dir = common::folder();
    write_key(dir.path(), &KEY, 0o600);
    let (_first, one) = link_in("HUMATAD2_RELEASE", dir.path()).await;
    let (_second, two) = link_in("HUMATAD2_RELEASE", dir.path()).await;
    let one = peer(&one).hello(hello()).await.unwrap().into_inner();
    let two = peer(&two).hello(hello()).await.unwrap().into_inner();
    assert_ne!(one.machine_nonce, two.machine_nonce);
    assert_ne!(one.proof, two.proof);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_host_proof_opens_the_link() {
    let (_dir, _child, channel) = proven_link("HUMATAD2_RELEASE").await;
    let not_built = peer(&channel)
        .list_holders(pb::ListHoldersRequest {})
        .await
        .unwrap_err();
    assert!(routed(&not_built));
}

#[tokio::test(flavor = "multi_thread")]
async fn nothing_else_is_served_before_the_host_proves_itself() {
    let dir = common::folder();
    write_key(dir.path(), &KEY, 0o600);
    let (_child, channel) = link_in("HUMATAD2_RELEASE", dir.path()).await;

    let before_hello = daemon(&channel)
        .get_server_info(pb::GetServerInfoRequest {})
        .await
        .unwrap_err();
    assert_eq!(peer_unproven(&before_hello), Reason::HostNotProven);

    peer(&channel).hello(hello()).await.unwrap();
    let before_proof = peer(&channel)
        .list_holders(pb::ListHoldersRequest {})
        .await
        .unwrap_err();
    assert_eq!(peer_unproven(&before_proof), Reason::HostNotProven);
    let relayed = pb::terminal_service_client::TerminalServiceClient::new(channel.clone())
        .list_tabs(pb::ListTabsRequest { session: None })
        .await
        .unwrap_err();
    assert_eq!(peer_unproven(&relayed), Reason::HostNotProven);

    // Asking who it is stays open, and the link is still nobody.
    let me = daemon(&channel)
        .who_am_i(pb::WhoAmIRequest {})
        .await
        .unwrap()
        .into_inner();
    assert!(me.actor.is_none());
}

/// Sends `proof` as the host's on a fresh link and checks that the machine
/// refuses it, closes the link with a failure, and keeps its error off the
/// link's stderr and in its log.
async fn a_wrong_host_proof_closes_the_link(proof: impl FnOnce(&pb::HelloResponse) -> Vec<u8>) {
    let dir = common::folder();
    write_key(dir.path(), &KEY, 0o600);
    let mut child = serve_stdio("HUMATAD2_RELEASE", dir.path())
        .args(["--sync", "7f3a"])
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    tokio::time::timeout(WAIT, stdout.read_line(&mut line))
        .await
        .expect("no sync marker")
        .unwrap();
    let channel = channel_over(stdout, child.stdin.take().unwrap()).await;

    let answer = peer(&channel).hello(hello()).await.unwrap().into_inner();
    let refused = peer(&channel)
        .prove_host(pb::ProveHostRequest {
            proof: proof(&answer),
        })
        .await
        .unwrap_err();
    assert_eq!(peer_unproven(&refused), Reason::HostProofWrong);

    assert!(!exits(&mut child).await.success());
    let mut written = Vec::new();
    stderr.read_to_end(&mut written).await.unwrap();
    assert!(written.is_empty(), "{}", String::from_utf8_lossy(&written));
    let log = dir.path().join("state/logs/peer-link.log");
    assert!(!std::fs::read(log).unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_machines_own_proof_reflected_is_refused() {
    a_wrong_host_proof_closes_the_link(|answer| answer.proof.clone()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_proof_under_another_key_is_refused() {
    a_wrong_host_proof_closes_the_link(|answer| host_proof(&[0x33; 32], answer)).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_empty_proof_is_refused() {
    a_wrong_host_proof_closes_the_link(|_| Vec::new()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_machine_with_no_key_refuses_hello() {
    let (_dir, _child, channel) = link("HUMATAD2_RELEASE").await;
    let refused = peer(&channel).hello(hello()).await.unwrap_err();
    assert_eq!(peer_unproven(&refused), Reason::NotEnrolled);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_key_others_can_read_is_refused() {
    let dir = common::folder();
    write_key(dir.path(), &KEY, 0o644);
    let (_child, channel) = link_in("HUMATAD2_RELEASE", dir.path()).await;
    let refused = peer(&channel).hello(hello()).await.unwrap_err();
    assert_eq!(peer_unproven(&refused), Reason::KeyUnsafe);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_short_host_nonce_is_refused() {
    let dir = common::folder();
    write_key(dir.path(), &KEY, 0o600);
    let (_child, channel) = link_in("HUMATAD2_RELEASE", dir.path()).await;
    let refused = peer(&channel)
        .hello(pb::HelloRequest {
            nonce: vec![7; 8],
            ..hello()
        })
        .await
        .unwrap_err();
    let (field, problem) = common::invalid_field(&refused);
    assert_eq!(field, "nonce");
    assert_eq!(problem, pb::invalid_field::Problem::OutOfRange);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_link_has_one_hello() {
    let dir = common::folder();
    write_key(dir.path(), &KEY, 0o600);
    let (_child, channel) = link_in("HUMATAD2_RELEASE", dir.path()).await;
    peer(&channel).hello(hello()).await.unwrap();
    let again = peer(&channel).hello(hello()).await.unwrap_err();
    assert_eq!(peer_unproven(&again), Reason::OutOfOrder);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_proof_before_hello_is_refused_and_the_link_stays_open() {
    let dir = common::folder();
    write_key(dir.path(), &KEY, 0o600);
    let (_child, channel) = link_in("HUMATAD2_RELEASE", dir.path()).await;
    let early = peer(&channel)
        .prove_host(pb::ProveHostRequest { proof: vec![0; 32] })
        .await
        .unwrap_err();
    assert_eq!(peer_unproven(&early), Reason::OutOfOrder);

    let answer = peer(&channel).hello(hello()).await.unwrap().into_inner();
    peer(&channel)
        .prove_host(pb::ProveHostRequest {
            proof: host_proof(&KEY, &answer),
        })
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn after_the_marker_stderr_goes_to_the_log() {
    let dir = common::folder();
    write_key(dir.path(), &KEY, 0o600);
    let mut child = serve_stdio("HUMATAD2_RELEASE", dir.path())
        .args(["--sync", "7f3a"])
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    tokio::time::timeout(WAIT, stdout.read_line(&mut line))
        .await
        .expect("no sync marker")
        .unwrap();
    let channel = channel_over(stdout, child.stdin.take().unwrap()).await;
    daemon(&channel).who_am_i(pb::WhoAmIRequest {}).await.unwrap();
    drop(channel);
    assert!(exits(&mut child).await.success());

    let mut written = Vec::new();
    stderr.read_to_end(&mut written).await.unwrap();
    assert!(written.is_empty(), "{}", String::from_utf8_lossy(&written));
    let log = std::fs::metadata(dir.path().join("state/logs/peer-link.log")).unwrap();
    assert_eq!(log.permissions().mode() & 0o777, 0o600);
}

#[tokio::test(flavor = "multi_thread")]
async fn peer_service_is_on_the_link() {
    let (_dir, _child, channel) = proven_link("HUMATAD2_RELEASE").await;

    // Nothing of PeerService past the handshake is built yet, but it is there.
    let not_built = peer(&channel)
        .list_holders(pb::ListHoldersRequest {})
        .await
        .unwrap_err();
    assert!(routed(&not_built));

    // So is a service the Mac relays to the machine. This link was given a
    // state folder and no socket, so it relays nothing (D146): the call
    // answers unavailable, with the API's error in the details.
    let relayed = pb::terminal_service_client::TerminalServiceClient::new(channel)
        .list_tabs(pb::ListTabsRequest { session: None })
        .await
        .unwrap_err();
    assert_eq!(relayed.code(), Code::Unavailable, "{relayed:?}");
    assert!(!relayed.details().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn client_calls_the_host_may_not_relay_are_refused_by_name() {
    let (_dir, _child, channel) = proven_link("HUMATAD2_RELEASE").await;

    let inbox = pb::inbox_service_client::InboxServiceClient::new(channel.clone())
        .list_requests(pb::ListRequestsRequest::default())
        .await
        .unwrap_err();
    assert_eq!(inbox.code(), Code::PermissionDenied, "{inbox:?}");

    let channels = pb::channel_service_client::ChannelServiceClient::new(channel)
        .list_channels(pb::ListChannelsRequest::default())
        .await
        .unwrap_err();
    assert_eq!(channels.code(), Code::PermissionDenied, "{channels:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_link_is_never_the_user() {
    // The admin build takes a call with no credential for the user on its
    // socket; on the peer link it doesn't.
    let (_dir, _child, channel) = link("HUMATAD2").await;
    let me = daemon(&channel)
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
    daemon(&channel).who_am_i(pb::WhoAmIRequest {}).await.unwrap();
    drop(channel);
    assert!(exits(&mut child).await.success());
}

/// Writes an executable shell script `name` in `dir`.
fn script(dir: &Path, name: &str, body: &str) {
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_proven_link_answers_a_probe_from_the_machine_it_runs_on() {
    let dir = common::folder();
    write_key(dir.path(), &KEY, 0o600);
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    script(&bin, "git", "echo 'git version 9.8.7'");
    script(
        &bin,
        "gh",
        "if [ \"$1\" = --version ]; then echo 'gh version 2.83.0 (2025-11-04)'; exit 0; fi; printf '%s\\n' 'github.com' '  - Logged in to github.com account fakeuser (keyring)' '  - Active account: true' \"  - Token scopes: 'repo', 'read:org'\"",
    );
    script(&bin, "claude", "echo '2.4.1 (Claude Code)'");
    // An agent that prints no version still counts as installed.
    script(&bin, "codex", "exit 0");
    let mut child = serve_stdio("HUMATAD2_RELEASE", dir.path())
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("HOME", dir.path())
        .spawn()
        .unwrap();
    let channel = channel_over(child.stdout.take().unwrap(), child.stdin.take().unwrap()).await;
    let answer = peer(&channel).hello(hello()).await.unwrap().into_inner();
    peer(&channel)
        .prove_host(pb::ProveHostRequest {
            proof: host_proof(&KEY, &answer),
        })
        .await
        .unwrap();

    let probed = peer(&channel)
        .probe(pb::ProbeRequest { measure_storage: false })
        .await
        .unwrap()
        .into_inner();
    assert!(!probed.os.is_empty());
    assert!(!probed.arch.is_empty());
    assert!(probed.disk_total_bytes > 0);
    assert!(probed.disk_free_bytes <= probed.disk_total_bytes);
    assert_eq!(probed.git_version, "9.8.7");
    assert_eq!(probed.gh_user, "fakeuser");
    assert_eq!(probed.gh_scopes, ["repo", "read:org"]);
    assert!(probed.agents_installed.iter().any(|agent| agent.value == "claude"));
    assert_eq!(probed.gh_version, "2.83.0");
    assert_eq!(probed.agent_versions.len(), probed.agents_installed.len());
    let versions: std::collections::BTreeMap<_, _> = probed
        .agents_installed
        .iter()
        .map(|agent| agent.value.as_str())
        .zip(probed.agent_versions.iter().map(String::as_str))
        .collect();
    assert_eq!(versions.get("claude"), Some(&"2.4.1"));
    assert_eq!(versions.get("codex"), Some(&""));
    assert!(probed.folder_bytes.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_probe_before_the_host_proves_itself_is_refused() {
    let (_dir, _child, channel) = link("HUMATAD2_RELEASE").await;
    let refused = peer(&channel)
        .probe(pb::ProbeRequest { measure_storage: false })
        .await
        .unwrap_err();
    assert_eq!(peer_unproven(&refused), Reason::HostNotProven);
}
