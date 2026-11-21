//! Settings by dotted key (CLI.md §5.7): `terminal.font`,
//! `retention.archive_idle_sessions_after_days` and so on, the field names
//! `--json` prints. A change reads the settings, changes one field and
//! writes its part back against the revision it read.

use humata2_proto::humata::v1 as pb;
use serde_json::Value as Json;

use super::parse;
use crate::call::{Ctx, Done};
use crate::console::Console;
use crate::daemon::{Daemon, rpc};
use crate::error::CliError;

const GLOBAL_KEYS: &str = "one of default_agent.agent, default_agent.model, terminal.font, \
     terminal.font_size, terminal.theme, terminal.scrollback_lines, branch_prefix, \
     retention_defaults.archive_pr_reviews_after_days, \
     retention_defaults.archive_idle_sessions_after_days, long_lines, skills.auto_sync, \
     skills.exclude_patterns, notifications.quiet_from, notifications.quiet_until, \
     notifications.vip_breaks_quiet_hours, security.contain_agents, \
     todo_automation.on_pr_merged, todo_automation.on_todo_closed_archive_sessions, \
     todo_automation.auto_wake_on_ci_fail, todo_automation.auto_wake_on_review_comments, \
     todo_automation.idle_nudge_days, sync.sync_enabled, sync.skills, sync.instructions, \
     sync.mcp, sync.settings, sync.skills_mode, fusefs_enabled";

const CHANNEL_KEYS: &str = "one of default_agent.agent, default_agent.model, branch_prefix, \
     retention.archive_pr_reviews_after_days, retention.archive_idle_sessions_after_days, \
     contain_agents, todo_automation.on_pr_merged, \
     todo_automation.on_todo_closed_archive_sessions, todo_automation.auto_wake_on_ci_fail, \
     todo_automation.auto_wake_on_review_comments, todo_automation.idle_nudge_days";

/// The value the channel's own setting is dropped with, so the global one
/// applies again.
const INHERIT: &str = "inherit";

/// One dotted key of a reply, or all of it.
fn pick(all: Json, key: Option<String>, keys: &'static str) -> Result<Json, CliError> {
    let Some(key) = key else { return Ok(all) };
    let mut at = &all;
    for part in key.split('.') {
        match at.get(part) {
            Some(next) => at = next,
            None => {
                return Err(CliError::BadValue {
                    field: "key",
                    value: key,
                    expected: keys,
                });
            }
        }
    }
    Ok(at.clone())
}

fn unknown(key: &str, keys: &'static str) -> CliError {
    CliError::BadValue {
        field: "key",
        value: key.into(),
        expected: keys,
    }
}

fn list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect()
}

pub(super) async fn get<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    key: Option<String>,
) -> Result<Done, CliError> {
    let all = ctx
        .json(rpc::settings::GET_SETTINGS, &pb::GetSettingsRequest {})
        .await?;
    pick(all, key, GLOBAL_KEYS).map(Done::reply)
}

fn parse_todo_mode(field: &'static str, value: &str) -> Result<i32, CliError> {
    match value {
        "ask" => Ok(pb::TodoSyncMode::Ask as i32),
        "auto" => Ok(pb::TodoSyncMode::Auto as i32),
        _ => Err(CliError::BadValue {
            field,
            value: value.into(),
            expected: "ask or auto",
        }),
    }
}

fn parse_sync_skills_mode(value: &str) -> Result<i32, CliError> {
    match value {
        "host_wins" => Ok(pb::SyncSkillsMode::HostWins as i32),
        "host_only" => Ok(pb::SyncSkillsMode::HostOnly as i32),
        _ => Err(CliError::BadValue {
            field: "sync.skills_mode",
            value: value.into(),
            expected: "host_wins or host_only",
        }),
    }
}

