//! The agent settings the daemon's SettingsService keeps: the default agent
//! and whether Codex's hooks need trusting. Where each agent asks is its
//! switch (D782, `agent_switches`).

use humata2_clients::DaemonError;
use humata2_clients::model::Agent;
use humata2_grpc::humata::v1 as pb;

use crate::connection::{Shared, fresh_key};

impl Shared {

    /// Whether Codex's hooks still need trusting in its /hooks (D202). A
    /// daemon that sends no agent settings has nothing to trust.
    pub(crate) async fn codex_needs_trust(&self) -> Result<bool, DaemonError> {
        const WHAT: &str = "Reading whether Codex's hooks need trusting";
        let shown = self
            .settings()
            .get_settings(pb::GetSettingsRequest {})
            .await
            .map_err(|status| self.error(&status, WHAT))?
            .into_inner();
        Ok(shown
            .agents
            .is_some_and(|agents| agents.codex_hooks_need_trust))
    }

    /// Makes `agent` Settings' default agent (ST-15).
    pub(crate) async fn change_default_agent(&self, agent: Agent) -> Result<(), DaemonError> {
        const WHAT: &str = "Changing the default agent";
        self.settings()
            .update_settings(pb::UpdateSettingsRequest {
                idempotency_key: fresh_key(),
                default_agent: Some(pb::DefaultAgent {
                    agent: Some(pb::AgentId { value: self.agents.name(agent) }),
                    model: String::new(),
                }),
                ..Default::default()
            })
            .await
            .map_err(|status| self.error(&status, WHAT))?;
        Ok(())
    }

}
