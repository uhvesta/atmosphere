//! This machine without naming it (B3, D672 (Provisional)): `host ls` lists
//! the machine the daemon runs on beside the ones it was given, and
//! `session start` goes to it when no `--machine` is given.

mod support;

use humata2_cli::{exit, rpc};
use humata2_proto::humata::v1 as pb;
use support::{FakeConsole, FakeDaemon, run};

fn here(daemon: &FakeDaemon) {
    daemon.reply(
        rpc::daemon::GET_SERVER_INFO,
        &pb::ServerInfo {
            machine: support::machine("ma_here"),
            ..Default::default()
        },
    );
}

#[tokio::test]
async fn host_ls_lists_this_machine_when_the_daemon_lists_none() {
    let daemon = FakeDaemon::new();
    here(&daemon);
    let console = FakeConsole::pipe();

    let exit = run(&daemon, &console, &["--json", "host", "ls"]).await;

    assert_eq!(exit, exit::OK);
    let printed = console.json();
    let machines = printed["result"]["machines"].as_array().expect("a machines list");
    assert_eq!(machines.len(), 1);
    assert_eq!(machines[0]["id"], "ma_here");
}

#[tokio::test]
async fn host_ls_puts_this_machine_beside_the_others() {
    let daemon = FakeDaemon::new();
    daemon.reply(
        rpc::machine::LIST_MACHINES,
        &pb::ListMachinesResponse {
            machines: vec![pb::Machine {
                id: support::machine("ma_box"),
                name: "box".into(),
                ..Default::default()
            }],
            ..Default::default()
        },
    );
    here(&daemon);
    let console = FakeConsole::pipe();

    let exit = run(&daemon, &console, &["--json", "host", "ls"]).await;

    assert_eq!(exit, exit::OK);
    let printed = console.json();
    let ids: Vec<_> = printed["result"]["machines"]
        .as_array()
        .expect("a machines list")
        .iter()
        .map(|machine| machine["id"].as_str().unwrap_or_default().to_owned())
        .collect();
    assert_eq!(ids, vec!["ma_here", "ma_box"]);
}

#[tokio::test]
async fn host_ls_does_not_list_this_machine_twice() {
    let daemon = FakeDaemon::new();
    daemon.reply(
        rpc::machine::LIST_MACHINES,
        &pb::ListMachinesResponse {
            machines: vec![pb::Machine {
                id: support::machine("ma_here"),
                name: "here".into(),
                ..Default::default()
            }],
            ..Default::default()
        },
    );
    here(&daemon);
    let console = FakeConsole::pipe();

    run(&daemon, &console, &["--json", "host", "ls"]).await;

    let printed = console.json();
    assert_eq!(printed["result"]["machines"].as_array().map(Vec::len), Some(1));
}

#[tokio::test]
async fn start_without_a_machine_goes_to_this_machine() {
    let daemon = FakeDaemon::new();
    here(&daemon);
    let console = FakeConsole::pipe();

    let exit = run(&daemon, &console, &["--json", "--channel", "ch_1", "session", "start"]).await;

    assert_eq!(exit, exit::OK);
    let sent: pb::StartSessionRequest = daemon.only(rpc::session::START_SESSION);
    assert_eq!(sent.machine, support::machine("ma_here"));
}

#[tokio::test]
async fn start_with_a_machine_keeps_it() {
    let daemon = FakeDaemon::new();
    here(&daemon);
    let console = FakeConsole::pipe();

    let exit = run(
        &daemon,
        &console,
        &["--json", "--channel", "ch_1", "--machine", "ma_box", "session", "start"],
    )
    .await;

    assert_eq!(exit, exit::OK);
    let sent: pb::StartSessionRequest = daemon.only(rpc::session::START_SESSION);
    assert_eq!(sent.machine, support::machine("ma_box"));
}

#[tokio::test]
async fn the_this_mac_alias_names_this_machine() {
    let daemon = FakeDaemon::new();
    here(&daemon);
    let console = FakeConsole::pipe();

    let exit = run(
        &daemon,
        &console,
        &["--json", "--channel", "ch_1", "--machine", "this-mac", "session", "start"],
    )
    .await;

    assert_eq!(exit, exit::OK);
    let sent: pb::StartSessionRequest = daemon.only(rpc::session::START_SESSION);
    assert_eq!(sent.machine, support::machine("ma_here"));
}

/// D768: the daemon of machine vm-a (`ma_vm_a`), whose agent the host
/// answers with This Mac (`ma_host`) and vm-b.
fn on_a_machine(daemon: &FakeDaemon) {
    daemon.reply(
        rpc::daemon::GET_SERVER_INFO,
        &pb::ServerInfo {
            machine: support::machine("ma_vm_a"),
            role: pb::DaemonRole::Machine as i32,
            ..Default::default()
        },
    );
    daemon.reply(
        rpc::machine::LIST_MACHINES,
        &pb::ListMachinesResponse {
            machines: vec![
                pb::Machine {
                    id: support::machine("ma_host"),
                    name: "This Mac".into(),
                    is_this_mac: true,
                    ..Default::default()
                },
                pb::Machine {
                    id: support::machine("ma_vm_a"),
                    name: "vm-a".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        },
    );
}

fn as_agent() -> humata2_cli::Environment {
    humata2_cli::Environment {
        session_token: Some("session-secret".into()),
        ..Default::default()
    }
}

#[tokio::test]
async fn this_mac_for_an_agent_on_a_machine_is_the_host_mac() {
    let daemon = FakeDaemon::new();
    on_a_machine(&daemon);
    let console = FakeConsole::pipe();
    let words = ["--json", "--channel", "ch_1", "cp", "this-mac:shared/agent-note.txt", "ma_vm_a:shared/"];

    let exit = support::run_as(&as_agent(), &daemon, &console, &words).await;

    assert_eq!(exit, exit::OK, "{}", console.stdout());
    let sent: pb::StartTransferRequest = daemon.only(rpc::transfer::START_TRANSFER);
    assert_eq!(sent.sources.len(), 1);
    assert_eq!(sent.sources[0].machine, support::machine("ma_host"));
}

#[tokio::test]
async fn host_ls_for_an_agent_on_a_machine_lists_this_mac_once() {
    let daemon = FakeDaemon::new();
    on_a_machine(&daemon);
    let console = FakeConsole::pipe();

    let exit = support::run_as(&as_agent(), &daemon, &console, &["--json", "host", "ls"]).await;

    assert_eq!(exit, exit::OK);
    let printed = console.json();
    let marked: Vec<_> = printed["result"]["machines"]
        .as_array()
        .expect("a machines list")
        .iter()
        .filter(|machine| machine["is_this_mac"] == true)
        .map(|machine| machine["id"].as_str().unwrap_or_default().to_owned())
        .collect();
    assert_eq!(marked, vec!["ma_host"]);
}
