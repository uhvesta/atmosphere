//! BoardService (boards.proto): board items (BD-1..BD-16, SL-6..SL-8, D155)
//! as a service over the store's `boards`. An agent's write the rules send
//! to Ask me waits in the Inbox with its change on the card and is done
//! when the user approves it (BD-15); everything applied goes on the feed.
//!
//! The store's calls block, so the work runs on blocking threads. Writes
//! take one lock, so the lists an edit adds to and takes from (sessions and
//! dependencies) are read and saved without another write in between.

mod asks;
mod history;
mod keys;
mod rank;
mod scope;

use std::sync::{Arc, Mutex};

use humata2_access::{AccessRequest, BoardAct, BoardRef, Call, Refusal};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{
    BoardItemChange, BoardItemRecord, BoardKey, BoardWrite, NewBoardItem, Store, StoreError,
};
use pb::invalid_field::Problem;
use prost::Message;
use tonic::{Code, Request, Response, Status};

use self::asks::{change_ask, field_change, new_item_ask, pending_for};
use self::scope::{
    board_for_policy, board_pb, by_of, column_label, column_number, column_pb, made_by_session,
};
use crate::channel_service::{gate, timestamp_ms};
use crate::events::{Feed, Scope};
use crate::guard::{Checked, Guard};
use crate::identity::{self, AgentCaller, Caller};
use crate::inbox::{BoardAsk, Deferred, Inbox, Run};
use crate::status;
use crate::trash_service::{Answers, answer_key, blocking, lock, now_ms};

/// Items per page when the caller names no size.
const PAGE_DEFAULT: usize = 200;

/// The most items in one page.
const PAGE_MAX: usize = 1000;

/// The column an item made without one starts in: Backlog.
const FIRST_COLUMN: i32 = 1;

/// The Done column.
const DONE: i32 = 5;

#[derive(Clone)]
pub(crate) struct BoardSvc {
    guard: Arc<Guard>,
    store: Arc<Store>,
    feed: Arc<Feed>,
    /// Where an agent's write a rule sends to Ask me waits (BD-15).
    inbox: Arc<Inbox>,
    machine: String,
    writes: Arc<Mutex<()>>,
    finished: Arc<Answers<pb::WriteItemResponse>>,
}

/// A write, as asked.
#[derive(Clone)]
enum Op {
    Create(pb::CreateItemRequest),
    Update(pb::UpdateItemRequest),
    Move(pb::MoveItemRequest),
    Delete(pb::DeleteItemRequest),
}

impl Op {
    /// The call's name, the item it is about and its idempotency key.
    fn identity(&self) -> (&'static str, String, Option<String>) {
        let key = |key: &Option<pb::IdempotencyKey>| key.as_ref().map(|key| key.value.clone());
        let id = |id: &Option<pb::BoardItemId>| id.as_ref().map(|id| id.value.clone()).unwrap_or_default();
        match self {
            Self::Create(r) => ("create_item", String::new(), key(&r.idempotency_key)),
            Self::Update(r) => ("update_item", id(&r.id), key(&r.idempotency_key)),
            Self::Move(r) => ("move_item", id(&r.id), key(&r.idempotency_key)),
            Self::Delete(r) => ("delete_item", id(&r.id), key(&r.idempotency_key)),
        }
    }
}

/// What a write came to: done, or stopped at a rule that asks.
enum Applied {
    Done(Box<Done>),
    Ask(Box<AccessRequest>, BoardAsk),
}

struct Done {
    response: pb::WriteItemResponse,
    /// The item, when the write deleted it: requests about it end.
    deleted: Option<String>,
}

fn no_item(id: &str) -> Status {
    status::not_found(
        pb::EntityKind::BoardItem,
        id,
        format!("there is no board item {id} here"),
        "check the id; the item may have been deleted, or be on a board you can't read",
    )
}

fn item_id(id: Option<&pb::BoardItemId>) -> Result<String, Status> {
    id.map(|id| id.value.clone())
        .filter(|id| !id.is_empty())
        .ok_or_else(|| status::missing("id"))
}

fn dependency_loop(cycle: &[String]) -> Status {
    status::failure(
        Code::FailedPrecondition,
        status::error(
            format!("that dependency would close a loop: {}", cycle.join(" → ")),
            "remove one of the dependencies around the loop, or don't add this one",
            pb::error::Kind::DependencyLoop(pb::DependencyLoop {
                cycle: cycle
                    .iter()
                    .map(|key| pb::BoardItemId { value: key.clone() })
                    .collect(),
            }),
        ),
    )
}

