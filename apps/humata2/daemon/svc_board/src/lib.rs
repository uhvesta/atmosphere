//! BoardService (boards.proto): board items (BD-1..BD-16, SL-6..SL-8, D155)
//! as a standalone service crate (`humata2_svc_board`) over the store's
//! `boards`. An agent's write the rules send to Ask me waits in the Inbox with
//! its change on the card and is done when the user approves it (BD-15);
//! everything applied goes on the feed.
//!
//! The store's calls block, so the work runs on blocking threads. Writes
//! take one lock, so the lists an edit adds to and takes from (sessions and
//! dependencies) are read and saved without another write in between.

mod asks;
mod hidden;
mod history;
mod journaled;
mod keys;
pub mod ports;
mod rank;
mod replay;
mod scope;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use humata2_access::{AccessRequest, BoardAct, BoardRef, Call, Refusal};
use humata2_daemon_ports::{BoardAsk, Caller, Deferred, Run};
#[allow(unused_imports, ambiguous_glob_reexports)]
pub(crate) mod pb {
    pub use humata2_grpc_board::humata::v1::*;
    pub use humata2_grpc_catalog::humata::v1::*;
    pub use humata2_grpc_core::humata::v1::*;
    pub use humata2_grpc_events::humata::v1::*;
    pub use humata2_grpc_files::humata::v1::*;
    pub use humata2_grpc_inbox::humata::v1::*;
    pub use humata2_grpc_machines::humata::v1::*;
    pub use humata2_grpc_peer::humata::v1::*;
}
use humata2_rpc_kit::{
    Answers, blocking, error, failure, internal, invalid_field, lock, missing, not_built_status,
    not_found, now_ms, timestamp_ms,
};
use humata2_store::{
    BoardItemChange, BoardItemRecord, BoardKey, BoardWrite, NewBoardItem, Store, StoreError,
};
use pb::invalid_field::Problem;
use prost::Message;
use tonic::{Code, Request, Response, Status};

pub use self::ports::{
    AwayCall, BoardFeed, BoardForwarder, BoardGuard, BoardInbox, Checked, HostCalls, HostFiles,
    HostMachines, HostRepos, HostTransfers, Method, OwnedItem, Placement, Replayed, Row, Scope,
    SessionTell,
};
use self::asks::{change_ask, field_change, new_item_ask, pending_for};
use self::ports::{agent_caller_of, answer_key, caller_of, gate, refused, store_error};
use self::scope::{
    board_for_policy, board_pb, by_of, column_label, column_number, column_pb, made_by_session,
};

/// Items per page when the caller names no size.
const PAGE_DEFAULT: usize = 200;

/// The most items in one page.
const PAGE_MAX: usize = 1000;

/// The column an item made without one starts in: To do, as the app's New
/// item and every door's default say (VF-21).
const DEFAULT_COLUMN: i32 = 2;

/// The Done column.
const DONE: i32 = 5;

#[derive(Clone)]
pub struct BoardSvc {
    guard: Arc<dyn BoardGuard>,
    store: Arc<Store>,
    feed: Arc<dyn BoardFeed>,
    /// Where an agent's write a rule sends to Ask me waits (BD-15).
    inbox: Arc<dyn BoardInbox>,
    machine: String,
    writes: Arc<Mutex<()>>,
    finished: Arc<Answers<pb::WriteItemResponse>>,
    /// On a machine: where its agents' board calls go while the host is linked (D227).
    forwarder: Option<Arc<dyn BoardForwarder>>,
    /// On the host: tells the feed a session changed, merged as the app
    /// sees it, when a replay leaves a write out of its summary (AW-9, D267).
    told: Option<Arc<dyn SessionTell>>,
    /// On the host: the repo library a machine's agents search (RL-12, D568).
    repos: Option<Arc<dyn HostRepos>>,
    /// On the host: the machine, file and transfer services a machine's
    /// agents reach through it (D768), given once they are made.
    host_calls: Arc<std::sync::OnceLock<HostCalls>>,
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
        let id = |id: &Option<pb::BoardItemId>| {
            id.as_ref()
                .map(|id| id.value.clone())
                .unwrap_or_default()
        };
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
    not_found(
        pb::EntityKind::BoardItem,
        id,
        format!("there is no board item {id} here"),
        "check the id; the item may have been deleted, or be on a board you can't read",
    )
}

