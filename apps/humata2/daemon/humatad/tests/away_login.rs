//! `humata login` with the approval passphrase on a machine while its host
//! is away (AWAY T6, SEC-10, SEC-13, AW-5, flow §26.10 step 2). A real
//! role-machine daemon gets the passphrase's hash in its policy copy over
//! a real `serve --stdio` link; the link is then dropped. The real `humata`
//! runs in a terminal of its own (a pty, its shell the session leader),
//! and the test types at its prompt as the person would.

use humatad_test_common as common;
use humatad_test_common::away;

use away::{
    Host, NEEDS_USER, OK, PASSPHRASE, RESOURCES, Terminal, WRONG, host_reachable_becomes,
    verifier,
};
use common::{Build, Daemon};

/// A role-machine daemon on which the test process is the app and every
/// `humata` it starts is judged as the release daemon judges it.
async fn machine_daemon() -> Daemon {
    away::machine_in(Build::AdminForThisProcess, common::folder()).await
}

/// A machine that was sent `verifier` and whose host then went away.
async fn away_machine_with(verifier: &str) -> Daemon {
    let daemon = machine_daemon().await;
    let host = Host::with_policy(&daemon, verifier).await;
    host_reachable_becomes(&daemon, true).await;
    drop(host);
    host_reachable_becomes(&daemon, false).await;
    daemon
}

/// A machine that holds the passphrase's hash and whose host went away.
async fn away_machine() -> Daemon {
    away_machine_with(&verifier()).await
}

#[tokio::test(flavor = "multi_thread")]
async fn the_approval_passphrase_logs_in_on_a_machine_while_its_host_is_away() {
    let daemon = away_machine().await;
    let mut terminal = Terminal::open(&daemon);
    assert!(!terminal.is_user().await);

    assert_eq!(terminal.log_in(PASSPHRASE).await, OK);
    assert!(terminal.is_user().await);

    // Another terminal is not covered by this one's login.
    let mut other = Terminal::open(&daemon);
    assert!(!other.is_user().await);

    assert_eq!(
        terminal
            .exit_code(r#""$HUMATA" --json logout > /dev/null 2>&1"#, None)
            .await,
        OK
    );
    assert!(!terminal.is_user().await);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wrong_passphrase_leaves_the_terminal_as_it_was() {
    let daemon = away_machine().await;
    let mut terminal = Terminal::open(&daemon);
    assert_eq!(terminal.log_in(WRONG).await, NEEDS_USER);
    assert!(!terminal.is_user().await);
    // A wrong try short of the fifth doesn't lock: the right one still works.
    assert_eq!(terminal.log_in(PASSPHRASE).await, OK);
    assert!(terminal.is_user().await);
}

#[tokio::test(flavor = "multi_thread")]
async fn five_wrong_tries_lock_the_passphrase_and_a_restart_keeps_the_lock() {
    let mut daemon = away_machine().await;
    let mut terminal = Terminal::open(&daemon);
    for _ in 0..4 {
        assert_eq!(terminal.log_in(WRONG).await, NEEDS_USER);
    }
    assert_eq!(terminal.log_in(WRONG).await, RESOURCES);
    // Locked: even the right passphrase is not checked.
    assert_eq!(terminal.log_in(PASSPHRASE).await, RESOURCES);
    assert!(!terminal.is_user().await);

    drop(terminal);
    daemon.restart().await;
    host_reachable_becomes(&daemon, false).await;
    let mut terminal = Terminal::open(&daemon);
    assert_eq!(terminal.log_in(PASSPHRASE).await, RESOURCES);
    assert!(!terminal.is_user().await);
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_tries_are_counted_across_a_restart() {
    let mut daemon = away_machine().await;
    let mut terminal = Terminal::open(&daemon);
    for _ in 0..4 {
        assert_eq!(terminal.log_in(WRONG).await, NEEDS_USER);
    }
    drop(terminal);
    daemon.restart().await;
    host_reachable_becomes(&daemon, false).await;
    let mut terminal = Terminal::open(&daemon);
    assert_eq!(terminal.log_in(WRONG).await, RESOURCES);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_machine_whose_host_is_reachable_refuses_the_passphrase() {
    let daemon = machine_daemon().await;
    let _host = Host::with_policy(&daemon, &verifier()).await;
    host_reachable_becomes(&daemon, true).await;
    let mut terminal = Terminal::open(&daemon);
    assert_ne!(terminal.log_in(PASSPHRASE).await, OK);
    assert!(!terminal.is_user().await);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_machine_never_sent_a_passphrase_refuses_the_login() {
    let daemon = away_machine_with("").await;
    let mut terminal = Terminal::open(&daemon);
    assert_ne!(terminal.log_in(PASSPHRASE).await, OK);
    assert!(!terminal.is_user().await);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_login_with_no_prompt_to_answer_needs_the_user() {
    // `--json` never prompts, so with no passphrase given the machine asks
    // for presence and the terminal stays as it was.
    let daemon = away_machine().await;
    let mut terminal = Terminal::open(&daemon);
    assert_eq!(
        terminal
            .exit_code(r#""$HUMATA" --json login > /dev/null 2>&1"#, None)
            .await,
        NEEDS_USER
    );
    assert!(!terminal.is_user().await);
}
