//! BoardService through `humatad2`'s gRPC API: item keys, sessions and
//! dependencies on items, revisions, ordering, history, and an agent's
//! write on a board it needs approval for (BD-1..BD-16, SL-6..SL-8, D155).
//! Move to `tests/boards.rs` once BoardService is wired.

mod common;

use std::path::Path;
use std::time::{Duration, Instant};

use common::{Attach, Daemon, WAIT, error_of, folder, invalid_field, not_found, open, state_folder};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{ChannelRecord, SessionRecord, Store};
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;
use tonic::{Code, Request, Status};

const PAYMENTS: &str = "ch_payments";
const PAYROLL: &str = "ch_payroll";
/// The session whose tab's agent acts.
const AGENT: &str = "se_agent";
/// Another session in the same channel, with no tab.
const OTHER: &str = "se_other";
const TOKEN_HEADER: &str = "humata-session-token";

struct World {
    daemon: Daemon,
    token: String,
    _attach: Attach,
}

impl World {
    async fn start() -> Self {
        let dir = folder();
        let store = Store::open(state_folder(dir.path()).join("store")).unwrap();
        for (id, name) in [(PAYMENTS, "Payments"), (PAYROLL, "Payroll")] {
            store
                .put_channel(&ChannelRecord {
                    id: id.to_owned(),
                    name: name.to_owned(),
                    folder: id.to_owned(),
                    branch_prefix: String::new(),
                    default_agent: "zsh".to_owned(),
                    repos: Vec::new(),
                    machines: Vec::new(),
                    revision: 1,
                    created_unix_ms: 0,
                })
                .unwrap();
        }
        for id in [AGENT, OTHER] {
            store
                .put_session(&SessionRecord {
                    id: id.to_owned(),
                    channel: PAYMENTS.to_owned(),
                    machine: String::new(),
                    title: id.to_owned(),
                    agent: "zsh".to_owned(),
                    folder: dir.path().to_owned(),
                    created_unix_ms: 0,
                })
                .unwrap();
        }
        drop(store);
        let daemon = Daemon::start_in(dir).await;
        let tab = daemon
            .terminals()
            .await
            .open_shell_tab(pb::OpenShellTabRequest {
                idempotency_key: None,
                session: Some(session(AGENT)),
                repo: None,
            })
            .await
            .unwrap()
            .into_inner()
            .id
            .unwrap();
        let mut attach = Attach::start(&daemon, open(&tab, 0)).await;
        let file = daemon.dir.path().join("token");
        attach
            .type_text(&format!(
                "printf '%s' \"$HUMATA_SESSION_TOKEN\" > {}\r",
                file.display()
            ))
            .await;
        let token = read_when_written(&file).await;
        Self {
            daemon,
            token,
            _attach: attach,
        }
    }

    async fn boards(&self) -> pb::board_service_client::BoardServiceClient<Channel> {
        pb::board_service_client::BoardServiceClient::new(self.daemon.channel().await)
    }

    async fn inbox(&self) -> pb::inbox_service_client::InboxServiceClient<Channel> {
        pb::inbox_service_client::InboxServiceClient::new(self.daemon.channel().await)
    }

    fn as_agent<T>(&self, message: T) -> Request<T> {
        let mut request = Request::new(message);
        request.metadata_mut().insert(
            TOKEN_HEADER,
            MetadataValue::try_from(self.token.as_str()).unwrap(),
        );
        request
    }

    /// The user makes an item.
    async fn make(&self, board: pb::BoardRef, title: &str, column: pb::BoardColumn) -> pb::BoardItem {
        self.try_make(board, title, column)
            .await
            .unwrap()
            .item
            .unwrap()
    }

    async fn try_make(
        &self,
        board: pb::BoardRef,
        title: &str,
        column: pb::BoardColumn,
    ) -> Result<pb::WriteItemResponse, Status> {
        self.boards()
            .await
            .create_item(pb::CreateItemRequest {
                board: Some(board),
                title: title.to_owned(),
                column: column as i32,
                ..Default::default()
            })
            .await
            .map(tonic::Response::into_inner)
    }

