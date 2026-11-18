//! Give a context-only fork its own copy of the code (FK-9, D185, D187): the
//! access rule and the checks are answered at once; the copy, the move of
//! the conversation and the start of the agent in the new folder run as the
//! `GiveOwnCopy` operation. A fork on another machine is given its copy
//! there: the host relays the call, shows the machine's steps as its own,
//! and then takes up the fork's new branch and folder in its record.

use std::path::Path;
use std::sync::Arc;

use humata2_access::{Capability, Target};
use humata2_grpc::humata::v1 as pb;
use humata2_sessions::{OwnCopyError, Starter};
use humata2_store::{StoreError, TabRecord};
use tonic::{Code, Status};

use super::archive_remote::{follow_to_end, machine_failed};
use super::lineage::publish_family;
use super::{Loaded, SessionSvc, Tell, advance, turns, watch};
use crate::channel_service::{Gated, blocking, error_of, failed};
use crate::host_relay::{Relayed, assigned};
use crate::identity::{self, Caller};
use crate::operations::{Progress, Started};
use crate::remote_work::RemoteWork;
use crate::restart::Tokens;
use crate::status;

#[derive(Debug, thiserror::Error)]
enum Refusal {
    #[error(
        "session {session} isn't a context-only fork, so it doesn't work in a parent's code to copy"
    )]
    NotContextOnly { session: String },
}

/// Why the host couldn't see a copy on another machine through.
#[derive(Debug, thiserror::Error)]
enum RemoteCopyError {
    #[error("the link to {machine} was lost while it gave session {session} its own copy, so it isn't known whether it finished")]
    LinkLost { machine: String, session: String },
    #[error("{machine} ended the copy of session {session} without saying why")]
    NoAnswer { machine: String, session: String },
    #[error("{machine} gave session {session} its own copy, but couldn't say where it is now")]
    NotShown {
        machine: String,
        session: String,
        #[source]
        source: Status,
    },
    #[error("{machine} gave session {session} its own copy, but this Mac couldn't record its new branch")]
    Record {
        machine: String,
        session: String,
        #[source]
        source: StoreError,
    },
}

impl SessionSvc {
    /// GiveOwnCopy: the fork's own clones on a new branch, its conversation
    /// moved to them, and its agent started again there when it was running.
    pub(super) async fn give_own_copy_of(
        &self,
        caller: &Caller,
        request: pb::GiveOwnCopyRequest,
        approved: bool,
    ) -> Result<Gated<pb::Operation>, Status> {
        Self::refuse_peer(caller)?;
        let fork = request
            .fork
            .as_ref()
            .map(|fork| fork.value.clone())
            .filter(|fork| !fork.is_empty())
            .ok_or_else(|| status::missing("fork"))?;
        let loaded = self.load(&fork).await?;
        // A session's own agent may change its own copy; another session's
        // needs the rule.
        let own = matches!(caller, Caller::Agent(agent) if agent.session.id.as_str() == fork);
        if !own
            && let Some(asked) = self.gate(
                caller,
                Capability::OwnSessionManage,
                Target::Session(self.session_ref(&loaded.record)),
                approved,
            )?
        {
            return Ok(Gated::Ask(asked));
        }
        if let Some(operation) = self
            .operations
            .existing(caller, request.idempotency_key.as_ref())?
        {
            return Ok(Gated::Done(operation));
        }
        let store = self.starter.store.clone();
        let key = fork.clone();
        let origin = blocking(move || store.fork_origin(&key))
            .await?
            .map_err(|error| status::store(error, &self.machine, "look up where the fork came from"))?;
        if origin.is_none_or(|origin| origin.kind != pb::ForkKind::ContextOnly as i32) {
            return Err(failed(
                Code::FailedPrecondition,
                &Refusal::NotContextOnly {
                    session: fork.clone(),
                },
                "give the copy only to a context-only fork; a fork that carries its code already has its own",
                None,
            ));
        }
        if let Some((machine, remote)) = self.lives_elsewhere(&loaded.record) {
            return self
                .own_copy_remote(caller, request, &loaded, machine, &remote)
                .await
                .map(Gated::Done);
        }
        let starter = self.starter.clone();
        let key = fork.clone();
        blocking(move || starter.check_own_copy(&key))
            .await?
            .map_err(|error| refused(&error))?;
        turns::refresh_hooks().await;

        let steps = steps_of(&loaded);
        let keys = steps.iter().map(|(key, _)| key.clone()).collect();
        let progress = match self.operations.start_steps(
            caller,
            pb::OperationKind::GiveOwnCopy,
            target_of(&fork),
            &steps,
            false,
            request.idempotency_key,
        )? {
            Started::New(progress) => progress,
            Started::Existing(operation) => return Ok(Gated::Done(operation)),
        };
        let operation = progress.operation();
        tokio::spawn(run_own_copy(
            self.starter.clone(),
            self.tell.clone(),
            self.tokens.clone(),
            self.lifecycle.lock_of(&fork),
            fork,
            progress,
            keys,
            identity::actor(caller),
        ));
        Ok(Gated::Done(operation))
    }

