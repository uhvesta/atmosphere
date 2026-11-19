//! The rest of flow 26.19 (D196): the default branch prefix is the host's
//! `gh` login plus "/", a Settings change is told on the Settings feed, and
//! a fork takes an edited branch as typed, as a start does.
//!
//! Waits are on the Settings feed or the fork's operation; nothing sleeps.

use humatad_test_common as common;

use std::path::{Path, PathBuf};

use common::{Build, Daemon, World, create_channel, error_of, finished, folder, get_session, git, machine_id, start_session, wait_status};
use humata2_grpc::humata::v1 as pb;
use tonic::Code;
use tonic::transport::Channel;

/// A signed-in `gh` whose login is `octo`, whichever way Humata asks.
const FAKE_GH: &str = r#"#!/bin/sh
case "$*" in
  "api user --jq .login"|"api user -q .login") echo "octo" ;;
  "api user") echo '{"login":"octo","id":1}' ;;
  "auth status"*) echo "github.com" ; echo "  ✓ Logged in to github.com account octo (keyring)" ;;
  *) echo "fake gh: unexpected $*" >&2; exit 1 ;;
esac
"#;

/// A signed-in `gh` whose login is whatever the `login` file beside it says.
const FILE_GH: &str = r#"#!/bin/sh
here=$(dirname "$0")
case "$*" in
  "api user --jq .login"|"api user -q .login") cat "$here/login" ;;
  *) echo "fake gh: unexpected $*" >&2; exit 1 ;;
esac
"#;

/// A daemon whose `gh` is the fake above.
async fn with_gh() -> (Daemon, World) {
    with_gh_script(FAKE_GH).await
}

async fn with_gh_script(script: &str) -> (Daemon, World) {
    let dir = folder();
    let world = World::new(dir.path());
    let gh_dir = dir.path().join("gh");
    std::fs::create_dir_all(&gh_dir).unwrap();
    let gh = gh_dir.join("gh");
    std::fs::write(gh_dir.join("login"), "octo\n").unwrap();
    std::fs::write(&gh, script).unwrap();
    std::fs::set_permissions(&gh, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let args = world.args();
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let gh_text = gh.display().to_string();
    let daemon = Daemon::start_with_env(Build::Admin, dir, &refs, &[("HUMATA2_GH", gh_text.as_str())]).await;
    (daemon, world)
}

async fn settings(daemon: &Daemon) -> pb::settings_service_client::SettingsServiceClient<Channel> {
    pb::settings_service_client::SettingsServiceClient::new(daemon.channel().await)
}

async fn settings_feed(daemon: &Daemon) -> tonic::Streaming<pb::Event> {
    pb::event_service_client::EventServiceClient::new(daemon.channel().await)
        .subscribe(pb::SubscribeRequest {
            topics: vec![pb::Topic::Settings as i32],
            ..Default::default()
        })
        .await
        .expect("the Settings feed opens")
        .into_inner()
}

/// The settings once `want` holds, read again on each Settings event.
async fn wait_settings(daemon: &Daemon, what: &str, want: impl Fn(&pb::Settings) -> bool) -> pb::Settings {
    let mut feed = settings_feed(daemon).await;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        let now = settings(daemon).await.get_settings(pb::GetSettingsRequest {}).await.unwrap().into_inner();
        if want(&now) {
            return now;
        }
        match tokio::time::timeout_at(deadline, feed.message()).await {
            Ok(Ok(Some(_))) => {}
            other => panic!("the settings were never {what}: {other:?}; last {now:?}"),
        }
    }
}

async fn preview(daemon: &Daemon, channel: &pb::Channel, title: &str) -> Vec<String> {
    daemon
        .session_service()
        .await
        .preview_branch_names(pb::PreviewBranchNamesRequest {
            channel: channel.id.clone(),
            title: title.to_owned(),
            machine: Some(machine_id(daemon).await),
            repos: Vec::new(),
        })
        .await
        .unwrap()
        .into_inner()
        .names
        .into_iter()
        .map(|name| name.branch)
        .collect()
}

fn head_of(clone: &Path) -> String {
    git(&["-C", &clone.display().to_string(), "rev-parse", "--abbrev-ref", "HEAD"])
}

fn clone_of(session: &pb::Session) -> PathBuf {
    PathBuf::from(&session.repos[0].path)
}

