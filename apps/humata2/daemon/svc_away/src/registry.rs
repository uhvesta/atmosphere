//! The calls a machine forwards to its host for its agents (AWAY, D227).
//! One table lists the methods that go up: the board reads, the board
//! writes with the name each has in the journal, four Inbox calls and the
//! repo library's reads (RL-12), the machine lookups only the host can
//! answer, and the file reads and transfers that reach other machines
//! (D768). Beside it are the machine's end of
//! the exchange (`Forwarder`: send a `ForwardedCall` on the Uplink, wait for
//! the `ForwardedResult` with the same call id) and the host's check that
//! a call or journal entry comes from a session it places on that machine.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::pb;
use humata2_rpc_kit::helpers::{self as status, lock};
use humata2_store::{SessionRecord, Store, StoreError};
use tokio::sync::{mpsc, oneshot};
use tonic::{Code, Status};

/// The board methods a machine's agents call that the host runs for them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    ListItems,
    GetItem,
    GetItemHistory,
    CreateItem,
    UpdateItem,
    MoveItem,
    DeleteItem,
    /// An agent's wait on an Inbox request the host holds (D268).
    WaitForAnswer,
    /// An agent's withdrawal of one (D268).
    WithdrawRequest,
    /// INBOXHOOKS: the owner's Ask why on a request the host holds.
    AskWhy,
    /// INBOXHOOKS: an agent's answer to Ask why on a request the host holds.
    ExplainAsk,
    /// REPOS (RL-12, D568): the library stays on the host, so a machine's
    /// agents search it there.
    SearchRepos,
    /// REPOS (RL-12, D568): and list it there.
    ListRepoLibrary,
    /// Only the host keeps machines (GRPC.md §4), so a machine's agents
    /// list their channel's machines there (D768).
    ListMachines,
    /// And look at one of them there (D768).
    GetMachine,
    /// A machine's agents reach another machine's files through the host,
    /// which holds the links (TP-8, D768): a folder's listing,
    ListDirectory,
    /// one entry,
    Stat,
    /// and a copy or move between machines, such as a pull into its own
    /// `shared/` (D755).
    StartTransfer,
    /// BROWSER (D891): a machine's agents query whether the host Mac's
    /// browser extension is connected.
    GetBrowserBridgeStatus,
    /// BROWSER (D891): a machine's agents execute an Inbox-approved browser
    /// action on the host Mac's connected browser extension.
    ExecuteBrowserAction,
    /// PROVIDERCLI (D906): a machine's callers list providers from the host.
    ListProviders,
    /// PROVIDERCLI (D906): a machine's callers read the host's providers file.
    GetProvidersConfig,
    /// PROVIDERCLI (D906): a machine's callers ask the host owner to import a
    /// machine provider file via the host Inbox.
    RequestProviderImport,
}

/// One row of the table.
#[derive(Debug)]
pub struct Row {
    pub method: Method,
    /// The full gRPC method name, as `ForwardedCall.method` carries it.
    pub name: &'static str,
    /// The operation a journaled write of this method is named in the
    /// journal; none for a read, which is never journaled.
    pub journal: Option<&'static str>,
}

static LIST_ITEMS: Row = Row {
    method: Method::ListItems,
    name: "/humata.v1.BoardService/ListItems",
    journal: None,
};
static GET_ITEM: Row = Row {
    method: Method::GetItem,
    name: "/humata.v1.BoardService/GetItem",
    journal: None,
};
static GET_ITEM_HISTORY: Row = Row {
    method: Method::GetItemHistory,
    name: "/humata.v1.BoardService/GetItemHistory",
    journal: None,
};
static CREATE_ITEM: Row = Row {
    method: Method::CreateItem,
    name: "/humata.v1.BoardService/CreateItem",
    journal: Some("board.item_create"),
};
static UPDATE_ITEM: Row = Row {
    method: Method::UpdateItem,
    name: "/humata.v1.BoardService/UpdateItem",
    journal: Some("board.item_update"),
};
static MOVE_ITEM: Row = Row {
    method: Method::MoveItem,
    name: "/humata.v1.BoardService/MoveItem",
    journal: Some("board.item_move"),
};
static DELETE_ITEM: Row = Row {
    method: Method::DeleteItem,
    name: "/humata.v1.BoardService/DeleteItem",
    journal: Some("board.item_delete"),
};

static WAIT_FOR_ANSWER: Row = Row {
    method: Method::WaitForAnswer,
    name: "/humata.v1.InboxService/WaitForAnswer",
    journal: None,
};
static WITHDRAW_REQUEST: Row = Row {
    method: Method::WithdrawRequest,
    name: "/humata.v1.InboxService/WithdrawRequest",
    journal: None,
};