fn revision_conflict(current: &BoardItemRecord, expected: Option<u64>) -> Status {
    let expected = expected.unwrap_or_default();
    status::failure(
        Code::Aborted,
        pb::Error {
            message: format!(
                "{} is at revision {}, not {expected} as the change was made against, and a field this change sets was changed since",
                current.key, current.revision
            ),
            fix: "read the item again, check the change still makes sense, and send it again; the change was kept in the item's history as lost".to_owned(),
            retry: None,
            kind: Some(pb::error::Kind::RevisionConflict(pb::RevisionConflict {
                kind: pb::EntityKind::BoardItem as i32,
                id: current.key.clone(),
                expected: Some(pb::Revision { value: expected }),
                actual: Some(pb::Revision {
                    value: current.revision,
                }),
            })),
        },
    )
}

/// `current` without `remove`, then `add` after it, each once.
fn merged(current: &[String], add: &[String], remove: &[String]) -> Vec<String> {
    let mut list: Vec<String> = current
        .iter()
        .filter(|item| !remove.contains(item))
        .cloned()
        .collect();
    for item in add {
        if !list.contains(item) {
            list.push(item.clone());
        }
    }
    list
}

fn ids(list: &[pb::SessionId]) -> Vec<String> {
    list.iter().map(|id| id.value.clone()).collect()
}

fn item_ids(list: &[pb::BoardItemId]) -> Vec<String> {
    list.iter().map(|id| id.value.clone()).collect()
}

fn page_of(page: Option<&pb::PageRequest>) -> Result<(usize, usize), Status> {
    let Some(page) = page else {
        return Ok((PAGE_DEFAULT, 0));
    };
    let size = match page.page_size as usize {
        0 => PAGE_DEFAULT,
        size => size.min(PAGE_MAX),
    };
    let start = if page.page_token.is_empty() {
        0
    } else {
        page.page_token.parse::<usize>().map_err(|_| {
            status::invalid_field(
                "page.page_token",
                Problem::BadFormat,
                "the page token isn't one this call gave out",
                "start again without a page token",
            )
        })?
    };
    Ok((size, start))
}

impl BoardSvc {
    pub(crate) fn new(
        guard: Arc<Guard>,
        store: Arc<Store>,
        feed: Arc<Feed>,
        inbox: Arc<Inbox>,
        machine: String,
    ) -> Self {
        Self {
            guard,
            store,
            feed,
            inbox,
            machine,
            writes: Arc::new(Mutex::new(())),
            finished: Arc::new(Answers::new()),
        }
    }

    fn refuse_peer(caller: &Caller) -> Result<(), Status> {
        if matches!(caller, Caller::Peer) {
            return Err(status::refused(&Refusal::PeerCannotCall));
        }
        Ok(())
    }

    fn failed(&self, error: StoreError, doing: &str) -> Status {
        match error {
            StoreError::NoBoardItem { key } => no_item(&key),
            StoreError::InvalidId { id } => no_item(&id),
            other => status::store(other, &self.machine, doing),
        }
    }

    fn policy_of(&self, item: &BoardItemRecord) -> BoardRef {
        board_for_policy(&item.board, &item.channel, &self.machine)
    }

    /// Whether the caller may read what's on `board` without being asked.
    fn can_read(&self, caller: &Caller, board: BoardRef) -> bool {
        !matches!(caller, Caller::Peer)
            && matches!(
                self.guard.check(
                    caller,
                    &Call::Board {
                        board,
                        act: BoardAct::Read
                    }
                ),
                Ok(Checked::Allowed)
            )
    }

    /// Item `key` as the caller may see it: one that is deleted, missing or
    /// unreadable looks the same, as not found (BD-14). `tombstones` lets a
    /// deleted one through, for its history.
    fn load(&self, caller: &Caller, key: &str, tombstones: bool) -> Result<BoardItemRecord, Status> {
        match self.store.board_item(key) {
            Ok(Some(item))
                if (tombstones || !item.deleted) && self.can_read(caller, self.policy_of(&item)) =>
            {
                Ok(item)
            }
            Ok(_) => Err(no_item(key)),
            Err(error) => Err(self.failed(error, "read the board item")),
        }
    }

    /// Checks a write on `board` for the caller; `Some` when its rule asks.
    fn gate(
        &self,
        caller: &Caller,
        board: BoardRef,
        act: BoardAct,
        approved: bool,
    ) -> Result<Option<Box<AccessRequest>>, Status> {
        gate(&self.guard, caller, &Call::Board { board, act }, approved)
    }

