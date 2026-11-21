//! The agent settings the daemon's SettingsService keeps: the default agent
//! and whether Codex's hooks need trusting. Where each agent asks is its
//! switch (D782, `agent_switches`).

use humata2_clients::DaemonError;
use humata2_clients::daemon::{
    ChannelSettings, ChannelSettingsPatch, DefaultAgentSetting, FullSettings, FuseMountStatus,
    LongLinesMode, MachineSyncSettings, NotificationSettings, RetentionDefaults, SecuritySettings,
    SettingsPatch, SkillsSettings, SyncChange, SyncSettings, SyncSkillsMode, SyncSnapshot,
    TerminalSettings, TodoAutomation, TodoMode, WebTodoSnapshot, WebTodoStatus,
};
use humata2_clients::model::{
    Agent, BrowserBridgeStatus, BrowserFamily, BrowserKind, ChannelId, ConnectedBrowserHost,
    DetectedBrowser, MachineId,
};
use crate::humata2_grpc::humata::v1 as pb;

use crate::connection::{Shared, fresh_key};

impl Shared {

    /// Whether Codex's hooks still need trusting in its /hooks (D202). A
    /// daemon that sends no agent settings has nothing to trust.
    pub(crate) async fn codex_needs_trust(&self) -> Result<bool, DaemonError> {
        const WHAT: &str = "Reading whether Codex's hooks need trusting";
        let shown = self
            .settings()
            .get_settings(pb::GetSettingsRequest {})
            .await
            .map_err(|status| self.error(&status, WHAT))?
            .into_inner();
        Ok(shown
            .agents
            .is_some_and(|agents| agents.codex_hooks_need_trust))
    }

    /// Makes `agent` Settings' default agent (ST-15), preserving any saved
    /// `default_agent.model` and sending `expected_revision` (SETTINGSFIX).
    pub(crate) async fn change_default_agent(&self, agent: Agent) -> Result<(), DaemonError> {
        const WHAT: &str = "Changing the default agent";
        let current = self
            .settings()
            .get_settings(pb::GetSettingsRequest {})
            .await
            .map_err(|status| self.error(&status, WHAT))?
            .into_inner();
        let request = build_default_agent_update(&current, self.agents.name(agent));
        self.settings()
            .update_settings(request)
            .await
            .map_err(|status| self.error(&status, WHAT))?;
        Ok(())
    }

    /// Reads the full global settings snapshot (`SettingsService.GetSettings`).
    pub(crate) async fn full_settings(&self) -> Result<FullSettings, DaemonError> {
        const WHAT: &str = "Reading the settings";
        let shown = self
            .settings()
            .get_settings(pb::GetSettingsRequest {})
            .await
            .map_err(|status| self.error(&status, WHAT))?
            .into_inner();
        Ok(full_settings_of_wire(&shown))
    }

    /// Applies a partial settings update (`SettingsService.UpdateSettings`).
    pub(crate) async fn update_settings(&self, patch: SettingsPatch) -> Result<FullSettings, DaemonError> {
        const WHAT: &str = "Saving the settings";
        let request = update_settings_to_wire(&patch);
        let saved = self
            .settings()
            .update_settings(request)
            .await
            .map_err(|status| self.error(&status, WHAT))?
            .into_inner();
        Ok(full_settings_of_wire(&saved))
    }

    /// Triggers a `gh` login refresh (which `GetSettings` kicks off whenever
    /// `branch_prefix_is_default` is true) and returns the current settings.
    pub(crate) async fn refresh_gh_login(&self) -> Result<FullSettings, DaemonError> {
        self.full_settings().await
    }

    /// Reads per-channel settings (`SettingsService.GetChannelSettings`).
    pub(crate) async fn channel_settings(
        &self,
        channel: ChannelId,
    ) -> Result<ChannelSettings, DaemonError> {
        const WHAT: &str = "Reading the channel's settings";
        let shown = self
            .settings()
            .get_channel_settings(pb::GetChannelSettingsRequest {
                channel: Some(pb::ChannelId {
                    value: channel.as_str().to_owned(),
                }),
            })
            .await
            .map_err(|status| self.error(&status, WHAT))?
            .into_inner();
        Ok(channel_settings_of_wire(&shown, channel))
    }

    /// Updates per-channel settings (`SettingsService.UpdateChannelSettings`).
    pub(crate) async fn update_channel_settings(
        &self,
        channel: ChannelId,
        patch: ChannelSettingsPatch,
    ) -> Result<ChannelSettings, DaemonError> {
        const WHAT: &str = "Saving the channel's settings";
        let request = update_channel_settings_to_wire(&channel, &patch);
        let saved = self
            .settings()
            .update_channel_settings(request)
            .await
            .map_err(|status| self.error(&status, WHAT))?
            .into_inner();
        Ok(channel_settings_of_wire(&saved, channel))
    }