    /// GiveOwnCopy for a fork on `machine`, after the host's own access
    /// check: the machine checks the fork and its refusal is answered at
    /// once, as a copy here is; an offline machine is refused at once too.
    /// The machine's operation is then followed on the host's.
    async fn own_copy_remote(
        &self,
        caller: &Caller,
        request: pb::GiveOwnCopyRequest,
        loaded: &Loaded,
        machine: String,
        remote: &RemoteWork,
    ) -> Result<pb::Operation, Status> {
        let fork = loaded.record.id.clone();
        let relayed = remote.relayed(&machine).map_err(Status::from)?;
        // The caller's key, so the machine answers a retry of this one copy
        // with the operation it already started.
        let key = request
            .idempotency_key
            .as_ref()
            .map_or_else(|| uuid::Uuid::new_v4().simple().to_string(), |key| key.value.clone());
        let asked = pb::GiveOwnCopyRequest {
            idempotency_key: Some(pb::IdempotencyKey {
                value: format!("host-own-copy-{key}"),
            }),
            fork: Some(pb::SessionId { value: fork.clone() }),
        };
        let first = relayed
            .session()
            .give_own_copy(assigned(asked, None).map_err(Status::from)?)
            .await;
        let first = match first {
            Ok(first) => first.into_inner(),
            Err(source) => return Err(remote.peer_refused(&machine, source).await),
        };
        let progress = match self.operations.start_steps(
            caller,
            pb::OperationKind::GiveOwnCopy,
            target_of(&fork),
            &steps_of(loaded),
            false,
            request.idempotency_key,
        )? {
            Started::New(progress) => progress,
            Started::Existing(operation) => return Ok(operation),
        };
        let operation = progress.operation();
        tokio::spawn(self.clone().run_own_copy_remote(
            relayed,
            machine,
            fork,
            first,
            progress,
            identity::actor(caller),
        ));
        Ok(operation)
    }

    /// Follows the machine's copy to its end on `progress`, then takes up
    /// the fork's new branch and folder in the host's record.
    async fn run_own_copy_remote(
        self,
        relayed: Relayed,
        machine: String,
        fork: String,
        first: pb::Operation,
        progress: Progress,
        by: pb::Actor,
    ) {
        let guard = self.lifecycle.lock_of(&fork).lock_owned().await;
        let lost = || {
            failed(
                Code::Unavailable,
                &RemoteCopyError::LinkLost {
                    machine: machine.clone(),
                    session: fork.clone(),
                },
                "reconnect the machine, then look at the fork: the machine finishes the copy on its own",
                None,
            )
        };
        let outcome = match follow_to_end(&relayed, &machine, first, &progress, lost).await {
            Ok(ended) if ended.state == pb::OperationState::Succeeded as i32 => {
                self.take_up_own_copy(&relayed, &machine, &fork).await
            }
            Ok(ended) => Err(ended.error.unwrap_or_else(|| {
                error_of(
                    &RemoteCopyError::NoAnswer {
                        machine: machine.clone(),
                        session: fork.clone(),
                    },
                    "look at the fork's last steps, then give it its own copy again",
                    None,
                )
            })),
            Err(status) => Err(failure_of(&status)),
        };
        drop(guard);
        match outcome {
            Ok(()) => progress.succeed(Some(pb::operation::Result::Session(pb::SessionId {
                value: fork.clone(),
            }))),
            Err(error) => progress.fail(error),
        }
        self.tell
            .session(&fork, pb::Change::Updated, Some(by.clone()))
            .await;
        publish_family(&self.tell, &fork, Some(by)).await;
    }