    fn actor_of(&self, by: &str) -> pb::Actor {
        use pb::actor::Actor;
        if by.is_empty() {
            return pb::Actor {
                actor: Some(Actor::User(pb::UserActor {
                    via: pb::user_actor::Via::App as i32,
                    machine: None,
                })),
            };
        }
        let agent = match self.store.session(by) {
            Ok(Some(record)) => AgentCaller::of(&record, &self.machine).actor(),
            _ => pb::AgentActor {
                session: Some(pb::SessionId {
                    value: by.to_owned(),
                }),
                ..Default::default()
            },
        };
        pb::Actor {
            actor: Some(Actor::Agent(agent)),
        }
    }

    /// Who made the last applied change, as the history says.
    fn updated_by(&self, item: &BoardItemRecord) -> String {
        if item.revision <= 1 {
            return item.created_by.clone();
        }
        self.store
            .board_item_history(&item.key, item.revision.saturating_sub(1), 64)
            .ok()
            .and_then(|entries| entries.into_iter().rev().find(|entry| !entry.lost))
            .map_or_else(|| item.created_by.clone(), |entry| entry.by)
    }

    fn dependency(&self, caller: &Caller, key: &str) -> pb::Dependency {
        let id = Some(pb::BoardItemId {
            value: key.to_owned(),
        });
        match self.store.board_item(key) {
            Ok(Some(item)) if !item.deleted && self.can_read(caller, self.policy_of(&item)) => {
                pb::Dependency {
                    id,
                    title: item.title,
                    column: column_pb(item.column) as i32,
                }
            }
            _ => pb::Dependency {
                id,
                ..Default::default()
            },
        }
    }

    /// `item` for the API. Only the user sees the requests waiting on it
    /// (`asks`), and dependencies the caller can't read show no title.
    fn item_pb(&self, caller: &Caller, item: &BoardItemRecord, asks: &[pb::InboxRequest]) -> pb::BoardItem {
        let pending = if matches!(caller, Caller::User(_)) {
            pending_for(item, asks)
        } else {
            Vec::new()
        };
        pb::BoardItem {
            id: Some(pb::BoardItemId {
                value: item.key.clone(),
            }),
            board: Some(board_pb(&item.board)),
            channel: (!item.channel.is_empty()).then(|| pb::ChannelId {
                value: item.channel.clone(),
            }),
            title: item.title.clone(),
            description: item.description.clone(),
            column: column_pb(item.column) as i32,
            rank: item.rank.clone(),
            created_by: Some(self.actor_of(&item.created_by)),
            created_at: Some(timestamp_ms(item.created_unix_ms)),
            updated: Some(pb::Timestamped {
                at: Some(timestamp_ms(item.updated_unix_ms)),
                by: Some(self.actor_of(&self.updated_by(item))),
            }),
            revision: Some(pb::Revision {
                value: item.revision,
            }),
            sessions: item
                .sessions
                .iter()
                .map(|id| pb::SessionId { value: id.clone() })
                .collect(),
            depends_on: item
                .depends_on
                .iter()
                .map(|key| self.dependency(caller, key))
                .collect(),
            done_at: item.done_unix_ms.map(timestamp_ms),
            pending,
            ..Default::default()
        }
    }

    /// Puts the item as it is now on the feed.
    fn tell(&self, caller: &Caller, change: pb::Change, item: &BoardItemRecord) {
        let session = match &item.board {
            BoardKey::Session(id) => id.clone(),
            _ => String::new(),
        };
        self.feed.publish_in(
            change,
            pb::event::Payload::BoardItem(self.item_pb(caller, item, &[])),
            Scope {
                channel: item.channel.clone(),
                session,
            },
            Some(identity::actor(caller)),
        );
    }

