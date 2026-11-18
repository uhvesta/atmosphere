//! `humata host root`: no path prints the machine's root, `--default` plans
//! the reset to `~/Humata`, and a path and `--default` together are refused.

mod support;

use humata2_cli::{exit, rpc};
use humata2_proto::humata::v1 as pb;
use support::{FakeConsole, FakeDaemon, run};

fn detail(root: &str) -> pb::MachineDetail {
    pb::MachineDetail {
        machine: Some(pb::Machine {
            id: Some(pb::MachineId { value: "ma_1".into() }),
            root: root.into(),
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[tokio::test]
async fn no_path_prints_the_root_and_changes_nothing() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::machine::GET_MACHINE, &detail("~/Work"));
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["--json", "host", "root", "ma_1"]).await;
    assert_eq!(exit, exit::OK);
    assert_eq!(console.json()["result"]["root"], "~/Work");
    assert!(!daemon.called(rpc::machine::PLAN_CHANGE_ROOT));
    assert!(!daemon.called(rpc::machine::CHANGE_ROOT));
}

#[tokio::test]
async fn default_plans_the_reset_to_the_default_root() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::machine::PLAN_CHANGE_ROOT, &pb::ChangeRootPlan::default());
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["--json", "--dry-run", "host", "root", "ma_1", "--default"]).await;
    assert_eq!(exit, exit::OK);
    let sent: pb::PlanChangeRootRequest = daemon.only(rpc::machine::PLAN_CHANGE_ROOT);
    assert_eq!(sent.root, "~/Humata");
    assert!(!daemon.called(rpc::machine::CHANGE_ROOT));
}

#[tokio::test]
async fn a_path_and_default_together_are_a_usage_error() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["host", "root", "ma_1", "~/Work", "--default"]).await;
    assert_eq!(exit, exit::USAGE);
    assert!(!daemon.called(rpc::machine::PLAN_CHANGE_ROOT));
}
