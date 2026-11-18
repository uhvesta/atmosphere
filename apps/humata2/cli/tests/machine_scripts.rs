//! `humata machine script list|check|run|log` over a fake daemon: what each
//! command asks the daemon, what `--json` carries back, and how a refusal or
//! an invalid rules file ends it (MACHINESCRIPTS, D687, D688).

mod support;

use humata2_cli::{exit, rpc};
use humata2_proto::humata::v1 as pb;
use support::{FakeConsole, FakeDaemon, machine, run, session};

fn passed_run() -> pb::MachineScriptRun {
    pb::MachineScriptRun {
        id: "run_1".into(),
        machine: machine("ma_1"),
        rule: "setup".into(),
        trigger: "connected".into(),
        took_ms: 1_500,
        outcome: pb::MachineScriptOutcome::Passed as i32,
        steps: vec![pb::MachineScriptStepRun {
            index: 0,
            kind: "run".into(),
            label: "setup-tools.sh".into(),
            outcome: pb::MachineScriptOutcome::Passed as i32,
            exit: Some(0),
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn listed() -> pb::ListMachineScriptsResponse {
    pb::ListMachineScriptsResponse {
        rules: vec![pb::MachineScriptRule {
            name: "setup".into(),
            triggers: vec!["added".into(), "connected".into()],
            machines: Vec::new(),
            once: false,
            steps: vec![pb::MachineScriptStep {
                kind: "run".into(),
                label: "setup-tools.sh".into(),
            }],
        }],
        last_runs: vec![passed_run()],
        scripts: vec!["setup-tools.sh".into()],
        problem: None,
    }
}

#[tokio::test]
async fn list_without_a_machine_asks_for_every_machine() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::machine::LIST_MACHINE_SCRIPTS, &listed());
    let console = FakeConsole::pipe();
    let code = run(&daemon, &console, &["machine", "script", "list"]).await;
    assert_eq!(code, exit::OK, "{}", console.stderr());
    let sent: pb::ListMachineScriptsRequest = daemon.only(rpc::machine::LIST_MACHINE_SCRIPTS);
    assert_eq!(sent.machine, None);
    assert!(console.stdout().contains("setup"), "{}", console.stdout());
}

#[tokio::test]
async fn list_with_a_machine_asks_for_that_machine() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::machine::LIST_MACHINE_SCRIPTS, &listed());
    let console = FakeConsole::pipe();
    let code = run(&daemon, &console, &["machine", "script", "list", "ma_1"]).await;
    assert_eq!(code, exit::OK, "{}", console.stderr());
    let sent: pb::ListMachineScriptsRequest = daemon.only(rpc::machine::LIST_MACHINE_SCRIPTS);
    assert_eq!(sent.machine, machine("ma_1"));
}

#[tokio::test]
async fn list_in_json_carries_the_rules_last_runs_and_scripts() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::machine::LIST_MACHINE_SCRIPTS, &listed());
    let console = FakeConsole::pipe();
    let code = run(&daemon, &console, &["--json", "machine", "script", "list"]).await;
    assert_eq!(code, exit::OK, "{}", console.stderr());
    let json = console.json();
    assert_eq!(json["result"]["rules"][0]["name"], "setup", "{json}");
    assert_eq!(json["result"]["rules"][0]["triggers"][1], "connected", "{json}");
    assert_eq!(json["result"]["last_runs"][0]["outcome"], "passed", "{json}");
    assert_eq!(json["result"]["scripts"][0], "setup-tools.sh", "{json}");
}

#[tokio::test]
async fn list_of_an_unknown_machine_is_not_found() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::machine::LIST_MACHINES, &pb::ListMachinesResponse::default());
    let console = FakeConsole::pipe();
    let code = run(&daemon, &console, &["machine", "script", "list", "ghost"]).await;
    assert_eq!(code, exit::NOT_FOUND, "{}", console.stderr());
    assert!(!daemon.called(rpc::machine::LIST_MACHINE_SCRIPTS));
}

#[tokio::test]
async fn check_of_a_valid_file_exits_ok_and_carries_the_warnings() {
    let daemon = FakeDaemon::new();
    daemon.reply(
        rpc::machine::CHECK_MACHINE_SCRIPTS,
        &pb::CheckMachineScriptsResponse {
            valid: true,
            problems: Vec::new(),
            warnings: vec!["rule setup names a machine that isn't added".into()],
        },
    );
    let console = FakeConsole::pipe();
    let code = run(&daemon, &console, &["--json", "machine", "script", "check"]).await;
    assert_eq!(code, exit::OK, "{}", console.stderr());
    let json = console.json();
    assert_eq!(json["result"]["valid"], true, "{json}");
    assert_eq!(json["result"]["warnings"].as_array().map(Vec::len), Some(1), "{json}");
}

#[tokio::test]
async fn check_of_an_invalid_file_exits_usage_and_lists_every_problem() {
    let daemon = FakeDaemon::new();
    daemon.reply(
        rpc::machine::CHECK_MACHINE_SCRIPTS,
        &pb::CheckMachineScriptsResponse {
            valid: false,
            problems: vec!["first problem".into(), "second problem".into()],
            warnings: Vec::new(),
        },
    );
    let console = FakeConsole::pipe();
    let code = run(&daemon, &console, &["--json", "machine", "script", "check"]).await;
    assert_eq!(code, exit::USAGE);
    let json = console.json();
    assert_eq!(json["ok"], false, "{json}");
    assert_eq!(json["error"]["kind"], "invalid_field", "{json}");
    assert_eq!(json["error"]["details"]["problems"].as_array().map(Vec::len), Some(2), "{json}");
}