    /// The board a request names: its store key, its channel (empty for
    /// Global) and that channel's name, which the item prefix comes from.
    fn resolve_board(&self, board: &pb::BoardRef) -> Result<(BoardKey, String, String), Status> {
        use pb::board_ref::Board;
        let channel_named = |id: &str| match self.store.channel(id) {
            Ok(Some(channel)) => Ok(channel.name),
            Ok(None) | Err(StoreError::InvalidId { .. }) => Err(status::not_found(
                pb::EntityKind::Channel,
                id,
                format!("there is no channel {id} on this machine"),
                "check the channel id",
            )),
            Err(error) => Err(self.failed(error, "look up the board's channel")),
        };
        match board.board.as_ref().ok_or_else(|| status::missing("board"))? {
            Board::Global(_) => Ok((BoardKey::Global, String::new(), String::new())),
            Board::Channel(id) => {
                let name = channel_named(&id.value)?;
                Ok((BoardKey::Channel(id.value.clone()), id.value.clone(), name))
            }
            Board::Session(id) => match self.store.session(&id.value) {
                Ok(Some(session)) => {
                    let name = self
                        .store
                        .channel(&session.channel)
                        .ok()
                        .flatten()
                        .map(|channel| channel.name)
                        .unwrap_or_default();
                    Ok((BoardKey::Session(id.value.clone()), session.channel, name))
                }
                Ok(None) | Err(StoreError::InvalidId { .. }) => Err(status::not_found(
                    pb::EntityKind::Session,
                    &id.value,
                    format!("there is no session {} on this machine", id.value),
                    "check the session id",
                )),
                Err(error) => Err(self.failed(error, "look up the board's session")),
            },
        }
    }

    /// Sessions that exist, as the store keeps their ids.
    fn known_sessions(&self, list: &[pb::SessionId], field: &str) -> Result<Vec<String>, Status> {
        let mut known = Vec::new();
        for id in list {
            match self.store.session(&id.value) {
                Ok(Some(_)) => known.push(id.value.clone()),
                Ok(None) | Err(StoreError::InvalidId { .. }) => {
                    return Err(status::invalid_field(
                        field,
                        Problem::UnknownValue,
                        format!("there is no session {} on this machine", id.value),
                        "use the id of a session that exists; list sessions to find it",
                    ));
                }
                Err(error) => return Err(self.failed(error, "look up a session")),
            }
        }
        Ok(known)
    }

    /// Items a dependency may point to: the ones the caller can read.
    fn readable_targets(&self, caller: &Caller, list: &[pb::BoardItemId]) -> Result<Vec<String>, Status> {
        list.iter()
            .map(|id| self.load(caller, &id.value, false).map(|item| item.key))
            .collect()
    }

    /// The rank that puts an item in `column` of `board` after `after`, or
    /// at the top when `after` is unset or isn't in that column there.
    fn rank_in(
        &self,
        board: &BoardKey,
        column: i32,
        moving: &str,
        after: Option<&str>,
    ) -> Result<String, Status> {
        let others: Vec<BoardItemRecord> = self
            .store
            .board_items(std::slice::from_ref(board))
            .map_err(|error| self.failed(error, "read the column"))?
            .into_iter()
            .filter(|item| item.column == column && item.key != moving)
            .collect();
        let place = after.and_then(|after| others.iter().position(|item| item.key == after));
        Ok(match place {
            Some(index) => rank::between(
                &others[index].rank,
                others.get(index + 1).map_or("", |next| next.rank.as_str()),
            ),
            None => rank::between("", others.first().map_or("", |first| first.rank.as_str())),
        })
    }

    fn response(&self, caller: &Caller, item: &BoardItemRecord) -> pb::WriteItemResponse {
        pb::WriteItemResponse {
            delivery: pb::Delivery::Applied as i32,
            item: Some(self.item_pb(caller, item, &[])),
            journal: None,
            provisional_id: None,
        }
    }

    /// What the store said to a change, as the call's answer.
    fn finish(
        &self,
        caller: &Caller,
        write: BoardWrite,
        before: u64,
        expected: Option<u64>,
    ) -> Result<Applied, Status> {
        match write {
            BoardWrite::Applied(item) => {
                if item.revision != before {
                    let change = if item.deleted {
                        pb::Change::Deleted
                    } else {
                        pb::Change::Updated
                    };
                    self.tell(caller, change, &item);
                }
                Ok(Applied::Done(Box::new(Done {
                    response: self.response(caller, &item),
                    deleted: item.deleted.then(|| item.key.clone()),
                })))
            }
            BoardWrite::Stale { current } => Err(revision_conflict(&current, expected)),
            BoardWrite::Gone { tombstone } => Err(no_item(&tombstone.key)),
            BoardWrite::Loop { cycle } => Err(dependency_loop(&cycle)),
            BoardWrite::MissingDependency { key } | BoardWrite::DeletedDependency { key } => {
                Err(no_item(&key))
            }
        }
    }

