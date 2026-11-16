//! The daemon as the MCP server sees it: one call per tool, and the Inbox
//! waits. humatad implements [`Daemon`] over gRPC (GRPC.md); the tests use
//! a fake.

use std::fmt;
use std::future::Future;
use std::time::{Duration, SystemTime};

use serde_json::{Map, Value};
use thiserror::Error;
use tokio::sync::mpsc;

use crate::session::SessionToken;
use crate::tools::Operation;

/// The daemon behind the MCP server, on the agent's own machine.
///
/// The daemon makes every decision: it checks the caller's capability
/// (humata2_access) for the bound session, makes Inbox requests, journals
/// with the host away and refuses. The server only carries calls and
/// answers.
pub trait Daemon: Send + Sync + 'static {
    /// Presents the session token so the daemon can bind this connection to
    /// that session (MCP.md §1). A token from outside the session's process
    /// tree fails with [`ErrorKind::IdentityMismatch`]. A failure marked
    /// retryable (the daemon wasn't running yet) is tried again on the next
    /// call; any other failure is final for this process.
    fn bind(&self, token: &SessionToken) -> impl Future<Output = Result<(), ToolError>> + Send;

    /// Runs one tool's call for the bound session. A call the daemon's
    /// rules send to the Inbox returns [`Reply::Pending`] at once with the
    /// new request; the server then waits with [`Daemon::wait`]. If the
    /// returned future is dropped before it finishes (the agent cancelled
    /// or exited), the daemon should withdraw any request it made for it.
    fn call(&self, call: Call) -> impl Future<Output = Result<Reply, ToolError>> + Send;

    /// Waits for the user to answer `request`, for at most `wait`
    /// (`InboxService.WaitForAnswer`). Sends [`WaitEvent::Keepalive`] about
    /// every 15 seconds and ends with one [`WaitEvent::Done`]: the call's
    /// reply once approved (the daemon runs it under its original key),
    /// the refusal once denied, or [`Reply::Pending`] when `wait` passes.
    fn wait(
        &self,
        request: &RequestId,
        wait: Duration,
    ) -> impl Future<Output = Result<mpsc::Receiver<WaitEvent>, ToolError>> + Send;

    /// Withdraws a request the agent stopped waiting on before it was told
    /// `pending` (TP-15, reason `CALL_ENDED`).
    fn withdraw(&self, request: &RequestId) -> impl Future<Output = Result<(), ToolError>> + Send;
}

/// One tool call, checked against the tool's schema.
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    pub operation: Operation,
    /// The tool's arguments as the agent gave them, less `wait_seconds`,
    /// which only the MCP server uses.
    pub arguments: Map<String, Value>,
    pub idempotency_key: IdempotencyKey,
}

/// The key a write carries (GRPC.md, TP-14): the JSON-RPC request id plus
/// this MCP process's nonce, so a client's retry of one request is
/// deduplicated and a new request is a new intent.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IdempotencyKey(String);

