//! Channel Desk agent tab strip rules (DESKTABS, MULTIAGENT):
//! flat total cap (`enabled_kinds + 2`) and max 2 open tabs per agent kind
//! (including plain `Agent::Shell` / Terminal).

use humata2_clients::model::Agent;

/// Maximum open Desk tabs of the same agent kind per channel.
pub const MAX_TABS_PER_KIND: usize = 2;

/// Flat cap on total open Desk tabs for a channel: `enabled_kinds + 2`.
#[must_use]
pub const fn flat_tab_cap(enabled_kinds: usize) -> usize {
    enabled_kinds + 2
}

/// Why an agent kind cannot be opened from the `+` tab picker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KindDisabledReason {
    /// Already at [`MAX_TABS_PER_KIND`] open tabs for this kind.
    AtKindLimit,
    /// Already at the channel's flat total tab cap.
    AtTotalLimit {
        /// The flat total tab cap.
        limit: usize,
    },
    /// Turned off in Settings › Agents.
    OffInSettings,
}

/// One row in the `+` "New Desk agent tab" popover.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KindOption {
    /// The agent kind (or [`Agent::Shell`] for Terminal).
    pub agent: Agent,
    /// How many channel-wide Desk tabs of this kind are currently open.
    pub open_count: usize,
    /// Whether this kind is enabled in Settings (`Agent::Shell` is always enabled).
    pub enabled: bool,
    /// Why this kind is currently disabled, if it is.
    pub disabled_reason: Option<KindDisabledReason>,
}

/// Computed state for a channel's Desk tab strip and `+` picker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabStripSummary {
    /// Total open channel-wide Desk tabs.
    pub total_open: usize,
    /// Maximum allowed open channel-wide Desk tabs (`enabled_kinds + 2`).
    pub total_cap: usize,
    /// Per-kind options in [`Agent::ALL`] order.
    pub options: Vec<KindOption>,
}

/// Computes the tab strip summary for a channel given its currently open
/// channel-wide Desk agents and the agents enabled in Settings.
#[must_use]
pub fn compute_tab_strip(open_agents: &[Agent], enabled_agents: &[Agent]) -> TabStripSummary {
    let enabled_count = Agent::ALL
        .iter()
        .filter(|a| **a != Agent::Shell && enabled_agents.contains(a))
        .count();
    let total_cap = flat_tab_cap(enabled_count);
    let total_open = open_agents.len();

    let options = Agent::ALL
        .iter()
        .copied()
        .map(|agent| {
            let open_count = open_agents.iter().filter(|a| **a == agent).count();
            let enabled = agent == Agent::Shell || enabled_agents.contains(&agent);
            let disabled_reason = if !enabled {
                Some(KindDisabledReason::OffInSettings)
            } else if open_count >= MAX_TABS_PER_KIND {
                Some(KindDisabledReason::AtKindLimit)
            } else if total_open >= total_cap {
                Some(KindDisabledReason::AtTotalLimit { limit: total_cap })
            } else {
                None
            };
            KindOption {
                agent,
                open_count,
                enabled,
                disabled_reason,
            }
        })
        .collect();

    TabStripSummary {
        total_open,
        total_cap,
        options,
    }
}
