//! `humata login` and `humata logout` (CLI.md §5.4, SEC-11): the login's
//! token is kept for this terminal session only, never printed, and sent
//! by every later run from that session.

mod support;

use std::path::Path;

use humata2_cli::{Environment, Principal, exit, principal, rpc};
use humata2_proto::humata::v1 as pb;
use support::{FakeConsole, FakeDaemon, run_as};

const TOKEN: &str = "0123456789abcdef";

fn terminal(home: &Path, session: i32) -> Environment {
    Environment {
        home: Some(home.to_path_buf()),
        terminal_session: Some(session),
        ..Environment::default()
    }
}

fn granted() -> pb::CliSession {
    pb::CliSession {
        id: "cl_1".into(),
        terminal: "Terminal, ttys004".into(),
        token: TOKEN.into(),
        ..Default::default()
    }
}

/// Logs in from terminal session `session` under `home`.
async fn log_in(home: &Path, session: i32) {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::cli_auth::BEGIN_CLI_SESSION, &granted());
    let console = FakeConsole::pipe();
    let code = run_as(&terminal(home, session), &daemon, &console, &["--json", "login"]).await;
    assert_eq!(code, exit::OK);
}

#[tokio::test]
async fn a_login_is_kept_for_its_terminal_session_and_its_token_never_printed() {
    let home = tempfile::tempdir().expect("temp dir");
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::cli_auth::BEGIN_CLI_SESSION, &granted());
    let console = FakeConsole::pipe();
    let code = run_as(
        &terminal(home.path(), 7),
        &daemon,
        &console,
        &["--json", "login", "--hours", "3"],
    )
    .await;
    assert_eq!(code, exit::OK);
    let sent: pb::BeginCliSessionRequest = daemon.only(rpc::cli_auth::BEGIN_CLI_SESSION);
    assert_eq!(sent.hours, 3);
    assert!(!console.stdout().contains(TOKEN));
    assert!(!console.stderr().contains(TOKEN));

    let same = terminal(home.path(), 7).with_saved_login();
    assert_eq!(
        principal(&same),
        Principal::LoggedIn {
            token: TOKEN.into()
        }
    );
    let other = terminal(home.path(), 8).with_saved_login();
    assert_eq!(principal(&other), Principal::User);
}

#[tokio::test]
async fn an_agents_token_wins_over_the_terminals_login() {
    let home = tempfile::tempdir().expect("temp dir");
    log_in(home.path(), 7).await;
    let environment = Environment {
        session_token: Some("agent".into()),
        ..terminal(home.path(), 7)
    }
    .with_saved_login();
    assert!(matches!(principal(&environment), Principal::Agent { .. }));
}

#[tokio::test]
async fn a_refused_login_keeps_nothing() {
    let home = tempfile::tempdir().expect("temp dir");
    let daemon = FakeDaemon::new();
    daemon.refuse(
        rpc::cli_auth::BEGIN_CLI_SESSION,
        pb::error::Kind::Denied(pb::Denied::default()),
    );
    let console = FakeConsole::pipe();
    let code = run_as(&terminal(home.path(), 7), &daemon, &console, &["--json", "login"]).await;
    assert_eq!(code, exit::DENIED);
    assert_eq!(terminal(home.path(), 7).with_saved_login().login, None);
}

#[tokio::test]
async fn a_login_outside_a_terminal_session_asks_nothing_of_the_daemon() {
    let home = tempfile::tempdir().expect("temp dir");
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let environment = Environment {
        home: Some(home.path().to_path_buf()),
        ..Environment::default()
    };
    let code = run_as(&environment, &daemon, &console, &["--json", "login"]).await;
    assert_eq!(code, exit::USAGE);
    assert!(!daemon.called(rpc::cli_auth::BEGIN_CLI_SESSION));
}

#[tokio::test]
async fn logout_ends_the_login_and_forgets_its_token() {
    let home = tempfile::tempdir().expect("temp dir");
    log_in(home.path(), 7).await;
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let environment = terminal(home.path(), 7).with_saved_login();
    let code = run_as(&environment, &daemon, &console, &["--json", "logout"]).await;
    assert_eq!(code, exit::OK);
    let sent: pb::EndCliSessionRequest = daemon.only(rpc::cli_auth::END_CLI_SESSION);
    assert_eq!(sent.id, "");
    assert_eq!(terminal(home.path(), 7).with_saved_login().login, None);
}

#[tokio::test]
async fn logout_forgets_a_login_the_daemon_already_dropped() {
    let home = tempfile::tempdir().expect("temp dir");
    log_in(home.path(), 7).await;
    let daemon = FakeDaemon::new();
    // UNAUTHENTICATED: expired, or humatad restarted.
    daemon.status(rpc::cli_auth::END_CLI_SESSION, 16);
    let console = FakeConsole::pipe();
    let environment = terminal(home.path(), 7).with_saved_login();
    let code = run_as(&environment, &daemon, &console, &["--json", "logout"]).await;
    assert_eq!(code, exit::OK);
    assert_eq!(terminal(home.path(), 7).with_saved_login().login, None);
}

#[tokio::test]
async fn logout_without_a_login_asks_nothing_of_the_daemon() {
    let home = tempfile::tempdir().expect("temp dir");
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let code = run_as(&terminal(home.path(), 7), &daemon, &console, &["--json", "logout"]).await;
    assert_eq!(code, exit::OK);
    assert!(!daemon.called(rpc::cli_auth::END_CLI_SESSION));
}
