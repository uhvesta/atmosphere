//! Per-channel settings reducer (`SettingsService.GetChannelSettings` /
//! `UpdateChannelSettings`): branch prefix (inherit vs custom), default agent
//! and model, contain agents, and retention.

use std::collections::BTreeMap;

use humata2_clients::daemon::{
    ChannelSettings, ChannelSettingsPatch, DefaultAgentSetting, RetentionDefaults,
    valid_branch_prefix,
};
use humata2_clients::model::ChannelId;
use humata2_clients::{Clients, DaemonError};
use tca::Effect;

/// Per-channel settings state and drafts for the active channel's Settings tab.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChannelSettingsState {
    /// Loaded settings keyed by channel id.
    pub by_channel: BTreeMap<ChannelId, ChannelSettings>,
    /// The channel whose drafts are loaded into the form fields below.
    pub active_channel: Option<ChannelId>,
    /// Whether this channel uses a custom branch prefix (`true`) or inherits global (`false`).
    pub branch_prefix_custom: bool,
    /// Draft text for the channel's custom branch prefix.
    pub branch_prefix_draft: String,
    /// Draft text for `default_agent.model` when a channel default agent is set.
    pub default_model_draft: String,
    /// Draft text for `retention.archive_pr_reviews_after_days`.
    pub archive_pr_days_draft: String,
    /// Draft text for `retention.archive_idle_sessions_after_days`.
    pub archive_idle_days_draft: String,
    /// Whether a load or save call is in flight.
    pub busy: bool,
    /// Last validation or daemon error.
    pub error: Option<String>,
}

impl ChannelSettingsState {
    /// The loaded settings for `channel`, if available.
    #[must_use]
    pub fn for_channel(&self, channel: &ChannelId) -> Option<&ChannelSettings> {
        self.by_channel.get(channel)
    }

    fn sync_drafts(&mut self, channel: &ChannelId, settings: &ChannelSettings) {
        self.active_channel = Some(channel.clone());
        self.branch_prefix_custom = settings.branch_prefix.is_some();
        self.branch_prefix_draft = settings.branch_prefix.clone().unwrap_or_default();
        self.default_model_draft = settings
            .default_agent
            .as_ref()
            .map(|a| a.model.clone())
            .unwrap_or_default();
        self.archive_pr_days_draft = settings.retention.archive_pr_reviews_after_days.to_string();
        self.archive_idle_days_draft = settings
            .retention
            .archive_idle_sessions_after_days
            .to_string();
    }
}

/// Actions for a channel's Settings tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChannelSettingsAction {
    Load(ChannelId),
    Loaded {
        channel: ChannelId,
        result: Result<ChannelSettings, DaemonError>,
    },
    SetBranchPrefixMode {
        channel: ChannelId,
        custom: bool,
    },
    BranchPrefixChanged(String),
    SaveBranchPrefix(ChannelId),
    SetDefaultAgent {
        channel: ChannelId,
        /// `None` clears the override (Inherit); `Some(id)` sets the channel's agent.
        agent: Option<String>,
    },
    DefaultModelChanged(String),
    SaveDefaultModel(ChannelId),
    SetContainAgents {
        channel: ChannelId,
        /// `None` clears the override (Inherit).
        value: Option<bool>,
    },
    ArchivePrDaysChanged(String),
    ArchiveIdleDaysChanged(String),
    SaveRetention(ChannelId),
    Saved {
        channel: ChannelId,
        result: Result<ChannelSettings, DaemonError>,
    },
}