impl IdempotencyKey {
    pub(crate) fn new(value: String) -> Self {
        Self(value)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An Inbox request's id.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RequestId(String);

impl RequestId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What a call did (MCP.md §3).
#[derive(Debug, Clone, PartialEq)]
pub enum Reply {
    /// Done on the host or this machine.
    Applied {
        result: Value,
        /// The Inbox request it went through, if any.
        request: Option<RequestId>,
        /// Reads only: the host is away and this is its last copy, from
        /// this time.
        stale_as_of: Option<SystemTime>,
    },
    /// Written to this machine's journal while the host is away; `result`
    /// is what was journaled, and a new item has no id until the host
    /// replays it.
    Journaled {
        result: Value,
        journal: JournalPosition,
    },
    /// Waiting for the user in the Inbox.
    Pending { request: RequestId },
}

/// Where a journaled write sits (`humata.v1.JournalPosition`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalPosition {
    pub machine: String,
    pub seq: u64,
}

/// One message on a wait.
#[derive(Debug, Clone, PartialEq)]
pub enum WaitEvent {
    /// Still waiting; the server sends progress if the agent asked for it.
    Keepalive,
    /// The wait's end.
    Done(Result<Reply, ToolError>),
}

/// A refusal or failure from the daemon (`humata.v1.Error`), sent to the
/// agent as a tool result with `isError: true` (MCP.md §3, D25).
#[derive(Debug, Clone, PartialEq, Error)]
#[error("{kind}: {message} {fix}")]
pub struct ToolError {
    pub kind: ErrorKind,
    /// What failed.
    pub message: String,
    /// How to fix it, or why the obvious fix is unsafe.
    pub fix: String,
    /// When retrying can succeed; `None` when it can't.
    pub retry: Option<Retry>,
    /// The kind's fields, with the proto's names and enum values lowercase
    /// without their prefix (D25), e.g. `source: "never_rule"`.
    pub details: Map<String, Value>,
}

/// `humata.v1.RetryAdvice`, with `after` already turned into a time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Retry {
    pub retryable: bool,
    pub after: Option<SystemTime>,
    pub same_key: bool,
}

macro_rules! error_kinds {
    ($($variant:ident => $name:literal,)*) => {
        /// The kinds of `humata.v1.Error`, which agents branch on.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum ErrorKind {
            $($variant,)*
        }

        impl ErrorKind {
            /// The kind's name in `errors.proto`, as agents see it.
            pub fn as_str(self) -> &'static str {
                match self {
                    $(ErrorKind::$variant => $name,)*
                }
            }
        }
    };
}

error_kinds! {
    NotFound => "not_found",
    MachineOffline => "machine_offline",
    HostUnreachable => "host_unreachable",
    Denied => "denied",
    UserOnly => "user_only",
    PresenceRequired => "presence_required",
    IdentityMismatch => "identity_mismatch",
    PeerUnproven => "peer_unproven",
    RevisionConflict => "revision_conflict",
    PlanStale => "plan_stale",
    NameTaken => "name_taken",
    InvalidField => "invalid_field",
    UnpushedWork => "unpushed_work",
    SessionBusy => "session_busy",
    ParentReadOnly => "parent_read_only",
    MainCloneReadOnly => "main_clone_read_only",
    OutsideHumataFolders => "outside_humata_folders",
    DiskFull => "disk_full",
    RateLimited => "rate_limited",
    GithubCli => "github_cli",
    GitFailed => "git_failed",
    PushRefused => "push_refused",
    SshFailed => "ssh_failed",
    HostKeyChanged => "host_key_changed",
    ChecksumMismatch => "checksum_mismatch",
    ResumeFailed => "resume_failed",
    AgentLacksFeature => "agent_lacks_feature",
    ProviderStepFailed => "provider_step_failed",
    ProviderIdentityInvalid => "provider_identity_invalid",
    LoginNeedsUser => "login_needs_user",
    PeerVersionUnsupported => "peer_version_unsupported",
    OffsetTrimmed => "offset_trimmed",
    NotCancellable => "not_cancellable",
    AlreadyAnswered => "already_answered",
    RequestWithdrawn => "request_withdrawn",
    UserAway => "user_away",
    NotInChannel => "not_in_channel",
    PrClosed => "pr_closed",
    ReviewPartlyPosted => "review_partly_posted",
    SkillNameClash => "skill_name_clash",
    LingerUnavailable => "linger_unavailable",
    DaemonAlreadyRunning => "daemon_already_running",
    DaemonNotRunning => "daemon_not_running",
    JournalEntryRejected => "journal_entry_rejected",
    RepoNotVisible => "repo_not_visible",
    StepFailed => "step_failed",
    PassphraseLocked => "passphrase_locked",
    DependencyLoop => "dependency_loop",
    Internal => "internal",
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
