//! Which agents Humata offers (AGENTSET, D720), over the daemon's
//! AgentService.

use humata2_clients::model::Agent;
use humata2_clients::{AgentSwitch, DaemonError, SwitchChange};
use humata2_grpc::humata::v1 as pb;

use crate::connection::Shared;

/// The app's switch for a daemon definition; `None` for the shell and for
/// custom kinds, which the app's agent model has no place for.
fn switch_of(definition: &pb::AgentDefinition) -> Option<AgentSwitch> {
    let id = definition.id.as_ref()?.value.as_str();
    let agent = Agent::ALL
        .into_iter()
        .find(|agent| *agent != Agent::Shell && agent.command() == id)?;
    Some(AgentSwitch {
        agent,
        enabled: definition.enabled,
        install_when_missing: definition.install_when_missing,
        can_install: definition.can_install,
        install_command: Some(definition.install_command.clone()).filter(|c| !c.is_empty()),
    })
}

impl Shared {
    /// Every built-in agent except the shell, enabled or not, in the
    /// daemon's order.
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
        agent: Agent,
        change: SwitchChange,
    ) -> Result<AgentSwitch, DaemonError> {
        const WHAT: &str = "Changing an agent's switches";
        let saved = self
            .agents()
            .set_agent_enabled(pb::SetAgentEnabledRequest {
                agent: Some(pb::AgentId {
                    value: agent.command().to_owned(),
                }),
                enabled: change.enabled,
                install_when_missing: change.install_when_missing,
            })
            .await
            .map_err(|status| self.error(&status, WHAT))?
            .into_inner();
        switch_of(&saved).ok_or_else(|| DaemonError::NotFound {
            what: agent.name().to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn definition(id: &str, enabled: bool) -> pb::AgentDefinition {
        pb::AgentDefinition {
            id: Some(pb::AgentId {
                value: id.to_owned(),
            }),
            enabled,
            ..Default::default()
        }
    }

    #[test]
    fn built_in_kinds_become_switches_and_the_shell_and_custom_kinds_do_not() {
        assert_eq!(
            switch_of(&definition("codex", false)).map(|s| (s.agent, s.enabled)),
            Some((Agent::Codex, false))
        );
        assert_eq!(switch_of(&definition("zsh", true)), None);
        assert_eq!(switch_of(&definition("my-agent", true)), None);
    }
}
