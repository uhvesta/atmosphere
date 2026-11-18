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
