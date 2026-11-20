//! ListTurns: the turns of a session's conversation, the places a fork before
//! a turn can be cut at (FK-3, D181).

use humata2_access::Call;
use humata2_grpc::humata::v1 as pb;
use humata2_sessions::TurnsError;
use tonic::{Code, Status};

use super::{Loaded, SessionSvc};
use crate::channel_service::{Gated, blocking, failed, gate};
use crate::history_service::rfc3339;
use crate::identity::Caller;
use crate::status;

/// A session's turns, as `SessionSvc::session_turns` reads them.
pub(super) type SessionTurns = Result<Option<Vec<pb::Turn>>, TurnsError>;

impl SessionSvc {
    /// Every turn of the session, in order; nothing is paged. An agent
    /// whose conversation format can't be read has none, nor has a
    /// conversation its adapter doesn't support (CV-3). A session's own
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
        let turns = match self.session_turns(&loaded).await? {
            Ok(turns) => turns.unwrap_or_default(),
            Err(TurnsError::Unsupported { .. }) => Vec::new(),
            Err(error) => {
                return Err(failed(
                    Code::Unavailable,
                    &error,
                    "check that the session's conversation is still there and readable by this user, then list the turns again",
                    None,
                ));
            }
        };
        Ok(Gated::Done(pb::ListTurnsResponse { turns, page: None }))
    }

    /// The turns of the session's conversation, read through its agent's
    /// conversation adapter (`AgentKind::turns`), the way a fork or a
    /// continue cuts it. None when its agent's turns can't be read; no turns
    /// when it has no agent record yet.
    pub(super) async fn session_turns(
        &self,
        loaded: &Loaded,
    ) -> Result<SessionTurns, Status> {
        let Ok(kind) = self.starter.agents.get_as_ran(&loaded.record.id, &loaded.record.agent) else {
            return Ok(Ok(None));
        };
        let Some(record) = loaded.agent.as_ref() else {
            return Ok(Ok(Some(Vec::new())));
        };
        let home = self.starter.home.clone();
        let folder = loaded.record.folder.clone();
        let id = record.launch.conversation_id.clone();
        let file = record.launch.conversation_file.clone();
        let version = record.launch.cli_version.clone();
        let read =
            blocking(move || kind.turns(&home, &folder, &id, &file, version.as_deref())).await?;
        Ok(read.map(|turns| {
            turns.map(|turns| {
                turns
                    .into_iter()
                    .map(|turn| pb::Turn {
                        index: turn.index,
                        user_text: turn.first_line,
                        at: turn.at.as_deref().and_then(rfc3339::parse),
                    })
                    .collect()
            })
        }))
    }
}