/// The update for one global key, with only that key's part set.
fn global_update(
    current: pb::Settings,
    key: &str,
    value: &str,
) -> Result<pb::UpdateSettingsRequest, CliError> {
    let mut update = pb::UpdateSettingsRequest {
        expected_revision: current.revision,
        ..Default::default()
    };
    let agent = current.default_agent.unwrap_or_default();
    let terminal = current.terminal.unwrap_or_default();
    let retention = current.retention_defaults.unwrap_or_default();
    let skills = current.skills.unwrap_or_default();
    let notifications = current.notifications.unwrap_or_default();
    let todo = current.todo_automation.unwrap_or_default();
    let sync = current.sync.unwrap_or_default();
    match key {
        "default_agent.agent" => {
            update.default_agent = Some(pb::DefaultAgent {
                agent: Some(pb::AgentId {
                    value: value.into(),
                }),
                ..agent
            });
        }
        "default_agent.model" => {
            update.default_agent = Some(pb::DefaultAgent {
                model: value.into(),
                ..agent
            });
        }
        "terminal.font" => {
            update.terminal = Some(pb::TerminalSettings {
                font: value.into(),
                ..terminal
            });
        }
        "terminal.font_size" => {
            update.terminal = Some(pb::TerminalSettings {
                font_size: parse::number("terminal.font_size", value)?,
                ..terminal
            });
        }
        "terminal.theme" => {
            update.terminal = Some(pb::TerminalSettings {
                theme: value.into(),
                ..terminal
            });
        }
        "terminal.scrollback_lines" => {
            update.terminal = Some(pb::TerminalSettings {
                scrollback_lines: parse::number("terminal.scrollback_lines", value)?,
                ..terminal
            });
        }
        "branch_prefix" => update.branch_prefix = Some(value.into()),
        "retention_defaults.archive_pr_reviews_after_days" => {
            update.retention_defaults = Some(pb::RetentionRules {
                archive_pr_reviews_after_days: parse::number(
                    "retention_defaults.archive_pr_reviews_after_days",
                    value,
                )?,
                ..retention
            });
        }
        "retention_defaults.archive_idle_sessions_after_days" => {
            update.retention_defaults = Some(pb::RetentionRules {
                archive_idle_sessions_after_days: parse::number(
                    "retention_defaults.archive_idle_sessions_after_days",
                    value,
                )?,
                ..retention
            });
        }
        "long_lines" => {
            let lines = match value {
                "wrap" => pb::LongLines::Wrap,
                "scroll" => pb::LongLines::Scroll,
                _ => {
                    return Err(CliError::BadValue {
                        field: "long_lines",
                        value: value.into(),
                        expected: "wrap or scroll",
                    });
                }
            };
            update.long_lines = Some(lines as i32);
        }
        "skills.auto_sync" => {
            update.skills = Some(pb::SkillsSettings {
                auto_sync: parse::on_off("skills.auto_sync", value)?,
                ..skills
            });
        }
        "skills.exclude_patterns" => {
            update.skills = Some(pb::SkillsSettings {
                exclude_patterns: list(value),
                ..skills
            });
        }
        "notifications.quiet_from" => {
            update.notifications = Some(pb::NotificationSettings {
                quiet_from: value.into(),
                ..notifications
            });
        }
        "notifications.quiet_until" => {
            update.notifications = Some(pb::NotificationSettings {
                quiet_until: value.into(),
                ..notifications
            });
        }
        "notifications.vip_breaks_quiet_hours" => {
            update.notifications = Some(pb::NotificationSettings {
                vip_breaks_quiet_hours: parse::on_off(
                    "notifications.vip_breaks_quiet_hours",
                    value,
                )?,
                ..notifications
            });
        }
        "security.contain_agents" => {
            update.security = Some(pb::SecuritySettings {
                contain_agents: parse::on_off("security.contain_agents", value)?,
            });
        }
        "todo_automation.on_pr_merged" => {
            update.todo_automation = Some(pb::TodoAutomationSettings {
                on_pr_merged: parse_todo_mode("todo_automation.on_pr_merged", value)?,
                ..todo
            });
        }
        "todo_automation.on_todo_closed_archive_sessions" => {
            update.todo_automation = Some(pb::TodoAutomationSettings {
                on_todo_closed_archive_sessions: parse_todo_mode(
                    "todo_automation.on_todo_closed_archive_sessions",
                    value,
                )?,
                ..todo
            });
        }
        "todo_automation.auto_wake_on_ci_fail" => {
            update.todo_automation = Some(pb::TodoAutomationSettings {
                auto_wake_on_ci_fail: parse::on_off(
                    "todo_automation.auto_wake_on_ci_fail",
                    value,
                )?,
                ..todo
            });
        }
        "todo_automation.auto_wake_on_review_comments" => {
            update.todo_automation = Some(pb::TodoAutomationSettings {
                auto_wake_on_review_comments: parse::on_off(
                    "todo_automation.auto_wake_on_review_comments",
                    value,
                )?,
                ..todo
            });
        }
        "todo_automation.idle_nudge_days" => {
            update.todo_automation = Some(pb::TodoAutomationSettings {
                idle_nudge_days: parse::number("todo_automation.idle_nudge_days", value)?,
                ..todo
            });
        }
        "sync.sync_enabled" => {
            update.sync = Some(pb::SyncSettings {
                sync_enabled: parse::on_off("sync.sync_enabled", value)?,
                ..sync
            });
        }
        "sync.skills" => {
            update.sync = Some(pb::SyncSettings {
                skills: parse::on_off("sync.skills", value)?,
                ..sync
            });
        }
        "sync.instructions" => {
            update.sync = Some(pb::SyncSettings {
                instructions: parse::on_off("sync.instructions", value)?,
                ..sync
            });
        }
        "sync.mcp" => {
            update.sync = Some(pb::SyncSettings {
                mcp: parse::on_off("sync.mcp", value)?,
                ..sync
            });
        }
        "sync.settings" => {
            update.sync = Some(pb::SyncSettings {
                settings: parse::on_off("sync.settings", value)?,
                ..sync
            });
        }
        "sync.skills_mode" => {
            update.sync = Some(pb::SyncSettings {
                skills_mode: parse_sync_skills_mode(value)?,
                ..sync
            });
        }
        "fusefs_enabled" => {
            update.fusefs_enabled = Some(parse::on_off("fusefs_enabled", value)?);
        }
        _ => return Err(unknown(key, GLOBAL_KEYS)),
    }
    Ok(update)
}

