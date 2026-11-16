//! Archive and restore of a session that runs on another machine (MV-7 to
//! MV-9, LD-2 to LD-5, D195). The machine archives it as it archives its own
//! (stops the agent, frees its clones once the undo window closes) and the
//! host writes an archive row of its own, so the session leaves the plain
//! list and shows `archive`. Restoring either asks the machine to undo its
//! archive in place, or, on this Mac, brings the session here: its clones
//! are cloned again from the branch, the conversation is read from the
//! machine while it is online, the agent is resumed in it, and the record
//! names this Mac.
//!
//! Nothing here waits on a timer: the host follows the machine's own
//! operation until it ends.

use std::collections::HashMap;

use humata2_grpc::humata::v1 as pb;
use humata2_store::{ArchiveRow, ArchiveState, SessionRecord, StoreError};
use prost::Message;
use tonic::{Code, Status};

use super::lineage::publish_family;
use super::{Gated, Loaded, SessionSvc, now_ms, run_resume, start_error, watch};
use crate::channel_service::{blocking, error_of, failed, timestamp_ms};
use crate::carry::Place;
use crate::host_relay::{Links, RelayError, Relayed, assigned};
use crate::identity::{self, Caller};
use crate::operations::{Progress, Started};
use crate::status;

/// Why an archive or restore of a session on another machine couldn't go on.
#[derive(Debug, thiserror::Error)]
enum RemoteError {
    #[error("this daemon has no way to reach other machines, so session {session} on {machine} can't be archived or restored from here")]
    NoLinks { session: String, machine: String },
    #[error("the link to {machine} was lost while it was {doing}, so it isn't known whether it finished")]
    LinkLost { machine: String, doing: &'static str },
    #[error("{machine} ended the {doing} without saying how it went")]
    NoAnswer { machine: String, doing: &'static str },
    #[error("couldn't save session {session}'s records on this machine")]
    Store {
        session: String,
        #[source]
        source: StoreError,
    },
    #[error("session {session} can be restored on {machine} or on this Mac; restoring it on another machine isn't built yet")]
    ElsewhereNotBuilt { session: String, machine: String },
    #[error("session {session} runs on {machine} and has no clone folder here to restore into")]
    NoDetail { session: String, machine: String },
}

/// A call to `machine` that failed: its own answer stays as it is, a link
/// that broke under the call is "offline".
fn machine_failed(machine: &str, source: Status) -> Status {
    if status::error_of(&source).is_none()
        && matches!(source.code(), Code::Unavailable | Code::Unknown | Code::Cancelled)
    {
        return Status::from(RelayError::Offline {
            machine: machine.to_owned(),
        });
    }
    source
}

fn finished(operation: &pb::Operation) -> bool {
    operation.state != pb::OperationState::Running as i32
}

/// Shows the steps `operation` has on the host's `progress`.
fn mirror_steps(
    operation: &pb::Operation,
    progress: &Progress,
    seen: &mut HashMap<String, (i32, String)>,
) {
    for step in &operation.steps {
        let now = (step.state, step.output_tail.clone());
        if seen.get(&step.key) == Some(&now) {
            continue;
        }
        let state = pb::StepState::try_from(step.state).unwrap_or(pb::StepState::Waiting);
        let tail = Some(step.output_tail.clone()).filter(|tail| !tail.is_empty());
        progress.set_step(&step.key, state, tail);
        seen.insert(step.key.clone(), now);
    }
}

/// Follows the machine's operation `first` until it ends, showing its steps
/// on `progress`. A link that breaks first is an error: the machine goes on
/// with the work on its own.
async fn follow_to_end(
    relayed: &Relayed,
    machine: &str,
    first: pb::Operation,
    progress: &Progress,
    doing: &'static str,
) -> Result<pb::Operation, Status> {
    let mut seen = HashMap::new();
    mirror_steps(&first, progress, &mut seen);
    if finished(&first) {
        return Ok(first);
    }
    let id = first.id.map(|id| id.value).unwrap_or_default();
    let lost = || {
        failed(
            Code::Unavailable,
            &RemoteError::LinkLost {
                machine: machine.to_owned(),
                doing,
            },
            "reconnect the machine and look at the session: the machine finishes what it started on its own",
            None,
        )
    };
    let mut stream = relayed
        .operation()
        .watch_operation(pb::WatchOperationRequest {
            id: Some(pb::OperationId { value: id }),
            from_version: 0,
        })
        .await
        .map_err(|source| machine_failed(machine, source))?
        .into_inner();
    loop {
        match stream.message().await {
            Ok(Some(operation)) => {
                mirror_steps(&operation, progress, &mut seen);
                if finished(&operation) {
                    return Ok(operation);
                }
            }
            Ok(None) | Err(_) => return Err(lost()),
        }
    }
}

/// What the machine's ended operation says went wrong.
fn operation_error(machine: &str, doing: &'static str, ended: &pb::Operation) -> pb::Error {
    ended.error.clone().unwrap_or_else(|| {
        error_of(
            &RemoteError::NoAnswer {
                machine: machine.to_owned(),
                doing,
            },
            "look at the session on the machine, then try again",
            None,
        )
    })
}

impl SessionSvc {
    /// The host's links to other machines, when this daemon has them.
    fn links_to(&self, session: &str, machine: &str) -> Result<Links, Status> {
        self.carry.get().map(|carry| carry.links().clone()).ok_or_else(|| {
            failed(
                Code::FailedPrecondition,
                &RemoteError::NoLinks {
                    session: session.to_owned(),
                    machine: machine.to_owned(),
                },
                "archive and restore it from the host",
                None,
            )
        })
    }

    fn relayed_to(&self, session: &str, machine: &str) -> Result<Relayed, Status> {
        self.links_to(session, machine)?
            .relayed(machine)
            .map_err(Status::from)
    }

    /// PlanArchive for a session on `machine`: the machine reads its own
    /// clones and answers with its plan and the token ArchiveSession takes.
    pub(super) async fn plan_archive_remote(
        &self,
        id: &str,
        machine: &str,
    ) -> Result<pb::ArchivePlan, Status> {
        let relayed = self.relayed_to(id, machine)?;
        let request = pb::PlanArchiveRequest {
            id: Some(pb::SessionId { value: id.to_owned() }),
        };
        relayed
            .session()
            .plan_archive(assigned(request, None).map_err(Status::from)?)
            .await
            .map(tonic::Response::into_inner)
            .map_err(|source| machine_failed(machine, source))
    }

    /// ArchiveSession for a session on `machine`: the host's operation shows
    /// the machine's, and once the machine has archived the session the host
    /// writes its own archive row.
    pub(super) async fn archive_remote(
        &self,
        caller: &Caller,
        request: pb::ArchiveSessionRequest,
        loaded: Loaded,
        choice: pb::ArchiveChoice,
        machine: String,
    ) -> Result<Gated<pb::Operation>, Status> {
        let id = loaded.record.id.clone();
        // An offline machine answers at once and changes nothing (CH-4).
        self.relayed_to(&id, &machine)?;
        let steps = [
            ("stop", "Stopping the agent"),
            ("summarize", "Writing what the session had"),
            ("free_clones", "Deleting the clones when the undo window closes"),
        ]
        .map(|(key, title)| (key.to_owned(), title.to_owned()));
        let target = pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Session(pb::SessionId { value: id.clone() })),
        };
        let progress = match self.operations.start_steps(
            caller,
            pb::OperationKind::ArchiveSession,
            target,
            &steps,
            false,
            request.idempotency_key.clone(),
        )? {
            Started::New(progress) => progress,
            Started::Existing(operation) => return Ok(Gated::Done(operation)),
        };
        let operation = progress.operation();
        let asked = pb::ArchiveSessionRequest {
            idempotency_key: None,
            id: Some(pb::SessionId { value: id }),
            choice: choice as i32,
            plan: request.plan,
        };
        tokio::spawn(self.clone().run_archive_remote(
            progress,
            asked,
            loaded,
            machine,
            identity::actor(caller),
        ));
        Ok(Gated::Done(operation))
    }

