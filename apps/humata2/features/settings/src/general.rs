//! Daemon-backed global settings (`SettingsService`): default branch prefix,
//! security containment, retention defaults, default agent model, terminal
//! font/scrollback, and notification quiet hours.

use humata2_clients::daemon::{
    DefaultAgentSetting, FullSettings, NotificationSettings, RetentionDefaults, SecuritySettings,
    SettingsPatch, TerminalSettings, valid_branch_prefix,
};
use humata2_clients::{Clients, DaemonError};
use tca::Effect;

/// Daemon-backed global settings state and in-progress form drafts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GeneralState {
    /// Latest snapshot from the daemon (`None` until loaded).
    pub settings: Option<FullSettings>,
    /// Draft text for the default branch prefix input.
    pub branch_prefix_draft: String,
    /// Last observed `@user/` default prefix when `branch_prefix_is_default` was true.
    pub default_branch_prefix: Option<String>,
    /// Draft text for `default_agent.model` in Settings › Agents.
    pub default_model_draft: String,
    /// Draft text for `terminal.font` in Settings › Layout.
    pub terminal_font_draft: String,
    /// Draft text for `terminal.scrollback_lines` in Settings › Layout.
    pub scrollback_draft: String,
    /// Draft text for `notifications.quiet_from` (`HH:MM`).
    pub quiet_from_draft: String,
    /// Draft text for `notifications.quiet_until` (`HH:MM`).
    pub quiet_until_draft: String,
    /// Draft text for `retention_defaults.archive_pr_reviews_after_days`.
    pub archive_pr_days_draft: String,
    /// Draft text for `retention_defaults.archive_idle_sessions_after_days`.
    pub archive_idle_days_draft: String,
    /// Whether a load, refresh, or save call is in flight.
    pub busy: bool,
    /// Last validation or daemon error for these fields.
    pub error: Option<String>,
}

impl GeneralState {
    fn sync_from(&mut self, full: &FullSettings) {
        self.branch_prefix_draft = full.branch_prefix.clone();
        if full.branch_prefix_is_default && !full.branch_prefix.is_empty() {
            self.default_branch_prefix = Some(full.branch_prefix.clone());
        }
        self.default_model_draft = full.default_agent.model.clone();
        self.terminal_font_draft = full.terminal.font.clone();
        self.scrollback_draft = full.terminal.scrollback_lines.to_string();
        self.quiet_from_draft = full.notifications.quiet_from.clone();
        self.quiet_until_draft = full.notifications.quiet_until.clone();
        self.archive_pr_days_draft = full
            .retention_defaults
            .archive_pr_reviews_after_days
            .to_string();
        self.archive_idle_days_draft = full
            .retention_defaults
            .archive_idle_sessions_after_days
            .to_string();
    }

    /// Current settings or compiled-in defaults while loading.
    #[must_use]
    pub fn current(&self) -> FullSettings {
        self.settings.clone().unwrap_or_default()
    }
}

/// Actions for daemon-backed global settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GeneralAction {
    Load,
    Loaded(Result<FullSettings, DaemonError>),
    BranchPrefixChanged(String),
    SaveBranchPrefix,
    ResetBranchPrefix,
    RefreshGhLogin,
    ToggleContainAgents,
    ArchivePrDaysChanged(String),
    ArchiveIdleDaysChanged(String),
    SaveRetention,
    DefaultModelChanged(String),
    SaveDefaultModel,
    TerminalFontChanged(String),
    SaveTerminalFont,
    ScrollbackChanged(String),
    SaveScrollback,
    QuietFromChanged(String),
    QuietUntilChanged(String),
    SaveQuietHours,
    ToggleVipBreaksQuietHours,
    Saved(Result<FullSettings, DaemonError>),
}

