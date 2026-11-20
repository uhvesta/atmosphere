//! Which agents Humata offers (AGENTSET, D720): one switch per agent kind,
//! built-in or custom, except the plain shell; whether Humata may install its CLI on a machine without it, and
//! whether its permission and question asks wait in the Inbox (D782).

use crate::model::Agent;

/// One agent's switches as the daemon holds them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentSwitch {
    /// The kind's id in `agents.toml` (`claude`, or a custom kind's own).
    pub kind: String,
    /// The kind's name, as the daemon gives it.
    pub name: String,
    /// The built-in agent it is; `None` for a custom kind, which pickers
    /// don't offer.
    pub agent: Option<Agent>,
    /// Offered in pickers and startable.
    pub enabled: bool,
    /// Humata may install the agent's CLI on a machine that doesn't have it.
    pub install_when_missing: bool,
    /// The agent has an install command, so "install when missing" can be on.
    pub can_install: bool,
    /// The command Humata runs to install it, as shown in Settings; `None`
    /// when it has none.
    pub install_command: Option<String>,
    /// Its permission and question asks wait in the Inbox; off, it asks in
    /// its own terminal (D782).
    pub asks_in_inbox: bool,
}

/// What to change on one agent's switches; `None` leaves it as it is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SwitchChange {
    pub enabled: Option<bool>,
    pub install_when_missing: Option<bool>,
    pub asks_in_inbox: Option<bool>,
}

/// Every agent is off in Settings › Agents, so nothing can be picked.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("Every agent is off. Turn one on in Settings › Agents, then try again.")]
pub struct NoAgentOn;

/// The agent a default takes: the first of `preferred` that is the plain
/// shell or on (in `offered`, which never holds the shell); else the first
/// agent that is on.
///
/// # Errors
/// [`NoAgentOn`] when none of `preferred` is usable and nothing is offered.
pub fn pick_agent(offered: &[Agent], preferred: &[Agent]) -> Result<Agent, NoAgentOn> {
    preferred
        .iter()
        .copied()
        .find(|agent| *agent == Agent::Shell || offered.contains(agent))
        .or_else(|| offered.first().copied())
        .ok_or(NoAgentOn)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ON: [Agent; 3] = [Agent::Copilot, Agent::OpenCode, Agent::Agy];

    #[test]
    fn an_off_preference_falls_to_the_first_agent_on() {
        assert_eq!(pick_agent(&ON, &[Agent::Claude]), Ok(Agent::Copilot));
    }

    #[test]
    fn the_next_preference_that_is_on_wins() {
        assert_eq!(pick_agent(&ON, &[Agent::Claude, Agent::OpenCode]), Ok(Agent::OpenCode));
    }

    #[test]
    fn nothing_on_is_an_error() {
        assert_eq!(pick_agent(&[], &[Agent::Claude]), Err(NoAgentOn));
    }

    #[test]
    fn the_shell_is_always_available() {
        assert_eq!(pick_agent(&[], &[Agent::Shell]), Ok(Agent::Shell));
    }
}
