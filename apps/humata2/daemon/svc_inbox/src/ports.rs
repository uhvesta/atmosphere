//! Narrow port traits and helpers decoupling `svc_inbox` from `humatad`
//! internals (`Guard`, `Feed`, `upgrade::items`, `session_service::Asking`,
//! `agent_gate`, `holders`).

use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use humata2_access::{
    AnswerError, Call, Capability, Context, Grant, GrantTarget, Policy, PresentUser, Principal,
    Refusal,
};
use humata2_daemon_ports::{AgentCaller, Caller};
use humata2_daemon_runtime::agent_switches::AgentSwitches;
use humata2_grpc::humata::v1 as pb;
use humata2_rpc_kit::helpers as status;
use humata2_sessions_core::Agents;
use humata2_store::{SessionRecord, Store, StoreError};
use humata2_svc_away::HostLink;
use tonic::{Code, Request, Status};

/// Scope for publishing an event to the daemon's event feed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EventScope {
    pub channel: Option<String>,
    pub session: Option<String>,
}

/// Event feed port used by `svc_inbox`.
pub trait InboxFeedPort: Send + Sync {
    fn publish(
        &self,
        change: pb::Change,
        payload: pb::event::Payload,
        actor: Option<pb::Actor>,
        scope: EventScope,
    );
}

/// Policy and guard port used by `svc_inbox`.
pub trait InboxGuardPort: Send + Sync {
    fn host_link(&self) -> Arc<HostLink>;
    fn with_policy_mut(&self, f: &mut dyn FnMut(&mut Policy, &Context));
    fn principal(&self, caller: &Caller) -> Principal;
    fn present_user(&self, caller: &Caller) -> Result<PresentUser, Status>;
    fn capability_pb(&self, capability: Capability) -> pb::Capability;
    fn capability_of(&self, capability: i32) -> Option<Capability>;
    fn grant_number(&self, grant: &pb::Grant) -> Option<u64>;
    fn grant_pb(&self, grant: &Grant, suspended: bool) -> pb::Grant;
    fn target_pb(&self, target: &GrantTarget) -> pb::GrantTarget;
    fn ask_ahead_call(
        &self,
        store: &Store,
        machine: &str,
        agent: &AgentCaller,
        capability: Capability,
        target: pb::GrantTarget,
    ) -> Result<Call, Status>;
    fn answer_failed(&self, error: &AnswerError) -> Status;
    fn refused_status(&self, refusal: &Refusal) -> Status;
    fn require(&self, caller: &Caller, call: &Call) -> Result<(), Status>;
    fn check_inbox_ask(&self, caller: &Caller, call: &Call) -> Result<(), Status>;
    fn hold_bound(&self, caller: &Caller, metadata: &tonic::metadata::MetadataMap) -> std::time::Duration;
    fn waits_for_host(&self) -> Status;
    fn agent_caller(
        &self,
        record: &SessionRecord,
        machine: &str,
        store: &Store,
    ) -> Result<AgentCaller, StoreError>;
    /// IN-8: how many agent actions followed the channel's rules or grants
    /// without asking today. Zero until the guard adapter wires it up.
    fn allowed_without_asking_today(&self) -> u32 {
        0
    }
}

/// Convenience helper for calling [`InboxGuardPort::with_policy_mut`] with a
/// typed return value.
pub fn with_policy<R>(
    guard: &dyn InboxGuardPort,
    f: impl FnOnce(&mut Policy, &Context) -> R,
) -> R {
    let mut f = Some(f);
    let mut out = None;
    guard.with_policy_mut(&mut |policy, context| {
        if let Some(f) = f.take() {
            out = Some(f(policy, context));
        }
    });
    out.expect("InboxGuardPort::with_policy_mut must invoke callback")
}

/// Migration approver port used by `svc_inbox` for Migrate items.
#[tonic::async_trait]
pub trait MigrateApproverPort: Send + Sync {
    async fn approve(&self, id: &str) -> Result<pb::InboxRequest, Status>;
    fn is_migrate(&self, request: &pb::InboxRequest) -> bool;
    fn refuse_answer(
        &self,
        request: &pb::InboxRequest,
        answer: &pb::request_answer::Answer,
    ) -> Option<Status>;
    fn finish_steps(&self, request: &mut pb::InboxRequest);
}

/// Guard port marking a session as currently asking the user.
pub trait AskingGuardPort: Send + Sync {
    fn hold(&self, session: &str) -> Box<dyn Send + Sync>;
}

/// Default no-op [`AskingGuardPort`].
pub struct NoopAsking;

impl AskingGuardPort for NoopAsking {
    fn hold(&self, _session: &str) -> Box<dyn Send + Sync> {
        Box::new(())
    }
}

/// Agent gate port for reading per-agent permission switches.
pub trait AgentGatePort: Send + Sync {
    fn switches(&self, agents: &Agents) -> Option<AgentSwitches>;
}

/// Terminal/holder port used by `screen_ask`.
#[tonic::async_trait]
pub trait ScreenHoldersPort: Send + Sync {
    /// The text the holder at `socket` shows on its screen now.
    async fn screen_at(&self, socket: &Path) -> Result<String, Status>;

