use super::FakeDaemon;
use crate::daemon::{AgentSwitch, Operation, Reply, SwitchChange};
use crate::model::Agent;

/// Every built-in agent on, installing off, install possible.
pub(super) fn defaults() -> Vec<AgentSwitch> {
    Agent::ALL
        .into_iter()
        .filter(|agent| *agent != Agent::Shell)
        .map(|agent| AgentSwitch {
            agent,
            enabled: true,
            install_when_missing: false,
            can_install: true,
            install_command: Some(install_command(agent).to_owned()),
        })
        .collect()
}

fn install_command(agent: Agent) -> &'static str {
    match agent {
        Agent::Claude => "curl -fsSL https://claude.ai/install.sh | bash",
        Agent::Codex => "npm install -g @openai/codex",
        Agent::Copilot => "npm install -g @github/copilot",
        Agent::OpenCode => "curl -fsSL https://opencode.ai/install | bash",
        Agent::Shell => "",
    }
}

pub(super) fn agent_switches(daemon: &FakeDaemon) -> Reply<Vec<AgentSwitch>> {
    daemon.reply(None, "agent_switches".to_owned(), |inner| {
        Ok(inner.switches.clone())
    })
}

pub(super) fn set_agent_switch(
    daemon: &FakeDaemon,
    agent: Agent,
    change: SwitchChange,
) -> Reply<AgentSwitch> {
    daemon.reply(
        Some(Operation::SetAgentSwitch),
        format!("set_agent_switch {} {change:?}", agent.command()),
        move |inner| {
            let Some(switch) = inner.switches.iter_mut().find(|s| s.agent == agent) else {
                return Err(crate::daemon::DaemonError::Scripted {
                    operation: Operation::SetAgentSwitch,
                    message: format!("{} has no switch", agent.name()),
                });
            };
            if let Some(enabled) = change.enabled {
                switch.enabled = enabled;
            }
            if let Some(install) = change.install_when_missing {
                switch.install_when_missing = install;
            }
            Ok(switch.clone())
        },
    )
}
