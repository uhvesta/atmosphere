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
