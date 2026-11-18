//! Presence, plans and waiting for answers (CLI.md §3, §6).

mod support;

use humata2_cli::{exit, rpc};
use humata2_proto::humata::v1 as pb;
use pb::presence_proof::Proof;
use pb::presence_required::Method;
use support::{FakeConsole, FakeDaemon, run, run_interrupted};

fn presence(accepted: &[Method], in_session: bool) -> pb::error::Kind {
    pb::error::Kind::PresenceRequired(pb::PresenceRequired {
        accepted: accepted.iter().map(|method| *method as i32).collect(),
        caller_in_session_terminal: in_session,
        tries_left: 0,
    })
}

#[tokio::test]
async fn presence_is_asked_for_in_the_app_and_the_call_sent_again() {
    let daemon = FakeDaemon::new();
    daemon.refuse(
        rpc::inbox::ANSWER_REQUEST,
        presence(&[Method::ConfirmInApp, Method::ApprovalPassphrase], false),
    );
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["--json", "inbox", "approve", "rq_1"]).await;
    assert_eq!(exit, exit::OK);
    let sent: Vec<pb::AnswerRequestRequest> = daemon.sent(rpc::inbox::ANSWER_REQUEST);
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0].presence, None);
    assert!(matches!(
        sent[1].presence.as_ref().and_then(|proof| proof.proof.as_ref()),
        Some(Proof::ConfirmInApp(_))
    ));
    // Both are the same intent.
    assert_eq!(sent[0].idempotency_key, sent[1].idempotency_key);
}

#[tokio::test]
async fn a_passphrase_is_typed_at_the_terminal_for_a_login() {
    let home = tempfile::tempdir().unwrap_or_else(|e| panic!("temp dir: {e}"));
    let environment = humata2_cli::Environment {
        home: Some(home.path().to_path_buf()),
        terminal_session: Some(7),
        ..humata2_cli::Environment::default()
    };
    let daemon = FakeDaemon::new();
    daemon
        .refuse(
            rpc::cli_auth::BEGIN_CLI_SESSION,
            presence(&[Method::ApprovalPassphrase], false),
        )
        .reply(
            rpc::cli_auth::BEGIN_CLI_SESSION,
            &pb::CliSession {
                id: "cl_1".into(),
                token: "0123456789abcdef".into(),
                ..Default::default()
            },
        );
    let console = FakeConsole::terminal().typing_secret("correct horse");
    assert_eq!(
        support::run_as(&environment, &daemon, &console, &["login"]).await,
        exit::OK
    );
    let sent: Vec<pb::BeginCliSessionRequest> = daemon.sent(rpc::cli_auth::BEGIN_CLI_SESSION);
    assert_eq!(
        sent[1].presence.as_ref().and_then(|proof| proof.proof.clone()),
        Some(Proof::ApprovalPassphrase("correct horse".into()))
    );
}

#[tokio::test]
async fn an_answer_refused_for_a_passphrase_is_not_retried_with_one() {
    // The daemon reads the passphrase only at `humata login`; a passphrase
    // sent with an answer would be ignored and the answer refused again.
    let daemon = FakeDaemon::new();
    daemon.refuse(
        rpc::inbox::ANSWER_REQUEST,
        presence(&[Method::ApprovalPassphrase], false),
    );
    let console = FakeConsole::terminal().typing_secret("correct horse");
    let exit = run(&daemon, &console, &["inbox", "approve", "rq_1"]).await;
    assert_eq!(exit, exit::NEEDS_USER);
    assert_eq!(daemon.sent::<pb::AnswerRequestRequest>(rpc::inbox::ANSWER_REQUEST).len(), 1);
}

#[tokio::test]
async fn a_passphrase_is_never_read_from_a_pipe() {
    let daemon = FakeDaemon::new();
    daemon.refuse(
        rpc::inbox::ANSWER_REQUEST,
        presence(&[Method::ApprovalPassphrase], false),
    );
    let console = FakeConsole::pipe().with_stdin(b"correct horse\n");
    let exit = run(&daemon, &console, &["--json", "inbox", "approve", "rq_1"]).await;
    assert_eq!(exit, exit::NEEDS_USER);
    assert_eq!(console.json()["error"]["kind"], "presence_required");
    assert_eq!(daemon.sent::<pb::AnswerRequestRequest>(rpc::inbox::ANSWER_REQUEST).len(), 1);
}

#[tokio::test]
async fn presence_is_never_offered_from_a_sessions_terminal() {
    let daemon = FakeDaemon::new();
    daemon.refuse(
        rpc::inbox::ANSWER_REQUEST,
        presence(&[Method::ConfirmInApp], true),
    );
    let console = FakeConsole::terminal().typing_secret("correct horse");
    let exit = run(&daemon, &console, &["inbox", "approve", "rq_1"]).await;
    assert_eq!(exit, exit::NEEDS_USER);
    assert_eq!(daemon.sent::<pb::AnswerRequestRequest>(rpc::inbox::ANSWER_REQUEST).len(), 1);
}