    /// The host's record of `fork` after its machine gave it its own copy:
    /// the branch the machine shows for each repository, and the fork's own
    /// folder, laid out on this Mac as its starts are.
    async fn take_up_own_copy(&self, relayed: &Relayed, machine: &str, fork: &str) -> Result<(), pb::Error> {
        let fix = "the fork has its own copy on the machine; reconnect the machine and open the fork to see it";
        let not_shown = |source: Status| {
            error_of(
                &RemoteCopyError::NotShown {
                    machine: machine.to_owned(),
                    session: fork.to_owned(),
                    source,
                },
                fix,
                None,
            )
        };
        let asked = pb::GetSessionRequest {
            id: Some(pb::SessionId { value: fork.to_owned() }),
        };
        let call = assigned(asked, None).map_err(|error| not_shown(Status::from(error)))?;
        let shown = relayed
            .session()
            .get_session(call)
            .await
            .map_err(|source| not_shown(machine_failed(machine, source)))?
            .into_inner();
        let folder_name = shown
            .repos
            .first()
            .and_then(|repo| Path::new(&repo.path).parent()?.file_name())
            .map(|name| name.to_string_lossy().into_owned());
        let (store, layout, id) = (self.starter.store.clone(), self.starter.layout.clone(), fork.to_owned());
        let saved = blocking(move || -> Result<(), StoreError> {
            let (Some(mut record), Some(mut detail)) = (store.session(&id)?, store.session_detail(&id)?) else {
                return Ok(());
            };
            let channel = store.channel(&record.channel)?;
            let folder = match (folder_name, channel) {
                (Some(name), Some(channel)) => Some(layout.session(&channel.folder, &name)),
                _ => None,
            };
            for repo in &mut detail.repos {
                let theirs = shown.repos.iter().find(|theirs| {
                    theirs
                        .repo
                        .as_ref()
                        .is_some_and(|named| named.owner == repo.owner && named.name == repo.name)
                });
                if let Some(theirs) = theirs {
                    repo.branch.clone_from(&theirs.branch);
                }
                if let Some(folder) = &folder {
                    repo.path = folder.join(&repo.name);
                }
            }
            detail.revision += 1;
            if let Some(folder) = folder {
                record.folder = folder;
            }
            store.put_session(&record)?;
            store.put_session_detail(&detail)
        })
        .await
        .map_err(|status| failure_of(&status))?;
        saved.map_err(|source| {
            error_of(
                &RemoteCopyError::Record {
                    machine: machine.to_owned(),
                    session: fork.to_owned(),
                    source,
                },
                "check that this Mac's disk has room, then restart Humata; the fork already has its own copy on the machine",
                None,
            )
        })
    }
}

/// The steps of a copy of `loaded`, as `(key, title)`: one per repository,
/// then the conversation's move and the agent's start. The machine's copy
/// has the same keys, so the host shows its steps as they are.
fn steps_of(loaded: &Loaded) -> Vec<(String, String)> {
    let mut steps: Vec<(String, String)> = loaded
        .detail
        .iter()
        .flat_map(|detail| detail.repos.iter())
        .map(|repo| (format!("copy_code:{}", repo.name), format!("Copying {}'s code", repo.name)))
        .collect();
    for (key, label) in [
        ("move_conversation", "Moving the conversation"),
        ("launch", "Starting the agent again"),
        ("verify", "Checking it took up its conversation"),
    ] {
        steps.push((key.to_owned(), label.to_owned()));
    }
    steps
}

