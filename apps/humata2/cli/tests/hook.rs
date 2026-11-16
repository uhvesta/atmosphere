//! `humata hook <agent> permission` (HOOKS, D191): passes the agent's
//! payload to the daemon and prints the daemon's answer as it is. It always
//! exits 0, and prints nothing when there's no answer, so the agent's own
//! prompt takes over whatever goes wrong.

mod support;

use humata2_cli::{exit, rpc};
use humata2_proto::humata::v1 as pb;
use support::{FakeConsole, FakeDaemon, run};

const PAYLOAD: &[u8] = br#"{"hook_event_name":"PermissionRequest","tool_name":"Bash","tool_input":{"command":"ls"}}"#;
const ALLOW: &[u8] = br#"{"behavior":"allow"}"#;

#[tokio::test]
async fn the_payload_goes_to_the_daemon_and_its_answer_is_printed_as_it_is() {
    let daemon = FakeDaemon::new();
    daemon.stream(
        rpc::session::HOOK,
        &[
            &pb::HookReply {
                keepalive: true,
                stdout: Vec::new(),
            },
            &pb::HookReply {
                keepalive: false,
                stdout: ALLOW.to_vec(),
            },
        ],
    );
    let console = FakeConsole::pipe().with_stdin(PAYLOAD);
    assert_eq!(
        run(&daemon, &console, &["hook", "copilot", "permission"]).await,
        exit::OK
    );
    let sent: pb::HookRequest = daemon.only(rpc::session::HOOK);
    assert_eq!(sent.agent, "copilot");
    assert_eq!(sent.event, "permission");
    assert_eq!(sent.payload, PAYLOAD);
    assert_eq!(console.written(), ALLOW);
    assert_eq!(console.stderr(), "");
}

#[tokio::test]
async fn no_decision_prints_nothing() {
    let daemon = FakeDaemon::new();
    daemon.stream(
        rpc::session::HOOK,
        &[&pb::HookReply {
            keepalive: false,
            stdout: Vec::new(),
        }],
    );
    let console = FakeConsole::pipe().with_stdin(PAYLOAD);
    assert_eq!(
        run(&daemon, &console, &["hook", "claude", "permission"]).await,
        exit::OK
    );
    assert!(console.written().is_empty());
    assert_eq!(console.stdout(), "");
}

#[tokio::test]
async fn a_stream_that_ends_with_no_answer_prints_nothing() {
    let daemon = FakeDaemon::new();
    daemon.stream(
        rpc::session::HOOK,
        &[&pb::HookReply {
            keepalive: true,
            stdout: Vec::new(),
        }],
    );
    let console = FakeConsole::pipe().with_stdin(PAYLOAD);
    assert_eq!(
        run(&daemon, &console, &["hook", "codex", "permission"]).await,
        exit::OK
    );
    assert!(console.written().is_empty());
    assert_eq!(console.stderr(), "");
}

#[tokio::test]
async fn a_refusal_or_a_daemon_error_prints_nothing_and_exits_0() {
    let daemon = FakeDaemon::new();
    daemon.status(rpc::session::HOOK, 14);
    let console = FakeConsole::pipe().with_stdin(PAYLOAD);
    assert_eq!(
        run(&daemon, &console, &["hook", "claude", "permission"]).await,
        exit::OK
    );
    assert!(console.written().is_empty());
    assert_eq!(console.stdout(), "");
    assert_eq!(console.stderr(), "");
}

#[tokio::test]
async fn a_payload_over_the_cap_is_not_sent() {
    let daemon = FakeDaemon::new();
    let huge = vec![b'a'; 2 * 1024 * 1024];
    let console = FakeConsole::pipe().with_stdin(&huge);
    assert_eq!(
        run(&daemon, &console, &["hook", "claude", "permission"]).await,
        exit::OK
    );
    assert!(!daemon.called(rpc::session::HOOK));
    assert!(console.written().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_daemon_that_never_answers_is_given_up_on() {
    let daemon = FakeDaemon::new();
    daemon.hang(rpc::session::HOOK);
    let console = FakeConsole::pipe().with_stdin(PAYLOAD);
    assert_eq!(
        run(&daemon, &console, &["hook", "claude", "permission"]).await,
        exit::OK
    );
    assert!(console.written().is_empty());
}
