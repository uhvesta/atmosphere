//! PER-29, D241: the restart report names each operation the start found
//! cut short, with how it ended. A host killed mid-move or mid-fork writes
//! `restarts/<time>/operations.jsonl` at its next start: the operation, the
//! step it reached, and a terminal state once its recovery has settled.
//!
//! Every wait is on the host's feeds; the report is read only after the
//! session shows what the recovery left (it is written before that).

use humatad_test_common as common;
use humatad_test_common::remote_rig;

use std::path::Path;
use std::time::{Duration, Instant};

use humata2_grpc::humata::v1 as pb;
use remote_rig::{Rig, is_live};
use tonic::transport::Channel;

const REPO: (&str, &str) = ("acme", "api");

/// What the newest report of the host's state folder says of each
/// operation: the last line per id, as `(id, kind, step, state)`.
fn operations(rig: &Rig) -> Vec<(String, String, String, String)> {
    operations_in(&rig.host.state)
}

/// The same for the daemon whose state folder is `state`.
fn operations_in(state: &Path) -> Vec<(String, String, String, String)> {
    let mut folders: Vec<_> = std::fs::read_dir(state.join("restarts"))
        .expect("the host wrote restart reports")
        .flatten()
        .map(|entry| entry.path())
        .collect();
    folders.sort();
    let file = folders
        .iter()
        .rev()
        .map(|folder| folder.join("operations.jsonl"))
        .find(|file| file.exists())
        .expect("a restart report names the operations");
    let mut latest: Vec<(String, String, String, String)> = Vec::new();
    for line in std::fs::read_to_string(file).unwrap().lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let field = |key: &str| value[key].as_str().unwrap_or_default().to_owned();
        let entry = (field("id"), field("kind"), field("step"), field("state"));
        match latest.iter_mut().find(|known| known.0 == entry.0) {
            Some(known) => *known = entry,
            None => latest.push(entry),
        }
    }
    latest
}

fn move_request(id: &pb::SessionId, to: &pb::MachineId, key: &str) -> pb::MoveSessionRequest {
    pb::MoveSessionRequest {
        idempotency_key: Some(pb::IdempotencyKey { value: key.to_owned() }),
        id: Some(id.clone()),
        to: Some(to.clone()),
        ..Default::default()
    }
}

/// Moves `id` to `to` under a host killed at `at`, then starts the host again.
async fn move_killed_at(rig: &mut Rig, id: &pb::SessionId, to: &pb::MachineId, at: &str) {
    rig.restart_host(&[("HUMATA2_STOP_AT", at)]).await;
    let _ = pb::session_service_client::SessionServiceClient::new(rig.channel().await)
        .move_session(move_request(id, to, &format!("move-killed-{at}")))
        .await;
    rig.host_stopped().await;
    rig.restart_host(&[]).await;
}

fn not_moving(session: &pb::Session) -> bool {
    session.status != pb::SessionStatus::Moving as i32
}

