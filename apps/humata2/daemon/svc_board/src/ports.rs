//! Narrow port traits and value types used by `humata2_svc_board` so the crate
//! compiles standalone without importing sibling services from `humatad`.

use std::sync::Arc;

use humata2_access::{AccessRequest, Call, ChannelId, MachineId, Refusal, SessionId, SessionRef};
use humata2_daemon_ports::{AgentCaller, BoardAsk, Caller, Run};
use crate::pb;
use humata2_rpc_kit::{
    answer_key_for_actor, chain, error, failure, internal, status_error_of,
};
use humata2_store::{SessionRecord, Store, StoreError};
use tonic::{Code, Request, Response, Status};

// ---------------------------------------------------------------------------
// Identity & status helpers
// ---------------------------------------------------------------------------

/// Extracts the authenticated [`Caller`] from a tonic request's extensions.
pub fn caller_of<T>(request: &Request<T>) -> Result<Caller, Status> {
    request
        .extensions()
        .get::<Caller>()
        .cloned()
        .ok_or_else(|| Status::unauthenticated("missing caller identity"))
}

/// Formats an idempotency key for `caller`.
pub fn answer_key(
    call: &str,
    caller: &Caller,
    key: Option<&pb::IdempotencyKey>,
    what: &str,
) -> Option<String> {
    answer_key_for_actor(call, &humata2_daemon_ports::actor(caller), key, what)
}

/// Converts a policy [`Refusal`] into a gRPC [`Status`].
pub fn refused(refusal: &Refusal) -> Status {
    let message = refusal.to_string();
    failure(
        Code::PermissionDenied,
        error(
            message,
            "",
            pb::error::Kind::Denied(pb::Denied::default()),
        ),
    )
}

/// Converts a [`StoreError`] into a gRPC [`Status`].
pub fn store_error(err: StoreError, machine: &str, doing: &str) -> Status {
    internal(
        format!("couldn't {doing} on {machine}: {}", chain(&err)),
        "",
    )
}

/// Extracts `pb::Error` from a `Status`, if present.
pub fn error_of_status(status: &Status) -> Option<pb::Error> {
    status_error_of(status)
}

/// Builds an [`AgentCaller`] from a stored [`SessionRecord`].
pub fn agent_caller_of(record: &SessionRecord, machine: &str) -> AgentCaller {
    AgentCaller {
        session: SessionRef {
            id: SessionId::new(record.id.as_str()),
            channel: ChannelId::new(record.channel.as_str()),
            machine: MachineId::new(if record.machine.is_empty() {
                machine
            } else {
                record.machine.as_str()
            }),
        },
        agent: record.agent.clone(),
        title: record.title.clone(),
        method: String::new(),
        context_only_parent: None,
        desk: None,
        review_push: None,
        channel_desk: None,
    }
}

/// Resolves fork metadata on `agent` from `store`.
pub fn resolve_fork(mut agent: AgentCaller, store: &Store) -> Result<AgentCaller, StoreError> {
    if let Some(origin) = store
        .fork_origins()?
        .into_iter()
        .find(|origin| origin.session_id == agent.session.id.as_str())
    {
        if origin.kind == pb::ForkKind::ContextOnly as i32 {
            agent.context_only_parent = Some(SessionId::new(origin.parent.as_str()));
        }
    }
    Ok(agent)
}

// ---------------------------------------------------------------------------
// Guard port
// ---------------------------------------------------------------------------

/// Policy check outcome for a normal call.
#[derive(Clone, Debug)]
pub enum Checked {
    Allowed,
    Ask(Box<AccessRequest>),
}

/// Policy check outcome for a call made while the host is away.
#[derive(Clone, Debug)]
pub enum AwayCall {
    Journal,
    Asks(Box<AccessRequest>),
    LastCopy,
    Reachable,
}

/// Narrow policy-guard port required by [`crate::BoardSvc`].
pub trait BoardGuard: Send + Sync {
    fn check(&self, caller: &Caller, call: &Call) -> Result<Checked, Status>;
    fn check_away(&self, caller: &Caller, call: &Call) -> Result<AwayCall, Status>;
    fn lists_low_risk(&self, asked: &AccessRequest) -> bool;
    fn policy_changed(&self);
    fn host_link(&self) -> String;
}

