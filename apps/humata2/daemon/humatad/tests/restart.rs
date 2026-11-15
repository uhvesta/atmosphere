//! What `humatad2` brings back when it starts again (PER-25–PER-31): a
//! shell whose holder died with it comes back, and the start says so in
//! GetServerInfo's `last_start`.

mod common;

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use common::{Attach, Daemon, open};
use humata2_grpc::humata::v1 as pb;
use humata2_store::Store;
use tokio::time::Instant;

async fn machine_shell(daemon: &Daemon) -> pb::TabId {
    daemon
        .terminals()
        .await
        .open_machine_shell(pb::OpenMachineShellRequest::default())
        .await
        .unwrap()
        .into_inner()
        .id
        .unwrap()
}

async fn server_info(daemon: &Daemon) -> pb::ServerInfo {
    pb::daemon_service_client::DaemonServiceClient::new(daemon.channel().await)
        .get_server_info(pb::GetServerInfoRequest {})
        .await
        .unwrap()
        .into_inner()
}

/// The start's account, once the daemon has brought everything back.
async fn last_start(daemon: &Daemon) -> pb::LastStart {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(start) = server_info(daemon).await.last_start {
            return start;
        }
        assert!(Instant::now() < deadline, "the daemon never finished bringing sessions back");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn running(pid: u32) -> bool {
    Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .status()
        .unwrap()
        .success()
}

/// Kills every holder the store knows of, as a reboot would.
async fn kill_holders(state: &Path) {
    let store = Store::open(state.join("store")).unwrap();
    for tab in store.tabs().unwrap() {
        Command::new("kill").arg("-9").arg(tab.holder_pid.to_string()).status().unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while running(tab.holder_pid) {
            assert!(Instant::now() < deadline, "holder {} never died", tab.holder_pid);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_shell_whose_holder_died_comes_back() {
    let mut daemon = Daemon::start().await;
    let tab = machine_shell(&daemon).await;

    daemon.kill();
    kill_holders(&daemon.state).await;
    daemon.restart().await;

    let start = last_start(&daemon).await;
    assert!(start.not_brought_back.is_empty(), "{:?}", start.not_brought_back);
    assert!(Path::new(&start.report).is_dir());
    let tabs = daemon
        .terminals()
        .await
        .list_tabs(pb::ListTabsRequest::default())
        .await
        .unwrap()
        .into_inner()
        .tabs;
    assert_eq!(tabs.len(), 1);
    let mut after = Attach::start(&daemon, open(&tab, 0)).await;
    after.type_text("echo bac''k\n").await;
    after.wait_for("back").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_first_start_has_nothing_left_behind() {
    let daemon = Daemon::start().await;

    let start = last_start(&daemon).await;

    assert!(start.not_brought_back.is_empty());
    assert_eq!(start.restored_copy_taken_unix_ms, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_shell_that_cant_come_back_is_one_line_and_the_others_still_do() {
    let mut daemon = Daemon::start().await;
    let lost = machine_shell(&daemon).await;
    let kept = machine_shell(&daemon).await;

    daemon.kill();
    kill_holders(&daemon.state).await;
    // What the lost tab's holder saved can't be read back.
    let store = Store::open(daemon.state.join("store")).unwrap();
    let socket = store.tab(&lost.value).unwrap().unwrap().holder_socket;
    drop(store);
    let saved = socket.with_extension("scrollback");
    let _ = std::fs::remove_file(&saved);
    std::fs::create_dir(&saved).unwrap();
    daemon.restart().await;

    let start = last_start(&daemon).await;
    assert_eq!(start.not_brought_back.len(), 1, "{:?}", start.not_brought_back);
    let line = &start.not_brought_back[0];
    assert_eq!(line.tab, lost.value);
    assert!(line.session.is_empty());
    assert!(!line.problem.is_empty());
    let mut after = Attach::start(&daemon, open(&kept, 0)).await;
    after.type_text("echo bac''k\n").await;
    after.wait_for("back").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_damaged_state_comes_back_from_its_last_good_copy_and_the_start_says_so() {
    let mut daemon = Daemon::start().await;
    let tab = machine_shell(&daemon).await;

    daemon.kill();
    kill_holders(&daemon.state).await;
    let folder = daemon.state.join("store");
    let copy = Store::open(&folder).unwrap().save_last_good().unwrap();
    std::fs::write(folder.join("state.sqlite"), vec![0x5a; 8192]).unwrap();
    daemon.restart().await;

    let start = last_start(&daemon).await;
    assert_eq!(start.restored_copy_taken_unix_ms, copy.taken_unix_ms);
    assert!(start.not_brought_back.is_empty(), "{:?}", start.not_brought_back);
    let mut after = Attach::start(&daemon, open(&tab, 0)).await;
    after.type_text("echo bac''k\n").await;
    after.wait_for("back").await;
}
