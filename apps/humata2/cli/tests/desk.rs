//! `humata desk keep [<session>]` (DK-2, D182.6): keeps a Desk session's
//! scratch folder past its close. With no session the daemon takes the
//! caller's own. Parked until `desk` is registered; moves up with it.

mod support;

use humata2_cli::{exit, rpc};
use humata2_proto::humata::v1 as pb;
use support::{FakeConsole, FakeDaemon, run};

fn kept(session: &str) -> pb::DeskSession {
    pb::DeskSession {
        session: Some(pb::SessionId {
            value: session.to_owned(),
        }),
        folder: Some(pb::DeskFolder {
            path: "/h/desk/plan-q4".to_owned(),
            kind: Some(pb::desk_folder::Kind::Scratch(pb::desk_folder::Scratch { kept: true })),
        }),
        ..Default::default()
    }
}

#[tokio::test]
async fn a_named_session_is_kept() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::desk::KEEP_DESK_FOLDER, &kept("se_1"));
    let console = FakeConsole::pipe();
    assert_eq!(run(&daemon, &console, &["desk", "keep", "se_1"]).await, exit::OK);
    let sent: pb::KeepDeskFolderRequest = daemon.only(rpc::desk::KEEP_DESK_FOLDER);
    assert_eq!(sent.session.map(|id| id.value), Some("se_1".to_owned()));
}

#[tokio::test]
async fn with_no_session_the_daemon_takes_the_callers_own() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::desk::KEEP_DESK_FOLDER, &kept("se_mine"));
    let console = FakeConsole::pipe();
    assert_eq!(run(&daemon, &console, &["desk", "keep"]).await, exit::OK);
    let sent: pb::KeepDeskFolderRequest = daemon.only(rpc::desk::KEEP_DESK_FOLDER);
    assert!(sent.session.is_none());
}

#[tokio::test]
async fn a_refusal_fails_the_command() {
    let daemon = FakeDaemon::new();
    daemon.refuse(
        rpc::desk::KEEP_DESK_FOLDER,
        pb::error::Kind::Denied(Default::default()),
    );
    let console = FakeConsole::pipe();
    assert_ne!(run(&daemon, &console, &["desk", "keep", "se_other"]).await, exit::OK);
}