static ASK_WHY: Row = Row {
    method: Method::AskWhy,
    name: "/humata.v1.InboxService/AskWhy",
    journal: None,
};
static EXPLAIN_ASK: Row = Row {
    method: Method::ExplainAsk,
    name: "/humata.v1.InboxService/ExplainAsk",
    journal: None,
};

static SEARCH_REPOS: Row = Row {
    method: Method::SearchRepos,
    name: "/humata.v1.RepoService/SearchRepos",
    journal: None,
};
static LIST_REPO_LIBRARY: Row = Row {
    method: Method::ListRepoLibrary,
    name: "/humata.v1.RepoService/ListRepoLibrary",
    journal: None,
};

static LIST_MACHINES: Row = Row {
    method: Method::ListMachines,
    name: "/humata.v1.MachineService/ListMachines",
    journal: None,
};
static GET_MACHINE: Row = Row {
    method: Method::GetMachine,
    name: "/humata.v1.MachineService/GetMachine",
    journal: None,
};

static LIST_DIRECTORY: Row = Row {
    method: Method::ListDirectory,
    name: "/humata.v1.FileService/ListDirectory",
    journal: None,
};
static STAT: Row = Row {
    method: Method::Stat,
    name: "/humata.v1.FileService/Stat",
    journal: None,
};
static START_TRANSFER: Row = Row {
    method: Method::StartTransfer,
    name: "/humata.v1.TransferService/StartTransfer",
    journal: None,
};
static GET_BROWSER_BRIDGE_STATUS: Row = Row {
    method: Method::GetBrowserBridgeStatus,
    name: "/humata.v1.BrowserBridgeService/GetBrowserBridgeStatus",
    journal: None,
};
static EXECUTE_BROWSER_ACTION: Row = Row {
    method: Method::ExecuteBrowserAction,
    name: "/humata.v1.BrowserBridgeService/ExecuteBrowserAction",
    journal: None,
};
static LIST_PROVIDERS: Row = Row {
    method: Method::ListProviders,
    name: "/humata.v1.ProviderService/ListProviders",
    journal: None,
};
static GET_PROVIDERS_CONFIG: Row = Row {
    method: Method::GetProvidersConfig,
    name: "/humata.v1.ProviderService/GetProvidersConfig",
    journal: None,
};
static REQUEST_PROVIDER_IMPORT: Row = Row {
    method: Method::RequestProviderImport,
    name: "/humata.v1.ProviderService/RequestProviderImport",
    journal: None,
};

static ROWS: [&Row; 23] = [
    &LIST_ITEMS,
    &GET_ITEM,
    &GET_ITEM_HISTORY,
    &CREATE_ITEM,
    &UPDATE_ITEM,
    &MOVE_ITEM,
    &DELETE_ITEM,
    &WAIT_FOR_ANSWER,
    &WITHDRAW_REQUEST,
    &ASK_WHY,
    &EXPLAIN_ASK,
    &SEARCH_REPOS,
    &LIST_REPO_LIBRARY,
    &LIST_MACHINES,
    &GET_MACHINE,
    &LIST_DIRECTORY,
    &STAT,
    &START_TRANSFER,
    &GET_BROWSER_BRIDGE_STATUS,
    &EXECUTE_BROWSER_ACTION,
    &LIST_PROVIDERS,
    &GET_PROVIDERS_CONFIG,
    &REQUEST_PROVIDER_IMPORT,
];

impl Row {
    /// Whether a call of this row reads a board, so that what the host
    /// answers it with is kept for the host being away (D227). The Inbox
    /// calls are about one request and are never kept.
    pub fn reads_board(&self) -> bool {
        matches!(
            self.method,
            Method::ListItems | Method::GetItem | Method::GetItemHistory
        )
    }
}

