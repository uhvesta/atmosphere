//! The approval passphrase is never taken from a pipe (SEC-10, SEC-13,
//! CLI.md §5.4). A real role-machine daemon whose host is away, and the
//! real `humata` in a terminal of its own: a script that pipes the right
//! passphrase into `humata login` doesn't log the terminal in, and the
//! person who then types it at the prompt still can.

mod common;
#[path = "common/away.rs"]
mod away;

use away::{
    Host, OK, PASSPHRASE, Terminal, host_reachable_becomes, verifier,
};
use common::Build;

#[tokio::test(flavor = "multi_thread")]
async fn the_passphrase_piped_into_login_is_not_read_and_a_typed_one_still_logs_in() {
    let daemon = away::machine_in(Build::AdminForThisProcess, common::folder()).await;
    let host = Host::with_policy(&daemon, &verifier()).await;
    host_reachable_becomes(&daemon, true).await;
    drop(host);
    host_reachable_becomes(&daemon, false).await;

    let mut terminal = Terminal::open(&daemon);
    let piped = format!(r#"printf '%s\n' '{PASSPHRASE}' | "$HUMATA" login > /dev/null 2>&1"#);
    assert_ne!(terminal.exit_code(&piped, None).await, OK);
    assert!(!terminal.is_user().await);

    // No try was counted against the lock: the typed passphrase works.
    assert_eq!(terminal.log_in(PASSPHRASE).await, OK);
    assert!(terminal.is_user().await);
}
