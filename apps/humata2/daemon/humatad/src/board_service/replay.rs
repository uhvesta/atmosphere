//! The host's half of a machine agent's board calls (AWAY, D227): running
//! the calls a machine forwards while it is linked, and replaying the
//! writes it journaled while the host was away.
//!
//! Both run the same service as the host's own calls, as the agent that
//! made them, under the host's rules as they are now (TP-8, AW-9). A forwarded
//! call is answered like any other call. A replayed entry is never asked
//! about: a rule that now says Ask or Never leaves the item as it is, keeps
//! the change in the item's history as lost, and the log says why. An Ask
//! the channel's While I'm away rule lists as low-risk is applied, since
//! the user let that kind through while the Mac was away, and so is one the
//! user approved on the machine, when the host's mirror of that request is
//! Approved, held by that machine, made by that session and about that item;
//! Never still leaves it out (D268). Each
//! entry moves the machine's mark in the same store transaction as its
//! board write, so a restart or a second delivery changes nothing.

use humata2_access::{BoardAct, BoardRef, Call};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{
    BoardItemChange, BoardItemRecord, BoardKey, BoardReplay, BoardReplayed, BoardWrite, LeftOut, NewBoardItem,
    StoreError,
};
use pb::invalid_field::Problem;
use prost::Message;
use tonic::{Code, Status};

use super::scope::{self, board_for_policy, by_of, column_number, made_by_session};
use super::{BoardSvc, FIRST_COLUMN, Op, history, ids, item_id, item_ids, keys, merged, page_of};
use crate::away::{Method, Placement, by_name, by_operation, placed_on};
use crate::guard::Checked;
use crate::identity::{AgentCaller, Caller};
use crate::inbox::Replayed;
use crate::status;
use crate::trash_service::{blocking, lock, now_ms};

/// What an approved request has to be about for it to stand in for the Ask
/// that stopped the entry (AW-9, D268): the host trusts the machine's
/// approval on the item, not on the field values.
enum About<'a> {
    /// A new item.
    NewItem,
    /// A change to, or the deletion of, this item.
    Item { key: &'a str, deletes: bool },
}

/// Whether the card of `request`, an approved request, is about `about`.
fn approves(request: &pb::InboxRequest, about: &About<'_>) -> bool {
    use pb::request_preview::Preview;
    match (request.preview.as_ref().and_then(|shown| shown.preview.as_ref()), about) {
        (Some(Preview::NewBoardItem(_)), About::NewItem) => true,
        (Some(Preview::BoardChange(change)), About::Item { key, deletes }) => {
            change.item.as_ref().is_some_and(|item| item.value == *key) && change.deletes_item == *deletes
        }
        _ => false,
    }
}

/// Why the host can't act as a session for a machine.
#[derive(Debug, thiserror::Error)]
enum AgentError {
    #[error(transparent)]
    Placement(#[from] Placement),

    #[error(
        "couldn't read session `{session}`'s fork details from this Mac's store; check that the disk is readable and the state folder is intact, then send it again"
    )]
    Fork {
        session: String,
        #[source]
        source: StoreError,
    },
}

/// Why the host's store couldn't take an entry: nothing moved, and the
/// machine sends it again.
#[derive(Debug, thiserror::Error)]
enum ReplayError {
    #[error(
        "couldn't save {machine}'s board journal entry {seq} in this Mac's store; check that the disk has room and the state folder is writable. Nothing after it is applied until it is saved, and Humata asks {machine} to send it again"
    )]
    Store {
        machine: String,
        seq: u64,
        #[source]
        source: StoreError,
    },

    #[error(
        "couldn't read this Mac's boards to apply {machine}'s journal entry {seq}; check that the disk is readable and the state folder is intact. Nothing after it is applied until it can, and Humata asks {machine} to send it again"
    )]
    Read {
        machine: String,
        seq: u64,
        #[source]
        source: Status,
    },
}

/// Why an entry isn't one the host can apply: it is left out and acked.
#[derive(Debug, thiserror::Error)]
enum Rejection {
    #[error("`{operation}` isn't a board write the host takes; nothing to do, the machine drops it")]
    UnknownOperation { operation: String },

    #[error("its request can't be read; update Humata on the machine so both ends speak the same version")]
    Undecodable {
        #[source]
        source: prost::DecodeError,
    },

    #[error("the session that made it can't be acted for")]
    Session {
        #[source]
        source: Placement,
    },

    #[error("item {key} isn't on this Mac's boards, so the change has nothing to apply to")]
    NoItem { key: String },

    #[error("session {session}'s board is hidden because the session is archived here; restore the session to put items on it")]
    Archived { session: String },

    #[error("it can't be applied here as it was asked; the change is dropped")]
    Unusable {
        #[source]
        source: Status,
    },

    #[error(
        "this Mac's rule for it now says Ask me, so the change isn't applied; the agent can make it again and the user is asked then"
    )]
    Asks,

    #[error(
        "this Mac's rules now refuse it, so the change isn't applied; change the channel's rule in Agent access if the agent should be able to do this"
    )]
    Refused {
        #[source]
        source: Status,
    },
}

/// What stopped planning an entry.
enum Stopped {
    /// The entry can't be applied; it is acked and dropped.
    Rejected(Rejection),
    /// The host's store failed: hold, and ask the machine for it again.
    Store(StoreError),
    /// Something of the host's own failed (a status of the internal kind).
    Failed(Status),
}

impl From<Rejection> for Stopped {
    fn from(rejection: Rejection) -> Self {
        Self::Rejected(rejection)
    }
}

/// A call or request the host's validation refused: dropped, unless it is
/// the host's own trouble.
fn unusable(status: Status) -> Stopped {
    if matches!(
        status.code(),
        Code::Internal | Code::ResourceExhausted | Code::Unavailable | Code::DataLoss
    ) {
        Stopped::Failed(status)
    } else {
        Stopped::Rejected(Rejection::Unusable { source: status })
    }
}

/// An entry worked out, with the rules as they are now.
struct Planned {
    caller: Caller,
    replay: BoardReplay,
    /// The item's revision before the entry, to tell whether it changed.
    before: u64,
    /// What stops the host's rules now (AW-9); the change is then kept in
    /// the item's history as lost, and not applied.
    stopped: Option<Rejection>,
    /// The item the entry is about, for the log; empty for a new one.
    key: String,
}

/// What became of an entry.
struct Finished {
    replayed: Replayed,
    /// The item the entry deleted: the requests about it end.
    deleted: Option<String>,
    /// The session whose summary of what was left out the entry changed
    /// (AW-9, D267): the feed is told it was updated.
    left_out: Option<String>,
}

impl Finished {
    fn of(replayed: Replayed) -> Self {
        Self {
            replayed,
            deleted: None,
            left_out: None,
        }
    }
}

/// What a write the rules now stop adds to its session's summary (AW-9,
/// D267). Only an Ask or a refusal does: `item` is the item whose history
/// keeps an edit as lost; a new item that wasn't made has none, and its
/// title is its only trace.
fn left_out_of(entry: &pb::JournalEntry, why: &Rejection, item: Option<&str>, link_up: i64) -> Option<LeftOut> {
    let refused = match why {
        Rejection::Asks => false,
        Rejection::Refused { .. } => true,
        _ => return None,
    };
    let session = entry.principal.as_ref()?.session.as_ref()?.value.clone();
    let not_made = match item {
        Some(_) => None,
        None => Some(decoded::<pb::CreateItemRequest>(entry).ok()?.title.trim().to_owned()),
    };
    Some(LeftOut {
        session,
        link_up_unix_ms: link_up,
        item: item.map(str::to_owned),
        not_made,
        refused,
        at_unix_ms: now_ms(),
    })
}

fn store_failed(machine: &str, seq: u64, source: StoreError) -> ReplayError {
    ReplayError::Store {
        machine: machine.to_owned(),
        seq,
        source,
    }
}