fn target_of(fork: &str) -> pb::EntityRef {
    pb::EntityRef {
        entity: Some(pb::entity_ref::Entity::Session(pb::SessionId {
            value: fork.to_owned(),
        })),
    }
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

/// The status code and fix for an own-copy refusal.
fn refused(error: &OwnCopyError) -> Status {
    failed(code_of(error), error, fix_of(error), None)
}

fn code_of(error: &OwnCopyError) -> Code {
    match error {
        OwnCopyError::NoSession { .. } => Code::NotFound,
        OwnCopyError::Store { .. }
        | OwnCopyError::Start { .. }
        | OwnCopyError::Agent { .. }
        | OwnCopyError::Git { .. }
        | OwnCopyError::Copy { .. }
        | OwnCopyError::Resume { .. }
        | OwnCopyError::NotResumed { .. }
        | OwnCopyError::EndHolder { .. }
        | OwnCopyError::HolderAnswers { .. } => Code::Unavailable,
        OwnCopyError::NotAFork { .. }
        | OwnCopyError::NoParent { .. }
        | OwnCopyError::HasOwnCode { .. }
        | OwnCopyError::NoAgent { .. }
        | OwnCopyError::Busy { .. }
        | OwnCopyError::NoConversation { .. } => Code::FailedPrecondition,
    }
}

fn fix_of(error: &OwnCopyError) -> &'static str {
    match error {
        OwnCopyError::NoSession { .. } => "list the sessions to see their ids",
        OwnCopyError::NotAFork { .. } => {
            "give the copy only to a context-only fork; a session started fresh has no parent's code"
        }
        OwnCopyError::NoParent { .. } => {
            "the parent's clones are gone with it, so start a new session in the repositories instead"
        }
        OwnCopyError::HasOwnCode { .. } => {
            "nothing to do: the fork already works in its own copy of the code"
        }
        OwnCopyError::NoAgent { .. } => "start a new session in the fork's code",
        OwnCopyError::Busy { .. } => "wait for the agent to start, then give the copy again",
        OwnCopyError::Agent { .. } => {
            "pick an agent the Settings list has, then give the copy again"
        }
        OwnCopyError::Start { .. } | OwnCopyError::Store { .. } => {
            "fix the cause the message names, then give the copy again; the fork still works in its parent's code"
        }
        OwnCopyError::Git { .. } => {
            "check that the parent's clone is still on disk and readable, then give the copy again; the fork still works in its parent's code"
        }
        OwnCopyError::Copy { .. } => {
            "check that the disk has room and the folder can be written, then give the copy again"
        }
        OwnCopyError::NoConversation { .. } => {
            "the fork's conversation file is gone, so start a new session in the fork's code instead"
        }
        OwnCopyError::HolderAnswers { .. } => {
            "stop the fork's agent from its terminal, then give the copy again"
        }
        OwnCopyError::EndHolder { .. } => {
            "end the fork's terminal holder as the cause says, then give the copy again; the fork still works in its parent's code"
        }
        OwnCopyError::Resume { .. } | OwnCopyError::NotResumed { .. } => {
            "the fork has its own copy now; resume it to start its agent again"
        }
    }
}

/// Runs the copy for `progress` under the fork's lock, so it doesn't
/// interleave with an archive, a push or another copy of it.
#[allow(clippy::too_many_arguments)]
async fn run_own_copy(
    starter: Starter,
    tell: Tell,
    tokens: Arc<Tokens>,
    lock: Arc<tokio::sync::Mutex<()>>,
    fork: String,
    progress: Progress,
    keys: Vec<String>,
    by: pb::Actor,
) {
    let guard = lock.lock_owned().await;
    let mut current = keys.first().cloned().unwrap_or_default();
    if let Some(first) = keys.first() {
        progress.set_step(first, pb::StepState::Running, None);
    }
    let env = |tab: &TabRecord| tokens.env(tab);
    let outcome = {
        let mut note = |event: &str| {
            crate::fault::point(&format!("own_copy.{event}"));
            current = advance(&progress, &keys, event);
        };
        starter.give_own_copy(&fork, &env, &mut note).await
    };
    drop(guard);
    let relaunched = matches!(outcome, Ok(true));
    match outcome {
        Ok(started_again) => {
            if started_again {
                progress.set_step("verify", pb::StepState::Done, None);
            } else {
                progress.set_step("launch", pb::StepState::Skipped, None);
                progress.set_step("verify", pb::StepState::Skipped, None);
            }
            progress.succeed(Some(pb::operation::Result::Session(pb::SessionId {
                value: fork.clone(),
            })));
        }
        Err(error) => {
            progress.set_step(&current, pb::StepState::Failed, Some(status::chain(&error)));
            progress.fail(error_of(&error, fix_of(&error), None));
        }
    }
    tell.session(&fork, pb::Change::Updated, Some(by.clone()))
        .await;
    publish_family(&tell, &fork, Some(by)).await;
    if relaunched {
        watch::agent(tell, fork);
    }
}