impl Method {
    pub fn row(self) -> &'static Row {
        match self {
            Self::ListItems => &LIST_ITEMS,
            Self::GetItem => &GET_ITEM,
            Self::GetItemHistory => &GET_ITEM_HISTORY,
            Self::CreateItem => &CREATE_ITEM,
            Self::UpdateItem => &UPDATE_ITEM,
            Self::MoveItem => &MOVE_ITEM,
            Self::DeleteItem => &DELETE_ITEM,
            Self::WaitForAnswer => &WAIT_FOR_ANSWER,
            Self::WithdrawRequest => &WITHDRAW_REQUEST,
            Self::AskWhy => &ASK_WHY,
            Self::ExplainAsk => &EXPLAIN_ASK,
            Self::SearchRepos => &SEARCH_REPOS,
            Self::ListRepoLibrary => &LIST_REPO_LIBRARY,
            Self::ListMachines => &LIST_MACHINES,
            Self::GetMachine => &GET_MACHINE,
            Self::ListDirectory => &LIST_DIRECTORY,
            Self::Stat => &STAT,
            Self::StartTransfer => &START_TRANSFER,
            Self::GetBrowserBridgeStatus => &GET_BROWSER_BRIDGE_STATUS,
            Self::ExecuteBrowserAction => &EXECUTE_BROWSER_ACTION,
            Self::ListProviders => &LIST_PROVIDERS,
            Self::GetProvidersConfig => &GET_PROVIDERS_CONFIG,
            Self::RequestProviderImport => &REQUEST_PROVIDER_IMPORT,
        }
    }
}

/// The row for a forwarded call's full method name.
pub fn by_name(name: &str) -> Option<&'static Row> {
    ROWS.iter().copied().find(|row| row.name == name)
}

/// The write row a journal entry's operation names.
pub fn by_operation(operation: &str) -> Option<&'static Row> {
    ROWS.iter().copied().find(|row| row.journal == Some(operation))
}

// ---------------------------------------------------------------- the host

/// Why the host won't act for a session on behalf of a machine.
#[derive(Debug, thiserror::Error)]
pub enum Placement {
    #[error(
        "session `{session}` isn't one this Mac knows; it may have been deleted here, so {machine} can't act for it"
    )]
    Unknown { session: String, machine: String },

    #[error("session `{session}` is placed on {placed}, not on {machine}, so {machine} can't act for it")]
    Elsewhere {
        session: String,
        placed: String,
        machine: String,
    },

    #[error(
        "couldn't read session `{session}` from this Mac's store; check that the disk is readable and the state folder is intact, then send the call again"
    )]
    Store {
        session: String,
        #[source]
        source: StoreError,
    },
}

impl Placement {
    /// How a forwarded call from such a session is answered.
    pub fn status(&self) -> Status {
        match self {
            Self::Unknown { session, machine } | Self::Elsewhere { session, machine, .. } => status::failure(
                Code::PermissionDenied,
                status::error(
                    self.to_string(),
                    "start the agent from a session tab of the machine the session is on, or ask for the session to be started again from the app",
                    pb::error::Kind::IdentityMismatch(pb::IdentityMismatch {
                        claimed: Some(pb::SessionId { value: session.clone() }),
                        claimed_machine: Some(pb::MachineId { value: machine.clone() }),
                    }),
                ),
            ),
            Self::Store { .. } => status::internal(status::chain(self), ""),
        }
    }
}

/// `session`'s record, when this Mac places it on `machine`: a machine can
/// only act for the sessions the host put there.
pub fn placed_on(store: &Store, machine: &str, session: &str) -> Result<SessionRecord, Placement> {
    let record = match store.session(session) {
        Ok(Some(record)) => record,
        Ok(None) | Err(StoreError::InvalidId { .. }) => {
            return Err(Placement::Unknown {
                session: session.to_owned(),
                machine: machine.to_owned(),
            });
        }
        Err(source) => {
            return Err(Placement::Store {
                session: session.to_owned(),
                source,
            });
        }
    };
    if record.machine != machine {
        let placed = if record.machine.is_empty() {
            "this Mac".to_owned()
        } else {
            record.machine.clone()
        };
        return Err(Placement::Elsewhere {
            session: session.to_owned(),
            placed,
            machine: machine.to_owned(),
        });
    }
    Ok(record)
}

// ------------------------------------------------------------- the machine

/// Why a forwarded call wasn't answered by the host.
#[derive(Debug, thiserror::Error)]
enum ForwardError {
    #[error("the host isn't linked to this machine, so a call that needs it can't be sent")]
    NotLinked,

