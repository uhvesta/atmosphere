//! Continue in another channel… (CV-8, D307): a new top-level session in
//! the chosen channel, set up as that channel's New session (its ready
//! repositories, this machine, a branch from its main clones), whose agent
//! resumes a copy of the session's conversation, from now or from before a
//! turn (CV-2), under a new id. The session itself is untouched.
//!
//! Access is a start in each channel, as a fork's is: the session's (its
//! conversation is read) and the chosen one (a session starts there). A
//! session on another machine continues on that machine, into a channel
//! that machine is in (see `there`).

use std::sync::Arc;

use humata2_access::{AgentId, Capability, ChannelId, MachineId, Target};
use humata2_grpc::humata::v1 as pb;
use humata2_sessions::{ContinueRequest, DESK_CHANNEL, Starter, TurnsError};
use humata2_store::{MainClone, TabRecord};
use pb::invalid_field::Problem;
use tokio::sync::oneshot;
use tonic::{Code, Status};

use super::{COLS, ROWS, SessionSvc, Tell, advance, remote, start_error, turns, watch};
use crate::channel_service::{Gated, blocking, failed};
use crate::identity::{self, Caller};
use crate::operations::{Progress, Started};
use crate::restart::Tokens;
use crate::status;

mod there;

#[derive(Debug, thiserror::Error)]
enum ContinueError {
    #[error("session {session} is archived, so its conversation can't be continued in another channel")]
    SourceArchived { session: String },
    #[error("session {session} has no conversation to continue: its agent hasn't started")]
    NoConversation { session: String },
    #[error("agent {agent}'s conversation holds what Humata can't copy to another folder safely")]
    CantCarry {
        agent: String,
        #[source]
        source: TurnsError,
    },
    #[error("agent {agent} keeps its conversation in a form Humata can't cut before a turn")]
    NoTurns { agent: String },
}

