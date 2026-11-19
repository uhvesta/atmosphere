//! Which agents Humata offers (AGENTSET, D720), over the daemon's
//! AgentService.

use humata2_clients::model::Agent;
use humata2_clients::{AgentSwitch, DaemonError, SwitchChange};
use crate::humata2_grpc::humata::v1 as pb;

use crate::connection::Shared;

/// The app's switch for a daemon definition: every kind, built-in or custom
/// (D782), except the plain shell, which has no switches.
fn switch_of(definition: &pb::AgentDefinition) -> Option<AgentSwitch> {
    let id = definition.id.as_ref()?.value.as_str();
    if id == Agent::Shell.command() {
        return None;
    }
    let agent = Agent::ALL.into_iter().find(|agent| agent.command() == id);
    let name = if definition.name.is_empty() {
        id.to_owned()
    } else {
        definition.name.clone()
    };
    Some(AgentSwitch {
        kind: id.to_owned(),
        name,
        agent,
        enabled: definition.enabled,
        install_when_missing: definition.install_when_missing,
        can_install: definition.can_install,
        install_command: Some(definition.install_command.clone()).filter(|c| !c.is_empty()),
        asks_in_inbox: definition.asks_in_inbox,
    })
}

impl Shared {
    /// Every agent kind except the shell, enabled or not, in the daemon's
    /// order.
    pub(crate) async fn agent_switches(&self) -> Result<Vec<AgentSwitch>, DaemonError> {
        const WHAT: &str = "Reading which agents are on";
        let listed = self
            .agents()
            .list_agents(pb::ListAgentsRequest {
                include_disabled: true,
            })
            .await
            .map_err(|status| self.error(&status, WHAT))?
            .into_inner();
        Ok(listed.agents.iter().filter_map(switch_of).collect())
    }

    /// Changes one agent's switches; gives what the daemon holds after.
    pub(crate) async fn set_agent_switch(
        &self,
        kind: String,
        change: SwitchChange,
    ) -> Result<AgentSwitch, DaemonError> {
        const WHAT: &str = "Changing an agent's switches";
        let saved = self
            .agents()
            .set_agent_enabled(pb::SetAgentEnabledRequest {
                agent: Some(pb::AgentId {
                    value: kind.clone(),
                }),
                enabled: change.enabled,
                install_when_missing: change.install_when_missing,
                asks_in_inbox: change.asks_in_inbox,
            })
            .await
            .map_err(|status| self.error(&status, WHAT))?
            .into_inner();
        switch_of(&saved).ok_or(DaemonError::NotFound { what: kind })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn definition(id: &str, name: &str, enabled: bool) -> pb::AgentDefinition {
        pb::AgentDefinition {
            id: Some(pb::AgentId {
                value: id.to_owned(),
            }),
            name: name.to_owned(),
            enabled,
            asks_in_inbox: true,
            ..Default::default()
        }
    }

    #[test]
    fn built_in_kinds_become_switches_and_the_shell_does_not() {
        assert_eq!(
            switch_of(&definition("codex", "Codex", false)).map(|s| (s.agent, s.enabled)),
            Some((Some(Agent::Codex), false))
        );
        assert_eq!(switch_of(&definition("zsh", "zsh", true)), None);
    }

    #[test]
    fn a_custom_kind_gets_switches_under_its_own_id_and_name() {
        let switch = switch_of(&definition("my-agent", "My agent", true));
        assert_eq!(
            switch.map(|s| (s.kind, s.name, s.agent, s.asks_in_inbox)),
            Some(("my-agent".to_owned(), "My agent".to_owned(), None, true))
        );
    }
}