    /// What Settings › Sync holds: the Sync section and the mount switch.
    pub(crate) async fn sync_snapshot(&self) -> Result<SyncSnapshot, DaemonError> {
        const WHAT: &str = "Reading the Sync settings";
        let shown = self
            .settings()
            .get_settings(pb::GetSettingsRequest {})
            .await
            .map_err(|status| self.error(&status, WHAT))?
            .into_inner();
        Ok(snapshot_of(&shown))
    }

    /// Writes the Sync section and/or the mount switch, sending only what
    /// `change` carries; gives what the daemon holds after.
    pub(crate) async fn change_sync_settings(
        &self,
        change: SyncChange,
    ) -> Result<SyncSnapshot, DaemonError> {
        const WHAT: &str = "Changing the Sync settings";
        let saved = self
            .settings()
            .update_settings(pb::UpdateSettingsRequest {
                idempotency_key: fresh_key(),
                sync: change.sync.as_ref().map(sync_to_wire),
                fusefs_enabled: change.fusefs_enabled,
                ..Default::default()
            })
            .await
            .map_err(|status| self.error(&status, WHAT))?
            .into_inner();
        Ok(snapshot_of(&saved))
    }

    /// Reads the To-do web UI switch and live loopback URL (WEBTODO, D905).
    pub(crate) async fn web_todo_snapshot(&self) -> Result<WebTodoSnapshot, DaemonError> {
        const WHAT: &str = "Reading the To-do web UI setting";
        let shown = self
            .settings()
            .get_settings(pb::GetSettingsRequest {})
            .await
            .map_err(|status| self.error(&status, WHAT))?
            .into_inner();
        Ok(web_todo_of(&shown))
    }

    /// Turns the host daemon's loopback To-do web server on or off (WEBTODO, D905).
    pub(crate) async fn change_web_todo_enabled(
        &self,
        enabled: bool,
    ) -> Result<WebTodoSnapshot, DaemonError> {
        const WHAT: &str = "Changing the To-do web UI setting";
        let saved = self
            .settings()
            .update_settings(pb::UpdateSettingsRequest {
                idempotency_key: fresh_key(),
                web_todo_enabled: Some(enabled),
                ..Default::default()
            })
            .await
            .map_err(|status| self.error(&status, WHAT))?
            .into_inner();
        Ok(web_todo_of(&saved))
    }

    /// What Settings › General › To-do holds (TODO2WIRE).
    pub(crate) async fn todo_automation(&self) -> Result<TodoAutomation, DaemonError> {
        const WHAT: &str = "Reading the To-do automation settings";
        let shown = self
            .settings()
            .get_settings(pb::GetSettingsRequest {})
            .await
            .map_err(|status| self.error(&status, WHAT))?
            .into_inner();
        Ok(todo_of_settings(&shown))
    }

    /// Replaces the whole To-do automation section; gives what the daemon
    /// holds after.
    pub(crate) async fn change_todo_automation(
        &self,
        section: TodoAutomation,
    ) -> Result<TodoAutomation, DaemonError> {
        const WHAT: &str = "Changing the To-do automation settings";
        let saved = self
            .settings()
            .update_settings(pb::UpdateSettingsRequest {
                idempotency_key: fresh_key(),
                todo_automation: Some(todo_to_wire(&section)),
                ..Default::default()
            })
            .await
            .map_err(|status| self.error(&status, WHAT))?
            .into_inner();
        Ok(todo_of_settings(&saved))
    }

    /// The browsers found on this Mac and whether each has the native messaging
    /// host set up (`BrowserBridgeService`, BROWSER).
    pub(crate) async fn list_detected_browsers(&self) -> Result<Vec<DetectedBrowser>, DaemonError> {
        const WHAT: &str = "Listing detected browsers";
        let reply = self
            .browser_bridge()
            .list_detected_browsers(pb::ListDetectedBrowsersRequest {})
            .await
            .map_err(|status| self.error(&status, WHAT))?
            .into_inner();
        Ok(reply.browsers.iter().filter_map(detected_browser_of_wire).collect())
    }

