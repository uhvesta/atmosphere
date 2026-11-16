//! Desk sessions' helpers (DK-4, DK-10, D182.3): the channel id every Desk
//! agent is filed under, and which calls its switches allow.

use crate::capability::Capability;
use crate::ids::ChannelId;
use crate::principal::SessionKind;

/// The channel id every Desk agent is filed under. No channel has this id:
/// the policy keeps no rules for it and uses the new-channel defaults.
const DESK_CHANNEL: &str = "desk";

/// True for the channel id a Desk agent is filed under.
pub(crate) fn is_desk_channel(channel: &ChannelId) -> bool {
    channel.as_str() == DESK_CHANNEL
}

/// True when a Desk switch that is on allows `capability`. The policy
/// checks Never first, so a Never rule still wins over a switch.
pub(crate) fn switch_allows(kind: &SessionKind, capability: Capability) -> bool {
    match kind {
        SessionKind::Desk(access) => match capability {
            Capability::GlobalRead | Capability::GlobalBoardWrite => access.boards,
            Capability::ChannelCreate | Capability::MachineProvision => {
                access.channels_and_machines
            }
            _ => false,
        },
        SessionKind::Ordinary
        | SessionKind::ContextOnlyFork { .. }
        | SessionKind::Review { .. } => false,
    }
}