impl SessionSvc {
    pub(super) async fn continue_in_channel_of(
        &self,
        caller: &Caller,
        request: pb::ContinueInChannelRequest,
        approved: bool,
    ) -> Result<Gated<pb::StartSessionResponse>, Status> {
        Self::refuse_peer(caller)?;
        let from_id = request
            .session
            .as_ref()
            .map(|session| session.value.clone())
            .filter(|session| !session.is_empty())
            .ok_or_else(|| status::missing("session"))?;
        let source = self.load(&from_id).await?;
        // Checked here wherever the session runs; one on another machine
        // goes there once the channel is known.
        let (store, id) = (self.starter.store.clone(), from_id.clone());
        let archived = blocking(move || store.archive(&id))
            .await?
            .map_err(|error| status::store(error, &self.machine, "look up the session"))?;
        if archived.is_some() {
            return Err(failed(
                Code::FailedPrecondition,
                &ContinueError::SourceArchived { session: from_id },
                "restore the session first, then continue it in another channel",
                None,
            ));
        }
        let channel_id = request
            .channel
            .as_ref()
            .map(|channel| channel.value.clone())
            .filter(|channel| !channel.is_empty())
            .ok_or_else(|| status::missing("channel"))?;
        if channel_id == DESK_CHANNEL {
            return Err(status::invalid_field(
                "channel",
                Problem::NotAllowedHere,
                "the Desk isn't a channel a session can be continued in",
                "pick one of your channels",
            ));
        }
        if channel_id == source.record.channel {
            return Err(status::invalid_field(
                "channel",
                Problem::NotAllowedHere,
                "the session is already in that channel",
                "pick another channel, or fork the session to go on in this one",
            ));
        }
        let channel = self.find_channel(&channel_id).await?;
        // A session on another machine continues there, where its
        // conversation is; that machine checks its agent and conversation.
        if remote::elsewhere(&source.record, &self.machine) {
            return self.continue_there(caller, &source.record, &channel, request, approved).await;
        }
        let agent = source.record.agent.clone();
        if source
            .agent
            .as_ref()
            .is_none_or(|agent| agent.launch.conversation_file.as_os_str().is_empty())
        {
            return Err(failed(
                Code::FailedPrecondition,
                &ContinueError::NoConversation { session: from_id },
                "wait until the session's agent has started, then continue it",
                None,
            ));
        }
        for gated in [&source.record.channel, &channel.id] {
            if let Some(asked) = self.gate(
                caller,
                Capability::SessionStartSameMachine,
                Target::Start {
                    channel: ChannelId::new(gated.as_str()),
                    machine: MachineId::new(self.machine.as_str()),
                    agent: AgentId::new(agent.as_str()),
                    fork_of: Some(humata2_access::SessionId::new(from_id.as_str())),
                },
                approved,
            )? {
                return Ok(Gated::Ask(asked));
            }
        }
        // The conversation is read through its agent's adapter, as the copy
        // reads it, so one it doesn't support (CV-3) is refused here, from
        // now too, before anything changes.
        let before_turn = request.before_turn;
        let turns = match self.session_turns(&source).await? {
            Ok(turns) => turns,
            Err(error @ TurnsError::Unsupported { .. }) => {
                return Err(failed(
                    Code::FailedPrecondition,
                    &ContinueError::CantCarry {
                        agent: agent.clone(),
                        source: error,
                    },
                    "start a new session in the other channel, or fork this one in its own channel; nothing was changed",
                    Some(pb::error::Kind::AgentLacksFeature(pb::AgentLacksFeature {
                        agent: Some(pb::AgentId { value: agent.clone() }),
                        feature: pb::agent_lacks_feature::Feature::ConversationMove as i32,
                    })),
                ));
            }
            Err(error) => {
                return Err(failed(
                    Code::Unavailable,
                    &error,
                    "check that the session's conversation is still there and readable by this user, then continue it again",
                    None,
                ));
            }
        };
        if before_turn != 0 {
            let Some(turns) = turns else {
                return Err(failed(
                    Code::FailedPrecondition,
                    &ContinueError::NoTurns { agent },
                    "continue from now, with before_turn 0",
                    None,
                ));
            };
            if before_turn == 1 {
                return Err(status::invalid_field(
                    "before_turn",
                    Problem::NotAllowedHere,
                    "turn 1 is the first prompt, so there is no conversation before it to carry",
                    "continue from now with before_turn 0, or start a new session in the other channel",
                ));
            }
            if !turns.iter().any(|turn| turn.index == before_turn) {
                return Err(status::invalid_field(
                    "before_turn",
                    Problem::UnknownValue,
                    format!(
                        "the session has {} turns, so there is no turn {before_turn} to continue before",
                        turns.len()
                    ),
                    "pick a turn the session has, or continue from now with before_turn 0",
                ));
            }
        }
        if let Some(operation) = self
            .operations
            .existing(caller, request.idempotency_key.as_ref())?
        {
            return Ok(Gated::Done(self.started_by(operation).await?));
        }
        turns::refresh_hooks().await;

        let mut keys = vec!["write_record".to_owned()];
        let mut steps = vec![("write_record".to_owned(), "Saving the session".to_owned())];
        for repo in channel.repos.iter().filter(|repo| repo.main == MainClone::Ready) {
            keys.push(format!("clone:{}", repo.name));
            steps.push((format!("clone:{}", repo.name), format!("Cloning {}", repo.name)));
        }
        keys.push("copy_conversation".to_owned());
        keys.push("launch".to_owned());
        keys.push("verify".to_owned());
        steps.push(("copy_conversation".to_owned(), "Copying the conversation".to_owned()));
        steps.push(("launch".to_owned(), "Starting the agent".to_owned()));
        steps.push(("verify".to_owned(), "Checking it started".to_owned()));
        let target = pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Channel(pb::ChannelId {
                value: channel.id.clone(),
            })),
        };
        let progress = match self.operations.start_steps(
            caller,
            pb::OperationKind::ContinueInChannel,
            target,
            &steps,
            false,
            request.idempotency_key,
        )? {
            Started::New(progress) => progress,
            Started::Existing(operation) => {
                return Ok(Gated::Done(self.started_by(operation).await?));
            }
        };
        let operation = progress.operation();
        let continued = ContinueRequest {
            id: None,
            from: from_id,
            channel: channel.id,
            machine: self.machine.clone(),
            before_turn,
            operation: operation.id.as_ref().map(|id| id.value.clone()),
            cols: COLS,
            rows: ROWS,
        };
        let (ready, recorded) = oneshot::channel();
        let by = identity::actor(caller);
        tokio::spawn(run_continue(
            self.starter.clone(),
            self.tell.clone(),
            self.tokens.clone(),
            continued,
            progress,
            keys,
            ready,
            by.clone(),
        ));
        match recorded.await {
            Ok(Ok(())) => {
                let started = self.started_by(operation).await?;
                if let Some(session) = &started.session {
                    self.tell
                        .session(&session.value, pb::Change::Created, Some(by))
                        .await;
                }
                Ok(Gated::Done(started))
            }
            Ok(Err(refused)) => Err(refused),
            Err(_) => Err(status::internal(
                "continuing the session in another channel stopped before the new session was recorded",
                "",
            )),
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_continue(
    starter: Starter,
    tell: Tell,
    tokens: Arc<Tokens>,
    request: ContinueRequest,
    progress: Progress,
    keys: Vec<String>,
    ready: oneshot::Sender<Result<(), Status>>,
    by: pb::Actor,
) {
    let mut ready = Some(ready);
    let operation = request.operation.clone();
    let mut current = "write_record".to_owned();
    progress.set_step("write_record", pb::StepState::Running, None);
    let env = |tab: &TabRecord| tokens.env(tab);
    let continued = {
        let mut note = |event: &str| {
            current = advance(&progress, &keys, event);
            if event == "write_record"
                && let Some(ready) = ready.take()
            {
                let _ = ready.send(Ok(()));
            }
            crate::fault::point(&format!("continue.{event}"));
        };
        starter.continue_in(request, &env, &mut note).await
    };
    match continued {
        Ok(started) => {
            progress.set_step("verify", pb::StepState::Done, None);
            let session = started.session.id;
            progress.succeed(Some(pb::operation::Result::Session(pb::SessionId {
                value: session.clone(),
            })));
            tell.session(&session, pb::Change::Updated, Some(by)).await;
            watch::agent(tell, session);
        }
        Err(error) => {
            let (code, failure) = start_error(&error);
            progress.set_step(&current, pb::StepState::Failed, Some(status::chain(&error)));
            if let Some(ready) = ready.take() {
                let _ = ready.send(Err(status::failure(code, failure.clone())));
            }
            progress.fail(failure);
            if let Some(operation) = operation
                && let Some(session) = tell.session_of(&operation).await
            {
                tell.session(&session, pb::Change::Updated, Some(by)).await;
            }
        }
    }
}
