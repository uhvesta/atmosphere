//! `humata views` and `humata view …` (CLI.md §5.3): the requests each
//! command sends and what it prints.

mod support;

use humata2_cli::{exit, rpc};
use humata2_proto::humata::v1 as pb;
use support::{FakeConsole, FakeDaemon, run};

fn view(id: &str, title: &str) -> pb::PreparedView {
    pb::PreparedView {
        id: Some(pb::ViewId { value: id.into() }),
        title: title.into(),
        state: pb::ViewState::Waiting as i32,
        ..Default::default()
    }
}

#[tokio::test]
async fn views_with_no_session_asks_for_every_session_and_prints_them() {
    let daemon = FakeDaemon::new();
    daemon.reply(
        rpc::view::LIST_VIEWS,
        &pb::ListViewsResponse {
            views: vec![view("vw_2", "Second"), view("vw_1", "First")],
            page: None,
        },
    );
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["--json", "views"]).await;
    assert_eq!(exit, exit::OK);
    let sent: pb::ListViewsRequest = daemon.only(rpc::view::LIST_VIEWS);
    assert_eq!(sent.session, None);
    let printed = console.json();
    let titles: Vec<_> = printed["result"]["views"]
        .as_array()
        .unwrap()
        .iter()
        .map(|view| view["title"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(titles, vec!["Second", "First"]);
}

#[tokio::test]
async fn views_of_one_session_names_it() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::view::LIST_VIEWS, &pb::ListViewsResponse::default());
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["--json", "views", "--session", "se_1"]).await;
    assert_eq!(exit, exit::OK);
    let sent: pb::ListViewsRequest = daemon.only(rpc::view::LIST_VIEWS);
    assert_eq!(sent.session, support::session("se_1"));
}

#[tokio::test]
async fn each_view_action_sets_its_state_once() {
    for (action, state) in [
        ("open", pb::ViewState::Opened),
        ("later", pb::ViewState::Later),
        ("not-now", pb::ViewState::NotNow),
        ("revert", pb::ViewState::Reverted),
        ("keep", pb::ViewState::Kept),
    ] {
        let daemon = FakeDaemon::new();
        daemon.reply(rpc::view::SET_VIEW_STATE, &view("vw_1", "First"));
        let console = FakeConsole::pipe();
        let exit = run(&daemon, &console, &["--json", "view", action, "vw_1"]).await;
        assert_eq!(exit, exit::OK, "humata view {action}");
        let sent: pb::SetViewStateRequest = daemon.only(rpc::view::SET_VIEW_STATE);
        assert_eq!(sent.id, Some(pb::ViewId { value: "vw_1".into() }));
        assert_eq!(sent.state(), state, "humata view {action}");
        assert!(
            sent.idempotency_key.is_some_and(|key| !key.value.is_empty()),
            "a retry of humata view {action} acts once"
        );
    }
}
