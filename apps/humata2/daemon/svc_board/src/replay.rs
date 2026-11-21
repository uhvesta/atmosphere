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

use humata2_access::{BoardAct, BoardRef, Call, Capability, PresenceMethod, Refusal};
use humata2_daemon_ports::{Caller, UserVia};
use crate::pb;
use humata2_rpc_kit::{blocking, chain, invalid_field, lock, missing, not_built_status, now_ms};
use humata2_store::{
    BoardItemChange, BoardItemRecord, BoardKey, BoardReplay, BoardReplayed, BoardWrite, LeftOut,
    NewBoardItem, StoreError,
};
use pb::invalid_field::Problem;
use prost::Message;
use tonic::{Code, Status};

use super::ports::{
    Checked, Method, Placement, Replayed, agent_caller_of, by_name, by_operation, error_of_status,
    on_link, placed_on, refused, resolve_fork, store_error,
};
use super::scope::{self, board_for_policy, by_of, column_number, made_by_session};
use super::{BoardSvc, DEFAULT_COLUMN, Op, history, ids, item_id, item_ids, keys, merged, page_of};

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
    match (
        request
            .preview
            .as_ref()
            .and_then(|shown| shown.preview.as_ref()),
        about,
    ) {
        (Some(Preview::NewBoardItem(_)), About::NewItem) => true,
        (Some(Preview::BoardChange(change)), About::Item { key, deletes }) => {
            change.item.as_ref().is_some_and(|item| item.value == *key)
                && change.deletes_item == *deletes
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
fn left_out_of(
    entry: &pb::JournalEntry,
    why: &Rejection,
    item: Option<&str>,
    link_up: i64,
) -> Option<LeftOut> {
    let refused = match why {
        Rejection::Asks => false,
        Rejection::Refused { .. } => true,
        _ => return None,
    };
    let session = entry.principal.as_ref()?.session.as_ref()?.value.clone();
    let not_made = match item {
        Some(_) => None,
        None => Some(
            decoded::<pb::CreateItemRequest>(entry)
                .ok()?
                .title
                .trim()
                .to_owned(),
        ),
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
    humata2_rpc_kit::log::say!(
        "humatad2: warning: {machine}'s journal entry {} ({}) was left out: {}",
        entry.seq,
        entry.operation,
        chain(why)
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

/// `request` as a call `caller` made, for a service that reads its caller
/// from the request as the socket's interceptor leaves it.
fn as_caller<T>(caller: Caller, request: T) -> tonic::Request<T> {
    let mut request = tonic::Request::new(request);
    request.extensions_mut().insert(caller);
    request
}

fn request_of<T: Message + Default>(bytes: &[u8], method: &str) -> Result<T, Status> {
    T::decode(bytes).map_err(|error| {
        invalid_field(
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
    fn agent_caller(
        &self,
        machine: &str,
        session: &str,
        method: &str,
    ) -> Result<Caller, AgentError> {
        let record = placed_on(&self.store, machine, session)?;
        let mut agent = resolve_fork(agent_caller_of(&record, &self.machine), &self.store)
            .map_err(|source| AgentError::Fork {
                session: session.to_owned(),
                source,
            })?;
        method.clone_into(&mut agent.method);
        Ok(Caller::Agent(agent))
    }

    /// Runs a call `machine` forwarded for one of its agents, as that agent
    /// under this Mac's rules, and answers it.
    pub async fn forwarded(&self, machine: &str, call: pb::ForwardedCall) -> pb::ForwardedResult {
        let outcome = match self.run_forwarded(machine, &call).await {
            Ok(response) => pb::forwarded_result::Outcome::Response(response),
            Err(failed) => {
                pb::forwarded_result::Outcome::Error(error_of_status(&failed).unwrap_or_else(|| {
                    pb::Error {
                        message: failed.message().to_owned(),
                        fix: String::new(),
                        retry: None,
                        kind: None,
                    }
                }))
            }
        };
        pb::ForwardedResult {
            call_id: call.call_id,
            outcome: Some(outcome),
        }
    }

    async fn run_forwarded(
        &self,
        machine: &str,
        call: &pb::ForwardedCall,
    ) -> Result<Vec<u8>, Status> {
        let Some(row) = by_name(&call.method) else {
            return Err(not_built_status(&call.method));
        };
        let bytes = call.request.as_slice();
        if row.method == Method::AskWhy {
            let caller = match &call.principal {
                Some(pb::forwarded_call::Principal::User(user)) if user.outside_sessions => {
                    Caller::User(UserVia::Host)
                }
                Some(pb::forwarded_call::Principal::User(_)) => {
                    return Err(refused(&Refusal::PresenceRequired {
                        accepted: vec![PresenceMethod::ApprovalPassphrase],
                    }));
                }
                Some(pb::forwarded_call::Principal::Agent(_)) => {
                    return Err(refused(&Refusal::UserOnly {
                        capability: Some(Capability::InboxAnswer),
                    }));
                }
                None => return Err(not_built_status(&call.method)),
            };
            let request: pb::AskWhyRequest = request_of(bytes, &call.method)?;
            return Ok(self.inbox.ask_why_as(caller, request).await?.encode_to_vec());
        }
        // PROVIDERCLI (D906): any remote caller (owner/script outside a session
        // or a session agent) can list providers, read config, or ask the host
        // owner to import a provider file into the host Inbox.
        if matches!(
            row.method,
            Method::ListProviders | Method::GetProvidersConfig | Method::RequestProviderImport
        ) {
            if let Some(pb::forwarded_call::Principal::User(user)) = &call.principal {
                if !user.outside_sessions {
                    return Err(refused(&Refusal::PresenceRequired {
                        accepted: vec![PresenceMethod::ApprovalPassphrase],
                    }));
                }
                let caller = Caller::User(UserVia::Host);
                let providers = &self.host_calls_for(&call.method)?.providers;
                return match row.method {
                    Method::ListProviders => {
                        let request: pb::ListProvidersRequest = request_of(bytes, &call.method)?;
                        Ok(providers
                            .list_providers(as_caller(caller, request))
                            .await?
                            .into_inner()
                            .encode_to_vec())
                    }
                    Method::GetProvidersConfig => {
                        let request: pb::GetProvidersConfigRequest =
                            request_of(bytes, &call.method)?;
                        Ok(providers
                            .get_providers_config(as_caller(caller, request))
                            .await?
                            .into_inner()
                            .encode_to_vec())
                    }
                    Method::RequestProviderImport => {
                        let request: pb::RequestProviderImportRequest =
                            request_of(bytes, &call.method)?;
                        Ok(providers
                            .request_provider_import(as_caller(caller, request))
                            .await?
                            .into_inner()
                            .encode_to_vec())
                    }
                    _ => unreachable!(),
                };
            }
        }
        let Some(pb::forwarded_call::Principal::Agent(principal)) = &call.principal else {
            return Err(not_built_status(&call.method));
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
                AgentError::Fork { source, .. } => {
                    store_error(source, &self.machine, "read the session")
                }
            })?;
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
                Ok(me
                    .write(caller, Op::Create(request))
                    .await?
                    .into_inner()
                    .encode_to_vec())
            }
            Method::UpdateItem => {
                let request: pb::UpdateItemRequest = request_of(bytes, &call.method)?;
                Ok(me
                    .write(caller, Op::Update(request))
                    .await?
                    .into_inner()
                    .encode_to_vec())
            }
            Method::MoveItem => {
                let request: pb::MoveItemRequest = request_of(bytes, &call.method)?;
                Ok(me
                    .write(caller, Op::Move(request))
                    .await?
                    .into_inner()
                    .encode_to_vec())
            }
            Method::DeleteItem => {
                let request: pb::DeleteItemRequest = request_of(bytes, &call.method)?;
                Ok(me
                    .write(caller, Op::Delete(request))
                    .await?
                    .into_inner()
                    .encode_to_vec())
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
            Method::AskWhy => Err(refused(&Refusal::UserOnly {
                capability: Some(Capability::InboxAnswer),
            })),
            Method::ExplainAsk => {
                let request: pb::ExplainAskRequest = request_of(bytes, &call.method)?;
                Ok(me.inbox.explain_as(caller, request).await?.encode_to_vec())
            }
            // REPOS (RL-12, D568): the host's library and search, as that
            // agent, under the host's rule for global reads.
            Method::SearchRepos => {
                let request: pb::SearchReposRequest = request_of(bytes, &call.method)?;
                let repos = me.repos_for(&call.method)?;
                Ok(repos
                    .search_repos(as_caller(caller, request))
                    .await?
                    .into_inner()
                    .encode_to_vec())
            }
            Method::ListRepoLibrary => {
                let request: pb::ListRepoLibraryRequest = request_of(bytes, &call.method)?;
                let repos = me.repos_for(&call.method)?;
                Ok(repos
                    .list_repo_library(as_caller(caller, request))
                    .await?
                    .into_inner()
                    .encode_to_vec())
            }
            // D768: only the host keeps machines and holds their links, so
            // a machine's agents look machines up, read other machines'
            // files and copy between machines here, as that agent.
            Method::ListMachines => {
                let request: pb::ListMachinesRequest = request_of(bytes, &call.method)?;
                let machines = &me.host_calls_for(&call.method)?.machines;
                Ok(machines
                    .list_for_machine_agent(as_caller(caller, request))
                    .await?
                    .into_inner()
                    .encode_to_vec())
            }
            Method::GetMachine => {
                let request: pb::GetMachineRequest = request_of(bytes, &call.method)?;
                let machines = &me.host_calls_for(&call.method)?.machines;
                Ok(machines
                    .get_machine(as_caller(caller, request))
                    .await?
                    .into_inner()
                    .encode_to_vec())
            }
            Method::ListDirectory => {
                let mut request: pb::ListDirectoryRequest = request_of(bytes, &call.method)?;
                request.location = request
                    .location
                    .map(|location| on_link(machine, location));
                let files = &me.host_calls_for(&call.method)?.files;
                Ok(files
                    .list_directory(as_caller(caller, request))
                    .await?
                    .into_inner()
                    .encode_to_vec())
            }
            Method::Stat => {
                let mut request: pb::StatRequest = request_of(bytes, &call.method)?;
                request.location = request
                    .location
                    .map(|location| on_link(machine, location));
                let files = &me.host_calls_for(&call.method)?.files;
                Ok(files
                    .stat(as_caller(caller, request))
                    .await?
                    .into_inner()
                    .encode_to_vec())
            }
            Method::StartTransfer => {
                let mut request: pb::StartTransferRequest = request_of(bytes, &call.method)?;
                request.sources = request
                    .sources
                    .into_iter()
                    .map(|location| on_link(machine, location))
                    .collect();
                request.destination = request
                    .destination
                    .map(|location| on_link(machine, location));
                let transfers = &me.host_calls_for(&call.method)?.transfers;
                Ok(transfers
                    .start_transfer(as_caller(caller, request))
                    .await?
                    .into_inner()
                    .encode_to_vec())
            }
            // BROWSER (D891): a machine's agents reach the host Mac's connected
            // browser extension as that agent, with Inbox approval verified on
            // both ends.
            Method::GetBrowserBridgeStatus => {
                let request: pb::GetBrowserBridgeStatusRequest = request_of(bytes, &call.method)?;
                let browser = &me.host_calls_for(&call.method)?.browser;
                Ok(browser
                    .get_browser_bridge_status(as_caller(caller, request))
                    .await?
                    .into_inner()
                    .encode_to_vec())
            }
            Method::ExecuteBrowserAction => {
                let request: pb::ExecuteBrowserActionRequest = request_of(bytes, &call.method)?;
                let browser = &me.host_calls_for(&call.method)?.browser;
                Ok(browser
                    .execute_browser_action(as_caller(caller, request))
                    .await?
                    .into_inner()
                    .encode_to_vec())
            }
            Method::ListProviders => {
                let request: pb::ListProvidersRequest = request_of(bytes, &call.method)?;
                let providers = &me.host_calls_for(&call.method)?.providers;
                Ok(providers
                    .list_providers(as_caller(caller, request))
                    .await?
                    .into_inner()
                    .encode_to_vec())
            }
            Method::GetProvidersConfig => {
                let request: pb::GetProvidersConfigRequest = request_of(bytes, &call.method)?;
                let providers = &me.host_calls_for(&call.method)?.providers;
                Ok(providers
                    .get_providers_config(as_caller(caller, request))
                    .await?
                    .into_inner()
                    .encode_to_vec())
            }
            Method::RequestProviderImport => {
                let request: pb::RequestProviderImportRequest = request_of(bytes, &call.method)?;
                let providers = &me.host_calls_for(&call.method)?.providers;
                Ok(providers
                    .request_provider_import(as_caller(caller, request))
                    .await?
                    .into_inner()
                    .encode_to_vec())
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
    pub async fn replay_at(
        &self,
        machine: &str,
        entry: &pb::JournalEntry,
        link_up_unix_ms: i64,
    ) -> Replayed {
        let approval = self.approval_of(machine, entry).await;
        let (me, name, taken) = (self.clone(), machine.to_owned(), entry.clone());
        match blocking(move || me.replay_entry(&name, &taken, approval.as_ref(), link_up_unix_ms))
            .await
        {
            Ok(Ok(finished)) => {
                if let Some(key) = &finished.deleted
                    && let Err(failed) = self.inbox.withdraw_about_item(key).await
                {
                    humata2_rpc_kit::log::say!(
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
                humata2_rpc_kit::log::say!("humatad2: warning: {}", chain(&error));
                Replayed::Held
            }
            Err(failed) => {
                humata2_rpc_kit::log::say!(
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
    async fn approval_of(
        &self,
        machine: &str,
        entry: &pb::JournalEntry,
    ) -> Option<pb::InboxRequest> {
        let id = entry.approved_request.as_ref()?;
        let session = entry
            .principal
            .as_ref()
            .and_then(|principal| principal.session.as_ref())
            .map(|session| session.value.as_str())
            .unwrap_or_default();
        let found = self.inbox.approved_on(machine, &id.value, session).await;
        if found.is_none() {
            humata2_rpc_kit::log::say!(
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
                    Some(noted) => self
                        .store
                        .apply_machine_journal_left_out(machine, seq, noted),
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
            humata2_rpc_kit::log::say!(
                "humatad2: {machine}'s journal entry {} ({}) on {} wasn't applied and is kept in its history as lost: {}",
                seq,
                entry.operation,
                planned.key,
                chain(why)
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
                self.store
                    .apply_machine_journal(machine, seq, None)
                    .map_err(at_store)?;
                Ok(Finished::of(Replayed::Rejected))
            }
            Err(source) => Err(store_failed(machine, seq, source)),
        }
    }

    /// Tells the feed what an applied entry did, and logs what it didn't.
    fn settled(
        &self,
        machine: &str,
        entry: &pb::JournalEntry,
        planned: &Planned,
        done: BoardReplayed,
    ) -> Finished {
        let archived = self.archived().unwrap_or_default();
        let kept_lost = |what: &str| {
            humata2_rpc_kit::log::say!(
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
            BoardReplayed::Written(
                BoardWrite::MissingDependency { key } | BoardWrite::DeletedDependency { key },
            ) => {
                kept_lost(&format!(
                    "it depends on {key}, which is gone here; nothing was changed"
                ));
                Finished::of(Replayed::Applied)
            }
            BoardReplayed::Refused | BoardReplayed::AlreadyApplied => {
                Finished::of(Replayed::Applied)
            }
        }
    }

    /// Works out what `entry` does on this Mac, and whether the rules as
    /// they are now let the agent do it.
    fn plan(
        &self,
        machine: &str,
        entry: &pb::JournalEntry,
        approval: Option<&pb::InboxRequest>,
    ) -> Result<Planned, Stopped> {
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
            Err(
                AgentError::Placement(Placement::Store { source, .. })
                | AgentError::Fork { source, .. },
            ) => {
                return Err(Stopped::Store(source));
            }
            Err(AgentError::Placement(source)) => return Err(Rejection::Session { source }.into()),
        };
        let expected = entry.base_revision.as_ref().map(|revision| revision.value);
        let at = at_ms(entry);
        match row.method {
            Method::CreateItem => self.plan_create(
                caller,
                &decoded::<pb::CreateItemRequest>(entry)?,
                at,
                approval,
            ),
            Method::UpdateItem => self.plan_update(
                caller,
                &decoded::<pb::UpdateItemRequest>(entry)?,
                expected,
                at,
                approval,
            ),
            Method::MoveItem => self.plan_move(
                caller,
                &decoded::<pb::MoveItemRequest>(entry)?,
                expected,
                at,
                approval,
            ),
            Method::DeleteItem => self.plan_delete(
                caller,
                &decoded::<pb::DeleteItemRequest>(entry)?,
                expected,
                at,
                approval,
            ),
            Method::ListItems
            | Method::GetItem
            | Method::GetItemHistory
            | Method::WaitForAnswer
            | Method::WithdrawRequest
            | Method::SearchRepos
            | Method::ListRepoLibrary
            | Method::ListMachines
            | Method::GetMachine
            | Method::ListDirectory
            | Method::Stat
            | Method::StartTransfer
            | Method::GetBrowserBridgeStatus
            | Method::ExecuteBrowserAction
            | Method::ListProviders
            | Method::GetProvidersConfig
            | Method::RequestProviderImport
            | Method::AskWhy
            | Method::ExplainAsk => Err(Rejection::UnknownOperation {
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
            Ok(Checked::Ask(_)) if approval.is_some_and(|request| approves(request, about)) => {
                Ok(())
            }
            Ok(Checked::Ask(_)) => Err(Rejection::Asks),
            Err(source) => Err(Rejection::Refused { source }),
        }
    }

    fn item_for_replay(&self, id: &str) -> Result<BoardItemRecord, Stopped> {
        match self.store.board_item(id) {
            Ok(Some(item)) => Ok(item),
            Ok(None) | Err(StoreError::InvalidId { .. }) => Err(Rejection::NoItem {
                key: id.to_owned(),
            }
            .into()),
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
            self.allowed(&caller, self.policy_of(item), act, approval, &about)
                .err()
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
            return Err(unusable(missing("title")));
        }
        let column = match request.column {
            0 => DEFAULT_COLUMN,
            column => column_number(column, "column").map_err(unusable)?,
        };
        let board = request
            .board
            .as_ref()
            .ok_or_else(|| unusable(missing("board")))?;
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
        let sessions = self
            .known_sessions(&request.sessions, "sessions")
            .map_err(unusable)?;
        let depends_on = self
            .readable_targets(&caller, &request.depends_on)
            .map_err(unusable)?;
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
        if patch
            .title
            .as_deref()
            .is_some_and(|title| title.trim().is_empty())
        {
            return Err(unusable(missing("patch.title")));
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
            description: patch
                .description
                .filter(|text| *text != item.description),
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
        let rank = self
            .rank_in(&item.board, column, &item.key, after)
            .map_err(unusable)?;
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
    use std::sync::{Arc, Mutex};

    use humata2_access::{AccessRequest, Call};
    use humata2_daemon_ports::{BoardAsk, Caller, Run};
    use humata2_store::{SessionRecord, Store};
    use tonic::Status;

    use super::*;
    use crate::ports::{AwayCall, BoardFeed, BoardGuard, BoardInbox, Checked, Scope};

    const HOST: &str = "ma_host";
    const BOX: &str = "ma_box";
    const AGENT: &str = "se_a";

    struct AllowAllGuard;

    impl BoardGuard for AllowAllGuard {
        fn check(&self, _caller: &Caller, _call: &Call) -> Result<Checked, Status> {
            Ok(Checked::Allowed)
        }
        fn check_away(&self, _caller: &Caller, _call: &Call) -> Result<AwayCall, Status> {
            Ok(AwayCall::Journal)
        }
        fn lists_low_risk(&self, _asked: &AccessRequest) -> bool {
            false
        }
        fn policy_changed(&self) {}
        fn host_link(&self) -> String {
            String::new()
        }
    }

    struct NoopFeed;

    impl BoardFeed for NoopFeed {
        fn publish_in(
            &self,
            _change: pb::Change,
            _payload: pb::event::Payload,
            _scope: Scope,
            _by: Option<pb::Actor>,
        ) {
        }
    }

    struct NoopInbox {
        approved: Mutex<Vec<(String, String, String, pb::InboxRequest)>>,
        asked_why: Mutex<Vec<(Caller, pb::AskWhyRequest)>>,
        explained: Mutex<Vec<(Caller, pb::ExplainAskRequest)>>,
    }

    #[tonic::async_trait]
    impl BoardInbox for NoopInbox {
        async fn withdraw_about_item(&self, _key: &str) -> Result<(), Status> {
            Ok(())
        }
        async fn defer_board(
            &self,
            _caller: &Caller,
            _asked: AccessRequest,
            _idempotency: Option<String>,
            _board: BoardAsk,
            _run: Run,
        ) -> Status {
            Status::failed_precondition("approval pending")
        }
        async fn pending_board_asks(&self) -> Vec<pb::InboxRequest> {
            Vec::new()
        }
        async fn wait_final(
            &self,
            _caller: Caller,
            _request: pb::WaitForAnswerRequest,
        ) -> Result<pb::WaitResponse, Status> {
            Err(Status::not_found("none"))
        }
        async fn withdraw_as(
            &self,
            _caller: Caller,
            _request: pb::WithdrawRequestRequest,
        ) -> Result<pb::InboxRequest, Status> {
            Err(Status::not_found("none"))
        }
        async fn ask_why_as(
            &self,
            caller: Caller,
            request: pb::AskWhyRequest,
        ) -> Result<pb::InboxRequest, Status> {
            let id = request.id.clone();
            self.asked_why.lock().unwrap().push((caller, request));
            Ok(pb::InboxRequest {
                id,
                state: pb::RequestState::Pending as i32,
                ..Default::default()
            })
        }
        async fn explain_as(
            &self,
            caller: Caller,
            request: pb::ExplainAskRequest,
        ) -> Result<pb::InboxRequest, Status> {
            let id = request.id.clone();
            let reply = request.reply.clone();
            self.explained.lock().unwrap().push((caller, request));
            Ok(pb::InboxRequest {
                id,
                state: pb::RequestState::Pending as i32,
                asked_why: Some(pb::AskedWhy {
                    reply,
                    ..Default::default()
                }),
                ..Default::default()
            })
        }
        async fn approved_on(
            &self,
            machine: &str,
            id: &str,
            session: &str,
        ) -> Option<pb::InboxRequest> {
            self.approved
                .lock()
                .unwrap()
                .iter()
                .find(|(m, i, s, _)| m == machine && i == id && s == session)
                .map(|(_, _, _, req)| req.clone())
        }
    }

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

    fn test_host_with_inbox() -> (tempfile::TempDir, Arc<Store>, Arc<NoopInbox>, BoardSvc) {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(dir.path()).unwrap());
        store.put_session(&session(AGENT, BOX)).unwrap();
        store.put_session(&session("se_far", "ma_other")).unwrap();
        store.claim_board_prefix("ch_a", "PAY").unwrap();
        let inbox = Arc::new(NoopInbox {
            approved: Mutex::new(Vec::new()),
            asked_why: Mutex::new(Vec::new()),
            explained: Mutex::new(Vec::new()),
        });
        let boards = BoardSvc::new(
            Arc::new(AllowAllGuard),
            Arc::clone(&store),
            Arc::new(NoopFeed),
            Arc::clone(&inbox) as Arc<dyn crate::ports::BoardInbox>,
            HOST.to_owned(),
        );
        (dir, store, inbox, boards)
    }

    fn test_host() -> (tempfile::TempDir, Arc<Store>, BoardSvc) {
        let (dir, store, _inbox, boards) = test_host_with_inbox();
        (dir, store, boards)
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
                id: Some(pb::BoardItemId {
                    value: key.to_owned(),
                }),
                patch: Some(pb::ItemPatch {
                    title: Some(title.to_owned()),
                    ..Default::default()
                }),
                ..Default::default()
            }
            .encode_to_vec(),
        )
    }

    #[tokio::test]
    async fn entries_apply_in_the_order_they_come_and_move_the_mark() {
        let (_dir, store, boards) = test_host();
        assert_eq!(boards.replay_at(BOX, &create(1, "first"), 1000).await, Replayed::Applied);
        assert_eq!(boards.replay_at(BOX, &retitle(2, "PAY-1", "second"), 1000).await, Replayed::Applied);
        assert_eq!(boards.replay_at(BOX, &retitle(3, "PAY-1", "third"), 1000).await, Replayed::Applied);
        let kept = store.board_item("PAY-1").unwrap().unwrap();
        assert_eq!(kept.title, "third");
        assert_eq!(kept.revision, 3);
        assert_eq!(kept.created_by, AGENT);
        assert_eq!(store.machine_journal_mark(BOX).unwrap(), 3);
    }

    #[tokio::test]
    async fn an_entry_delivered_twice_changes_nothing() {
        let (_dir, store, boards) = test_host();
        boards.replay_at(BOX, &create(1, "first"), 1000).await;
        assert_eq!(boards.replay_at(BOX, &create(1, "first"), 1000).await, Replayed::Applied);
        assert_eq!(
            store.board_items(&[BoardKey::Session(AGENT.to_owned())]).unwrap().len(),
            1
        );
    }

    #[tokio::test]
    async fn an_entry_for_a_session_placed_elsewhere_is_left_out_and_acked() {
        let (_dir, store, boards) = test_host();
        let from_far = entry(1, "board.item_create", "se_far", Vec::new());
        assert_eq!(boards.replay_at(BOX, &from_far, 1000).await, Replayed::Rejected);
        let unknown = entry(2, "board.item_create", "se_missing", Vec::new());
        assert_eq!(boards.replay_at(BOX, &unknown, 1000).await, Replayed::Rejected);
        assert_eq!(store.machine_journal_mark(BOX).unwrap(), 2);
    }

    #[test]
    fn by_name_finds_ask_why_and_explain_ask() {
        assert_eq!(
            by_name("/humata.v1.InboxService/AskWhy").map(|row| row.method),
            Some(Method::AskWhy)
        );
        assert_eq!(
            by_name("/humata.v1.InboxService/ExplainAsk").map(|row| row.method),
            Some(Method::ExplainAsk)
        );
        assert_eq!(Method::AskWhy.row().name, "/humata.v1.InboxService/AskWhy");
        assert_eq!(Method::ExplainAsk.row().name, "/humata.v1.InboxService/ExplainAsk");
    }

    #[tokio::test]
    async fn forwarded_ask_why_allows_trusted_peer_user_and_refuses_agent_or_in_session_user() {
        let (_dir, _store, inbox, boards) = test_host_with_inbox();
        let body = pb::AskWhyRequest {
            id: Some(pb::RequestId { value: "rq_1".to_owned() }),
            ..Default::default()
        }
        .encode_to_vec();

        // An agent trying to forward AskWhy is refused.
        let as_agent = boards
            .forwarded(
                BOX,
                pb::ForwardedCall {
                    call_id: 1,
                    principal: Some(pb::forwarded_call::Principal::Agent(pb::AgentPrincipal {
                        session: Some(pb::SessionId { value: AGENT.to_owned() }),
                        pid: 0,
                    })),
                    method: "/humata.v1.InboxService/AskWhy".to_owned(),
                    request: body.clone(),
                    ..Default::default()
                },
            )
            .await;
        assert!(matches!(as_agent.outcome, Some(pb::forwarded_result::Outcome::Error(_))));
        assert!(inbox.asked_why.lock().unwrap().is_empty());

        // A user inside a session terminal is refused.
        let inside = boards
            .forwarded(
                BOX,
                pb::ForwardedCall {
                    call_id: 2,
                    principal: Some(pb::forwarded_call::Principal::User(pb::MachineUser {
                        presence: None,
                        outside_sessions: false,
                    })),
                    method: "/humata.v1.InboxService/AskWhy".to_owned(),
                    request: body.clone(),
                    ..Default::default()
                },
            )
            .await;
        assert!(matches!(inside.outcome, Some(pb::forwarded_result::Outcome::Error(_))));
        assert!(inbox.asked_why.lock().unwrap().is_empty());

        // A user outside sessions on a trusted peer runs as Caller::User(UserVia::Host).
        let ok = boards
            .forwarded(
                BOX,
                pb::ForwardedCall {
                    call_id: 3,
                    principal: Some(pb::forwarded_call::Principal::User(pb::MachineUser {
                        presence: None,
                        outside_sessions: true,
                    })),
                    method: "/humata.v1.InboxService/AskWhy".to_owned(),
                    request: body,
                    ..Default::default()
                },
            )
            .await;
        let Some(pb::forwarded_result::Outcome::Response(bytes)) = ok.outcome else {
            panic!("expected AskWhy response, got {:?}", ok.outcome);
        };
        let decoded = pb::InboxRequest::decode(bytes.as_slice()).unwrap();
        assert_eq!(decoded.id.map(|id| id.value).as_deref(), Some("rq_1"));
        let recorded = inbox.asked_why.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        assert!(matches!(recorded[0].0, Caller::User(UserVia::Host)));
    }

    #[tokio::test]
    async fn forwarded_explain_ask_routes_for_placed_session_and_refuses_session_elsewhere() {
        let (_dir, _store, inbox, boards) = test_host_with_inbox();
        let body = pb::ExplainAskRequest {
            id: Some(pb::RequestId { value: "rq_1".to_owned() }),
            reply: "needed for the build".to_owned(),
        }
        .encode_to_vec();

        let wrong_machine = boards
            .forwarded(
                BOX,
                pb::ForwardedCall {
                    call_id: 1,
                    principal: Some(pb::forwarded_call::Principal::Agent(pb::AgentPrincipal {
                        session: Some(pb::SessionId { value: "se_far".to_owned() }),
                        pid: 0,
                    })),
                    method: "/humata.v1.InboxService/ExplainAsk".to_owned(),
                    request: body.clone(),
                    ..Default::default()
                },
            )
            .await;
        assert!(matches!(
            wrong_machine.outcome,
            Some(pb::forwarded_result::Outcome::Error(pb::Error {
                kind: Some(pb::error::Kind::IdentityMismatch(_)),
                ..
            }))
        ));
        assert!(inbox.explained.lock().unwrap().is_empty());

        let ok = boards
            .forwarded(
                BOX,
                pb::ForwardedCall {
                    call_id: 2,
                    principal: Some(pb::forwarded_call::Principal::Agent(pb::AgentPrincipal {
                        session: Some(pb::SessionId { value: AGENT.to_owned() }),
                        pid: 0,
                    })),
                    method: "/humata.v1.InboxService/ExplainAsk".to_owned(),
                    request: body,
                    ..Default::default()
                },
            )
            .await;
        let Some(pb::forwarded_result::Outcome::Response(bytes)) = ok.outcome else {
            panic!("expected ExplainAsk response, got {:?}", ok.outcome);
        };
        let decoded = pb::InboxRequest::decode(bytes.as_slice()).unwrap();
        assert_eq!(
            decoded.asked_why.map(|why| why.reply).as_deref(),
            Some("needed for the build")
        );
        let recorded = inbox.explained.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        assert!(matches!(
            &recorded[0].0,
            Caller::Agent(agent) if agent.session.id.as_str() == AGENT
        ));
    }
}