    /// Installs or updates Humata's native messaging host manifest for `kinds`
    /// (every found browser when empty) via `BrowserBridgeService` (BROWSER).
    pub(crate) async fn install_browser_host(
        &self,
        kinds: Vec<BrowserKind>,
    ) -> Result<Vec<DetectedBrowser>, DaemonError> {
        const WHAT: &str = "Setting up the browser host";
        let reply = self
            .browser_bridge()
            .install_browser_host(pb::InstallBrowserHostRequest {
                idempotency_key: fresh_key(),
                kinds: kinds.into_iter().map(browser_kind_to_wire).collect(),
            })
            .await
            .map_err(|status| self.error(&status, WHAT))?
            .into_inner();
        Ok(reply.browsers.iter().filter_map(detected_browser_of_wire).collect())
    }

    /// Removes Humata's native messaging host manifest for `kinds` (every found
    /// browser when empty) via `BrowserBridgeService` (BROWSER).
    pub(crate) async fn uninstall_browser_host(
        &self,
        kinds: Vec<BrowserKind>,
    ) -> Result<Vec<DetectedBrowser>, DaemonError> {
        const WHAT: &str = "Uninstalling the browser host";
        let reply = self
            .browser_bridge()
            .uninstall_browser_host(pb::UninstallBrowserHostRequest {
                idempotency_key: fresh_key(),
                kinds: kinds.into_iter().map(browser_kind_to_wire).collect(),
            })
            .await
            .map_err(|status| self.error(&status, WHAT))?
            .into_inner();
        Ok(reply.browsers.iter().filter_map(detected_browser_of_wire).collect())
    }

    /// Live status of the daemon's browser bridge (`BrowserBridgeService`, BROWSER).
    pub(crate) async fn browser_bridge_status(&self) -> Result<BrowserBridgeStatus, DaemonError> {
        const WHAT: &str = "Checking browser bridge status";
        let reply = self
            .browser_bridge()
            .get_browser_bridge_status(pb::GetBrowserBridgeStatusRequest {})
            .await
            .map_err(|status| self.error(&status, WHAT))?
            .into_inner();
        Ok(bridge_status_of_wire(&reply))
    }
}

fn browser_kind_of_wire(raw: i32) -> Option<BrowserKind> {
    match pb::BrowserKind::try_from(raw).ok()? {
        pb::BrowserKind::Chrome => Some(BrowserKind::Chrome),
        pb::BrowserKind::Brave => Some(BrowserKind::Brave),
        pb::BrowserKind::Edge => Some(BrowserKind::Edge),
        pb::BrowserKind::Chromium => Some(BrowserKind::Chromium),
        pb::BrowserKind::Arc => Some(BrowserKind::Arc),
        pb::BrowserKind::Vivaldi => Some(BrowserKind::Vivaldi),
        pb::BrowserKind::Firefox => Some(BrowserKind::Firefox),
        pb::BrowserKind::Unspecified => None,
    }
}

fn browser_kind_to_wire(kind: BrowserKind) -> i32 {
    match kind {
        BrowserKind::Chrome => pb::BrowserKind::Chrome as i32,
        BrowserKind::Brave => pb::BrowserKind::Brave as i32,
        BrowserKind::Edge => pb::BrowserKind::Edge as i32,
        BrowserKind::Chromium => pb::BrowserKind::Chromium as i32,
        BrowserKind::Arc => pb::BrowserKind::Arc as i32,
        BrowserKind::Vivaldi => pb::BrowserKind::Vivaldi as i32,
        BrowserKind::Firefox => pb::BrowserKind::Firefox as i32,
    }
}

fn browser_family_of_wire(raw: i32) -> BrowserFamily {
    match pb::BrowserFamily::try_from(raw) {
        Ok(pb::BrowserFamily::Firefox) => BrowserFamily::Firefox,
        Ok(pb::BrowserFamily::Chromium | pb::BrowserFamily::Unspecified) | Err(_) => {
            BrowserFamily::Chromium
        }
    }
}

fn detected_browser_of_wire(wire: &pb::DetectedBrowser) -> Option<DetectedBrowser> {
    let kind = browser_kind_of_wire(wire.kind)?;
    Some(DetectedBrowser {
        kind,
        family: browser_family_of_wire(wire.family),
        display_name: if wire.display_name.is_empty() {
            kind.display_name().to_owned()
        } else {
            wire.display_name.clone()
        },
        binary_path: wire.binary_path.clone(),
        native_messaging_dir: wire.native_messaging_dir.clone(),
        manifest_path: wire.manifest_path.clone(),
        host_installed: wire.host_installed,
        host_matches_current: wire.host_matches_current,
        connected: wire.connected,
        extension_path: wire.extension_path.clone(),
    })
}

