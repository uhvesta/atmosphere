//! Settings › Agents › Agents (AGENTSET, D725): which agents Humata offers,
//! whether it may install each one's CLI on a machine without it, and
//! whether each one's asks wait in the Inbox (D782).

use humata2_clients::model::Agent;
use humata2_clients::{AgentSwitch, Clients, DaemonError, SwitchChange};
use tca::Effect;

/// The agents' switches as the daemon last said.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentsState {
    /// Every agent kind except the shell, built-in or custom, in the
    /// daemon's order; `None` until the daemon has said.
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
                    .any(|switch| switch.agent == Some(*agent) && switch.enabled),
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
    /// Turn an agent kind on or off, by its id.
    Toggle(String),
    /// Turn "Install when missing" on or off for an agent kind.
    ToggleInstall(String),
    /// Turn "Ask in the Inbox" on or off for an agent kind (D782).
    ToggleAsks(String),
    /// The daemon saved an agent kind's switches, or refused.
    Saved {
        /// The id of the kind changed.
        kind: String,
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
        AgentsAction::Toggle(kind) => change(clients, state, kind, |switch| SwitchChange {
            enabled: Some(!switch.enabled),
            ..SwitchChange::default()
        }),
        AgentsAction::ToggleInstall(kind) => {
            change(clients, state, kind, |switch| SwitchChange {
                install_when_missing: Some(!switch.install_when_missing),
                ..SwitchChange::default()
            })
        }
        AgentsAction::ToggleAsks(kind) => change(clients, state, kind, |switch| SwitchChange {
            asks_in_inbox: Some(!switch.asks_in_inbox),
            ..SwitchChange::default()
        }),
        AgentsAction::Saved {
            kind,
            before,
            result,
        } => {
            let slot = state
                .switches
                .as_mut()
                .and_then(|all| all.iter_mut().find(|switch| switch.kind == kind));
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
    kind: String,
    wanted: impl Fn(&AgentSwitch) -> SwitchChange,
) -> Effect<AgentsAction> {
    let Some(slot) = state
        .switches
        .as_mut()
        .and_then(|all| all.iter_mut().find(|switch| switch.kind == kind))
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
    if let Some(inbox) = change.asks_in_inbox {
        slot.asks_in_inbox = inbox;
    }
    let daemon = clients.daemon.clone();
    Effect::future(async move {
        let result = daemon.set_agent_switch(kind.clone(), change).await;
        AgentsAction::Saved {
            kind,
            before,
            result,
        }
    })
}
