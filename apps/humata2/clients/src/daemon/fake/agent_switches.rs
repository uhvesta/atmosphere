use super::FakeDaemon;
use crate::daemon::{AgentSwitch, Operation, Reply, SwitchChange};
use crate::model::Agent;

/// Every built-in agent on, installing off, install possible, asks in the
/// Inbox.
pub(super) fn defaults() -> Vec<AgentSwitch> {
    Agent::ALL
        .into_iter()
        .filter(|agent| *agent != Agent::Shell)
        .map(|agent| AgentSwitch {
            kind: agent.command().to_owned(),
            name: agent.name().to_owned(),
            agent: Some(agent),
            enabled: true,
            install_when_missing: false,
            can_install: true,
            install_command: Some(install_command(agent).to_owned()),
            asks_in_inbox: true,
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
    kind: String,
    change: SwitchChange,
) -> Reply<AgentSwitch> {
    daemon.reply(
        Some(Operation::SetAgentSwitch),
        format!("set_agent_switch {kind} {change:?}"),
        move |inner| {
            let Some(switch) = inner.switches.iter_mut().find(|s| s.kind == kind) else {
                return Err(crate::daemon::DaemonError::Scripted {
                    operation: Operation::SetAgentSwitch,
                    message: format!("{kind} has no switch"),
                });
            };
            if let Some(enabled) = change.enabled {
                switch.enabled = enabled;
            }
            if let Some(install) = change.install_when_missing {
                switch.install_when_missing = install;
            }
            if let Some(inbox) = change.asks_in_inbox {
                switch.asks_in_inbox = inbox;
            }
            Ok(switch.clone())
        },
    )
}