fn bridge_status_of_wire(wire: &pb::GetBrowserBridgeStatusResponse) -> BrowserBridgeStatus {
    BrowserBridgeStatus {
        connected: wire.connected,
        hosts: wire
            .hosts
            .iter()
            .map(|host| ConnectedBrowserHost {
                extension_origin: host.extension_origin.clone(),
                host_pid: host.host_pid,
                family: browser_family_of_wire(host.family),
            })
            .collect(),
    }
}

fn todo_of_settings(settings: &pb::Settings) -> TodoAutomation {
    settings.todo_automation.as_ref().map(todo_of_wire).unwrap_or_default()
}

fn todo_mode_of_wire(raw: i32) -> TodoMode {
    match pb::TodoSyncMode::try_from(raw) {
        Ok(pb::TodoSyncMode::Auto) => TodoMode::Auto,
        Ok(pb::TodoSyncMode::Ask | pb::TodoSyncMode::Unspecified) | Err(_) => TodoMode::Ask,
    }
}

fn todo_mode_to_wire(mode: TodoMode) -> i32 {
    match mode {
        TodoMode::Ask => pb::TodoSyncMode::Ask as i32,
        TodoMode::Auto => pb::TodoSyncMode::Auto as i32,
    }
}

fn todo_of_wire(wire: &pb::TodoAutomationSettings) -> TodoAutomation {
    TodoAutomation {
        on_pr_merged: todo_mode_of_wire(wire.on_pr_merged),
        on_todo_closed_archive_sessions: todo_mode_of_wire(wire.on_todo_closed_archive_sessions),
        auto_wake_on_ci_fail: wire.auto_wake_on_ci_fail,
        auto_wake_on_review_comments: wire.auto_wake_on_review_comments,
        idle_nudge_days: wire.idle_nudge_days,
    }
    .normalized()
}

fn todo_to_wire(section: &TodoAutomation) -> pb::TodoAutomationSettings {
    pb::TodoAutomationSettings {
        on_pr_merged: todo_mode_to_wire(section.on_pr_merged),
        on_todo_closed_archive_sessions: todo_mode_to_wire(section.on_todo_closed_archive_sessions),
        auto_wake_on_ci_fail: section.auto_wake_on_ci_fail,
        auto_wake_on_review_comments: section.auto_wake_on_review_comments,
        idle_nudge_days: section.idle_nudge_days,
    }
}

fn snapshot_of(settings: &pb::Settings) -> SyncSnapshot {
    SyncSnapshot {
        sync: settings.sync.as_ref().map(sync_of_wire).unwrap_or_default(),
        fusefs_enabled: settings.fusefs_enabled,
        fusefs_status: fusefs_status_of(settings.fusefs_status.as_ref()),
    }
}

fn fusefs_status_of(wire: Option<&pb::FuseMountStatus>) -> FuseMountStatus {
    use pb::fuse_mount_status::State;
    let Some(wire) = wire else {
        return FuseMountStatus::Off;
    };
    match State::try_from(wire.state) {
        Ok(State::Starting) => FuseMountStatus::Starting,
        Ok(State::Mounted) => FuseMountStatus::Mounted { path: wire.mount_root.clone() },
        Ok(State::CannotSetUp) => FuseMountStatus::CannotSetUp { reason: wire.reason.clone() },
        Ok(State::Off | State::Unspecified) | Err(_) => FuseMountStatus::Off,
    }
}

fn web_todo_of(settings: &pb::Settings) -> WebTodoSnapshot {
    WebTodoSnapshot {
        enabled: settings.web_todo_enabled,
        status: web_todo_status_of(settings.web_todo_status.as_ref()),
    }
}

fn web_todo_status_of(wire: Option<&pb::WebTodoStatus>) -> WebTodoStatus {
    use pb::web_todo_status::State;
    let Some(wire) = wire else {
        return WebTodoStatus::Off;
    };
    match State::try_from(wire.state) {
        Ok(State::Starting) => WebTodoStatus::Starting,
        Ok(State::Listening) => WebTodoStatus::Listening { url: wire.url.clone() },
        Ok(State::Failed) => WebTodoStatus::Failed { reason: wire.reason.clone() },
        Ok(State::Off | State::Unspecified) | Err(_) => WebTodoStatus::Off,
    }
}

fn mode_of_wire(raw: i32) -> SyncSkillsMode {
    match pb::SyncSkillsMode::try_from(raw) {
        Ok(pb::SyncSkillsMode::HostOnly) => SyncSkillsMode::HostOnly,
        Ok(pb::SyncSkillsMode::HostWins | pb::SyncSkillsMode::Unspecified) | Err(_) => {
            SyncSkillsMode::HostWins
        }
    }
}

