//! UP-1, D281 g: a daemon killed at any point of an upgrade comes back
//! whole on one release, never on neither. The daemon here is the faults
//! build, killed at a named point (`HUMATA2_STOP_AT`, which an upgrade's
//! restart in the same process keeps, armed with `HUMATA2_FAULT_ARM` once
//! release `a` answers), and started again.
//!
//! Releases `a` and `b` carry the same daemon bytes and differ in their
//! `bundle.json`, so the release a daemon runs is told by the bundle on
//! disk. A cut before the new build was ready ends with release `a` back
//! and the record saying the upgrade did not take; a retry then upgrades.
//!
//! What is not covered here: putting a store back that the new build had
//! migrated, which needs a database at an older schema; that is covered
//! with the store's own copy and put-back (`store:last_good_test`) and the
//! unit tests of `upgrade/rollback.rs`.

use humatad_test_common as common;
use humatad_test_common::upgrade;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::{Build, Daemon, binary, folder};
use humata2_grpc::humata::v1 as pb;
use hyper_util::rt::TokioIo;
use tokio::net::UnixStream;
use tonic::transport::{Channel, Endpoint, Uri};
use tonic::Request;

/// A safety net for a daemon that starts itself again, never a pace.
const LONG: Duration = Duration::from_secs(90);

/// A point no run is stopped at: it only makes a start the faults build.
const NEVER: &str = "upgrade.never.reached";

const FILES: [(&str, &str); 4] = [
    ("humatad2", "HUMATAD2_FAULTS"),
    ("humata2_holder", "HUMATA2_HOLDER"),
    ("humata", "HUMATA"),
    ("stack", "STACK"),
];

fn put_file(variable: &str, to: &Path) {
    std::fs::copy(binary(variable), to).unwrap_or_else(|error| panic!("couldn't copy {variable} to {}: {error}", to.display()));
    std::fs::set_permissions(to, std::fs::Permissions::from_mode(0o755)).expect("the file is made runnable");
}

fn daemon_sha() -> String {
    upgrade::sha256_of(&binary("HUMATAD2_FAULTS"))
}

fn manifest(release: &str) -> Vec<u8> {
    upgrade::manifest(&daemon_sha(), release)
}

/// Release `a` installed in `bin`; returns its bundle id.
fn install_a(bin: &Path) -> String {
    std::fs::create_dir_all(bin).expect("the bin folder is made");
    for (name, variable) in FILES {
        put_file(variable, &bin.join(name));
    }
    upgrade::install(bin, &manifest("a"))
}

/// Release `b` staged in `bin`, carrying `daemon_var` as `humatad2`; returns
/// its bundle id.
fn stage_b_with(bin: &Path, daemon_var: &str) -> String {
    for (name, variable) in FILES {
        let source = if name == "humatad2" { daemon_var } else { variable };
        put_file(source, &bin.join(format!(".{name}.next")));
    }
    let bundle = upgrade::manifest(&upgrade::sha256_of(&binary(daemon_var)), "b");
    let staged = bin.join(".bundle.json.next");
    std::fs::write(&staged, &bundle).expect("the staged bundle is written");
    std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o644)).expect("readable");
    upgrade::bundle_id(&bundle)
}

/// Release `b` staged in `bin`; returns its bundle id.
fn stage_b(bin: &Path) -> String {
    stage_b_with(bin, "HUMATAD2_FAULTS")
}

async fn try_info(daemon: &Daemon) -> Option<pb::ServerInfo> {
    let socket = daemon.socket.clone();
    let channel: Channel = Endpoint::try_from("http://[::]:50051")
        .ok()?
        .connect_with_connector(tower::service_fn(move |_: Uri| {
            let socket = socket.clone();
            async move { Ok::<_, std::io::Error>(TokioIo::new(UnixStream::connect(socket).await?)) }
        }))
        .await
        .ok()?;
    pb::daemon_service_client::DaemonServiceClient::new(channel)
        .get_server_info(pb::GetServerInfoRequest {})
        .await
        .ok()
        .map(tonic::Response::into_inner)
}

