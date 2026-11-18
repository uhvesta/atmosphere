//! HO-6, D784's remote amendment: a machine shell on another machine. The
//! host relays OpenMachineShell to the box's own daemon, which opens it in
//! its own tab model; attach is relayed by the tab's machine; the box lists
//! it among its running holders, and stop, resume and end reach it through
//! the host. An offline box refuses the open, and nothing is queued.

mod common;
#[path = "common/remote_rig.rs"]
mod remote_rig;

use std::time::{Duration, Instant};

use common::{Attach, holder_answers, open};
use humata2_grpc::humata::v1 as pb;
use remote_rig::Rig;
use tonic::{Code, Status};
use tonic::transport::Channel;

const BOX: &str = "box-1";
const DEADLINE: Duration = Duration::from_secs(30);

async fn machines(rig: &Rig) -> pb::machine_service_client::MachineServiceClient<Channel> {
    pb::machine_service_client::MachineServiceClient::new(rig.channel().await)
}

async fn list(rig: &Rig) -> Vec<pb::RunningHolder> {
    machines(rig)
        .await
        .list_running_holders(pb::ListRunningHoldersRequest { machine: Some(Rig::box_id(BOX)) })
        .await
        .expect("the box's holders are listed")
        .into_inner()
        .holders
}

async fn open_shell(rig: &Rig, machine: &str) -> Result<pb::Tab, Status> {
    rig.host
        .terminals()
        .await
        .open_machine_shell(pb::OpenMachineShellRequest {
            machine: Some(Rig::box_id(machine)),
            ..Default::default()
        })
        .await
        .map(|response| response.into_inner())
}

/// Attaching to the machine shell `tab` through the host, as the CLI does:
/// the tab's machine is the one the open answered with.
fn open_on(tab: &pb::Tab) -> pb::AttachOpen {
    pb::AttachOpen {
        machine: tab.machine.clone(),
        ..open(tab.id.as_ref().expect("the shell has a tab"), 0)
    }
}

/// The box's stored tab `tab`.
fn stored(rig: &Rig, tab: &pb::TabId) -> humata2_store::TabRecord {
    rig.the(BOX)
        .tabs()
        .into_iter()
        .find(|record| record.id == tab.value)
        .expect("the tab is kept on the box")
}

async fn wait_holder(tab: &humata2_store::TabRecord, runs: bool, what: &str) {
    let deadline = Instant::now() + DEADLINE;
    while holder_answers(tab) != runs {
        assert!(Instant::now() < deadline, "{what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_shell_opened_on_a_box_through_the_host_runs_there_and_echoes_typing() {
    let rig = Rig::start(&[BOX], &[]).await;
    let host_tabs = common::stored_tabs(&rig.host.state).len();

    let tab = open_shell(&rig, BOX).await.expect("a shell opens on the box");

    assert_eq!(tab.machine, Some(Rig::box_id(BOX)), "the shell isn't the box's");
    assert_eq!(tab.session, None, "a machine shell has no session");
    let id = tab.id.clone().expect("the shell has a tab");
    assert!(holder_answers(&stored(&rig, &id)), "the box runs no holder for the shell");
    assert_eq!(
        common::stored_tabs(&rig.host.state).len(),
        host_tabs,
        "the host kept a tab for a shell on the box"
    );
    let mut attach = Attach::start(&rig.host, open_on(&tab)).await;
    attach.type_text("echo box-$((40+2))\n").await;
    attach.wait_for("box-42").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_box_shell_is_listed_in_the_boxs_running_holders_as_in_no_channel() {
    let rig = Rig::start(&[BOX], &[]).await;
    let tab = open_shell(&rig, BOX).await.expect("a shell opens on the box");

    let rows = list(&rig).await;
    let row = rows
        .iter()
        .find(|row| row.tab == tab.id)
        .unwrap_or_else(|| panic!("the box shell isn't listed: {rows:?}"));
    assert_eq!(row.session, None, "a machine shell is listed under a session");
    assert_eq!(row.channel, None, "a machine shell is listed in a channel");
    assert_eq!(row.machine, Some(Rig::box_id(BOX)));
    assert!(!row.stopped, "a running box shell is listed as stopped");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_box_shell_stops_and_resumes_by_tab_through_the_host_where_it_was() {
    let rig = Rig::start(&[BOX], &[]).await;
    let tab = open_shell(&rig, BOX).await.expect("a shell opens on the box");
    let id = tab.id.clone().unwrap();
    let mut attach = Attach::start(&rig.host, open_on(&tab)).await;
    attach.type_text("echo stop-$((40+2))\n").await;
    attach.wait_for("stop-42").await;
    drop(attach);
    let holder = stored(&rig, &id);

    machines(&rig)
        .await
        .stop_running_holder(pb::StopRunningHolderRequest {
            machine: Some(Rig::box_id(BOX)),
            tab: Some(id.clone()),
        })
        .await
        .expect("the shell stops through the host");
    wait_holder(&holder, false, "a stopped box shell still answers").await;
    assert!(
        list(&rig).await.iter().any(|row| row.tab.as_ref() == Some(&id) && row.stopped),
        "a stopped box shell isn't listed as stopped"
    );

    machines(&rig)
        .await
        .resume_running_holder(pb::ResumeRunningHolderRequest {
            machine: Some(Rig::box_id(BOX)),
            tab: Some(id.clone()),
        })
        .await
        .expect("the shell resumes through the host");
    wait_holder(&stored(&rig, &id), true, "a resumed box shell never started").await;
    let mut attach = Attach::start(&rig.host, open_on(&tab)).await;
    attach.wait_for("stop-42").await;
    attach.type_text("echo again-$((40+2))\n").await;
    attach.wait_for("again-42").await;
    drop(attach);
    assert!(
        list(&rig).await.iter().any(|row| row.tab.as_ref() == Some(&id) && !row.stopped),
        "a resumed box shell isn't listed as running"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ending_a_box_shell_through_the_host_removes_it() {
    let rig = Rig::start(&[BOX], &[]).await;
    let tab = open_shell(&rig, BOX).await.expect("a shell opens on the box");
    let id = tab.id.clone().unwrap();
    let holder = stored(&rig, &id);

    machines(&rig)
        .await
        .end_running_holder(pb::EndRunningHolderRequest {
            machine: Some(Rig::box_id(BOX)),
            tab: Some(id.clone()),
        })
        .await
        .expect("the shell ends through the host");

    wait_holder(&holder, false, "an ended box shell still answers").await;
    assert!(
        list(&rig).await.iter().all(|row| row.tab.as_ref() != Some(&id)),
        "an ended box shell is still listed"
    );
    assert!(
        rig.the(BOX).tabs().iter().all(|record| record.id != id.value),
        "an ended box shell's tab is kept on the box"
    );
    let refused = Attach::try_start(&rig.host, open_on(&tab)).await;
    assert!(refused.is_err(), "an ended box shell still attaches");
}

#[tokio::test(flavor = "multi_thread")]
async fn opening_a_shell_on_an_offline_box_is_refused_and_nothing_is_queued() {
    let rig = Rig::start(&[BOX], &[]).await;
    let before = rig.the(BOX).tabs().len();
    rig.link_lost(BOX).await;

    let refused = open_shell(&rig, BOX).await.expect_err("a shell opened on an offline box");

    assert_eq!(refused.code(), Code::Unavailable, "{refused:?}");
    rig.link_back(BOX).await;
    // Anything queued would run once the link is back.
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(rig.the(BOX).tabs().len(), before, "the refused open opened a shell later");
}
