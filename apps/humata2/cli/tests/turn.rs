//! `humata turn` (SE-21, D169): reports the turn and prints nothing, and a
//! hook can never fail or wait long on the daemon.

mod support;

use humata2_cli::{exit, rpc};
use humata2_proto::humata::v1 as pb;
use support::{FakeConsole, FakeDaemon, run};

#[tokio::test]
async fn each_state_is_sent_as_a_turn_report_and_nothing_is_printed() {
    for (word, state) in [
        ("working", pb::TurnState::Working),
        ("idle", pb::TurnState::Idle),
        ("needs-input", pb::TurnState::NeedsInput),
    ] {
        let daemon = FakeDaemon::new();
        daemon.reply(rpc::session::REPORT_TURN, &pb::ReportTurnResponse {});
        let console = FakeConsole::pipe();
        assert_eq!(run(&daemon, &console, &["turn", word]).await, exit::OK);
        let sent: pb::ReportTurnRequest = daemon.only(rpc::session::REPORT_TURN);
        assert_eq!(sent.state, state as i32);
        assert!(sent.reported_at_unix_ns > 0);
        assert_eq!(console.stdout(), "");
        assert_eq!(console.stderr(), "");
    }
}

#[tokio::test]
async fn a_refusal_is_not_an_error_for_the_hook() {
    let daemon = FakeDaemon::new();
    daemon.status(rpc::session::REPORT_TURN, 7);
    let console = FakeConsole::pipe();
    assert_eq!(run(&daemon, &console, &["turn", "idle"]).await, exit::OK);
    assert_eq!(console.stdout(), "");
    assert_eq!(console.stderr(), "");
}

#[tokio::test(start_paused = true)]
async fn a_daemon_that_never_answers_is_given_up_on() {
    let daemon = FakeDaemon::new();
    daemon.hang(rpc::session::REPORT_TURN);
    let console = FakeConsole::pipe();
    assert_eq!(run(&daemon, &console, &["turn", "working"]).await, exit::OK);
}

#[tokio::test]
async fn an_unknown_state_is_a_usage_error() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    assert_eq!(run(&daemon, &console, &["turn", "sleepy"]).await, exit::USAGE);
    assert!(!daemon.called(rpc::session::REPORT_TURN));
}
