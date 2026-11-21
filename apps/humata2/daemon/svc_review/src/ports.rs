//! Port traits and helpers decoupling `humata2_svc_review` from
//! `humatad`-internal services (`Guard`, `Feed`, `Operations`, `SessionSvc`,
//! `Tell`, `Starter`, `terminal::Core`).

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use humata2_access::{AccessRequest, Call, ChannelId, MachineId, Refusal, SessionId, SessionRef};
use humata2_daemon_ports::Caller;
use crate::pb;
use humata2_store::{ChannelRecord, SessionRecord, StoreError};
use tonic::{Code, Request, Status};

pub use humata2_daemon_ports::{AgentCaller, actor};
pub use humata2_rpc_kit::{
    blocking, chain, error, failure, internal, invalid_field, missing, not_built_status as not_built,
    not_found, status_error_of,
};

/// Result of a policy guard check.
#[derive(Debug)]
pub enum Checked {
    Allowed,
    Ask(Box<AccessRequest>),
}

/// Access-control port implemented by `humatad::guard::Guard`.
pub trait ReviewGuard: Send + Sync {
    fn check(&self, caller: &Caller, call: &Call) -> Result<Checked, Status>;
    fn require(&self, caller: &Caller, call: &Call) -> Result<(), Status>;
}

/// Scope of an event published on the feed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    Global,
    Channel(String),
    Session(String),
}

/// Event-feed port implemented by `humatad::events::Feed`.
pub trait ReviewFeed: Send + Sync {
    fn send(
        &self,
        scope: Scope,
        change: pb::Change,
        payload: pb::event::Payload,
        actor: Option<pb::Actor>,
    );
}

/// VW-12, D225: where a machine's daemon journals a view its agent just
/// offered, so the host applies it and owns its state from then on.
/// Implemented by `humatad` over the away journal; the host keeps none.
#[tonic::async_trait]
pub trait ViewJournal: Send + Sync {
    /// Keeps `view`, `session`'s, for the host. An error means the view
    /// can't reach the host, so the offer must not be taken.
    async fn offered(&self, session: &str, view: &pb::PreparedView) -> Result<(), Status>;
}

/// Session-update notification port implemented by `humatad::session_service::Tell`.
#[tonic::async_trait]
pub trait ReviewTell: Send + Sync {
    async fn session(&self, id: &str, change: pb::Change, actor: Option<pb::Actor>);
}

/// Session-service port implemented by `humatad::session_service::SessionSvc`.
#[tonic::async_trait]
pub trait ReviewSessions: Send + Sync {
    async fn begin_review(
        &self,
        caller: &Caller,
        channel: ChannelRecord,
        request: &pb::StartReviewRequest,
        prs: Vec<humata2_store::ReviewPr>,
        push_allowed: bool,
        // The session's title and the review chat's first message (PR-5).
        title: String,
        first_message: String,
    ) -> Result<pb::StartSessionResponse, Status>;
}

/// Channel/clone starter port wrapping `humata2_sessions::Starter`.
pub trait ReviewStarter: Send + Sync {
    fn channels_root(&self) -> PathBuf;
    fn remote_template(&self) -> String;
    fn create_pr_reviews_channel(
        &self,
        machine: &str,
        default_agent: &str,
    ) -> Result<ChannelRecord, Status>;
    fn clone_pr_reviews_mains(&self, _channel_id: &str) -> Result<(), Status> {
        Ok(())
    }
}

/// Handle to a running background operation, wrapping `humatad::operations::Started`.
pub trait ReviewOperation: Send + Sync {
    fn id(&self) -> String;
    fn pb(&self) -> pb::Operation;
    fn step_started(&self, index: usize);
    fn step_done(&self, index: usize);
    fn step_failed(&self, index: usize, err: pb::Error);
    fn finish(&self, result: Vec<u8>, result_type: String);
    fn fail(&self, err: pb::Error);
}

