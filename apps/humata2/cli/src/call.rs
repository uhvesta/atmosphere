//! What every command shares: calling the daemon, the idempotency key,
//! presence, plans and waiting (CLI.md §3, §4, §6).

use humata2_proto::humata::v1 as pb;
use prost::Message;
use serde_json::Value as Json;

use crate::args::Global;
use crate::as_json;
use crate::console::Console;
use crate::daemon::{Daemon, DaemonError, Replies, Rpc, rpc};
use crate::error::CliError;
use crate::principal::{Environment, Principal};

/// What a command did, for the output to print.
pub struct Done {
    pub result: Json,
    pub outcome: Outcome,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Applied or journaled, as the result's `delivery` says.
    Reply,
    /// `--dry-run`: the plan, nothing changed.
    Planned,
    /// The command printed as it went (a file, a terminal, events).
    Printed,
    /// The command ran a program in this terminal and exits as it did.
    Exited(i32),
}

impl Done {
    pub fn reply(result: Json) -> Self {
        Self {
            result,
            outcome: Outcome::Reply,
        }
    }

    pub fn printed() -> Self {
        Self {
            result: Json::Null,
            outcome: Outcome::Printed,
        }
    }
}

pub struct Ctx<'a, D, C> {
    pub daemon: &'a D,
    pub console: &'a C,
    pub flags: &'a Global,
    pub principal: &'a Principal,
    /// Where the principal came from, and where `humata login` keeps this
    /// terminal's login.
    pub environment: &'a Environment,
    key: String,
}

impl<'a, D: Daemon, C: Console> Ctx<'a, D, C> {
    pub fn new(
        daemon: &'a D,
        console: &'a C,
        flags: &'a Global,
        principal: &'a Principal,
        environment: &'a Environment,
    ) -> Self {
        let key = flags
            .key
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        Self {
            daemon,
            console,
            flags,
            principal,
            environment,
            key,
        }
    }

    /// The run's idempotency key: `--key`, or one new key per run, so a
    /// presence retry within the run is the same intent.
    pub fn key(&self) -> Option<pb::IdempotencyKey> {
        Some(pb::IdempotencyKey {
            value: self.key.clone(),
        })
    }

    /// Whether a person may be asked things here.
    pub fn interactive(&self) -> bool {
        !self.flags.json && self.console.interactive()
    }

    /// Progress and prompts, on stderr, and only for a person.
    pub fn note(&self, text: &str) {
        if !self.flags.json {
            self.console.eprint(text);
        }
    }

    /// One call, its reply bytes. A call the daemon holds for the user's
    /// approval (an agent's Ask) ends in exit 12 `pending` with the request's
    /// id, or waits for the answer when `--wait` or a terminal says to, and
    /// returns what the approved call returned.
    pub async fn raw(&self, rpc: Rpc, request: &impl Message) -> Result<Vec<u8>, CliError> {
        self.dry_run_guard(rpc)?;
        match self.daemon.unary(rpc, request.encode_to_vec()).await {
            Ok(bytes) => Ok(bytes),
            Err(DaemonError::Refused {
                rpc: path,
                code,
                error,
            }) => match &error.kind {
                Some(pb::error::Kind::ApprovalPending(pending)) => {
                    let id = pending.request.clone().unwrap_or_default();
                    match self.approved_result(id).await? {
                        Approved::Result(bytes) => Ok(bytes),
                        // D768: an approved read ran nothing; repeated now,
                        // it goes through and this is its real result.
                        Approved::Repeat => Ok(self.daemon.unary(rpc, request.encode_to_vec()).await?),
                    }
                }
                _ => Err(DaemonError::Refused {
                    rpc: path,
                    code,
                    error,
                }
                .into()),
            },
            Err(other) => Err(other.into()),
        }
    }