fn left_out(machine: &str, entry: &pb::JournalEntry, why: &Rejection) {
    crate::log::say!(
        "humatad2: warning: {machine}'s journal entry {} ({}) was left out: {}",
        entry.seq,
        entry.operation,
        status::chain(why)
    );
}

/// The store says the entry names something that isn't there: it can't be
/// applied, and never will.
fn names_nothing(error: &StoreError) -> bool {
    matches!(
        error,
        StoreError::NoBoardItem { .. }
            | StoreError::NoBoardPrefix { .. }
            | StoreError::InvalidBoardColumn { .. }
            | StoreError::InvalidId { .. }
    )
}

/// When the entry was made, in milliseconds.
fn at_ms(entry: &pb::JournalEntry) -> i64 {
    entry.at.as_ref().map_or_else(now_ms, |at| {
        at.seconds
            .saturating_mul(1000)
            .saturating_add(i64::from(at.nanos / 1_000_000))
    })
}

fn decoded<T: Message + Default>(entry: &pb::JournalEntry) -> Result<T, Rejection> {
    T::decode(entry.request.as_slice()).map_err(|source| Rejection::Undecodable { source })
}

fn request_of<T: Message + Default>(bytes: &[u8], method: &str) -> Result<T, Status> {
    T::decode(bytes).map_err(|error| {
        status::invalid_field(
            "request",
            Problem::BadFormat,
            format!("the request for {method} can't be read: {error}"),
            "update Humata on the machine so both ends speak the same version",
        )
    })
}

impl BoardSvc {
    // ----------------------------------------------------------- reads

    /// Item `id` as `caller` sees it.
    pub(super) fn read_item(
        &self,
        caller: &Caller,
        id: &str,
        asks: &[pb::InboxRequest],
    ) -> Result<pb::BoardItem, Status> {
        let item = self.load(caller, id, false)?;
        let archived = self.archived()?;
        Ok(self.item_pb(caller, &item, asks, &archived))
    }

    /// A page of an item's history as `caller` sees it.
    pub(super) fn read_history(
        &self,
        caller: &Caller,
        request: &pb::GetItemHistoryRequest,
    ) -> Result<pb::GetItemHistoryResponse, Status> {
        let id = item_id(request.id.as_ref())?;
        let (size, after) = page_of(request.page.as_ref())?;
        let item = self.load(caller, &id, true)?;
        let mut found = self
            .store
            .board_item_history(&item.key, after as u64, size + 1)
            .map_err(|error| self.failed(error, "read the item's history"))?;
        let more = found.len() > size;
        found.truncate(size);
        let next_page_token = match (more, found.last()) {
            (true, Some(last)) => last.n.to_string(),
            _ => String::new(),
        };
        Ok(pb::GetItemHistoryResponse {
            entries: found
                .iter()
                .map(|entry| history::entry_pb(entry, self.actor_of(&entry.by)))
                .collect(),
            page: Some(pb::PageResponse { next_page_token }),
        })
    }

    // ------------------------------------------------ forwarded calls

    /// The agent `session` of `machine` as the caller of `method`: the
    /// session must be one this Mac places on that machine.
    fn agent_caller(&self, machine: &str, session: &str, method: &str) -> Result<Caller, AgentError> {
        let record = placed_on(&self.store, machine, session)?;
        let mut agent = AgentCaller::of(&record, &self.machine)
            .resolve_fork(&self.store)
            .map_err(|source| AgentError::Fork {
                session: session.to_owned(),
                source,
            })?;
        method.clone_into(&mut agent.method);
        Ok(Caller::Agent(agent))
    }

    /// Runs a call `machine` forwarded for one of its agents, as that agent
    /// under this Mac's rules, and answers it.
    pub(crate) async fn forwarded(&self, machine: &str, call: pb::ForwardedCall) -> pb::ForwardedResult {
        let outcome = match self.run_forwarded(machine, &call).await {
            Ok(response) => pb::forwarded_result::Outcome::Response(response),
            Err(failed) => pb::forwarded_result::Outcome::Error(status::error_of(&failed).unwrap_or_else(|| {
                pb::Error {
                    message: failed.message().to_owned(),
                    fix: String::new(),
                    retry: None,
                    kind: None,
                }
            })),
        };
        pb::ForwardedResult {
            call_id: call.call_id,
            outcome: Some(outcome),
        }
    }

    async fn run_forwarded(&self, machine: &str, call: &pb::ForwardedCall) -> Result<Vec<u8>, Status> {
        let Some(row) = by_name(&call.method) else {
            return Err(status::not_built(&call.method));
        };
        let Some(pb::forwarded_call::Principal::Agent(principal)) = &call.principal else {
            return Err(status::not_built(&call.method));
        };
        let session = principal
            .session
            .as_ref()
            .map(|session| session.value.clone())
            .unwrap_or_default();
        let (me, name, method) = (self.clone(), machine.to_owned(), call.method.clone());
        let caller = blocking(move || me.agent_caller(&name, &session, &method))
            .await?
            .map_err(|error| match error {
                AgentError::Placement(placement) => placement.status(),
                AgentError::Fork { source, .. } => status::store(source, &self.machine, "read the session"),
            })?;
        let bytes = call.request.as_slice();
        let me = self.clone();
        match row.method {
            Method::ListItems => {
                let request: pb::ListItemsRequest = request_of(bytes, &call.method)?;
                let list = blocking(move || me.list(&caller, &request, &[])).await??;
                Ok(list.encode_to_vec())
            }
            Method::GetItem => {
                let request: pb::GetItemRequest = request_of(bytes, &call.method)?;
                let id = item_id(request.id.as_ref())?;
                let item = blocking(move || me.read_item(&caller, &id, &[])).await??;
                Ok(item.encode_to_vec())
            }
            Method::GetItemHistory => {
                let request: pb::GetItemHistoryRequest = request_of(bytes, &call.method)?;
                let page = blocking(move || me.read_history(&caller, &request)).await??;
                Ok(page.encode_to_vec())
            }
            Method::CreateItem => {
                let request: pb::CreateItemRequest = request_of(bytes, &call.method)?;
                Ok(me.write(caller, Op::Create(request)).await?.into_inner().encode_to_vec())
            }
            Method::UpdateItem => {
                let request: pb::UpdateItemRequest = request_of(bytes, &call.method)?;
                Ok(me.write(caller, Op::Update(request)).await?.into_inner().encode_to_vec())
            }
            Method::MoveItem => {
                let request: pb::MoveItemRequest = request_of(bytes, &call.method)?;
                Ok(me.write(caller, Op::Move(request)).await?.into_inner().encode_to_vec())
            }
            Method::DeleteItem => {
                let request: pb::DeleteItemRequest = request_of(bytes, &call.method)?;
                Ok(me.write(caller, Op::Delete(request)).await?.into_inner().encode_to_vec())
            }
            // The agent's wait on a request an Ask of its made here, as that
            // agent: only its own session's requests are found (D268).
            Method::WaitForAnswer => {
                let request: pb::WaitForAnswerRequest = request_of(bytes, &call.method)?;
                Ok(me.inbox.wait_final(caller, request).await?.encode_to_vec())
            }
            Method::WithdrawRequest => {
                let request: pb::WithdrawRequestRequest = request_of(bytes, &call.method)?;
                Ok(me.inbox.withdraw_as(caller, request).await?.encode_to_vec())
            }
        }
    }

    // --------------------------------------------------------- replay

