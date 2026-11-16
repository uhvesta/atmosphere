//! ListTurns: the turns of a session's conversation, the places a fork before
//! a turn can be cut at (FK-3, D181).

use humata2_access::Call;
use humata2_grpc::humata::v1 as pb;
use humata2_mirror::{TurnMark, turns_of};
use tonic::{Code, Status};

use super::SessionSvc;
use crate::channel_service::{Gated, blocking, failed, gate};
use crate::history_service::rfc3339;
use crate::identity::Caller;
use crate::status;

impl SessionSvc {
    /// Every turn of the session, in order; nothing is paged. An agent
    /// whose conversation format can't be read has none. A session's own
    /// agent reads its own turns freely; anyone else is held to the rule
    /// that reads the session (as GetSession is), and a rule that asks
    /// puts the call in the Inbox until the user answers (`approved`).
    pub(super) async fn list_turns_of(
        &self,
        caller: &Caller,
        request: pb::ListTurnsRequest,
        approved: bool,
    ) -> Result<Gated<pb::ListTurnsResponse>, Status> {
        Self::refuse_peer(caller)?;
        let id = request
            .session
            .map(|session| session.value)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| status::missing("session"))?;
        let loaded = self.load(&id).await?;
        let own = matches!(caller, Caller::Agent(agent) if agent.session.id.as_str() == id);
        if !own {
            let call = Call::ReadSession(self.session_ref(&loaded.record));
            if let Some(asked) = gate(&self.guard, caller, &call, approved)? {
                return Ok(Gated::Ask(asked));
            }
        }
        let reads = self
            .starter
            .agents
            .get(&loaded.record.agent)
            .is_ok_and(|kind| kind.conversation_format.reads_turns());
        let Some(record) = loaded.agent.as_ref().filter(|_| reads) else {
            return Ok(Gated::Done(pb::ListTurnsResponse {
                turns: Vec::new(),
                page: None,
            }));
        };
        let path = record.launch.conversation_file.clone();
        let marks = blocking(move || turns_of(&path))
            .await?
            .map_err(|error| {
                failed(
                    Code::Unavailable,
                    &error,
                    "check that the session's conversation file is still there and readable by this user, then list the turns again",
                    None,
                )
            })?;
        Ok(Gated::Done(pb::ListTurnsResponse {
            turns: marks.into_iter().map(turn_message).collect(),
            page: None,
        }))
    }
}

fn turn_message(mark: TurnMark) -> pb::Turn {
    pb::Turn {
        index: mark.index,
        user_text: mark.first_line,
        at: mark.at.as_deref().and_then(rfc3339::parse),
    }
}
