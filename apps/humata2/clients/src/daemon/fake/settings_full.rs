//! In-memory fake for full global `Settings` and per-channel `ChannelSettings`.

use super::FakeDaemon;
use crate::daemon::{
    ChannelSettings, ChannelSettingsPatch, DaemonError, Event, FullSettings, Operation, Reply,
    SettingsPatch, valid_branch_prefix,
};
use crate::model::ChannelId;

pub(super) fn full_settings(daemon: &FakeDaemon) -> Reply<FullSettings> {
    daemon.reply(None, "full_settings".to_owned(), |inner| {
        Ok(inner.full_settings.clone())
    })
}

pub(super) fn update_settings(daemon: &FakeDaemon, patch: SettingsPatch) -> Reply<FullSettings> {
    daemon.reply(
        Some(Operation::UpdateSettings),
        format!("update_settings {patch:?}"),
        move |inner| {
            if let Some(expected) = patch.expected_revision
                && expected != inner.full_settings.revision
            {
                return Err(DaemonError::Conflict {
                    what: "Settings".to_owned(),
                    reason: "the settings changed since you opened them, so nothing was saved"
                        .to_owned(),
                });
            }
            if let Some(prefix) = &patch.branch_prefix
                && !valid_branch_prefix(prefix)
            {
                return Err(DaemonError::Refused {
                    what: "branch_prefix".to_owned(),
                    reason: "that prefix can't begin a branch name".to_owned(),
                });
            }
            if let Some(agent) = patch.default_agent {
                inner.full_settings.default_agent = agent;
            }
            if let Some(terminal) = patch.terminal {
                inner.full_settings.terminal = terminal;
            }
            if let Some(prefix) = patch.branch_prefix {
                inner.full_settings.branch_prefix = prefix;
                inner.full_settings.branch_prefix_is_default = false;
            }
            if let Some(retention) = patch.retention_defaults {
                inner.full_settings.retention_defaults = retention;
            }
            if let Some(long_lines) = patch.long_lines {
                inner.full_settings.long_lines = long_lines;
            }
            if let Some(skills) = patch.skills {
                inner.full_settings.skills = skills;
            }
            if let Some(notifications) = patch.notifications {
                inner.full_settings.notifications = notifications;
            }
            if let Some(security) = patch.security {
                inner.full_settings.security = security;
            }
            if let Some(use_worktrees) = patch.use_worktrees {
                inner.full_settings.use_worktrees = use_worktrees;
            }
            inner.full_settings.revision += 1;
            let saved = inner.full_settings.clone();
            inner.emit(Event::SettingsChanged);
            Ok(saved)
        },
    )
}

pub(super) fn refresh_gh_login(daemon: &FakeDaemon) -> Reply<FullSettings> {
    daemon.reply(
        Some(Operation::RefreshGhLogin),
        "refresh_gh_login".to_owned(),
        |inner| {
            if inner.full_settings.branch_prefix_is_default {
                inner.full_settings.branch_prefix = inner.gh_default_prefix.clone();
            }
            let shown = inner.full_settings.clone();
            inner.emit(Event::SettingsChanged);
            Ok(shown)
        },
    )
}

pub(super) fn channel_settings(daemon: &FakeDaemon, channel: &ChannelId) -> Reply<ChannelSettings> {
    let id = channel.clone();
    daemon.reply(None, format!("channel_settings {channel}"), move |inner| {
        if inner.workspace.channel(&id).is_none() {
            return Err(DaemonError::NotFound {
                what: format!("channel {id}"),
            });
        }
        Ok(inner
            .channel_settings
            .get(&id)
            .cloned()
            .unwrap_or_else(|| ChannelSettings {
                channel: id,
                default_agent: None,
                branch_prefix: None,
                retention: inner.full_settings.retention_defaults,
                contain_agents: None,
                todo_automation: None,
                revision: 0,
            }))
    })
}