    async fn run_archive_remote(
        self,
        progress: Progress,
        asked: pb::ArchiveSessionRequest,
        loaded: Loaded,
        machine: String,
        by: pb::Actor,
    ) {
        let id = loaded.record.id.clone();
        let guard = self.lifecycle.lock_of(&id).lock_owned().await;
        progress.set_step("stop", pb::StepState::Running, None);
        let ended = async {
            let relayed = self.relayed_to(&id, &machine)?;
            let first = relayed
                .session()
                .archive_session(assigned(asked, None).map_err(Status::from)?)
                .await
                .map_err(|source| machine_failed(&machine, source))?
                .into_inner();
            follow_to_end(&relayed, &machine, first, &progress, "archive").await
        }
        .await;
        let ended = match ended {
            Ok(ended) => ended,
            Err(refused) => {
                progress.fail(status::error_of(&refused).unwrap_or_else(|| pb::Error {
                    message: refused.message().to_owned(),
                    fix: "try the archive again".to_owned(),
                    retry: None,
                    kind: None,
                }));
                return;
            }
        };
        if ended.state != pb::OperationState::Succeeded as i32 {
            progress.fail(operation_error(&machine, "archive", &ended));
            return;
        }
        if let Err(refused) = self.write_remote_row(&loaded, &machine).await {
            progress.fail(status::error_of(&refused).unwrap_or_default());
            return;
        }
        drop(guard);
        // The session no longer waits on anyone (IN-22, TP-15).
        if let Err(error) = self.end_grants_of(&id).await {
            crate::log::say!(
                "humatad2: ending archived session {id}'s grants: {}",
                error.message()
            );
        }
        if let Err(error) = self
            .withdraw_requests_of(&id, pb::WithdrawReason::SessionArchived)
            .await
        {
            crate::log::say!(
                "humatad2: withdrawing archived session {id}'s requests: {}",
                error.message()
            );
        }
        progress.succeed(Some(pb::operation::Result::Session(pb::SessionId {
            value: id.clone(),
        })));
        self.tell.session(&id, pb::Change::Updated, Some(by.clone())).await;
        publish_family(&self.tell, &id, Some(by)).await;
    }