    /// A server stream whose opening the daemon may hold for the user's
    /// approval (D768): it waits for the answer as [`Self::raw`] does.
    pub(crate) async fn stream(&self, rpc: Rpc, request: &impl Message) -> Result<Opened, CliError> {
        match self.daemon.server_stream(rpc, request.encode_to_vec()).await {
            Ok(replies) => Ok(Opened::Stream(replies)),
            Err(DaemonError::Refused {
                rpc: path,
                code,
                error,
            }) => match &error.kind {
                Some(pb::error::Kind::ApprovalPending(pending)) => {
                    let id = pending.request.clone().unwrap_or_default();
                    match self.approved_result(id).await? {
                        Approved::Result(bytes) => Ok(Opened::Held(bytes)),
                        Approved::Repeat => Ok(Opened::Stream(
                            self.daemon.server_stream(rpc, request.encode_to_vec()).await?,
                        )),
                    }
                }
                _ => Err(DaemonError::Refused {
                    rpc: path,
                    code,
                    error,
                }
                .into()),
            },
            Err(other) => Err(other.into()),
        }
    }

    /// The result of the call held as Inbox request `id`, once the user
    /// approves it: waits `--wait` seconds, until answered in a terminal,
    /// or not at all with `--json` (CLI.md §6).
    async fn approved_result(&self, id: pb::RequestId) -> Result<Approved, CliError> {
        let request = id.value.clone();
        let seconds = match self.flags.wait {
            Some(seconds) => seconds,
            None if self.interactive() => u32::MAX,
            None => 0,
        };
        if seconds == 0 {
            return Err(CliError::Pending { request });
        }
        self.note(&format!(
            "Waiting for an answer (request {request}). Ctrl-C stops waiting; the request stays."
        ));
        let rpc = rpc::inbox::WAIT_FOR_ANSWER;
        let asked = pb::WaitForAnswerRequest {
            id: Some(id),
            wait_seconds: seconds,
        };
        let mut replies = self
            .daemon
            .server_stream(rpc, asked.encode_to_vec())
            .await?;
        while let Some(message) = replies.recv().await {
            let message: pb::WaitResponse = decode(rpc, &message?)?;
            if message.keepalive {
                continue;
            }
            if message.state == pb::RequestState::Pending as i32 {
                return Err(CliError::Pending { request });
            }
            return answered_result(rpc, message);
        }
        // The stream closed with no answer: the request is still pending.
        Err(CliError::Pending { request })
    }

    /// Under `--dry-run`, refuses a call that isn't read-only before it is
    /// sent: a change that reaches here has no plan of its own, so it must
    /// not be sent at all (fail closed).
    pub fn dry_run_guard(&self, rpc: Rpc) -> Result<(), CliError> {
        dry_run_refusal(self.flags.dry_run, rpc)
    }

    /// One call, its reply decoded.
    pub async fn call<R: Message + Default>(
        &self,
        rpc: Rpc,
        request: &impl Message,
    ) -> Result<R, CliError> {
        let bytes = self.raw(rpc, request).await?;
        decode(rpc, &bytes)
    }

    /// One call, its reply as JSON.
    pub async fn json(&self, rpc: Rpc, request: &impl Message) -> Result<Json, CliError> {
        let bytes = self.raw(rpc, request).await?;
        Ok(as_json::encoded(rpc.response, &bytes))
    }

    /// A call whose request carries a presence proof: sent without one
    /// first, and again with one if the daemon asks (CLI.md §3).
    pub async fn present<Req: Message>(
        &self,
        rpc: Rpc,
        build: impl Fn(Option<pb::PresenceProof>) -> Req,
    ) -> Result<Vec<u8>, CliError> {
        let needed = match self.raw(rpc, &build(None)).await {
            Err(CliError::Daemon(DaemonError::Refused { error, rpc, code })) => match &error.kind {
                Some(pb::error::Kind::PresenceRequired(needed)) => needed.clone(),
                _ => return Err(DaemonError::Refused { error, rpc, code }.into()),
            },
            other => return other,
        };
        let proof = self.proof(rpc, &needed)?;
        self.raw(rpc, &build(Some(proof))).await
    }