/// An archive cut after its intent was saved: the restart finishes it, and
/// the report says so.
#[tokio::test(flavor = "multi_thread")]
async fn an_archive_cut_short_is_named_in_the_restart_report_with_how_it_ended() {
    let (mut daemon, _world, _channel, session) = daemon_with_session("0").await;
    let id = session.id.clone().expect("a session has an id");
    daemon
        .restart_env(&[("HUMATA2_ARCHIVE_UNDO_MS", "0"), ("HUMATA2_STOP_AT", "archive.recorded")])
        .await;
    common::last_start(&daemon).await;
    let sessions = daemon.session_service().await;
    let plan = sessions
        .clone()
        .plan_archive(pb::PlanArchiveRequest { id: Some(id.clone()) })
        .await
        .expect("the archive is planned")
        .into_inner();
    let _ = sessions
        .clone()
        .archive_session(pb::ArchiveSessionRequest {
            idempotency_key: None,
            id: Some(id.clone()),
            choice: pb::ArchiveChoice::Archive as i32,
            plan: plan.token,
            end: Some(true),
        })
        .await;
    daemon.stopped(common::WAIT * 8).await;
    daemon.restart_env(&[("HUMATA2_ARCHIVE_UNDO_MS", "0")]).await;
    common::last_start(&daemon).await;
    assert_named_pending(&daemon.state, "archive", &id.value);
    // The report's line comes before the folder is freed.
    let folder = std::path::PathBuf::from(&session.repos[0].path);
    let folder = folder.parent().expect("a clone is in a session folder").to_owned();
    until("the session's folder being freed", || !folder.exists()).await;

    let named = operations_in(&daemon.state);
    let ended = ended_as(&named, "archive", &id.value);
    assert_eq!(ended, "finished", "{named:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_cut_short_is_named_in_the_restart_report_with_how_it_ended() {
    for (at, step, state, back_on_one) in [
        ("move-copied", "copied", "undone", true),
        ("move-switched", "switched", "finished", false),
    ] {
        let mut rig = Rig::start(&["box-1", "box-2"], &[REPO]).await;
        let (one, two) = (Rig::box_id("box-1"), Rig::box_id("box-2"));
        let channel = rig.create_channel("Payments", &[REPO], &[rig.host_id().await, one.clone(), two.clone()]).await;
        let id = rig.start_on(&channel, &one, "Cache warmup").await;
        rig.wait_session(&id, "live", is_live).await;

        move_killed_at(&mut rig, &id, &two, at).await;

        let settled = if back_on_one { &one } else { &two };
        rig.wait_session(&id, "settled", |s| s.machine.as_ref() == Some(settled) && is_live(s) && not_moving(s)).await;
        let named = operations(&rig);
        assert_eq!(named.len(), 1, "{at}: {named:?}");
        let (_, kind, reached, ended) = &named[0];
        assert_eq!((kind.as_str(), reached.as_str(), ended.as_str()), ("move", step, state), "{at}");
    }
}

/// How `kind` of `object` ended in the report: `finished` or `undone`.
/// Panics when the report names none, or one that didn't settle.
fn ended_as(named: &[(String, String, String, String)], kind: &str, object: &str) -> String {
    let found: Vec<_> = named.iter().filter(|(id, k, _, _)| k == kind && id.contains(object)).collect();
    assert_eq!(found.len(), 1, "the report should name one {kind} of {object}: {named:?}");
    let state = found[0].3.clone();
    assert!(state == "finished" || state == "undone", "the {kind} of {object} ended {state}: {named:?}");
    state
}

/// Whether the newest report of `state` says `kind` of `object` ended as `ended`;
/// false while there is no report yet.
fn report_says(state: &Path, kind: &str, object: &str, ended: &str) -> bool {
    let written = std::fs::read_dir(state.join("restarts"))
        .is_ok_and(|dir| dir.flatten().any(|e| e.path().join("operations.jsonl").exists()));
    written
        && operations_in(state)
            .iter()
            .any(|(id, k, _, s)| k == kind && id.contains(object) && s == ended)
}

/// That the report already says `kind` of `object` is pending: read the
/// moment the start is served, before the recovery has been waited for, so
/// a report that reads as finished while an operation is owed fails here.
fn assert_named_pending(state: &Path, kind: &str, object: &str) {
    let mut folders: Vec<_> = std::fs::read_dir(state.join("restarts"))
        .expect("the daemon wrote restart reports")
        .flatten()
        .map(|entry| entry.path())
        .collect();
    folders.sort();
    let file = folders.last().expect("a restart report").join("operations.jsonl");
    let text = std::fs::read_to_string(&file).unwrap_or_default();
    let pending = text
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .any(|value| {
            value["kind"] == kind
                && value["state"] == "pending"
                && value["id"].as_str().is_some_and(|id| id.contains(object))
        });
    assert!(pending, "the report should name the {kind} of {object} as pending: {text}");
}

/// Waits, bounded, until `done` holds: for effects on the file system the
/// daemon leaves after its last line of the report.
async fn until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + common::WAIT;
    while !done() {
        assert!(Instant::now() < deadline, "{what} never happened");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// A single daemon with one channel `Payments` on `acme/api` and one running
/// session, under an archive window of `window` ms.
async fn daemon_with_session(window: &str) -> (common::Daemon, common::World, pb::Channel, pb::Session) {
    let dir = common::folder();
    let world = common::World::new(dir.path());
    let args = world.args();
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let daemon = common::Daemon::start_with_env(common::Build::Admin, dir, &refs, &[("HUMATA2_ARCHIVE_UNDO_MS", window)]).await;
    world.remote("acme", "api");
    let (channel, _) = common::create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    let (id, _) = common::start_session(&daemon, &channel, "Fix login").await;
    let session = common::wait_running(&daemon, &id).await;
    (daemon, world, channel, session)
}

/// A channel delete cut after the agents stopped: the restart finishes it.
#[tokio::test(flavor = "multi_thread")]
async fn a_channel_delete_cut_short_is_named_in_the_restart_report_with_how_it_ended() {
    let (mut daemon, world, channel, _session) = daemon_with_session("0").await;
    let id = channel.id.clone().expect("a channel has an id");
    daemon.restart_env(&[("HUMATA2_STOP_AT", "delete_channel.agents_stopped")]).await;
    common::last_start(&daemon).await;
    let channels = daemon.channel_service().await;
    let plan = channels
        .clone()
        .plan_delete_channel(pb::PlanDeleteChannelRequest { channel: Some(id.clone()) })
        .await
        .expect("the delete is planned")
        .into_inner();
    let _ = channels
        .clone()
        .delete_channel(pb::DeleteChannelRequest {
            idempotency_key: None,
            channel: plan.channel.clone(),
            plan: plan.token.clone(),
            presence: None,
            delete_unpushed_work: true,
            end_live: Some(true),
        })
        .await;
    daemon.stopped(common::WAIT * 8).await;
    daemon.restart().await;
    common::last_start(&daemon).await;
    assert_named_pending(&daemon.state, "delete_channel", &id.value);
    // The delete's note goes after the report's line.
    let notes = world.channels.join(".deleting");
    until("the note of the delete going", || std::fs::read_dir(&notes).map_or(true, |mut left| left.next().is_none())).await;

    let named = operations_in(&daemon.state);
    assert_eq!(ended_as(&named, "delete_channel", &id.value), "finished", "{named:?}");
}

async fn machines(rig: &Rig) -> pb::machine_service_client::MachineServiceClient<Channel> {
    pb::machine_service_client::MachineServiceClient::new(rig.channel().await)
}

fn forget_request(plan: &pb::ForgetMachinePlan, machine: &pb::MachineId, key: &str) -> pb::ForgetMachineRequest {
    pb::ForgetMachineRequest {
        idempotency_key: Some(pb::IdempotencyKey { value: key.to_owned() }),
        id: Some(machine.clone()),
        plan: plan.token.clone(),
        presence: None,
    }
}

async fn plan_forget(rig: &Rig, machine: &pb::MachineId) -> pb::ForgetMachinePlan {
    machines(rig)
        .await
        .plan_forget_machine(pb::PlanForgetMachineRequest { id: Some(machine.clone()) })
        .await
        .expect("the plan is made")
        .into_inner()
}

/// A forget cut after it was kept and before anything was archived: the
/// restart finishes it with no one asking again.
#[tokio::test(flavor = "multi_thread")]
async fn a_forget_cut_short_is_named_in_the_restart_report_with_how_it_ended() {
    let mut rig = Rig::start(&["box-1"], &[REPO]).await;
    let (host, one) = (rig.host_id().await, Rig::box_id("box-1"));
    let channel = rig.create_channel("Payments", &[REPO], &[host, one.clone()]).await;
    let on_box = rig.start_on(&channel, &one, "On the box").await;
    rig.wait_session(&on_box, "live", is_live).await;
    rig.the_mut("box-1").kill();
    rig.link_lost("box-1").await;

    // The plan lives in the host's memory, so the stop-point host makes its own.
    rig.restart_host(&[("HUMATA2_STOP_AT", "forget.begun")]).await;
    let plan = plan_forget(&rig, &one).await;
    let _ = machines(&rig).await.forget_machine(forget_request(&plan, &one, "forget-cut")).await;
    rig.host_stopped().await;
    rig.restart_host(&[]).await;
    common::last_start(&rig.host).await;
    assert_named_pending(&rig.host.state, "forget", &one.value);
    rig.wait_session(&on_box, "archived as lost", |s| s.archive.as_ref().is_some_and(|a| a.machine_lost)).await;
    // The same key answers once the forget is finished, and the report's line
    // comes before the answer is kept.
    machines(&rig).await.forget_machine(forget_request(&plan, &one, "forget-cut")).await.expect("the key answers");

    let named = operations(&rig);
    assert_eq!(ended_as(&named, "forget", &one.value), "finished", "{named:?}");
}

const CLONE: &str = "sessions/cache-warmup/api";

/// A session on box-1 whose branch was pushed, with its conversation
/// mirrored on the host; box-1 then dies and is forgotten, so the session
/// is archived as lost and can be rebuilt.
async fn lost_session(rig: &mut Rig, channel: &pb::Channel) -> pb::SessionId {
    let one = Rig::box_id("box-1");
    let id = rig.start_on(channel, &one, "Cache warmup").await;
    let session = rig.wait_session(&id, "live", is_live).await;
    let tab = common::agent_tab(&rig.host, &id).await;
    let open = pb::AttachOpen { session: Some(id.clone()), ..common::open(&tab, 0) };
    let mut agent = common::Attach::start(&rig.host, open).await;
    agent.wait_for("agent up").await;
    let line = r#"{"type":"user","message":{"content":"before the loss"}}"#;
    agent.type_text(&format!("{line}\n")).await;
    agent.wait_for(&format!("got: {line}")).await;
    let clone = rig.the("box-1").path(channel, CLONE).display().to_string();
    let branch = &session.repos[0].branch;
    std::fs::write(format!("{clone}/pushed.txt"), "pushed\n").unwrap();
    common::git(&["-C", &clone, "add", "pushed.txt"]);
    common::git(&["-C", &clone, "commit", "-m", "pushed work"]);
    common::git(&["-C", &clone, "push", "-u", "origin", branch]);

    rig.the_mut("box-1").kill();
    rig.link_lost("box-1").await;
    let plan = plan_forget(rig, &one).await;
    machines(rig).await.forget_machine(forget_request(&plan, &one, "forget-1")).await.expect("box-1 is forgotten");
    rig.wait_session(&id, "archived as lost", |s| s.archive.as_ref().is_some_and(|a| a.machine_lost)).await;
    id
}

/// A rebuild cut on its machine after the clone, before its agent was
/// launched: the machine's next start takes it back, and its report says so.
#[tokio::test(flavor = "multi_thread")]
async fn a_rebuild_cut_short_is_named_in_the_restart_report_with_how_it_ended() {
    let mut rig = Rig::start(&["box-1", "box-2"], &[REPO]).await;
    let all = [rig.host_id().await, Rig::box_id("box-1"), Rig::box_id("box-2")];
    let channel = rig.create_channel("Payments", &[REPO], &all).await;
    let id = lost_session(&mut rig, &channel).await;
    let two = Rig::box_id("box-2");

    rig.the_mut("box-2").kill();
    rig.link_lost("box-2").await;
    rig.the_mut("box-2").relaunch(&[("HUMATA2_STOP_AT", "rebuild.cloned")]).await;
    rig.link_back("box-2").await;
    let started = pb::session_service_client::SessionServiceClient::new(rig.channel().await)
        .rebuild_session(pb::RebuildSessionRequest {
            idempotency_key: Some(pb::IdempotencyKey { value: "rebuild-cut".to_owned() }),
            id: Some(id.clone()),
            machine: Some(two),
        })
        .await
        .expect("the rebuild starts")
        .into_inner();
    let cut = rig.ended(&started).await;
    assert_eq!(cut.state, pb::OperationState::Failed as i32, "the machine died at its stop point: {cut:?}");
    rig.the_mut("box-2").relaunch(&[]).await;
    // The box's daemon isn't admin, so its socket takes no credential-less call:
    // its own report says when the start has taken the rebuild back.
    let state = rig.the("box-2").daemon().state.clone();
    until("box-2's report names the rebuild as undone", || {
        report_says(&state, "rebuild", &id.value, "undone")
    })
    .await;
    assert_named_pending(&rig.the("box-2").daemon().state, "rebuild", &id.value);

    let named = operations_in(&rig.the("box-2").daemon().state);
    assert_eq!(ended_as(&named, "rebuild", &id.value), "undone", "{named:?}");
}


async fn channel_svc(rig: &Rig) -> pb::channel_service_client::ChannelServiceClient<Channel> {
    pb::channel_service_client::ChannelServiceClient::new(rig.channel().await)
}

/// A machine removal cut after its note was written: the restart finishes
/// it, the report names it pending as soon as the start is served and
/// finished after.
#[tokio::test(flavor = "multi_thread")]
async fn a_machine_removal_cut_short_is_named_in_the_restart_report_with_how_it_ended() {
    let mut rig = Rig::start(&["box-1"], &[REPO]).await;
    let one = Rig::box_id("box-1");
    let channel = rig.create_channel("Payments", &[REPO], &[rig.host_id().await, one.clone()]).await;
    let session = rig.start_on(&channel, &one, "Cache warmup").await;
    rig.wait_session(&session, "live", is_live).await;

    // The plan lives in the host's memory, so the stop-point host makes its own.
    rig.restart_host(&[("HUMATA2_STOP_AT", "remove_machine.intent")]).await;
    let mut channels = channel_svc(&rig).await;
    let plan = channels
        .plan_remove_machine(pb::PlanRemoveChannelMachineRequest {
            channel: channel.id.clone(),
            machine: Some(one.clone()),
        })
        .await
        .expect("the removal is planned")
        .into_inner();
    let _ = channels
        .remove_machine(pb::RemoveChannelMachineRequest {
            idempotency_key: None,
            channel: channel.id.clone(),
            machine: Some(one.clone()),
            dispositions: vec![pb::SessionDisposition {
                session: Some(session.clone()),
                disposition: Some(pb::session_disposition::Disposition::Archive(pb::ArchiveChoice::Archive as i32)),
            }],
            plan: plan.token.clone(),
        })
        .await;
    rig.host_stopped().await;
    rig.restart_host(&[]).await;
    common::last_start(&rig.host).await;

    let channel_id = channel.id.as_ref().expect("a channel has an id").value.clone();
    assert_named_pending(&rig.host.state, "remove_machine", &channel_id);
    rig.wait_session(&session, "archived", |s| s.archive.is_some()).await;
    // The report's last line comes just after the machine is dropped.
    until("the report ending the removal", || {
        operations_in(&rig.host.state)
            .iter()
            .any(|(id, kind, _, state)| kind == "remove_machine" && id.contains(&channel_id) && state == "finished")
    })
    .await;

    let named = operations(&rig);
    assert_eq!(ended_as(&named, "remove_machine", &channel_id), "finished", "{named:?}");
}

/// A channel restored under another folder and cut right after its records
/// came back: the restart moves its files, the report names it pending as
/// soon as the start is served and finished after.
#[tokio::test(flavor = "multi_thread")]
async fn a_channel_restore_cut_short_is_named_in_the_restart_report_with_how_it_ended() {
    let (mut daemon, world, channel, _session) = daemon_with_session("0").await;
    let id = channel.id.clone().expect("a channel has an id");
    let mut channels = daemon.channel_service().await;
    let plan = channels
        .plan_delete_channel(pb::PlanDeleteChannelRequest { channel: Some(id.clone()) })
        .await
        .expect("the delete is planned")
        .into_inner();
    let deleted = channels
        .delete_channel(pb::DeleteChannelRequest {
            idempotency_key: None,
            channel: plan.channel.clone(),
            plan: plan.token.clone(),
            presence: None,
            delete_unpushed_work: true,
            end_live: Some(true),
        })
        .await
        .expect("the channel is deleted")
        .into_inner();
    let operation = deleted.operation.expect("a delete is an operation");
    let done = common::finished(&daemon, &operation).await;
    assert_eq!(done.state, pb::OperationState::Succeeded as i32, "{done:?}");
    let item = deleted.trash_item.expect("the folder went to the trash");
    // Something takes the old folder, so the channel comes back at `-2` and
    // a note of the files still to move is kept.
    std::fs::create_dir_all(world.channels.join(&channel.folder)).unwrap();

    daemon.restart_env(&[("HUMATA2_STOP_AT", "restore_channel.imported")]).await;
    common::last_start(&daemon).await;
    // The daemon kills itself at the point, so the reply may not come.
    let _ = pb::trash_service_client::TrashServiceClient::new(daemon.channel().await)
        .restore_from_trash(pb::RestoreFromTrashRequest { idempotency_key: None, id: Some(item) })
        .await;
    daemon.stopped(common::WAIT * 8).await;
    daemon.restart().await;
    common::last_start(&daemon).await;

    assert_named_pending(&daemon.state, "restore_channel", &id.value);
    // The report's line comes before the note goes.
    let notes = world.channels.join(".restoring");
    until("the note of the restore going", || std::fs::read_dir(&notes).map_or(true, |mut left| left.next().is_none())).await;
    let named = operations_in(&daemon.state);
    assert_eq!(ended_as(&named, "restore_channel", &id.value), "finished", "{named:?}");
}
