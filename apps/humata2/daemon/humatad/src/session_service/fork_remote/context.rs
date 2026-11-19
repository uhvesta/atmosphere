//! A context-only fork of a session that runs on another machine (D187,
//! D208). It works in its parent's clones, so it runs on the parent's
//! machine: the host relays ForkSession there, where the machine forks as it
//! forks any of its own sessions, follows the machine's operation to its
//! end, then writes its own record of the fork as a session on that
//! machine, with the parent's folder and clones and its fork origin, so
//! lists, lineage and access see it. The parent is never touched.
//!
//! The fork is journaled on the host before it is asked for, and asked
//! under a key made from the host's operation, so a link lost or a host
//! restart before the host wrote its records is finished once the
//! machine's link is up again: the host asks again under the same key (or
//! follows the machine's operation it already knew), then writes its
//! records once (D221).
//!
//! Nothing here waits on a timer: the host follows the machine's operation
//! and the machine's link state.

use std::sync::Arc;

use humata2_grpc::humata::v1 as pb;
use humata2_store::{ForkOrigin, SessionDetail, SessionRecord, StartStep};
use tonic::{Code, Status};

use super::super::archive_remote::{finished, follow_to_end, machine_failed};
use super::super::lineage::publish_family;
use super::super::{SessionSvc, now_ms};
use crate::carry::{Carry, ContextAsk, ForkJournal};
use crate::channel_service::{blocking, failed};
use crate::host_relay::{LinkState, Relayed, assigned};
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
    #[error("the parent {session} of a context-only fork cut short has no start record on this Mac")]
    NoParentDetail { session: String },
}

/// How asking the machine, or following its operation, went wrong.
enum Asked {
    /// The link went before the machine answered: it may have the fork.
    Lost(Status),
    /// The machine answered with a refusal: it has no fork from this ask.
    Refused(Status),
}

/// `source`, a relayed call's failure, sorted into a lost link or the
/// machine's own answer.
fn asked_error(machine: &str, source: Status) -> Asked {
    let lost = status::error_of(&source).is_none()
        && matches!(source.code(), Code::Unavailable | Code::Unknown | Code::Cancelled);
    let status = machine_failed(machine, source);
    if lost { Asked::Lost(status) } else { Asked::Refused(status) }
}