/// Checks `call` against `guard`, returning `Ok(None)` when allowed or already
/// approved, and `Ok(Some(asked))` when the rule sends the call to Ask me.
pub fn gate(
    guard: &dyn BoardGuard,
    caller: &Caller,
    call: &Call,
    approved: bool,
) -> Result<Option<Box<AccessRequest>>, Status> {
    match guard.check(caller, call)? {
        Checked::Allowed => Ok(None),
        Checked::Ask(_) if approved => Ok(None),
        Checked::Ask(asked) => Ok(Some(asked)),
    }
}

// ---------------------------------------------------------------------------
// Event feed port
// ---------------------------------------------------------------------------

/// Event routing scope on the daemon's event feed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scope {
    pub channel: String,
    pub session: String,
}

/// Narrow event-feed port required by [`crate::BoardSvc`].
pub trait BoardFeed: Send + Sync {
    fn publish_in(
        &self,
        change: pb::Change,
        payload: pb::event::Payload,
        scope: Scope,
        by: Option<pb::Actor>,
    );
}

// ---------------------------------------------------------------------------
// Inbox port
// ---------------------------------------------------------------------------

/// Outcome of replaying a journaled entry on the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Replayed {
    Applied,
    Rejected,
    Held,
}

/// Narrow inbox port required by [`crate::BoardSvc`].
#[tonic::async_trait]
pub trait BoardInbox: Send + Sync {
    async fn withdraw_about_item(&self, key: &str) -> Result<(), Status>;
    async fn defer_board(
        &self,
        caller: &Caller,
        asked: AccessRequest,
        idempotency: Option<String>,
        board: BoardAsk,
        run: Run,
    ) -> Status;
    async fn pending_board_asks(&self) -> Vec<pb::InboxRequest>;
    async fn wait_final(
        &self,
        caller: Caller,
        request: pb::WaitForAnswerRequest,
    ) -> Result<pb::WaitResponse, Status>;
    async fn withdraw_as(
        &self,
        caller: Caller,
        request: pb::WithdrawRequestRequest,
    ) -> Result<pb::InboxRequest, Status>;
    async fn ask_why_as(
        &self,
        caller: Caller,
        request: pb::AskWhyRequest,
    ) -> Result<pb::InboxRequest, Status>;
    async fn explain_as(
        &self,
        caller: Caller,
        request: pb::ExplainAskRequest,
    ) -> Result<pb::InboxRequest, Status>;
    async fn approved_on(
        &self,
        machine: &str,
        id: &str,
        session: &str,
    ) -> Option<pb::InboxRequest>;
}

// ---------------------------------------------------------------------------
// Away / forwarding port
// ---------------------------------------------------------------------------

/// How a read answered from the last copy says how old it is (D227): the
/// one header the MCP bridge and clients read.
pub use humata2_svc_away::stale;

/// Methods that may be forwarded between a machine and the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    ListItems,
    GetItem,
    GetItemHistory,
    CreateItem,
    UpdateItem,
    MoveItem,
    DeleteItem,
    WaitForAnswer,
    WithdrawRequest,
    AskWhy,
    ExplainAsk,
    SearchRepos,
    ListRepoLibrary,
    ListMachines,
    GetMachine,
    ListDirectory,
    Stat,
    StartTransfer,
}

/// Static descriptor for a forwarded / journaled method.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub method: Method,
    pub name: &'static str,
    pub journal: Option<&'static str>,
}

impl Row {
    pub fn reads_board(&self) -> bool {
        matches!(
            self.method,
            Method::ListItems | Method::GetItem | Method::GetItemHistory
        )
    }
}

