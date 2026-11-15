//! `humatad2 bridge` carries the API over its stdin and stdout, the way SSH
//! would, starting the daemon when none answers.

mod common;

use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use common::{Daemon, WAIT, channel_over, error_of};
use humata2_grpc::humata::v1 as pb;
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tonic::Code;
use tonic::transport::Channel;

fn binary(variable: &str) -> PathBuf {
    std::fs::canonicalize(PathBuf::from(std::env::var_os(variable).unwrap())).unwrap()
}

fn bridge(program: &Path) -> Command {
    let mut command = Command::new(program);
    command
        .arg("bridge")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .kill_on_drop(true);
    command
}

async fn exits(child: &mut Child) -> ExitStatus {
    tokio::time::timeout(WAIT, child.wait())
        .await
        .expect("the bridge kept running")
        .unwrap()
}

async fn who_am_i(channel: Channel) -> pb::WhoAmIResponse {
    pb::daemon_service_client::DaemonServiceClient::new(channel)
        .who_am_i(pb::WhoAmIRequest {})
        .await
        .unwrap()
        .into_inner()
}

/// A machine's install: `$HOME/.humata2/bin` with humatad2 and, if
/// `with_holder`, its holder beside it, in a fresh home folder.
fn machine_home(daemon: &str, with_holder: bool) -> (TempDir, PathBuf) {
    let home = common::folder();
    let bin = home.path().join(".humata2/bin");
    // As the install makes it: only this user can open it.
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&bin)
        .unwrap();
    std::fs::copy(binary(daemon), bin.join("humatad2")).unwrap();
    if with_holder {
        std::fs::copy(binary("HUMATA2_HOLDER"), bin.join("humata2_holder")).unwrap();
    }
    (home, bin.join("humatad2"))
}

/// Stops the daemon a bridge started in `home`, which runs detached.
struct StopDetached(PathBuf);

impl Drop for StopDetached {
    fn drop(&mut self) {
        let pattern = format!("{} serve", self.0.display());
        let _ = std::process::Command::new("pkill")
            .arg("-f")
            .arg(pattern)
            .status();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn grpc_works_through_the_bridge() {
    let daemon = Daemon::start().await;
    let mut bridge = bridge(&binary("HUMATAD2"))
        .arg("--socket")
        .arg(&daemon.socket)
        .spawn()
        .unwrap();
    let channel = channel_over(bridge.stdout.take().unwrap(), bridge.stdin.take().unwrap()).await;

    // The admin daemon takes a call with no credential as the user.
    let me = who_am_i(channel.clone()).await;
    assert!(me.actor.is_some());

    let info = pb::daemon_service_client::DaemonServiceClient::new(channel.clone())
        .get_server_info(pb::GetServerInfoRequest {})
        .await
        .unwrap()
        .into_inner();
    assert_eq!(info.api_major, 1);

    // A failure's details make it through as well.
    let refused = pb::activity_service_client::ActivityServiceClient::new(channel)
        .list_activity(pb::ListActivityRequest::default())
        .await
        .unwrap_err();
    assert!(!error_of(&refused).message.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_sync_marker_comes_before_the_stream() {
    let daemon = Daemon::start().await;
    let mut bridge = bridge(&binary("HUMATAD2"))
        .arg("--socket")
        .arg(&daemon.socket)
        .arg("--sync")
        .arg("7f3a")
        .spawn()
        .unwrap();
    let mut stdout = BufReader::new(bridge.stdout.take().unwrap());
    let mut line = String::new();
    tokio::time::timeout(WAIT, stdout.read_line(&mut line))
        .await
        .expect("no sync marker")
        .unwrap();
    assert_eq!(line, "HUMATA_SYNC_7f3a\n");

    // Everything after the marker is the stream.
    let channel = channel_over(stdout, bridge.stdin.take().unwrap()).await;
    assert!(who_am_i(channel).await.actor.is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bridge_with_no_daemon_starts_one_that_outlives_it() {
    let (home, program) = machine_home("HUMATAD2_RELEASE", true);
    let _stop = StopDetached(program.clone());
    let mut bridge = bridge(&program)
        .env("HOME", home.path())
        .env("SHELL", "/bin/sh")
        .spawn()
        .unwrap();
    let channel = channel_over(bridge.stdout.take().unwrap(), bridge.stdin.take().unwrap()).await;

    // The started daemon is a release one: a call with no credential gets
    // only WhoAmI, which shows no one; the bridge lends it no identity.
    let me = who_am_i(channel.clone()).await;
    assert!(me.actor.is_none());
    let refused = pb::daemon_service_client::DaemonServiceClient::new(channel.clone())
        .get_server_info(pb::GetServerInfoRequest {})
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::Unauthenticated);

    // Closing the far end ends the bridge; the daemon keeps running on
    // the default socket.
    drop(channel);
    assert!(exits(&mut bridge).await.success());
    let socket = home.path().join(".humata2/humatad.sock");
    assert!(
        who_am_i(common::connect(&socket).await)
            .await
            .actor
            .is_none()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_bridge_ends_when_the_far_side_closes() {
    let daemon = Daemon::start().await;
    let mut bridge = bridge(&binary("HUMATAD2"))
        .arg("--socket")
        .arg(&daemon.socket)
        .spawn()
        .unwrap();
    let stdin = bridge.stdin.take().unwrap();
    // Give it time to connect before hanging up.
    tokio::time::sleep(Duration::from_millis(200)).await;
    drop(stdin);
    assert!(exits(&mut bridge).await.success());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_bridge_ends_when_the_daemon_stops() {
    let mut daemon = Daemon::start().await;
    let mut bridge = bridge(&binary("HUMATAD2"))
        .arg("--socket")
        .arg(&daemon.socket)
        .spawn()
        .unwrap();
    let channel = channel_over(bridge.stdout.take().unwrap(), bridge.stdin.take().unwrap()).await;
    who_am_i(channel.clone()).await;

    daemon.kill();
    exits(&mut bridge).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bridge_whose_daemon_cannot_start_fails() {
    // No holder beside humatad2, so the daemon it starts exits at once.
    let (home, program) = machine_home("HUMATAD2_RELEASE", false);
    let _stop = StopDetached(program.clone());

    let status = Command::new(&program)
        .arg("bridge")
        .env("HOME", home.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .unwrap();

    assert!(!status.success());
}
