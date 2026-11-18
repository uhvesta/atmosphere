//! Which agents Humata offers (AGENTSET, D720): one switch per built-in
//! agent, and whether Humata may install its CLI on a machine without it.

use crate::model::Agent;

/// One agent's switches as the daemon holds them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentSwitch {
    pub agent: Agent,
    /// Offered in pickers and startable.
    pub enabled: bool,
    /// Humata may install the agent's CLI on a machine that doesn't have it.
    pub install_when_missing: bool,
    /// The agent has an install command, so "install when missing" can be on.
    pub can_install: bool,
    /// The command Humata runs to install it, as shown in Settings; `None`
    /// when it has none.
    pub install_command: Option<String>,
}

/// What to change on one agent's switches; `None` leaves it as it is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SwitchChange {
    pub enabled: Option<bool>,
    pub install_when_missing: Option<bool>,
}