pub(super) async fn set<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    key: &str,
    value: &str,
) -> Result<Done, CliError> {
    let current: pb::Settings = ctx
        .call(rpc::settings::GET_SETTINGS, &pb::GetSettingsRequest {})
        .await?;
    let update = pb::UpdateSettingsRequest {
        idempotency_key: ctx.key(),
        ..global_update(current, key, value)?
    };
    ctx.change_present(rpc::settings::UPDATE_SETTINGS, |presence| {
        pb::UpdateSettingsRequest {
            presence,
            ..update.clone()
        }
    }).await
}

pub(super) async fn channel_get<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    channel: pb::ChannelId,
    key: Option<String>,
) -> Result<Done, CliError> {
    let request = pb::GetChannelSettingsRequest {
        channel: Some(channel),
    };
    let all = ctx
        .json(rpc::settings::GET_CHANNEL_SETTINGS, &request)
        .await?;
    pick(all, key, CHANNEL_KEYS).map(Done::reply)
}

/// The update for one channel key; `inherit` drops the channel's own value.
fn channel_update(
    current: pb::ChannelSettings,
    key: &str,
    value: &str,
) -> Result<pb::UpdateChannelSettingsRequest, CliError> {
    let mut update = pb::UpdateChannelSettingsRequest {
        channel: current.channel,
        expected_revision: current.revision,
        ..Default::default()
    };
    let agent = current.default_agent.unwrap_or_default();
    let retention = current.retention.unwrap_or_default();
    let todo = current.todo_automation.unwrap_or_default();
    let inherit = value == INHERIT;
    match key {
        "default_agent.agent" | "default_agent.model" if inherit => {
            update.clear_default_agent = true;
        }
        "default_agent.agent" => {
            update.default_agent = Some(pb::DefaultAgent {
                agent: Some(pb::AgentId {
                    value: value.into(),
                }),
                ..agent
            });
        }
        "default_agent.model" => {
            update.default_agent = Some(pb::DefaultAgent {
                model: value.into(),
                ..agent
            });
        }
        "branch_prefix" if inherit => update.clear_branch_prefix = true,
        "branch_prefix" => update.branch_prefix = Some(value.into()),
        "retention.archive_pr_reviews_after_days" => {
            update.retention = Some(pb::RetentionRules {
                archive_pr_reviews_after_days: parse::number(
                    "retention.archive_pr_reviews_after_days",
                    value,
                )?,
                ..retention
            });
        }
        "retention.archive_idle_sessions_after_days" => {
            update.retention = Some(pb::RetentionRules {
                archive_idle_sessions_after_days: parse::number(
                    "retention.archive_idle_sessions_after_days",
                    value,
                )?,
                ..retention
            });
        }
        "contain_agents" if inherit => update.clear_contain_agents = true,
        "contain_agents" => {
            update.contain_agents = Some(parse::on_off("contain_agents", value)?);
        }
        "todo_automation.on_pr_merged"
        | "todo_automation.on_todo_closed_archive_sessions"
        | "todo_automation.auto_wake_on_ci_fail"
        | "todo_automation.auto_wake_on_review_comments"
        | "todo_automation.idle_nudge_days"
            if inherit =>
        {
            update.clear_todo_automation = true;
        }
        "todo_automation.on_pr_merged" => {
            update.todo_automation = Some(pb::TodoAutomationSettings {
                on_pr_merged: parse_todo_mode("todo_automation.on_pr_merged", value)?,
                ..todo
            });
        }
        "todo_automation.on_todo_closed_archive_sessions" => {
            update.todo_automation = Some(pb::TodoAutomationSettings {
                on_todo_closed_archive_sessions: parse_todo_mode(
                    "todo_automation.on_todo_closed_archive_sessions",
                    value,
                )?,
                ..todo
            });
        }
        "todo_automation.auto_wake_on_ci_fail" => {
            update.todo_automation = Some(pb::TodoAutomationSettings {
                auto_wake_on_ci_fail: parse::on_off(
                    "todo_automation.auto_wake_on_ci_fail",
                    value,
                )?,
                ..todo
            });
        }
        "todo_automation.auto_wake_on_review_comments" => {
            update.todo_automation = Some(pb::TodoAutomationSettings {
                auto_wake_on_review_comments: parse::on_off(
                    "todo_automation.auto_wake_on_review_comments",
                    value,
                )?,
                ..todo
            });
        }
        "todo_automation.idle_nudge_days" => {
            update.todo_automation = Some(pb::TodoAutomationSettings {
                idle_nudge_days: parse::number("todo_automation.idle_nudge_days", value)?,
                ..todo
            });
        }
        _ => return Err(unknown(key, CHANNEL_KEYS)),
    }
    Ok(update)
}

