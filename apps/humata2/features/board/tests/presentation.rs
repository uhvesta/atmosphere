//! BD-7 and dashboard counts always refer to the visible board scope.
use humata2_board::{Board, BoardAction as A, BoardState, Filter, Mode};
use humata2_clients::{
    Clients, fixture,
    model::{ChannelId, ItemId, ItemStatus as S, Scope},
};
use std::sync::Arc;
use tca::{TestClock, TestStore};
#[test]
fn global_board_includes_channel_items_and_drills_without_leaking_other_items() {
    let workspace = Arc::new(fixture::design());
    let (clients, _) = Clients::fake((*workspace).clone(), Arc::new(TestClock::new()));
    let state = BoardState {
        workspace,
        ..Default::default()
    };
    assert_eq!(state.column(S::Backlog).len(), 2);
    assert_eq!(state.column(S::InProgress).len(), 2);
    let mut store = TestStore::new(state, Board::new(clients));
    store.send(A::Drill(Filter::Waiting), |s| {
        s.view.mode = Mode::Table;
        s.view.filter = Filter::Waiting;
    });
    assert_eq!(
        store
            .state()
            .column(S::InProgress)
            .iter()
            .map(|i| i.id.as_str())
            .collect::<Vec<_>>(),
        vec!["PAY-3"]
    );
    store.send(A::ClearFilter, |s| s.view.filter = Filter::All);
    store.send(A::SetScope(Scope::Channel(ChannelId::new("infra"))), |s| {
        s.scope = Scope::Channel(ChannelId::new("infra"))
    });
    assert_eq!(
        store
            .state()
            .column(S::Backlog)
            .iter()
            .map(|i| i.id.as_str())
            .collect::<Vec<_>>(),
        vec!["INF-4"]
    );
    store.send(A::FilterText("Postgres".into()), |s| {
        s.view.query = "Postgres".into()
    });
    assert!(store.state().column(S::Backlog).is_empty());
    assert_eq!(store.state().column(S::InProgress).len(), 1);
    store.send(A::GlobalOnly(true), |s| {
        s.scope = Scope::Global;
        s.view.global_only = true;
    });
    assert!(store.state().column(S::InProgress).is_empty());
    store.finish();
}
#[test]
fn drawer_rejects_unknown_items_and_new_items_keep_the_requested_column() {
    let workspace = Arc::new(fixture::design());
    let (clients, _) = Clients::fake((*workspace).clone(), Arc::new(TestClock::new()));
    let mut store = TestStore::new(
        BoardState {
            workspace,
            ..Default::default()
        },
        Board::new(clients),
    );
    store.send(A::Open(ItemId::new("missing")), |_| {});
    store.send(A::Open(ItemId::new("PAY-3")), |s| {
        s.drawer = Some(ItemId::new("PAY-3"))
    });
    // Opening reads the item's history (D177).
    store.receive_matching(
        "the item's history",
        |a| matches!(a, A::History(id, Ok(_)) if id == &ItemId::new("PAY-3")),
        |s| s.history = s.workspace.item_details[&ItemId::new("PAY-3")].history.clone(),
    );
    assert_eq!(
        store.state().opened().unwrap().depends_on,
        vec![ItemId::new("PAY-2")]
    );
    store.send(A::Expand, |s| s.expanded = true);
    store.send(A::Close, |s| {
        s.drawer = None;
        s.expanded = false;
        s.history = Vec::new();
    });
    store.send(A::StartNewIn(S::Backlog), |s| {
        s.new_title = Some(String::new());
        s.new_status = S::Backlog;
    });
    store.send(A::NewTitle("   ".into()), |s| {
        s.new_title = Some("   ".into())
    });
    store.send(A::CreateNew, |s| s.new_title = None);
    store.finish();
}

#[test]
fn dashboard_drills_show_exactly_the_counted_items() {
    let workspace = Arc::new(fixture::design());
    let (clients, _) = Clients::fake((*workspace).clone(), Arc::new(TestClock::new()));
    let mut store = TestStore::new(
        BoardState {
            workspace,
            ..Default::default()
        },
        Board::new(clients),
    );
    store.send(A::Drill(Filter::Unassigned), |s| {
        s.view.mode = Mode::Table;
        s.view.filter = Filter::Unassigned;
    });
    let ids: Vec<_> = store
        .state()
        .workspace
        .items
        .iter()
        .filter(|i| store.state().matches(i))
        .map(|i| i.id.as_str())
        .collect();
    assert_eq!(ids, vec!["PAY-12", "INF-4", "PAY-7"]);
    store.send(A::Drill(Filter::DoneDay(10)), |s| {
        s.view.filter = Filter::DoneDay(10)
    });
    assert_eq!(
        store
            .state()
            .column(S::Done)
            .iter()
            .map(|i| i.id.as_str())
            .collect::<Vec<_>>(),
        vec!["PAY-2"]
    );
    store.send(A::Drill(Filter::DoneWeek), |s| {
        s.view.filter = Filter::DoneWeek
    });
    assert_eq!(store.state().column(S::Done).len(), 2);
    store.finish();
}
