//! MachineService.PlanChangeRoot and ChangeRoot (FC-10, PL-12): a machine's
//! root is read from the machine, a plan names the old and new root, and
//! applying it makes the new root the one new channels use. Through a real
//! `humatad2` with a fake box. The plan lists the channels on the machine and
//! the sessions running in them, This Mac's root can be read and changed like
//! a box's. Moving channels is refused while a session runs. On This Mac it
//! moves each channel with its files and sessions, and the new root is the one
//! every new channel uses, also after a restart. A box does the same on its own
//! daemon, which the host relays to: the plan and its token are the box's.

use humatad_test_common as common;
use humatad_test_common::remote_rig;

use std::path::PathBuf;

use humata2_grpc::humata::v1 as pb;
use remote_rig::{Rig, is_live};
use tonic::transport::Channel;
use tonic::{Code, Status};

const DEFAULT: &str = "~/Humata";

async fn client(rig: &Rig) -> pb::machine_service_client::MachineServiceClient<Channel> {
    pb::machine_service_client::MachineServiceClient::new(rig.channel().await)
}

async fn root_of(rig: &Rig) -> String {
    client(rig)
        .await
        .get_machine(pb::GetMachineRequest { id: Some(Rig::box_id("box-1")) })
        .await
        .expect("the machine is read")
        .into_inner()
        .machine
        .expect("it names the machine")
        .root
}

async fn plan(rig: &Rig, root: &str) -> Result<pb::ChangeRootPlan, Status> {
    client(rig)
        .await
        .plan_change_root(pb::PlanChangeRootRequest { id: Some(Rig::box_id("box-1")), root: root.to_owned() })
        .await
        .map(tonic::Response::into_inner)
}