pub(super) async fn channel_set<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    channel: pb::ChannelId,
    key: &str,
    value: &str,
) -> Result<Done, CliError> {
    let request = pb::GetChannelSettingsRequest {
        channel: Some(channel.clone()),
    };
    let mut current: pb::ChannelSettings = ctx
        .call(rpc::settings::GET_CHANNEL_SETTINGS, &request)
        .await?;
    current.channel = Some(channel);
    let update = pb::UpdateChannelSettingsRequest {
        idempotency_key: ctx.key(),
        ..channel_update(current, key, value)?
    };
    ctx.change_present(rpc::settings::UPDATE_CHANNEL_SETTINGS, |presence| {
        pb::UpdateChannelSettingsRequest {
            presence,
            ..update.clone()
        }
    }).await
}

/// `humata settings contain on|off`: globally, or for `--channel`, where
/// `--inherit` drops the channel's own choice.
pub(super) async fn contain<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    channel: Option<pb::ChannelId>,
    on: bool,
    inherit: bool,
) -> Result<Done, CliError> {
    let value = match (inherit, on) {
        (true, _) => INHERIT,
        (false, true) => "on",
        (false, false) => "off",
    };
    match channel {
        Some(channel) => channel_set(ctx, channel, "contain_agents", value).await,
        None => set(ctx, "security.contain_agents", value).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_global_change_keeps_the_rest_of_its_part() {
        let current = pb::Settings {
            terminal: Some(pb::TerminalSettings {
                font: "Menlo".into(),
                font_size: 13.0,
                ..Default::default()
            }),
            revision: Some(pb::Revision { value: 7 }),
            ..Default::default()
        };
        let update = global_update(current, "terminal.font_size", "15").unwrap();
        let terminal = update.terminal.unwrap();
        assert_eq!(terminal.font, "Menlo");
        assert_eq!(terminal.font_size, 15.0);
        assert_eq!(update.expected_revision, Some(pb::Revision { value: 7 }));
        assert!(update.default_agent.is_none());
    }

    #[test]
    fn inherit_clears_the_channel_value() {
        let update =
            channel_update(pb::ChannelSettings::default(), "branch_prefix", INHERIT).unwrap();
        assert!(update.clear_branch_prefix);
        assert_eq!(update.branch_prefix, None);
    }

    #[test]
    fn an_unknown_key_is_refused() {
        let refused = global_update(pb::Settings::default(), "terminal.colour", "red");
        assert!(matches!(refused, Err(CliError::BadValue { field: "key", .. })));
    }

    #[test]
    fn a_missing_key_is_refused_when_reading() {
        let all = serde_json::json!({ "terminal": { "font": "Menlo" } });
        assert_eq!(
            pick(all.clone(), Some("terminal.font".into()), GLOBAL_KEYS).unwrap(),
            serde_json::json!("Menlo")
        );
        assert!(pick(all, Some("terminal.nope".into()), GLOBAL_KEYS).is_err());
    }

    #[test]
    fn global_todo_sync_and_fusefs_keys_update_their_sections() {
        let current = pb::Settings {
            todo_automation: Some(pb::TodoAutomationSettings {
                on_pr_merged: pb::TodoSyncMode::Ask as i32,
                idle_nudge_days: 3,
                ..Default::default()
            }),
            sync: Some(pb::SyncSettings {
                sync_enabled: true,
                skills: true,
                ..Default::default()
            }),
            ..Default::default()
        };
        let todo_u = global_update(current.clone(), "todo_automation.on_pr_merged", "auto").unwrap();
        assert_eq!(
            todo_u.todo_automation.as_ref().map(|t| t.on_pr_merged),
            Some(pb::TodoSyncMode::Auto as i32)
        );
        assert_eq!(todo_u.todo_automation.as_ref().map(|t| t.idle_nudge_days), Some(3));

        let sync_u = global_update(current.clone(), "sync.skills_mode", "host_only").unwrap();
        assert_eq!(
            sync_u.sync.as_ref().map(|s| s.skills_mode),
            Some(pb::SyncSkillsMode::HostOnly as i32)
        );
        assert_eq!(sync_u.sync.as_ref().map(|s| s.sync_enabled), Some(true));

        let fuse_u = global_update(current, "fusefs_enabled", "on").unwrap();
        assert_eq!(fuse_u.fusefs_enabled, Some(true));
    }

    #[test]
    fn channel_todo_automation_supports_override_and_inherit() {
        let current = pb::ChannelSettings::default();
        let set = channel_update(current.clone(), "todo_automation.idle_nudge_days", "7").unwrap();
        assert_eq!(set.todo_automation.as_ref().map(|t| t.idle_nudge_days), Some(7));
        assert!(!set.clear_todo_automation);

        let cleared = channel_update(current, "todo_automation.on_pr_merged", INHERIT).unwrap();
        assert!(cleared.clear_todo_automation);
        assert!(cleared.todo_automation.is_none());
    }
}