async fn wait_at_release(daemon: &Daemon, bundle: &str) {
    let deadline = Instant::now() + LONG;
    loop {
        if try_info(daemon).await.and_then(|info| info.build).is_some_and(|build| build.bundle == bundle) {
            return;
        }
        assert!(Instant::now() < deadline, "the daemon never answered as release {bundle}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

async fn upgrade_to(daemon: &Daemon, to_bundle: &str, id: &str) -> Result<(), tonic::Status> {
    pb::daemon_service_client::DaemonServiceClient::new(daemon.channel().await)
        .upgrade(Request::new(pb::UpgradeRequest {
            to_bundle: to_bundle.to_owned(),
            upgrade_id: id.to_owned(),
            thing: pb::MigrateThing::Unspecified.into(),
        }))
        .await
        .map(|_| ())
}

fn last_record(state: &Path) -> serde_json::Value {
    let path = state.join("upgrade/last.json");
    serde_json::from_slice(&std::fs::read(&path).unwrap_or_else(|error| panic!("{} is missing: {error}", path.display())))
        .expect("last.json is JSON")
}

/// Release `a` is installed whole: its bundle, and every file it names.
fn assert_release_a_whole(bin: &Path) {
    assert_eq!(std::fs::read(bin.join("bundle.json")).expect("bundle.json is installed"), manifest("a"));
    for (name, variable) in FILES {
        assert_eq!(
            upgrade::sha256_of(&bin.join(name)),
            upgrade::sha256_of(&binary(variable)),
            "{name} is the installed one"
        );
    }
}

/// Upgrades a machine from `a` to `b` with its daemon killed at `point`,
/// starts it again, and returns what is left once it is up.
async fn cut_at(point: &str) -> (Daemon, PathBuf, String, String) {
    let dir = folder();
    let bin = dir.path().join("bin");
    let from = install_a(&bin);
    // Some points, such as the store being opened, come in every start:
    // armed only once release a answers, they count in the upgrade's starts
    // alone. An upgrade's exec keeps the environment.
    let arm = dir.path().join("armed");
    let arm_path = arm.to_str().expect("the test folder's path is UTF-8").to_owned();
    let mut daemon = Daemon::start_with_env(
        Build::Admin,
        dir,
        &[],
        &[("HUMATA2_STOP_AT", point), ("HUMATA2_FAULT_ARM", arm_path.as_str())],
    )
    .await;
    wait_at_release(&daemon, &from).await;
    std::fs::write(&arm, b"").expect("the cut point is armed");
    let to = stage_b(&bin);
    // The call may be cut off with the daemon that was answering it.
    let _ = upgrade_to(&daemon, &to, "up_cut").await;
    daemon.stopped(LONG).await;

    daemon.restart_env(&[("HUMATA2_STOP_AT", NEVER)]).await;
    wait_at_release(&daemon, &from).await;
    (daemon, bin, from, to)
}

async fn assert_whole_on_release_a_and_retryable(point: &str, ended: &str) {
    let (daemon, bin, from, to) = cut_at(point).await;

    assert_release_a_whole(&bin);
    assert!(!daemon.state.join("upgrade/this.json").exists(), "nothing is left under way");
    let last = last_record(&daemon.state);
    assert_eq!(last["ended"]["result"], ended, "{last}");
    assert_eq!(last["from_bundle"], from.as_str());

    // A click retries: the same release is staged again and the upgrade
    // goes through.
    let to_again = stage_b(&bin);
    assert_eq!(to_again, to);
    upgrade_to(&daemon, &to, "up_retry").await.expect("the retry is taken");
    wait_at_release(&daemon, &to).await;
    let deadline = Instant::now() + LONG;
    while daemon.state.join("upgrade/this.json").exists() {
        assert!(Instant::now() < deadline, "the retry never settled");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(last_record(&daemon.state)["upgrade_id"], "up_retry");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cut_between_the_daemon_and_the_bundle_being_swapped_leaves_release_a_whole() {
    assert_whole_on_release_a_and_retryable("upgrade.binary.swapped", "abandoned").await;
}

/// D439: the start that rolls back relaunches release a by exec before it
/// has said it listens; release a says it, on the pipe the starter passed
/// in, so a starter waiting on that word (D398) hears it instead of the
/// pipe closing.
///
/// Release `a` carries `HUMATAD2_FAULTS` and release `b` carries `HUMATAD2`
/// so `installed != own` on rollback and `Start::Restart` -> `exec_at`
/// actually relaunches release `a` while handing on `--ready-fd`.
#[tokio::test(flavor = "multi_thread")]
async fn a_rollback_relaunch_says_ready_on_the_starters_pipe() {
    assert_ne!(daemon_sha(), upgrade::admin_sha(), "release a and release b must carry distinct daemon binaries");
    let dir = folder();
    let bin = dir.path().join("bin");
    let from = install_a(&bin);
    let arm = dir.path().join("armed");
    let arm_path = arm.to_str().expect("the test folder's path is UTF-8").to_owned();
    let mut daemon = Daemon::start_with_env(
        Build::Admin,
        dir,
        &[],
        &[("HUMATA2_STOP_AT", "upgrade.schema.migrated"), ("HUMATA2_FAULT_ARM", arm_path.as_str())],
    )
    .await;
    wait_at_release(&daemon, &from).await;
    std::fs::write(&arm, b"").expect("the cut point is armed");
    let to = stage_b_with(&bin, "HUMATAD2");
    // The call may be cut off with the daemon that was answering it.
    let _ = upgrade_to(&daemon, &to, "up_ready").await;
    daemon.stopped(LONG).await;

    // Empty env starts `HUMATAD2` (release `b`'s binary), which rolls back to
    // release `a` (`HUMATAD2_FAULTS`) and execs it with `--ready-fd` kept open.
    let heard = daemon.restart_hearing_ready(&[]).await;

    assert!(heard.starts_with(b"READY=1"), "the starter hears READY=1: {heard:?}");
    wait_at_release(&daemon, &from).await;
    assert_eq!(
        upgrade::build_of(&daemon).await.own_sha256,
        daemon_sha(),
        "the rollback relaunched release a's binary via exec"
    );
    assert_eq!(upgrade::sha256_of(&bin.join("humatad2")), daemon_sha(), "release a's daemon is back on disk");
    assert_eq!(last_record(&daemon.state)["ended"]["result"], "rolled_back");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_build_cut_after_opening_the_store_at_the_new_schema_is_rolled_back_to_release_a() {
    assert_whole_on_release_a_and_retryable("upgrade.schema.migrated", "rolled_back").await;
}
