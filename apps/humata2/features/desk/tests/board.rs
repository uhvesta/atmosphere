//! DK-4, BD-7: the lanes a Desk session's Board panel shows.
use std::sync::Arc;

use humata2_clients::model::{
    ChannelId, Item, ItemDetails, ItemId, ItemKind, ItemStatus, Scope, SessionId,
};
use humata2_clients::{Workspace, fixture};
use humata2_desk::{DeskState, Lane};

fn state(workspace: Workspace) -> DeskState {
    DeskState {
        workspace: Arc::new(workspace),
        ..DeskState::default()
    }
}

/// The design workspace with the Desk of frame 36.
fn desk() -> DeskState {
    let mut workspace = fixture::design();
    fixture::desk(&mut workspace);
    state(workspace)
}

fn id(text: &str) -> SessionId {
    SessionId::new(text)
}

fn item(id: &str, scope: Scope, status: ItemStatus) -> Item {
    Item {
        id: ItemId::new(id),
        scope,
        kind: ItemKind::Task,
        title: id.to_owned(),
        status,
        depends_on: Vec::new(),
        session: None,
        parent: None,
        revision: 0,
    }
}

fn ids(lane: &Lane<'_>) -> Vec<String> {
    lane.items
        .iter()
        .map(|entry| entry.item.id.to_string())
        .collect()
}

fn lane<'a>(lanes: &'a [Lane<'a>], status: ItemStatus) -> &'a Lane<'a> {
    match lanes.iter().find(|lane| lane.status == status) {
        Some(lane) => lane,
        None => panic!("no {status:?} lane in {lanes:?}"),
    }
}

#[test]
fn every_boards_items_show_once_each() {
    let state = desk();
    let lanes = state.board(&id("desk-plan-q4"));
    let mut shown: Vec<String> = lanes.iter().flat_map(ids).collect();
    let mut all: Vec<String> = state
        .workspace
        .items
        .iter()
        .map(|item| item.id.to_string())
        .collect();
    shown.sort();
    all.sort();
    assert_eq!(shown, all);
    for channel in ["payments-rewrite", "infra"] {
        let scope = Scope::Channel(ChannelId::new(channel));
        assert!(
            lanes
                .iter()
                .flat_map(|lane| &lane.items)
                .any(|entry| entry.item.scope == scope),
            "{channel}"
        );
    }
}

#[test]
fn global_items_show_beside_the_channels() {
    let state = state(Workspace {
        items: vec![
            item("G-1", Scope::Global, ItemStatus::Todo),
            item(
                "W-1",
                Scope::Channel(ChannelId::new("web")),
                ItemStatus::Todo,
            ),
        ],
        ..Workspace::default()
    });
    let lanes = state.board(&id("desk-one"));
    assert_eq!(ids(lane(&lanes, ItemStatus::Todo)), ["G-1", "W-1"]);
}

#[test]
fn lanes_come_in_the_compact_order_without_the_empty_ones() {
    let web = || Scope::Channel(ChannelId::new("web"));
    let state = state(Workspace {
        items: vec![
            item("W-1", web(), ItemStatus::Done),
            item("W-2", web(), ItemStatus::InProgress),
            item("W-3", web(), ItemStatus::Backlog),
            item("W-4", web(), ItemStatus::InProgress),
        ],
        ..Workspace::default()
    });
    let lanes = state.board(&id("desk-one"));
    let statuses: Vec<ItemStatus> = lanes.iter().map(|lane| lane.status).collect();
    assert_eq!(
        statuses,
        [
            ItemStatus::InProgress,
            ItemStatus::Backlog,
            ItemStatus::Done
        ]
    );
    assert_eq!(ids(lane(&lanes, ItemStatus::InProgress)), ["W-2", "W-4"]);
}

#[test]
fn a_workspace_with_no_items_has_no_lanes() {
    let state = state(Workspace::default());
    assert!(state.board(&id("desk-one")).is_empty());
}

#[test]
fn what_the_session_added_is_marked_and_comes_first_in_its_lane() {
    let web = || Scope::Channel(ChannelId::new("web"));
    let mut workspace = Workspace {
        items: vec![
            item("W-1", web(), ItemStatus::Todo),
            item("W-2", web(), ItemStatus::Todo),
            item("W-3", web(), ItemStatus::Todo),
            item("W-4", web(), ItemStatus::Todo),
        ],
        ..Workspace::default()
    };
    for (key, by) in [("W-2", "desk-one"), ("W-3", "desk-two"), ("W-4", "desk-one")] {
        workspace.item_details.insert(
            ItemId::new(key),
            ItemDetails {
                created_by: Some(id(by)),
                ..ItemDetails::default()
            },
        );
    }
    let state = state(workspace);
    let lanes = state.board(&id("desk-one"));
    let todo = lane(&lanes, ItemStatus::Todo);
    assert_eq!(ids(todo), ["W-2", "W-4", "W-1", "W-3"]);
    let added: Vec<bool> = todo.items.iter().map(|entry| entry.added).collect();
    assert_eq!(added, [true, true, false, false]);
}

#[test]
fn the_plan_session_added_two_payments_items() {
    let state = desk();
    let lanes = state.board(&id("desk-plan-q4"));
    let added: Vec<String> = lanes
        .iter()
        .flat_map(|lane| &lane.items)
        .filter(|entry| entry.added)
        .map(|entry| entry.item.id.to_string())
        .collect();
    assert_eq!(added, ["PAY-13", "PAY-14"]);
    let todo = ids(lane(&lanes, ItemStatus::Todo));
    assert_eq!(todo.first().map(String::as_str), Some("PAY-13"));
}

#[test]
fn another_desk_session_added_none_of_them() {
    let state = desk();
    let lanes = state.board(&id("desk-brin-rfc"));
    assert!(
        lanes
            .iter()
            .flat_map(|lane| &lane.items)
            .all(|entry| !entry.added)
    );
}

#[test]
fn the_desks_lanes_follow_the_compact_order() {
    let state = desk();
    let lanes = state.board(&id("desk-plan-q4"));
    let order = |status: &ItemStatus| ItemStatus::COMPACT.iter().position(|s| s == status);
    let positions: Vec<_> = lanes.iter().map(|lane| order(&lane.status)).collect();
    assert!(positions.iter().all(Option::is_some));
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]), "{positions:?}");
}