fn mode_to_wire(mode: SyncSkillsMode) -> i32 {
    match mode {
        SyncSkillsMode::HostWins => pb::SyncSkillsMode::HostWins as i32,
        SyncSkillsMode::HostOnly => pb::SyncSkillsMode::HostOnly as i32,
    }
}

fn sync_of_wire(wire: &pb::SyncSettings) -> SyncSettings {
    SyncSettings {
        sync_enabled: wire.sync_enabled,
        skills: wire.skills,
        instructions: wire.instructions,
        mcp: wire.mcp,
        settings: wire.settings,
        skills_mode: mode_of_wire(wire.skills_mode),
        machines: wire
            .machines
            .iter()
            .map(|(id, m)| {
                (
                    id.clone(),
                    MachineSyncSettings {
                        enabled: m.enabled,
                        skills: m.skills,
                        instructions: m.instructions,
                        mcp: m.mcp,
                        settings: m.settings,
                        skills_mode: m.skills_mode.map(mode_of_wire),
                    },
                )
            })
            .collect(),
    }
}

fn sync_to_wire(sync: &SyncSettings) -> pb::SyncSettings {
    pb::SyncSettings {
        sync_enabled: sync.sync_enabled,
        skills: sync.skills,
        instructions: sync.instructions,
        mcp: sync.mcp,
        settings: sync.settings,
        skills_mode: mode_to_wire(sync.skills_mode),
        machines: sync
            .machines
            .iter()
            .map(|(id, m)| {
                (
                    id.clone(),
                    pb::MachineSyncOverride {
                        enabled: m.enabled,
                        skills: m.skills,
                        instructions: m.instructions,
                        mcp: m.mcp,
                        settings: m.settings,
                        skills_mode: m.skills_mode.map(mode_to_wire),
                    },
                )
            })
            .collect(),
    }
}

fn build_default_agent_update(
    current: &pb::Settings,
    agent_id: String,
) -> pb::UpdateSettingsRequest {
    let model = current
        .default_agent
        .as_ref()
        .map(|d| d.model.clone())
        .unwrap_or_default();
    pb::UpdateSettingsRequest {
        idempotency_key: fresh_key(),
        default_agent: Some(pb::DefaultAgent {
            agent: Some(pb::AgentId { value: agent_id }),
            model,
        }),
        expected_revision: current.revision.clone(),
        ..Default::default()
    }
}

fn long_lines_of_wire(raw: i32) -> LongLinesMode {
    match pb::LongLines::try_from(raw) {
        Ok(pb::LongLines::Scroll) => LongLinesMode::Scroll,
        Ok(pb::LongLines::Wrap | pb::LongLines::Unspecified) | Err(_) => LongLinesMode::Wrap,
    }
}

fn long_lines_to_wire(mode: LongLinesMode) -> i32 {
    match mode {
        LongLinesMode::Wrap => pb::LongLines::Wrap as i32,
        LongLinesMode::Scroll => pb::LongLines::Scroll as i32,
    }
}

fn default_agent_of_wire(wire: &pb::DefaultAgent) -> DefaultAgentSetting {
    DefaultAgentSetting {
        agent: wire
            .agent
            .as_ref()
            .map(|a| a.value.clone())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| Agent::Claude.command().to_owned()),
        model: wire.model.clone(),
    }
}

fn default_agent_to_wire(setting: &DefaultAgentSetting) -> pb::DefaultAgent {
    pb::DefaultAgent {
        agent: Some(pb::AgentId {
            value: setting.agent.clone(),
        }),
        model: setting.model.clone(),
    }
}

fn retention_of_wire(wire: &pb::RetentionRules) -> RetentionDefaults {
    RetentionDefaults {
        archive_pr_reviews_after_days: wire.archive_pr_reviews_after_days,
        archive_idle_sessions_after_days: wire.archive_idle_sessions_after_days,
    }
}

fn retention_to_wire(rules: RetentionDefaults) -> pb::RetentionRules {
    pb::RetentionRules {
        archive_pr_reviews_after_days: rules.archive_pr_reviews_after_days,
        archive_idle_sessions_after_days: rules.archive_idle_sessions_after_days,
    }
}