    /// As [`Self::present`], the reply as JSON.
    pub async fn present_json<Req: Message>(
        &self,
        rpc: Rpc,
        build: impl Fn(Option<pb::PresenceProof>) -> Req,
    ) -> Result<Json, CliError> {
        let bytes = self.present(rpc, build).await?;
        Ok(as_json::encoded(rpc.response, &bytes))
    }

    /// The proof to retry `rpc` with. The approval passphrase is only read
    /// by the daemon at `humata login` (SEC-13: log in with it, then run
    /// `humata inbox`); every other call ignores a passphrase sent with it,
    /// so asking for one there would only be refused again.
    fn proof(&self, rpc: Rpc, needed: &pb::PresenceRequired) -> Result<pb::PresenceProof, CliError> {
        use pb::presence_proof::Proof;
        use pb::presence_required::Method;
        let accepted: Vec<Method> = needed
            .accepted
            .iter()
            .filter_map(|method| Method::try_from(*method).ok())
            .collect();
        let refuse = || CliError::NoPresence {
            accepted: accepted.iter().map(|method| method_name(*method)).collect(),
            in_session: needed.caller_in_session_terminal,
        };
        if needed.caller_in_session_terminal {
            return Err(refuse());
        }
        if accepted.contains(&Method::ConfirmInApp) {
            self.note("Confirm it in the Humata app on the Mac…");
            return Ok(pb::PresenceProof {
                proof: Some(Proof::ConfirmInApp(pb::ConfirmInApp {})),
            });
        }
        let logging_in = rpc.path == rpc::cli_auth::BEGIN_CLI_SESSION.path;
        if accepted.contains(&Method::ApprovalPassphrase) && logging_in && self.interactive() {
            let passphrase = self
                .console
                .secret("Approval passphrase: ")
                .map_err(|source| CliError::Console { source })?;
            return Ok(pb::PresenceProof {
                proof: Some(Proof::ApprovalPassphrase(passphrase)),
            });
        }
        Err(refuse())
    }

    /// A destructive command (GRPC.md §5.4): `--plan` runs it against a
    /// plan printed earlier; `--dry-run` prints the plan and stops;
    /// otherwise it prints the plan and asks "Continue?" unless `--yes`.
    pub async fn planned<P, F, Fut>(
        &self,
        plan_rpc: Rpc,
        plan_request: &impl Message,
        token: impl Fn(&P) -> Option<pb::PlanToken>,
        execute: F,
    ) -> Result<Done, CliError>
    where
        P: Message + Default,
        F: FnOnce(Option<pb::PlanToken>) -> Fut,
        Fut: Future<Output = Result<Json, CliError>>,
    {
        if let Some(plan) = &self.flags.plan {
            let result = execute(Some(pb::PlanToken {
                value: plan.clone(),
            }))
            .await?;
            return Ok(Done::reply(result));
        }
        let bytes = self.raw(plan_rpc, plan_request).await?;
        let plan: P = decode(plan_rpc, &bytes)?;
        let shown = as_json::encoded(plan_rpc.response, &bytes);
        if self.flags.dry_run {
            return Ok(Done {
                result: shown,
                outcome: Outcome::Planned,
            });
        }
        self.confirm(&shown)?;
        Ok(Done::reply(execute(token(&plan)).await?))
    }

    /// A change the daemon takes no plan for (`inbox approve`, `review start`,
    /// `channel repo add`): with `--dry-run` it prints the request it would
    /// send, as a plan, and sends nothing; otherwise it sends the request.
    pub async fn change(&self, rpc: Rpc, request: &impl Message) -> Result<Done, CliError> {
        if self.flags.dry_run {
            return Ok(would_send(rpc, request));
        }
        self.json(rpc, request).await.map(Done::reply)
    }

    /// As [`Self::change`], for a request that carries a presence proof: the
    /// plan is the request without one.
    pub async fn change_present<Req: Message>(
        &self,
        rpc: Rpc,
        build: impl Fn(Option<pb::PresenceProof>) -> Req,
    ) -> Result<Done, CliError> {
        if self.flags.dry_run {
            return Ok(would_send(rpc, &build(None)));
        }
        self.present_json(rpc, build).await.map(Done::reply)
    }