    /// The host's own row for a session its machine archived. It is written
    /// archived at once: the host has no clones of the session to free.
    async fn write_remote_row(&self, loaded: &Loaded, machine: &str) -> Result<(), Status> {
        let id = loaded.record.id.clone();
        let archived = now_ms();
        let pushed = self
            .remote
            .as_ref()
            .and_then(|remote| remote.view(machine, &id))
            .map(|view| view.pointers)
            .unwrap_or_default();
        let summary = pb::ArchiveSummary {
            archived_at: Some(timestamp_ms(archived)),
            repos: loaded
                .detail
                .iter()
                .flat_map(|detail| detail.repos.iter())
                .map(|repo| pb::archive_summary::Repo {
                    repo: Some(pb::RepoRef {
                        owner: repo.owner.clone(),
                        name: repo.name.clone(),
                    }),
                    branch: repo.branch.clone(),
                    last_pushed_sha: pushed
                        .iter()
                        .find(|pointer| {
                            pointer.pushed
                                && pointer.repo.as_ref().is_some_and(|named| {
                                    named.owner == repo.owner && named.name == repo.name
                                })
                        })
                        .map(|pointer| pointer.head_sha.clone())
                        .unwrap_or_default(),
                    files: Vec::new(),
                    unpushed_subjects: Vec::new(),
                })
                .collect(),
            machine_lost: false,
            lost: Vec::new(),
        };
        let row = ArchiveRow {
            session_id: id.clone(),
            archived_unix_ms: archived,
            state: ArchiveState::Archived,
            summary: summary.encode_to_vec(),
            free_after_unix_ms: archived,
        };
        let store = self.starter.store.clone();
        blocking(move || store.put_archive(&row))
            .await?
            .map_err(|source| {
                failed(
                    Code::Internal,
                    &RemoteError::Store { session: id, source },
                    "restart Humata on this machine and archive the session again",
                    None,
                )
            })
    }