fn full_settings_of_wire(wire: &pb::Settings) -> FullSettings {
    let terminal = wire
        .terminal
        .as_ref()
        .map(|t| TerminalSettings {
            font: if t.font.is_empty() {
                TerminalSettings::default().font
            } else {
                t.font.clone()
            },
            font_size: if t.font_size <= 0.0 {
                TerminalSettings::default().font_size
            } else {
                t.font_size.round().clamp(1.0, 255.0) as u8
            },
            theme: t.theme.clone(),
            scrollback_lines: t.scrollback_lines,
        })
        .unwrap_or_default();
    let skills = wire
        .skills
        .as_ref()
        .map(|s| SkillsSettings {
            auto_sync: s.auto_sync,
            opted_out: s
                .opted_out
                .iter()
                .map(|m| MachineId::new(m.value.clone()))
                .collect(),
            exclude_patterns: s.exclude_patterns.clone(),
        })
        .unwrap_or_default();
    let notifications = wire
        .notifications
        .as_ref()
        .map(|n| NotificationSettings {
            quiet_from: n.quiet_from.clone(),
            quiet_until: n.quiet_until.clone(),
            vip_breaks_quiet_hours: n.vip_breaks_quiet_hours,
        })
        .unwrap_or_default();
    let security = wire
        .security
        .as_ref()
        .map(|s| SecuritySettings {
            contain_agents: s.contain_agents,
        })
        .unwrap_or_default();
    FullSettings {
        default_agent: wire
            .default_agent
            .as_ref()
            .map(default_agent_of_wire)
            .unwrap_or_default(),
        terminal,
        branch_prefix: wire.branch_prefix.clone(),
        branch_prefix_is_default: wire.branch_prefix_is_default,
        retention_defaults: wire
            .retention_defaults
            .as_ref()
            .map(retention_of_wire)
            .unwrap_or_default(),
        long_lines: long_lines_of_wire(wire.long_lines),
        skills,
        notifications,
        security,
        use_worktrees: wire.use_worktrees,
        revision: wire.revision.as_ref().map_or(0, |r| r.value),
    }
}

fn update_settings_to_wire(patch: &SettingsPatch) -> pb::UpdateSettingsRequest {
    pb::UpdateSettingsRequest {
        idempotency_key: fresh_key(),
        default_agent: patch.default_agent.as_ref().map(default_agent_to_wire),
        terminal: patch.terminal.as_ref().map(|t| pb::TerminalSettings {
            font: t.font.clone(),
            font_size: f32::from(t.font_size),
            theme: t.theme.clone(),
            scrollback_lines: t.scrollback_lines,
        }),
        branch_prefix: patch.branch_prefix.clone(),
        retention_defaults: patch.retention_defaults.map(retention_to_wire),
        long_lines: patch.long_lines.map(long_lines_to_wire),
        skills: patch.skills.as_ref().map(|s| pb::SkillsSettings {
            auto_sync: s.auto_sync,
            opted_out: s
                .opted_out
                .iter()
                .map(|m| pb::MachineId {
                    value: m.as_str().to_owned(),
                })
                .collect(),
            exclude_patterns: s.exclude_patterns.clone(),
        }),
        notifications: patch.notifications.as_ref().map(|n| pb::NotificationSettings {
            quiet_from: n.quiet_from.clone(),
            quiet_until: n.quiet_until.clone(),
            vip_breaks_quiet_hours: n.vip_breaks_quiet_hours,
        }),
        security: patch.security.map(|s| pb::SecuritySettings {
            contain_agents: s.contain_agents,
        }),
        use_worktrees: patch.use_worktrees,
        expected_revision: patch
            .expected_revision
            .map(|value| pb::Revision { value }),
        ..Default::default()
    }
}

fn channel_settings_of_wire(
    wire: &pb::ChannelSettings,
    fallback_channel: ChannelId,
) -> ChannelSettings {
    let channel = wire
        .channel
        .as_ref()
        .map(|c| c.value.clone())
        .filter(|v| !v.is_empty())
        .map_or(fallback_channel, ChannelId::new);
    ChannelSettings {
        channel,
        default_agent: wire.default_agent.as_ref().map(default_agent_of_wire),
        branch_prefix: wire.branch_prefix.clone(),
        retention: wire
            .retention
            .as_ref()
            .map(retention_of_wire)
            .unwrap_or_default(),
        contain_agents: wire.contain_agents,
        todo_automation: wire.todo_automation.as_ref().map(todo_of_wire),
        revision: wire.revision.as_ref().map_or(0, |r| r.value),
    }
}