const ROWS: &[Row] = &[
    Row {
        method: Method::ListItems,
        name: "/humata.v1.BoardService/ListItems",
        journal: None,
    },
    Row {
        method: Method::GetItem,
        name: "/humata.v1.BoardService/GetItem",
        journal: None,
    },
    Row {
        method: Method::GetItemHistory,
        name: "/humata.v1.BoardService/GetItemHistory",
        journal: None,
    },
    Row {
        method: Method::CreateItem,
        name: "/humata.v1.BoardService/CreateItem",
        journal: Some("board.item_create"),
    },
    Row {
        method: Method::UpdateItem,
        name: "/humata.v1.BoardService/UpdateItem",
        journal: Some("board.item_update"),
    },
    Row {
        method: Method::MoveItem,
        name: "/humata.v1.BoardService/MoveItem",
        journal: Some("board.item_move"),
    },
    Row {
        method: Method::DeleteItem,
        name: "/humata.v1.BoardService/DeleteItem",
        journal: Some("board.item_delete"),
    },
    Row {
        method: Method::WaitForAnswer,
        name: "/humata.v1.InboxService/WaitForAnswer",
        journal: None,
    },
    Row {
        method: Method::WithdrawRequest,
        name: "/humata.v1.InboxService/WithdrawRequest",
        journal: None,
    },
    Row {
        method: Method::AskWhy,
        name: "/humata.v1.InboxService/AskWhy",
        journal: None,
    },
    Row {
        method: Method::ExplainAsk,
        name: "/humata.v1.InboxService/ExplainAsk",
        journal: None,
    },
    Row {
        method: Method::SearchRepos,
        name: "/humata.v1.RepoService/SearchRepos",
        journal: None,
    },
    Row {
        method: Method::ListRepoLibrary,
        name: "/humata.v1.RepoService/ListRepoLibrary",
        journal: None,
    },
    Row {
        method: Method::ListMachines,
        name: "/humata.v1.MachineService/ListMachines",
        journal: None,
    },
    Row {
        method: Method::GetMachine,
        name: "/humata.v1.MachineService/GetMachine",
        journal: None,
    },
    Row {
        method: Method::ListDirectory,
        name: "/humata.v1.FileService/ListDirectory",
        journal: None,
    },
    Row {
        method: Method::Stat,
        name: "/humata.v1.FileService/Stat",
        journal: None,
    },
    Row {
        method: Method::StartTransfer,
        name: "/humata.v1.TransferService/StartTransfer",
        journal: None,
    },
];

impl Method {
    pub fn row(self) -> &'static Row {
        ROWS.iter()
            .find(|row| row.method == self)
            .expect("every Method variant has a Row")
    }
}

pub fn by_name(name: &str) -> Option<&'static Row> {
    ROWS.iter().find(|row| row.name == name)
}

pub fn by_operation(operation: &str) -> Option<&'static Row> {
    ROWS.iter().find(|row| row.journal == Some(operation))
}

/// Owned-item entry from a machine's policy copy.
#[derive(Clone, Debug)]
pub struct OwnedItem {
    pub board: Option<pb::BoardRef>,
}

/// Narrow forwarding and away-journal port required by [`crate::BoardSvc`].
#[tonic::async_trait]
pub trait BoardForwarder: Send + Sync {
    fn on_machine(&self) -> bool;
    fn forwards(&self) -> bool;
    async fn call(
        &self,
        session: &str,
        row: &'static Row,
        request: Vec<u8>,
        key: Option<pb::IdempotencyKey>,
    ) -> Result<Vec<u8>, Status>;
    async fn record_board_journal(
        &self,
        store: &Store,
        host_link: &str,
        session: &str,
        operation: &'static str,
        bytes: Vec<u8>,
        base: Option<u64>,
        approved: Option<String>,
    ) -> Result<u64, Status>;
    fn owned_item(
        &self,
        store: &Store,
        owner: &str,
        item: &str,
    ) -> Result<Option<OwnedItem>, Status>;
}

/// Why a session isn't placed on the machine that forwarded or journaled a call.
#[derive(Debug, thiserror::Error)]
pub enum Placement {
    #[error("there is no session `{session}` on this Mac")]
    Missing { session: String },
    #[error("session `{session}` is placed on `{actual}`, not `{expected}`")]
    WrongMachine {
        session: String,
        expected: String,
        actual: String,
    },
    #[error("couldn't read session `{session}` from this Mac's store")]
    Store {
        session: String,
        #[source]
        source: StoreError,
    },
}