    /// Applies `machine`'s journaled board write `entry`. Entries are
    /// applied in the order they come; one at or below the machine's mark
    /// was applied before and changes nothing. `link_up_unix_ms` is when the
    /// link that carried it came up: what the rules leave out of one link-up's
    /// entries is summed on the session, and a later link-up's replaces it
    /// (AW-9, D267).
    pub(crate) async fn replay_at(&self, machine: &str, entry: &pb::JournalEntry, link_up_unix_ms: i64) -> Replayed {
        let approval = self.approval_of(machine, entry).await;
        let (me, name, taken) = (self.clone(), machine.to_owned(), entry.clone());
        match blocking(move || me.replay_entry(&name, &taken, approval.as_ref(), link_up_unix_ms)).await {
            Ok(Ok(finished)) => {
                if let Some(key) = &finished.deleted
                    && let Err(failed) = self.inbox.withdraw_about_item(key).await
                {
                    crate::log::say!(
                        "humatad2: couldn't end the requests about deleted item {key}: {}",
                        failed.message()
                    );
                }
                if let Some(session) = &finished.left_out
                    && let Some(tell) = &self.told
                {
                    tell.session(session, pb::Change::Updated, None).await;
                }
                finished.replayed
            }
            Ok(Err(error)) => {
                crate::log::say!("humatad2: warning: {}", status::chain(&error));
                Replayed::Held
            }
            Err(failed) => {
                crate::log::say!(
                    "humatad2: warning: applying {machine}'s board journal entry {} stopped: {}; Humata asks {machine} to send it again",
                    entry.seq,
                    failed.message()
                );
                Replayed::Held
            }
        }
    }

    /// The host's mirror of the request `entry` says the user approved, when
    /// it is `machine`'s, made by the entry's session, and Approved (AW-9,
    /// D268). An entry that names one that isn't is judged without it, and
    /// the log says so.
    async fn approval_of(&self, machine: &str, entry: &pb::JournalEntry) -> Option<pb::InboxRequest> {
        let id = entry.approved_request.as_ref()?;
        let session = entry
            .principal
            .as_ref()
            .and_then(|principal| principal.session.as_ref())
            .map(|session| session.value.as_str())
            .unwrap_or_default();
        let found = self.inbox.approved_on(machine, &id.value, session).await;
        if found.is_none() {
            crate::log::say!(
                "humatad2: {machine}'s journal entry {} ({}) names request {} as its approval, but this Mac's Inbox doesn't show it approved by that machine for that session, so the rules judge the entry alone",
                entry.seq,
                entry.operation,
                id.value
            );
        }
        found
    }

    fn replay_entry(
        &self,
        machine: &str,
        entry: &pb::JournalEntry,
        approval: Option<&pb::InboxRequest>,
        link_up: i64,
    ) -> Result<Finished, ReplayError> {
        let seq = entry.seq;
        let at_store = |source| store_failed(machine, seq, source);
        // Held while deciding and saving, so an entry and a call on the host
        // can't interleave on one item.
        let _write = lock(&self.writes);
        if seq <= self.store.machine_journal_mark(machine).map_err(at_store)? {
            return Ok(Finished::of(Replayed::Applied));
        }
        let planned = match self.plan(machine, entry, approval) {
            Ok(planned) => planned,
            Err(Stopped::Rejected(why)) => {
                left_out(machine, entry, &why);
                // A new item the rules now stop leaves nothing in any history:
                // the session's summary is its only trace (AW-9, D267).
                let noted = left_out_of(entry, &why, None, link_up);
                let saved = match &noted {
                    Some(noted) => self.store.apply_machine_journal_left_out(machine, seq, noted),
                    None => self.store.apply_machine_journal(machine, seq, None),
                };
                saved.map_err(at_store)?;
                let mut finished = Finished::of(Replayed::Rejected);
                finished.left_out = noted.map(|noted| noted.session);
                return Ok(finished);
            }
            Err(Stopped::Store(source)) => return Err(store_failed(machine, seq, source)),
            Err(Stopped::Failed(source)) => {
                return Err(ReplayError::Read {
                    machine: machine.to_owned(),
                    seq,
                    source,
                });
            }
        };
        if let Some(why) = &planned.stopped {
            crate::log::say!(
                "humatad2: {machine}'s journal entry {} ({}) on {} wasn't applied and is kept in its history as lost: {}",
                seq,
                entry.operation,
                planned.key,
                status::chain(why)
            );
        }
        let noted = planned
            .stopped
            .as_ref()
            .and_then(|why| left_out_of(entry, why, Some(planned.key.as_str()), link_up));
        match self
            .store
            .replay_board_entry_noting(machine, seq, &planned.replay, noted.as_ref())
        {
            Ok(done) => {
                let mut finished = self.settled(machine, entry, &planned, done);
                finished.left_out = noted.map(|noted| noted.session);
                Ok(finished)
            }
            Err(error) if names_nothing(&error) => {
                left_out(
                    machine,
                    entry,
                    &Rejection::NoItem {
                        key: planned.key.clone(),
                    },
                );
                self.store.apply_machine_journal(machine, seq, None).map_err(at_store)?;
                Ok(Finished::of(Replayed::Rejected))
            }
            Err(source) => Err(store_failed(machine, seq, source)),
        }
    }

    /// Tells the feed what an applied entry did, and logs what it didn't.
    fn settled(&self, machine: &str, entry: &pb::JournalEntry, planned: &Planned, done: BoardReplayed) -> Finished {
        let archived = self.archived().unwrap_or_default();
        let kept_lost = |what: &str| {
            crate::log::say!(
                "humatad2: {machine}'s journal entry {} ({}) on {}: {what}",
                entry.seq,
                entry.operation,
                planned.key
            );
        };
        match done {
            BoardReplayed::Created(item) => {
                self.tell(&planned.caller, pb::Change::Created, &item, &archived);
                Finished::of(Replayed::Applied)
            }
            BoardReplayed::Written(BoardWrite::Applied(item)) => {
                if item.revision != planned.before {
                    let change = if item.deleted {
                        pb::Change::Deleted
                    } else {
                        pb::Change::Updated
                    };
                    self.tell(&planned.caller, change, &item, &archived);
                }
                Finished {
                    replayed: Replayed::Applied,
                    deleted: item.deleted.then(|| item.key.clone()),
                    left_out: None,
                }
            }
            BoardReplayed::Written(BoardWrite::Stale { .. }) => {
                kept_lost("the item changed since the agent saw it, so the delete lost and is kept in its history as lost");
                Finished::of(Replayed::Applied)
            }
            BoardReplayed::Written(BoardWrite::Gone { .. }) => {
                kept_lost("the item was deleted here; the change is kept in its history as lost, and the item stays deleted");
                Finished::of(Replayed::Applied)
            }
            BoardReplayed::Written(BoardWrite::Loop { cycle }) => {
                kept_lost(&format!(
                    "the new dependencies would close a loop ({}); nothing was changed",
                    cycle.join(" -> ")
                ));
                Finished::of(Replayed::Applied)
            }
            BoardReplayed::Written(BoardWrite::MissingDependency { key } | BoardWrite::DeletedDependency { key }) => {
                kept_lost(&format!("it depends on {key}, which is gone here; nothing was changed"));
                Finished::of(Replayed::Applied)
            }
            BoardReplayed::Refused | BoardReplayed::AlreadyApplied => Finished::of(Replayed::Applied),
        }
    }

    /// Works out what `entry` does on this Mac, and whether the rules as
    /// they are now let the agent do it.
    fn plan(&self, machine: &str, entry: &pb::JournalEntry, approval: Option<&pb::InboxRequest>) -> Result<Planned, Stopped> {
        let row = by_operation(&entry.operation).ok_or_else(|| Rejection::UnknownOperation {
            operation: entry.operation.clone(),
        })?;
        let session = entry
            .principal
            .as_ref()
            .and_then(|principal| principal.session.as_ref())
            .map(|session| session.value.as_str())
            .unwrap_or_default();
        let caller = match self.agent_caller(machine, session, row.name) {
            Ok(caller) => caller,
            Err(AgentError::Placement(Placement::Store { source, .. }) | AgentError::Fork { source, .. }) => {
                return Err(Stopped::Store(source));
            }
            Err(AgentError::Placement(source)) => return Err(Rejection::Session { source }.into()),
        };
        let expected = entry.base_revision.as_ref().map(|revision| revision.value);
        let at = at_ms(entry);
        match row.method {
            Method::CreateItem => self.plan_create(caller, &decoded::<pb::CreateItemRequest>(entry)?, at, approval),
            Method::UpdateItem => {
                self.plan_update(caller, &decoded::<pb::UpdateItemRequest>(entry)?, expected, at, approval)
            }
            Method::MoveItem => self.plan_move(caller, &decoded::<pb::MoveItemRequest>(entry)?, expected, at, approval),
            Method::DeleteItem => {
                self.plan_delete(caller, &decoded::<pb::DeleteItemRequest>(entry)?, expected, at, approval)
            }
            Method::ListItems
            | Method::GetItem
            | Method::GetItemHistory
            | Method::WaitForAnswer
            | Method::WithdrawRequest => Err(Rejection::UnknownOperation {
                operation: entry.operation.clone(),
            }
            .into()),
        }
    }

