//! A context-only fork of a session that runs on another machine (D187,
//! D208). It works in its parent's clones, so it runs on the parent's
//! machine: the host relays ForkSession there, where the machine forks as it
//! forks any of its own sessions, follows the machine's operation to its
//! end, then writes its own record of the fork as a session on that
//! machine, with the parent's folder and clones and its fork origin, so
//! lists, lineage and access see it. The parent is never touched.
//!
//! Nothing here waits on a timer: the host follows the machine's operation.

use humata2_grpc::humata::v1 as pb;
use humata2_store::{ForkOrigin, SessionDetail, SessionRecord, StartStep};
use tonic::{Code, Status};

use super::super::archive_remote::{follow_to_end, machine_failed};
use super::super::lineage::publish_family;
use super::super::{SessionSvc, now_ms};
use crate::carry::Carry;
use crate::channel_service::{blocking, failed};
use crate::host_relay::{Relayed, assigned};
use crate::identity::{self, Caller};
use crate::operations::{Progress, Started};
use crate::status;

/// The machine's own steps for a context-only fork, as `(key, title)`.
const STEPS: [(&str, &str); 4] = [
    ("write_record", "Saving the fork"),
    ("copy_conversation", "Copying the conversation"),
    ("launch", "Starting the agent"),
    ("verify", "Checking it started"),
];

#[derive(Debug, thiserror::Error)]
enum ContextThereError {
    #[error("the link to {machine} was lost while it made the fork, so it isn't known whether it finished")]
    LinkLost { machine: String },
    #[error("{machine} ended the fork without saying how it went")]
    NoAnswer { machine: String },
    #[error("{machine} said the fork succeeded but didn't name the session it made")]
    NoSession { machine: String },
}

/// A context-only fork on the parent's machine, checked and the access
/// rule passed.
pub(super) struct ContextThere {
    pub(super) carry: Carry,
    pub(super) parent: SessionRecord,
    pub(super) parent_detail: SessionDetail,
    /// The parent's machine, where the fork runs.
    pub(super) machine: String,
    pub(super) title: String,
}

/// The failure an operation records for `status`.
fn failure_of(status: &Status) -> pb::Error {
    status::error_of(status).unwrap_or_else(|| pb::Error {
        message: status.message().to_owned(),
        fix: String::new(),
        retry: None,
        kind: None,
    })
}

/// The fork's id from the machine's ended operation, or why it has none.
fn made_by(machine: &str, ended: pb::Operation) -> Result<String, Status> {
    if ended.state != pb::OperationState::Succeeded as i32 {
        let error = ended.error.unwrap_or_else(|| {
            crate::channel_service::error_of(
                &ContextThereError::NoAnswer {
                    machine: machine.to_owned(),
                },
                "look at the parent on the machine, then fork again",
                None,
            )
        });
        return Err(status::failure(Code::FailedPrecondition, error));
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

impl SessionSvc {
    /// ForkSession, context only, of a session on another machine, onto
    /// that machine: the machine forks it, and the host records the fork
    /// once it ends. A machine that refuses the fork answers the caller.
    pub(super) async fn fork_context_there(
        &self,
        caller: &Caller,
        request: &pb::ForkSessionRequest,
        there: ContextThere,
    ) -> Result<pb::Operation, Status> {
        let steps: Vec<(String, String)> = STEPS
            .iter()
            .map(|(key, title)| ((*key).to_owned(), (*title).to_owned()))
            .collect();
        let target = pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Session(pb::SessionId {
                value: there.parent.id.clone(),
            })),
        };
        let progress = match self.operations.start_steps(
            caller,
            pb::OperationKind::ForkSession,
            target,
            &steps,
            false,
            request.idempotency_key.clone(),
        )? {
            Started::New(progress) => progress,
            Started::Existing(operation) => return Ok(operation),
        };
        let operation = progress.operation();
        // The machine is the one it is running on, and it keys nothing:
        // the host's operation is the one the caller retries.
        let asked = pb::ForkSessionRequest {
            idempotency_key: None,
            machine: None,
            title: there.title.clone(),
            ..request.clone()
        };
        let relayed = match there.carry.links().relayed(&there.machine) {
            Ok(relayed) => relayed,
            Err(error) => {
                let status = Status::from(error);
                progress.fail(failure_of(&status));
                return Err(status);
            }
        };
        let first = match assigned(asked, None) {
            Ok(call) => relayed
                .session()
                .fork_session(call)
                .await
                .map(tonic::Response::into_inner)
                .map_err(|source| machine_failed(&there.machine, source)),
            Err(error) => Err(Status::from(error)),
        };
        let first = match first {
            Ok(first) => first,
            Err(status) => {
                progress.fail(failure_of(&status));
                return Err(status);
            }
        };
        tokio::spawn(self.clone().finish_context_there(
            relayed,
            there,
            first,
            progress,
            identity::actor(caller),
            request.before_turn,
        ));
        Ok(operation)
    }

    /// Follows the machine's fork to its end, then writes the host's
    /// records of it and ends `progress`.
    async fn finish_context_there(
        self,
        relayed: Relayed,
        there: ContextThere,
        first: pb::Operation,
        progress: Progress,
        by: pb::Actor,
        before_turn: u32,
    ) {
        let machine = there.machine.clone();
        let lost = move || {
            failed(
                Code::Unavailable,
                &ContextThereError::LinkLost { machine },
                "reconnect the machine and look at its sessions: it finishes the fork on its own, but this Mac won't list it",
                None,
            )
        };
        let ended = follow_to_end(&relayed, &there.machine, first, &progress, lost).await;
        let made = match ended.and_then(|ended| made_by(&there.machine, ended)) {
            Ok(session) => self
                .record_context_there(&there, &session, before_turn)
                .await
                .map(|()| session),
            Err(status) => Err(status),
        };
        match made {
            Ok(session) => {
                self.tell.session(&session, pb::Change::Created, Some(by.clone())).await;
                publish_family(&self.tell, &there.parent.id, Some(by)).await;
                progress.succeed(Some(pb::operation::Result::Session(pb::SessionId { value: session })));
            }
            Err(status) => progress.fail(failure_of(&status)),
        }
    }

    /// The host's record of fork `session` on the parent's machine: the
    /// parent's folder and clones on their branches, and where it came from.
    async fn record_context_there(&self, there: &ContextThere, session: &str, before_turn: u32) -> Result<(), Status> {
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
        let store = self.starter.store.clone();
        blocking(move || -> Result<(), humata2_store::StoreError> {
            store.put_session(&record)?;
            store.put_session_detail(&detail)?;
            store.put_fork_origin(&origin)
        })
        .await?
        .map_err(|error| status::store(error, &self.machine, "save the fork"))
    }
}