    fn apply(&self, caller: &Caller, op: &Op, approved: bool) -> Result<Applied, Status> {
        Self::refuse_peer(caller)?;
        match op {
            Op::Create(request) => self.create(caller, request, approved),
            Op::Update(request) => self.update(caller, request, approved),
            Op::Move(request) => self.move_item(caller, request, approved),
            Op::Delete(request) => self.delete(caller, request, approved),
        }
    }

    fn create(&self, caller: &Caller, request: &pb::CreateItemRequest, approved: bool) -> Result<Applied, Status> {
        scope::refuse_assigned_in_create(request)?;
        let title = request.title.trim();
        if title.is_empty() {
            return Err(status::missing("title"));
        }
        let column = match request.column {
            0 => FIRST_COLUMN,
            column => column_number(column, "column")?,
        };
        let board = request.board.as_ref().ok_or_else(|| status::missing("board"))?;
        let (key, channel, channel_name) = self.resolve_board(board)?;
        let sessions = self.known_sessions(&request.sessions, "sessions")?;
        let depends_on = self.readable_targets(caller, &request.depends_on)?;
        let policy = board_for_policy(&key, &channel, &self.machine);
        if let Some(asked) = self.gate(caller, policy, BoardAct::Add, approved)? {
            let ask = new_item_ask(
                pb::NewBoardItemPreview {
                    board: Some(board.clone()),
                    title: title.to_owned(),
                    column: column_pb(column) as i32,
                    description: request.description.clone(),
                },
                &request.reason,
            );
            return Ok(Applied::Ask(asked, ask));
        }
        let _write = lock(&self.writes);
        let owner = match &key {
            BoardKey::Global => keys::GLOBAL_OWNER.to_owned(),
            _ => channel.clone(),
        };
        keys::prefix_of(&self.store, &owner, &channel_name)
            .map_err(|error| self.failed(error, "choose the item's key prefix"))?;
        let rank = self.rank_in(&key, column, "", None)?;
        let item = self
            .store
            .create_board_item(&NewBoardItem {
                board: key,
                channel,
                title: title.to_owned(),
                description: request.description.clone(),
                column,
                rank,
                created_by: by_of(caller),
                at_unix_ms: now_ms(),
                sessions,
                depends_on,
            })
            .map_err(|error| self.failed(error, "make the board item"))?;
        self.tell(caller, pb::Change::Created, &item);
        Ok(Applied::Done(Box::new(Done {
            response: self.response(caller, &item),
            deleted: None,
        })))
    }

    fn update(&self, caller: &Caller, request: &pb::UpdateItemRequest, approved: bool) -> Result<Applied, Status> {
        let id = item_id(request.id.as_ref())?;
        let patch = request.patch.clone().unwrap_or_default();
        scope::refuse_assigned_in_patch(&patch)?;
        if patch.title.as_deref().is_some_and(|title| title.trim().is_empty()) {
            return Err(status::missing("patch.title"));
        }
        let item = self.load(caller, &id, false)?;
        let add = self.known_sessions(&patch.sessions_add, "patch.sessions_add")?;
        let remove = ids(&patch.sessions_remove);
        let deps_add = self.readable_targets(caller, &patch.depends_on_add)?;
        let deps_remove = item_ids(&patch.depends_on_remove);
        let sessions = merged(&item.sessions, &add, &remove);
        let deps = merged(&item.depends_on, &deps_add, &deps_remove);

        let mut changes = Vec::new();
        if let Some(title) = patch.title.as_ref().filter(|title| **title != item.title) {
            changes.push(field_change("title", &item.title, title));
        }
        if let Some(text) = patch.description.as_ref().filter(|text| **text != item.description) {
            changes.push(field_change("description", &item.description, text));
        }
        if sessions != item.sessions {
            changes.push(field_change("sessions", &item.sessions.join(", "), &sessions.join(", ")));
        }
        if deps != item.depends_on {
            changes.push(field_change("depends_on", &item.depends_on.join(", "), &deps.join(", ")));
        }
        let act = BoardAct::Edit {
            created_by: made_by_session(&item.created_by),
        };
        if let Some(asked) = self.gate(caller, self.policy_of(&item), act, approved)? {
            return Ok(Applied::Ask(asked, change_ask(&item, changes, false, &request.reason)));
        }

        let _write = lock(&self.writes);
        let fresh = self.load(caller, &id, false)?;
        let sessions = merged(&fresh.sessions, &add, &remove);
        let deps = merged(&fresh.depends_on, &deps_add, &deps_remove);
        let expected = request.expected_revision.as_ref().map(|revision| revision.value);
        let change = BoardItemChange {
            key: id,
            expected_revision: expected,
            by: by_of(caller),
            at_unix_ms: now_ms(),
            title: patch.title.filter(|title| *title != fresh.title),
            description: patch.description.filter(|text| *text != fresh.description),
            column: None,
            rank: None,
            sessions: (sessions != fresh.sessions).then_some(sessions),
            depends_on: (deps != fresh.depends_on).then_some(deps),
            delete: false,
            cause: String::new(),
        };
        let write = self
            .store
            .change_board_item(&change)
            .map_err(|error| self.failed(error, "change the board item"))?;
        self.finish(caller, write, fresh.revision, expected)
    }

