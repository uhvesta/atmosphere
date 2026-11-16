//! Give a context-only fork its own copy of the code (FK-9, D185, D187): the
//! access rule and the checks are answered at once; the copy, the move of
//! the conversation and the start of the agent in the new folder run as the
//! `GiveOwnCopy` operation.

use std::sync::Arc;

use humata2_access::{Capability, Target};
use humata2_grpc::humata::v1 as pb;
use humata2_sessions::{OwnCopyError, Starter};
use humata2_store::TabRecord;
use tonic::{Code, Status};

use super::fork::not_built_part;
use super::lineage::publish_family;
use super::{SessionSvc, Tell, advance, turns, watch};
use crate::channel_service::{Gated, blocking, error_of, failed};
use crate::identity::{self, Caller};
use crate::operations::{Progress, Started};
use crate::restart::Tokens;
use crate::status;

const GIVE_OWN_COPY: &str = "humata.v1.SessionService.GiveOwnCopy";

#[derive(Debug, thiserror::Error)]
enum Refusal {
    #[error(
        "session {session} isn't a context-only fork, so it doesn't work in a parent's code to copy"
    )]
    NotContextOnly { session: String },
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
        if self.lives_elsewhere(&loaded.record).is_some() {
            return Err(not_built_part(GIVE_OWN_COPY, "a session on another machine"));
        }
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
        let starter = self.starter.clone();
        let key = fork.clone();
        blocking(move || starter.check_own_copy(&key))
            .await?
            .map_err(|error| refused(&error))?;
        turns::refresh_hooks().await;

        let mut keys = Vec::new();
        let mut steps = Vec::new();
        for repo in loaded.detail.iter().flat_map(|detail| detail.repos.iter()) {
            keys.push(format!("copy_code:{}", repo.name));
            steps.push((
                format!("copy_code:{}", repo.name),
                format!("Copying {}'s code", repo.name),
            ));
        }
        for (key, label) in [
            ("move_conversation", "Moving the conversation"),
            ("launch", "Starting the agent again"),
            ("verify", "Checking it took up its conversation"),
        ] {
            keys.push(key.to_owned());
            steps.push((key.to_owned(), label.to_owned()));
        }
        let target = pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Session(pb::SessionId {
                value: fork.clone(),
            })),
        };
        let progress = match self.operations.start_steps(
            caller,
            pb::OperationKind::GiveOwnCopy,
            target,
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