#[tokio::test]
async fn presence_asked_twice_is_reported() {
    let daemon = FakeDaemon::new();
    let kind = || presence(&[Method::ConfirmInApp], false);
    daemon
        .refuse(rpc::inbox::ANSWER_REQUEST, kind())
        .refuse(rpc::inbox::ANSWER_REQUEST, kind());
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["--json", "inbox", "approve", "rq_1"]).await;
    assert_eq!(exit, exit::NEEDS_USER);
    assert_eq!(daemon.sent::<pb::AnswerRequestRequest>(rpc::inbox::ANSWER_REQUEST).len(), 2);
}

fn plan() -> pb::EmptyTrashPlan {
    pb::EmptyTrashPlan {
        machine: support::machine("ma_1"),
        frees_bytes: 4096,
        items: 3,
        token: Some(pb::PlanToken {
            value: "plan-7".into(),
        }),
    }
}

#[tokio::test]
async fn dry_run_prints_the_plan_and_changes_nothing() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::trash::PLAN_EMPTY_TRASH, &plan());
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &["--json", "--dry-run", "--machine", "ma_1", "trash", "empty"],
    )
    .await;
    assert_eq!(exit, exit::OK);
    let printed = console.json();
    assert_eq!(printed["outcome"], "planned");
    assert_eq!(printed["result"]["token"], "plan-7");
    assert_eq!(printed["result"]["items"], 3);
    assert!(!daemon.called(rpc::trash::EMPTY_TRASH));
}

#[tokio::test]
async fn plan_does_what_the_printed_plan_said() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &["--json", "--plan", "plan-7", "--machine", "ma_1", "trash", "empty"],
    )
    .await;
    assert_eq!(exit, exit::OK);
    assert!(!daemon.called(rpc::trash::PLAN_EMPTY_TRASH));
    let sent: pb::EmptyTrashRequest = daemon.only(rpc::trash::EMPTY_TRASH);
    assert_eq!(sent.plan, Some(pb::PlanToken { value: "plan-7".into() }));
    assert_eq!(sent.machine, support::machine("ma_1"));
}

#[tokio::test]
async fn a_change_with_no_one_to_confirm_it_is_refused() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::trash::PLAN_EMPTY_TRASH, &plan());
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["--json", "--machine", "ma_1", "trash", "empty"]).await;
    assert_eq!(exit, exit::USAGE);
    assert!(!daemon.called(rpc::trash::EMPTY_TRASH));
}

#[tokio::test]
async fn yes_skips_the_question_and_uses_the_plans_token() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::trash::PLAN_EMPTY_TRASH, &plan());
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["--json", "--yes", "--machine", "ma_1", "trash", "empty"]).await;
    assert_eq!(exit, exit::OK);
    let sent: pb::EmptyTrashRequest = daemon.only(rpc::trash::EMPTY_TRASH);
    assert_eq!(sent.plan, Some(pb::PlanToken { value: "plan-7".into() }));
}

#[tokio::test]
async fn answering_no_changes_nothing() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::trash::PLAN_EMPTY_TRASH, &plan());
    let console = FakeConsole::terminal().answering(false);
    let exit = run(&daemon, &console, &["--machine", "ma_1", "trash", "empty"]).await;
    // Declining is not Ctrl-C (130): scripts tell the two apart.
    assert_eq!(exit, exit::DECLINED);
    assert_ne!(exit, exit::INTERRUPTED);
    assert!(!console.stderr().is_empty(), "a declined run says so on stderr");
    assert!(!daemon.called(rpc::trash::EMPTY_TRASH));
}

#[tokio::test]
async fn answering_yes_goes_ahead() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::trash::PLAN_EMPTY_TRASH, &plan());
    let console = FakeConsole::terminal().answering(true);
    let exit = run(&daemon, &console, &["--machine", "ma_1", "trash", "empty"]).await;
    assert_eq!(exit, exit::OK);
    assert!(daemon.called(rpc::trash::EMPTY_TRASH));
}

fn pending() -> pb::WaitResponse {
    pb::WaitResponse {
        id: Some(pb::RequestId {
            value: "rq_9".into(),
        }),
        state: pb::RequestState::Pending as i32,
        ..Default::default()
    }
}

const ASK_AHEAD: [&str; 6] = [
    "access",
    "ask-ahead",
    "network_reach",
    "network:api.example.com:443",
    "--reason",
    "file the follow-ups",
];