    fn move_item(&self, caller: &Caller, request: &pb::MoveItemRequest, approved: bool) -> Result<Applied, Status> {
        let id = item_id(request.id.as_ref())?;
        let column = column_number(request.column, "column")?;
        let after = request
            .after
            .as_ref()
            .map(|after| after.value.as_str())
            .filter(|after| !after.is_empty());
        let item = self.load(caller, &id, false)?;
        let change = if item.column == column {
            field_change("position", "", "moved within the column")
        } else {
            field_change("status", column_label(item.column), column_label(column))
        };
        let act = BoardAct::Edit {
            created_by: made_by_session(&item.created_by),
        };
        if let Some(asked) = self.gate(caller, self.policy_of(&item), act, approved)? {
            return Ok(Applied::Ask(asked, change_ask(&item, vec![change], false, &request.reason)));
        }
        let _write = lock(&self.writes);
        let fresh = self.load(caller, &id, false)?;
        let rank = self.rank_in(&fresh.board, column, &fresh.key, after)?;
        let expected = request.expected_revision.as_ref().map(|revision| revision.value);
        let write = self
            .store
            .change_board_item(&BoardItemChange {
                key: id,
                expected_revision: expected,
                by: by_of(caller),
                at_unix_ms: now_ms(),
                title: None,
                description: None,
                column: (fresh.column != column).then_some(column),
                rank: Some(rank),
                sessions: None,
                depends_on: None,
                delete: false,
                cause: String::new(),
            })
            .map_err(|error| self.failed(error, "move the board item"))?;
        self.finish(caller, write, fresh.revision, expected)
    }

    fn delete(&self, caller: &Caller, request: &pb::DeleteItemRequest, approved: bool) -> Result<Applied, Status> {
        let id = item_id(request.id.as_ref())?;
        let item = self.load(caller, &id, false)?;
        let act = BoardAct::Edit {
            created_by: made_by_session(&item.created_by),
        };
        if let Some(asked) = self.gate(caller, self.policy_of(&item), act, approved)? {
            return Ok(Applied::Ask(asked, change_ask(&item, Vec::new(), true, &request.reason)));
        }
        let _write = lock(&self.writes);
        let expected = request.expected_revision.as_ref().map(|revision| revision.value);
        let write = self
            .store
            .change_board_item(&BoardItemChange {
                key: id,
                expected_revision: expected,
                by: by_of(caller),
                at_unix_ms: now_ms(),
                title: None,
                description: None,
                column: None,
                rank: None,
                sessions: None,
                depends_on: None,
                delete: true,
                cause: String::new(),
            })
            .map_err(|error| self.failed(error, "delete the board item"))?;
        self.finish(caller, write, item.revision, expected)
    }

    /// One try at a write, and the requests about an item it deleted ended.
    async fn attempt(&self, caller: &Caller, op: &Op, approved: bool) -> Result<Applied, Status> {
        let (me, who, op) = (self.clone(), caller.clone(), op.clone());
        let applied = blocking(move || me.apply(&who, &op, approved)).await??;
        if let Applied::Done(done) = &applied
            && let Some(key) = &done.deleted
            && let Err(failed) = self.inbox.withdraw_about_item(key).await
        {
            crate::log::say!(
                "humatad2: couldn't end the requests about deleted item {key}: {}",
                failed.message()
            );
        }
        Ok(applied)
    }