    async fn get(&self, id: &str) -> Result<pb::BoardItem, Status> {
        self.boards()
            .await
            .get_item(pb::GetItemRequest {
                id: Some(item(id)),
            })
            .await
            .map(tonic::Response::into_inner)
    }

    async fn list(&self, board: pb::BoardRef) -> pb::ListItemsResponse {
        self.boards()
            .await
            .list_items(pb::ListItemsRequest {
                view: Some(pb::BoardView {
                    view: Some(pb::board_view::View::Board(board)),
                }),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
    }

    async fn update(&self, id: &str, patch: pb::ItemPatch, expected: Option<u64>) -> Result<pb::BoardItem, Status> {
        self.boards()
            .await
            .update_item(pb::UpdateItemRequest {
                id: Some(item(id)),
                patch: Some(patch),
                expected_revision: expected.map(|value| pb::Revision { value }),
                ..Default::default()
            })
            .await
            .map(|done| done.into_inner().item.unwrap())
    }

    async fn move_to(&self, id: &str, column: pb::BoardColumn, after: Option<&str>) -> Result<pb::BoardItem, Status> {
        self.boards()
            .await
            .move_item(pb::MoveItemRequest {
                id: Some(item(id)),
                column: column as i32,
                after: after.map(item),
                ..Default::default()
            })
            .await
            .map(|done| done.into_inner().item.unwrap())
    }

    async fn delete(&self, id: &str) -> Result<pb::WriteItemResponse, Status> {
        self.boards()
            .await
            .delete_item(pb::DeleteItemRequest {
                id: Some(item(id)),
                ..Default::default()
            })
            .await
            .map(tonic::Response::into_inner)
    }

    /// The user approves request `id`.
    async fn approve(&self, id: &pb::RequestId) {
        self.inbox()
            .await
            .answer_request(pb::AnswerRequestRequest {
                id: Some(id.clone()),
                answer: Some(pb::RequestAnswer {
                    answer: Some(pb::request_answer::Answer::Approve(pb::Approve {})),
                }),
                ..Default::default()
            })
            .await
            .unwrap();
    }

    async fn request(&self, id: &pb::RequestId) -> pb::InboxRequest {
        self.inbox()
            .await
            .get_request(pb::GetRequestRequest {
                id: Some(id.clone()),
            })
            .await
            .unwrap()
            .into_inner()
    }

    /// The request an agent's call is waiting on.
    fn waiting_on(status: &Status) -> pb::RequestId {
        assert_eq!(status.code(), Code::FailedPrecondition, "{status:?}");
        match error_of(status).kind {
            Some(pb::error::Kind::ApprovalPending(pending)) => pending.request.unwrap(),
            other => panic!("expected approval_pending, got {other:?}"),
        }
    }
}

fn session(id: &str) -> pb::SessionId {
    pb::SessionId {
        value: id.to_owned(),
    }
}

fn item(id: &str) -> pb::BoardItemId {
    pb::BoardItemId {
        value: id.to_owned(),
    }
}

fn global() -> pb::BoardRef {
    pb::BoardRef {
        board: Some(pb::board_ref::Board::Global(pb::GlobalBoard {})),
    }
}

fn channel(id: &str) -> pb::BoardRef {
    pb::BoardRef {
        board: Some(pb::board_ref::Board::Channel(pb::ChannelId {
            value: id.to_owned(),
        })),
    }
}

fn id_of(item: &pb::BoardItem) -> String {
    item.id.clone().unwrap().value
}

fn ids(items: &[pb::BoardItem]) -> Vec<String> {
    items.iter().map(id_of).collect()
}

fn depends(id: &str) -> pb::ItemPatch {
    pb::ItemPatch {
        depends_on_add: vec![item(id)],
        ..Default::default()
    }
}

/// `path`'s contents once something non-empty is written there.
async fn read_when_written(path: &Path) -> String {
    let deadline = Instant::now() + WAIT;
    loop {
        match std::fs::read_to_string(path) {
            Ok(text) if !text.is_empty() => return text,
            _ => {}
        }
        assert!(Instant::now() < deadline, "{} was never written", path.display());
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Waits until `check` holds of the list of `board`.
async fn until_listed(world: &World, board: pb::BoardRef, check: impl Fn(&[pb::BoardItem]) -> bool) -> Vec<pb::BoardItem> {
    let deadline = Instant::now() + WAIT;
    loop {
        let items = world.list(board.clone()).await.items;
        if check(&items) {
            return items;
        }
        assert!(Instant::now() < deadline, "the board never showed it");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn keys_count_up_per_channel_and_a_taken_prefix_lengthens() {
    let world = World::start().await;
    let first = world.make(channel(PAYMENTS), "one", pb::BoardColumn::ToDo).await;
    let second = world.make(channel(PAYMENTS), "two", pb::BoardColumn::ToDo).await;
    let other = world.make(channel(PAYROLL), "three", pb::BoardColumn::ToDo).await;
    assert_eq!(id_of(&first), "PAY-1");
    assert_eq!(id_of(&second), "PAY-2");
    // Payroll's three letters are Payments's, so it takes a fourth.
    assert_eq!(id_of(&other), "PAYR-1");
}

#[tokio::test]
async fn the_global_board_numbers_under_glb() {
    let world = World::start().await;
    let made = world.make(global(), "global", pb::BoardColumn::Backlog).await;
    assert_eq!(id_of(&made), "GLB-1");
    assert!(made.channel.is_none());
}

#[tokio::test]
async fn a_session_item_uses_its_channels_prefix() {
    let world = World::start().await;
    let board = pb::BoardRef {
        board: Some(pb::board_ref::Board::Session(session(OTHER))),
    };
    let made = world.make(board, "mine", pb::BoardColumn::ToDo).await;
    assert!(id_of(&made).starts_with("PAY-"));
    assert_eq!(made.channel.unwrap().value, PAYMENTS);
}

#[tokio::test]
async fn sessions_are_added_and_removed_and_unknown_ones_refused() {
    let world = World::start().await;
    let made = world.make(channel(PAYMENTS), "work", pb::BoardColumn::ToDo).await;
    let id = id_of(&made);
    let both = pb::ItemPatch {
        sessions_add: vec![session(AGENT), session(OTHER)],
        ..Default::default()
    };
    let changed = world.update(&id, both, None).await.unwrap();
    assert_eq!(changed.sessions, vec![session(AGENT), session(OTHER)]);
    let one_less = pb::ItemPatch {
        sessions_remove: vec![session(AGENT)],
        ..Default::default()
    };
    let changed = world.update(&id, one_less, None).await.unwrap();
    assert_eq!(changed.sessions, vec![session(OTHER)]);

    let unknown = pb::ItemPatch {
        sessions_add: vec![session("se_nobody")],
        ..Default::default()
    };
    let refused = world.update(&id, unknown, None).await.unwrap_err();
    assert_eq!(invalid_field(&refused).1, pb::invalid_field::Problem::UnknownValue);
}

#[tokio::test]
#[allow(deprecated)]
async fn the_replaced_assignment_fields_are_refused() {
    let world = World::start().await;
    let made = world.make(channel(PAYMENTS), "work", pb::BoardColumn::ToDo).await;
    let patch = pb::ItemPatch {
        assigned_session: Some(session(AGENT)),
        ..Default::default()
    };
    let refused = world.update(&id_of(&made), patch, None).await.unwrap_err();
    assert_eq!(invalid_field(&refused).0, "assigned_session");
    let refused = world
        .boards()
        .await
        .create_item(pb::CreateItemRequest {
            board: Some(channel(PAYMENTS)),
            title: "x".to_owned(),
            assigned_session: Some(session(AGENT)),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert_eq!(invalid_field(&refused).0, "assigned_session");
}

#[tokio::test]
async fn a_dependency_that_closes_a_loop_is_refused_with_the_loop() {
    let world = World::start().await;
    let a = id_of(&world.make(channel(PAYMENTS), "a", pb::BoardColumn::ToDo).await);
    let b = id_of(&world.make(global(), "b", pb::BoardColumn::ToDo).await);
    let c = id_of(&world.make(channel(PAYROLL), "c", pb::BoardColumn::ToDo).await);
    world.update(&a, depends(&b), None).await.unwrap();
    world.update(&b, depends(&c), None).await.unwrap();
    let refused = world.update(&c, depends(&a), None).await.unwrap_err();
    assert_eq!(refused.code(), Code::FailedPrecondition);
    match error_of(&refused).kind {
        Some(pb::error::Kind::DependencyLoop(found)) => {
            let around: Vec<String> = found.cycle.iter().map(|id| id.value.clone()).collect();
            assert_eq!(around.first(), Some(&c));
            assert_eq!(around.last(), Some(&c));
            assert!(around.contains(&a) && around.contains(&b));
        }
        other => panic!("expected dependency_loop, got {other:?}"),
    }
    // Nothing was kept of the refused edge.
    assert!(world.get(&c).await.unwrap().depends_on.is_empty());
}

#[tokio::test]
async fn an_item_depending_on_itself_is_a_loop_of_one() {
    let world = World::start().await;
    let a = id_of(&world.make(channel(PAYMENTS), "a", pb::BoardColumn::ToDo).await);
    let refused = world.update(&a, depends(&a), None).await.unwrap_err();
    assert!(matches!(
        error_of(&refused).kind,
        Some(pb::error::Kind::DependencyLoop(_))
    ));
}

#[tokio::test]
async fn a_dependency_on_a_missing_item_is_not_found() {
    let world = World::start().await;
    let a = id_of(&world.make(channel(PAYMENTS), "a", pb::BoardColumn::ToDo).await);
    let refused = world.update(&a, depends("NOPE-9"), None).await.unwrap_err();
    assert_eq!(not_found(&refused), pb::EntityKind::BoardItem);
}

#[tokio::test]
async fn a_dependency_shows_its_title_and_column() {
    let world = World::start().await;
    let a = id_of(&world.make(channel(PAYMENTS), "a", pb::BoardColumn::ToDo).await);
    let b = world.make(channel(PAYMENTS), "the blocker", pb::BoardColumn::Done).await;
    let changed = world.update(&a, depends(&id_of(&b)), None).await.unwrap();
    assert_eq!(changed.depends_on.len(), 1);
    assert_eq!(changed.depends_on[0].title, "the blocker");
    assert_eq!(changed.depends_on[0].column(), pb::BoardColumn::Done);
}

#[tokio::test]
async fn a_change_made_against_an_old_revision_conflicts_only_on_fields_changed_since() {
    let world = World::start().await;
    let made = world.make(channel(PAYMENTS), "first", pb::BoardColumn::ToDo).await;
    let id = id_of(&made);
    let revision = made.revision.unwrap().value;
    let renamed = pb::ItemPatch {
        title: Some("second".to_owned()),
        ..Default::default()
    };
    world.update(&id, renamed, None).await.unwrap();

    // The description wasn't touched since: it still lands.
    let described = pb::ItemPatch {
        description: Some("words".to_owned()),
        ..Default::default()
    };
    assert!(world.update(&id, described, Some(revision)).await.is_ok());

    let again = pb::ItemPatch {
        title: Some("third".to_owned()),
        ..Default::default()
    };
    let refused = world.update(&id, again, Some(revision)).await.unwrap_err();
    match error_of(&refused).kind {
        Some(pb::error::Kind::RevisionConflict(conflict)) => {
            assert_eq!(conflict.id, id);
            assert_eq!(conflict.expected.unwrap().value, revision);
            assert!(conflict.actual.unwrap().value > revision);
        }
        other => panic!("expected revision_conflict, got {other:?}"),
    }
    assert_eq!(world.get(&id).await.unwrap().title, "second");
}

#[tokio::test]
async fn a_write_with_no_revision_never_conflicts() {
    let world = World::start().await;
    let id = id_of(&world.make(channel(PAYMENTS), "a", pb::BoardColumn::ToDo).await);
    for title in ["b", "c", "d"] {
        let patch = pb::ItemPatch {
            title: Some(title.to_owned()),
            ..Default::default()
        };
        world.update(&id, patch, None).await.unwrap();
    }
    assert_eq!(world.get(&id).await.unwrap().title, "d");
}

#[tokio::test]
async fn deleting_an_item_drops_the_edges_to_it() {
    let world = World::start().await;
    let a = id_of(&world.make(channel(PAYMENTS), "a", pb::BoardColumn::ToDo).await);
    let b = id_of(&world.make(channel(PAYMENTS), "b", pb::BoardColumn::ToDo).await);
    world.update(&a, depends(&b), None).await.unwrap();
    world.delete(&b).await.unwrap();
    assert!(world.get(&a).await.unwrap().depends_on.is_empty());
    assert_eq!(not_found(&world.get(&b).await.unwrap_err()), pb::EntityKind::BoardItem);
    assert_eq!(
        ids(&world.list(channel(PAYMENTS)).await.items),
        vec![a]
    );
}

#[tokio::test]
async fn done_at_is_set_in_done_and_cleared_out_of_it() {
    let world = World::start().await;
    let made = world.make(channel(PAYMENTS), "a", pb::BoardColumn::ToDo).await;
    assert!(made.done_at.is_none());
    let id = id_of(&made);
    let done = world.move_to(&id, pb::BoardColumn::Done, None).await.unwrap();
    assert!(done.done_at.is_some());
    let back = world.move_to(&id, pb::BoardColumn::ToDo, None).await.unwrap();
    assert!(back.done_at.is_none());
}

#[tokio::test]
async fn new_items_go_to_the_top_and_a_move_goes_after_the_item_named() {
    let world = World::start().await;
    let a = id_of(&world.make(channel(PAYMENTS), "a", pb::BoardColumn::ToDo).await);
    let b = id_of(&world.make(channel(PAYMENTS), "b", pb::BoardColumn::ToDo).await);
    let c = id_of(&world.make(channel(PAYMENTS), "c", pb::BoardColumn::ToDo).await);
    let column = |items: Vec<pb::BoardItem>| -> Vec<String> {
        ids(&items.into_iter().filter(|i| i.column() == pb::BoardColumn::ToDo).collect::<Vec<_>>())
    };
    assert_eq!(column(world.list(channel(PAYMENTS)).await.items), vec![c.clone(), b.clone(), a.clone()]);

    // c moves between b and a.
    world.move_to(&c, pb::BoardColumn::ToDo, Some(&b)).await.unwrap();
    assert_eq!(column(world.list(channel(PAYMENTS)).await.items), vec![b.clone(), c.clone(), a.clone()]);
    // After the last one.
    world.move_to(&b, pb::BoardColumn::ToDo, Some(&a)).await.unwrap();
    assert_eq!(column(world.list(channel(PAYMENTS)).await.items), vec![c.clone(), a.clone(), b.clone()]);
    // No `after`: the top.
    world.move_to(&b, pb::BoardColumn::ToDo, None).await.unwrap();
    assert_eq!(column(world.list(channel(PAYMENTS)).await.items), vec![b, c, a]);
}

#[tokio::test]
async fn an_after_that_is_not_in_the_column_puts_the_item_on_top() {
    let world = World::start().await;
    let a = id_of(&world.make(channel(PAYMENTS), "a", pb::BoardColumn::ToDo).await);
    let b = id_of(&world.make(channel(PAYMENTS), "b", pb::BoardColumn::Backlog).await);
    let moved = world
        .move_to(&b, pb::BoardColumn::ToDo, Some("PAY-99"))
        .await
        .unwrap();
    assert_eq!(moved.column(), pb::BoardColumn::ToDo);
    let items = world.list(channel(PAYMENTS)).await.items;
    assert_eq!(ids(&items), vec![b, a]);
}

#[tokio::test]
async fn a_list_counts_every_column_and_filters_by_the_columns_asked() {
    let world = World::start().await;
    world.make(channel(PAYMENTS), "a", pb::BoardColumn::ToDo).await;
    world.make(channel(PAYMENTS), "b", pb::BoardColumn::ToDo).await;
    world.make(channel(PAYMENTS), "c", pb::BoardColumn::Done).await;
    let listed = world
        .boards()
        .await
        .list_items(pb::ListItemsRequest {
            view: Some(pb::BoardView {
                view: Some(pb::board_view::View::ChannelWithSessions(pb::ChannelId {
                    value: PAYMENTS.to_owned(),
                })),
            }),
            columns: vec![pb::BoardColumn::Done as i32],
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(listed.items.len(), 1);
    let count = |column: pb::BoardColumn| {
        listed
            .counts
            .iter()
            .find(|count| count.column() == column)
            .map_or(0, |count| count.items)
    };
    assert_eq!(count(pb::BoardColumn::ToDo), 2);
    assert_eq!(count(pb::BoardColumn::Done), 1);
    assert_eq!(count(pb::BoardColumn::Backlog), 0);
}

#[tokio::test]
async fn a_list_is_paged() {
    let world = World::start().await;
    for title in ["a", "b", "c"] {
        world.make(channel(PAYMENTS), title, pb::BoardColumn::ToDo).await;
    }
    let mut seen = Vec::new();
    let mut token = String::new();
    loop {
        let page = world
            .boards()
            .await
            .list_items(pb::ListItemsRequest {
                view: Some(pb::BoardView {
                    view: Some(pb::board_view::View::Board(channel(PAYMENTS))),
                }),
                page: Some(pb::PageRequest {
                    page_size: 2,
                    page_token: token.clone(),
                }),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner();
        assert!(page.items.len() <= 2);
        seen.extend(ids(&page.items));
        token = page.page.unwrap().next_page_token;
        if token.is_empty() {
            break;
        }
    }
    assert_eq!(seen.len(), 3);
}

#[tokio::test]
async fn an_agents_write_on_the_global_board_waits_in_the_inbox_until_approved() {
    let world = World::start().await;
    let asked = world
        .boards()
        .await
        .create_item(world.as_agent(pb::CreateItemRequest {
            board: Some(global()),
            title: "from the agent".to_owned(),
            column: pb::BoardColumn::ToDo as i32,
            reason: "it is needed".to_owned(),
            ..Default::default()
        }))
        .await
        .unwrap_err();
    let request = World::waiting_on(&asked);
    assert!(world.list(global()).await.items.is_empty());

    let card = world.request(&request).await;
    assert!(matches!(
        card.preview.and_then(|preview| preview.preview),
        Some(pb::request_preview::Preview::NewBoardItem(_))
    ));

    world.approve(&request).await;
    let items = until_listed(&world, global(), |items| items.len() == 1).await;
    assert_eq!(items[0].title, "from the agent");
}

#[tokio::test]
async fn an_agents_move_waits_and_applies_on_approval_and_only_the_user_sees_it_pending() {
    let world = World::start().await;
    let made = id_of(&world.make(global(), "task", pb::BoardColumn::ToDo).await);
    let asked = world
        .boards()
        .await
        .move_item(world.as_agent(pb::MoveItemRequest {
            id: Some(item(&made)),
            column: pb::BoardColumn::InReview as i32,
            reason: "ready".to_owned(),
            ..Default::default()
        }))
        .await
        .unwrap_err();
    let request = World::waiting_on(&asked);

    // Still where it was, with the request on it for the user only.
    let seen_by_user = world.get(&made).await.unwrap();
    assert_eq!(seen_by_user.column(), pb::BoardColumn::ToDo);
    assert_eq!(seen_by_user.pending.len(), 1);
    assert_eq!(seen_by_user.pending[0].request.clone().unwrap(), request);
    assert_eq!(seen_by_user.pending[0].session.clone().unwrap(), session(AGENT));
    let change = seen_by_user.pending[0].change.clone().unwrap();
    assert_eq!(change.changes.len(), 1);
    let seen_by_agent = world
        .boards()
        .await
        .get_item(world.as_agent(pb::GetItemRequest {
            id: Some(item(&made)),
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(seen_by_agent.pending.is_empty());

    world.approve(&request).await;
    let deadline = Instant::now() + WAIT;
    loop {
        let now = world.get(&made).await.unwrap();
        if now.column() == pb::BoardColumn::InReview {
            assert!(now.pending.is_empty());
            break;
        }
        assert!(Instant::now() < deadline, "the move was never applied");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn deleting_an_item_withdraws_the_requests_about_it() {
    let world = World::start().await;
    let made = id_of(&world.make(global(), "task", pb::BoardColumn::ToDo).await);
    let asked = world
        .boards()
        .await
        .update_item(world.as_agent(pb::UpdateItemRequest {
            id: Some(item(&made)),
            patch: Some(pb::ItemPatch {
                title: Some("renamed".to_owned()),
                ..Default::default()
            }),
            ..Default::default()
        }))
        .await
        .unwrap_err();
    let request = World::waiting_on(&asked);
    world.delete(&made).await.unwrap();
    let card = world.request(&request).await;
    assert_eq!(card.state(), pb::RequestState::Withdrawn);
    assert_eq!(card.withdraw_reason(), pb::WithdrawReason::ItemDeleted);
}

#[tokio::test]
async fn a_repeated_create_with_the_same_key_makes_one_item() {
    let world = World::start().await;
    let request = pb::CreateItemRequest {
        idempotency_key: Some(pb::IdempotencyKey {
            value: "once".to_owned(),
        }),
        board: Some(channel(PAYMENTS)),
        title: "a".to_owned(),
        column: pb::BoardColumn::ToDo as i32,
        ..Default::default()
    };
    let first = world.boards().await.create_item(request.clone()).await.unwrap().into_inner();
    let second = world.boards().await.create_item(request).await.unwrap().into_inner();
    assert_eq!(first.item.unwrap().id, second.item.unwrap().id);
    assert_eq!(world.list(channel(PAYMENTS)).await.items.len(), 1);
}

#[tokio::test]
async fn history_records_what_happened_in_order_with_its_kind() {
    let world = World::start().await;
    let id = id_of(&world.make(channel(PAYMENTS), "a", pb::BoardColumn::ToDo).await);
    world
        .update(
            &id,
            pb::ItemPatch {
                title: Some("b".to_owned()),
                ..Default::default()
            },
            None,
        )
        .await
        .unwrap();
    world.move_to(&id, pb::BoardColumn::InReview, None).await.unwrap();
    world
        .update(
            &id,
            pb::ItemPatch {
                sessions_add: vec![session(OTHER)],
                ..Default::default()
            },
            None,
        )
        .await
        .unwrap();
    world.delete(&id).await.unwrap();
    let history = world
        .boards()
        .await
        .get_item_history(pb::GetItemHistoryRequest {
            id: Some(item(&id)),
            page: None,
        })
        .await
        .unwrap()
        .into_inner();
    let kinds: Vec<pb::HistoryKind> = history.entries.iter().map(|entry| entry.kind()).collect();
    assert_eq!(kinds.first(), Some(&pb::HistoryKind::Created));
    assert_eq!(kinds.last(), Some(&pb::HistoryKind::Deleted));
    for expected in [
        pb::HistoryKind::Changed,
        pb::HistoryKind::Moved,
        pb::HistoryKind::Sessions,
    ] {
        assert!(kinds.contains(&expected), "{expected:?} in {kinds:?}");
    }
    assert!(history.entries.iter().all(|entry| !entry.lost_conflict));
    assert!(history.entries.iter().all(|entry| entry.at.is_some()));
}
