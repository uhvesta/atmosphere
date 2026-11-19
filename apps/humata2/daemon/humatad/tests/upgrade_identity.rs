//! UP-1, D281 a: a daemon says which release it is, and only when its
//! binary matches the bundle. ServerInfo and the link's Hello carry a
//! `BuildInfo` whose bundle id is the sha256 of `bin/bundle.json` beside
//! the daemon's state, set only when the daemon's own sha256 is its
//! platform's `humatad2` entry there; otherwise it is "".

use humatad_test_common as common;
use humatad_test_common::upgrade;

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Stdio;

use common::{Build, Daemon, folder, state_folder};
use humata2_grpc::humata::v1 as pb;
use tokio::process::{Child, Command};
use tonic::transport::Channel;

/// A daemon started with `bundle_json` installed beside its state first;
/// returns it with the bundle id.
async fn daemon_with(bundle_json: &[u8]) -> (Daemon, String) {
    let dir = folder();
    let id = upgrade::install(&dir.path().join("bin"), bundle_json);
    (Daemon::start_in(dir).await, id)
}

/// What every build reports, whatever its bundle says.
fn assert_identifies_itself(build: &pb::BuildInfo, info: &pb::ServerInfo) {
    assert!(!build.version.is_empty(), "{build:?}");
    assert_eq!(build.version, info.daemon_version);
    assert_eq!(build.own_sha256, upgrade::admin_sha(), "the running binary's own hash");
    let schema = u32::try_from(humata2_store::SCHEMA_VERSION).expect("the schema version fits in u32");
    assert_eq!(build.store_schema, schema);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_daemon_whose_binary_is_its_platforms_entry_names_the_bundle() {
    let bundle = upgrade::manifest(&upgrade::admin_sha(), "a");
    let (daemon, id) = daemon_with(&bundle).await;

    let info = common::server_info(&daemon).await;
    let build = info.build.clone().expect("ServerInfo carries the daemon's build");

    assert_identifies_itself(&build, &info);
    assert_eq!(build.bundle, id);
    let mut files: Vec<(String, String)> = build
        .files
        .iter()
        .map(|file| (file.name.clone(), file.sha256.clone()))
        .collect();
    files.sort();
    let mut expected = vec![
        ("humata".to_owned(), upgrade::sha256_of(&common::binary("HUMATA"))),
        ("humata2_holder".to_owned(), upgrade::sha256_of(&common::binary("HUMATA2_HOLDER"))),
        ("humatad2".to_owned(), upgrade::admin_sha()),
        ("stack".to_owned(), upgrade::sha256_of(&common::binary("STACK"))),
    ];
    expected.sort();
    assert_eq!(files, expected, "the bundle's files for this platform");
}

#[tokio::test(flavor = "multi_thread")]
async fn two_releases_that_differ_have_different_bundle_ids() {
    let (one, _) = daemon_with(&upgrade::manifest(&upgrade::admin_sha(), "a")).await;
    let (two, _) = daemon_with(&upgrade::manifest(&upgrade::admin_sha(), "b")).await;

    let (one, two) = (upgrade::build_of(&one).await, upgrade::build_of(&two).await);

    assert!(!one.bundle.is_empty() && !two.bundle.is_empty(), "{one:?} {two:?}");
    assert_ne!(one.bundle, two.bundle);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_daemon_whose_binary_differs_from_its_entry_names_no_bundle() {
    // The release build's hash in the entry: this daemon is the admin one.
    let bundle = upgrade::manifest(&upgrade::release_sha(), "a");
    let (daemon, _) = daemon_with(&bundle).await;

    let info = common::server_info(&daemon).await;
    let build = info.build.clone().expect("ServerInfo carries the daemon's build");

    assert_eq!(build.bundle, "");
    assert_identifies_itself(&build, &info);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_daemon_with_no_bundle_file_names_no_bundle() {
    let daemon = Daemon::start_in(folder()).await;

    let info = common::server_info(&daemon).await;
    let build = info.build.clone().expect("ServerInfo carries the daemon's build");

    assert_eq!(build.bundle, "");
    assert_identifies_itself(&build, &info);
}

#[tokio::test(flavor = "multi_thread")]
async fn only_its_own_platforms_entry_counts() {
    // This binary's hash, but as another platform's daemon.
    let bundle = upgrade::manifest_on(upgrade::other_platform(), &upgrade::admin_sha(), "a");
    let (daemon, _) = daemon_with(&bundle).await;

    assert_eq!(upgrade::build_of(&daemon).await.bundle, "");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bundle_file_that_does_not_parse_names_no_bundle_and_the_daemon_still_serves() {
    let (daemon, _) = daemon_with(b"{\"platforms\": {\"darwin-arm64\": ").await;

    let info = common::server_info(&daemon).await;
    let build = info.build.clone().expect("ServerInfo carries the daemon's build");

    assert_eq!(build.bundle, "");
    assert_identifies_itself(&build, &info);
    let tabs = daemon.terminals().await.list_tabs(pb::ListTabsRequest::default()).await;
    assert!(tabs.is_ok(), "{tabs:?}");
}

// ---------------------------------------------------------------------------
// The link's Hello: what the host learns a machine is.

const KEY: [u8; 32] = [0x5a; 32];

fn write_key(dir: &Path) {
    let path = state_folder(dir).join("enrollment-key");
    std::fs::write(&path, format!("{}\n", upgrade::hex(&KEY))).expect("the key is written");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("the key is the owner's alone");
}

/// A machine's admin daemon with its key and `bundle_json` in place
/// before it starts.
async fn machine_with(bundle_json: &[u8]) -> Daemon {
    let dir = folder();
    write_key(dir.path());
    upgrade::install(&dir.path().join("bin"), bundle_json);
    Daemon::start_with(Build::Admin, dir, &["--role", "machine"]).await
}

/// `serve --stdio` of the release build over `daemon`'s state and socket,
/// as the host's `ssh` starts it: another binary than the daemon's.
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
        .expect("serve --stdio starts");
    let stdout = child.stdout.take().expect("the link's stdout is piped");
    let stdin = child.stdin.take().expect("the link's stdin is piped");
    let channel = common::channel_over(stdout, stdin).await;
    (child, channel)
}

async fn hello(channel: &Channel) -> pb::HelloResponse {
    pb::peer_service_client::PeerServiceClient::new(channel.clone())
        .hello(pb::HelloRequest {
            host_api_major: 1,
            nonce: vec![7; 32],
            ..Default::default()
        })
        .await
        .expect("the link answers Hello")
        .into_inner()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_hello_reply_carries_the_build_the_machines_daemon_reports() {
    let daemon = machine_with(&upgrade::manifest(&upgrade::admin_sha(), "a")).await;
    let (_child, channel) = link(&daemon).await;

    let answer = hello(&channel).await;
    let reported = upgrade::build_of(&daemon).await;

    assert!(!reported.bundle.is_empty(), "{reported:?}");
    // The daemon the link relays to, not the link's own binary: after an
    // install the link may already be the next build while the daemon that
    // serves is still the old one.
    assert_eq!(answer.build, Some(reported));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_hello_reply_of_a_machine_off_its_bundle_names_no_bundle() {
    let daemon = machine_with(&upgrade::manifest(&upgrade::release_sha(), "a")).await;
    let (_child, channel) = link(&daemon).await;

    let answer = hello(&channel).await;
    let build = answer.build.expect("Hello carries the machine's build");

    assert_eq!(build.bundle, "");
    assert_eq!(build.own_sha256, upgrade::admin_sha(), "the machine's daemon, not the link");
    assert_eq!(build, upgrade::build_of(&daemon).await);
}
