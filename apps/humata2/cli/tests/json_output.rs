//! What `--json` prints (CLI.md §7): one object with `ok`, `operation`,
//! `outcome` and the reply as `result`, and how journaled writes show.

mod support;

use humata2_cli::{exit, rpc};
use humata2_proto::humata::v1 as pb;
use serde_json::json;
use support::{FakeConsole, FakeDaemon, run};

fn stopped() -> pb::Session {
    pb::Session {
        id: support::session("se_1"),
        title: "Fix login".into(),
        status: pb::SessionStatus::Exited as i32,
        ..Default::default()
    }
}

#[tokio::test]
async fn an_applied_change_is_ok_with_the_reply_as_result() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::session::STOP_SESSION, &stopped());
    let console = FakeConsole::pipe();
    assert_eq!(run(&daemon, &console, &["--json", "session", "stop", "se_1"]).await, exit::OK);
    let printed = console.json();
    assert_eq!(printed["ok"], true);
    assert_eq!(printed["outcome"], "applied");
    assert!(printed["operation"].is_string());
    assert_eq!(printed["result"]["id"], "se_1");
    assert_eq!(printed["result"]["title"], "Fix login");
    assert_eq!(printed["result"]["status"], "exited");
    assert_eq!(printed["result"]["board_item"], json!(null));
}

#[tokio::test]
async fn a_person_gets_the_reply_as_text() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::session::STOP_SESSION, &stopped());
    let console = FakeConsole::pipe();
    assert_eq!(run(&daemon, &console, &["session", "stop", "se_1"]).await, exit::OK);
    assert!(console.stdout().contains("Fix login"));
    assert!(serde_json::from_str::<serde_json::Value>(&console.stdout()).is_err());
}

fn journaled() -> pb::WriteItemResponse {
    pb::WriteItemResponse {
        delivery: pb::Delivery::Journaled as i32,
        item: Some(pb::BoardItem {
            title: "Write docs".into(),
            column: pb::BoardColumn::ToDo as i32,
            ..Default::default()
        }),
        journal: Some(pb::JournalPosition {
            machine: support::machine("ma_laptop"),
            seq: 41,
        }),
        provisional_id: Some(pb::BoardItemId {
            value: "provisional-1".into(),
        }),
    }
}

#[tokio::test]
async fn a_journaled_change_is_ok_and_says_where_it_waits() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::board::CREATE_ITEM, &journaled());
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &["--json", "board", "--global", "add", "Write docs", "--column", "to-do"],
    )
    .await;
    assert_eq!(exit, exit::OK);
    let printed = console.json();
    assert_eq!(printed["ok"], true);
    assert_eq!(printed["outcome"], "journaled");
    assert_eq!(printed["journal"], json!({ "machine": "ma_laptop", "seq": 41 }));
    assert_eq!(printed["result"]["item"]["column"], "to_do");
    let sent: pb::CreateItemRequest = daemon.only(rpc::board::CREATE_ITEM);
    assert_eq!(sent.column, pb::BoardColumn::ToDo as i32);
    assert!(matches!(
        sent.board.and_then(|board| board.board),
        Some(pb::board_ref::Board::Global(_))
    ));
}

#[tokio::test]
async fn strict_fails_a_journaled_change_with_20() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::board::CREATE_ITEM, &journaled());
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &["--json", "--strict", "board", "--global", "add", "Write docs"],
    )
    .await;
    assert_eq!(exit, exit::JOURNALED_STRICT);
    let printed = console.json();
    assert_eq!(printed["ok"], false);
    assert_eq!(printed["error"]["kind"], "journaled");
    // The write still happened: the result is there to find it later.
    assert_eq!(printed["result"]["provisional_id"], "provisional-1");
}

#[tokio::test]
async fn strict_fails_a_journaled_change_without_json_too() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::board::CREATE_ITEM, &journaled());
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["--strict", "board", "--global", "add", "Write docs"]).await;
    assert_eq!(exit, exit::JOURNALED_STRICT);
}

#[tokio::test]
async fn strict_lets_an_applied_change_through() {
    let daemon = FakeDaemon::new();
    daemon.reply(
        rpc::board::CREATE_ITEM,
        &pb::WriteItemResponse {
            delivery: pb::Delivery::Applied as i32,
            ..Default::default()
        },
    );
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &["--json", "--strict", "board", "--global", "add", "Write docs"],
    )
    .await;
    assert_eq!(exit, exit::OK);
    assert_eq!(console.json()["outcome"], "applied");
}

#[tokio::test]
async fn a_list_is_every_page_as_one_result() {
    let daemon = FakeDaemon::new();
    let page = |id: &str, next: &str| pb::ListSessionsResponse {
        sessions: vec![pb::Session {
            id: support::session(id),
            ..Default::default()
        }],
        page: Some(pb::PageResponse {
            next_page_token: next.into(),
        }),
    };
    daemon
        .reply(rpc::session::LIST_SESSIONS, &page("se_1", "after-1"))
        .reply(rpc::session::LIST_SESSIONS, &page("se_2", ""));
    let console = FakeConsole::pipe();
    assert_eq!(run(&daemon, &console, &["--json", "ls"]).await, exit::OK);
    let result = &console.json()["result"];
    let ids: Vec<_> = result["sessions"]
        .as_array()
        .expect("sessions")
        .iter()
        .map(|session| session["id"].clone())
        .collect();
    assert_eq!(ids, vec![json!("se_1"), json!("se_2")]);
    assert!(result.get("page").is_none());
    let sent: Vec<pb::ListSessionsRequest> = daemon.sent(rpc::session::LIST_SESSIONS);
    let tokens: Vec<_> = sent
        .iter()
        .map(|request| request.page.clone().unwrap_or_default().page_token)
        .collect();
    assert_eq!(tokens, vec![String::new(), "after-1".to_owned()]);
}

#[tokio::test]
async fn a_failure_prints_kind_message_fix_and_details() {
    let daemon = FakeDaemon::new();
    daemon.refuse_with(
        rpc::session::STOP_SESSION,
        pb::Error {
            message: "The session is busy.".into(),
            fix: "Wait for its turn to end.".into(),
            kind: Some(pb::error::Kind::SessionBusy(Default::default())),
            ..Default::default()
        },
    );
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["--json", "session", "stop", "se_1"]).await;
    assert_eq!(exit, exit::PRECONDITION);
    let error = &console.json()["error"];
    assert_eq!(error["kind"], "session_busy");
    assert_eq!(error["message"], "The session is busy.");
    assert_eq!(error["fix"], "Wait for its turn to end.");
    assert!(error["details"].is_object());
}

#[tokio::test]
async fn retry_advice_says_when_and_whether_with_the_same_key() {
    let mut retry = pb::RetryAdvice {
        same_key: true,
        ..Default::default()
    };
    let mut after = retry.after.take().unwrap_or_default();
    after.seconds = 90;
    retry.after = Some(after);
    let daemon = FakeDaemon::new();
    daemon.refuse_with(
        rpc::session::STOP_SESSION,
        pb::Error {
            kind: Some(pb::error::Kind::RateLimited(Default::default())),
            retry: Some(retry),
            ..Default::default()
        },
    );
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["--json", "session", "stop", "se_1"]).await;
    assert_eq!(exit, exit::RESOURCES);
    let retry = &console.json()["error"]["retry"];
    assert_eq!(retry["same_key"], true);
    assert!(retry["after"].is_string());
}
