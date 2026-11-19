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

pub mod there;

use humata2_access::{AgentId, Call, Capability, ChannelId, MachineId, Target};
use humata2_daemon_ports::Caller;
use humata2_rpc_kit::{Gated, blocking, failed, invalid_field, missing, not_found};
use humata2_sessions::{DESK_CHANNEL, TurnsError};
use humata2_svc_sessions_common::pb;
use pb::invalid_field::Problem;
use tonic::{Code, Status};

use crate::SessionTransferSvc;
use crate::ports::Checked;

#[derive(Debug, thiserror::Error)]
pub enum ContinueError {
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

impl SessionTransferSvc {
    pub async fn handle_continue_in_channel(
        &self,
        caller: &Caller,
        request: pb::ContinueInChannelRequest,
        approved: bool,
    ) -> Result<Gated<pb::StartSessionResponse>, Status> {
        let from_id = request
            .session
            .as_ref()
            .map(|session| session.value.clone())
            .filter(|session| !session.is_empty())
            .ok_or_else(|| missing("session"))?;
        let source = self.env.load_session(&self.core, &from_id)?;
        let (store, id) = (self.core.store.clone(), from_id.clone());
        let archived = blocking(move || store.archive(&id))
            .await?
            .map_err(|error| failed(Code::Internal, &error, "look up the session", None))?;
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
            .ok_or_else(|| missing("channel"))?;
        if channel_id == DESK_CHANNEL {
            return Err(invalid_field(
                "channel",
                Problem::NotAllowedHere,
                "the Desk isn't a channel a session can be continued in",
                "pick one of your channels",
            ));
        }
        if channel_id == source.record.channel {
            return Err(invalid_field(
                "channel",
                Problem::NotAllowedHere,
                "the session is already in that channel",
                "pick another channel, or fork the session to go on in this one",
            ));
        }
        let (store, ch_id) = (self.core.store.clone(), channel_id.clone());
        let channel = blocking(move || store.channel(&ch_id))
            .await?
            .map_err(|error| failed(Code::Internal, &error, "look up the channel", None))?
            .ok_or_else(|| {
                not_found(
                    pb::EntityKind::Channel,
                    &channel_id,
                    "channel was not found",
                    "check the channel id and try again",
                )
            })?;
        // A session on another machine continues there, where its
        // conversation is; that machine checks its agent and conversation.
        if !source.record.machine.is_empty() && !self.env.is_host(&source.record.machine) {
            return self
                .handle_continue_there(caller, &source.record, &channel, request, approved)
                .await;
        }
        let agent = source.record.agent.clone();
        if !approved {
            for gated in [&source.record.channel, &channel.id] {
                if let Checked::Ask(asked) = self.env.check_capability(
                    caller,
                    Capability::SessionStartSameMachine,
                    Target::Start {
                        channel: ChannelId::new(gated.as_str()),
                        machine: MachineId::new(self.core.machine.as_str()),
                        agent: AgentId::new(agent.as_str()),
                        fork_of: Some(humata2_access::SessionId::new(from_id.as_str())),
                    },
                )? {
                    return Ok(Gated::Ask(Box::new(asked)));
                }
            }
        }
        if request.before_turn == 1 {
            return Err(invalid_field(
                "before_turn",
                Problem::NotAllowedHere,
                "turn 1 is the first prompt, so there is no conversation before it to carry",
                "continue from now with before_turn 0, or start a new session in the other channel",
            ));
        }
        Err(self.env.not_wired("SessionService.ContinueInChannel"))
    }
}