pub(crate) fn reduce(
    clients: &Clients,
    state: &mut GeneralState,
    action: GeneralAction,
) -> Effect<GeneralAction> {
    match action {
        GeneralAction::Load => {
            state.busy = true;
            let daemon = clients.daemon.clone();
            Effect::future(async move { GeneralAction::Loaded(daemon.full_settings().await) })
        }
        GeneralAction::Loaded(result) => {
            state.busy = false;
            match result {
                Ok(full) => {
                    state.sync_from(&full);
                    state.settings = Some(full);
                    state.error = None;
                }
                Err(error) => state.error = Some(error.to_string()),
            }
            Effect::none()
        }
        GeneralAction::BranchPrefixChanged(text) => {
            state.branch_prefix_draft = text;
            state.error = None;
            Effect::none()
        }
        GeneralAction::SaveBranchPrefix => {
            let prefix = state.branch_prefix_draft.trim().to_owned();
            if !valid_branch_prefix(&prefix) {
                state.error = Some(
                    "That prefix can't begin a git branch name. Use letters, digits, -, _, . or /."
                        .to_owned(),
                );
                return Effect::none();
            }
            let expected_revision = state.settings.as_ref().map(|s| s.revision);
            save(
                clients,
                state,
                SettingsPatch {
                    branch_prefix: Some(prefix),
                    expected_revision,
                    ..Default::default()
                },
            )
        }
        GeneralAction::ResetBranchPrefix => {
            let reset_to = state
                .default_branch_prefix
                .clone()
                .unwrap_or_default();
            let expected_revision = state.settings.as_ref().map(|s| s.revision);
            save(
                clients,
                state,
                SettingsPatch {
                    branch_prefix: Some(reset_to),
                    expected_revision,
                    ..Default::default()
                },
            )
        }
        GeneralAction::RefreshGhLogin => {
            state.busy = true;
            state.error = None;
            let daemon = clients.daemon.clone();
            Effect::future(async move { GeneralAction::Loaded(daemon.refresh_gh_login().await) })
        }
        GeneralAction::ToggleContainAgents => {
            let current = state.current();
            let expected_revision = state.settings.as_ref().map(|s| s.revision);
            save(
                clients,
                state,
                SettingsPatch {
                    security: Some(SecuritySettings {
                        contain_agents: !current.security.contain_agents,
                    }),
                    expected_revision,
                    ..Default::default()
                },
            )
        }
        GeneralAction::ArchivePrDaysChanged(text) => {
            state.archive_pr_days_draft = text;
            state.error = None;
            Effect::none()
        }
        GeneralAction::ArchiveIdleDaysChanged(text) => {
            state.archive_idle_days_draft = text;
            state.error = None;
            Effect::none()
        }
        GeneralAction::SaveRetention => {
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
            let expected_revision = state.settings.as_ref().map(|s| s.revision);
            save(
                clients,
                state,
                SettingsPatch {
                    retention_defaults: Some(RetentionDefaults {
                        archive_pr_reviews_after_days: pr_days,
                        archive_idle_sessions_after_days: idle_days,
                    }),
                    expected_revision,
                    ..Default::default()
                },
            )
        }
        GeneralAction::DefaultModelChanged(text) => {
            state.default_model_draft = text;
            state.error = None;
            Effect::none()
        }
        GeneralAction::SaveDefaultModel => {
            let current = state.current();
            let expected_revision = state.settings.as_ref().map(|s| s.revision);
            save(
                clients,
                state,
                SettingsPatch {
                    default_agent: Some(DefaultAgentSetting {
                        agent: current.default_agent.agent,
                        model: state.default_model_draft.trim().to_owned(),
                    }),
                    expected_revision,
                    ..Default::default()
                },
            )
        }
        GeneralAction::TerminalFontChanged(text) => {
            state.terminal_font_draft = text;
            state.error = None;
            Effect::none()
        }
        GeneralAction::SaveTerminalFont => {
            let font = state.terminal_font_draft.trim().to_owned();
            if font.is_empty() {
                state.error = Some("Terminal font name cannot be empty.".to_owned());
                return Effect::none();
            }
            let current = state.current();
            let expected_revision = state.settings.as_ref().map(|s| s.revision);
            save(
                clients,
                state,
                SettingsPatch {
                    terminal: Some(TerminalSettings {
                        font,
                        ..current.terminal
                    }),
                    expected_revision,
                    ..Default::default()
                },
            )
        }
        GeneralAction::ScrollbackChanged(text) => {
            state.scrollback_draft = text;
            state.error = None;
            Effect::none()
        }
        GeneralAction::SaveScrollback => {
            let lines = match state.scrollback_draft.trim().parse::<u32>() {
                Ok(n) if n > 0 => n,
                _ => {
                    state.error = Some("Scrollback lines must be a positive number.".to_owned());
                    return Effect::none();
                }
            };
            let current = state.current();
            let expected_revision = state.settings.as_ref().map(|s| s.revision);
            save(
                clients,
                state,
                SettingsPatch {
                    terminal: Some(TerminalSettings {
                        scrollback_lines: lines,
                        ..current.terminal
                    }),
                    expected_revision,
                    ..Default::default()
                },
            )
        }
        GeneralAction::QuietFromChanged(text) => {
            state.quiet_from_draft = text;
            state.error = None;
            Effect::none()
        }
        GeneralAction::QuietUntilChanged(text) => {
            state.quiet_until_draft = text;
            state.error = None;
            Effect::none()
        }
        GeneralAction::SaveQuietHours => {
            let from = state.quiet_from_draft.trim().to_owned();
            let until = state.quiet_until_draft.trim().to_owned();
            if from.is_empty() != until.is_empty() {
                state.error = Some("Set both quiet hours start and end, or leave both empty.".to_owned());
                return Effect::none();
            }
            let current = state.current();
            let expected_revision = state.settings.as_ref().map(|s| s.revision);
            save(
                clients,
                state,
                SettingsPatch {
                    notifications: Some(NotificationSettings {
                        quiet_from: from,
                        quiet_until: until,
                        vip_breaks_quiet_hours: current.notifications.vip_breaks_quiet_hours,
                    }),
                    expected_revision,
                    ..Default::default()
                },
            )
        }
        GeneralAction::ToggleVipBreaksQuietHours => {
            let current = state.current();
            let expected_revision = state.settings.as_ref().map(|s| s.revision);
            save(
                clients,
                state,
                SettingsPatch {
                    notifications: Some(NotificationSettings {
                        vip_breaks_quiet_hours: !current.notifications.vip_breaks_quiet_hours,
                        ..current.notifications
                    }),
                    expected_revision,
                    ..Default::default()
                },
            )
        }
        GeneralAction::Saved(result) => {
            state.busy = false;
            match result {
                Ok(full) => {
                    state.sync_from(&full);
                    state.settings = Some(full);
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
    state: &mut GeneralState,
    patch: SettingsPatch,
) -> Effect<GeneralAction> {
    state.busy = true;
    state.error = None;
    let daemon = clients.daemon.clone();
    Effect::future(async move { GeneralAction::Saved(daemon.update_settings(patch).await) })
}
