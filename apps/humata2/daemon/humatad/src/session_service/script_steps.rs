//! What a machine script's `start` and `send` steps need of the session
//! service (D682): starting a session of an agent kind in a named channel
//! through the same start every client uses, and reading a session's turn
//! state to wait for it to be idle. The rules are the owner's own
//! automation, so nothing here asks the Inbox.

use std::sync::Arc;

use humata2_grpc::humata::v1 as pb;
use tonic::Status;

use super::{Gated, SessionSvc, blocking, no_channel, turns};
use crate::identity::{Caller, UserVia};
use crate::machine_scripts::MachineScripts;
use crate::status;

/// A session as a `send` step's wait sees it.
pub(crate) struct ScriptView {
    pub(crate) status: pb::SessionStatus,
    pub(crate) turn_end: pb::TurnEndSource,
    pub(crate) agent: String,
}

impl SessionSvc {
    /// Gives the move the machine scripts engine, for its session
    /// triggers. Once.
    pub(crate) fn use_machine_scripts(&self, scripts: Arc<MachineScripts>) {
        let _ = self.scripts.set(scripts);
    }

    /// The machine scripts engine, once `serve` has set it.
    pub(super) fn machine_scripts(&self) -> Option<Arc<MachineScripts>> {
        self.scripts.get().cloned()
    }

    /// Starts a session of `agent` in the channel named `channel` (its name
    /// or its id) on `machine`, once for `key`, and answers its id. The
    /// session goes on starting in the background.
    pub(crate) async fn start_for_script(
        &self,
        machine: &str,
        agent: &str,
        channel: &str,
        title: &str,
        key: &str,
    ) -> Result<String, Status> {
        let caller = Caller::User(UserVia::Host);
        crate::stop::gate::require_running(&caller)?;
        let channel = self.script_channel(channel).await?;
        let elsewhere = (machine != self.machine).then(|| pb::MachineId { value: machine.to_owned() });
        if let Some(named) = &elsewhere {
            crate::upgrade::gate::machine_ready(&self.migrations, named)?;
        }
        let request = pb::StartSessionRequest {
            idempotency_key: Some(pb::IdempotencyKey { value: key.to_owned() }),
            channel: Some(pb::ChannelId { value: channel }),
            title: title.to_owned(),
            agent: Some(pb::AgentId { value: agent.to_owned() }),
            machine: elsewhere,
            ..pb::StartSessionRequest::default()
        };
        match self.begin_session(&caller, request, None, true).await? {
            Gated::Done(started) => started
                .session
                .map(|session| session.value)
                .filter(|session| !session.is_empty())
                .ok_or_else(|| status::internal("a start for a machine script answered no session", "")),
            Gated::Ask(_) => Err(status::internal(
                "a start for a machine script was sent to Ask me, which an approved start never is",
                "",
            )),
        }
    }

    /// The id of the channel a rule names, by id or by name.
    async fn script_channel(&self, wanted: &str) -> Result<String, Status> {
        let store = self.starter.store.clone();
        let channels = blocking(move || store.channels())
            .await?
            .map_err(|error| status::store(error, &self.machine, "list the channels"))?;
        channels
            .iter()
            .find(|channel| channel.id == wanted)
            .or_else(|| channels.iter().find(|channel| channel.name.eq_ignore_ascii_case(wanted)))
            .map(|channel| channel.id.clone())
            .ok_or_else(|| no_channel(wanted))
    }

    /// `session`'s status and where its turn ends come from, as the host
    /// shows them, also for a session on another machine.
    pub(crate) async fn script_view(&self, session: &str) -> Result<ScriptView, Status> {
        let loaded = self.load(session).await?;
        let shown = self.show(&loaded);
        Ok(ScriptView {
            status: shown.status(),
            turn_end: turns::source_of(&loaded.record.agent),
            agent: loaded.record.agent.clone(),
        })
    }
}