/// D196: with nothing set, the prefix is the gh login and "/", shown as the
/// default; setting one replaces it.
#[tokio::test(flavor = "multi_thread")]
async fn the_default_prefix_is_the_gh_login_and_a_set_prefix_replaces_it() {
    let (daemon, world) = with_gh().await;
    world.remote("acme", "api");
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;

    let shown = wait_settings(&daemon, "showing the gh login", |s| s.branch_prefix == "octo/").await;
    assert!(shown.branch_prefix_is_default);
    assert_eq!(preview(&daemon, &channel, "Cache warmup").await, ["octo/cache-warmup"]);

    let set = settings(&daemon)
        .await
        .update_settings(pb::UpdateSettingsRequest { branch_prefix: Some("mine/".to_owned()), ..Default::default() })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(set.branch_prefix, "mine/");
    assert!(!set.branch_prefix_is_default);
    assert_eq!(preview(&daemon, &channel, "Cache warmup").await, ["mine/cache-warmup"]);
}

/// ST-7, D196's owed event: a change is told on the Settings feed with the
/// settings as they are after it.
#[tokio::test(flavor = "multi_thread")]
async fn changing_the_prefix_is_told_on_the_settings_feed() {
    let (daemon, _world) = with_gh().await;
    let mut feed = settings_feed(&daemon).await;

    settings(&daemon)
        .await
        .update_settings(pb::UpdateSettingsRequest { branch_prefix: Some("mine/".to_owned()), ..Default::default() })
        .await
        .unwrap();

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        let event = tokio::time::timeout_at(deadline, feed.message())
            .await
            .expect("a Settings event within a minute")
            .unwrap()
            .expect("the feed stays open");
        if let Some(pb::event::Payload::Settings(settings)) = event.payload
            && settings.branch_prefix == "mine/"
        {
            assert!(!settings.branch_prefix_is_default);
            return;
        }
    }
}

fn fork_request(parent: &pb::SessionId, title: &str, branch: &str) -> pb::ForkSessionRequest {
    pb::ForkSessionRequest {
        parent: Some(parent.clone()),
        kind: pb::ForkKind::CodeAndContext as i32,
        title: title.to_owned(),
        branch: branch.to_owned(),
        ..Default::default()
    }
}

/// FC-15 for forks: an edited branch is used as typed, and a taken one is
/// refused with the free name suggested, before anything is copied.
#[tokio::test(flavor = "multi_thread")]
async fn a_fork_with_an_edited_branch_uses_it_as_typed_and_a_taken_one_is_refused() {
    let (daemon, world) = with_gh().await;
    world.remote("acme", "api");
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    let (parent, _) = start_session(&daemon, &channel, "Cache warmup").await;
    let parent_session = wait_status(&daemon, &parent, pb::SessionStatus::Working).await;
    let taken = parent_session.repos[0].branch.clone();

    let refused = daemon.session_service().await.fork_session(fork_request(&parent, "Try it", &taken)).await.unwrap_err();
    assert_eq!(refused.code(), Code::AlreadyExists, "{refused:?}");
    let Some(pb::error::Kind::NameTaken(name)) = error_of(&refused).kind else {
        panic!("a taken branch names itself: {refused:?}");
    };
    assert!(!name.suggestion.is_empty() && name.suggestion != taken);

    let bad = daemon.session_service().await.fork_session(fork_request(&parent, "Try it", "no..good")).await.unwrap_err();
    assert_eq!(bad.code(), Code::InvalidArgument, "{bad:?}");

    let started = daemon
        .session_service()
        .await
        .fork_session(fork_request(&parent, "Try it", "mine/try-it"))
        .await
        .unwrap()
        .into_inner();
    let ended = finished(&daemon, &started).await;
    let Some(pb::operation::Result::Session(fork)) = ended.result else {
        panic!("the fork made a session: {ended:?}");
    };
    let fork = get_session(&daemon, &fork).await;
    assert_eq!(fork.repos[0].branch, "mine/try-it");
    assert_eq!(head_of(&clone_of(&fork)), "mine/try-it");
    assert_eq!(head_of(&clone_of(&parent_session)), taken, "the parent keeps its branch");
}

/// The default follows a `gh` login that changes while the daemon runs:
/// reading the settings again, as the Settings page does when it opens,
/// shows the new login (no timer asks on its own).
#[tokio::test(flavor = "multi_thread")]
async fn a_changed_gh_login_shows_when_the_settings_are_read_again() {
    let (daemon, _world) = with_gh_script(FILE_GH).await;
    wait_settings(&daemon, "showing the first login", |s| s.branch_prefix == "octo/").await;

    std::fs::write(daemon.dir.path().join("gh").join("login"), "hubot\n").unwrap();

    let shown = wait_settings(&daemon, "showing the new login", |s| s.branch_prefix == "hubot/").await;
    assert!(shown.branch_prefix_is_default);
}
