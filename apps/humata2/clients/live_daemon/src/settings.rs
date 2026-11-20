//! The agent settings the daemon's SettingsService keeps: the default agent
//! and whether Codex's hooks need trusting. Where each agent asks is its
//! switch (D782, `agent_switches`).

use humata2_clients::DaemonError;
use humata2_clients::daemon::{
    FuseMountStatus, MachineSyncSettings, SyncChange, SyncSettings, SyncSkillsMode, SyncSnapshot, TodoAutomation,
    TodoMode,
};
use humata2_clients::model::Agent;
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

    /// Makes `agent` Settings' default agent (ST-15).
    pub(crate) async fn change_default_agent(&self, agent: Agent) -> Result<(), DaemonError> {
        const WHAT: &str = "Changing the default agent";
        self.settings()
            .update_settings(pb::UpdateSettingsRequest {
                idempotency_key: fresh_key(),
                default_agent: Some(pb::DefaultAgent {
                    agent: Some(pb::AgentId { value: self.agents.name(agent) }),
                    model: String::new(),
                }),
                ..Default::default()
            })
            .await
            .map_err(|status| self.error(&status, WHAT))?;
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
