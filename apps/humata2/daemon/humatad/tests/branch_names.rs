//! Branch names from the work (flow 26.19; FC-11..FC-17, ST-13; D196),
//! through a real daemon: the prefix set in Settings and per channel, the
//! numbered name for a taken one, a branch edited before the start, and
//! "Rename the branch too" only while the branch was never pushed.

use humatad_test_common as common;

use std::path::{Path, PathBuf};

use common::{Daemon, create_channel, error_of, git, machine_id, shell_token, start_request, start_world, wait_status};
use humata2_grpc::humata::v1 as pb;
use tonic::Code;
use tonic::transport::Channel;

type Status = pb::SessionStatus;

async fn settings(daemon: &Daemon) -> pb::settings_service_client::SettingsServiceClient<Channel> {
    pb::settings_service_client::SettingsServiceClient::new(daemon.channel().await)
}

async fn set_prefix(daemon: &Daemon, prefix: &str) -> pb::Settings {
    settings(daemon)
        .await
        .update_settings(pb::UpdateSettingsRequest { branch_prefix: Some(prefix.to_owned()), ..Default::default() })
        .await
        .expect("the user sets the branch prefix")
        .into_inner()
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

/// Starts `title` with `branch` typed in the dialog (empty: Humata's pick).
async fn start(daemon: &Daemon, channel: &pb::Channel, title: &str, branch: &str) -> Result<pb::SessionId, tonic::Status> {
    let mut request = start_request(channel.id.as_ref().unwrap(), &machine_id(daemon).await, title);
    request.branch = branch.to_owned();
    let response = daemon.session_service().await.start_session(request).await?.into_inner();
    let id = response.session.unwrap();
    wait_status(daemon, &id, Status::Working).await;
    Ok(id)
}

fn head_of(clone: &Path) -> String {
    git(&["-C", &clone.display().to_string(), "rev-parse", "--abbrev-ref", "HEAD"])
}

fn has_branch(clone: &Path, branch: &str) -> bool {
    !git(&["-C", &clone.display().to_string(), "branch", "--list", branch]).is_empty()
}

fn clone_of(session: &pb::Session) -> PathBuf {
    PathBuf::from(&session.repos[0].path)
}

async fn rename(daemon: &Daemon, id: &pb::SessionId, title: &str, rename_branch: bool) -> Result<pb::Session, tonic::Status> {
    daemon
        .session_service()
        .await
        .rename_session(pb::RenameSessionRequest {
            id: Some(id.clone()),
            title: title.to_owned(),
            rename_branch,
            ..Default::default()
        })
        .await
        .map(tonic::Response::into_inner)
}

async fn session(daemon: &Daemon, id: &pb::SessionId) -> pb::Session {
    common::get_session(daemon, id).await
}

/// Flow 26.19 steps 1–3 and 5: the prefix from Settings, the slug of the
/// title, `-2` for the same title again, and no id or machine in the name.
#[tokio::test(flavor = "multi_thread")]
async fn the_prefix_from_settings_names_the_branch_and_the_same_title_again_gets_2() {
    let (daemon, world) = start_world().await;
    world.remote("acme", "api");
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    let shown = set_prefix(&daemon, "uhvesta/").await;
    assert_eq!(shown.branch_prefix, "uhvesta/");
    assert!(!shown.branch_prefix_is_default);
    assert_eq!(settings(&daemon).await.get_settings(pb::GetSettingsRequest {}).await.unwrap().into_inner().branch_prefix, "uhvesta/");

    assert_eq!(preview(&daemon, &channel, "Fix refund rounding").await, ["uhvesta/fix-refund-rounding"]);
    let first = start(&daemon, &channel, "Fix refund rounding", "").await.unwrap();
    let first = session(&daemon, &first).await;
    assert_eq!(first.repos[0].branch, "uhvesta/fix-refund-rounding");
    assert_eq!(head_of(&clone_of(&first)), "uhvesta/fix-refund-rounding");

    assert_eq!(preview(&daemon, &channel, "Fix refund rounding").await, ["uhvesta/fix-refund-rounding-2"]);
    let second = start(&daemon, &channel, "Fix refund rounding", "").await.unwrap();
    let second = session(&daemon, &second).await;
    assert_eq!(head_of(&clone_of(&second)), "uhvesta/fix-refund-rounding-2");

    // FC-16: nothing internal in the name.
    let machine = machine_id(&daemon).await.value;
    for shown in [&first, &second] {
        let branch = &shown.repos[0].branch;
        let id = &shown.id.as_ref().unwrap().value;
        assert!(!branch.contains(id.as_str()) && !branch.contains(machine.as_str()), "{branch}");
    }
}

/// FC-12, ST-13: a channel's own prefix wins, even an empty one, and
/// clearing it goes back to the one in Settings.
#[tokio::test(flavor = "multi_thread")]
async fn a_channels_own_prefix_wins_even_when_empty_and_clearing_it_goes_back_to_settings() {
    let (daemon, world) = start_world().await;
    world.remote("acme", "api");
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    set_prefix(&daemon, "uhvesta/").await;

    let own = settings(&daemon)
        .await
        .update_channel_settings(pb::UpdateChannelSettingsRequest {
            channel: channel.id.clone(),
            branch_prefix: Some(String::new()),
            ..Default::default()
        })
        .await
        .expect("the user sets the channel's prefix")
        .into_inner();
    assert_eq!(own.branch_prefix.as_deref(), Some(""));
    assert_eq!(preview(&daemon, &channel, "Fix refund rounding").await, ["fix-refund-rounding"]);

    let cleared = settings(&daemon)
        .await
        .update_channel_settings(pb::UpdateChannelSettingsRequest {
            channel: channel.id.clone(),
            clear_branch_prefix: true,
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(cleared.branch_prefix, None);
    assert_eq!(preview(&daemon, &channel, "Fix refund rounding").await, ["uhvesta/fix-refund-rounding"]);
}

/// The prefix lives in the store: a restart keeps it.
#[tokio::test(flavor = "multi_thread")]
async fn the_prefix_survives_a_restart() {
    let (mut daemon, world) = start_world().await;
    world.remote("acme", "api");
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    set_prefix(&daemon, "uhvesta/").await;
    daemon.restart().await;
    assert_eq!(preview(&daemon, &channel, "Fix refund rounding").await, ["uhvesta/fix-refund-rounding"]);
}

/// FC-15, D196: a branch typed in the dialog is used as typed; a taken one
/// is refused with the free name to use, and nothing starts.
#[tokio::test(flavor = "multi_thread")]
async fn an_edited_branch_is_used_as_typed_and_a_taken_one_is_refused() {
    let (daemon, world) = start_world().await;
    world.remote("acme", "api");
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    set_prefix(&daemon, "uhvesta/").await;

    let id = start(&daemon, &channel, "Fix refund rounding", "team/refunds").await.unwrap();
    let started = session(&daemon, &id).await;
    assert_eq!(started.repos[0].branch, "team/refunds");
    assert_eq!(head_of(&clone_of(&started)), "team/refunds");

    let refused = start(&daemon, &channel, "Another try", "team/refunds").await.expect_err("a taken branch is refused");
    match error_of(&refused).kind {
        Some(pb::error::Kind::NameTaken(taken)) => assert!(!taken.suggestion.is_empty() && taken.suggestion != "team/refunds"),
        other => panic!("expected name_taken, got {other:?}"),
    }
    let bad = start(&daemon, &channel, "Bad name", "no..dots").await.expect_err("a bad ref name is refused");
    assert_eq!(bad.code(), Code::InvalidArgument);

    let listed = daemon.session_service().await.list_sessions(pb::ListSessionsRequest::default()).await.unwrap().into_inner().sessions;
    assert_eq!(listed.len(), 1, "only the first session started");
}

/// Flow 26.19 step 4, FC-17: renaming with "Rename the branch too" before
/// any push renames the branch from the new title; the folder stays.
#[tokio::test(flavor = "multi_thread")]
async fn renaming_with_rename_the_branch_too_renames_a_branch_never_pushed() {
    let (daemon, world) = start_world().await;
    world.remote("acme", "api");
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    set_prefix(&daemon, "uhvesta/").await;
    let id = start(&daemon, &channel, "Fix refund rounding", "").await.unwrap();
    let before = session(&daemon, &id).await;
    let clone = clone_of(&before);

    let renamed = rename(&daemon, &id, "Round refunds half up", true).await.expect("an unpushed branch is renamed");
    assert_eq!(renamed.title, "Round refunds half up");
    assert_eq!(renamed.repos[0].branch, "uhvesta/round-refunds-half-up");
    assert_eq!(clone_of(&renamed), clone, "the folder keeps its name");
    assert_eq!(head_of(&clone), "uhvesta/round-refunds-half-up");
    assert!(!has_branch(&clone, "uhvesta/fix-refund-rounding"), "the old branch is gone");
}

/// FC-17: a plain rename never touches the branch, and once the branch is
/// pushed "Rename the branch too" is refused and changes nothing.
#[tokio::test(flavor = "multi_thread")]
async fn a_pushed_branch_keeps_its_name_and_the_tick_is_refused_changing_nothing() {
    let (daemon, world) = start_world().await;
    world.remote("acme", "api");
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    set_prefix(&daemon, "uhvesta/").await;
    let id = start(&daemon, &channel, "Fix refund rounding", "").await.unwrap();
    let clone = clone_of(&session(&daemon, &id).await);

    let plain = rename(&daemon, &id, "Refund rounding", false).await.unwrap();
    assert_eq!(plain.title, "Refund rounding");
    assert_eq!(head_of(&clone), "uhvesta/fix-refund-rounding");

    git(&["-C", &clone.display().to_string(), "push", "-u", "origin", "uhvesta/fix-refund-rounding"]);
    let refused = rename(&daemon, &id, "Round refunds half up", true).await.expect_err("a pushed branch isn't renamed");
    assert_eq!(refused.code(), Code::FailedPrecondition);
    let after = session(&daemon, &id).await;
    assert_eq!(after.title, "Refund rounding", "nothing changed, the title included");
    assert_eq!(after.repos[0].branch, "uhvesta/fix-refund-rounding");
    assert_eq!(head_of(&clone), "uhvesta/fix-refund-rounding");
}

/// SETTINGSFIX Slice 3: an agent proposing a branch prefix change gets an
/// Inbox approval card (AskNoAllow) rather than a hard `user_only` refusal;
/// nothing changes until the user approves it.
#[tokio::test(flavor = "multi_thread")]
async fn an_agent_cannot_change_the_branch_prefix() {
    let (daemon, world) = start_world().await;
    world.remote("acme", "api");
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    let id = start(&daemon, &channel, "Fix refund rounding", "").await.unwrap();
    let (token, _shell) = shell_token(&daemon, &id).await;

    let mut request = tonic::Request::new(pb::UpdateSettingsRequest { branch_prefix: Some("agent/".to_owned()), ..Default::default() });
    request.metadata_mut().insert("humata-session-token", tonic::metadata::MetadataValue::try_from(token.as_str()).unwrap());
    let pending = settings(&daemon).await.update_settings(request).await.expect_err("an agent's Settings edit waits in the Inbox");
    assert!(matches!(error_of(&pending).kind, Some(pb::error::Kind::ApprovalPending(_))));
    assert_eq!(preview(&daemon, &channel, "Next").await, ["next"]);
}
