//! `humata channel rename` (D314): a rename moves the channel's folder and
//! stops the sessions that run in it, so a session in a turn is cut off only
//! when the person said so with `--confirm-running`.

mod support;

use humata2_cli::{exit, rpc};
use humata2_proto::humata::v1 as pb;
use support::{FakeConsole, FakeDaemon, run};

fn renamed() -> pb::Channel {
    pb::Channel {
        id: support::channel("ch_1"),
        name: "Billing".into(),
        folder: "billing".into(),
        ..Default::default()
    }
}

#[tokio::test]
async fn confirm_running_is_sent_when_the_flag_is_given() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::channel::RENAME_CHANNEL, &renamed());
    let console = FakeConsole::pipe();

    let exit = run(
        &daemon,
        &console,
        &["--json", "channel", "rename", "ch_1", "Billing", "--confirm-running"],
    )
    .await;

    assert_eq!(exit, exit::OK);
    let sent: pb::RenameChannelRequest = daemon.only(rpc::channel::RENAME_CHANNEL);
    assert_eq!(sent.id, support::channel("ch_1"));
    assert_eq!(sent.name, "Billing");
    assert!(sent.confirm_running);
}

#[tokio::test]
async fn without_the_flag_a_rename_does_not_confirm_running_sessions() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::channel::RENAME_CHANNEL, &renamed());
    let console = FakeConsole::pipe();

    let exit = run(&daemon, &console, &["--json", "channel", "rename", "ch_1", "Billing"]).await;

    assert_eq!(exit, exit::OK);
    let sent: pb::RenameChannelRequest = daemon.only(rpc::channel::RENAME_CHANNEL);
    assert!(!sent.confirm_running);
}

#[tokio::test]
async fn a_rename_refused_for_a_session_in_a_turn_exits_as_a_failed_precondition() {
    let daemon = FakeDaemon::new();
    daemon.refuse(
        rpc::channel::RENAME_CHANNEL,
        pb::error::Kind::SessionBusy(pb::SessionBusy {
            session: support::session("se_1"),
            ..Default::default()
        }),
    );
    let console = FakeConsole::pipe();

    let exit = run(&daemon, &console, &["--json", "channel", "rename", "ch_1", "Billing"]).await;

    assert_eq!(exit, exit::PRECONDITION);
    let printed = console.json();
    assert_eq!(printed["ok"], false);
    assert_eq!(printed["error"]["kind"], "session_busy");
    let sent: pb::RenameChannelRequest = daemon.only(rpc::channel::RENAME_CHANNEL);
    assert!(!sent.confirm_running, "nothing was confirmed on the person's behalf");
}