fn update_channel_settings_to_wire(
    channel: &ChannelId,
    patch: &ChannelSettingsPatch,
) -> pb::UpdateChannelSettingsRequest {
    pb::UpdateChannelSettingsRequest {
        idempotency_key: fresh_key(),
        channel: Some(pb::ChannelId {
            value: channel.as_str().to_owned(),
        }),
        default_agent: patch.default_agent.as_ref().map(default_agent_to_wire),
        clear_default_agent: patch.clear_default_agent,
        branch_prefix: patch.branch_prefix.clone(),
        clear_branch_prefix: patch.clear_branch_prefix,
        retention: patch.retention.map(retention_to_wire),
        contain_agents: patch.contain_agents,
        clear_contain_agents: patch.clear_contain_agents,
        todo_automation: patch.todo_automation.as_ref().map(todo_to_wire),
        clear_todo_automation: patch.clear_todo_automation,
        expected_revision: patch
            .expected_revision
            .map(|value| pb::Revision { value }),
        presence: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn change_default_agent_preserves_existing_model_and_expected_revision() {
        let current = pb::Settings {
            default_agent: Some(pb::DefaultAgent {
                agent: Some(pb::AgentId {
                    value: "claude".into(),
                }),
                model: "claude-opus-4-6".into(),
            }),
            revision: Some(pb::Revision { value: 42 }),
            ..Default::default()
        };
        let req = build_default_agent_update(&current, "codex".into());
        let agent = req.default_agent.expect("default_agent set");
        assert_eq!(agent.agent.map(|a| a.value), Some("codex".into()));
        assert_eq!(agent.model, "claude-opus-4-6");
        assert_eq!(req.expected_revision, Some(pb::Revision { value: 42 }));
    }

    #[test]
    fn full_settings_and_channel_settings_round_trip_wire() {
        let wire = pb::Settings {
            default_agent: Some(pb::DefaultAgent {
                agent: Some(pb::AgentId { value: "codex".into() }),
                model: "o3".into(),
            }),
            terminal: Some(pb::TerminalSettings {
                font: "JetBrains Mono".into(),
                font_size: 14.0,
                theme: "dark".into(),
                scrollback_lines: 25_000,
            }),
            branch_prefix: "alice/".into(),
            branch_prefix_is_default: true,
            retention_defaults: Some(pb::RetentionRules {
                archive_pr_reviews_after_days: 14,
                archive_idle_sessions_after_days: 30,
            }),
            long_lines: pb::LongLines::Scroll as i32,
            skills: Some(pb::SkillsSettings {
                auto_sync: true,
                opted_out: vec![pb::MachineId { value: "devbox".into() }],
                exclude_patterns: vec!["*.tmp".into()],
            }),
            notifications: Some(pb::NotificationSettings {
                quiet_from: "22:00".into(),
                quiet_until: "07:30".into(),
                vip_breaks_quiet_hours: true,
            }),
            security: Some(pb::SecuritySettings { contain_agents: true }),
            use_worktrees: true,
            revision: Some(pb::Revision { value: 9 }),
            ..Default::default()
        };
        let parsed = full_settings_of_wire(&wire);
        assert_eq!(parsed.branch_prefix, "alice/");
        assert!(parsed.branch_prefix_is_default);
        assert_eq!(parsed.default_agent.agent, "codex");
        assert_eq!(parsed.default_agent.model, "o3");
        assert_eq!(parsed.terminal.font, "JetBrains Mono");
        assert_eq!(parsed.terminal.font_size, 14);
        assert_eq!(parsed.terminal.theme, "dark");
        assert_eq!(parsed.terminal.scrollback_lines, 25_000);
        assert_eq!(parsed.long_lines, LongLinesMode::Scroll);
        assert_eq!(parsed.retention_defaults.archive_pr_reviews_after_days, 14);
        assert_eq!(parsed.retention_defaults.archive_idle_sessions_after_days, 30);
        assert!(parsed.skills.auto_sync);
        assert_eq!(parsed.notifications.quiet_from, "22:00");
        assert_eq!(parsed.notifications.quiet_until, "07:30");
        assert!(parsed.notifications.vip_breaks_quiet_hours);
        assert!(parsed.security.contain_agents);
        assert!(parsed.use_worktrees);
        assert_eq!(parsed.revision, 9);

        let patch = SettingsPatch {
            branch_prefix: Some("team/".into()),
            use_worktrees: Some(false),
            expected_revision: Some(9),
            ..Default::default()
        };
        let req = update_settings_to_wire(&patch);
        assert_eq!(req.branch_prefix.as_deref(), Some("team/"));
        assert_eq!(req.use_worktrees, Some(false));
        assert_eq!(req.expected_revision, Some(pb::Revision { value: 9 }));

        let ch_id = ChannelId::new("eng");
        let ch_patch = ChannelSettingsPatch {
            clear_branch_prefix: true,
            contain_agents: Some(true),
            expected_revision: Some(3),
            ..Default::default()
        };
        let ch_req = update_channel_settings_to_wire(&ch_id, &ch_patch);
        assert_eq!(ch_req.channel.map(|c| c.value), Some("eng".into()));
        assert!(ch_req.clear_branch_prefix);
        assert_eq!(ch_req.contain_agents, Some(true));
        assert_eq!(ch_req.expected_revision, Some(pb::Revision { value: 3 }));
    }

    #[test]
    fn the_section_survives_the_wire() {
        let mut sync = SyncSettings { mcp: true, skills_mode: SyncSkillsMode::HostOnly, ..SyncSettings::default() };
        sync.machines.insert(
            "devbox".to_owned(),
            MachineSyncSettings { enabled: Some(false), skills_mode: Some(SyncSkillsMode::HostWins), ..Default::default() },
        );
        assert_eq!(sync_of_wire(&sync_to_wire(&sync)), sync);
    }

    #[test]
    fn the_todo_section_survives_the_wire() {
        let section = TodoAutomation {
            on_pr_merged: TodoMode::Auto,
            on_todo_closed_archive_sessions: TodoMode::Auto,
            auto_wake_on_ci_fail: true,
            auto_wake_on_review_comments: true,
            idle_nudge_days: 14,
        };
        assert_eq!(todo_of_wire(&todo_to_wire(&section)), section);
    }

    #[test]
    fn an_unset_todo_section_reads_as_ask_ask_off_off_three_days() {
        let unset = pb::Settings { todo_automation: Some(pb::TodoAutomationSettings::default()), ..Default::default() };
        assert_eq!(todo_of_settings(&unset), TodoAutomation::default());
        assert_eq!(todo_of_settings(&pb::Settings::default()), TodoAutomation::default());
    }

    #[test]
    fn a_daemon_that_sends_no_section_gives_the_defaults() {
        let snapshot = snapshot_of(&pb::Settings::default());
        assert_eq!(snapshot.sync, SyncSettings::default());
        assert!(!snapshot.fusefs_enabled);
        assert_eq!(snapshot.fusefs_status, FuseMountStatus::Off);
    }

    #[test]
    fn the_mount_state_reads_from_the_wire() {
        use pb::fuse_mount_status::State;
        let wire = |state: State, root: &str, reason: &str| pb::Settings {
            fusefs_status: Some(pb::FuseMountStatus {
                state: state as i32,
                mount_root: root.to_owned(),
                reason: reason.to_owned(),
            }),
            ..Default::default()
        };
        assert_eq!(snapshot_of(&wire(State::Starting, "", "")).fusefs_status, FuseMountStatus::Starting);
        assert_eq!(
            snapshot_of(&wire(State::Mounted, "/h/Humata", "")).fusefs_status,
            FuseMountStatus::Mounted { path: "/h/Humata".to_owned() }
        );
        assert_eq!(
            snapshot_of(&wire(State::CannotSetUp, "", "No FUSE.")).fusefs_status,
            FuseMountStatus::CannotSetUp { reason: "No FUSE.".to_owned() }
        );
    }

    #[test]
    fn detected_browsers_and_bridge_status_round_trip_wire() {
        for kind in [
            BrowserKind::Chrome,
            BrowserKind::Brave,
            BrowserKind::Edge,
            BrowserKind::Chromium,
            BrowserKind::Arc,
            BrowserKind::Vivaldi,
            BrowserKind::Firefox,
        ] {
            assert_eq!(browser_kind_of_wire(browser_kind_to_wire(kind)), Some(kind));
        }
        let wire = pb::DetectedBrowser {
            kind: pb::BrowserKind::Firefox as i32,
            family: pb::BrowserFamily::Firefox as i32,
            display_name: "Firefox".into(),
            binary_path: "/Applications/Firefox.app".into(),
            native_messaging_dir: "/tmp/nm".into(),
            manifest_path: "/tmp/nm/app.humata2.browser_host.json".into(),
            host_installed: true,
            host_matches_current: true,
            connected: false,
            extension_path: "apps/humata2/browser_extension/firefox".into(),
        };
        let mapped = detected_browser_of_wire(&wire).expect("mapped");
        assert_eq!(mapped.kind, BrowserKind::Firefox);
        assert_eq!(mapped.family, BrowserFamily::Firefox);
        assert!(mapped.host_installed && mapped.host_matches_current && !mapped.connected);

        let status = bridge_status_of_wire(&pb::GetBrowserBridgeStatusResponse {
            connected: true,
            hosts: vec![pb::ConnectedBrowserHost {
                extension_origin: "chrome-extension://abc/".into(),
                host_pid: 99,
                family: pb::BrowserFamily::Chromium as i32,
            }],
        });
        assert!(status.connected);
        assert_eq!(status.hosts.len(), 1);
        assert_eq!(status.hosts[0].family, BrowserFamily::Chromium);
    }
}