#[tokio::test]
async fn json_doesnt_wait_and_exits_12_with_the_request() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::inbox::REQUEST_ACCESS, &pending());
    let console = FakeConsole::pipe();
    let words: Vec<&str> = ["--json"].into_iter().chain(ASK_AHEAD).collect();
    assert_eq!(run(&daemon, &console, &words).await, exit::PENDING);
    let printed = console.json();
    assert_eq!(printed["error"]["kind"], "pending");
    assert_eq!(printed["error"]["details"]["request"], "rq_9");
    let sent: pb::RequestAccessRequest = daemon.only(rpc::inbox::REQUEST_ACCESS);
    assert_eq!(sent.wait_seconds, 0);
    assert_eq!(sent.capability, pb::Capability::NetworkReach as i32);
    assert!(!daemon.called(rpc::inbox::WAIT_FOR_ANSWER));
}

#[tokio::test]
async fn wait_asks_the_daemon_to_hold_the_answer_that_long() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::inbox::REQUEST_ACCESS, &pending());
    let console = FakeConsole::pipe();
    let words: Vec<&str> = ["--json", "--wait", "30"].into_iter().chain(ASK_AHEAD).collect();
    assert_eq!(run(&daemon, &console, &words).await, exit::PENDING);
    let sent: pb::RequestAccessRequest = daemon.only(rpc::inbox::REQUEST_ACCESS);
    assert_eq!(sent.wait_seconds, 30);
}

#[tokio::test]
async fn a_terminal_waits_until_the_answer_comes() {
    let daemon = FakeDaemon::new();
    let keepalive = pb::WaitResponse {
        keepalive: true,
        ..pending()
    };
    let approved = pb::WaitResponse {
        state: pb::RequestState::Approved as i32,
        ..pending()
    };
    daemon
        .reply(rpc::inbox::REQUEST_ACCESS, &pending())
        .stream(rpc::inbox::WAIT_FOR_ANSWER, &[&keepalive, &keepalive, &approved]);
    let console = FakeConsole::terminal();
    assert_eq!(run(&daemon, &console, &ASK_AHEAD).await, exit::OK);
    // One stream held until the answer, never asked again (TR-7).
    let wait: pb::WaitForAnswerRequest = daemon.only(rpc::inbox::WAIT_FOR_ANSWER);
    assert_eq!(wait.id, pending().id);
}

#[tokio::test]
async fn a_wait_that_ends_unanswered_is_pending_not_a_version_mismatch() {
    let daemon = FakeDaemon::new();
    let keepalive = pb::WaitResponse {
        keepalive: true,
        ..pending()
    };
    daemon
        .reply(rpc::inbox::REQUEST_ACCESS, &pending())
        .stream(rpc::inbox::WAIT_FOR_ANSWER, &[&keepalive]);
    let console = FakeConsole::terminal();
    assert_eq!(run(&daemon, &console, &ASK_AHEAD).await, exit::PENDING);
    assert_eq!(daemon.sent::<pb::WaitForAnswerRequest>(rpc::inbox::WAIT_FOR_ANSWER).len(), 1);
    // The error names the request that is still waiting.
    let json_console = FakeConsole::pipe();
    let again = FakeDaemon::new();
    again
        .reply(rpc::inbox::REQUEST_ACCESS, &pending())
        .stream(rpc::inbox::WAIT_FOR_ANSWER, &[&pb::WaitResponse { keepalive: true, ..pending() }]);
    let words: Vec<&str> = ["--json", "--wait", "5"].into_iter().chain(ASK_AHEAD).collect();
    assert_eq!(run(&again, &json_console, &words).await, exit::PENDING);
    assert_eq!(json_console.json()["error"]["kind"], "pending");
    assert_eq!(json_console.json()["error"]["details"]["request"], "rq_9");
}

#[tokio::test]
async fn a_denied_request_is_an_error_with_its_kind() {
    let daemon = FakeDaemon::new();
    daemon.reply(
        rpc::inbox::REQUEST_ACCESS,
        &pb::WaitResponse {
            state: pb::RequestState::Denied as i32,
            error: Some(pb::Error {
                kind: Some(pb::error::Kind::Denied(Default::default())),
                ..Default::default()
            }),
            ..pending()
        },
    );
    let console = FakeConsole::pipe();
    let words: Vec<&str> = ["--json"].into_iter().chain(ASK_AHEAD).collect();
    assert_eq!(run(&daemon, &console, &words).await, exit::DENIED);
    assert_eq!(console.json()["error"]["kind"], "denied");
}

#[tokio::test]
async fn ctrl_c_stops_waiting_with_130() {
    let daemon = FakeDaemon::new();
    daemon
        .reply(rpc::inbox::REQUEST_ACCESS, &pending())
        .hang(rpc::inbox::WAIT_FOR_ANSWER);
    let console = FakeConsole::terminal();
    assert_eq!(run_interrupted(&daemon, &console, &ASK_AHEAD).await, exit::INTERRUPTED);
}