    /// The CLI's own check after printing a plan: never presence, and
    /// skipped with `--yes`.
    pub fn confirm(&self, plan: &Json) -> Result<(), CliError> {
        if self.flags.yes {
            return Ok(());
        }
        if !self.interactive() {
            return Err(CliError::NeedsConfirmation);
        }
        self.console.print(&crate::output::human(plan));
        let go = self
            .console
            .confirm("Continue?")
            .map_err(|source| CliError::Console { source })?;
        if go {
            Ok(())
        } else {
            Err(CliError::NotConfirmed)
        }
    }

    /// Waits for an Inbox request's answer: `--wait` seconds, or until
    /// answered in a terminal, or not at all with `--json` (CLI.md §6).
    pub async fn wait(&self, first: pb::WaitResponse) -> Result<pb::WaitResponse, CliError> {
        let mut reply = first;
        let pending = pb::RequestState::Pending as i32;
        if reply.state != pending {
            return Ok(reply);
        }
        let request = reply
            .id
            .as_ref()
            .map(|id| id.value.clone())
            .unwrap_or_default();
        // `--wait N` waits N seconds even when the daemon answered pending
        // at once, as RequestAccess and AskUser do; without it a terminal
        // waits until answered and `--json` not at all.
        let wait_seconds = match self.flags.wait {
            Some(0) => return Err(CliError::Pending { request }),
            Some(seconds) => seconds,
            None if self.interactive() => u32::MAX,
            None => return Err(CliError::Pending { request }),
        };
        self.note(&format!(
            "Waiting for an answer (request {request}). Ctrl-C stops waiting; the request stays."
        ));
        // One stream held until the answer (TR-7): the daemon pushes it,
        // and its keepalives are skipped rather than asked again.
        let rpc = rpc::inbox::WAIT_FOR_ANSWER;
        let asked = pb::WaitForAnswerRequest {
            id: reply.id.clone(),
            wait_seconds,
        };
        let mut replies = self
            .daemon
            .server_stream(rpc, asked.encode_to_vec())
            .await?;
        while let Some(message) = replies.recv().await {
            let message: pb::WaitResponse = decode(rpc, &message?)?;
            if message.keepalive {
                continue;
            }
            reply = message;
            if reply.state != pending {
                return Ok(reply);
            }
        }
        if reply.state == pending {
            // A wait that ran out, or a stream that closed with no answer,
            // leaves the request waiting.
            return Err(CliError::Pending { request });
        }
        Ok(reply)
    }

    /// The seconds to ask the daemon to wait before it answers `pending`.
    pub fn wait_seconds(&self) -> u32 {
        match self.flags.wait {
            Some(seconds) => seconds,
            None if self.interactive() => 60,
            None => 0,
        }
    }
}

/// What an answered request's held call returned: its result, or the
/// error it failed with, or the user's no.
/// A server stream [`Ctx::stream`] opened.
pub(crate) enum Opened {
    Stream(Replies),
    /// The approved call ran while held; this is its one reply.
    Held(Vec<u8>),
}

/// What an approved held call gives back.
enum Approved {
    /// The call ran: its reply.
    Result(Vec<u8>),
    /// An asked read: approving it ran nothing, so the call is repeated.
    Repeat,
}

/// The daemon's result type for an approved asked read (D768), which comes
/// with no bytes.
const READ_APPROVED: &str = "humata2.ReadApproved";