    /// RestoreSession for an archived session of another machine: with no
    /// machine named, or the session's own, the machine undoes its archive
    /// where it is; naming this Mac brings the session here.
    pub(super) async fn restore_remote(
        &self,
        caller: &Caller,
        request: pb::RestoreSessionRequest,
        loaded: Loaded,
        machine: String,
    ) -> Result<Gated<pb::Operation>, Status> {
        let id = loaded.record.id.clone();
        let wanted = request
            .machine
            .as_ref()
            .map(|named| named.value.clone())
            .filter(|named| !named.is_empty());
        let here = wanted.as_deref() == Some(self.machine.as_str());
        match wanted {
            Some(named) if named != self.machine && named != machine => {
                return Err(failed(
                    Code::FailedPrecondition,
                    &RemoteError::ElsewhereNotBuilt {
                        session: id,
                        machine: machine.clone(),
                    },
                    "restore it on its own machine or on this Mac",
                    None,
                ));
            }
            _ => {}
        }
        // An offline machine answers at once and changes nothing (CH-4).
        self.relayed_to(&id, &machine)?;
        let mut steps = Vec::new();
        if here {
            for repo in loaded.detail.iter().flat_map(|detail| detail.repos.iter()) {
                steps.push((format!("clone:{}", repo.name), format!("Cloning {}", repo.name)));
            }
            steps.push((
                "import_conversation".to_owned(),
                "Bringing the conversation back".to_owned(),
            ));
            steps.push(("launch".to_owned(), "Starting the agent again".to_owned()));
            steps.push((
                "verify".to_owned(),
                "Checking it took up its conversation".to_owned(),
            ));
        } else {
            steps.push(("restore".to_owned(), format!("Restoring it on {machine}")));
        }
        let target = pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Session(pb::SessionId { value: id.clone() })),
        };
        let progress = match self.operations.start_steps(
            caller,
            pb::OperationKind::RestoreSession,
            target,
            &steps,
            false,
            request.idempotency_key,
        )? {
            Started::New(progress) => progress,
            Started::Existing(operation) => return Ok(Gated::Done(operation)),
        };
        let operation = progress.operation();
        let by = identity::actor(caller);
        if here {
            tokio::spawn(self.clone().run_restore_here(progress, loaded, machine, by));
        } else {
            tokio::spawn(self.clone().run_restore_there(progress, id, machine, by));
        }
        Ok(Gated::Done(operation))
    }

    /// Undo in place: the machine restores the session where it is, and the
    /// host drops its own archive row.
    async fn run_restore_there(self, progress: Progress, id: String, machine: String, by: pb::Actor) {
        let guard = self.lifecycle.lock_of(&id).lock_owned().await;
        progress.set_step("restore", pb::StepState::Running, None);
        let ended = async {
            let relayed = self.relayed_to(&id, &machine)?;
            let request = pb::RestoreSessionRequest {
                idempotency_key: None,
                id: Some(pb::SessionId { value: id.clone() }),
                machine: None,
            };
            let first = relayed
                .session()
                .restore_session(assigned(request, None).map_err(Status::from)?)
                .await
                .map_err(|source| machine_failed(&machine, source))?
                .into_inner();
            follow_to_end(&relayed, &machine, first, &progress, "restore").await
        }
        .await;
        let ended = match ended {
            Ok(ended) => ended,
            Err(refused) => {
                progress.set_step("restore", pb::StepState::Failed, Some(refused.message().to_owned()));
                progress.fail(status::error_of(&refused).unwrap_or_else(|| pb::Error {
                    message: refused.message().to_owned(),
                    fix: "try the restore again".to_owned(),
                    retry: None,
                    kind: None,
                }));
                return;
            }
        };
        if ended.state != pb::OperationState::Succeeded as i32 {
            progress.set_step("restore", pb::StepState::Failed, None);
            progress.fail(operation_error(&machine, "restore", &ended));
            return;
        }
        let (store, key) = (self.starter.store.clone(), id.clone());
        match blocking(move || store.remove_archive(&key)).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                progress.fail(error_of(
                    &error,
                    "fix the cause the message names, then restore again",
                    None,
                ));
                return;
            }
            Err(refused) => {
                progress.fail(status::error_of(&refused).unwrap_or_default());
                return;
            }
        }
        drop(guard);
        progress.set_step("restore", pb::StepState::Done, None);
        progress.succeed(Some(pb::operation::Result::Session(pb::SessionId {
            value: id.clone(),
        })));
        self.tell.session(&id, pb::Change::Updated, Some(by.clone())).await;
        publish_family(&self.tell, &id, Some(by)).await;
    }

    /// Restore on this Mac: the machine stages the session's records and
    /// conversation and they are copied here and taken in (the same steps a
    /// move uses, without the folder, which was freed), the branch is cloned
    /// again, the record names this Mac and the agent is resumed in its
    /// conversation.
    async fn run_restore_here(self, progress: Progress, loaded: Loaded, machine: String, by: pb::Actor) {
        let id = loaded.record.id.clone();
        let guard = self.lifecycle.lock_of(&id).lock_owned().await;
        let operation = progress.operation().id.map(|id| id.value).unwrap_or_default();
        let repos: Vec<String> = loaded
            .detail
            .iter()
            .flat_map(|detail| detail.repos.iter())
            .map(|repo| repo.name.clone())
            .collect();
        let Some(detail) = loaded.detail.as_ref() else {
            progress.fail(error_of(
                &RemoteError::NoDetail {
                    session: id,
                    machine,
                },
                "restore it on its own machine",
                None,
            ));
            return;
        };

        progress.set_step("import_conversation", pb::StepState::Running, None);
        if let Err(status) = self.take_conversation(&loaded.record, &machine, &operation).await {
            progress.set_step(
                "import_conversation",
                pb::StepState::Failed,
                Some(status.message().to_owned()),
            );
            progress.fail(error_from(&status));
            return;
        }
        progress.set_step("import_conversation", pb::StepState::Done, None);

        for name in &repos {
            progress.set_step(&format!("clone:{name}"), pb::StepState::Running, None);
        }
        if let Err(error) = self.starter.reclone(&loaded.record, detail).await {
            let (_, failure) = start_error(&error);
            for name in &repos {
                progress.set_step(
                    &format!("clone:{name}"),
                    pb::StepState::Failed,
                    Some(status::chain(&error)),
                );
            }
            self.undo_conversation(&loaded.record, &operation).await;
            progress.fail(failure);
            return;
        }
        for name in &repos {
            progress.set_step(&format!("clone:{name}"), pb::StepState::Done, None);
        }

        // Switch: the record names this Mac, and the archive row goes.
        // The mirror first reads the conversation taken here, which repeats
        // what it held from the machine, so nothing shows twice (D211).
        let here = self.machine.clone();
        self.rebase_mirror(&id, &here, None).await;
        let mut record = loaded.record.clone();
        record.machine.clone_from(&self.machine);
        let (store, saved, key) = (self.starter.store.clone(), record.clone(), id.clone());
        let switched = blocking(move || -> Result<(), StoreError> {
            store.put_session(&saved)?;
            store.remove_archive(&key)
        })
        .await;
        match switched {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                self.rebase_mirror(&id, &machine, None).await;
                progress.fail(error_of(
                    &error,
                    "fix the cause the message names, then restore again",
                    None,
                ));
                return;
            }
            Err(refused) => {
                self.rebase_mirror(&id, &machine, None).await;
                progress.fail(error_from(&refused));
                return;
            }
        }
        drop(guard);
        if let Some(remote) = &self.remote {
            remote.forget_session(&machine, &id).await;
        }
        self.release_old_copy(&machine, &id, &record, &operation).await;
        self.tell.session(&id, pb::Change::Updated, Some(by.clone())).await;
        publish_family(&self.tell, &id, Some(by.clone())).await;

        let Ok(loaded) = self.load(&id).await else {
            progress.fail(error_of(
                &RemoteError::NoDetail {
                    session: id,
                    machine,
                },
                "open the session from the list",
                None,
            ));
            return;
        };
        run_resume(
            self.starter.clone(),
            self.tell.clone(),
            by,
            self.tokens.clone(),
            loaded,
            progress,
        )
        .await;
        watch::agent(self.tell.clone(), id);
    }

    /// The machine's copy of the session goes to its trash and its records
    /// are dropped, now that the session runs here; the staged files here go
    /// too. A failure leaves the old copy where it is and is logged.
    async fn release_old_copy(&self, machine: &str, id: &str, record: &SessionRecord, operation: &str) {
        let Some(carry) = self.carry.get() else {
            return;
        };
        if let Err(error) = carry.release(machine, operation, id, &record.channel).await {
            crate::log::say!(
                "humatad2: session {id} runs here now, but its old copy on {machine} is still there: {}",
                error.message()
            );
        }
        self.unstage_here(record, operation).await;
    }

    /// Removes the files a restore staged on this machine.
    async fn unstage_here(&self, record: &SessionRecord, operation: &str) {
        let Some(carry) = self.carry.get() else {
            return;
        };
        let place = Place {
            machine: self.machine.clone(),
            channel: record.channel.clone(),
            folder: String::new(),
        };
        let name = record
            .folder
            .file_name()
            .map(|name| format!("sessions/{}", name.to_string_lossy()))
            .unwrap_or_default();
        if let Err(error) = carry.abort(&self.machine, operation, &record.id, &place, &name, false).await {
            crate::log::say!(
                "humatad2: couldn't remove the files staged here for restoring session {}: {}",
                record.id,
                error.message()
            );
        }
    }

    /// Takes back what taking the conversation in put on this machine.
    async fn undo_conversation(&self, record: &SessionRecord, operation: &str) {
        let Some(carry) = self.carry.get() else {
            return;
        };
        let name = record
            .folder
            .file_name()
            .map(|name| format!("sessions/{}", name.to_string_lossy()))
            .unwrap_or_default();
        let place = Place {
            machine: self.machine.clone(),
            channel: record.channel.clone(),
            folder: String::new(),
        };
        let _ = carry
            .abort(&self.machine, operation, &record.id, &place, &name, true)
            .await;
        let _ = carry
            .abort(&record.machine, operation, &record.id, &place, &name, false)
            .await;
    }

    /// Brings session `record`'s records and conversation from `machine`
    /// here as a move does: the machine stages them (its folder was freed, so
    /// only the records and the agent's files), they are copied across and
    /// taken in, with the agent saved as stopped and ready to resume.
    async fn take_conversation(&self, record: &SessionRecord, machine: &str, operation: &str) -> Result<(), Status> {
        let Some(carry) = self.carry.get().cloned() else {
            return Err(self.links_to(&record.id, machine).err().unwrap_or_else(|| {
                status::internal("the carry driver is missing", "")
            }));
        };
        let channel = self.find_channel(&record.channel).await?;
        let at = |machine: &str| Place {
            machine: machine.to_owned(),
            channel: channel.id.clone(),
            folder: channel.folder.clone(),
        };
        let (from, here) = (at(machine), at(&self.machine));
        let packed = carry.pack(machine, operation, &record.id).await?;
        carry
            .copy(
                &format!("{operation}_stage"),
                (&from, &packed.stage),
                (&here, &format!("{}/{operation}/in", humata2_sessions::CARRY_DIR)),
            )
            .await?;
        carry
            .unpack(&self.machine, operation, &record.id, &channel.id)
            .await
            .map(drop)
    }
}

