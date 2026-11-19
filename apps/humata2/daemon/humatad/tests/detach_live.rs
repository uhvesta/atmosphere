//! D801 I: an automatic end becomes a detach. The channel-desk watch and a
//! move's release never end a running terminal: it keeps its holder (same
//! pid) and is listed by the machine under "Not in a channel" (no session
//! names it), where the owner attaches, adopts or ends it (D784).
//!
//! The move test uses the two-machine rig, as move_session.rs does.

mod common;
#[path = "common/remote_rig.rs"]
mod remote_rig;

use std::time::{Duration, Instant};

use common::{Build, Daemon, World, create_channel, folder, holder_answers, start_session, wait_status};
use humata2_grpc::humata::v1 as pb;
use remote_rig::{Rig, is_live};
use tonic::transport::Channel;

const REPO: (&str, &str) = ("acme", "api");

async fn machines(channel: Channel) -> pb::machine_service_client::MachineServiceClient<Channel> {
    pb::machine_service_client::MachineServiceClient::new(channel)
}

/// Every running holder `machine` lists (`None`: the daemon's own).
async fn listed(channel: Channel, machine: Option<pb::MachineId>) -> Vec<pb::RunningHolder> {
    machines(channel)
        .await
        .list_running_holders(pb::ListRunningHoldersRequest { machine })
        .await
        .expect("the running holders list")
        .into_inner()
        .holders
}

/// Whether a process with `pid` exists, asked of `ps`, which signals nothing.
fn alive(pid: u32) -> bool {
    std::process::Command::new("ps")
        .args(["-o", "pid=", "-p", &pid.to_string()])
        .output()
        .is_ok_and(|out| out.status.success() && !out.stdout.trim_ascii().is_empty())
}

async fn until<F, Fut>(what: &str, done: F)
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = Instant::now() + Duration::from_secs(30);
    while !done().await {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// CD-5, D801 I: the target is archived past its undo window, so the watch
/// closes the channel Desk session; its running agent keeps its holder and
/// is listed with no session.
#[tokio::test(flavor = "multi_thread")]
async fn a_channel_desk_the_watch_closes_leaves_its_running_agent_running_with_no_session() {
    let dir = folder();
    let world = World::new(dir.path());
    let args = world.args();
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let daemon = Daemon::start_with_env(Build::Admin, dir, &refs, &[("HUMATA2_ARCHIVE_UNDO_MS", "0")]).await;
    world.remote(REPO.0, REPO.1);
    let (channel, _) = create_channel(&daemon, "Payments", &[REPO]).await;
    let (target, _) = start_session(&daemon, &channel, "Ledger fix").await;
    wait_status(&daemon, &target, pb::SessionStatus::Working).await;

    let mut desks = pb::desk_service_client::DeskServiceClient::new(daemon.channel().await);
    let started = desks
        .start_desk_session(pb::StartDeskSessionRequest {
            title: "Watch the ledger".to_owned(),
            channel: channel.id.clone(),
            target: Some(target.clone()),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    let desk = started.session.unwrap();
    wait_status(&daemon, &desk, pb::SessionStatus::Working).await;
    let before = listed(daemon.channel().await, None)
        .await
        .into_iter()
        .find(|holder| holder.session.as_ref() == Some(&desk))
        .expect("the Desk session's agent runs");
    let (tab, pid) = (before.tab.clone().unwrap(), before.pid);

    // The user archives the target; the watch closes the Desk session.
    let mut sessions = daemon.session_service().await;
    let plan = sessions
        .plan_archive(pb::PlanArchiveRequest { id: Some(target.clone()) })
        .await
        .unwrap()
        .into_inner();
    let choice = if plan.choices.contains(&(pb::ArchiveChoice::Archive as i32)) {
        pb::ArchiveChoice::Archive
    } else {
        pb::ArchiveChoice::ArchiveAndDeleteWork
    };
    let archived = sessions
        .archive_session(pb::ArchiveSessionRequest {
            idempotency_key: None,
            id: Some(target.clone()),
            choice: choice as i32,
            plan: plan.token,
            end: Some(true),
        })
        .await
        .unwrap()
        .into_inner();
    let done = common::finished(&daemon, &archived).await;
    assert_eq!(done.state, pb::OperationState::Succeeded as i32, "{done:?}");

    until("the Desk session closing", || async {
        listed(daemon.channel().await, None)
            .await
            .iter()
            .any(|holder| holder.tab.as_ref() == Some(&tab) && holder.session.is_none())
    })
    .await;
    let after = listed(daemon.channel().await, None)
        .await
        .into_iter()
        .find(|holder| holder.tab.as_ref() == Some(&tab))
        .expect("the agent is still listed");
    assert_eq!(after.pid, pid, "the same holder runs");
    assert!(after.channel.is_none() && !after.desk, "it is under Not in a channel");
    assert!(alive(pid), "the holder process was not ended");

    // The owner ends it, as this test started it.
    machines(daemon.channel().await)
        .await
        .end_running_holder(pb::EndRunningHolderRequest { machine: None, tab: Some(tab) })
        .await
        .expect("the owner ends the orphan");
}

/// D801 I: after a move, the old machine's release leaves the session's
/// shell running, listed with no session.
#[tokio::test(flavor = "multi_thread")]
async fn a_move_leaves_the_old_machines_shell_running_with_no_session() {
    let rig = Rig::start(&["box-1", "box-2"], &[REPO]).await;
    let (one, two) = (Rig::box_id("box-1"), Rig::box_id("box-2"));
    let on = [rig.host_id().await, one.clone(), two.clone()];
    let channel = rig.create_channel("Payments", &[REPO], &on).await;
    let id = rig.start_on(&channel, &one, "Cache warmup").await;
    rig.wait_session(&id, "live", is_live).await;
    let tab = rig
        .host
        .terminals()
        .await
        .open_shell_tab(pb::OpenShellTabRequest { session: Some(id.clone()), ..Default::default() })
        .await
        .expect("a shell tab opens on box-1")
        .into_inner()
        .id
        .expect("the tab has an id");
    let old = rig
        .the("box-1")
        .tabs()
        .into_iter()
        .find(|record| record.id == tab.value)
        .expect("box-1 keeps the shell tab");
    let pid = old.holder_pid;

    let started = pb::session_service_client::SessionServiceClient::new(rig.channel().await)
        .move_session(pb::MoveSessionRequest {
            idempotency_key: Some(pb::IdempotencyKey { value: "detach-live-move".to_owned() }),
            id: Some(id.clone()),
            to: Some(two.clone()),
        })
        .await
        .expect("the move starts")
        .into_inner();
    let ended = rig.ended(&started).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "the move failed: {:?}", ended.error);
    rig.wait_session(&id, "live on box-2", |s| s.machine.as_ref() == Some(&two) && is_live(s)).await;

    assert!(holder_answers(&old), "box-1's shell keeps running");
    let shown = listed(rig.channel().await, Some(one.clone()))
        .await
        .into_iter()
        .find(|holder| holder.pid == pid)
        .expect("box-1 lists the shell it left");
    assert!(shown.session.is_none(), "no session names it: {shown:?}");

    machines(rig.channel().await)
        .await
        .end_running_holder(pb::EndRunningHolderRequest { machine: Some(one), tab: shown.tab })
        .await
        .expect("the owner ends the orphan");
}
