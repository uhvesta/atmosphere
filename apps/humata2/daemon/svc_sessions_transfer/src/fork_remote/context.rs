//! A context-only fork of a session that runs on another machine (D187,
//! D208). It works in its parent's clones, so it runs on the parent's
//! machine: the host relays ForkSession there, where the machine forks as it
//! forks any of its own sessions, follows the machine's operation to its
//! end, then writes its own record of the fork as a session on that
//! machine, with the parent's folder and clones and its fork origin, so
//! lists, lineage and access see it. The parent is never touched.

use humata2_daemon_ports::Caller;
use humata2_rpc_kit::{blocking, error_of, failed, failure, now_ms, status_error_of};
use humata2_store::{ForkOrigin, SessionDetail, SessionRecord, StartStep};
use humata2_svc_sessions_common::pb;
use tonic::{Code, Status};

use crate::SessionTransferSvc;

/// The machine's own steps for a context-only fork, as `(key, title)`.
pub const STEPS: [(&str, &str); 4] = [
    ("write_record", "Saving the fork"),
    ("copy_conversation", "Copying the conversation"),
    ("launch", "Starting the agent"),
    ("verify", "Checking it started"),
];

#[derive(Debug, thiserror::Error)]
pub enum ContextThereError {
    #[error("the link to {machine} was lost while it made the fork, so it isn't known whether it finished")]
    LinkLost { machine: String },
    #[error("{machine} ended the fork without saying how it went")]
    NoAnswer { machine: String },
    #[error("{machine} said the fork succeeded but didn't name the session it made")]
    NoSession { machine: String },
    #[error("the parent {session} of a context-only fork cut short has no start record on this Mac")]
    NoParentDetail { session: String },
}

/// Parameters of a context-only fork relayed to the parent's machine.
#[derive(Clone, Debug, Default)]
pub struct ContextAsk {
    pub title: String,
    pub before_turn: u32,
    pub agent: String,
    pub model: String,
    pub asked: String,
}

/// A context-only fork on the parent's machine, checked and the access
/// rule passed.
#[derive(Clone, Debug)]
pub struct ContextThere {
    pub parent: SessionRecord,
    pub parent_detail: SessionDetail,
    /// The parent's machine, where the fork runs.
    pub machine: String,
    pub title: String,
}

/// The ForkSession the host relays for host operation `operation`. The
/// key is the host operation's, so the machine answers a retry with the
/// operation it already started; the machine is the one it runs on.
pub fn asked_request(parent: &str, operation: &str, ask: &ContextAsk) -> pb::ForkSessionRequest {
    pb::ForkSessionRequest {
        idempotency_key: Some(pb::IdempotencyKey {
            value: format!("host-fork-{operation}"),
        }),
        parent: Some(pb::SessionId {
            value: parent.to_owned(),
        }),
        kind: pb::ForkKind::ContextOnly as i32,
        before_turn: ask.before_turn,
        title: ask.title.clone(),
        agent: Some(pb::AgentId {
            value: ask.agent.clone(),
        })
        .filter(|agent| !agent.value.is_empty()),
        model: ask.model.clone(),
        machine: None,
        branch: String::new(),
        reason: String::new(),
    }
}

/// The failure an operation records for `status`.
pub fn failure_of(status: &Status) -> pb::Error {
    status_error_of(status).unwrap_or_else(|| pb::Error {
        message: status.message().to_owned(),
        fix: String::new(),
        retry: None,
        kind: None,
    })
}

/// The fork's id from the machine's ended operation, or why it has none.
pub fn made_by(machine: &str, ended: pb::Operation) -> Result<String, Status> {
    if ended.state != pb::OperationState::Succeeded as i32 {
        let error = ended.error.unwrap_or_else(|| {
            error_of(
                &ContextThereError::NoAnswer {
                    machine: machine.to_owned(),
                },
                "look at the parent on the machine, then fork again",
                None,
            )
        });
        return Err(failure(Code::FailedPrecondition, error));
    }
    match ended.result {
        Some(pb::operation::Result::Session(id)) if !id.value.is_empty() => Ok(id.value),
        _ => Err(failed(
            Code::Internal,
            &ContextThereError::NoSession {
                machine: machine.to_owned(),
            },
            "look at the machine's sessions: the fork may run there without this Mac listing it",
            None,
        )),
    }
}

impl SessionTransferSvc {
    /// ForkSession, context only, of a session on another machine, onto
    /// that machine: the machine forks it, and the host records the fork
    /// once it ends.
    pub async fn fork_context_there(
        &self,
        _caller: &Caller,
        _request: &pb::ForkSessionRequest,
        _there: ContextThere,
    ) -> Result<pb::Operation, Status> {
        Err(self.env.not_wired("SessionService.ForkSession (context-only remote)"))
    }

    /// The host's record of fork `session` on the parent's machine: the
    /// parent's folder and clones on their branches, and where it came
    /// from. Written once: a fork already recorded is left as it is.
    pub async fn record_context_there(
        &self,
        there: &ContextThere,
        session: &str,
        before_turn: u32,
    ) -> Result<(), Status> {
        let origin = ForkOrigin {
            session_id: session.to_owned(),
            parent: there.parent.id.clone(),
            kind: pb::ForkKind::ContextOnly as i32,
            before_turn,
            created_unix_ms: now_ms(),
        };
        let record = SessionRecord {
            id: session.to_owned(),
            channel: there.parent.channel.clone(),
            machine: there.machine.clone(),
            title: there.title.clone(),
            agent: there.parent.agent.clone(),
            folder: there.parent.folder.clone(),
            created_unix_ms: origin.created_unix_ms,
        };
        let detail = SessionDetail {
            session: session.to_owned(),
            repos: there.parent_detail.repos.clone(),
            model: there.parent_detail.model.clone(),
            first_message: None,
            start: StartStep::Done,
            operation: None,
            revision: 1,
        };
        let store = self.core.store.clone();
        blocking(move || -> Result<(), humata2_store::StoreError> {
            if store.fork_origin(&origin.session_id)?.is_some() {
                return Ok(());
            }
            store.put_session(&record)?;
            store.put_session_detail(&detail)?;
            store.put_fork_origin(&origin)
        })
        .await?
        .map_err(|error| failed(Code::Internal, &error, "save the fork", None))
    }
}