fn item_id(id: Option<&pb::BoardItemId>) -> Result<String, Status> {
    id.map(|id| id.value.clone())
        .filter(|id| !id.is_empty())
        .ok_or_else(|| missing("id"))
}

fn dependency_loop(cycle: &[String]) -> Status {
    failure(
        Code::FailedPrecondition,
        error(
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
    failure(
        Code::Aborted,
        pb::Error {
            message: format!(
                "{} changed since you read it: it is at revision {}, not {expected}, and a field this change sets changed too",
                current.key, current.revision
            ),
            fix: "read the item again with `humata board` or `board_item_get`, check the change still makes sense, and send it again with the new revision; the refused change is kept in the item's history as lost".to_owned(),
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

/// An item made with no board goes on the calling agent's own session board
/// (VF-21, BD-9): the daemon decides, so the CLI, MCP and the app agree. Done
/// before forwarding and journaling, so the board travels with the request.
/// A person names the board, and `create` refuses one that doesn't.
fn board_defaulted(
    caller: &Caller,
    mut request: pb::CreateItemRequest,
) -> pb::CreateItemRequest {
    if let (None, Caller::Agent(agent)) = (&request.board, caller) {
        request.board = Some(pb::BoardRef {
            board: Some(pb::board_ref::Board::Session(pb::SessionId {
                value: agent.session.id.to_string(),
            })),
        });
    }
    request
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
            invalid_field(
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
    pub fn new(
        guard: Arc<dyn BoardGuard>,
        store: Arc<Store>,
        feed: Arc<dyn BoardFeed>,
        inbox: Arc<dyn BoardInbox>,
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
            forwarder: None,
            told: None,
            repos: None,
            host_calls: Arc::new(std::sync::OnceLock::new()),
        }
    }

    /// Tells the feed of a session whose summary of what the host left out
    /// of its agent's replayed writes changed (AW-9, D267).
    pub fn telling(mut self, told: Arc<dyn SessionTell>) -> Self {
        self.told = Some(told);
        self
    }

    /// Runs the repo searches and library lists a machine forwards for its
    /// agents against `repos`, the host's library (RL-12, D568).
    pub fn with_repos(mut self, repos: Arc<dyn HostRepos>) -> Self {
        self.repos = Some(repos);
        self
    }

    /// Runs a machine's agents' machine lookups, file reads and transfers
    /// against `calls`, the host's own services (D768). The first call wins;
    /// every clone of this service sees it.
    pub fn answer_host_calls(&self, calls: HostCalls) {
        let _ = self.host_calls.set(calls);
    }

    /// The host's services, or why a forwarded `method` can't be run.
    fn host_calls_for(&self, method: &str) -> Result<&HostCalls, Status> {
        self.host_calls
            .get()
            .ok_or_else(|| not_built_status(method))
    }

    /// The host's repo library, or why a forwarded `method` can't be run.
    fn repos_for(&self, method: &str) -> Result<Arc<dyn HostRepos>, Status> {
        self.repos
            .clone()
            .ok_or_else(|| not_built_status(method))
    }

    /// Sends the board calls of this machine's agents to the host while it
    /// is linked (D227): `forwarder` is the machine's end of the Uplink.
    pub fn forwarding(mut self, forwarder: Arc<dyn BoardForwarder>) -> Self {
        self.forwarder = Some(forwarder);
        self
    }

    /// The machine's forwarder and the session, when `caller` is an agent on
    /// a machine whose host is linked: its call is the host's to run. Away,
    /// the call goes on as it always did.
    fn forwarder_for(&self, caller: &Caller) -> Option<(&Arc<dyn BoardForwarder>, String)> {
        let Caller::Agent(agent) = caller else {
            return None;
        };
        let forwarder = self.forwarder.as_ref()?;
        forwarder
            .forwards()
            .then(|| (forwarder, agent.session.id.to_string()))
    }

    /// Sends `request` to `row`'s method on the host as agent `session`, and
    /// answers with the host's response.
    async fn forward<Req: Message, Res: Message + Default>(
        &self,
        forwarder: &dyn BoardForwarder,
        session: &str,
        row: &'static Row,
        request: &Req,
        key: Option<pb::IdempotencyKey>,
    ) -> Result<Response<Res>, Status> {
        let request_bytes = request.encode_to_vec();
        let bytes = forwarder
            .call(session, row, request_bytes.clone(), key)
            .await?;
        // What the host answered a read with is what the machine answers it with
        // when the host is away (D227).
        if row.reads_board() {
            self.keep_last_copy(session, row, request_bytes, &bytes)
                .await;
        }
        Res::decode(bytes.as_slice())
            .map(Response::new)
            .map_err(|error| {
                internal(
                    format!(
                        "the host's answer to {} couldn't be read: {error}; update Humata on this machine so both ends speak the same version",
                        row.name
                    ),
                    "",
                )
            })
    }

    fn refuse_peer(caller: &Caller) -> Result<(), Status> {
        if matches!(caller, Caller::Peer) {
            return Err(refused(&Refusal::PeerCannotCall));
        }
        Ok(())
    }

    fn failed(&self, error: StoreError, doing: &str) -> Status {
        match error {
            StoreError::NoBoardItem { key } => no_item(&key),
            StoreError::InvalidId { id } => no_item(&id),
            other => store_error(other, &self.machine, doing),
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

    /// Item `key` as the caller may see it: one that is deleted, hidden,
    /// missing or unreadable looks the same, as not found (BD-14, D216).
    /// `tombstones` lets a deleted one through, for its history.
    fn load(
        &self,
        caller: &Caller,
        key: &str,
        tombstones: bool,
    ) -> Result<BoardItemRecord, Status> {
        let item = match self.store.board_item(key) {
            Ok(Some(item)) if tombstones || !item.deleted => item,
            Ok(_) => return Err(no_item(key)),
            Err(error) => return Err(self.failed(error, "read the board item")),
        };
        if !self.can_read(caller, self.policy_of(&item)) {
            return Err(no_item(key));
        }
        match hidden::is_hidden_now(&self.store, &item) {
            Ok(false) => Ok(item),
            Ok(true) => Err(no_item(key)),
            Err(error) => Err(self.failed(error, "read the board item's archive")),
        }
    }

    /// The ids of archived sessions, read once for the call.
    fn archived(&self) -> Result<HashSet<String>, Status> {
        hidden::archived(&self.store)
            .map_err(|error| self.failed(error, "read the archived sessions"))
    }

    /// Checks a write on `board` for the caller; `Some` when its rule asks.
    fn gate(
        &self,
        caller: &Caller,
        board: BoardRef,
        act: BoardAct,
        approved: bool,
    ) -> Result<Option<Box<AccessRequest>>, Status> {
        gate(
            self.guard.as_ref(),
            caller,
            &Call::Board { board, act },
            approved,
        )
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
            Ok(Some(record)) => agent_caller_of(&record, &self.machine).actor(),
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

    /// The dependency on `key` as the caller sees it, or `None` when `key` is
    /// hidden: such an edge is left out (D216).
    fn dependency(
        &self,
        caller: &Caller,
        key: &str,
        archived: &HashSet<String>,
    ) -> Option<pb::Dependency> {
        let id = Some(pb::BoardItemId {
            value: key.to_owned(),
        });
        match self.store.board_item(key) {
            Ok(Some(item)) if hidden::is_hidden(&item, archived) => None,
            Ok(Some(item)) if !item.deleted && self.can_read(caller, self.policy_of(&item)) => {
                Some(pb::Dependency {
                    id,
                    title: item.title,
                    column: column_pb(item.column) as i32,
                })
            }
            _ => Some(pb::Dependency {
                id,
                ..Default::default()
            }),
        }
    }

    /// `item` for the API. Only the user sees the requests waiting on it
    /// (`asks`), dependencies the caller can't read show no title, and
    /// dependencies on hidden items are left out (`archived` is the call's
    /// archived sessions).
    fn item_pb(
        &self,
        caller: &Caller,
        item: &BoardItemRecord,
        asks: &[pb::InboxRequest],
        archived: &HashSet<String>,
    ) -> pb::BoardItem {
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
                .filter_map(|key| self.dependency(caller, key, archived))
                .collect(),
            done_at: item.done_unix_ms.map(timestamp_ms),
            pending,
            ..Default::default()
        }
    }

    /// Puts the item as it is now on the feed.
    fn tell(
        &self,
        caller: &Caller,
        change: pb::Change,
        item: &BoardItemRecord,
        archived: &HashSet<String>,
    ) {
        // The copy each machine holds lists the items its sessions own (D227).
        if journaled::changes_owned_items(change, item) {
            self.guard.policy_changed();
        }
        let session = match &item.board {
            BoardKey::Session(id) => id.clone(),
            _ => String::new(),
        };
        self.feed.publish_in(
            change,
            pb::event::Payload::BoardItem(self.item_pb(caller, item, &[], archived)),
            Scope {
                channel: item.channel.clone(),
                session,
            },
            Some(humata2_daemon_ports::actor(caller)),
        );
    }

    /// The board a request names: its store key, its channel (empty for
    /// Global) and that channel's name, which the item prefix comes from.
    fn resolve_board(&self, board: &pb::BoardRef) -> Result<(BoardKey, String, String), Status> {
        use pb::board_ref::Board;
        let channel_named = |id: &str| match self.store.channel(id) {
            Ok(Some(channel)) => Ok(channel.name),
            Ok(None) | Err(StoreError::InvalidId { .. }) => Err(not_found(
                pb::EntityKind::Channel,
                id,
                format!("there is no channel {id} on this machine"),
                "check the channel id",
            )),
            Err(error) => Err(self.failed(error, "look up the board's channel")),
        };
        match board.board.as_ref().ok_or_else(|| missing("board"))? {
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
                Ok(None) | Err(StoreError::InvalidId { .. }) => Err(not_found(
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
                    return Err(invalid_field(
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
    fn readable_targets(
        &self,
        caller: &Caller,
        list: &[pb::BoardItemId],
    ) -> Result<Vec<String>, Status> {
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

    fn response(
        &self,
        caller: &Caller,
        item: &BoardItemRecord,
        archived: &HashSet<String>,
    ) -> pb::WriteItemResponse {
        pb::WriteItemResponse {
            delivery: pb::Delivery::Applied as i32,
            item: Some(self.item_pb(caller, item, &[], archived)),
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
                let archived = self.archived()?;
                if item.revision != before {
                    let change = if item.deleted {
                        pb::Change::Deleted
                    } else {
                        pb::Change::Updated
                    };
                    self.tell(caller, change, &item, &archived);
                }
                Ok(Applied::Done(Box::new(Done {
                    response: self.response(caller, &item, &archived),
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

    fn create(
        &self,
        caller: &Caller,
        request: &pb::CreateItemRequest,
        approved: bool,
    ) -> Result<Applied, Status> {
        scope::refuse_assigned_in_create(request)?;
        let title = request.title.trim();
        if title.is_empty() {
            return Err(missing("title"));
        }
        let column = match request.column {
            0 => DEFAULT_COLUMN,
            column => column_number(column, "column")?,
        };
        let board = request.board.as_ref().ok_or_else(|| {
            invalid_field(
                "board",
                pb::invalid_field::Problem::Missing,
                "this item names no board, and only an agent's item defaults to its session's board",
                "name the board with --global, --session or --channel, or on the Board page of the app",
            )
        })?;
        let (key, channel, channel_name) = self.resolve_board(board)?;
        // D216: an archived session's board is hidden, so it takes no new
        // items, as a missing session's board doesn't.
        if let BoardKey::Session(session) = &key {
            match self.store.archive(session) {
                Ok(None) => {}
                Ok(Some(_)) => {
                    return Err(not_found(
                        pb::EntityKind::Session,
                        session,
                        format!("session {session} is archived, so its board takes no new items"),
                        "restore the session first, or add the item to its channel's board",
                    ));
                }
                Err(error) => {
                    return Err(
                        self.failed(error, "check whether the board's session is archived")
                    );
                }
            }
        }
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
        let archived = self.archived()?;
        self.tell(caller, pb::Change::Created, &item, &archived);
        Ok(Applied::Done(Box::new(Done {
            response: self.response(caller, &item, &archived),
            deleted: None,
        })))
    }

    fn update(
        &self,
        caller: &Caller,
        request: &pb::UpdateItemRequest,
        approved: bool,
    ) -> Result<Applied, Status> {
        let id = item_id(request.id.as_ref())?;
        let patch = request.patch.clone().unwrap_or_default();
        scope::refuse_assigned_in_patch(&patch)?;
        if patch
            .title
            .as_deref()
            .is_some_and(|title| title.trim().is_empty())
        {
            return Err(missing("patch.title"));
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
        if let Some(text) = patch
            .description
            .as_ref()
            .filter(|text| **text != item.description)
        {
            changes.push(field_change("description", &item.description, text));
        }
        if sessions != item.sessions {
            changes.push(field_change(
                "sessions",
                &item.sessions.join(", "),
                &sessions.join(", "),
            ));
        }
        if deps != item.depends_on {
            changes.push(field_change(
                "depends_on",
                &item.depends_on.join(", "),
                &deps.join(", "),
            ));
        }
        let act = BoardAct::Edit {
            created_by: made_by_session(&item.created_by),
        };
        if let Some(asked) = self.gate(caller, self.policy_of(&item), act, approved)? {
            return Ok(Applied::Ask(
                asked,
                change_ask(&item, changes, false, &request.reason),
            ));
        }

        let _write = lock(&self.writes);
        let fresh = self.load(caller, &id, false)?;
        let sessions = merged(&fresh.sessions, &add, &remove);
        let deps = merged(&fresh.depends_on, &deps_add, &deps_remove);
        let expected = request
            .expected_revision
            .as_ref()
            .map(|revision| revision.value);
        let change = BoardItemChange {
            key: id,
            expected_revision: expected,
            by: by_of(caller),
            at_unix_ms: now_ms(),
            title: patch.title.filter(|title| *title != fresh.title),
            description: patch
                .description
                .filter(|text| *text != fresh.description),
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

    fn move_item(
        &self,
        caller: &Caller,
        request: &pb::MoveItemRequest,
        approved: bool,
    ) -> Result<Applied, Status> {
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
            return Ok(Applied::Ask(
                asked,
                change_ask(&item, vec![change], false, &request.reason),
            ));
        }
        let _write = lock(&self.writes);
        let fresh = self.load(caller, &id, false)?;
        let rank = self.rank_in(&fresh.board, column, &fresh.key, after)?;
        let expected = request
            .expected_revision
            .as_ref()
            .map(|revision| revision.value);
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

    fn delete(
        &self,
        caller: &Caller,
        request: &pb::DeleteItemRequest,
        approved: bool,
    ) -> Result<Applied, Status> {
        let id = item_id(request.id.as_ref())?;
        let item = self.load(caller, &id, false)?;
        let act = BoardAct::Edit {
            created_by: made_by_session(&item.created_by),
        };
        if let Some(asked) = self.gate(caller, self.policy_of(&item), act, approved)? {
            return Ok(Applied::Ask(
                asked,
                change_ask(&item, Vec::new(), true, &request.reason),
            ));
        }
        let _write = lock(&self.writes);
        let expected = request
            .expected_revision
            .as_ref()
            .map(|revision| revision.value);
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
            humata2_rpc_kit::log::say!(
                "humatad2: couldn't end the requests about deleted item {key}: {}",
                failed.message()
            );
        }
        Ok(applied)
    }

    /// Runs a write: applied now, or put in the Inbox when a rule asks, and
    /// run once the user approves it (D38.6, BD-15).
    async fn write(
        &self,
        caller: Caller,
        op: Op,
    ) -> Result<Response<pb::WriteItemResponse>, Status> {
        let (name, what, idempotency) = op.identity();
        let key = answer_key(
            name,
            &caller,
            idempotency
                .clone()
                .map(|value| pb::IdempotencyKey { value })
                .as_ref(),
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
                let run: Run = Box::new(move |_| {
                    Box::pin(async move {
                        match me.attempt(&who, &op, true).await? {
                            Applied::Done(done) => Ok(Deferred {
                                result: done.response.encode_to_vec(),
                                result_type: "humata.v1.WriteItemResponse".to_owned(),
                            }),
                            Applied::Ask(..) => Err(internal(
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
        match view.view.as_ref().ok_or_else(|| missing("view"))? {
            View::Board(board) => {
                let (key, _, _) = match self.resolve_board(board) {
                    Ok(resolved) => resolved,
                    Err(status) if status.code() == Code::NotFound => return Ok(Vec::new()),
                    Err(status) => return Err(status),
                };
                read(self.store.board_items(&[key]))
            }
            View::ChannelWithSessions(channel) => {
                read(self.store.board_items_in_channel(&channel.value))
            }
            View::SessionWithForks(session) => {
                let origins = self
                    .store
                    .fork_origins()
                    .map_err(|error| self.failed(error, "list the forks"))?;
                let boards: Vec<BoardKey> = hidden::family(&origins, &session.value)
                    .into_iter()
                    .map(BoardKey::Session)
                    .collect();
                read(self.store.board_items(&boards))
            }
            View::GlobalWithChannels(scope) if scope.global_only => {
                read(self.store.board_items(&[BoardKey::Global]))
            }
            View::GlobalWithChannels(scope) => match scope.channel.as_ref() {
                Some(channel) => read(self.store.board_items(&[
                    BoardKey::Global,
                    BoardKey::Channel(channel.value.clone()),
                ])),
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
        let view = request.view.as_ref().ok_or_else(|| missing("view"))?;
        let wanted = request
            .columns
            .iter()
            .map(|column| column_number(*column, "columns"))
            .collect::<Result<Vec<_>, _>>()?;
        let (size, start) = page_of(request.page.as_ref())?;
        let archived = self.archived()?;
        // Boards the caller can't read are left out, not refused: a scope
        // that spans boards shows the ones it may see. Hidden items (D216)
        // are left out of every view, the counts included.
        let mut readable: Vec<(BoardKey, bool)> = Vec::new();
        let mut visible = Vec::new();
        for item in self.in_view(view)? {
            if hidden::is_hidden(&item, &archived) {
                continue;
            }
            let known = readable
                .iter()
                .find(|(board, _)| *board == item.board)
                .map(|(_, ok)| *ok);
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
            items: page
                .iter()
                .map(|item| self.item_pb(caller, item, asks, &archived))
                .collect(),
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
        let caller = caller_of(&request)?;
        Self::refuse_peer(&caller)?;
        let request = request.into_inner();
        if let Some((forwarder, session)) = self.forwarder_for(&caller) {
            return self
                .forward(
                    forwarder.as_ref(),
                    &session,
                    Method::ListItems.row(),
                    &request,
                    None,
                )
                .await;
        }
        if let Some(agent) = self.away_agent(&caller) {
            return self
                .last_copy(
                    &caller,
                    agent,
                    Method::ListItems.row(),
                    &request,
                    Self::list_boards,
                )
                .await;
        }
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
        let caller = caller_of(&request)?;
        Self::refuse_peer(&caller)?;
        let request = request.into_inner();
        if let Some((forwarder, session)) = self.forwarder_for(&caller) {
            return self
                .forward(
                    forwarder.as_ref(),
                    &session,
                    Method::GetItem.row(),
                    &request,
                    None,
                )
                .await;
        }
        let id = item_id(request.id.as_ref())?;
        if let Some(agent) = self.away_agent(&caller) {
            return self
                .last_copy(
                    &caller,
                    agent,
                    Method::GetItem.row(),
                    &request,
                    Self::item_boards,
                )
                .await;
        }
        let asks = self.asks_for(&caller).await;
        let me = self.clone();
        blocking(move || me.read_item(&caller, &id, &asks))
            .await?
            .map(Response::new)
    }

    async fn create_item(
        &self,
        request: Request<pb::CreateItemRequest>,
    ) -> Result<Response<pb::WriteItemResponse>, Status> {
        let caller = caller_of(&request)?;
        let request = board_defaulted(&caller, request.into_inner());
        if let Some((forwarder, session)) = self.forwarder_for(&caller) {
            return self
                .forward(
                    forwarder.as_ref(),
                    &session,
                    Method::CreateItem.row(),
                    &request,
                    request.idempotency_key.clone(),
                )
                .await;
        }
        let op = Op::Create(request);
        // Away, a write the rules allow and the copy proves is kept in the journal (D227).
        if let Some(done) = self.journaled(&caller, &op).await? {
            return Ok(done);
        }
        self.write(caller, op).await
    }

    async fn update_item(
        &self,
        request: Request<pb::UpdateItemRequest>,
    ) -> Result<Response<pb::WriteItemResponse>, Status> {
        let caller = caller_of(&request)?;
        let request = request.into_inner();
        if let Some((forwarder, session)) = self.forwarder_for(&caller) {
            return self
                .forward(
                    forwarder.as_ref(),
                    &session,
                    Method::UpdateItem.row(),
                    &request,
                    request.idempotency_key.clone(),
                )
                .await;
        }
        let op = Op::Update(request);
        // Away, a write the rules allow and the copy proves is kept in the journal (D227).
        if let Some(done) = self.journaled(&caller, &op).await? {
            return Ok(done);
        }
        self.write(caller, op).await
    }

    async fn move_item(
        &self,
        request: Request<pb::MoveItemRequest>,
    ) -> Result<Response<pb::WriteItemResponse>, Status> {
        let caller = caller_of(&request)?;
        let request = request.into_inner();
        if let Some((forwarder, session)) = self.forwarder_for(&caller) {
            return self
                .forward(
                    forwarder.as_ref(),
                    &session,
                    Method::MoveItem.row(),
                    &request,
                    request.idempotency_key.clone(),
                )
                .await;
        }
        let op = Op::Move(request);
        // Away, a write the rules allow and the copy proves is kept in the journal (D227).
        if let Some(done) = self.journaled(&caller, &op).await? {
            return Ok(done);
        }
        self.write(caller, op).await
    }

    async fn delete_item(
        &self,
        request: Request<pb::DeleteItemRequest>,
    ) -> Result<Response<pb::WriteItemResponse>, Status> {
        let caller = caller_of(&request)?;
        let request = request.into_inner();
        if let Some((forwarder, session)) = self.forwarder_for(&caller) {
            return self
                .forward(
                    forwarder.as_ref(),
                    &session,
                    Method::DeleteItem.row(),
                    &request,
                    request.idempotency_key.clone(),
                )
                .await;
        }
        let op = Op::Delete(request);
        // Away, a write the rules allow and the copy proves is kept in the journal (D227).
        if let Some(done) = self.journaled(&caller, &op).await? {
            return Ok(done);
        }
        self.write(caller, op).await
    }

    async fn get_item_history(
        &self,
        request: Request<pb::GetItemHistoryRequest>,
    ) -> Result<Response<pb::GetItemHistoryResponse>, Status> {
        let caller = caller_of(&request)?;
        Self::refuse_peer(&caller)?;
        let request = request.into_inner();
        if let Some((forwarder, session)) = self.forwarder_for(&caller) {
            return self
                .forward(
                    forwarder.as_ref(),
                    &session,
                    Method::GetItemHistory.row(),
                    &request,
                    None,
                )
                .await;
        }
        if let Some(agent) = self.away_agent(&caller) {
            return self
                .last_copy(
                    &caller,
                    agent,
                    Method::GetItemHistory.row(),
                    &request,
                    Self::history_boards,
                )
                .await;
        }
        let me = self.clone();
        blocking(move || me.read_history(&caller, &request))
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
