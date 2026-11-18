//! `humata shell --session`: a shell tab inside a session, not a machine's.

mod support;

use humata2_cli::{exit, rpc};
use humata2_proto::humata::v1 as pb;
use support::{FakeConsole, FakeDaemon, run};

#[tokio::test]
async fn shell_with_a_session_opens_a_tab_in_that_session() {
    let daemon = FakeDaemon::new();
    daemon.reply(
        rpc::terminal::OPEN_SHELL_TAB,
        &pb::Tab {
            id: Some(pb::TabId { value: "tb_7".into() }),
            session: support::session("se_1"),
            ..Default::default()
        },
    );
    let console = FakeConsole::pipe();
    let code = run(&daemon, &console, &["--json", "shell", "--session", "se_1"]).await;
    assert_eq!(code, exit::OK, "{}", console.stderr());
    let sent: pb::OpenShellTabRequest = daemon.only(rpc::terminal::OPEN_SHELL_TAB);
    assert_eq!(sent.session, support::session("se_1"));
    assert_eq!(console.json()["result"]["id"], "tb_7");
    assert!(!daemon.called(rpc::terminal::OPEN_MACHINE_SHELL));
}

#[tokio::test]
async fn shell_with_a_machine_still_opens_a_machine_shell() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::terminal::OPEN_MACHINE_SHELL, &pb::Tab::default());
    let console = FakeConsole::pipe();
    let code = run(&daemon, &console, &["--json", "shell", "ma_1"]).await;
    assert_eq!(code, exit::OK, "{}", console.stderr());
    assert!(!daemon.called(rpc::terminal::OPEN_SHELL_TAB));
}

#[tokio::test]
async fn shell_with_neither_or_both_is_a_usage_error() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    for words in [&["shell"][..], &["shell", "ma_1", "--session", "se_1"][..]] {
        assert_eq!(run(&daemon, &console, words).await, exit::USAGE, "{words:?}");
    }
    assert!(!daemon.called(rpc::terminal::OPEN_SHELL_TAB));
    assert!(!daemon.called(rpc::terminal::OPEN_MACHINE_SHELL));
}