    /// Whether the rules now let `caller` do `act` on `board`. An Ask stands
    /// when `approval`, the request the user approved on the machine, is
    /// about `about`; Never never does.
    fn allowed(
        &self,
        caller: &Caller,
        board: BoardRef,
        act: BoardAct,
        approval: Option<&pb::InboxRequest>,
        about: &About<'_>,
    ) -> Result<(), Rejection> {
        match self.guard.check(caller, &Call::Board { board, act }) {
            Ok(Checked::Allowed) => Ok(()),
            // A kind the While I'm away rule lists as low-risk was let through
            // for the time the Mac was away: that is when this was made (D268).
            Ok(Checked::Ask(asked)) if self.guard.lists_low_risk(&asked) => Ok(()),
            Ok(Checked::Ask(_)) if approval.is_some_and(|request| approves(request, about)) => Ok(()),
            Ok(Checked::Ask(_)) => Err(Rejection::Asks),
            Err(source) => Err(Rejection::Refused { source }),
        }
    }

    fn item_for_replay(&self, id: &str) -> Result<BoardItemRecord, Stopped> {
        match self.store.board_item(id) {
            Ok(Some(item)) => Ok(item),
            Ok(None) | Err(StoreError::InvalidId { .. }) => Err(Rejection::NoItem { key: id.to_owned() }.into()),
            Err(source) => Err(Stopped::Store(source)),
        }
    }

    /// An edit of `item`: applied when the rules allow it, kept as lost
    /// when they don't (AW-9). A deleted item takes no check: the store
    /// keeps the change as lost and never revives it (SL-8).
    fn judged(
        &self,
        caller: Caller,
        item: &BoardItemRecord,
        change: BoardItemChange,
        approval: Option<&pb::InboxRequest>,
    ) -> Planned {
        let act = BoardAct::Edit {
            created_by: made_by_session(&item.created_by),
        };
        let stopped = if item.deleted {
            None
        } else {
            let about = About::Item {
                key: &item.key,
                deletes: change.delete,
            };
            self.allowed(&caller, self.policy_of(item), act, approval, &about).err()
        };
        let replay = if stopped.is_some() {
            BoardReplay::Refused(change)
        } else {
            BoardReplay::Change(change)
        };
        Planned {
            caller,
            replay,
            before: item.revision,
            stopped,
            key: item.key.clone(),
        }
    }

    fn plan_create(
        &self,
        caller: Caller,
        request: &pb::CreateItemRequest,
        at: i64,
        approval: Option<&pb::InboxRequest>,
    ) -> Result<Planned, Stopped> {
        scope::refuse_assigned_in_create(request).map_err(unusable)?;
        let title = request.title.trim();
        if title.is_empty() {
            return Err(unusable(status::missing("title")));
        }
        let column = match request.column {
            0 => FIRST_COLUMN,
            column => column_number(column, "column").map_err(unusable)?,
        };
        let board = request
            .board
            .as_ref()
            .ok_or_else(|| unusable(status::missing("board")))?;
        let (key, channel, channel_name) = self.resolve_board(board).map_err(unusable)?;
        if let BoardKey::Session(session) = &key {
            match self.store.archive(session) {
                Ok(None) => {}
                Ok(Some(_)) => {
                    return Err(Rejection::Archived {
                        session: session.clone(),
                    }
                    .into());
                }
                Err(source) => return Err(Stopped::Store(source)),
            }
        }
        let sessions = self.known_sessions(&request.sessions, "sessions").map_err(unusable)?;
        let depends_on = self.readable_targets(&caller, &request.depends_on).map_err(unusable)?;
        self.allowed(
            &caller,
            board_for_policy(&key, &channel, &self.machine),
            BoardAct::Add,
            approval,
            &About::NewItem,
        )?;
        let owner = match &key {
            BoardKey::Global => keys::GLOBAL_OWNER.to_owned(),
            _ => channel.clone(),
        };
        keys::prefix_of(&self.store, &owner, &channel_name).map_err(Stopped::Store)?;
        let rank = self.rank_in(&key, column, "", None).map_err(unusable)?;
        let created_by = by_of(&caller);
        Ok(Planned {
            caller,
            replay: BoardReplay::Create(NewBoardItem {
                board: key,
                channel,
                title: title.to_owned(),
                description: request.description.clone(),
                column,
                rank,
                created_by,
                at_unix_ms: at,
                sessions,
                depends_on,
            }),
            before: 0,
            stopped: None,
            key: String::new(),
        })
    }

    fn plan_update(
        &self,
        caller: Caller,
        request: &pb::UpdateItemRequest,
        expected: Option<u64>,
        at: i64,
        approval: Option<&pb::InboxRequest>,
    ) -> Result<Planned, Stopped> {
        let id = item_id(request.id.as_ref()).map_err(unusable)?;
        let patch = request.patch.clone().unwrap_or_default();
        scope::refuse_assigned_in_patch(&patch).map_err(unusable)?;
        if patch.title.as_deref().is_some_and(|title| title.trim().is_empty()) {
            return Err(unusable(status::missing("patch.title")));
        }
        let item = self.item_for_replay(&id)?;
        let add = self
            .known_sessions(&patch.sessions_add, "patch.sessions_add")
            .map_err(unusable)?;
        let remove = ids(&patch.sessions_remove);
        let deps_add = self
            .readable_targets(&caller, &patch.depends_on_add)
            .map_err(unusable)?;
        let deps_remove = item_ids(&patch.depends_on_remove);
        let sessions = merged(&item.sessions, &add, &remove);
        let deps = merged(&item.depends_on, &deps_add, &deps_remove);
        let change = BoardItemChange {
            key: id,
            expected_revision: expected,
            by: by_of(&caller),
            at_unix_ms: at,
            title: patch.title.filter(|title| *title != item.title),
            description: patch.description.filter(|text| *text != item.description),
            column: None,
            rank: None,
            sessions: (sessions != item.sessions).then_some(sessions),
            depends_on: (deps != item.depends_on).then_some(deps),
            delete: false,
            cause: String::new(),
        };
        Ok(self.judged(caller, &item, change, approval))
    }

    fn plan_move(
        &self,
        caller: Caller,
        request: &pb::MoveItemRequest,
        expected: Option<u64>,
        at: i64,
        approval: Option<&pb::InboxRequest>,
    ) -> Result<Planned, Stopped> {
        let id = item_id(request.id.as_ref()).map_err(unusable)?;
        let column = column_number(request.column, "column").map_err(unusable)?;
        let after = request
            .after
            .as_ref()
            .map(|after| after.value.as_str())
            .filter(|after| !after.is_empty());
        let item = self.item_for_replay(&id)?;
        let rank = self.rank_in(&item.board, column, &item.key, after).map_err(unusable)?;
        let change = BoardItemChange {
            key: id,
            expected_revision: expected,
            by: by_of(&caller),
            at_unix_ms: at,
            title: None,
            description: None,
            column: (item.column != column).then_some(column),
            rank: Some(rank),
            sessions: None,
            depends_on: None,
            delete: false,
            cause: String::new(),
        };
        Ok(self.judged(caller, &item, change, approval))
    }