impl Placement {
    pub fn status(&self) -> Status {
        match self {
            Self::Missing { session } => failure(
                Code::PermissionDenied,
                error(
                    self.to_string(),
                    "check the session id",
                    pb::error::Kind::IdentityMismatch(pb::IdentityMismatch {
                        claimed: Some(pb::SessionId {
                            value: session.clone(),
                        }),
                        claimed_machine: None,
                    }),
                ),
            ),
            Self::WrongMachine {
                session, expected, ..
            } => failure(
                Code::PermissionDenied,
                error(
                    self.to_string(),
                    "run the call from the machine that holds the session",
                    pb::error::Kind::IdentityMismatch(pb::IdentityMismatch {
                        claimed: Some(pb::SessionId {
                            value: session.clone(),
                        }),
                        claimed_machine: Some(pb::MachineId {
                            value: expected.clone(),
                        }),
                    }),
                ),
            ),
            Self::Store { source, .. } => internal(
                format!("couldn't read the session: {}", chain(source)),
                "",
            ),
        }
    }
}

/// Checks that `session` exists in `store` and is placed on `machine`.
pub fn placed_on(store: &Store, machine: &str, session: &str) -> Result<SessionRecord, Placement> {
    match store.session(session) {
        Ok(Some(record)) if record.machine == machine => Ok(record),
        Ok(Some(record)) => Err(Placement::WrongMachine {
            session: session.to_owned(),
            expected: machine.to_owned(),
            actual: record.machine,
        }),
        Ok(None) | Err(StoreError::InvalidId { .. }) => Err(Placement::Missing {
            session: session.to_owned(),
        }),
        Err(source) => Err(Placement::Store {
            session: session.to_owned(),
            source,
        }),
    }
}

/// Rewrites an unqualified file location sent over `machine`'s link so it
/// names `machine`.
pub fn on_link(machine: &str, mut location: pb::FileLocation) -> pb::FileLocation {
    if location.machine.is_none() {
        location.machine = Some(pb::MachineId {
            value: machine.to_owned(),
        });
    }
    location
}

// ---------------------------------------------------------------------------
// Sibling host service ports (for forwarded calls & replay notifications)
// ---------------------------------------------------------------------------

/// Notifies the session feed when a replay updates a session's left-out summary.
#[tonic::async_trait]
pub trait SessionTell: Send + Sync {
    async fn session(&self, id: &str, change: pb::Change, by: Option<pb::Actor>);
}

/// Host repo library port for forwarded repo searches and library lists.
#[tonic::async_trait]
pub trait HostRepos: Send + Sync {
    async fn search_repos(
        &self,
        request: Request<pb::SearchReposRequest>,
    ) -> Result<Response<pb::SearchReposResponse>, Status>;
    async fn list_repo_library(
        &self,
        request: Request<pb::ListRepoLibraryRequest>,
    ) -> Result<Response<pb::ListRepoLibraryResponse>, Status>;
}

/// Host machine service port for forwarded machine lookups.
#[tonic::async_trait]
pub trait HostMachines: Send + Sync {
    async fn list_for_machine_agent(
        &self,
        request: Request<pb::ListMachinesRequest>,
    ) -> Result<Response<pb::ListMachinesResponse>, Status>;
    async fn get_machine(
        &self,
        request: Request<pb::GetMachineRequest>,
    ) -> Result<Response<pb::MachineDetail>, Status>;
}

/// Host file service port for forwarded directory listings and stats.
#[tonic::async_trait]
pub trait HostFiles: Send + Sync {
    async fn list_directory(
        &self,
        request: Request<pb::ListDirectoryRequest>,
    ) -> Result<Response<pb::ListDirectoryResponse>, Status>;
    async fn stat(
        &self,
        request: Request<pb::StatRequest>,
    ) -> Result<Response<pb::FileEntry>, Status>;
}

/// Host transfer service port for forwarded transfers.
#[tonic::async_trait]
pub trait HostTransfers: Send + Sync {
    async fn start_transfer(
        &self,
        request: Request<pb::StartTransferRequest>,
    ) -> Result<Response<pb::Transfer>, Status>;
}

