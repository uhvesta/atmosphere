//! What every command shares: calling the daemon, the idempotency key,
//! presence, plans and waiting (CLI.md §3, §4, §6).

use humata2_proto::humata::v1 as pb;
use prost::Message;
use serde_json::Value as Json;

use crate::args::Global;
use crate::as_json;
use crate::console::Console;
use crate::daemon::{Daemon, DaemonError, Rpc, rpc};
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
        match self.daemon.unary(rpc, request.encode_to_vec()).await {
            Ok(bytes) => Ok(bytes),
            Err(DaemonError::Refused {
                rpc: path,
                code,
                error,
            }) => match &error.kind {
                Some(pb::error::Kind::ApprovalPending(pending)) => {
                    let id = pending.request.clone().unwrap_or_default();
                    self.approved_result(id).await
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
    async fn approved_result(&self, id: pb::RequestId) -> Result<Vec<u8>, CliError> {
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
        Err(DaemonError::Ended { rpc: rpc.path }.into())
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
        let proof = self.proof(&needed)?;
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

    fn proof(&self, needed: &pb::PresenceRequired) -> Result<pb::PresenceProof, CliError> {
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
        if accepted.contains(&Method::ApprovalPassphrase) && self.interactive() {
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
        let forever = self.flags.wait.is_none() && self.interactive();
        if !forever {
            return Err(CliError::Pending { request });
        }
        self.note(&format!(
            "Waiting for an answer (request {request}). Ctrl-C stops waiting; the request stays."
        ));
        // One stream held until the answer (TR-7): the daemon pushes it,
        // and its keepalives are skipped rather than asked again.
        let rpc = rpc::inbox::WAIT_FOR_ANSWER;
        let asked = pb::WaitForAnswerRequest {
            id: reply.id.clone(),
            wait_seconds: u32::MAX,
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
            return Err(DaemonError::Ended { rpc: rpc.path }.into());
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
fn answered_result(rpc: Rpc, answer: pb::WaitResponse) -> Result<Vec<u8>, CliError> {
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
    if state == pb::RequestState::Approved as i32 && !answer.result.is_empty() {
        return Ok(answer.result);
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