/// The ForkSession the host relays for host operation `operation`. The
/// key is the host operation's, so the machine answers a retry with the
/// operation it already started; the machine is the one it runs on.
fn asked_request(parent: &str, operation: &str, ask: &ContextAsk) -> pb::ForkSessionRequest {
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

/// Asks the machine for the fork.
async fn ask_machine(relayed: &Relayed, machine: &str, request: pb::ForkSessionRequest) -> Result<pb::Operation, Asked> {
    let call = assigned(request, None).map_err(|error| Asked::Refused(Status::from(error)))?;
    relayed
        .session()
        .fork_session(call)
        .await
        .map(tonic::Response::into_inner)
        .map_err(|source| asked_error(machine, source))
}

/// Follows the machine's operation `first` to its end, with no host
/// operation to show it on.
async fn watch_end(relayed: &Relayed, machine: &str, first: pb::Operation) -> Result<pb::Operation, Asked> {
    if finished(&first) {
        return Ok(first);
    }
    let id = first.id.map(|id| id.value).unwrap_or_default();
    let mut stream = relayed
        .operation()
        .watch_operation(pb::WatchOperationRequest {
            id: Some(pb::OperationId { value: id }),
            from_version: 0,
        })
        .await
        .map_err(|source| asked_error(machine, source))?
        .into_inner();
    loop {
        match stream.message().await {
            Ok(Some(operation)) if finished(&operation) => return Ok(operation),
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => {
                return Err(Asked::Lost(Status::from(crate::host_relay::RelayError::Offline {
                    machine: machine.to_owned(),
                })));
            }
        }
    }
}

/// The journal of host operation `operation`'s context-only fork.
fn journal_of(there: &ContextThere, operation: &str, ask: &ContextAsk) -> ForkJournal {
    ForkJournal {
        operation: operation.to_owned(),
        parent: there.parent.id.clone(),
        session: String::new(),
        from: there.machine.clone(),
        to: there.machine.clone(),
        channel: there.parent.channel.clone(),
        folder: String::new(),
        phase: "context".to_owned(),
        context: Some(ask.clone()),
    }
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
        let host_operation = operation.id.as_ref().map(|id| id.value.clone()).unwrap_or_default();
        let mut ask = ContextAsk {
            title: there.title.clone(),
            before_turn: request.before_turn,
            agent: request.agent.as_ref().map(|agent| agent.value.clone()).unwrap_or_default(),
            model: request.model.clone(),
            asked: String::new(),
        };
        let relayed = match super::super::move_recover::relayed_provider(there.carry.links(), &there.machine) {
            Ok(relayed) => relayed,
            Err(error) => {
                let status = Status::from(error);
                progress.fail(failure_of(&status));
                return Err(status);
            }
        };
        // Journaled before it is asked: from here the machine may have it.
        there.carry.journal_fork(&journal_of(&there, &host_operation, &ask)).await;
        let first = ask_machine(
            &relayed,
            &there.machine,
            asked_request(&there.parent.id, &host_operation, &ask),
        )
        .await;
        crate::fault::point("fork-context-asked");
        let first = match first {
            Ok(first) => first,
            Err(Asked::Refused(status)) => {
                there.carry.fork_done(&host_operation).await;
                progress.fail(failure_of(&status));
                return Err(status);
            }
            Err(Asked::Lost(status)) => {
                progress.fail(failure_of(&status));
                let journal = journal_of(&there, &host_operation, &ask);
                self.recover_context_when_linked(there.carry.clone(), journal, None);
                return Err(status);
            }
        };
        ask.asked = first.id.as_ref().map(|id| id.value.clone()).unwrap_or_default();
        let journal = journal_of(&there, &host_operation, &ask);
        there.carry.journal_fork(&journal).await;
        tokio::spawn(self.clone().finish_context_there(
            relayed,
            there,
            journal,
            first,
            progress,
            identity::actor(caller),
        ));
        Ok(operation)
    }

    /// Follows the machine's fork to its end, then writes the host's
    /// records of it and ends `progress`. A link lost before the end
    /// fails `progress`, and the fork is finished once the link is back.
    async fn finish_context_there(
        self,
        relayed: Relayed,
        there: ContextThere,
        journal: ForkJournal,
        first: pb::Operation,
        progress: Progress,
        by: pb::Actor,
    ) {
        let machine = there.machine.clone();
        let lost = move || {
            failed(
                Code::Unavailable,
                &ContextThereError::LinkLost { machine },
                "reconnect the machine: once its link is up this Mac lists the fork if the machine made it",
                None,
            )
        };
        let ended = match follow_to_end(&relayed, &there.machine, first, &progress, lost).await {
            Ok(ended) => ended,
            Err(status) => {
                progress.fail(failure_of(&status));
                self.recover_context_when_linked(there.carry.clone(), journal, None);
                return;
            }
        };
        let before_turn = journal.context.as_ref().map_or(0, |ask| ask.before_turn);
        match self.conclude_context_there(&there, &journal.operation, ended, before_turn, Some(by)).await {
            Ok(session) => progress.succeed(Some(pb::operation::Result::Session(pb::SessionId { value: session }))),
            Err(status) => progress.fail(failure_of(&status)),
        }
    }

    /// The machine's fork operation `ended` ended: a fork it made gets the
    /// host's records, and the journal goes either way. A record that
    /// can't be written keeps the journal, so the next start tries again.
    async fn conclude_context_there(
        &self,
        there: &ContextThere,
        operation: &str,
        ended: pb::Operation,
        before_turn: u32,
        by: Option<pb::Actor>,
    ) -> Result<String, Status> {
        let session = match made_by(&there.machine, ended) {
            Ok(session) => session,
            Err(status) => {
                there.carry.fork_done(operation).await;
                return Err(status);
            }
        };
        self.record_context_there(there, &session, before_turn).await?;
        there.carry.fork_done(operation).await;
        self.tell.session(&session, pb::Change::Created, by.clone()).await;
        publish_family(&self.tell, &there.parent.id, by).await;
        Ok(session)
    }

    /// Finishes the context-only fork `journal` names once its machine's
    /// link is up (after `started`, when given): asks the machine again
    /// under the same key, or follows the operation it named, and writes
    /// the host's records when it ends. A machine that refuses, or no
    /// longer knows the operation, ends it with nothing listed.
    pub(super) fn recover_context_when_linked(
        &self,
        carry: Carry,
        journal: ForkJournal,
        started: Option<Arc<crate::restart::Started>>,
    ) {
        let svc = self.clone();
        let operations = started.as_ref().map(|started| started.operations().clone());
        if let Some(operations) = &operations {
            operations.record(
                &journal.operation,
                "fork",
                &journal.phase,
                crate::restart::OperationState::Pending {
                    waits_for: format!("the start to finish, then the link of {}", journal.to),
                },
            );
        }
        tokio::spawn(async move {
            if let Some(started) = started {
                started.wait().await;
            }
            let operation = journal.operation.clone();
            let phase = journal.phase.clone();
            let outcome = svc.recover_context(carry, journal).await;
            if let Some(operations) = &operations {
                operations.record(
                    &operation,
                    "fork",
                    &phase,
                    match &outcome {
                        Ok(()) => crate::restart::OperationState::Finished,
                        Err(error) => crate::restart::OperationState::Stuck { reason: status::chain(error) },
                    },
                );
            }
            if let Err(error) = outcome {
                crate::log::say!(
                    "humatad2: the context-only fork {operation} cut short wasn't listed: {}. Look at the parent's machine: the fork may run there without this Mac listing it",
                    status::chain(&error)
                );
            }
        });
    }

    async fn recover_context(&self, carry: Carry, journal: ForkJournal) -> Result<(), Status> {
        let Some(ask) = journal.context.clone() else {
            return Ok(());
        };
        let loaded = self.load(&journal.parent).await?;
        let Some(parent_detail) = loaded.detail.clone() else {
            return Err(failed(
                Code::FailedPrecondition,
                &ContextThereError::NoParentDetail {
                    session: journal.parent.clone(),
                },
                "look at the parent's machine for the fork; this Mac can't list it",
                None,
            ));
        };
        let there = ContextThere {
            carry: carry.clone(),
            parent: loaded.record,
            parent_detail,
            machine: journal.to.clone(),
            title: ask.title.clone(),
        };
        let mut state = super::super::move_recover::watch_provider(carry.links(), &there.machine);
        loop {
            let Ok(up) = state.wait_for(|now| matches!(now, LinkState::Up { .. })).await.map(|now| *now) else {
                crate::log::say!(
                    "humatad2: the context-only fork {} of session {} was cut short, and {} was forgotten before it was listed. Nothing was deleted",
                    journal.operation, journal.parent, there.machine
                );
                return Ok(());
            };
            let asked = match super::super::move_recover::relayed_provider(carry.links(), &there.machine) {
                Ok(relayed) => {
                    let first = if ask.asked.is_empty() {
                        ask_machine(
                            &relayed,
                            &there.machine,
                            asked_request(&there.parent.id, &journal.operation, &ask),
                        )
                        .await
                    } else {
                        Ok(pb::Operation {
                            id: Some(pb::OperationId { value: ask.asked.clone() }),
                            state: pb::OperationState::Running as i32,
                            ..Default::default()
                        })
                    };
                    match first {
                        Ok(first) => watch_end(&relayed, &there.machine, first).await,
                        Err(asked) => Err(asked),
                    }
                }
                Err(error) => Err(Asked::Lost(Status::from(error))),
            };
            match asked {
                Ok(ended) => {
                    return self
                        .conclude_context_there(&there, &journal.operation, ended, ask.before_turn, None)
                        .await
                        .map(drop);
                }
                Err(Asked::Refused(status)) => {
                    carry.fork_done(&journal.operation).await;
                    return Err(status);
                }
                // The link went again: wait for the next one.
                Err(Asked::Lost(_)) => {
                    if state.wait_for(|now| *now != up).await.is_err() {
                        return Ok(());
                    }
                }
            }
        }
    }

    /// The host's record of fork `session` on the parent's machine: the
    /// parent's folder and clones on their branches, and where it came
    /// from. Written once: a fork already recorded is left as it is.
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
            if store.fork_origin(&origin.session_id)?.is_some() {
                return Ok(());
            }
            store.put_session(&record)?;
            store.put_session_detail(&detail)?;
            store.put_fork_origin(&origin)
        })
        .await?
        .map_err(|error| status::store(error, &self.machine, "save the fork"))
    }
}