/// The host's services that answer a machine's agents' lookups, file reads
/// and transfers, as those agents (D768).
pub struct HostCalls {
    pub machines: Arc<dyn HostMachines>,
    pub files: Arc<dyn HostFiles>,
    pub transfers: Arc<dyn HostTransfers>,
}

// ---------------------------------------------------------------------------
// Pull requests that ask for the user's review
// ---------------------------------------------------------------------------

/// A pull request someone asked the user to review.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoundPr {
    pub owner: String,
    pub name: String,
    pub number: u32,
    pub title: String,
    pub author: String,
    pub url: String,
    pub head_ref: String,
    pub base_ref: String,
    /// When the review was asked, in Unix milliseconds.
    pub requested_unix_ms: i64,
}

/// Why the pull requests that ask for the user's review couldn't be read.
#[derive(Debug, thiserror::Error)]
pub enum ReviewFetchError {
    #[error(
        "Humata couldn't run {program} to look for pull requests that ask for your review ({source}). Install the GitHub CLI on this host (brew install gh); if it already runs in your terminal, put its folder on PATH in your login profile (~/.zprofile), because the daemon reads your login shell's PATH when it starts. Review requests in your To-do stay as they are until then"
    )]
    GhMissing {
        program: String,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "the GitHub CLI on this host is signed out, so review requests can't be read. In a terminal on this host, run `gh auth login`. Review requests in your To-do stay as they are until then"
    )]
    SignedOut { stderr: String },
    #[error(
        "GitHub is limiting requests, so review requests weren't read. Wait {wait}, then they are read again on their own; signing in again doesn't lift the limit",
        wait = wait_text(*wait_seconds)
    )]
    RateLimited { wait_seconds: Option<u64> },
    #[error(
        "GitHub couldn't be reached to read review requests ({first_line}). Nothing on this machine fixes it: they are read again when the host is online"
    )]
    Unreachable { first_line: String },
    #[error(
        "GitHub's answer about review requests isn't one this Humata can read. Update the GitHub CLI on the host (brew upgrade gh); if it keeps failing, report it"
    )]
    Unreadable {
        #[source]
        source: serde_json::Error,
    },
}

fn wait_text(seconds: Option<u64>) -> String {
    match seconds {
        Some(seconds) => format!("{seconds} seconds"),
        None => "a few minutes".to_owned(),
    }
}

/// Where the pull requests that ask for the user's review come from.
#[tonic::async_trait]
pub trait ReviewRequestsPort: Send + Sync {
    /// Every open pull request whose review the user was asked for.
    async fn fetch(&self) -> Result<Vec<FoundPr>, ReviewFetchError>;
}

// ---------------------------------------------------------------------------
// Live session events, To-do automation and PR watching (TODO2WIRE)
// ---------------------------------------------------------------------------

/// What the To-do list sees of a session's status. The wiring maps the
/// daemon's `SessionStatus` onto it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SeenStatus {
    #[default]
    Unknown,
    Starting,
    Working,
    NeedsInput,
    Idle,
    /// The output went quiet; the turn has probably ended (SE-22).
    ProbablyIdle,
    Exited,
    /// The person stopped the agent (D784).
    Stopped,
    ResumeFailed,
    Archived,
    /// Its machine can't be reached or is stopped: nothing is known.
    Offline,
}

/// One session as the To-do list sees it, after a change on the feed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionSeen {
    pub id: String,
    pub title: String,
    /// The agent kind, such as `claude`; empty for a plain shell.
    pub agent: String,
    pub model: String,
    pub status: SeenStatus,
    pub exit_code: i32,
    /// The board item the session was started for, if any.
    pub board_item: Option<String>,
    /// When the agent last printed something, in Unix milliseconds.
    pub last_output_ms: Option<i64>,
}

/// Ask (a quiet 1-click prompt) or Auto (done, with Undo where it can be).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SyncMode {
    #[default]
    Ask,
    Auto,
}