    /// The text and output end offset of `tab`'s screen at `socket`, reusing a
    /// live link's screen when available and returning `Ok(None)` when the
    /// holder's output end offset is unchanged from `last_end`.
    async fn screen_for_tab(
        &self,
        _tab: &str,
        socket: &Path,
        _last_end: Option<u64>,
    ) -> Result<Option<(u64, String)>, Status> {
        let text = self.screen_at(socket).await?;
        Ok(Some((0, text)))
    }
}

/// Port allowing `screen_ask` to inspect and auto-clear stale hook-raised asks
/// without coupling `screen_ask` to `Inbox` internals.
#[tonic::async_trait]
pub trait HookAskPort: Send + Sync {
    /// Returns `true` when `session` currently has at least one pending
    /// hook-raised ask in the Inbox.
    async fn has_pending_hook_ask(&self, session: &str) -> bool;

    /// Auto-clears pending hook-raised asks for `session` as resolved in
    /// terminal (`pb::WithdrawReason::Resolved`, never `Denied`).
    async fn resolve_pending_hook_asks(&self, session: &str);
}

/// Default no-op [`HookAskPort`].
pub struct NoopHookAsk;

#[tonic::async_trait]
impl HookAskPort for NoopHookAsk {
    async fn has_pending_hook_ask(&self, _session: &str) -> bool {
        false
    }

    async fn resolve_pending_hook_asks(&self, _session: &str) {}
}

/// Converts a [`SystemTime`] to a [`humata2_grpc::Timestamp`].
pub fn timestamp(at: SystemTime) -> humata2_grpc::Timestamp {
    let since = at.duration_since(UNIX_EPOCH).unwrap_or_default();
    humata2_grpc::Timestamp {
        seconds: i64::try_from(since.as_secs()).unwrap_or(i64::MAX),
        nanos: i32::try_from(since.subsec_nanos()).unwrap_or(0),
    }
}

/// Current time as a [`humata2_grpc::Timestamp`].
pub fn now_timestamp() -> humata2_grpc::Timestamp {
    timestamp(SystemTime::now())
}

/// Extracts the authenticated [`Caller`] from request extensions.
pub fn caller<T>(request: &Request<T>) -> Result<Caller, Status> {
    request
        .extensions()
        .get::<Caller>()
        .cloned()
        .ok_or_else(|| status::internal("caller identity missing from request", ""))
}

/// Maps a [`StoreError`] to a gRPC [`Status`].
pub fn store_status(error: StoreError, machine: &str, doing: &str) -> Status {
    if error.is_disk_full() {
        return status::failure(
            Code::ResourceExhausted,
            status::error(
                format!("couldn't {doing}: the disk on {machine} is full"),
                format!("free space on {machine} and try again"),
                pb::error::Kind::DiskFull(pb::DiskFull {
                    machine: Some(pb::MachineId {
                        value: machine.to_owned(),
                    }),
                    path: String::new(),
                    needed_bytes: 0,
                    free_bytes: 0,
                }),
            ),
        );
    }
    status::internal(format!("couldn't {doing}: {}", status::chain(&error)), "")
}

/// What an approved asked read keeps as its result type (D768): approving
/// it runs nothing, so the waiter is told to repeat the read, which then goes
/// through. Not a proto message: no bytes come with it.
pub const READ_APPROVED: &str = "humata2.ReadApproved";

pub fn needs_approval(guard: &dyn InboxGuardPort, capability: Capability) -> Status {
    let message = format!(
        "\"{capability}\" needs the user's approval here, and this call can't wait for it in the Inbox, so nothing was done"
    );
    status::failure(
        Code::PermissionDenied,
        status::error(
            message,
            "ask the user with inbox_ask to do it themselves in the app, or to let agents do it in the channel's rules; don't retry the call, since it is asked again each time",
            pb::error::Kind::Denied(pb::Denied {
                source: pb::denied::Source::Unspecified as i32,
                capability: guard.capability_pb(capability) as i32,
                request: None,
                user_note: String::new(),
            }),
        ),
    )
}

pub fn approval_pending(request: &pb::RequestId) -> Status {
    status::failure(
        Code::FailedPrecondition,
        status::error(
            format!(
                "this call needs the user's approval and is waiting for their answer in the Inbox as request {}",
                request.value
            ),
            "call inbox_wait (WaitForAnswer) with this request id to get the result once the user approves; don't repeat the call without the same idempotency key, which would put a second request in the Inbox",
            pb::error::Kind::ApprovalPending(pb::ApprovalPending {
                request: Some(request.clone()),
            }),
        ),
    )
}

pub fn read_approval_pending(request: &pb::RequestId, runs: bool) -> Status {
    let fix = if runs {
        "call inbox_wait (WaitForAnswer) with this request id: once the user approves, it returns the read's result. Repeating the read before then shows this request again and doesn't add another"
    } else {
        "call inbox_wait (WaitForAnswer) with this request id; once the user approves, repeat the same read to get its result. Repeating it before then shows this request again and doesn't add another"
    };
    status::failure(
        Code::FailedPrecondition,
        status::error(
            format!(
                "this read needs the user's approval and is waiting for their answer in the Inbox as request {}",
                request.value
            ),
            fix,
            pb::error::Kind::ApprovalPending(pb::ApprovalPending {
                request: Some(request.clone()),
            }),
        ),
    )
}