fn answered_result(rpc: Rpc, answer: pb::WaitResponse) -> Result<Approved, CliError> {
    let refused = |error: pb::Error| {
        CliError::Daemon(DaemonError::Refused {
            rpc: rpc.path,
            code: UNKNOWN,
            error: Box::new(error),
        })
    };
    if let Some(error) = answer.error {
        return Err(refused(error));
    }
    let state = answer.state;
    if state == pb::RequestState::Approved as i32 && answer.result_type == READ_APPROVED {
        return Ok(Approved::Repeat);
    }
    if state == pb::RequestState::Approved as i32
        && (!answer.result.is_empty() || !answer.result_type.is_empty())
    {
        return Ok(Approved::Result(answer.result));
    }
    let (message, fix, kind) = if state == pb::RequestState::Denied as i32 {
        (
            "You said no to this request.",
            "Nothing was changed. Ask again if you've changed your mind.",
            pb::error::Kind::Denied(pb::Denied {
                source: pb::denied::Source::UserDenied as i32,
                ..Default::default()
            }),
        )
    } else {
        (
            "The request ended without a result humata can show.",
            "Run the command again: the held call's result is kept for 24 hours and a restart of humatad drops it.",
            pb::error::Kind::RequestWithdrawn(pb::RequestWithdrawn::default()),
        )
    };
    Err(refused(pb::Error {
        message: message.to_owned(),
        fix: fix.to_owned(),
        kind: Some(kind),
        ..Default::default()
    }))
}

/// The gRPC status a held call's own error is reported with.
const UNKNOWN: i32 = 2;

pub fn decode<R: Message + Default>(rpc: Rpc, bytes: &[u8]) -> Result<R, CliError> {
    R::decode(bytes).map_err(|source| {
        DaemonError::Decode {
            rpc: rpc.path,
            expected: rpc.response,
            source,
        }
        .into()
    })
}

fn method_name(method: pb::presence_required::Method) -> &'static str {
    use pb::presence_required::Method;
    match method {
        Method::Unspecified => "unspecified",
        Method::ConfirmInApp => "confirm_in_app",
        Method::CliLogin => "cli_login",
        Method::ApprovalPassphrase => "approval_passphrase",
    }
}

/// The calls `--dry-run` may send: reads, plans and waits, by name. Anything
/// else is a change, and `--dry-run` refuses it (see [`dry_run_refusal`]).
const READ_ONLY: &[Rpc] = &[
    rpc::daemon::GET_SERVER_INFO,
    rpc::daemon::WHO_AM_I,
    rpc::access::GET_CHANNEL_RULES,
    rpc::access::LIST_GRANTS,
    rpc::access::EXPLAIN,
    rpc::cli_auth::LIST_CLI_SESSIONS,
    rpc::cli_auth::LIST_SCRIPT_TOKENS,
    rpc::audit::LIST_AUDIT,
    rpc::board::LIST_ITEMS,
    rpc::board::GET_ITEM_HISTORY,
    rpc::channel::GET_CHANNEL,
    rpc::channel::LIST_CHANNELS,
    rpc::channel::PLAN_REMOVE_REPO,
    rpc::channel::PLAN_REMOVE_MACHINE,
    rpc::channel::GET_GRID,
    rpc::channel::PLAN_DELETE_CHANNEL,
    rpc::activity::LIST_ACTIVITY,
    rpc::events::SUBSCRIBE,
    rpc::files::LIST_DIRECTORY,
    rpc::files::READ_FILE,
    rpc::files::FIND_FILES,
    rpc::transfer::LIST_TRANSFERS,
    rpc::trash::LIST_TRASH,
    rpc::trash::PLAN_EMPTY_TRASH,
    rpc::machine::LIST_MACHINES,
    rpc::machine::GET_MACHINE,
    rpc::machine::PLAN_REMOVE_MACHINE,
    rpc::machine::PLAN_FORGET_MACHINE,
    rpc::machine::PLAN_CHANGE_ROOT,
    rpc::machine::GET_STORAGE,
    rpc::machine::LIST_HOST_VARIABLES,
    rpc::machine::LIST_MACHINE_SCRIPTS,
    rpc::machine::CHECK_MACHINE_SCRIPTS,
    rpc::machine::LIST_MACHINE_SCRIPT_RUNS,
    rpc::provider::LIST_PROVIDERS,
    rpc::provider::GET_PROVIDERS_CONFIG,
    rpc::provider::PREVIEW_PROVIDER_IMPORT,
    rpc::skills::PREVIEW_SKILLS_SYNC,
    rpc::bridge::LIST_PORT_FORWARDS,
    rpc::inbox::LIST_REQUESTS,
    rpc::inbox::GET_REQUEST,
    rpc::inbox::WAIT_FOR_ANSWER,
    rpc::operation::GET_OPERATION,
    rpc::operation::WATCH_OPERATION,
    rpc::operation::LIST_OPERATIONS,
    rpc::diff::GET_DIFF,
    rpc::comment::LIST_COMMENTS,
    rpc::review::RESOLVE_PULL_REQUEST,
    rpc::review::PLAN_SUBMIT,
    rpc::view::LIST_VIEWS,
    rpc::session::LIST_SESSIONS,
    rpc::session::GET_SESSION,
    rpc::session::PREVIEW_FORK,
    rpc::session::PLAN_ARCHIVE,
    rpc::session::PLAN_DELETE,
    rpc::session::PLAN_DELETE_SESSIONS,
    rpc::session::PLAN_DROP_REPO,
    rpc::session::PLAN_PUSH,
    rpc::session::READ_CONVERSATION,
    rpc::settings::GET_SETTINGS,
    rpc::settings::GET_CHANNEL_SETTINGS,
    rpc::agents::LIST_AGENTS,
    rpc::marks::LIST_PINNED,
    rpc::history::SEARCH_SESSIONS,
    rpc::history::READ_TURNS,
    rpc::repos::SEARCH_REPOS,
    rpc::repos::LIST_REPO_LIBRARY,
    rpc::repos::GET_REPO_SCOPE,
    rpc::terminal::LIST_QUEUED,
    rpc::terminal::LIST_TABS,
];

