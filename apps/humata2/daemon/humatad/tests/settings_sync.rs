//! FUSEFSWIRE (D895): Settings › Sync against the real daemon. The sync
//! section and `fusefs_enabled` are saved, shown again, kept across a
//! restart, told on the Settings feed, and left alone by an update that
//! names other sections. The mount is off until Settings turns it on.
//!
//! Waits are on the Settings feed; nothing sleeps.

use humatad_test_common as common;

use std::time::Duration;

use common::Daemon;
use humata2_grpc::humata::v1 as pb;
use tonic::transport::Channel;

async fn settings(daemon: &Daemon) -> pb::settings_service_client::SettingsServiceClient<Channel> {
    pb::settings_service_client::SettingsServiceClient::new(daemon.channel().await)
}

async fn get(daemon: &Daemon) -> pb::Settings {
    settings(daemon).await.get_settings(pb::GetSettingsRequest {}).await.unwrap().into_inner()
}

async fn update(daemon: &Daemon, request: pb::UpdateSettingsRequest) -> Result<pb::Settings, tonic::Status> {
    settings(daemon).await.update_settings(request).await.map(tonic::Response::into_inner)
}

fn wanted_sync() -> pb::SyncSettings {
    pb::SyncSettings {
        sync_enabled: false,
        skills: true,
        instructions: false,
        mcp: true,
        settings: false,
        skills_mode: pb::SyncSkillsMode::HostOnly as i32,
        machines: [(
            "box-1".to_owned(),
            pb::MachineSyncOverride {
                enabled: Some(true),
                mcp: Some(false),
                skills_mode: Some(pb::SyncSkillsMode::HostWins as i32),
                ..Default::default()
            },
        )]
        .into_iter()
        .collect(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_mount_is_off_and_sync_follows_the_defaults_until_set() {
    let daemon = Daemon::start().await;

    let shown = get(&daemon).await;

    assert!(!shown.fusefs_enabled);
    let sync = shown.sync.expect("sync is shown");
    assert!(sync.sync_enabled && sync.skills && sync.instructions);
    assert!(!sync.mcp && !sync.settings);
    assert!(sync.machines.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn sync_and_the_mount_switch_are_saved_and_kept_across_a_restart() {
    let mut daemon = Daemon::start().await;

    let saved = update(
        &daemon,
        pb::UpdateSettingsRequest { sync: Some(wanted_sync()), fusefs_enabled: Some(true), ..Default::default() },
    )
    .await
    .unwrap();

    assert_eq!(saved.sync, Some(wanted_sync()));
    assert!(saved.fusefs_enabled);
    assert_eq!(get(&daemon).await.sync, Some(wanted_sync()));
    assert!(daemon.dir.path().join("config/config-sync.toml").exists());

    daemon.restart().await;
    let after = get(&daemon).await;
    assert_eq!(after.sync, Some(wanted_sync()));
    assert!(after.fusefs_enabled);
    assert_eq!(after.revision, saved.revision);
}

#[tokio::test(flavor = "multi_thread")]
async fn updating_only_sync_leaves_the_other_sections() {
    let daemon = Daemon::start().await;
    update(
        &daemon,
        pb::UpdateSettingsRequest {
            long_lines: Some(pb::LongLines::Scroll as i32),
            fusefs_enabled: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let before = get(&daemon).await;

    let after = update(&daemon, pb::UpdateSettingsRequest { sync: Some(wanted_sync()), ..Default::default() })
        .await
        .unwrap();

    assert_eq!(after.long_lines(), pb::LongLines::Scroll);
    assert!(after.fusefs_enabled);
    assert_eq!(after.terminal, before.terminal);
    assert_eq!(after.skills, before.skills);
    assert_eq!(after.notifications, before.notifications);
    assert_eq!(after.retention_defaults, before.retention_defaults);
    assert!(after.revision.unwrap().value > before.revision.unwrap().value);
}

#[tokio::test(flavor = "multi_thread")]
async fn turning_the_mount_off_again_is_kept() {
    let daemon = Daemon::start().await;
    update(&daemon, pb::UpdateSettingsRequest { fusefs_enabled: Some(true), ..Default::default() })
        .await
        .unwrap();

    let off = update(&daemon, pb::UpdateSettingsRequest { fusefs_enabled: Some(false), ..Default::default() })
        .await
        .unwrap();

    assert!(!off.fusefs_enabled);
    assert!(!get(&daemon).await.fusefs_enabled);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sync_update_is_told_on_the_settings_feed() {
    let daemon = Daemon::start().await;
    let mut feed = pb::event_service_client::EventServiceClient::new(daemon.channel().await)
        .subscribe(pb::SubscribeRequest { topics: vec![pb::Topic::Settings as i32], ..Default::default() })
        .await
        .expect("the settings feed opens")
        .into_inner();

    update(&daemon, pb::UpdateSettingsRequest { sync: Some(wanted_sync()), ..Default::default() })
        .await
        .unwrap();

    let told = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let event = feed.message().await.unwrap().expect("the event feed stayed open");
            if let Some(pb::event::Payload::Settings(shown)) = event.payload {
                return shown;
            }
        }
    })
    .await
    .expect("the settings feed told the change");
    assert_eq!(told.sync, Some(wanted_sync()));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_skills_mode_saves_nothing() {
    let daemon = Daemon::start().await;
    let before = get(&daemon).await;

    let refused = update(
        &daemon,
        pb::UpdateSettingsRequest {
            sync: Some(pb::SyncSettings { skills_mode: 99, ..wanted_sync() }),
            ..Default::default()
        },
    )
    .await
    .unwrap_err();

    assert_eq!(refused.code(), tonic::Code::InvalidArgument);
    assert_eq!(get(&daemon).await.sync, before.sync);
    assert!(!daemon.dir.path().join("config/config-sync.toml").exists());
}