    fn plan_delete(
        &self,
        caller: Caller,
        request: &pb::DeleteItemRequest,
        expected: Option<u64>,
        at: i64,
        approval: Option<&pb::InboxRequest>,
    ) -> Result<Planned, Stopped> {
        let id = item_id(request.id.as_ref()).map_err(unusable)?;
        let item = self.item_for_replay(&id)?;
        let change = BoardItemChange {
            key: id,
            expected_revision: expected,
            by: by_of(&caller),
            at_unix_ms: at,
            title: None,
            description: None,
            column: None,
            rank: None,
            sessions: None,
            depends_on: None,
            delete: true,
            cause: String::new(),
        };
        Ok(self.judged(caller, &item, change, approval))
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use humata2_access::{AwayRule, Capability, ChannelId, Decision, Principal, Rules};
    use humata2_store::{SessionRecord, Store};

    use super::*;
    use crate::away::{JOURNAL_ANSWER, JOURNAL_REQUEST};
    use crate::events::Feed;
    use crate::guard::Guard;
    use crate::inbox::Inbox;

    const HOST: &str = "ma_host";
    const BOX: &str = "ma_box";
    const AGENT: &str = "se_a";

    fn session(id: &str, machine: &str) -> SessionRecord {
        SessionRecord {
            id: id.to_owned(),
            channel: "ch_a".to_owned(),
            machine: machine.to_owned(),
            title: "agent".to_owned(),
            agent: "claude".to_owned(),
            folder: PathBuf::new(),
            created_unix_ms: 0,
        }
    }

    struct Host {
        _directory: tempfile::TempDir,
        store: Arc<Store>,
        guard: Arc<Guard>,
        inbox: Arc<Inbox>,
        boards: BoardSvc,
    }

    /// A host that places `AGENT` on `BOX` and another session elsewhere,
    /// with the agent's own session board to write to.
    fn host() -> Host {
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(directory.path()).unwrap());
        store.put_session(&session(AGENT, BOX)).unwrap();
        store.put_session(&session("se_far", "ma_other")).unwrap();
        store.claim_board_prefix("ch_a", "PAY").unwrap();
        let guard = Arc::new(Guard::new(HOST, pb::DaemonRole::Host));
        guard.with_policy(|policy, _| {
            let _ = policy.add_channel(ChannelId::new("ch_a"));
        });
        let feed = Arc::new(Feed::default());
        let inbox = Arc::new(Inbox::new(
            Arc::clone(&guard),
            Arc::clone(&store),
            HOST.to_owned(),
            Arc::clone(&feed),
        ));
        let boards = BoardSvc::new(Arc::clone(&guard), Arc::clone(&store), feed, Arc::clone(&inbox), HOST.to_owned());
        Host {
            _directory: directory,
            store,
            guard,
            inbox,
            boards,
        }
    }

    /// The channel's rule for the agent changing its own session board.
    fn rule(host: &Host, decision: Decision) {
        host.guard.with_policy(|policy, context| {
            let user = Principal::App.present_user(context).unwrap();
            policy
                .set_rule(
                    &user,
                    &Rules::Channel(ChannelId::new("ch_a")),
                    Capability::OwnBoardWrite,
                    decision,
                )
                .unwrap();
        });
    }

    /// The channel's While I'm away rule: allow `kinds` as low-risk.
    fn low_risk(host: &Host, kinds: &[Capability]) {
        host.guard.with_policy(|policy, context| {
            let user = Principal::App.present_user(context).unwrap();
            policy
                .set_away_rule(
                    &user,
                    &Rules::Channel(ChannelId::new("ch_a")),
                    AwayRule::AllowLowRisk(kinds.iter().copied().collect()),
                )
                .unwrap();
        });
    }

    fn entry(seq: u64, operation: &str, session: &str, request: Vec<u8>) -> pb::JournalEntry {
        pb::JournalEntry {
            seq,
            principal: Some(pb::AgentPrincipal {
                session: Some(pb::SessionId {
                    value: session.to_owned(),
                }),
                pid: 0,
            }),
            operation: operation.to_owned(),
            request,
            ..Default::default()
        }
    }

    fn create(seq: u64, title: &str) -> pb::JournalEntry {
        entry(
            seq,
            "board.item_create",
            AGENT,
            pb::CreateItemRequest {
                board: Some(pb::BoardRef {
                    board: Some(pb::board_ref::Board::Session(pb::SessionId {
                        value: AGENT.to_owned(),
                    })),
                }),
                title: title.to_owned(),
                ..Default::default()
            }
            .encode_to_vec(),
        )
    }

    fn retitle(seq: u64, key: &str, title: &str) -> pb::JournalEntry {
        entry(
            seq,
            "board.item_update",
            AGENT,
            pb::UpdateItemRequest {
                id: Some(pb::BoardItemId { value: key.to_owned() }),
                patch: Some(pb::ItemPatch {
                    title: Some(title.to_owned()),
                    ..Default::default()
                }),
                ..Default::default()
            }
            .encode_to_vec(),
        )
    }

    fn item(host: &Host, key: &str) -> BoardItemRecord {
        host.store.board_item(key).unwrap().unwrap()
    }

    /// The link-up the tests that don't care about it replay under.
    const LINK_UP: i64 = 1_000;

    impl BoardSvc {
        async fn replay(&self, machine: &str, entry: &pb::JournalEntry) -> Replayed {
            self.replay_at(machine, entry, LINK_UP).await
        }
    }

    #[tokio::test]
    async fn entries_apply_in_the_order_they_come_and_move_the_mark() {
        let host = host();
        assert_eq!(host.boards.replay(BOX, &create(1, "first")).await, Replayed::Applied);
        assert_eq!(host.boards.replay(BOX, &retitle(2, "PAY-1", "second")).await, Replayed::Applied);
        assert_eq!(host.boards.replay(BOX, &retitle(3, "PAY-1", "third")).await, Replayed::Applied);
        let kept = item(&host, "PAY-1");
        assert_eq!(kept.title, "third");
        assert_eq!(kept.revision, 3);
        assert_eq!(kept.created_by, AGENT);
        assert_eq!(host.store.machine_journal_mark(BOX).unwrap(), 3);
    }