/// Whether `--dry-run` (when `dry_run` is on) may send `rpc`: only the
/// read-only calls. Any other call is refused, so an unplanned change can't
/// slip through a command that forgot its own dry-run check.
fn dry_run_refusal(dry_run: bool, rpc: Rpc) -> Result<(), CliError> {
    if !dry_run || READ_ONLY.iter().any(|read| read.path == rpc.path) {
        return Ok(());
    }
    Err(CliError::DryRunWouldSend {
        method: rpc.path.to_owned(),
        request: as_json::request_name(rpc.path).unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MADE_UP: Rpc = Rpc {
        path: "/humata.v1.DaemonService/Frobnicate",
        response: "humata.v1.FrobnicateResponse",
    };

    #[test]
    fn every_read_only_call_passes_under_dry_run() {
        for read in READ_ONLY {
            assert!(dry_run_refusal(true, *read).is_ok(), "{} refused", read.path);
        }
    }

    #[test]
    fn every_other_call_is_refused_under_dry_run_and_sends_nothing() {
        for call in rpc::ALL {
            if READ_ONLY.iter().any(|read| read.path == call.path) {
                continue;
            }
            let refused = dry_run_refusal(true, *call);
            assert!(
                matches!(refused, Err(CliError::DryRunWouldSend { ref method, .. }) if method == call.path),
                "{} was not refused",
                call.path
            );
        }
    }

    #[test]
    fn a_made_up_change_is_refused_under_dry_run() {
        assert!(matches!(
            dry_run_refusal(true, MADE_UP),
            Err(CliError::DryRunWouldSend { .. })
        ));
    }

    #[test]
    fn every_call_passes_without_dry_run() {
        assert!(dry_run_refusal(false, MADE_UP).is_ok());
        for call in rpc::ALL {
            assert!(dry_run_refusal(false, *call).is_ok(), "{} refused", call.path);
        }
    }
}

/// The plan for a change with no plan of its own: the method it would call
/// and the request, as JSON.
fn would_send(rpc: Rpc, request: &impl Message) -> Done {
    let name = as_json::request_name(rpc.path).unwrap_or_default();
    Done {
        result: serde_json::json!({
            "method": rpc.path,
            "request": as_json::typed(&name, request),
        }),
        outcome: Outcome::Planned,
    }
}
