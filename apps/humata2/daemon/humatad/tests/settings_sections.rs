//! The Settings sections beside the branch prefix, against the real daemon:
//! default agent (ST-15), terminal (ST-3), retention defaults (ST-14,
//! LD-16), long lines (ST-7), skills (ST-16) and notifications (ST-19,
//! NV-23), globally and per channel. Each reads its default until set, a
//! set section is kept across restarts in the humata home's
//! `config/settings.toml`, an update changes only its sections, a bad value
//! saves nothing, and a start that names no agent takes the channel's
//! default agent, or else the one in Settings.
//!
//! Waits are on the start's operation; nothing sleeps.

use humatad_test_common as common;

use common::{Build, Daemon, World, create_channel, folder, get_session, invalid_field};
use humata2_grpc::humata::v1 as pb;
use tonic::Code;
use tonic::transport::Channel;

/// The second test agent, the same script as the first under another name.
const SECOND: &str = "second";

/// A daemon whose world has two agents: `fake` (the daemon's default) and `second`.
async fn with_two_agents() -> (Daemon, World) {
    let dir = folder();
    let world = World::new(dir.path());
    let script = dir.path().join("agent.sh");
    let agent = |name: &str| {
        serde_json::json!({
            "name": name,
            "program": "/bin/sh",
            "start_args": [script, "--session-id", "{conversation}"],
            "resume_args": [script, "--resume", "{conversation}"],
            "conversation_file": format!("{}/{{conversation}}.jsonl", world.conv.display()),
        })
    };
    let agents = serde_json::json!([agent(common::AGENT_NAME), agent(SECOND)]);
    std::fs::write(&world.agents, agents.to_string()).unwrap();
    let args = world.args();
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let daemon = Daemon::start_with(Build::Admin, dir, &refs).await;
    (daemon, world)
}

async fn settings(daemon: &Daemon) -> pb::settings_service_client::SettingsServiceClient<Channel> {
    pb::settings_service_client::SettingsServiceClient::new(daemon.channel().await)
}

async fn get(daemon: &Daemon) -> pb::Settings {
    settings(daemon).await.get_settings(pb::GetSettingsRequest {}).await.unwrap().into_inner()
}

async fn update(daemon: &Daemon, request: pb::UpdateSettingsRequest) -> Result<pb::Settings, tonic::Status> {
    settings(daemon).await.update_settings(request).await.map(tonic::Response::into_inner)
}

async fn get_channel(daemon: &Daemon, channel: &pb::Channel) -> pb::ChannelSettings {
    settings(daemon)
        .await
        .get_channel_settings(pb::GetChannelSettingsRequest { channel: channel.id.clone() })
        .await
        .unwrap()
        .into_inner()
}

async fn update_channel(
    daemon: &Daemon,
    request: pb::UpdateChannelSettingsRequest,
) -> Result<pb::ChannelSettings, tonic::Status> {
    settings(daemon).await.update_channel_settings(request).await.map(tonic::Response::into_inner)
}

fn agent(name: &str) -> Option<pb::DefaultAgent> {
    Some(pb::DefaultAgent {
        agent: Some(pb::AgentId { value: name.to_owned() }),
        model: String::new(),
    })
}

fn every_section() -> pb::UpdateSettingsRequest {
    pb::UpdateSettingsRequest {
        default_agent: agent(SECOND),
        terminal: Some(pb::TerminalSettings {
            font: "Menlo".to_owned(),
            font_size: 14.0,
            theme: "dark".to_owned(),
            scrollback_lines: 20_000,
        }),
        retention_defaults: Some(pb::RetentionRules {
            archive_pr_reviews_after_days: 7,
            archive_idle_sessions_after_days: 30,
        }),
        long_lines: Some(pb::LongLines::Scroll as i32),
        skills: Some(pb::SkillsSettings {
            auto_sync: false,
            opted_out: vec![pb::MachineId { value: "box-1".to_owned() }],
            exclude_patterns: vec!["*.bak".to_owned()],
        }),
        notifications: Some(pb::NotificationSettings {
            quiet_from: "22:00".to_owned(),
            quiet_until: "07:30".to_owned(),
            vip_breaks_quiet_hours: true,
        }),
        ..Default::default()
    }
}

/// The sections this file is about, without the branch prefix, which
/// follows the host's `gh` login on its own.
fn sections(shown: &pb::Settings) -> impl PartialEq + std::fmt::Debug {
    (
        shown.default_agent.clone(),
        shown.terminal.clone(),
        shown.retention_defaults,
        shown.long_lines,
        shown.skills.clone(),
        shown.notifications.clone(),
        shown.revision,
    )
}