async fn change(rig: &Rig, root: &str, plan: &pb::ChangeRootPlan, move_channels: bool) -> Result<pb::Operation, Status> {
    client(rig)
        .await
        .change_root(pb::ChangeRootRequest {
            id: Some(Rig::box_id("box-1")),
            root: root.to_owned(),
            plan: plan.token.clone(),
            move_channels,
            ..Default::default()
        })
        .await
        .map(tonic::Response::into_inner)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plan_names_the_old_and_new_root_and_changes_nothing() {
    let rig = Rig::start(&["box-1"], &[]).await;
    let planned = plan(&rig, "~/Work").await.expect("the change is planned");
    assert_eq!(planned.old_root, DEFAULT);
    assert_eq!(planned.new_root, "~/Work");
    assert!(planned.token.is_some());
    assert_eq!(root_of(&rig).await, DEFAULT);
}

#[tokio::test(flavor = "multi_thread")]
async fn applying_the_plan_keeps_the_new_root_across_reads() {
    let rig = Rig::start(&["box-1"], &[]).await;
    let planned = plan(&rig, "~/Work").await.expect("planned");
    let started = change(&rig, "~/Work", &planned, false).await.expect("the change starts");
    let ended = rig.ended(&started).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    assert_eq!(root_of(&rig).await, "~/Work");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plan_works_once_and_only_for_the_root_it_was_made_for() {
    let rig = Rig::start(&["box-1"], &[]).await;
    let planned = plan(&rig, "~/Work").await.expect("planned");
    let other = change(&rig, "~/Else", &planned, false).await.unwrap_err();
    assert_eq!(other.code(), Code::FailedPrecondition);
    change(&rig, "~/Work", &planned, false).await.expect("the plan still serves its own root");
    let again = change(&rig, "~/Work", &planned, false).await.unwrap_err();
    assert_eq!(again.code(), Code::FailedPrecondition);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_root_that_is_not_an_absolute_or_home_path_is_refused() {
    let rig = Rig::start(&["box-1"], &[]).await;
    for bad in ["Work", "", "/", "~", "~/a\nb"] {
        let refused = plan(&rig, bad).await.unwrap_err();
        assert_eq!(refused.code(), Code::InvalidArgument, "{bad:?}");
    }
    assert_eq!(root_of(&rig).await, DEFAULT);
}

#[tokio::test(flavor = "multi_thread")]
async fn moving_a_box_with_no_channels_changes_its_root() {
    let rig = Rig::start(&["box-1"], &[]).await;
    let planned = plan(&rig, "~/Work").await.expect("planned");
    let started = change(&rig, "~/Work", &planned, true).await.expect("the move starts");
    let ended = rig.ended(&started).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    assert_eq!(root_of(&rig).await, "~/Work");
}

fn id_of(name: &str) -> Option<pb::MachineId> {
    Some(pb::MachineId { value: name.to_owned() })
}

async fn this_mac_root(rig: &Rig, id: &pb::MachineId) -> String {
    client(rig)
        .await
        .get_machine(pb::GetMachineRequest { id: Some(id.clone()) })
        .await
        .expect("This Mac is a machine whose root can be read")
        .into_inner()
        .machine
        .expect("it names the machine")
        .root
}

async fn plan_of(rig: &Rig, id: &pb::MachineId, root: &str) -> Result<pb::ChangeRootPlan, Status> {
    client(rig)
        .await
        .plan_change_root(pb::PlanChangeRootRequest { id: Some(id.clone()), root: root.to_owned() })
        .await
        .map(tonic::Response::into_inner)
}

async fn change_of(
    rig: &Rig,
    id: &pb::MachineId,
    root: &str,
    plan: &pb::ChangeRootPlan,
    move_channels: bool,
) -> Result<pb::Operation, Status> {
    client(rig)
        .await
        .change_root(pb::ChangeRootRequest {
            id: Some(id.clone()),
            root: root.to_owned(),
            plan: plan.token.clone(),
            move_channels,
            ..Default::default()
        })
        .await
        .map(tonic::Response::into_inner)
}

#[tokio::test(flavor = "multi_thread")]
async fn this_macs_root_is_the_folder_channels_live_in_and_it_changes_like_a_boxs() {
    let rig = Rig::start(&["box-1"], &[]).await;
    let host = rig.host_id().await;
    let channels = rig.world.channels.display().to_string();
    assert_eq!(this_mac_root(&rig, &host).await, channels);
    // The CLI and the app call it by name as well as by id.
    let alias = id_of("this-mac").expect("an id");
    assert_eq!(this_mac_root(&rig, &alias).await, channels);

    let planned = plan_of(&rig, &host, "~/Work").await.expect("the change is planned");
    assert_eq!(planned.old_root, channels);
    assert_eq!(planned.new_root, "~/Work");
    assert_eq!(this_mac_root(&rig, &host).await, channels, "a plan changes nothing");
    let started = change_of(&rig, &host, "~/Work", &planned, false).await.expect("the change starts");
    let ended = rig.ended(&started).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    assert_eq!(this_mac_root(&rig, &host).await, "~/Work");
    let again = change_of(&rig, &host, "~/Work", &planned, false).await.unwrap_err();
    assert_eq!(again.code(), Code::FailedPrecondition, "a plan serves once");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plan_lists_the_channels_on_the_machine_and_the_sessions_running_in_them() {
    let rig = Rig::start(&["box-1"], &[("acme", "api")]).await;
    let host = rig.host_id().await;
    let here = rig.create_channel("Payments", &[("acme", "api")], &[host.clone()]).await;
    let elsewhere = rig.create_channel("Billing", &[("acme", "api")], &[Rig::box_id("box-1")]).await;

    let quiet = plan_of(&rig, &host, "~/Work").await.expect("planned");
    assert_eq!(quiet.channels, vec![here.id.clone().expect("an id")]);
    assert!(quiet.running.is_empty());

    let session = rig.start_on(&here, &host, "Fix the ledger").await;
    rig.wait_session(&session, "running", |session| common::is_running(session)).await;
    let busy = plan_of(&rig, &host, "~/Work").await.expect("planned");
    assert_eq!(busy.running, vec![session.clone()]);

    let on_box = plan_of(&rig, &Rig::box_id("box-1"), "~/Work").await.expect("planned");
    assert_eq!(on_box.channels, vec![elsewhere.id.expect("an id")]);
    assert!(on_box.running.is_empty(), "the host's session isn't the box's");
}

#[tokio::test(flavor = "multi_thread")]
async fn moving_channels_is_refused_while_a_session_runs_and_new_channels_only_still_goes_through() {
    let rig = Rig::start(&["box-1"], &[("acme", "api")]).await;
    let host = rig.host_id().await;
    let channel = rig.create_channel("Payments", &[("acme", "api")], &[host.clone()]).await;
    let session = rig.start_on(&channel, &host, "Fix the ledger").await;
    rig.wait_session(&session, "running", |session| common::is_running(session)).await;
    let before = this_mac_root(&rig, &host).await;

    let planned = plan_of(&rig, &host, "~/Work").await.expect("planned");
    let refused = change_of(&rig, &host, "~/Work", &planned, true).await.unwrap_err();
    assert_eq!(refused.code(), Code::FailedPrecondition);
    assert_eq!(this_mac_root(&rig, &host).await, before, "a refused move changes nothing");

    let started = change_of(&rig, &host, "~/Work", &planned, false)
        .await
        .expect("the refusal kept the plan for the new-channels-only change");
    let ended = rig.ended(&started).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    assert_eq!(this_mac_root(&rig, &host).await, "~/Work");
}

const REPO: (&str, &str) = ("acme", "api");

/// A root of the test's own, as the string the call takes.
fn new_root_of(rig: &Rig, name: &str) -> (PathBuf, String) {
    let path = rig.world.root.join(name);
    let raw = path.display().to_string();
    (path, raw)
}

/// Changes This Mac's root to `raw` and waits for it to succeed.
async fn change_here(rig: &Rig, raw: &str, move_channels: bool) {
    let host = rig.host_id().await;
    let planned = plan_of(rig, &host, raw).await.expect("planned");
    let started = change_of(rig, &host, raw, &planned, move_channels).await.expect("the change starts");
    let ended = rig.ended(&started).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
}

async fn sessions(rig: &Rig) -> pb::session_service_client::SessionServiceClient<Channel> {
    pb::session_service_client::SessionServiceClient::new(rig.channel().await)
}

/// A session's own folder as the host shows it.
async fn folder_of(rig: &Rig, id: &pb::SessionId) -> PathBuf {
    PathBuf::from(rig.session(id).await.folder)
}

/// Starts a session, then stops it, so its folder is quiet and can be moved.
async fn stopped_session(rig: &Rig, channel: &pb::Channel, host: &pb::MachineId, title: &str) -> pb::SessionId {
    let id = rig.start_on(channel, host, title).await;
    rig.wait_session(&id, "running", |session| common::is_running(session)).await;
    sessions(rig)
        .await
        .stop_session(pb::StopSessionRequest { idempotency_key: None, id: Some(id.clone()), for_move: false })
        .await
        .expect("the session stops");
    rig.wait_session(&id, "stopped", |session| !is_live(session)).await;
    id
}

#[tokio::test(flavor = "multi_thread")]
async fn new_channels_land_under_the_new_root_and_existing_ones_stay_where_they_are() {
    let rig = Rig::start(&[], &[REPO]).await;
    let host = rig.host_id().await;
    let existing = rig.create_channel("Payments", &[REPO], &[host.clone()]).await;
    let (new_root, raw) = new_root_of(&rig, "NewRoot");

    change_here(&rig, &raw, false).await;

    let fresh = rig.create_channel("Billing", &[REPO], &[host.clone()]).await;
    assert!(new_root.join(&fresh.folder).join("main").is_dir(), "a new channel is made under the new root");
    assert!(!rig.world.channels.join(&fresh.folder).exists(), "and not under the old one");
    assert!(rig.host_path(&existing, "main").is_dir(), "the existing channel keeps its folder");
    assert!(!new_root.join(&existing.folder).exists(), "and nothing of it is under the new root");

    let session = rig.start_on(&existing, &host, "Fix the ledger").await;
    assert!(
        folder_of(&rig, &session).await.starts_with(rig.host_path(&existing, "")),
        "a session of the existing channel starts where the channel is"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn moving_channels_brings_each_channel_with_its_files_and_sessions_to_the_new_root() {
    let rig = Rig::start(&[], &[REPO]).await;
    let host = rig.host_id().await;
    let channel = rig.create_channel("Payments", &[REPO], &[host.clone()]).await;
    let session = stopped_session(&rig, &channel, &host, "Fix the ledger").await;
    let old_folder = rig.host_path(&channel, "");
    let old_session = folder_of(&rig, &session).await;
    let inside = old_session.strip_prefix(&old_folder).expect("the session is in its channel").to_owned();
    std::fs::write(old_session.join("notes.txt"), "keep me").unwrap();
    std::fs::write(rig.host_path(&channel, "shared/doc.txt"), "shared").unwrap();
    let (new_root, raw) = new_root_of(&rig, "NewRoot");
    let new_folder = new_root.join(&channel.folder);

    change_here(&rig, &raw, true).await;

    assert!(!old_folder.exists(), "the old folder is gone");
    assert_eq!(std::fs::read_to_string(new_folder.join("shared/doc.txt")).unwrap(), "shared");
    assert_eq!(std::fs::read_to_string(new_folder.join(&inside).join("notes.txt")).unwrap(), "keep me");
    assert!(new_folder.join("main").join(REPO.1).join(".git").exists(), "the main clone came along");
    assert_eq!(this_mac_root(&rig, &host).await, raw);

    let shown = folder_of(&rig, &session).await;
    assert_eq!(shown, new_folder.join(&inside), "the session names its new folder");
    let listed = sessions(&rig)
        .await
        .list_sessions(pb::ListSessionsRequest { channel: channel.id.clone(), ..Default::default() })
        .await
        .expect("sessions are listed")
        .into_inner()
        .sessions;
    assert!(listed.iter().any(|one| one.folder == shown.display().to_string()), "the list names it too");
    let channel_now = pb::channel_service_client::ChannelServiceClient::new(rig.channel().await)
        .get_channel(pb::GetChannelRequest { id: channel.id.clone() })
        .await
        .expect("the channel is still there")
        .into_inner();
    assert_eq!(channel_now.folder, channel.folder);

    sessions(&rig)
        .await
        .resume_session(pb::ResumeSessionRequest { id: Some(session.clone()), ..Default::default() })
        .await
        .expect("the stopped session resumes");
    let resumed = rig.wait_session(&session, "live again", |one| is_live(one)).await;
    assert_eq!(PathBuf::from(resumed.folder), new_folder.join(&inside), "it resumes in the new folder");

    let later = rig.start_on(&channel, &host, "Later").await;
    assert!(folder_of(&rig, &later).await.starts_with(&new_folder), "a session started after starts there");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_new_root_is_kept_across_a_restart_of_the_daemon() {
    let mut rig = Rig::start(&[], &[REPO]).await;
    let host = rig.host_id().await;
    let existing = rig.create_channel("Payments", &[REPO], &[host.clone()]).await;
    let (new_root, raw) = new_root_of(&rig, "NewRoot");
    change_here(&rig, &raw, false).await;

    rig.restart_host(&[]).await;

    assert_eq!(this_mac_root(&rig, &host).await, raw);
    let fresh = rig.create_channel("Billing", &[REPO], &[host.clone()]).await;
    assert!(new_root.join(&fresh.folder).join("main").is_dir(), "the new root is used after the restart");
    assert!(rig.host_path(&existing, "main").is_dir(), "the old channel is still where it was");
    let session = rig.start_on(&existing, &host, "Fix the ledger").await;
    assert!(folder_of(&rig, &session).await.starts_with(rig.host_path(&existing, "")));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_into_a_root_with_a_clashing_folder_is_refused_and_changes_nothing() {
    let rig = Rig::start(&[], &[REPO]).await;
    let host = rig.host_id().await;
    let channel = rig.create_channel("Payments", &[REPO], &[host.clone()]).await;
    let other = rig.create_channel("Billing", &[REPO], &[host.clone()]).await;
    let session = stopped_session(&rig, &channel, &host, "Fix the ledger").await;
    let before = folder_of(&rig, &session).await;
    let root_before = this_mac_root(&rig, &host).await;
    let (new_root, raw) = new_root_of(&rig, "NewRoot");
    std::fs::create_dir_all(new_root.join(&other.folder)).unwrap();
    std::fs::write(new_root.join(&other.folder).join("theirs.txt"), "not ours").unwrap();

    let planned = plan_of(&rig, &host, &raw).await.expect("planned");
    let refused = change_of(&rig, &host, &raw, &planned, true).await.unwrap_err();

    assert_eq!(refused.code(), Code::FailedPrecondition);
    assert!(rig.host_path(&channel, "main").is_dir() && rig.host_path(&other, "main").is_dir());
    assert!(!new_root.join(&channel.folder).exists(), "nothing was moved ahead of the clash");
    assert_eq!(std::fs::read_to_string(new_root.join(&other.folder).join("theirs.txt")).unwrap(), "not ours");
    assert_eq!(folder_of(&rig, &session).await, before);
    assert_eq!(this_mac_root(&rig, &host).await, root_before);
}

// -- a box's own folder, changed on the box -------------------------------

const ON_BOX: &str = "box-1";

/// A root on the box's own disk, as the string the call takes.
fn box_root_of(rig: &Rig, name: &str) -> (PathBuf, String) {
    let path = rig.the(ON_BOX).home().join(name);
    let raw = path.display().to_string();
    (path, raw)
}

/// Changes the box's root to `raw` and waits for it to succeed.
async fn change_box(rig: &Rig, raw: &str, move_channels: bool) {
    let planned = plan(rig, raw).await.expect("planned");
    let started = change(rig, raw, &planned, move_channels).await.expect("the change starts");
    let ended = rig.ended(&started).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_box_changed_for_new_channels_shows_the_new_root_and_makes_the_next_channel_there() {
    let rig = Rig::start(&[ON_BOX], &[REPO]).await;
    let one = Rig::box_id(ON_BOX);
    let existing = rig.create_channel("Payments", &[REPO], &[one.clone()]).await;
    let (new_root, raw) = box_root_of(&rig, "NewRoot");

    change_box(&rig, &raw, false).await;

    assert_eq!(root_of(&rig).await, raw);
    let fresh = rig.create_channel("Billing", &[REPO], &[one]).await;
    assert!(new_root.join(&fresh.folder).join("main").is_dir(), "the next channel is made under the new root on the box");
    assert!(!rig.the(ON_BOX).channels().join(&fresh.folder).exists(), "and not under the old one");
    assert!(rig.the(ON_BOX).path(&existing, "main").is_dir(), "the existing channel stays where it was");
}

#[tokio::test(flavor = "multi_thread")]
async fn moving_a_box_brings_its_channels_with_their_files_and_sessions_to_the_new_root() {
    let rig = Rig::start(&[ON_BOX], &[REPO]).await;
    let one = Rig::box_id(ON_BOX);
    let channel = rig.create_channel("Payments", &[REPO], &[one.clone()]).await;
    let session = stopped_session(&rig, &channel, &one, "Fix the ledger").await;
    let old_folder = rig.the(ON_BOX).path(&channel, "");
    std::fs::create_dir_all(rig.the(ON_BOX).path(&channel, "shared")).unwrap();
    std::fs::write(rig.the(ON_BOX).path(&channel, "shared/doc.txt"), "shared").unwrap();
    let (new_root, raw) = box_root_of(&rig, "NewRoot");
    let new_folder = new_root.join(&channel.folder);

    change_box(&rig, &raw, true).await;

    assert!(!old_folder.exists(), "the old folder is gone on the box");
    assert_eq!(std::fs::read_to_string(new_folder.join("shared/doc.txt")).unwrap(), "shared");
    assert!(new_folder.join("main").join(REPO.1).join(".git").exists(), "the main clone came along");
    assert_eq!(root_of(&rig).await, raw);
    let kept = rig
        .the(ON_BOX)
        .store()
        .session(&session.value)
        .unwrap()
        .expect("the box still has the session")
        .folder;
    assert!(kept.starts_with(&new_folder), "the box's record names the new folder: {}", kept.display());

    sessions(&rig)
        .await
        .resume_session(pb::ResumeSessionRequest { id: Some(session.clone()), ..Default::default() })
        .await
        .expect("the stopped session resumes");
    rig.wait_session(&session, "live again", |one| is_live(one)).await;
    let later = rig.start_on(&channel, &one, "Later").await;
    assert!(
        rig.the(ON_BOX).store().session(&later.value).unwrap().expect("recorded").folder.starts_with(&new_folder),
        "a session started after the move works under the new root"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn moving_a_box_is_refused_while_a_session_runs_on_it_and_changes_nothing() {
    let rig = Rig::start(&[ON_BOX], &[REPO]).await;
    let one = Rig::box_id(ON_BOX);
    let channel = rig.create_channel("Payments", &[REPO], &[one.clone()]).await;
    let session = rig.start_on(&channel, &one, "Fix the ledger").await;
    rig.wait_session(&session, "running", |session| common::is_running(session)).await;
    let before = root_of(&rig).await;
    let (new_root, raw) = box_root_of(&rig, "NewRoot");

    let planned = plan(&rig, &raw).await.expect("planned");
    assert_eq!(planned.running, vec![session.clone()], "the plan lists the box's running session");
    let refused = change(&rig, &raw, &planned, true).await.unwrap_err();

    assert_eq!(refused.code(), Code::FailedPrecondition);
    assert_eq!(root_of(&rig).await, before);
    assert!(rig.the(ON_BOX).path(&channel, "main").is_dir(), "the channel didn't move");
    assert!(!new_root.exists(), "nothing was made under the new root");

    let started = change(&rig, &raw, &planned, false)
        .await
        .expect("the refusal kept the plan for the new-channels-only change");
    let ended = rig.ended(&started).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    assert_eq!(root_of(&rig).await, raw);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_box_plan_names_the_box_and_serves_once_for_its_own_root() {
    let rig = Rig::start(&[ON_BOX], &[REPO]).await;
    let one = Rig::box_id(ON_BOX);
    let channel = rig.create_channel("Payments", &[REPO], &[one.clone()]).await;
    let planned = plan(&rig, "~/Work").await.expect("planned");
    assert_eq!(planned.machine, Some(one));
    assert_eq!(planned.channels, vec![channel.id.expect("an id")]);
    let other = change(&rig, "~/Else", &planned, false).await.unwrap_err();
    assert_eq!(other.code(), Code::FailedPrecondition);
    change(&rig, "~/Work", &planned, false).await.expect("the plan still serves its own root");
    let again = change(&rig, "~/Work", &planned, false).await.unwrap_err();
    assert_eq!(again.code(), Code::FailedPrecondition);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_box_that_is_offline_answers_at_once_and_the_root_stays() {
    let rig = Rig::start(&[ON_BOX], &[]).await;
    let planned = plan(&rig, "~/Work").await.expect("planned while the box is up");
    rig.link_lost(ON_BOX).await;
    let before = root_of(&rig).await;

    let no_plan = plan(&rig, "~/Work").await.unwrap_err();
    assert!(matches!(common::error_of(&no_plan).kind, Some(pb::error::Kind::MachineOffline(_))), "{no_plan:?}");
    let no_change = change(&rig, "~/Work", &planned, false).await.unwrap_err();
    assert!(matches!(common::error_of(&no_change).kind, Some(pb::error::Kind::MachineOffline(_))), "{no_change:?}");
    assert_eq!(root_of(&rig).await, before);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_cannot_change_a_boxs_root() {
    let rig = Rig::start(&[ON_BOX], &[REPO]).await;
    let host = rig.host_id().await;
    let channel = rig.create_channel("Payments", &[REPO], &[host.clone()]).await;
    let on_host = rig.start_on(&channel, &host, "Fix the ledger").await;
    rig.wait_session(&on_host, "running", |session| common::is_running(session)).await;
    let (token, _shell) = common::shell_token(&rig.host, &on_host).await;
    let before = root_of(&rig).await;
    let planned = plan(&rig, "~/Work").await.expect("the user plans");

    fn with_token<T>(token: &str, message: T) -> tonic::Request<T> {
        let mut request = tonic::Request::new(message);
        request.metadata_mut().insert("humata-session-token", tonic::metadata::MetadataValue::try_from(token).unwrap());
        request
    }
    let as_agent = |message| with_token(&token, message);
    let as_agent_change = |message| with_token(&token, message);
    let mut machines = client(&rig).await;
    let planning = machines
        .plan_change_root(as_agent(pb::PlanChangeRootRequest { id: Some(Rig::box_id(ON_BOX)), root: "~/Work".to_owned() }))
        .await
        .unwrap_err();
    assert_eq!(planning.code(), Code::PermissionDenied);
    let changing = machines
        .change_root(as_agent_change(pb::ChangeRootRequest {
            id: Some(Rig::box_id(ON_BOX)),
            root: "~/Work".to_owned(),
            plan: planned.token.clone(),
            ..Default::default()
        }))
        .await
        .unwrap_err();
    assert_eq!(changing.code(), Code::PermissionDenied);
    assert_eq!(root_of(&rig).await, before);
    change(&rig, "~/Work", &planned, false).await.expect("the user's plan was not spent by the agent");
}

// -- the host's records follow a box's folder, and the change survives a failure --

/// Starts a plain Desk session on the box, then stops it, so its folder is
/// quiet and can move. The host records the box's own path for it.
async fn stopped_desk_session_on_box(rig: &Rig) -> pb::SessionId {
    let started = pb::desk_service_client::DeskServiceClient::new(rig.channel().await)
        .start_desk_session(pb::StartDeskSessionRequest {
            idempotency_key: Some(pb::IdempotencyKey { value: "desk-scratch".to_owned() }),
            title: "Scratch notes".to_owned(),
            machine: Some(Rig::box_id(ON_BOX)),
            access: Some(pb::DeskAccess { boards: true, channels_and_machines: false }),
            ..Default::default()
        })
        .await
        .expect("the Desk session starts on the box")
        .into_inner();
    if let Some(operation) = &started.operation {
        let ended = rig.ended(operation).await;
        assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    }
    let id = started.session.expect("a start names its session");
    rig.wait_session(&id, "running", |session| is_live(session)).await;
    sessions(rig)
        .await
        .stop_session(pb::StopSessionRequest { idempotency_key: None, id: Some(id.clone()), for_move: false })
        .await
        .expect("the session stops");
    rig.wait_session(&id, "stopped", |session| !is_live(session)).await;
    id
}

async fn operations(rig: &Rig) -> pb::operation_service_client::OperationServiceClient<Channel> {
    pb::operation_service_client::OperationServiceClient::new(rig.channel().await)
}

/// The host's operations that change a machine's folder, oldest first.
async fn root_operations(rig: &Rig) -> Vec<pb::Operation> {
    let mut found: Vec<pb::Operation> = operations(rig)
        .await
        .list_operations(pb::ListOperationsRequest::default())
        .await
        .expect("operations are listed")
        .into_inner()
        .operations
        .into_iter()
        .filter(|operation| operation.kind == pb::OperationKind::ChangeRoot as i32)
        .collect();
    found.sort_by_key(|operation| operation.started_at.as_ref().map(|at| (at.seconds, at.nanos)));
    found
}

#[tokio::test(flavor = "multi_thread")]
async fn the_hosts_record_of_a_boxs_desk_session_names_the_new_folder_after_the_channels_move() {
    let rig = Rig::start(&[ON_BOX], &[REPO]).await;
    let session = stopped_desk_session_on_box(&rig).await;
    let before = folder_of(&rig, &session).await;
    assert!(before.starts_with(rig.the(ON_BOX).channels()), "the host names the box's own folder: {}", before.display());
    let (new_root, raw) = box_root_of(&rig, "NewRoot");

    change_box(&rig, &raw, true).await;

    let after = folder_of(&rig, &session).await;
    assert_eq!(after, new_root.join("desk").join(before.file_name().expect("a folder name")), "the host names the new folder");
    assert!(after.is_dir(), "and the folder is there on the box");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_desk_session_the_box_moved_resumes_in_the_new_place() {
    let rig = Rig::start(&[ON_BOX], &[REPO]).await;
    let session = stopped_desk_session_on_box(&rig).await;
    let (new_root, raw) = box_root_of(&rig, "NewRoot");
    change_box(&rig, &raw, true).await;

    sessions(&rig)
        .await
        .resume_session(pb::ResumeSessionRequest { id: Some(session.clone()), ..Default::default() })
        .await
        .expect("the stopped session resumes");
    let resumed = rig.wait_session(&session, "live again", |one| is_live(one)).await;

    assert!(PathBuf::from(resumed.folder).starts_with(&new_root), "it works under the new root");
}

#[tokio::test(flavor = "multi_thread")]
async fn changing_a_box_for_new_channels_only_leaves_the_hosts_records_of_its_sessions_alone() {
    let rig = Rig::start(&[ON_BOX], &[REPO]).await;
    let session = stopped_desk_session_on_box(&rig).await;
    let before = folder_of(&rig, &session).await;
    let (_, raw) = box_root_of(&rig, "NewRoot");

    change_box(&rig, &raw, false).await;

    assert_eq!(folder_of(&rig, &session).await, before, "nothing moved, so the record stays");
    assert!(before.is_dir(), "the folder is where the record says");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refusal_of_the_box_leaves_a_failed_operation_that_names_it_and_the_same_key_finds_it() {
    let rig = Rig::start(&[ON_BOX], &[REPO]).await;
    let one = Rig::box_id(ON_BOX);
    let channel = rig.create_channel("Payments", &[REPO], &[one.clone()]).await;
    let session = rig.start_on(&channel, &one, "Fix the ledger").await;
    rig.wait_session(&session, "running", |session| common::is_running(session)).await;
    let before = root_of(&rig).await;
    let (_, raw) = box_root_of(&rig, "NewRoot");
    let planned = plan(&rig, &raw).await.expect("planned");
    let asked = pb::ChangeRootRequest {
        idempotency_key: Some(pb::IdempotencyKey { value: "move-box".to_owned() }),
        id: Some(one.clone()),
        root: raw.clone(),
        plan: planned.token.clone(),
        move_channels: true,
        ..Default::default()
    };

    let refused = client(&rig).await.change_root(asked.clone()).await.unwrap_err();

    assert_eq!(refused.code(), Code::FailedPrecondition);
    let listed = root_operations(&rig).await;
    assert_eq!(listed.len(), 1, "the change left one operation");
    let failed = &listed[0];
    assert_eq!(failed.state, pb::OperationState::Failed as i32);
    let error = failed.error.as_ref().expect("a failed operation says why");
    assert!(
        matches!(&error.kind, Some(pb::error::Kind::SessionBusy(busy)) if busy.session.as_ref() == Some(&session)),
        "it names the session the box refused for: {error:?}"
    );
    let found = operations(&rig)
        .await
        .get_operation(pb::GetOperationRequest { id: failed.id.clone() })
        .await
        .expect("the operation is there by id")
        .into_inner();
    assert_eq!(found.state, pb::OperationState::Failed as i32);
    assert_eq!(root_of(&rig).await, before, "a refused change keeps the root");

    let again = client(&rig).await.change_root(asked).await.expect("the same key finds the operation").into_inner();
    assert_eq!(again.id, failed.id);
    assert_eq!(root_operations(&rig).await.len(), 1, "and starts no other");
}

#[tokio::test(flavor = "multi_thread")]
async fn retrying_the_failed_change_asks_the_box_again_and_ends_done_with_the_root_saved() {
    let rig = Rig::start(&[ON_BOX], &[REPO]).await;
    let one = Rig::box_id(ON_BOX);
    let channel = rig.create_channel("Payments", &[REPO], &[one.clone()]).await;
    let again = rig.start_on(&channel, &one, "Fix the ledger").await;
    rig.wait_session(&again, "running", |session| common::is_running(session)).await;
    let (new_root, raw) = box_root_of(&rig, "NewRoot");
    let planned = plan(&rig, &raw).await.expect("planned");
    change(&rig, &raw, &planned, true).await.unwrap_err();
    let failed = root_operations(&rig).await.remove(0);
    sessions(&rig)
        .await
        .stop_session(pb::StopSessionRequest { idempotency_key: None, id: Some(again.clone()), for_move: false })
        .await
        .expect("the session stops");
    rig.wait_session(&again, "stopped", |one| !is_live(one)).await;

    let retried = operations(&rig)
        .await
        .retry_operation(pb::RetryOperationRequest { id: failed.id.clone(), idempotency_key: None })
        .await
        .expect("the failed change is retried")
        .into_inner();
    let ended = rig.ended(&retried).await;

    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    assert_eq!(ended.id, failed.id, "it is the same operation");
    assert_eq!(root_of(&rig).await, raw);
    assert!(new_root.join(&channel.folder).join("main").is_dir(), "the channel moved");
    assert!(!rig.the(ON_BOX).path(&channel, "main").exists(), "and left the old folder");
}