pub(super) fn update_channel_settings(
    daemon: &FakeDaemon,
    channel: &ChannelId,
    patch: ChannelSettingsPatch,
) -> Reply<ChannelSettings> {
    let id = channel.clone();
    daemon.reply(
        Some(Operation::UpdateChannelSettings),
        format!("update_channel_settings {channel} {patch:?}"),
        move |inner| {
            if inner.workspace.channel(&id).is_none() {
                return Err(DaemonError::NotFound {
                    what: format!("channel {id}"),
                });
            }
            if let Some(prefix) = &patch.branch_prefix
                && !patch.clear_branch_prefix
                && !valid_branch_prefix(prefix)
            {
                return Err(DaemonError::Refused {
                    what: "branch_prefix".to_owned(),
                    reason: "that prefix can't begin a branch name".to_owned(),
                });
            }
            let fallback_retention = inner.full_settings.retention_defaults;
            let entry = inner
                .channel_settings
                .entry(id.clone())
                .or_insert_with(|| ChannelSettings {
                    channel: id.clone(),
                    default_agent: None,
                    branch_prefix: None,
                    retention: fallback_retention,
                    contain_agents: None,
                    todo_automation: None,
                    revision: 0,
                });
            if let Some(expected) = patch.expected_revision
                && expected != entry.revision
            {
                return Err(DaemonError::Conflict {
                    what: "Channel settings".to_owned(),
                    reason: "the channel's settings changed since you opened them, so nothing was saved"
                        .to_owned(),
                });
            }
            if patch.clear_default_agent {
                entry.default_agent = None;
            } else if let Some(agent) = patch.default_agent {
                entry.default_agent = Some(agent);
            }
            if patch.clear_branch_prefix {
                entry.branch_prefix = None;
            } else if let Some(prefix) = patch.branch_prefix {
                entry.branch_prefix = Some(prefix);
            }
            if let Some(retention) = patch.retention {
                entry.retention = retention;
            }
            if patch.clear_contain_agents {
                entry.contain_agents = None;
            } else if let Some(contain) = patch.contain_agents {
                entry.contain_agents = Some(contain);
            }
            if patch.clear_todo_automation {
                entry.todo_automation = None;
            } else if let Some(todo) = patch.todo_automation {
                entry.todo_automation = Some(todo);
            }
            entry.revision += 1;
            let saved = entry.clone();
            Ok(saved)
        },
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use tca::TestClock;

    use super::*;
    use crate::daemon::{Daemon, DefaultAgentSetting, RetentionDefaults};
    use crate::fixture;

    fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime")
            .block_on(future)
    }

    fn daemon() -> FakeDaemon {
        FakeDaemon::new(fixture::design(), Arc::new(TestClock::new()))
    }

    #[test]
    fn updating_global_branch_prefix_and_refreshing_gh_login() {
        let daemon = daemon();
        daemon.set_gh_default_prefix("octocat/");
        let refreshed = block_on(daemon.refresh_gh_login()).expect("refresh");
        assert_eq!(refreshed.branch_prefix, "octocat/");
        assert!(refreshed.branch_prefix_is_default);

        let updated = block_on(daemon.update_settings(SettingsPatch {
            branch_prefix: Some("team/".to_owned()),
            expected_revision: Some(refreshed.revision),
            ..Default::default()
        }))
        .expect("update");
        assert_eq!(updated.branch_prefix, "team/");
        assert!(!updated.branch_prefix_is_default);
        assert_eq!(updated.revision, refreshed.revision + 1);

        // Stale revision is rejected.
        let conflict = block_on(daemon.update_settings(SettingsPatch {
            branch_prefix: Some("other/".to_owned()),
            expected_revision: Some(refreshed.revision),
            ..Default::default()
        }));
        assert!(matches!(conflict, Err(DaemonError::Conflict { .. })));
    }

    #[test]
    fn channel_settings_override_and_clear_to_inherit() {
        let daemon = daemon();
        let ch = daemon.workspace().channels[0].id.clone();

        let initial = block_on(daemon.channel_settings(&ch)).expect("read");
        assert_eq!(initial.branch_prefix, None);

        let saved = block_on(daemon.update_channel_settings(
            &ch,
            ChannelSettingsPatch {
                branch_prefix: Some("ops/".to_owned()),
                default_agent: Some(DefaultAgentSetting {
                    agent: "codex".to_owned(),
                    model: "o3".to_owned(),
                }),
                contain_agents: Some(true),
                retention: Some(RetentionDefaults {
                    archive_pr_reviews_after_days: 7,
                    archive_idle_sessions_after_days: 14,
                }),
                expected_revision: Some(initial.revision),
                ..Default::default()
            },
        ))
        .expect("override");
        assert_eq!(saved.branch_prefix.as_deref(), Some("ops/"));
        assert_eq!(
            saved.default_agent.as_ref().map(|d| d.model.as_str()),
            Some("o3")
        );
        assert_eq!(saved.contain_agents, Some(true));
        assert_eq!(saved.retention.archive_pr_reviews_after_days, 7);

        let cleared = block_on(daemon.update_channel_settings(
            &ch,
            ChannelSettingsPatch {
                clear_branch_prefix: true,
                clear_default_agent: true,
                clear_contain_agents: true,
                expected_revision: Some(saved.revision),
                ..Default::default()
            },
        ))
        .expect("clear");
        assert_eq!(cleared.branch_prefix, None);
        assert_eq!(cleared.default_agent, None);
        assert_eq!(cleared.contain_agents, None);
    }
}