    #[tokio::test]
    async fn an_entry_delivered_twice_changes_nothing() {
        let host = host();
        host.boards.replay(BOX, &create(1, "first")).await;
        assert_eq!(host.boards.replay(BOX, &create(1, "first")).await, Replayed::Applied);
        assert_eq!(host.store.board_items(&[BoardKey::Session(AGENT.to_owned())]).unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_rule_now_set_to_ask_leaves_the_item_alone_and_keeps_the_change_as_lost() {
        let host = host();
        host.boards.replay(BOX, &create(1, "first")).await;
        rule(&host, Decision::Ask);
        assert_eq!(host.boards.replay(BOX, &retitle(2, "PAY-1", "changed")).await, Replayed::Applied);
        let kept = item(&host, "PAY-1");
        assert_eq!((kept.title.as_str(), kept.revision), ("first", 1));
        let history = host.store.board_item_history("PAY-1", 0, 10).unwrap();
        let lost = history.last().unwrap();
        assert!(lost.lost);
        assert_eq!((lost.field.as_str(), lost.before.as_str(), lost.after.as_str()), ("title", "first", "changed"));
        assert_eq!(host.store.machine_journal_mark(BOX).unwrap(), 2);
    }

    #[tokio::test]
    async fn a_rule_now_set_to_never_leaves_the_item_alone_and_keeps_the_change_as_lost() {
        let host = host();
        host.boards.replay(BOX, &create(1, "first")).await;
        rule(&host, Decision::Never);
        assert_eq!(host.boards.replay(BOX, &retitle(2, "PAY-1", "changed")).await, Replayed::Applied);
        assert_eq!(item(&host, "PAY-1").title, "first");
        assert!(host.store.board_item_history("PAY-1", 0, 10).unwrap().last().unwrap().lost);
        assert_eq!(host.store.machine_journal_mark(BOX).unwrap(), 2);
    }

    #[tokio::test]
    async fn a_create_the_rules_now_refuse_makes_no_item_and_moves_the_mark() {
        let host = host();
        rule(&host, Decision::Never);
        assert_eq!(host.boards.replay(BOX, &create(1, "first")).await, Replayed::Rejected);
        assert!(host.store.board_items(&[BoardKey::Session(AGENT.to_owned())]).unwrap().is_empty());
        assert_eq!(host.store.machine_journal_mark(BOX).unwrap(), 1);
    }

    #[tokio::test]
    async fn an_entry_for_a_session_placed_elsewhere_is_left_out_and_acked() {
        let host = host();
        let from_far = entry(1, "board.item_create", "se_far", Vec::new());
        assert_eq!(host.boards.replay(BOX, &from_far).await, Replayed::Rejected);
        let unknown = entry(2, "board.item_create", "se_missing", Vec::new());
        assert_eq!(host.boards.replay(BOX, &unknown).await, Replayed::Rejected);
        assert_eq!(host.store.machine_journal_mark(BOX).unwrap(), 2);
    }

    #[tokio::test]
    async fn an_operation_that_isnt_a_board_write_is_left_out_and_acked() {
        let host = host();
        let other = entry(1, "board.comment_add", AGENT, Vec::new());
        assert_eq!(host.boards.replay(BOX, &other).await, Replayed::Rejected);
        let read = entry(2, "board.item_list", AGENT, Vec::new());
        assert_eq!(host.boards.replay(BOX, &read).await, Replayed::Rejected);
        assert_eq!(host.store.machine_journal_mark(BOX).unwrap(), 2);
    }

    #[tokio::test]
    async fn a_request_that_cant_be_read_is_left_out_and_acked() {
        let host = host();
        let garbled = entry(1, "board.item_create", AGENT, vec![0xff, 0xff, 0xff]);
        assert_eq!(host.boards.replay(BOX, &garbled).await, Replayed::Rejected);
        assert_eq!(host.store.machine_journal_mark(BOX).unwrap(), 1);
    }

    #[tokio::test]
    async fn an_edit_of_an_item_that_does_not_exist_is_left_out_and_acked() {
        let host = host();
        assert_eq!(host.boards.replay(BOX, &retitle(1, "PAY-9", "x")).await, Replayed::Rejected);
        assert_eq!(host.store.machine_journal_mark(BOX).unwrap(), 1);
    }

    #[tokio::test]
    async fn a_conflicting_edit_replays_over_the_hosts_and_keeps_the_hosts_value_as_lost() {
        let host = host();
        host.boards.replay(BOX, &create(1, "first")).await;
        // The host's user changes the title while the machine is away.
        host.store
            .change_board_item(&BoardItemChange {
                key: "PAY-1".to_owned(),
                expected_revision: None,
                by: String::new(),
                at_unix_ms: 5,
                title: Some("host".to_owned()),
                description: None,
                column: None,
                rank: None,
                sessions: None,
                depends_on: None,
                delete: false,
                cause: String::new(),
            })
            .unwrap();
        let mut stale = retitle(2, "PAY-1", "agent");
        stale.base_revision = Some(pb::Revision { value: 1 });
        assert_eq!(host.boards.replay(BOX, &stale).await, Replayed::Applied);
        assert_eq!(item(&host, "PAY-1").title, "agent");
        let lost: Vec<_> = host
            .store
            .board_item_history("PAY-1", 0, 10)
            .unwrap()
            .into_iter()
            .filter(|entry| entry.lost)
            .collect();
        assert_eq!(lost.len(), 1);
        assert_eq!((lost[0].before.as_str(), lost[0].after.as_str()), ("agent", "host"));
    }

    fn forwarded_create(session: &str, call_id: u64) -> pb::ForwardedCall {
        pb::ForwardedCall {
            call_id,
            principal: Some(pb::forwarded_call::Principal::Agent(pb::AgentPrincipal {
                session: Some(pb::SessionId {
                    value: session.to_owned(),
                }),
                pid: 0,
            })),
            method: "/humata.v1.BoardService/CreateItem".to_owned(),
            request: pb::CreateItemRequest {
                board: Some(pb::BoardRef {
                    board: Some(pb::board_ref::Board::Session(pb::SessionId {
                        value: AGENT.to_owned(),
                    })),
                }),
                title: "forwarded".to_owned(),
                ..Default::default()
            }
            .encode_to_vec(),
            deadline: None,
            idempotency_key: None,
        }
    }

    #[tokio::test]
    async fn a_forwarded_create_from_a_session_on_that_machine_runs_as_that_agent() {
        let host = host();
        let result = host.boards.forwarded(BOX, forwarded_create(AGENT, 7)).await;
        assert_eq!(result.call_id, 7);
        let Some(pb::forwarded_result::Outcome::Response(bytes)) = result.outcome else {
            panic!("the host answers with the response: {result:?}");
        };
        let response = pb::WriteItemResponse::decode(bytes.as_slice()).unwrap();
        assert_eq!(
            response.item.and_then(|item| item.id).map(|id| id.value).as_deref(),
            Some("PAY-1")
        );
        assert_eq!(item(&host, "PAY-1").created_by, AGENT);
    }

    #[tokio::test]
    async fn a_forwarded_call_for_a_session_the_host_places_elsewhere_is_refused_and_changes_nothing() {
        let host = host();
        let result = host.boards.forwarded(BOX, forwarded_create("se_far", 1)).await;
        let Some(pb::forwarded_result::Outcome::Error(error)) = result.outcome else {
            panic!("the host refuses: {result:?}");
        };
        assert!(matches!(error.kind, Some(pb::error::Kind::IdentityMismatch(_))));
        let unknown = host.boards.forwarded(BOX, forwarded_create("se_missing", 2)).await;
        assert!(matches!(unknown.outcome, Some(pb::forwarded_result::Outcome::Error(_))));
        assert!(host.store.board_items(&[BoardKey::Session(AGENT.to_owned())]).unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_forwarded_call_the_hosts_rules_refuse_is_answered_with_the_hosts_error() {
        let host = host();
        rule(&host, Decision::Never);
        let result = host.boards.forwarded(BOX, forwarded_create(AGENT, 3)).await;
        assert!(matches!(result.outcome, Some(pb::forwarded_result::Outcome::Error(_))));
        assert!(host.store.board_items(&[BoardKey::Session(AGENT.to_owned())]).unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_forwarded_method_the_host_doesnt_forward_is_refused() {
        let host = host();
        let mut call = forwarded_create(AGENT, 4);
        call.method = "/humata.v1.BoardService/Elsewhere".to_owned();
        let result = host.boards.forwarded(BOX, call).await;
        assert!(matches!(result.outcome, Some(pb::forwarded_result::Outcome::Error(_))));
    }

    /// A write the rules now ask about, which the While I'm away rule
    /// listed as low-risk when the entry was made, is applied (D268).
    #[tokio::test]
    async fn an_ask_the_away_rule_lists_as_low_risk_is_applied() {
        let host = host();
        host.boards.replay(BOX, &create(1, "first")).await;
        rule(&host, Decision::Ask);
        low_risk(&host, &[Capability::OwnBoardWrite]);
        assert_eq!(host.boards.replay(BOX, &retitle(2, "PAY-1", "changed")).await, Replayed::Applied);
        let kept = item(&host, "PAY-1");
        assert_eq!((kept.title.as_str(), kept.revision), ("changed", 2));
        assert_eq!(left_out_of_agent(&host), None);
    }

    #[tokio::test]
    async fn an_ask_the_away_rule_no_longer_lists_is_left_out_as_asked() {
        let host = host();
        host.boards.replay(BOX, &create(1, "first")).await;
        rule(&host, Decision::Ask);
        low_risk(&host, &[Capability::OwnBoardWrite]);
        low_risk(&host, &[Capability::SessionNote]);
        host.boards.replay(BOX, &retitle(2, "PAY-1", "changed")).await;
        assert_eq!(item(&host, "PAY-1").title, "first");
        let kept = left_out_of_agent(&host).expect("the edit was left out");
        assert!(!kept.refused, "Ask me, not a refusal");
    }

    #[tokio::test]
    async fn a_create_under_an_ask_the_away_rule_lists_is_made() {
        let host = host();
        rule(&host, Decision::Ask);
        low_risk(&host, &[Capability::OwnBoardWrite]);
        assert_eq!(host.boards.replay(BOX, &create(1, "first")).await, Replayed::Applied);
        assert_eq!(item(&host, "PAY-1").title, "first");
    }

    #[tokio::test]
    async fn listing_a_kind_as_low_risk_doesnt_override_never() {
        let host = host();
        host.boards.replay(BOX, &create(1, "first")).await;
        rule(&host, Decision::Never);
        low_risk(&host, &[Capability::OwnBoardWrite]);
        host.boards.replay(BOX, &retitle(2, "PAY-1", "changed")).await;
        assert_eq!(item(&host, "PAY-1").title, "first");
        assert!(left_out_of_agent(&host).expect("the edit was left out").refused);
    }

    fn forwarded_wait(session: &str, call_id: u64, id: &str) -> pb::ForwardedCall {
        let mut call = forwarded_create(session, call_id);
        call.method = "/humata.v1.InboxService/WaitForAnswer".to_owned();
        call.request = pb::WaitForAnswerRequest {
            id: Some(pb::RequestId { value: id.to_owned() }),
            wait_seconds: 0,
        }
        .encode_to_vec();
        call
    }

    /// The id of the request a forwarded edit by the agent made the host
    /// ask about (BD-15), the host's Inbox holding it.
    async fn asked_about_edit(host: &Host) -> String {
        host.boards.replay(BOX, &create(1, "first")).await;
        rule(host, Decision::Ask);
        let mut call = forwarded_create(AGENT, 10);
        call.method = "/humata.v1.BoardService/UpdateItem".to_owned();
        call.request = pb::UpdateItemRequest {
            id: Some(pb::BoardItemId { value: "PAY-1".to_owned() }),
            patch: Some(pb::ItemPatch {
                title: Some("changed".to_owned()),
                ..Default::default()
            }),
            ..Default::default()
        }
        .encode_to_vec();
        let result = host.boards.forwarded(BOX, call).await;
        let Some(pb::forwarded_result::Outcome::Error(error)) = result.outcome else {
            panic!("the host asks the user: {result:?}");
        };
        let Some(pb::error::Kind::ApprovalPending(pending)) = error.kind else {
            panic!("the host's answer is approval_pending: {error:?}");
        };
        pending.request.expect("it names the request").value
    }

    #[tokio::test]
    async fn a_forwarded_wait_finds_only_the_forwarding_agents_own_request() {
        let host = host();
        host.store.put_session(&session("se_other", BOX)).unwrap();
        let id = asked_about_edit(&host).await;

        let own = host.boards.forwarded(BOX, forwarded_wait(AGENT, 11, &id)).await;
        let Some(pb::forwarded_result::Outcome::Response(bytes)) = own.outcome else {
            panic!("the agent's own request is found: {own:?}");
        };
        let waited = pb::WaitResponse::decode(bytes.as_slice()).unwrap();
        assert_eq!(waited.state, pb::RequestState::Pending as i32);
        assert!(!waited.keepalive, "a forwarded wait answers with the final state only");

        let other = host.boards.forwarded(BOX, forwarded_wait("se_other", 12, &id)).await;
        let Some(pb::forwarded_result::Outcome::Error(error)) = other.outcome else {
            panic!("another session's request isn't found: {other:?}");
        };
        assert!(matches!(error.kind, Some(pb::error::Kind::NotFound(_))));
    }

    #[tokio::test]
    async fn a_forwarded_withdrawal_ends_only_the_forwarding_agents_own_request() {
        let host = host();
        host.store.put_session(&session("se_other", BOX)).unwrap();
        let id = asked_about_edit(&host).await;
        let withdraw = |session: &str, call_id| {
            let mut call = forwarded_create(session, call_id);
            call.method = "/humata.v1.InboxService/WithdrawRequest".to_owned();
            call.request = pb::WithdrawRequestRequest {
                idempotency_key: None,
                id: Some(pb::RequestId { value: id.clone() }),
            }
            .encode_to_vec();
            call
        };
        let other = host.boards.forwarded(BOX, withdraw("se_other", 13)).await;
        assert!(matches!(other.outcome, Some(pb::forwarded_result::Outcome::Error(_))));
        let own = host.boards.forwarded(BOX, withdraw(AGENT, 14)).await;
        let Some(pb::forwarded_result::Outcome::Response(bytes)) = own.outcome else {
            panic!("the agent withdraws its own request: {own:?}");
        };
        let withdrawn = pb::InboxRequest::decode(bytes.as_slice()).unwrap();
        assert_eq!(withdrawn.state, pb::RequestState::Withdrawn as i32);
    }

    fn left_out_of_agent(host: &Host) -> Option<humata2_store::AwayLeftOut> {
        host.store.away_left_out(AGENT).unwrap()
    }

    // ----- An entry the user approved on the machine (D268). -----

    use pb::request_preview::Preview;

    /// The card the machine made for a write, with `preview`, as its
    /// journal carried it.
    fn card_of(id: &str, session: &str, state: pb::RequestState, preview: &Preview) -> pb::InboxRequest {
        pb::InboxRequest {
            id: Some(pb::RequestId { value: id.to_owned() }),
            state: state as i32,
            requester: Some(pb::AgentActor {
                session: Some(pb::SessionId {
                    value: session.to_owned(),
                }),
                ..Default::default()
            }),
            preview: Some(pb::RequestPreview {
                preview: Some(preview.clone()),
            }),
            ..Default::default()
        }
    }

    /// A card about a change to item `key`.
    fn about_item(key: &str) -> Preview {
        Preview::BoardChange(pb::BoardChangePreview {
            item: Some(pb::BoardItemId { value: key.to_owned() }),
            ..Default::default()
        })
    }

    fn about_new_item() -> Preview {
        Preview::NewBoardItem(pb::NewBoardItemPreview::default())
    }

    /// The host's Inbox takes `machine`'s journal entries `first` (the
    /// request) and, unless it is still pending, `first + 1` (its answer), as
    /// a link that brought them would.
    async fn mirror(host: &Host, machine: &str, first: u64, session: &str, state: pb::RequestState, preview: &Preview) {
        let asked = card_of("rq_1", session, pb::RequestState::Pending, preview);
        let kept = host
            .inbox
            .replay(machine, &entry(first, JOURNAL_REQUEST, session, asked.encode_to_vec()))
            .await;
        assert_eq!(kept, Replayed::Applied);
        if state == pb::RequestState::Pending {
            return;
        }
        let mut answered = card_of("rq_1", session, state, preview);
        answered.answer = Some(pb::RequestAnswer {
            answer: Some(match state {
                pb::RequestState::Approved => pb::request_answer::Answer::Approve(pb::Approve {}),
                _ => pb::request_answer::Answer::Deny(pb::Deny::default()),
            }),
        });
        let kept = host
            .inbox
            .replay(machine, &entry(first + 1, JOURNAL_ANSWER, session, answered.encode_to_vec()))
            .await;
        assert_eq!(kept, Replayed::Applied);
    }

    /// `entry` as the machine journals a write the user approved there.
    fn approving(mut entry: pb::JournalEntry, id: &str) -> pb::JournalEntry {
        entry.approved_request = Some(pb::RequestId { value: id.to_owned() });
        entry
    }

    fn delete_of(seq: u64, key: &str) -> pb::JournalEntry {
        entry(
            seq,
            "board.item_delete",
            AGENT,
            pb::DeleteItemRequest {
                id: Some(pb::BoardItemId { value: key.to_owned() }),
                ..Default::default()
            }
            .encode_to_vec(),
        )
    }

    /// A host with item `PAY-1` (journal entry 1) and a rule that asks.
    async fn asking() -> Host {
        let host = host();
        host.boards.replay(BOX, &create(1, "first")).await;
        rule(&host, Decision::Ask);
        host
    }

    /// Whether the edit that named `rq_1` as its approval changed the item.
    async fn edit_applied(host: &Host) -> bool {
        let outcome = host
            .boards
            .replay(BOX, &approving(retitle(4, "PAY-1", "changed"), "rq_1"))
            .await;
        assert_eq!(outcome, Replayed::Applied, "the entry is acked either way");
        item(host, "PAY-1").title == "changed"
    }

    #[tokio::test]
    async fn an_ask_the_user_approved_on_the_machine_is_applied() {
        let host = asking().await;
        mirror(&host, BOX, 2, AGENT, pb::RequestState::Approved, &about_item("PAY-1")).await;
        assert!(edit_applied(&host).await);
        assert_eq!(item(&host, "PAY-1").revision, 2);
        assert_eq!(left_out_of_agent(&host), None);
    }

    #[tokio::test]
    async fn an_ask_whose_request_is_pending_or_denied_is_left_out_as_asked() {
        for state in [pb::RequestState::Pending, pb::RequestState::Denied] {
            let host = asking().await;
            mirror(&host, BOX, 2, AGENT, state, &about_item("PAY-1")).await;
            assert!(!edit_applied(&host).await, "{state:?}");
            let kept = left_out_of_agent(&host).expect("the edit was left out");
            assert!(!kept.refused, "{state:?}: Ask me, not a refusal");
        }
    }

    #[tokio::test]
    async fn an_approval_made_by_another_session_is_not_this_sessions() {
        let host = asking().await;
        host.store.put_session(&session("se_two", BOX)).unwrap();
        mirror(&host, BOX, 2, "se_two", pb::RequestState::Approved, &about_item("PAY-1")).await;
        assert!(!edit_applied(&host).await);
        assert!(!left_out_of_agent(&host).expect("the edit was left out").refused);
    }

    #[tokio::test]
    async fn an_approval_about_another_item_does_not_approve_this_one() {
        let host = asking().await;
        mirror(&host, BOX, 2, AGENT, pb::RequestState::Approved, &about_item("PAY-2")).await;
        assert!(!edit_applied(&host).await);
        assert!(!left_out_of_agent(&host).expect("the edit was left out").refused);
    }

    #[tokio::test]
    async fn an_approval_held_by_another_machine_does_not_approve_this_ones_entry() {
        let host = asking().await;
        mirror(&host, "ma_other", 1, "se_far", pb::RequestState::Approved, &about_item("PAY-1")).await;
        assert!(!edit_applied(&host).await);
        assert!(!left_out_of_agent(&host).expect("the edit was left out").refused);
    }

    #[tokio::test]
    async fn an_approval_of_a_change_is_not_an_approval_of_a_deletion() {
        let host = asking().await;
        mirror(&host, BOX, 2, AGENT, pb::RequestState::Approved, &about_item("PAY-1")).await;
        let outcome = host.boards.replay(BOX, &approving(delete_of(4, "PAY-1"), "rq_1")).await;
        assert_eq!(outcome, Replayed::Applied);
        assert!(!item(&host, "PAY-1").deleted);
    }

    #[tokio::test]
    async fn an_approval_never_overrides_never() {
        let host = asking().await;
        rule(&host, Decision::Never);
        mirror(&host, BOX, 2, AGENT, pb::RequestState::Approved, &about_item("PAY-1")).await;
        assert!(!edit_applied(&host).await);
        assert!(left_out_of_agent(&host).expect("the edit was left out").refused);
    }

    #[tokio::test]
    async fn an_approval_under_an_allow_rule_changes_nothing() {
        let host = asking().await;
        rule(&host, Decision::Allow);
        mirror(&host, BOX, 2, AGENT, pb::RequestState::Approved, &about_item("PAY-1")).await;
        assert!(edit_applied(&host).await);
        assert_eq!(left_out_of_agent(&host), None);
    }

    #[tokio::test]
    async fn a_new_item_the_user_approved_on_the_machine_is_made() {
        let host = host();
        rule(&host, Decision::Ask);
        mirror(&host, BOX, 1, AGENT, pb::RequestState::Approved, &about_new_item()).await;
        let outcome = host.boards.replay(BOX, &approving(create(3, "first"), "rq_1")).await;
        assert_eq!(outcome, Replayed::Applied);
        assert_eq!(item(&host, "PAY-1").title, "first");
        assert_eq!(left_out_of_agent(&host), None);
    }

    #[tokio::test]
    async fn an_approved_entry_delivered_twice_is_applied_once() {
        let host = asking().await;
        mirror(&host, BOX, 2, AGENT, pb::RequestState::Approved, &about_item("PAY-1")).await;
        assert!(edit_applied(&host).await);
        assert!(edit_applied(&host).await);
        assert_eq!(item(&host, "PAY-1").revision, 2);
    }

    #[tokio::test]
    async fn an_edit_a_rule_now_asks_about_is_summed_as_asked_not_refused() {
        let host = host();
        host.boards.replay(BOX, &create(1, "first")).await;
        rule(&host, Decision::Ask);
        host.boards.replay(BOX, &retitle(2, "PAY-1", "changed")).await;
        let kept = left_out_of_agent(&host).expect("the edit was left out");
        assert_eq!(kept.count, 1);
        assert_eq!(kept.items, vec!["PAY-1".to_owned()]);
        assert!(!kept.refused);
        let item = item(&host, "PAY-1");
        assert_eq!((item.title.as_str(), item.revision), ("first", 1));
        assert!(host.store.board_item_history("PAY-1", 0, 10).unwrap().last().unwrap().lost);
    }

    #[tokio::test]
    async fn a_create_a_rule_now_refuses_is_summed_by_its_title_as_refused() {
        let host = host();
        rule(&host, Decision::Never);
        assert_eq!(host.boards.replay(BOX, &create(1, "Fix login")).await, Replayed::Rejected);
        let kept = left_out_of_agent(&host).expect("the create was left out");
        assert_eq!(kept.count, 1);
        assert!(kept.items.is_empty());
        assert_eq!(kept.not_made, vec!["Fix login".to_owned()]);
        assert!(kept.refused);
    }

    #[tokio::test]
    async fn the_same_entry_delivered_twice_is_counted_once() {
        let host = host();
        rule(&host, Decision::Never);
        host.boards.replay(BOX, &create(1, "first")).await;
        host.boards.replay(BOX, &create(1, "first")).await;
        assert_eq!(left_out_of_agent(&host).unwrap().count, 1);
    }

    #[tokio::test]
    async fn one_link_up_adds_up_and_a_later_one_replaces_the_summary() {
        let host = host();
        host.boards.replay(BOX, &create(1, "first")).await;
        rule(&host, Decision::Ask);
        host.boards.replay_at(BOX, &retitle(2, "PAY-1", "a"), 10).await;
        rule(&host, Decision::Never);
        host.boards.replay_at(BOX, &create(3, "second"), 10).await;
        let sum = left_out_of_agent(&host).unwrap();
        assert_eq!(sum.count, 2);
        assert!(sum.refused, "the stricter wins when they are mixed");
        host.boards.replay_at(BOX, &retitle(4, "PAY-1", "b"), 20).await;
        let later = left_out_of_agent(&host).unwrap();
        assert_eq!((later.count, later.not_made.len()), (1, 0));
    }

    #[tokio::test]
    async fn what_isnt_a_rule_stopping_the_write_leaves_no_summary() {
        let host = host();
        // Allowed: applied.
        host.boards.replay(BOX, &create(1, "first")).await;
        host.boards.replay(BOX, &retitle(2, "PAY-1", "changed")).await;
        // An item that isn't there, a request that can't be read, a session placed elsewhere.
        rule(&host, Decision::Never);
        host.boards.replay(BOX, &retitle(3, "PAY-9", "x")).await;
        host.boards.replay(BOX, &entry(4, "board.item_create", AGENT, vec![0xff, 0xff, 0xff])).await;
        host.boards.replay(BOX, &entry(5, "board.item_create", "se_far", Vec::new())).await;
        assert_eq!(left_out_of_agent(&host), None);
    }
}