/// What a machine's removal does with one of its sessions (CH-8, LD-10).
#[derive(Clone)]
pub(crate) enum Removal {
    /// Move it to this machine of the channel.
    MoveTo(String),
    /// Archive it as the user chose in the plan.
    Archive(pb::ArchiveChoice),
}

/// The error a failed call or operation says, whole.
fn error_from(status: &Status) -> pb::Error {
    status::error_of(status).unwrap_or_else(|| pb::Error {
        message: status.message().to_owned(),
        fix: "try again".to_owned(),
        retry: None,
        kind: None,
    })
}

impl SessionSvc {
    /// Moves or archives one session of a machine that is being removed, as
    /// a user who chose it in the removal's dialog would, and waits for the
    /// operation to end. The access rules of the call that asked for the
    /// removal already ran, so the moves and archives here are approved.
    pub(crate) async fn dispose_for_removal(
        &self,
        caller: &Caller,
        session: &str,
        how: Removal,
    ) -> Result<(), pb::Error> {
        let id = Some(pb::SessionId {
            value: session.to_owned(),
        });
        let asked = async {
            match how {
                Removal::MoveTo(to) => {
                    let request = pb::MoveSessionRequest {
                        idempotency_key: None,
                        id,
                        to: Some(pb::MachineId { value: to }),
                    };
                    self.move_session_of(caller, request, true).await
                }
                Removal::Archive(choice) => {
                    let plan = match self
                        .plan_archive_of(caller, pb::PlanArchiveRequest { id: id.clone() }, true)
                        .await?
                    {
                        Gated::Done(plan) => plan,
                        Gated::Ask(_) => return Err(asked_twice()),
                    };
                    let request = pb::ArchiveSessionRequest {
                        idempotency_key: None,
                        id,
                        choice: choice as i32,
                        plan: plan.token,
                    };
                    self.archive_session_of(caller, request, true).await
                }
            }
        }
        .await
        .map_err(|status| error_from(&status))?;
        let started = match asked {
            Gated::Done(operation) => operation,
            Gated::Ask(_) => return Err(error_from(&asked_twice())),
        };
        let operation = started.id.map(|id| id.value).unwrap_or_default();
        let mut watching = self
            .operations
            .progress_of(caller, &operation)
            .map_err(|status| error_from(&status))?
            .watch();
        let ended = loop {
            let now = watching.borrow_and_update().clone();
            if now.state != pb::OperationState::Running as i32 {
                break now;
            }
            if watching.changed().await.is_err() {
                break now;
            }
        };
        if ended.state == pb::OperationState::Succeeded as i32 {
            return Ok(());
        }
        Err(ended.error.unwrap_or_else(|| pb::Error {
            message: format!("session {session}'s work ended without saying how it went"),
            fix: "look at the session, then try the removal again".to_owned(),
            retry: None,
            kind: None,
        }))
    }
}

fn asked_twice() -> Status {
    status::internal(
        "an approved call was sent to Ask me a second time",
        "repeat the call",
    )
}
