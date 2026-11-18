//! `humata stop` and `humata start` on a machine (STOP, D263) over a fake
//! daemon: what each asks the daemon, what it prints, and how a refusal
//! ends it.

mod support;

use humata2_cli::{Environment, exit, rpc};
use humata2_proto::humata::v1 as pb;
use support::{FakeConsole, FakeDaemon, run, run_as};

fn stopped(stop_id: &str) -> pb::StopSessionsResponse {
    pb::StopSessionsResponse {
        stopped: Some(pb::SessionsStopped {
            stop_id: stop_id.to_owned(),
            reason: "loop".to_owned(),
            by: Some(pb::StoppedBy {
                kind: pb::StoppedByKind::Terminal as i32,
                via: pb::StopVia::Cli as i32,
                ..Default::default()
            }),
            ..Default::default()
        }),
    }
}

fn agent() -> Environment {
    Environment {
        session_token: Some("session-secret".into()),
        ..Environment::default()
    }
}

#[tokio::test]
async fn stop_asks_the_daemon_once_from_the_cli_with_the_reason_and_a_stop_id() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::daemon::STOP_SESSIONS, &stopped("s1"));
    let console = FakeConsole::pipe();
    let code = run(&daemon, &console, &["stop", "--reason", "agents looping"]).await;
    assert_eq!(code, exit::OK, "{}", console.stderr());
    let sent: pb::StopSessionsRequest = daemon.only(rpc::daemon::STOP_SESSIONS);
    assert_eq!(sent.reason, "agents looping");
    assert_eq!(sent.via(), pb::StopVia::Cli);
    assert!(!sent.stop_id.is_empty(), "the stop carries an id, so a retry is the same stop");
    assert!(!daemon.called(rpc::daemon::START_SESSIONS));
}

#[tokio::test]
async fn stop_without_a_reason_still_stops() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::daemon::STOP_SESSIONS, &stopped("s1"));
    let console = FakeConsole::pipe();
    let code = run(&daemon, &console, &["stop"]).await;
    assert_eq!(code, exit::OK, "{}", console.stderr());
    let sent: pb::StopSessionsRequest = daemon.only(rpc::daemon::STOP_SESSIONS);
    assert_eq!(sent.via(), pb::StopVia::Cli);
}

#[tokio::test]
async fn stop_in_json_names_the_stop_the_daemon_made() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::daemon::STOP_SESSIONS, &stopped("s1"));
    let console = FakeConsole::pipe();
    let code = run(&daemon, &console, &["--json", "stop", "--reason", "loop"]).await;
    assert_eq!(code, exit::OK, "{}", console.stderr());
    let json = console.json();
    assert_eq!(json["ok"], true, "{json}");
    assert_eq!(json["result"]["stopped"]["stop_id"], "s1", "{json}");
}

#[tokio::test]
async fn start_asks_the_daemon_to_start_the_sessions() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::daemon::START_SESSIONS, &pb::StartSessionsResponse::default());
    let console = FakeConsole::pipe();
    let code = run(&daemon, &console, &["start"]).await;
    assert_eq!(code, exit::OK, "{}", console.stderr());
    daemon.only::<pb::StartSessionsRequest>(rpc::daemon::START_SESSIONS);
    assert!(!daemon.called(rpc::daemon::STOP_SESSIONS));
}

#[tokio::test]
async fn an_agents_stop_denied_by_its_rule_exits_denied_with_the_fix() {
    let daemon = FakeDaemon::new();
    daemon.refuse_with(
        rpc::daemon::STOP_SESSIONS,
        pb::Error {
            message: "Payments' rule never lets an agent stop or start a machine".into(),
            fix: "Ask the user to stop it, or change the rule in Agent access".into(),
            kind: Some(pb::error::Kind::Denied(pb::Denied {
                source: pb::denied::Source::NeverRule as i32,
                capability: pb::Capability::MachineSessionsStop as i32,
                ..Default::default()
            })),
            ..Default::default()
        },
    );
    let console = FakeConsole::pipe();
    let code = run_as(&agent(), &daemon, &console, &["--json", "stop", "--reason", "loop"]).await;
    assert_eq!(code, exit::DENIED);
    let json = console.json();
    assert_eq!(json["error"]["kind"], "denied", "{json}");
    assert!(json["error"]["fix"].as_str().is_some_and(|fix| !fix.is_empty()), "{json}");
}

#[tokio::test]
async fn a_start_refused_for_want_of_the_user_exits_needs_user() {
    let daemon = FakeDaemon::new();
    daemon.refuse(
        rpc::daemon::START_SESSIONS,
        pb::error::Kind::UserOnly(pb::UserOnly {
            capability: pb::Capability::MachineSessionsStop as i32,
        }),
    );
    let console = FakeConsole::pipe();
    let code = run(&daemon, &console, &["--json", "start"]).await;
    assert_eq!(code, exit::NEEDS_USER);
}

#[tokio::test]
async fn stop_takes_no_machine_and_session_stop_is_unchanged() {
    // The top-level `stop` means this machine; a session is stopped with
    // `humata session stop <session>`.
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let code = run(&daemon, &console, &["stop", "se_1"]).await;
    assert_eq!(code, exit::USAGE);
    assert!(!daemon.called(rpc::daemon::STOP_SESSIONS));
}
