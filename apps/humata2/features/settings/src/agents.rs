//! Settings › Agents › Agents (AGENTSET, D725): which agents Humata offers,
//! and whether it may install each one's CLI on a machine without it.

use humata2_clients::model::Agent;
use humata2_clients::{AgentSwitch, Clients, DaemonError, SwitchChange};
use tca::Effect;

/// The agents' switches as the daemon last said.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentsState {
    /// Every built-in agent except the shell, in the daemon's order; `None`
    /// until the daemon has said.
    pub switches: Option<Vec<AgentSwitch>>,
}

impl AgentsState {
    /// The built-in agents pickers list, in menu order: the enabled ones, or
    /// all of them until the daemon has said. Never the shell.
    #[must_use]
    pub fn offered(&self) -> Vec<Agent> {
        Agent::ALL
            .into_iter()
            .filter(|agent| *agent != Agent::Shell)
            .filter(|agent| match &self.switches {
                None => true,
                Some(switches) => switches
                    .iter()
                    .any(|switch| switch.agent == *agent && switch.enabled),
            })
            .collect()
    }
}

/// What happens on the Agents card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentsAction {
    /// Read the switches from the daemon.
    Load,
    /// The daemon gave the switches, or couldn't.
    Loaded(Result<Vec<AgentSwitch>, DaemonError>),
    /// Turn an agent on or off.
    Toggle(Agent),
    /// Turn "Install when missing" on or off for an agent.
    ToggleInstall(Agent),
    /// The daemon saved an agent's switches, or refused.
    Saved {
        /// The agent changed.
        agent: Agent,
        /// What its switches were before the change, to put back on a refusal.
        before: AgentSwitch,
        /// What the daemon holds now, or why it refused.
        result: Result<AgentSwitch, DaemonError>,
    },
}

/// Applies `action`, calling the daemon through `clients`. `error` is the
/// settings page's error line.
pub(crate) fn reduce(
    clients: &Clients,
    state: &mut AgentsState,
    error: &mut Option<String>,
    action: AgentsAction,
) -> Effect<AgentsAction> {
    match action {
        AgentsAction::Load => {
            let daemon = clients.daemon.clone();
            Effect::future(async move { AgentsAction::Loaded(daemon.agent_switches().await) })
        }
        AgentsAction::Loaded(result) => {
            match result {
                Ok(switches) => state.switches = Some(switches),
                Err(failure) => *error = Some(failure.to_string()),
            }
            Effect::none()
        }
        AgentsAction::Toggle(agent) => change(clients, state, agent, |switch| SwitchChange {
            enabled: Some(!switch.enabled),
            install_when_missing: None,
        }),
        AgentsAction::ToggleInstall(agent) => {
            change(clients, state, agent, |switch| SwitchChange {
                enabled: None,
                install_when_missing: Some(!switch.install_when_missing),
            })
        }
        AgentsAction::Saved {
            agent,
            before,
            result,
        } => {
            let slot = state
                .switches
                .as_mut()
                .and_then(|all| all.iter_mut().find(|switch| switch.agent == agent));
            match result {
                Ok(saved) => {
                    if let Some(slot) = slot {
                        *slot = saved;
                    }
                }
                Err(failure) => {
                    if let Some(slot) = slot {
                        *slot = before;
                    }
                    *error = Some(failure.to_string());
                }
            }
            Effect::none()
        }
    }
}

/// Shows the change at once and asks the daemon to save it.
fn change(
    clients: &Clients,
    state: &mut AgentsState,
    agent: Agent,
    wanted: impl Fn(&AgentSwitch) -> SwitchChange,
) -> Effect<AgentsAction> {
    let Some(slot) = state
        .switches
        .as_mut()
        .and_then(|all| all.iter_mut().find(|switch| switch.agent == agent))
    else {
        return Effect::none();
    };
    let before = slot.clone();
    let change = wanted(&before);
    if let Some(enabled) = change.enabled {
        slot.enabled = enabled;
    }
    if let Some(install) = change.install_when_missing {
        slot.install_when_missing = install;
    }
    let daemon = clients.daemon.clone();
    Effect::future(async move {
        let result = daemon.set_agent_switch(agent, change).await;
        AgentsAction::Saved {
            agent,
            before,
            result,
        }
    })
}