/// The To-do automation settings in effect (Settings, or a channel's own).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TodoAutomation {
    pub on_pr_merged: SyncMode,
    pub on_todo_closed_archive_sessions: SyncMode,
    pub auto_wake_on_ci_fail: bool,
    pub auto_wake_on_review_comments: bool,
    /// Days of inactivity before the idle nudge.
    pub idle_nudge_days: u32,
}

/// The idle nudge's default, when Settings leave it at 0.
pub const IDLE_NUDGE_DAYS: u32 = 3;

impl Default for TodoAutomation {
    fn default() -> Self {
        Self {
            on_pr_merged: SyncMode::Ask,
            on_todo_closed_archive_sessions: SyncMode::Ask,
            auto_wake_on_ci_fail: false,
            auto_wake_on_review_comments: false,
            idle_nudge_days: IDLE_NUDGE_DAYS,
        }
    }
}

/// Where the To-do automation settings are read from. Read at each use, so
/// a change in Settings applies with no restart.
pub trait TodoAutomationSource: Send + Sync {
    /// The settings for an item in `channel` (none: not in a channel).
    fn settings(&self, channel: Option<&str>) -> TodoAutomation;
}

/// What archiving a session that may hold unsaved work came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArchiveOutcome {
    Archived,
    /// Left running; `reason` says why, for the person, such as
    /// "has 2 unpushed commits".
    Kept { reason: String },
}

/// The session actions the To-do list takes for the person: wake an agent
/// with a note, archive clean sessions when an item closes, undo that.
#[tonic::async_trait]
pub trait SessionControl: Send + Sync {
    /// Types `text` into the session as one message when its turn ends.
    async fn send_note(&self, session: &str, text: &str) -> Result<(), Status>;
    /// Archives the session only if that loses nothing.
    async fn archive_if_clean(&self, session: &str) -> Result<ArchiveOutcome, Status>;
    /// Brings an archived session back.
    async fn restore(&self, session: &str) -> Result<(), Status>;
}

/// A watched pull request as GitHub has it now.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PrStatus {
    pub title: String,
    pub merged: bool,
    pub closed_without_merge: bool,
    /// "passing", "failing", "pending" or empty when it has no checks.
    pub ci_state: String,
    pub unresolved_comments: u32,
    pub has_conflicts: bool,
    /// The pull request below it in its stack was merged, so it needs a rebase.
    pub upstream_merged: bool,
}

/// Why a watched pull request couldn't be read.
#[derive(Debug, thiserror::Error)]
pub enum PrStatusError {
    #[error(
        "Humata couldn't run {program} to check watched pull requests ({source}). Install the GitHub CLI on this host (brew install gh); if it already runs in your terminal, put its folder on PATH in your login profile (~/.zprofile), because the daemon reads your login shell's PATH when it starts. Watched pull requests keep their last state until then"
    )]
    GhMissing {
        program: String,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "the GitHub CLI on this host is signed out, so watched pull requests can't be checked. In a terminal on this host, run `gh auth login`"
    )]
    SignedOut { stderr: String },
    #[error(
        "GitHub is limiting requests, so watched pull requests weren't checked. Wait {wait}; they are checked again on their own after that, and signing in again doesn't lift the limit",
        wait = wait_text(*wait_seconds)
    )]
    RateLimited { wait_seconds: Option<u64> },
    #[error("pull request {repo_pr} wasn't found. Check it still exists and that `gh` on this host can see its repository; remove it from the item if it was deleted")]
    NotFound { repo_pr: String },
    #[error(
        "GitHub couldn't be reached to check watched pull requests ({first_line}). Nothing on this machine fixes it: they are checked again when the host is online"
    )]
    Unreachable { first_line: String },
    #[error(
        "GitHub's answer about a watched pull request isn't one this Humata can read. Update the GitHub CLI on the host (brew upgrade gh); if it keeps failing, report it"
    )]
    Unreadable {
        #[source]
        source: serde_json::Error,
    },
}

/// Where watched pull requests' state is read from.
#[tonic::async_trait]
pub trait PrStatusPort: Send + Sync {
    async fn fetch(&self, owner: &str, name: &str, number: u32) -> Result<PrStatus, PrStatusError>;
}