    /// Runs a write: applied now, or put in the Inbox when a rule asks, and
    /// run once the user approves it (D38.6, BD-15).
    async fn write(&self, caller: Caller, op: Op) -> Result<Response<pb::WriteItemResponse>, Status> {
        let (name, what, idempotency) = op.identity();
        let key = answer_key(
            name,
            &caller,
            idempotency.clone().map(|value| pb::IdempotencyKey { value }).as_ref(),
            &what,
        );
        if let Some(done) = key.as_deref().and_then(|key| self.finished.get(key)) {
            return Ok(Response::new(done));
        }
        match self.attempt(&caller, &op, false).await? {
            Applied::Done(done) => {
                if let Some(key) = key {
                    self.finished.put(key, done.response.clone());
                }
                Ok(Response::new(done.response))
            }
            Applied::Ask(asked, board) => {
                let (me, who) = (self.clone(), caller.clone());
                let run: Run = Box::new(move || {
                    Box::pin(async move {
                        match me.attempt(&who, &op, true).await? {
                            Applied::Done(done) => Ok(Deferred {
                                result: done.response.encode_to_vec(),
                                result_type: "humata.v1.WriteItemResponse".to_owned(),
                            }),
                            Applied::Ask(..) => Err(status::internal(
                                "an approved call was sent to Ask me a second time",
                                "repeat the call",
                            )),
                        }
                    })
                });
                Err(self
                    .inbox
                    .defer_board(&caller, *asked, idempotency, board, run)
                    .await)
            }
        }
    }

    /// The items in the scope a view names, by column then rank.
    fn in_view(&self, view: &pb::BoardView) -> Result<Vec<BoardItemRecord>, Status> {
        use pb::board_view::View;
        let read = |found: Result<Vec<BoardItemRecord>, StoreError>| match found {
            Ok(items) => Ok(items),
            Err(StoreError::InvalidId { .. }) => Ok(Vec::new()),
            Err(error) => Err(self.failed(error, "list board items")),
        };
        match view.view.as_ref().ok_or_else(|| status::missing("view"))? {
            View::Board(board) => {
                let (key, _, _) = match self.resolve_board(board) {
                    Ok(resolved) => resolved,
                    Err(status) if status.code() == Code::NotFound => return Ok(Vec::new()),
                    Err(status) => return Err(status),
                };
                read(self.store.board_items(&[key]))
            }
            View::ChannelWithSessions(channel) => read(self.store.board_items_in_channel(&channel.value)),
            View::SessionWithForks(session) => {
                read(self.store.board_items(&[BoardKey::Session(session.value.clone())]))
            }
            View::GlobalWithChannels(scope) if scope.global_only => {
                read(self.store.board_items(&[BoardKey::Global]))
            }
            View::GlobalWithChannels(scope) => match scope.channel.as_ref() {
                Some(channel) => read(
                    self.store
                        .board_items(&[BoardKey::Global, BoardKey::Channel(channel.value.clone())]),
                ),
                None => read(self.store.all_board_items_but_sessions()),
            },
        }
    }

    fn list(
        &self,
        caller: &Caller,
        request: &pb::ListItemsRequest,
        asks: &[pb::InboxRequest],
    ) -> Result<pb::ListItemsResponse, Status> {
        let view = request.view.as_ref().ok_or_else(|| status::missing("view"))?;
        let wanted = request
            .columns
            .iter()
            .map(|column| column_number(*column, "columns"))
            .collect::<Result<Vec<_>, _>>()?;
        let (size, start) = page_of(request.page.as_ref())?;
        // Boards the caller can't read are left out, not refused: a scope
        // that spans boards shows the ones it may see.
        let mut readable: Vec<(BoardKey, bool)> = Vec::new();
        let mut visible = Vec::new();
        for item in self.in_view(view)? {
            let known = readable.iter().find(|(board, _)| *board == item.board).map(|(_, ok)| *ok);
            let ok = match known {
                Some(ok) => ok,
                None => {
                    let ok = self.can_read(caller, self.policy_of(&item));
                    readable.push((item.board.clone(), ok));
                    ok
                }
            };
            if ok {
                visible.push(item);
            }
        }
        let counts = (1..=DONE)
            .map(|column| pb::ColumnCount {
                column: column_pb(column) as i32,
                items: u32::try_from(visible.iter().filter(|item| item.column == column).count())
                    .unwrap_or(u32::MAX),
            })
            .collect();
        let shown: Vec<&BoardItemRecord> = visible
            .iter()
            .filter(|item| wanted.is_empty() || wanted.contains(&item.column))
            .collect();
        let page: Vec<&&BoardItemRecord> = shown.iter().skip(start).take(size).collect();
        let next_page_token = if start + page.len() < shown.len() {
            (start + page.len()).to_string()
        } else {
            String::new()
        };
        Ok(pb::ListItemsResponse {
            items: page.iter().map(|item| self.item_pb(caller, item, asks)).collect(),
            counts,
            page: Some(pb::PageResponse { next_page_token }),
        })
    }