fn assert_every_section_set(shown: &pb::Settings) {
    assert_eq!(shown.default_agent, agent(SECOND));
    assert_eq!(every_section().terminal, shown.terminal);
    assert_eq!(every_section().retention_defaults, shown.retention_defaults);
    assert_eq!(shown.long_lines(), pb::LongLines::Scroll);
    assert_eq!(every_section().skills, shown.skills);
    assert_eq!(every_section().notifications, shown.notifications);
}

/// ST-7, ST-14, ST-16, NV-23: what Settings shows before anything is set.
#[tokio::test(flavor = "multi_thread")]
async fn every_section_reads_its_default_until_set() {
    let (daemon, _world) = with_two_agents().await;

    let shown = get(&daemon).await;

    assert_eq!(shown.long_lines(), pb::LongLines::Wrap);
    let retention = shown.retention_defaults.expect("retention defaults are shown");
    assert_eq!((retention.archive_pr_reviews_after_days, retention.archive_idle_sessions_after_days), (0, 0));
    assert!(shown.skills.expect("skills are shown").auto_sync);
    let notifications = shown.notifications.expect("notifications are shown");
    assert!(!notifications.vip_breaks_quiet_hours);
    assert!(notifications.quiet_from.is_empty() && notifications.quiet_until.is_empty());
    let terminal = shown.terminal.expect("the terminal is shown");
    assert!(terminal.theme.is_empty(), "the theme follows the system by default");
    assert!(terminal.scrollback_lines > 0);
    assert!(shown.default_agent.is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn every_section_is_saved_and_kept_across_a_restart() {
    let (mut daemon, _world) = with_two_agents().await;
    let before = get(&daemon).await.revision.unwrap().value;

    let saved = update(&daemon, every_section()).await.unwrap();
    assert_every_section_set(&saved);
    assert!(saved.revision.unwrap().value > before);
    assert_every_section_set(&get(&daemon).await);
    assert!(daemon.dir.path().join("config/settings.toml").exists());

    daemon.restart().await;
    let after = get(&daemon).await;
    assert_every_section_set(&after);
    assert_eq!(after.revision, saved.revision);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_update_changes_only_the_sections_it_sets() {
    let (daemon, _world) = with_two_agents().await;
    update(&daemon, every_section()).await.unwrap();

    let changed = update(
        &daemon,
        pb::UpdateSettingsRequest {
            long_lines: Some(pb::LongLines::Wrap as i32),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    assert_eq!(changed.long_lines(), pb::LongLines::Wrap);
    assert_eq!(changed.terminal, every_section().terminal);
    assert_eq!(changed.notifications, every_section().notifications);
    assert_eq!(changed.default_agent, agent(SECOND));
}

/// Portable config: the humata home's `config/settings.toml` is read as the
/// user left it, and the sections it doesn't name keep their defaults.
#[tokio::test(flavor = "multi_thread")]
async fn a_settings_file_written_by_hand_is_read() {
    let dir = folder();
    std::fs::create_dir_all(dir.path().join("config")).unwrap();
    std::fs::write(dir.path().join("config/settings.toml"), "long_lines = \"scroll\"\n").unwrap();
    let daemon = Daemon::start_in(dir).await;

    let shown = get(&daemon).await;

    assert_eq!(shown.long_lines(), pb::LongLines::Scroll);
    assert!(shown.skills.unwrap().auto_sync);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bad_value_is_refused_and_nothing_in_the_request_is_saved() {
    let (daemon, _world) = with_two_agents().await;
    let before = get(&daemon).await;

    let refusals = [
        ("default_agent.agent", pb::UpdateSettingsRequest { default_agent: agent("nobody"), ..every_section() }),
        (
            "notifications.quiet_from",
            pb::UpdateSettingsRequest {
                notifications: Some(pb::NotificationSettings {
                    quiet_from: "25:00".to_owned(),
                    quiet_until: "07:00".to_owned(),
                    vip_breaks_quiet_hours: false,
                }),
                ..every_section()
            },
        ),
        (
            "notifications.quiet_until",
            pb::UpdateSettingsRequest {
                notifications: Some(pb::NotificationSettings {
                    quiet_from: "22:00".to_owned(),
                    quiet_until: String::new(),
                    vip_breaks_quiet_hours: false,
                }),
                ..every_section()
            },
        ),
        ("long_lines", pb::UpdateSettingsRequest { long_lines: Some(pb::LongLines::Unspecified as i32), ..every_section() }),
        (
            "terminal.font_size",
            pb::UpdateSettingsRequest {
                terminal: Some(pb::TerminalSettings { font_size: -1.0, ..every_section().terminal.unwrap() }),
                ..every_section()
            },
        ),
        (
            "skills.exclude_patterns",
            pb::UpdateSettingsRequest {
                skills: Some(pb::SkillsSettings { exclude_patterns: vec!["  ".to_owned()], ..every_section().skills.unwrap() }),
                ..every_section()
            },
        ),
    ];
    for (field, request) in refusals {
        let refused = update(&daemon, request).await.unwrap_err();
        assert_eq!(invalid_field(&refused).0, field, "{refused:?}");
    }

    assert_eq!(sections(&get(&daemon).await), sections(&before));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_writer_that_read_an_old_revision_is_refused() {
    let (daemon, _world) = with_two_agents().await;
    let old = get(&daemon).await.revision;
    update(&daemon, pb::UpdateSettingsRequest { long_lines: Some(pb::LongLines::Scroll as i32), ..Default::default() })
        .await
        .unwrap();

    let stale = update(
        &daemon,
        pb::UpdateSettingsRequest {
            terminal: every_section().terminal,
            expected_revision: old,
            ..Default::default()
        },
    )
    .await
    .unwrap_err();

    assert_eq!(stale.code(), Code::FailedPrecondition);
    assert_ne!(get(&daemon).await.terminal, every_section().terminal);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_channel_keeps_its_own_default_agent_and_retention_until_cleared() {
    let (mut daemon, world) = with_two_agents().await;
    world.remote("acme", "api");
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    update(&daemon, every_section()).await.unwrap();

    // Unset, the channel follows Settings.
    let inherited = get_channel(&daemon, &channel).await;
    assert_eq!(inherited.default_agent, None);
    assert_eq!(inherited.retention, every_section().retention_defaults);

    let own_retention = Some(pb::RetentionRules {
        archive_pr_reviews_after_days: 3,
        archive_idle_sessions_after_days: 0,
    });
    let set = update_channel(
        &daemon,
        pb::UpdateChannelSettingsRequest {
            channel: channel.id.clone(),
            default_agent: agent(common::AGENT_NAME),
            retention: own_retention,
            expected_revision: inherited.revision,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(set.default_agent, agent(common::AGENT_NAME));
    assert_eq!(set.retention, own_retention);
    assert!(set.revision.unwrap().value > inherited.revision.unwrap().value);

    daemon.restart().await;
    let kept = get_channel(&daemon, &channel).await;
    assert_eq!(kept.default_agent, agent(common::AGENT_NAME));
    assert_eq!(kept.retention, own_retention);

    let cleared = update_channel(
        &daemon,
        pb::UpdateChannelSettingsRequest {
            channel: channel.id.clone(),
            clear_default_agent: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(cleared.default_agent, None);
    assert_eq!(cleared.retention, own_retention);

    let unknown = update_channel(
        &daemon,
        pb::UpdateChannelSettingsRequest {
            channel: channel.id.clone(),
            default_agent: agent("nobody"),
            ..Default::default()
        },
    )
    .await
    .unwrap_err();
    assert_eq!(invalid_field(&unknown).0, "default_agent.agent");
}

/// ST-15: a start that names no agent takes the channel's own default
/// agent, else the one in Settings.
#[tokio::test(flavor = "multi_thread")]
async fn a_start_that_names_no_agent_takes_the_default_agent() {
    let (daemon, world) = with_two_agents().await;
    world.remote("acme", "api");
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;

    update(&daemon, pb::UpdateSettingsRequest { default_agent: agent(SECOND), ..Default::default() })
        .await
        .unwrap();
    let (from_settings, _) = common::start_session(&daemon, &channel, "From settings").await;
    let agent_of = |session: pb::Session| session.agent.map(|agent| agent.value);
    assert_eq!(agent_of(get_session(&daemon, &from_settings).await).as_deref(), Some(SECOND));

    update_channel(
        &daemon,
        pb::UpdateChannelSettingsRequest {
            channel: channel.id.clone(),
            default_agent: agent(common::AGENT_NAME),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let (from_channel, _) = common::start_session(&daemon, &channel, "From the channel").await;
    assert_eq!(agent_of(get_session(&daemon, &from_channel).await).as_deref(), Some(common::AGENT_NAME));
}

/// D770: the agent a channel shows as its default is the one a start that
/// names none takes, from the channel's own, else Settings', else the one it
/// was made with, so the app's dialogs preselect what will start.
#[tokio::test(flavor = "multi_thread")]
async fn a_channel_shows_the_agent_a_start_would_take() {
    let (daemon, world) = with_two_agents().await;
    world.remote("acme", "api");
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    async fn shown(daemon: &Daemon, id: Option<pb::ChannelId>) -> Option<String> {
        daemon
            .channel_service()
            .await
            .get_channel(pb::GetChannelRequest { id })
            .await
            .unwrap()
            .into_inner()
            .default_agent
            .map(|agent| agent.value)
    }
    let made_with = shown(&daemon, channel.id.clone()).await;
    assert_eq!(made_with.as_deref(), Some(common::AGENT_NAME));

    update(&daemon, pb::UpdateSettingsRequest { default_agent: agent(SECOND), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(shown(&daemon, channel.id.clone()).await.as_deref(), Some(SECOND));

    update_channel(
        &daemon,
        pb::UpdateChannelSettingsRequest {
            channel: channel.id.clone(),
            default_agent: agent(common::AGENT_NAME),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(shown(&daemon, channel.id.clone()).await.as_deref(), Some(common::AGENT_NAME));
}

fn todo(on_merge: pb::TodoSyncMode, wake: bool, nudge: u32) -> pb::TodoAutomationSettings {
    pb::TodoAutomationSettings {
        on_pr_merged: on_merge as i32,
        on_todo_closed_archive_sessions: pb::TodoSyncMode::Auto as i32,
        auto_wake_on_ci_fail: wake,
        auto_wake_on_review_comments: !wake,
        idle_nudge_days: nudge,
    }
}

/// T7: the To-do automation settings read Ask until set, round-trip, and a
/// channel's own override wins over Settings until it is cleared.
#[tokio::test(flavor = "multi_thread")]
async fn todo_automation_is_kept_globally_and_per_channel() {
    let (mut daemon, world) = with_two_agents().await;
    world.remote("acme", "api");
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;

    let unset = get(&daemon).await.todo_automation.expect("todo automation is shown");
    assert_eq!(unset.on_pr_merged(), pb::TodoSyncMode::Ask);
    assert_eq!(unset.on_todo_closed_archive_sessions(), pb::TodoSyncMode::Ask);
    assert!(!unset.auto_wake_on_ci_fail && !unset.auto_wake_on_review_comments);
    assert_eq!(unset.idle_nudge_days, 0);
    assert_eq!(get_channel(&daemon, &channel).await.todo_automation, None);

    let global = todo(pb::TodoSyncMode::Auto, true, 5);
    let shown = update(&daemon, pb::UpdateSettingsRequest { todo_automation: Some(global.clone()), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(shown.todo_automation, Some(global.clone()));
    daemon.restart().await;
    assert_eq!(get(&daemon).await.todo_automation, Some(global.clone()));
    // No override: the channel shows none and follows Settings.
    assert_eq!(get_channel(&daemon, &channel).await.todo_automation, None);

    // Unspecified reads Ask.
    let own = pb::TodoAutomationSettings { on_pr_merged: pb::TodoSyncMode::Unspecified as i32, ..todo(pb::TodoSyncMode::Ask, false, 9) };
    let set = update_channel(
        &daemon,
        pb::UpdateChannelSettingsRequest { channel: channel.id.clone(), todo_automation: Some(own), ..Default::default() },
    )
    .await
    .unwrap();
    let kept = set.todo_automation.expect("the channel's own settings are shown");
    assert_eq!(kept.on_pr_merged(), pb::TodoSyncMode::Ask);
    assert_eq!((kept.auto_wake_on_review_comments, kept.idle_nudge_days), (true, 9));
    assert_eq!(get(&daemon).await.todo_automation, Some(global.clone()), "Settings are unchanged");

    let cleared = update_channel(
        &daemon,
        pb::UpdateChannelSettingsRequest { channel: channel.id.clone(), clear_todo_automation: true, ..Default::default() },
    )
    .await
    .unwrap();
    assert_eq!(cleared.todo_automation, None);

    let too_long = update(
        &daemon,
        pb::UpdateSettingsRequest { todo_automation: Some(todo(pb::TodoSyncMode::Ask, false, 99_999)), ..Default::default() },
    )
    .await
    .unwrap_err();
    assert_eq!(too_long.code(), Code::InvalidArgument);
}
