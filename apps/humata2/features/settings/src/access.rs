//! The channel Agent access tab (IN-17, IN-19, IN-21, IN-23): change a rule's
//! setting, remove a grant. The rules themselves live in the workspace; the
//! fake or live daemon reports each change as an event the app folds in, so
//! this reducer only tracks what is in flight and what went wrong.

use humata2_clients::model::{Capability, ChannelId, GrantId, RuleSetting};
use humata2_clients::{Clients, DaemonError};
use std::collections::BTreeSet;
use tca::Effect;

/// What the Agent access tab is waiting on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AccessState {
    /// Rules whose new setting the daemon hasn't confirmed yet.
    pub pending: BTreeSet<(ChannelId, Capability)>,
    /// Grants being removed.
    pub removing: BTreeSet<GrantId>,
    /// What went wrong last, until dismissed.
    pub error: Option<String>,
}

/// What happens on the Agent access tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccessAction {
    /// Set a rule. Ignored while the same rule's last change is pending.
    Set {
        /// The channel the rule belongs to.
        channel: ChannelId,
        /// What the rule covers.
        capability: Capability,
        /// The new setting.
        setting: RuleSetting,
    },
    /// The daemon took a rule change, or didn't.
    Done {
        /// The channel the rule belongs to.
        channel: ChannelId,
        /// What the rule covers.
        capability: Capability,
        /// How it went.
        result: Result<(), DaemonError>,
    },
    /// Remove a grant.
    RemoveGrant(GrantId),
    /// The daemon removed a grant, or didn't.
    GrantRemoved {
        /// The grant.
        grant: GrantId,
        /// How it went.
        result: Result<(), DaemonError>,
    },
    /// Hide the error.
    DismissError,
}

/// Applies `action`, calling the daemon through `clients`.
pub(crate) fn reduce(
    clients: &Clients,
    state: &mut AccessState,
    action: AccessAction,
) -> Effect<AccessAction> {
    match action {
        AccessAction::Set {
            channel,
            capability,
            setting,
        } => {
            if !state.pending.insert((channel.clone(), capability.clone())) {
                return Effect::none();
            }
            let daemon = clients.daemon.clone();
            Effect::future(async move {
                let result = daemon.set_rule(&channel, &capability, setting).await;
                AccessAction::Done {
                    channel,
                    capability,
                    result,
                }
            })
        }
        AccessAction::Done {
            channel,
            capability,
            result,
        } => {
            state.pending.remove(&(channel, capability));
            if let Err(error) = result {
                state.error = Some(error.to_string());
            }
            Effect::none()
        }
        AccessAction::RemoveGrant(grant) => {
            if !state.removing.insert(grant.clone()) {
                return Effect::none();
            }
            let daemon = clients.daemon.clone();
            Effect::future(async move {
                let result = daemon.remove_grant(&grant).await;
                AccessAction::GrantRemoved { grant, result }
            })
        }
        AccessAction::GrantRemoved { grant, result } => {
            state.removing.remove(&grant);
            if let Err(error) = result {
                state.error = Some(error.to_string());
            }
            Effect::none()
        }
        AccessAction::DismissError => {
            state.error = None;
            Effect::none()
        }
    }
}