    async fn asks_for(&self, caller: &Caller) -> Vec<pb::InboxRequest> {
        if matches!(caller, Caller::User(_)) {
            self.inbox.pending_board_asks().await
        } else {
            Vec::new()
        }
    }
}

#[tonic::async_trait]
impl pb::board_service_server::BoardService for BoardSvc {
    async fn list_items(
        &self,
        request: Request<pb::ListItemsRequest>,
    ) -> Result<Response<pb::ListItemsResponse>, Status> {
        let caller = identity::caller(&request)?;
        Self::refuse_peer(&caller)?;
        let request = request.into_inner();
        let asks = self.asks_for(&caller).await;
        let me = self.clone();
        blocking(move || me.list(&caller, &request, &asks))
            .await?
            .map(Response::new)
    }

    async fn get_item(
        &self,
        request: Request<pb::GetItemRequest>,
    ) -> Result<Response<pb::BoardItem>, Status> {
        let caller = identity::caller(&request)?;
        Self::refuse_peer(&caller)?;
        let id = item_id(request.into_inner().id.as_ref())?;
        let asks = self.asks_for(&caller).await;
        let me = self.clone();
        blocking(move || {
            let item = me.load(&caller, &id, false)?;
            Ok(me.item_pb(&caller, &item, &asks))
        })
        .await?
        .map(Response::new)
    }

    async fn create_item(
        &self,
        request: Request<pb::CreateItemRequest>,
    ) -> Result<Response<pb::WriteItemResponse>, Status> {
        let caller = identity::caller(&request)?;
        self.write(caller, Op::Create(request.into_inner())).await
    }

    async fn update_item(
        &self,
        request: Request<pb::UpdateItemRequest>,
    ) -> Result<Response<pb::WriteItemResponse>, Status> {
        let caller = identity::caller(&request)?;
        self.write(caller, Op::Update(request.into_inner())).await
    }

    async fn move_item(
        &self,
        request: Request<pb::MoveItemRequest>,
    ) -> Result<Response<pb::WriteItemResponse>, Status> {
        let caller = identity::caller(&request)?;
        self.write(caller, Op::Move(request.into_inner())).await
    }

    async fn delete_item(
        &self,
        request: Request<pb::DeleteItemRequest>,
    ) -> Result<Response<pb::WriteItemResponse>, Status> {
        let caller = identity::caller(&request)?;
        self.write(caller, Op::Delete(request.into_inner())).await
    }

    async fn get_item_history(
        &self,
        request: Request<pb::GetItemHistoryRequest>,
    ) -> Result<Response<pb::GetItemHistoryResponse>, Status> {
        let caller = identity::caller(&request)?;
        Self::refuse_peer(&caller)?;
        let request = request.into_inner();
        let id = item_id(request.id.as_ref())?;
        let (size, after) = page_of(request.page.as_ref())?;
        let me = self.clone();
        blocking(move || {
            let item = me.load(&caller, &id, true)?;
            let mut found = me
                .store
                .board_item_history(&item.key, after as u64, size + 1)
                .map_err(|error| me.failed(error, "read the item's history"))?;
            let more = found.len() > size;
            found.truncate(size);
            let next_page_token = match (more, found.last()) {
                (true, Some(last)) => last.n.to_string(),
                _ => String::new(),
            };
            Ok(pb::GetItemHistoryResponse {
                entries: found
                    .iter()
                    .map(|entry| history::entry_pb(entry, me.actor_of(&entry.by)))
                    .collect(),
                page: Some(pb::PageResponse { next_page_token }),
            })
        })
        .await?
        .map(Response::new)
    }
}

#[cfg(test)]
mod tests {
    use super::{merged, page_of, pb};

    fn list(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    #[test]
    fn a_list_edit_removes_then_adds_each_once_keeping_the_order() {
        assert_eq!(
            merged(&list(&["a", "b", "c"]), &list(&["d", "a"]), &list(&["b"])),
            list(&["a", "c", "d"])
        );
    }

    #[test]
    fn a_page_token_is_where_the_next_page_starts() {
        let page = pb::PageRequest {
            page_size: 5,
            page_token: "10".to_owned(),
        };
        assert_eq!(page_of(Some(&page)).ok(), Some((5, 10)));
        let bad = pb::PageRequest {
            page_size: 5,
            page_token: "x".to_owned(),
        };
        assert!(page_of(Some(&bad)).is_err());
    }
}
