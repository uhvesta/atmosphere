//! `humata board add` and `edit` send sessions and Depends on as lists
//! (BD-14, BD-16, D155).

mod support;

use humata2_cli::{exit, rpc};
use humata2_proto::humata::v1 as pb;
use support::{FakeConsole, FakeDaemon, run};

fn ids(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn written() -> pb::WriteItemResponse {
    pb::WriteItemResponse {
        item: Some(pb::BoardItem {
            id: Some(pb::BoardItemId { value: "GLB-1".into() }),
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[tokio::test]
async fn a_read_from_a_machines_copy_keeps_its_age() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::board::LIST_ITEMS, &pb::ListItemsResponse::default());
    daemon.copy_made_at("2026-11-15T09:30:00Z");
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["--json", "board", "--global"]).await;
    assert_eq!(exit, exit::OK);
    assert_eq!(console.json()["stale_as_of"], "2026-11-15T09:30:00Z");
}

#[tokio::test]
async fn a_live_read_carries_no_age() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::board::LIST_ITEMS, &pb::ListItemsResponse::default());
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["--json", "board", "--global"]).await;
    assert_eq!(exit, exit::OK);
    assert!(console.json().get("stale_as_of").is_none());
}

#[tokio::test]
async fn add_sends_every_session_and_dependency_in_order() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::board::CREATE_ITEM, &written());
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &[
            "--json", "board", "--global", "add", "Ship it", "--assign", "se_1", "--assign",
            "se_2", "--depends-on", "PAY-3",
        ],
    )
    .await;
    assert_eq!(exit, exit::OK);
    let sent: pb::CreateItemRequest = daemon.only(rpc::board::CREATE_ITEM);
    let sessions: Vec<String> = sent.sessions.into_iter().map(|id| id.value).collect();
    assert_eq!(sessions, ids(&["se_1", "se_2"]));
    let depends: Vec<String> = sent.depends_on.into_iter().map(|id| id.value).collect();
    assert_eq!(depends, ids(&["PAY-3"]));
}

#[tokio::test]
async fn edit_adds_and_removes_without_replacing() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::board::UPDATE_ITEM, &written());
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &[
            "--json", "board", "edit", "GLB-1", "--assign", "se_3", "--unassign", "se_1",
            "--depends-on", "PAY-4", "--drop-depends-on", "PAY-3",
        ],
    )
    .await;
    assert_eq!(exit, exit::OK);
    let sent: pb::UpdateItemRequest = daemon.only(rpc::board::UPDATE_ITEM);
    let patch = sent.patch.unwrap_or_default();
    let values = |list: Vec<pb::SessionId>| list.into_iter().map(|id| id.value).collect::<Vec<_>>();
    let items = |list: Vec<pb::BoardItemId>| list.into_iter().map(|id| id.value).collect::<Vec<_>>();
    assert_eq!(values(patch.sessions_add), ids(&["se_3"]));
    assert_eq!(values(patch.sessions_remove), ids(&["se_1"]));
    assert_eq!(items(patch.depends_on_add), ids(&["PAY-4"]));
    assert_eq!(items(patch.depends_on_remove), ids(&["PAY-3"]));
    assert!(patch.title.is_none());
}

/// VF-20: a write names the revision the caller read, and the daemon's
/// refusal of a stale one reaches the caller as a conflict.
#[tokio::test]
async fn move_sends_the_revision_it_read_and_a_stale_one_is_refused() {
    let daemon = FakeDaemon::new();
    daemon.refuse(
        rpc::board::MOVE_ITEM,
        pb::error::Kind::RevisionConflict(pb::RevisionConflict::default()),
    );
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &["--json", "board", "move", "GLB-1", "in-progress", "--revision", "3"],
    )
    .await;
    assert_eq!(exit, exit::CONFLICT);
    let sent: pb::MoveItemRequest = daemon.only(rpc::board::MOVE_ITEM);
    assert_eq!(sent.expected_revision, Some(pb::Revision { value: 3 }));
}

#[tokio::test]
async fn edit_and_rm_send_the_revision_they_read() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::board::UPDATE_ITEM, &written());
    daemon.reply(rpc::board::DELETE_ITEM, &pb::WriteItemResponse::default());
    let console = FakeConsole::pipe();
    let edited = run(
        &daemon,
        &console,
        &["--json", "board", "edit", "GLB-1", "--title", "Renamed", "--revision", "5"],
    )
    .await;
    assert_eq!(edited, exit::OK);
    let removed = run(&daemon, &console, &["--json", "board", "rm", "GLB-1", "--revision", "6"]).await;
    assert_eq!(removed, exit::OK);
    let edit: pb::UpdateItemRequest = daemon.only(rpc::board::UPDATE_ITEM);
    assert_eq!(edit.expected_revision, Some(pb::Revision { value: 5 }));
    let rm: pb::DeleteItemRequest = daemon.only(rpc::board::DELETE_ITEM);
    assert_eq!(rm.expected_revision, Some(pb::Revision { value: 6 }));
}

/// VF-21: `board add` names no board and no column unless told to, so the
/// daemon picks the caller's own session board and the To do column.
#[tokio::test]
async fn add_without_a_board_leaves_the_board_and_column_to_the_daemon() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::board::CREATE_ITEM, &written());
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["--json", "board", "add", "Ship it"]).await;
    assert_eq!(exit, exit::OK);
    let sent: pb::CreateItemRequest = daemon.only(rpc::board::CREATE_ITEM);
    assert_eq!(sent.board, None);
    assert_eq!(sent.column, pb::BoardColumn::Unspecified as i32);
}