    #[error("the link to the host dropped while {method} waited for its answer; the host may or may not have done it")]
    LinkLost { method: &'static str },

    #[error("the host refused {method}")]
    HostSaid { method: &'static str, error: Box<pb::Error> },

    #[error("the host answered {method} with nothing")]
    Empty { method: &'static str },
}

impl From<ForwardError> for Status {
    fn from(error: ForwardError) -> Self {
        match error {
            // The host may have done it before the link dropped: a retry
            // keeps the key so it isn't done twice.
            ForwardError::NotLinked | ForwardError::LinkLost { .. } => status::failure(
                Code::Unavailable,
                pb::Error {
                    retry: Some(pb::RetryAdvice {
                        after: None,
                        same_key: true,
                    }),
                    ..status::error(
                        status::chain(&error),
                        "wait until the host Mac is awake and linked to this machine, then try again with the same idempotency key",
                        pb::error::Kind::HostUnreachable(pb::HostUnreachable { since: None }),
                    )
                },
            ),
            ForwardError::HostSaid { error, .. } => {
                let code = code_of(&error);
                status::failure(code, *error)
            }
            ForwardError::Empty { .. } => status::failure(
                Code::Unavailable,
                status::error(
                    status::chain(&error),
                    "repeat the call with the same idempotency key once the host is linked; the user's answer, if any, is seen then",
                    pb::error::Kind::HostUnreachable(pb::HostUnreachable { since: None }),
                ),
            ),
        }
    }
}

/// The status code a host's `Error` goes with, as the host's own call
/// would have returned it.
fn code_of(error: &pb::Error) -> Code {
    use pb::error::Kind;
    match &error.kind {
        Some(Kind::NotFound(_)) => Code::NotFound,
        Some(Kind::InvalidField(_)) => Code::InvalidArgument,
        Some(Kind::RevisionConflict(_)) => Code::Aborted,
        Some(Kind::NameTaken(_)) => Code::AlreadyExists,
        Some(Kind::Denied(_) | Kind::UserOnly(_) | Kind::PresenceRequired(_) | Kind::IdentityMismatch(_)) => {
            Code::PermissionDenied
        }
        Some(Kind::HostUnreachable(_) | Kind::MachineOffline(_) | Kind::UserAway(_) | Kind::RateLimited(_)) => {
            Code::Unavailable
        }
        Some(Kind::ApprovalPending(_) | Kind::DependencyLoop(_)) => Code::FailedPrecondition,
        Some(Kind::DiskFull(_)) => Code::ResourceExhausted,
        Some(Kind::Internal(_)) => Code::Internal,
        _ => Code::Unknown,
    }
}

#[derive(Default)]
struct State {
    /// The uplink the call goes up on, while the host is linked.
    uplink: Option<Attached>,
    /// How many uplinks have attached; each is numbered by it.
    epochs: u64,
    waiting: HashMap<u64, Waiting>,
    /// The Inbox requests the host answered a call with approval_pending (D268):
    /// they are the host's, which a machine whose host is away can't reach.
    /// Kept in memory only, and bounded.
    held: HashSet<String>,
}

/// The most host-held request ids a machine remembers; past it the set
/// starts over, and an id forgotten is answered as not found.
const HELD_MAX: usize = 4096;

struct Attached {
    epoch: u64,
    to_host: mpsc::Sender<Result<pb::MachineToHost, Status>>,
}

struct Waiting {
    epoch: u64,
    answer: oneshot::Sender<pb::ForwardedResult>,
}

/// A machine's way of sending its agents' calls to the host over the open
/// Uplink and waiting for the host's answer. On the host's own daemon, and
/// on a machine whose host is away, it forwards nothing.
pub struct Forwarder {
    on_host: bool,
    next_call: AtomicU64,
    state: Mutex<State>,
}

impl Forwarder {
    pub fn new(on_host: bool) -> Self {
        Self {
            on_host,
            next_call: AtomicU64::new(0),
            state: Mutex::new(State::default()),
        }
    }

    /// Whether this is a machine's daemon, whose agents' board calls go to
    /// the host while it is linked and are the machine's own while it is away.
    pub fn on_machine(&self) -> bool {
        !self.on_host
    }

    /// Whether a call made now goes to the host: this is a machine and its
    /// host's Uplink is open.
    pub fn forwards(&self) -> bool {
        !self.on_host && lock(&self.state).uplink.is_some()
    }

    /// An Uplink opened: calls go up on `to_host` from now on. Returns the
    /// number [`Self::detach`] takes when that Uplink ends.
    pub fn attach(&self, to_host: mpsc::Sender<Result<pb::MachineToHost, Status>>) -> u64 {
        let mut state = lock(&self.state);
        state.epochs += 1;
        let epoch = state.epochs;
        state.uplink = Some(Attached { epoch, to_host });
        epoch
    }

    /// The Uplink numbered `epoch` ended: the calls waiting on it are
    /// answered as unreachable, and nothing goes up until another opens.
    pub fn detach(&self, epoch: u64) {
        let mut state = lock(&self.state);
        if state.uplink.as_ref().is_some_and(|attached| attached.epoch == epoch) {
            state.uplink = None;
        }
        // Dropping the senders wakes each waiting call.
        state.waiting.retain(|_, waiting| waiting.epoch != epoch);
    }

    /// The host's answer to a call. One for a call that is no longer
    /// waiting (it was cancelled) is dropped.
    pub fn answered(&self, result: pb::ForwardedResult) {
        let waiting = lock(&self.state).waiting.remove(&result.call_id);
        if let Some(waiting) = waiting {
            let _ = waiting.answer.send(result);
        }
    }

    /// Whether the host holds Inbox request `id`, as far as this machine
    /// has seen: a call it forwarded came back approval_pending with it, or a
    /// wait on it was cut by the link.
    pub fn host_holds(&self, id: &str) -> bool {
        lock(&self.state).held.contains(id)
    }

    /// Notes that the host holds Inbox request `id`.
    pub fn remember_held(&self, id: &str) {
        let mut state = lock(&self.state);
        if state.held.len() >= HELD_MAX {
            state.held.clear();
        }
        state.held.insert(id.to_owned());
    }

    /// Sends `request` to `method`'s host for the agent session `session`
    /// and returns the host's response bytes. A call that can't reach the
    /// host, or whose link drops before the answer, is host_unreachable and
    /// retryable with the same key.
    pub async fn call(
        &self,
        session: &str,
        row: &'static Row,
        request: Vec<u8>,
        key: Option<pb::IdempotencyKey>,
    ) -> Result<Vec<u8>, Status> {
        let principal = pb::forwarded_call::Principal::Agent(pb::AgentPrincipal {
            session: Some(pb::SessionId {
                value: session.to_owned(),
            }),
            pid: 0,
        });
        Ok(self.try_call(principal, row, request, key).await?)
    }

    /// Sends `request` to `method`'s host for a user authenticated on this
    /// machine outside every session terminal (for example Ask why on a
    /// request the host holds), and returns the host's response bytes.
    pub async fn call_as_user(
        &self,
        row: &'static Row,
        request: Vec<u8>,
        key: Option<pb::IdempotencyKey>,
    ) -> Result<Vec<u8>, Status> {
        let principal = pb::forwarded_call::Principal::User(pb::MachineUser {
            presence: None,
            outside_sessions: true,
        });
        Ok(self.try_call(principal, row, request, key).await?)
    }

    async fn try_call(
        &self,
        principal: pb::forwarded_call::Principal,
        row: &'static Row,
        request: Vec<u8>,
        key: Option<pb::IdempotencyKey>,
    ) -> Result<Vec<u8>, ForwardError> {
        let (call_id, to_host, answer) = {
            let mut state = lock(&self.state);
            let Some(uplink) = state.uplink.as_ref() else {
                return Err(ForwardError::NotLinked);
            };
            let (epoch, to_host) = (uplink.epoch, uplink.to_host.clone());
            let call_id = self.next_call.fetch_add(1, Ordering::Relaxed) + 1;
            let (sender, receiver) = oneshot::channel();
            state.waiting.insert(call_id, Waiting { epoch, answer: sender });
            (call_id, to_host, receiver)
        };
        let mut leaving = Leaving {
            forwarder: self,
            call_id,
            to_host: to_host.clone(),
            answered: false,
        };
        let call = pb::ForwardedCall {
            call_id,
            principal: Some(principal),
            method: row.name.to_owned(),
            request,
            deadline: None,
            idempotency_key: key,
        };
        let message = pb::MachineToHost {
            message: Some(pb::machine_to_host::Message::Call(call)),
        };
        to_host
            .send(Ok(message))
            .await
            .map_err(|_| ForwardError::LinkLost { method: row.name })?;
        let result = answer.await.map_err(|_| ForwardError::LinkLost { method: row.name })?;
        leaving.answered = true;
        match result.outcome {
            Some(pb::forwarded_result::Outcome::Response(bytes)) => Ok(bytes),
            Some(pb::forwarded_result::Outcome::Error(error)) => {
                if let Some(pb::error::Kind::ApprovalPending(pending)) = &error.kind
                    && let Some(request) = &pending.request
                {
                    self.remember_held(&request.value);
                }
                Err(ForwardError::HostSaid {
                    method: row.name,
                    error: Box::new(error),
                })
            }
            None => Err(ForwardError::Empty { method: row.name }),
        }
    }
}

/// A forwarded call that is no longer waited on: it stops waiting, and the
/// host is told to drop it if it hadn't answered (the caller went away).
struct Leaving<'a> {
    forwarder: &'a Forwarder,
    call_id: u64,
    to_host: mpsc::Sender<Result<pb::MachineToHost, Status>>,
    answered: bool,
}

impl Drop for Leaving<'_> {
    fn drop(&mut self) {
        lock(&self.forwarder.state).waiting.remove(&self.call_id);
        if !self.answered {
            let cancel = pb::MachineToHost {
                message: Some(pb::machine_to_host::Message::Cancel(pb::ForwardedCallCancel {
                    call_id: self.call_id,
                })),
            };
            // The uplink may be gone or full; the host drops the call with it.
            let _ = self.to_host.try_send(Ok(cancel));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use humata2_store::SessionRecord;

    use super::*;

    const BOUND: Duration = Duration::from_secs(5);

    fn session(id: &str, machine: &str) -> SessionRecord {
        SessionRecord {
            id: id.to_owned(),
            channel: "ch_1".to_owned(),
            machine: machine.to_owned(),
            title: "Fix the build".to_owned(),
            agent: "claude".to_owned(),
            folder: std::path::PathBuf::from("/work/web"),
            created_unix_ms: 1,
        }
    }

    fn host_unreachable(status: &Status) -> bool {
        matches!(
            status::status_error_of(status).and_then(|error| error.kind),
            Some(pb::error::Kind::HostUnreachable(_))
        )
    }

    #[test]
    fn a_write_row_has_its_journal_name_and_a_read_has_none() {
        assert_eq!(
            by_operation("board.item_move").map(|row| row.method),
            Some(Method::MoveItem)
        );
        assert!(by_operation("board.item_list").is_none());
        assert!(by_operation("inbox.request").is_none());
        let read = by_name("/humata.v1.BoardService/GetItem");
        assert_eq!(read.map(|row| row.method), Some(Method::GetItem));
        assert!(read.is_some_and(|row| row.journal.is_none()));
        assert!(by_name("/humata.v1.BoardService/Nothing").is_none());
    }

    #[test]
    fn the_ask_why_calls_are_forwarded_reads_that_are_never_journaled() {
        for (name, method) in [
            ("/humata.v1.InboxService/AskWhy", Method::AskWhy),
            ("/humata.v1.InboxService/ExplainAsk", Method::ExplainAsk),
        ] {
            let row = by_name(name).expect("registered");
            assert_eq!(row.method, method);
            assert!(row.journal.is_none() && !row.reads_board());
        }
    }

    #[test]
    fn every_method_finds_its_own_row_by_name() {
        for method in [
            Method::ListItems,
            Method::GetItem,
            Method::GetItemHistory,
            Method::CreateItem,
            Method::UpdateItem,
            Method::MoveItem,
            Method::DeleteItem,
            Method::WaitForAnswer,
            Method::WithdrawRequest,
            Method::AskWhy,
            Method::ExplainAsk,
            Method::SearchRepos,
            Method::ListRepoLibrary,
            Method::ListMachines,
            Method::GetMachine,
            Method::ListDirectory,
            Method::Stat,
            Method::StartTransfer,
            Method::GetBrowserBridgeStatus,
            Method::ExecuteBrowserAction,
            Method::ListProviders,
            Method::GetProvidersConfig,
            Method::RequestProviderImport,
        ] {
            assert_eq!(by_name(method.row().name).map(|row| row.method), Some(method));
        }
    }

    #[test]
    fn a_session_the_host_places_on_the_machine_is_accepted_and_others_are_not() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path()).unwrap();
        store.put_session(&session("se_a", "ma_a")).unwrap();
        store.put_session(&session("se_b", "ma_b")).unwrap();
        store.put_session(&session("se_here", "")).unwrap();
        assert!(placed_on(&store, "ma_a", "se_a").is_ok());
        assert!(matches!(
            placed_on(&store, "ma_a", "se_b"),
            Err(Placement::Elsewhere { .. })
        ));
        assert!(matches!(
            placed_on(&store, "ma_a", "se_here"),
            Err(Placement::Elsewhere { .. })
        ));
        assert!(matches!(
            placed_on(&store, "ma_a", "se_missing"),
            Err(Placement::Unknown { .. })
        ));
        assert!(matches!(
            placed_on(&store, "ma_a", "not a valid id/"),
            Err(Placement::Unknown { .. })
        ));
    }

    #[tokio::test]
    async fn with_no_uplink_a_call_is_host_unreachable_and_nothing_forwards() {
        let forwarder = Forwarder::new(false);
        assert!(!forwarder.forwards());
        let failed = forwarder
            .call("se_a", Method::CreateItem.row(), Vec::new(), None)
            .await
            .unwrap_err();
        assert!(host_unreachable(&failed));
    }

    #[test]
    fn the_hosts_own_daemon_forwards_nothing_even_with_an_uplink() {
        let forwarder = Forwarder::new(true);
        let (to_host, _from) = mpsc::channel(4);
        forwarder.attach(to_host);
        assert!(!forwarder.forwards());
    }

    #[tokio::test]
    async fn a_call_goes_up_with_its_principal_and_key_and_gets_the_hosts_response() {
        let forwarder = Arc::new(Forwarder::new(false));
        let (to_host, mut from_machine) = mpsc::channel(4);
        forwarder.attach(to_host);
        assert!(forwarder.forwards());
        let key = pb::IdempotencyKey { value: "k1".to_owned() };
        let waiting = {
            let forwarder = Arc::clone(&forwarder);
            let key = key.clone();
            tokio::spawn(async move {
                forwarder
                    .call("se_a", Method::UpdateItem.row(), vec![1, 2, 3], Some(key))
                    .await
            })
        };
        let sent = tokio::time::timeout(BOUND, from_machine.recv())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let Some(pb::machine_to_host::Message::Call(call)) = sent.message else {
            panic!("the machine sent a call");
        };
        assert_eq!(call.method, "/humata.v1.BoardService/UpdateItem");
        assert_eq!(call.request, vec![1, 2, 3]);
        assert_eq!(call.idempotency_key, Some(key));
        let Some(pb::forwarded_call::Principal::Agent(agent)) = call.principal else {
            panic!("an agent made the call");
        };
        assert_eq!(agent.session.map(|session| session.value).as_deref(), Some("se_a"));
        forwarder.answered(pb::ForwardedResult {
            call_id: call.call_id,
            outcome: Some(pb::forwarded_result::Outcome::Response(vec![9, 9])),
        });
        let answer = tokio::time::timeout(BOUND, waiting).await.unwrap().unwrap();
        assert_eq!(answer.unwrap(), vec![9, 9]);
    }

    #[tokio::test]
    async fn two_calls_waiting_each_get_their_own_answer() {
        let forwarder = Arc::new(Forwarder::new(false));
        let (to_host, mut from_machine) = mpsc::channel(4);
        forwarder.attach(to_host);
        let spawn = |body: u8| {
            let forwarder = Arc::clone(&forwarder);
            tokio::spawn(async move {
                forwarder
                    .call("se_a", Method::CreateItem.row(), vec![body], None)
                    .await
            })
        };
        let (first, second) = (spawn(1), spawn(2));
        let mut calls = Vec::new();
        for _ in 0..2 {
            let sent = tokio::time::timeout(BOUND, from_machine.recv())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            let Some(pb::machine_to_host::Message::Call(call)) = sent.message else {
                panic!("the machine sent a call");
            };
            calls.push(call);
        }
        // Answered in the opposite order, each with its request echoed.
        for call in calls.iter().rev() {
            forwarder.answered(pb::ForwardedResult {
                call_id: call.call_id,
                outcome: Some(pb::forwarded_result::Outcome::Response(call.request.clone())),
            });
        }
        assert_eq!(first.await.unwrap().unwrap(), vec![1]);
        assert_eq!(second.await.unwrap().unwrap(), vec![2]);
    }

    #[tokio::test]
    async fn a_link_that_drops_mid_call_is_host_unreachable() {
        let forwarder = Arc::new(Forwarder::new(false));
        let (to_host, mut from_machine) = mpsc::channel(4);
        let epoch = forwarder.attach(to_host);
        let waiting = {
            let forwarder = Arc::clone(&forwarder);
            tokio::spawn(async move {
                forwarder
                    .call("se_a", Method::CreateItem.row(), Vec::new(), None)
                    .await
            })
        };
        tokio::time::timeout(BOUND, from_machine.recv()).await.unwrap().unwrap().unwrap();
        forwarder.detach(epoch);
        let failed = tokio::time::timeout(BOUND, waiting).await.unwrap().unwrap().unwrap_err();
        assert!(host_unreachable(&failed));
        assert!(!forwarder.forwards());
    }

    #[tokio::test]
    async fn a_newer_uplink_keeps_forwarding_when_the_older_one_ends() {
        let forwarder = Forwarder::new(false);
        let (first, _first_end) = mpsc::channel(4);
        let older = forwarder.attach(first);
        let (second, _second_end) = mpsc::channel(4);
        forwarder.attach(second);
        forwarder.detach(older);
        assert!(forwarder.forwards());
    }

    #[tokio::test]
    async fn a_request_the_host_answers_approval_pending_is_remembered_as_the_hosts() {
        let forwarder = Arc::new(Forwarder::new(false));
        let (to_host, mut from_machine) = mpsc::channel(4);
        forwarder.attach(to_host);
        let waiting = {
            let forwarder = Arc::clone(&forwarder);
            tokio::spawn(async move {
                forwarder
                    .call("se_a", Method::UpdateItem.row(), Vec::new(), None)
                    .await
            })
        };
        let sent = tokio::time::timeout(BOUND, from_machine.recv())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let Some(pb::machine_to_host::Message::Call(call)) = sent.message else {
            panic!("the machine sent a call");
        };
        assert!(!forwarder.host_holds("rq_1"));
        forwarder.answered(pb::ForwardedResult {
            call_id: call.call_id,
            outcome: Some(pb::forwarded_result::Outcome::Error(pb::Error {
                message: "waiting".to_owned(),
                fix: "wait".to_owned(),
                retry: None,
                kind: Some(pb::error::Kind::ApprovalPending(pb::ApprovalPending {
                    request: Some(pb::RequestId {
                        value: "rq_1".to_owned(),
                    }),
                })),
            })),
        });
        let failed = tokio::time::timeout(BOUND, waiting).await.unwrap().unwrap().unwrap_err();
        assert_eq!(failed.code(), Code::FailedPrecondition);
        assert!(forwarder.host_holds("rq_1"));
        assert!(!forwarder.host_holds("rq_2"));
    }

    #[tokio::test]
    async fn a_call_that_is_dropped_tells_the_host_to_cancel_it() {
        let forwarder = Arc::new(Forwarder::new(false));
        let (to_host, mut from_machine) = mpsc::channel(4);
        forwarder.attach(to_host);
        let waiting = {
            let forwarder = Arc::clone(&forwarder);
            tokio::spawn(async move {
                forwarder
                    .call("se_a", Method::CreateItem.row(), Vec::new(), None)
                    .await
            })
        };
        let sent = tokio::time::timeout(BOUND, from_machine.recv())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let Some(pb::machine_to_host::Message::Call(call)) = sent.message else {
            panic!("the machine sent a call");
        };
        waiting.abort();
        let cancel = tokio::time::timeout(BOUND, from_machine.recv())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(matches!(
            cancel.message,
            Some(pb::machine_to_host::Message::Cancel(cancelled)) if cancelled.call_id == call.call_id
        ));
    }

    #[test]
    fn an_error_the_host_returned_keeps_its_status_code() {
        let host_says = |error: pb::Error| -> Code {
            let status: Status = ForwardError::HostSaid {
                method: "/humata.v1.BoardService/GetItem",
                error: Box::new(error),
            }
            .into();
            status.code()
        };
        let kind = |kind| pb::Error {
            message: "no".to_owned(),
            fix: "do it differently".to_owned(),
            retry: None,
            kind: Some(kind),
        };
        assert_eq!(
            host_says(kind(pb::error::Kind::NotFound(pb::NotFound::default()))),
            Code::NotFound
        );
        assert_eq!(
            host_says(kind(pb::error::Kind::RevisionConflict(pb::RevisionConflict::default()))),
            Code::Aborted
        );
        assert_eq!(
            host_says(kind(pb::error::Kind::HostUnreachable(pb::HostUnreachable::default()))),
            Code::Unavailable
        );
    }

    #[tokio::test]
    async fn a_user_call_goes_up_with_a_user_principal_and_gets_the_hosts_response() {
        let forwarder = Arc::new(Forwarder::new(false));
        let (to_host, mut from_machine) = mpsc::channel(4);
        forwarder.attach(to_host);
        let waiting = {
            let forwarder = Arc::clone(&forwarder);
            tokio::spawn(async move {
                forwarder
                    .call_as_user(Method::AskWhy.row(), vec![4, 5], None)
                    .await
            })
        };
        let sent = tokio::time::timeout(BOUND, from_machine.recv())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let Some(pb::machine_to_host::Message::Call(call)) = sent.message else {
            panic!("the machine sent a call");
        };
        assert_eq!(call.method, "/humata.v1.InboxService/AskWhy");
        assert_eq!(call.request, vec![4, 5]);
        let Some(pb::forwarded_call::Principal::User(user)) = call.principal else {
            panic!("a user made the call");
        };
        assert!(user.outside_sessions);
        forwarder.answered(pb::ForwardedResult {
            call_id: call.call_id,
            outcome: Some(pb::forwarded_result::Outcome::Response(vec![7, 8])),
        });
        let answer = tokio::time::timeout(BOUND, waiting).await.unwrap().unwrap();
        assert_eq!(answer.unwrap(), vec![7, 8]);
    }
}