/// Operations tracker port implemented by `humatad::operations::Operations`.
pub trait ReviewOperations: Send + Sync {
    fn start(
        &self,
        kind: pb::OperationKind,
        title: String,
        steps: Vec<String>,
        actor: Option<pb::Actor>,
    ) -> Box<dyn ReviewOperation>;
}

/// Target for typing a batch of comments into an agent terminal.
#[derive(Clone, Debug)]
pub struct CommentsTarget {
    pub tab: String,
    pub holder: u64,
}

/// Outcome of typing a batch of comments.
pub enum CommentsTyped {
    Typed,
    NotTyped(Status),
    Unknown(Status),
}

/// Whether a batch of comments arrived at its holder.
pub enum CommentsArrived {
    Yes,
    No,
    Unknown,
}

/// Terminal core port implemented by `humatad::terminal::Core`.
#[tonic::async_trait]
pub trait ReviewTerminalCore: Send + Sync {
    async fn comments_target(&self, session: &str) -> Result<CommentsTarget, Status>;
    async fn type_comments(
        &self,
        target: &CommentsTarget,
        key: &str,
        paste: &[u8],
        shown_wait: Duration,
        return_pause: Duration,
    ) -> CommentsTyped;
    async fn comments_arrived(&self, target: &CommentsTarget, key: &str) -> CommentsArrived;
}

/// Extracts the authenticated [`Caller`] from a tonic request's extensions.
pub fn caller_of<T>(request: &Request<T>) -> Result<Caller, Status> {
    request.extensions().get::<Caller>().cloned().ok_or_else(|| {
        internal(
            "this call reached its handler without being authenticated",
            "",
        )
    })
}

/// Converts a [`SessionRecord`] to a policy [`SessionRef`].
pub fn session_ref(record: &SessionRecord, default_machine: &str) -> SessionRef {
    let machine = match record.machine.is_empty() {
        true => default_machine,
        false => record.machine.as_str(),
    };
    SessionRef {
        id: SessionId::new(record.id.as_str()),
        channel: ChannelId::new(record.channel.as_str()),
        machine: MachineId::new(machine),
    }
}

/// Formats a [`StoreError`] as a gRPC `Status`.
pub fn store_error(err: StoreError, _machine: &str, doing: &str) -> Status {
    internal(format!("couldn't {doing}: {}", chain(&err)), "")
}

/// Formats a [`Refusal`] as a gRPC `PermissionDenied` status.
pub fn refused(refusal: &Refusal) -> Status {
    let kind = match refusal {
        Refusal::UserOnly { .. } => pb::error::Kind::UserOnly(pb::UserOnly { capability: 0 }),
        _ => pb::error::Kind::Denied(pb::Denied {
            source: pb::denied::Source::Unspecified as i32,
            capability: 0,
            request: None,
            user_note: String::new(),
        }),
    };
    failure(
        Code::PermissionDenied,
        error(
            refusal.to_string(),
            "check your permissions and try again",
            kind,
        ),
    )
}

/// Wraps a remote call failure status with context.
pub fn call_failed(machine: &str, status: Status) -> Status {
    if status_error_of(&status).is_some() {
        return status;
    }
    failure(
        status.code(),
        error(
            format!("call to {machine} failed: {}", status.message()),
            "check that the machine is online and try again",
            pb::error::Kind::MachineOffline(pb::MachineOffline {
                machine: Some(pb::MachineId {
                    value: machine.to_owned(),
                }),
                machine_name: machine.to_owned(),
                since: None,
            }),
        ),
    )
}

/// Converts a `SystemTime` to a protobuf `Timestamp`.
pub fn timestamp(at: SystemTime) -> humata2_grpc_core::Timestamp {
    let since = at.duration_since(UNIX_EPOCH).unwrap_or_default();
    humata2_grpc_core::Timestamp {
        seconds: i64::try_from(since.as_secs()).unwrap_or(i64::MAX),
        nanos: i32::try_from(since.subsec_nanos()).unwrap_or(0),
    }
}