pub(crate) fn reduce(
    clients: &Clients,
    state: &mut ChannelSettingsState,
    action: ChannelSettingsAction,
) -> Effect<ChannelSettingsAction> {
    match action {
        ChannelSettingsAction::Load(channel) => {
            state.busy = true;
            state.error = None;
            state.active_channel = Some(channel.clone());
            let daemon = clients.daemon.clone();
            Effect::future(async move {
                let result = daemon.channel_settings(&channel).await;
                ChannelSettingsAction::Loaded { channel, result }
            })
        }
        ChannelSettingsAction::Loaded { channel, result } => {
            state.busy = false;
            match result {
                Ok(settings) => {
                    state.sync_drafts(&channel, &settings);
                    state.by_channel.insert(channel, settings);
                    state.error = None;
                }
                Err(error) => state.error = Some(error.to_string()),
            }
            Effect::none()
        }
        ChannelSettingsAction::SetBranchPrefixMode { channel, custom } => {
            state.branch_prefix_custom = custom;
            state.error = None;
            if !custom {
                let expected_revision = state.by_channel.get(&channel).map(|s| s.revision);
                return save(
                    clients,
                    state,
                    channel,
                    ChannelSettingsPatch {
                        clear_branch_prefix: true,
                        expected_revision,
                        ..Default::default()
                    },
                );
            }
            Effect::none()
        }
        ChannelSettingsAction::BranchPrefixChanged(text) => {
            state.branch_prefix_draft = text;
            state.branch_prefix_custom = true;
            state.error = None;
            Effect::none()
        }
        ChannelSettingsAction::SaveBranchPrefix(channel) => {
            let prefix = state.branch_prefix_draft.trim().to_owned();
            if !valid_branch_prefix(&prefix) {
                state.error = Some(
                    "That prefix can't begin a git branch name. Use letters, digits, -, _, . or /."
                        .to_owned(),
                );
                return Effect::none();
            }
            let expected_revision = state.by_channel.get(&channel).map(|s| s.revision);
            save(
                clients,
                state,
                channel,
                ChannelSettingsPatch {
                    branch_prefix: Some(prefix),
                    expected_revision,
                    ..Default::default()
                },
            )
        }
        ChannelSettingsAction::SetDefaultAgent { channel, agent } => {
            let expected_revision = state.by_channel.get(&channel).map(|s| s.revision);
            let existing_model = state
                .by_channel
                .get(&channel)
                .and_then(|s| s.default_agent.as_ref())
                .map(|a| a.model.clone())
                .unwrap_or_default();
            let patch = match agent {
                None => ChannelSettingsPatch {
                    clear_default_agent: true,
                    expected_revision,
                    ..Default::default()
                },
                Some(agent_id) => ChannelSettingsPatch {
                    default_agent: Some(DefaultAgentSetting {
                        agent: agent_id,
                        model: existing_model,
                    }),
                    expected_revision,
                    ..Default::default()
                },
            };
            save(clients, state, channel, patch)
        }
        ChannelSettingsAction::DefaultModelChanged(text) => {
            state.default_model_draft = text;
            state.error = None;
            Effect::none()
        }
        ChannelSettingsAction::SaveDefaultModel(channel) => {
            let Some(current) = state.by_channel.get(&channel) else {
                return Effect::none();
            };
            let agent = current
                .default_agent
                .as_ref()
                .map(|a| a.agent.clone())
                .unwrap_or_else(|| "claude".to_owned());
            let expected_revision = Some(current.revision);
            save(
                clients,
                state,
                channel,
                ChannelSettingsPatch {
                    default_agent: Some(DefaultAgentSetting {
                        agent,
                        model: state.default_model_draft.trim().to_owned(),
                    }),
                    expected_revision,
                    ..Default::default()
                },
            )
        }
        ChannelSettingsAction::SetContainAgents { channel, value } => {
            let expected_revision = state.by_channel.get(&channel).map(|s| s.revision);
            let patch = match value {
                None => ChannelSettingsPatch {
                    clear_contain_agents: true,
                    expected_revision,
                    ..Default::default()
                },
                Some(on) => ChannelSettingsPatch {
                    contain_agents: Some(on),
                    expected_revision,
                    ..Default::default()
                },
            };
            save(clients, state, channel, patch)
        }
        ChannelSettingsAction::ArchivePrDaysChanged(text) => {
            state.archive_pr_days_draft = text;
            state.error = None;
            Effect::none()
        }
        ChannelSettingsAction::ArchiveIdleDaysChanged(text) => {
            state.archive_idle_days_draft = text;
            state.error = None;
            Effect::none()
        }
        ChannelSettingsAction::SaveRetention(channel) => {
            let pr_days = match state.archive_pr_days_draft.trim().parse::<u32>() {
                Ok(n) => n,
                Err(_) => {
                    state.error = Some("PR review retention days must be a whole number (0 = off).".to_owned());
                    return Effect::none();
                }
            };
            let idle_days = match state.archive_idle_days_draft.trim().parse::<u32>() {
                Ok(n) => n,
                Err(_) => {
                    state.error = Some("Idle session retention days must be a whole number (0 = off).".to_owned());
                    return Effect::none();
                }
            };
            let expected_revision = state.by_channel.get(&channel).map(|s| s.revision);
            save(
                clients,
                state,
                channel,
                ChannelSettingsPatch {
                    retention: Some(RetentionDefaults {
                        archive_pr_reviews_after_days: pr_days,
                        archive_idle_sessions_after_days: idle_days,
                    }),
                    expected_revision,
                    ..Default::default()
                },
            )
        }
        ChannelSettingsAction::Saved { channel, result } => {
            state.busy = false;
            match result {
                Ok(settings) => {
                    state.sync_drafts(&channel, &settings);
                    state.by_channel.insert(channel, settings);
                    state.error = None;
                }
                Err(error) => state.error = Some(error.to_string()),
            }
            Effect::none()
        }
    }
}

fn save(
    clients: &Clients,
    state: &mut ChannelSettingsState,
    channel: ChannelId,
    patch: ChannelSettingsPatch,
) -> Effect<ChannelSettingsAction> {
    state.busy = true;
    state.error = None;
    let daemon = clients.daemon.clone();
    Effect::future(async move {
        let result = daemon.update_channel_settings(&channel, patch).await;
        ChannelSettingsAction::Saved { channel, result }
    })
}