#[tokio::test]
async fn run_sends_the_rule_machine_and_session_with_a_key() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::machine::RUN_MACHINE_SCRIPT, &pb::Operation::default());
    let console = FakeConsole::pipe();
    let code = run(
        &daemon,
        &console,
        &["machine", "script", "run", "setup", "ma_1", "--session", "se_9"],
    )
    .await;
    assert_eq!(code, exit::OK, "{}", console.stderr());
    let sent: pb::RunMachineScriptRequest = daemon.only(rpc::machine::RUN_MACHINE_SCRIPT);
    assert_eq!(sent.rule, "setup");
    assert_eq!(sent.machine, machine("ma_1"));
    assert_eq!(sent.session, session("se_9"));
    assert!(sent.idempotency_key.is_some_and(|key| !key.value.is_empty()));
}

#[tokio::test]
async fn run_without_a_session_sends_none() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::machine::RUN_MACHINE_SCRIPT, &pb::Operation::default());
    let console = FakeConsole::pipe();
    let code = run(&daemon, &console, &["machine", "script", "run", "setup", "ma_1"]).await;
    assert_eq!(code, exit::OK, "{}", console.stderr());
    let sent: pb::RunMachineScriptRequest = daemon.only(rpc::machine::RUN_MACHINE_SCRIPT);
    assert_eq!(sent.session, None);
}

#[tokio::test]
async fn a_refused_run_exits_denied() {
    let daemon = FakeDaemon::new();
    daemon.refuse(
        rpc::machine::RUN_MACHINE_SCRIPT,
        pb::error::Kind::Denied(pb::Denied::default()),
    );
    let console = FakeConsole::pipe();
    let code = run(&daemon, &console, &["--json", "machine", "script", "run", "setup", "ma_1"]).await;
    assert_eq!(code, exit::DENIED);
    assert_eq!(console.json()["error"]["kind"], "denied");
}

#[tokio::test]
async fn run_on_an_unknown_machine_is_not_found_and_runs_nothing() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::machine::LIST_MACHINES, &pb::ListMachinesResponse::default());
    let console = FakeConsole::pipe();
    let code = run(&daemon, &console, &["machine", "script", "run", "setup", "ghost"]).await;
    assert_eq!(code, exit::NOT_FOUND, "{}", console.stderr());
    assert!(!daemon.called(rpc::machine::RUN_MACHINE_SCRIPT));
}

#[tokio::test]
async fn log_sends_the_machine_rule_and_limit() {
    let daemon = FakeDaemon::new();
    daemon.reply(
        rpc::machine::LIST_MACHINE_SCRIPT_RUNS,
        &pb::ListMachineScriptRunsResponse { runs: vec![passed_run()] },
    );
    let console = FakeConsole::pipe();
    let code = run(
        &daemon,
        &console,
        &["machine", "script", "log", "ma_1", "--rule", "setup", "--limit", "5"],
    )
    .await;
    assert_eq!(code, exit::OK, "{}", console.stderr());
    let sent: pb::ListMachineScriptRunsRequest = daemon.only(rpc::machine::LIST_MACHINE_SCRIPT_RUNS);
    assert_eq!(sent.machine, machine("ma_1"));
    assert_eq!(sent.rule, "setup");
    assert_eq!(sent.limit, 5);
}

#[tokio::test]
async fn log_without_options_asks_for_every_rule_and_the_default_limit() {
    let daemon = FakeDaemon::new();
    daemon.reply(
        rpc::machine::LIST_MACHINE_SCRIPT_RUNS,
        &pb::ListMachineScriptRunsResponse::default(),
    );
    let console = FakeConsole::pipe();
    let code = run(&daemon, &console, &["machine", "script", "log", "ma_1"]).await;
    assert_eq!(code, exit::OK, "{}", console.stderr());
    let sent: pb::ListMachineScriptRunsRequest = daemon.only(rpc::machine::LIST_MACHINE_SCRIPT_RUNS);
    assert_eq!(sent.rule, "");
    assert_eq!(sent.limit, 0);
}

#[tokio::test]
async fn log_in_json_carries_each_run_and_its_steps() {
    let daemon = FakeDaemon::new();
    daemon.reply(
        rpc::machine::LIST_MACHINE_SCRIPT_RUNS,
        &pb::ListMachineScriptRunsResponse { runs: vec![passed_run()] },
    );
    let console = FakeConsole::pipe();
    let code = run(&daemon, &console, &["--json", "machine", "script", "log", "ma_1"]).await;
    assert_eq!(code, exit::OK, "{}", console.stderr());
    let json = console.json();
    let first = &json["result"]["runs"][0];
    assert_eq!(first["id"], "run_1", "{json}");
    assert_eq!(first["trigger"], "connected", "{json}");
    assert_eq!(first["steps"][0]["label"], "setup-tools.sh", "{json}");
}
