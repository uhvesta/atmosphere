//! Full daemon settings and per-channel settings types (`SettingsService`).
//! Covers global settings (`branch_prefix`, `branch_prefix_is_default`,
//! `default_agent`, `terminal`, `long_lines`, `retention_defaults`, `skills`,
//! `notifications`, `security.contain_agents`) and per-channel overrides.

use crate::daemon::TodoAutomation;
use crate::model::{Agent, ChannelId, MachineId};

/// How long lines wrap or scroll in diffs, editors and reviews (`LongLines`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LongLinesMode {
    #[default]
    Wrap,
    Scroll,
}

/// The default agent and optional model name (`DefaultAgent`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefaultAgentSetting {
    /// Agent id (e.g. `"claude"`, `"codex"`, `"agy"`, `"shell"`, or a custom agent id).
    pub agent: String,
    /// Optional model identifier; empty uses the agent's own default.
    pub model: String,
}

impl Default for DefaultAgentSetting {
    fn default() -> Self {
        Self {
            agent: Agent::Claude.command().to_owned(),
            model: String::new(),
        }
    }
}

impl DefaultAgentSetting {
    /// Maps the agent id to a built-in [`Agent`], if it is one.
    #[must_use]
    pub fn built_in(&self) -> Option<Agent> {
        Some(crate::daemon::agent_of_kind(&self.agent))
    }
}

/// Terminal appearance and scrollback settings (`TerminalSettings`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalSettings {
    /// Monospace font family name (e.g. `"Menlo"`).
    pub font: String,
    /// Font size in points.
    pub font_size: u8,
    /// Theme name (`""` = follow system, `"light"`, `"dark"`).
    pub theme: String,
    /// Scrollback buffer depth in lines.
    pub scrollback_lines: u32,
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            font: "Menlo".to_owned(),
            font_size: 12,
            theme: String::new(),
            scrollback_lines: 10_000,
        }
    }
}

/// Auto-archive retention rules (`RetentionRules`). `0` disables auto-archiving.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RetentionDefaults {
    pub archive_pr_reviews_after_days: u32,
    pub archive_idle_sessions_after_days: u32,
}

/// Skills sync settings (`SkillsSettings`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SkillsSettings {
    pub auto_sync: bool,
    pub opted_out: Vec<MachineId>,
    pub exclude_patterns: Vec<String>,
}

/// Notification quiet hours (`NotificationSettings`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotificationSettings {
    /// Local time `"HH:MM"`, empty for no quiet hours.
    pub quiet_from: String,
    /// Local time `"HH:MM"`, empty for no quiet hours.
    pub quiet_until: String,
    /// Whether VIP sessions break through quiet hours.
    pub vip_breaks_quiet_hours: bool,
}

/// Security settings (`SecuritySettings`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SecuritySettings {
    /// Whether agents run inside containment (`SEC-6`).
    pub contain_agents: bool,
}

/// Full global settings snapshot from `SettingsService.GetSettings`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FullSettings {
    pub default_agent: DefaultAgentSetting,
    pub terminal: TerminalSettings,
    pub branch_prefix: String,
    pub branch_prefix_is_default: bool,
    pub retention_defaults: RetentionDefaults,
    pub long_lines: LongLinesMode,
    pub skills: SkillsSettings,
    pub notifications: NotificationSettings,
    pub security: SecuritySettings,
    pub use_worktrees: bool,
    pub revision: u64,
}

impl Default for FullSettings {
    fn default() -> Self {
        Self {
            default_agent: DefaultAgentSetting::default(),
            terminal: TerminalSettings::default(),
            branch_prefix: String::new(),
            branch_prefix_is_default: true,
            retention_defaults: RetentionDefaults::default(),
            long_lines: LongLinesMode::Wrap,
            skills: SkillsSettings::default(),
            notifications: NotificationSettings::default(),
            security: SecuritySettings::default(),
            use_worktrees: true,
            revision: 0,
        }
    }
}

/// Partial update sent to `SettingsService.UpdateSettings`.
/// Only `Some` fields are sent on the wire.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SettingsPatch {
    pub default_agent: Option<DefaultAgentSetting>,
    pub terminal: Option<TerminalSettings>,
    pub branch_prefix: Option<String>,
    pub retention_defaults: Option<RetentionDefaults>,
    pub long_lines: Option<LongLinesMode>,
    pub skills: Option<SkillsSettings>,
    pub notifications: Option<NotificationSettings>,
    pub security: Option<SecuritySettings>,
    pub use_worktrees: Option<bool>,
    pub expected_revision: Option<u64>,
}

/// Per-channel settings snapshot from `SettingsService.GetChannelSettings`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelSettings {
    pub channel: ChannelId,
    /// `None` inherits the global default agent.
    pub default_agent: Option<DefaultAgentSetting>,
    /// `None` inherits the global branch prefix.
    pub branch_prefix: Option<String>,
    pub retention: RetentionDefaults,
    /// `None` inherits the global `security.contain_agents`.
    pub contain_agents: Option<bool>,
    /// `None` inherits the global To-do automation settings.
    pub todo_automation: Option<TodoAutomation>,
    pub revision: u64,
}

/// Partial update sent to `SettingsService.UpdateChannelSettings`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChannelSettingsPatch {
    pub default_agent: Option<DefaultAgentSetting>,
    pub clear_default_agent: bool,
    pub branch_prefix: Option<String>,
    pub clear_branch_prefix: bool,
    pub retention: Option<RetentionDefaults>,
    pub contain_agents: Option<bool>,
    pub clear_contain_agents: bool,
    pub todo_automation: Option<TodoAutomation>,
    pub clear_todo_automation: bool,
    pub expected_revision: Option<u64>,
}

/// Live sample branch name for a prefix (e.g. `"alice/"` -> `"alice/fix-login"`).
#[must_use]
pub fn sample_branch(prefix: &str) -> String {
    format!("{prefix}fix-login")
}

/// Whether `prefix` can start a git branch name (mirrors the daemon's `valid_prefix`).
#[must_use]
pub fn valid_branch_prefix(prefix: &str) -> bool {
    if prefix.is_empty() {
        return true;
    }
    if prefix.starts_with('/') || prefix.contains("..") || prefix.contains("//") {
        return false;
    }
    !prefix.chars().any(|c| {
        c.is_control()
            || matches!(c, ' ' | '~' | '^' | ':' | '?' | '*' | '[' | '\\' | '\t' | '\n' | '\r')
    })
}
