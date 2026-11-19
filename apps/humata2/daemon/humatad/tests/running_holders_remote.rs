//! D784 on another machine: the host lists, stops, resumes and ends the
//! running holders of a box, relayed to the box's own daemon. A shell
//! stops and resumes where it was; an agent's tab is its session's to
//! stop, resume or end; a box that is offline fails the call clearly.

use humatad_test_common as common;
use humatad_test_common::remote_rig;

use std::time::{Duration, Instant};

use common::{Attach, agent_tab, holder_answers, open, stored_tabs};
use humata2_grpc::humata::v1 as pb;
use remote_rig::{Rig, is_live};
use tonic::Status;
use tonic::transport::Channel;

const REPO: (&str, &str) = ("acme", "api");
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

async fn stop(rig: &Rig, tab: &pb::TabId) -> Result<(), Status> {
    machines(rig)
        .await
        .stop_running_holder(pb::StopRunningHolderRequest {
            machine: Some(Rig::box_id(BOX)),
            tab: Some(tab.clone()),
        })
        .await
        .map(drop)
}

async fn resume(rig: &Rig, tab: &pb::TabId) -> Result<(), Status> {
    machines(rig)
        .await
        .resume_running_holder(pb::ResumeRunningHolderRequest {
            machine: Some(Rig::box_id(BOX)),
            tab: Some(tab.clone()),
        })
        .await
        .map(drop)
}

async fn end(rig: &Rig, tab: &pb::TabId) -> Result<(), Status> {
    machines(rig)
        .await
        .end_running_holder(pb::EndRunningHolderRequest {
            machine: Some(Rig::box_id(BOX)),
            tab: Some(tab.clone()),
        })
        .await
        .map(drop)
}

/// A shell tab of a session on the box, opened through the host as the app
/// opens one (the host opens no machine shell on another machine yet).
async fn box_shell(rig: &Rig, id: &pb::SessionId) -> pb::TabId {
    rig.host
        .terminals()
        .await
        .open_shell_tab(pb::OpenShellTabRequest { session: Some(id.clone()), ..Default::default() })
        .await
        .expect("a shell tab opens on the box")
        .into_inner()
        .id
        .expect("the shell has a tab")
}

/// Attaching to `tab` of the box session `id`, through the host.
fn open_in(id: &pb::SessionId, tab: &pb::TabId) -> pb::AttachOpen {
    pb::AttachOpen { session: Some(id.clone()), ..open(tab, 0) }
}

/// The box's stored tab `tab`.
fn stored(rig: &Rig, tab: &pb::TabId) -> humata2_store::TabRecord {
    stored_tabs(&rig.the(BOX).daemon().state)
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

/// A rig with a session running on the box.
async fn with_session() -> (Rig, pb::SessionId) {
    let rig = Rig::start(&[BOX], &[REPO]).await;
    let (host, one) = (rig.host_id().await, Rig::box_id(BOX));
    let channel = rig.create_channel("Payments", &[REPO], &[host, one.clone()]).await;
    let id = rig.start_on(&channel, &one, "Fix login").await;
    rig.wait_session(&id, "live", is_live).await;
    (rig, id)
}

#[tokio::test(flavor = "multi_thread")]
async fn listing_the_box_shows_its_session_holders_and_none_of_the_hosts() {
    let (rig, id) = with_session().await;
    let host_shell = rig
        .host
        .terminals()
        .await
        .open_machine_shell(pb::OpenMachineShellRequest::default())
        .await
        .unwrap()
        .into_inner()
        .id
        .unwrap();

    let rows = list(&rig).await;
    assert!(
        rows.iter().any(|row| row.session.as_ref() == Some(&id)),
        "the box's session holder isn't listed: {rows:?}"
    );
    assert!(
        rows.iter().all(|row| row.machine.as_ref() == Some(&Rig::box_id(BOX))),
        "a row isn't the box's: {rows:?}"
    );
    assert!(
        rows.iter().all(|row| row.tab.as_ref() != Some(&host_shell)),
        "a host-only holder is listed for the box: {rows:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_box_shell_stops_and_resumes_through_the_host_where_it_was() {
    let (rig, id) = with_session().await;
    let tab = box_shell(&rig, &id).await;
    let mut attach = Attach::start(&rig.host, open_in(&id, &tab)).await;
    attach.type_text("echo stop-$((40+2))\n").await;
    attach.wait_for("stop-42").await;
    drop(attach);
    let holder = stored(&rig, &tab);

    stop(&rig, &tab).await.expect("the shell stops through the host");
    wait_holder(&holder, false, "a stopped box shell still answers").await;
    let row = list(&rig)
        .await
        .into_iter()
        .find(|row| row.tab.as_ref() == Some(&tab))
        .expect("a stopped box shell isn't listed");
    assert!(row.stopped, "a stopped box shell is listed as running");

    resume(&rig, &tab).await.expect("the shell resumes through the host");
    wait_holder(&stored(&rig, &tab), true, "a resumed box shell never started").await;
    let mut attach = Attach::start(&rig.host, open_in(&id, &tab)).await;
    attach.wait_for("stop-42").await;
    drop(attach);
    assert!(
        list(&rig).await.iter().any(|row| row.tab.as_ref() == Some(&tab) && !row.stopped),
        "a resumed box shell isn't listed as running"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ending_a_box_shell_through_the_host_removes_it_from_the_box() {
    let (rig, id) = with_session().await;
    let tab = box_shell(&rig, &id).await;
    let holder = stored(&rig, &tab);
    assert!(list(&rig).await.iter().any(|row| row.tab.as_ref() == Some(&tab)));

    end(&rig, &tab).await.expect("the shell ends through the host");

    wait_holder(&holder, false, "an ended box shell still answers").await;
    assert!(
        list(&rig).await.iter().all(|row| row.tab.as_ref() != Some(&tab)),
        "an ended box shell is still listed"
    );
    assert!(
        rig.the(BOX).tabs().iter().all(|record| record.id != tab.value),
        "an ended box shell's tab is kept on the box"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_box_agent_tab_is_refused_through_the_host() {
    let (rig, id) = with_session().await;
    let agent = agent_tab(&rig.host, &id).await;

    for refused in [
        stop(&rig, &agent).await,
        resume(&rig, &agent).await,
        end(&rig, &agent).await,
    ] {
        assert_eq!(refused.unwrap_err().code(), tonic::Code::FailedPrecondition);
    }
    assert!(holder_answers(&stored(&rig, &agent)), "a refused call touched the agent");
}

#[tokio::test(flavor = "multi_thread")]
async fn with_the_box_offline_stopping_and_ending_a_shell_fail() {
    let (rig, id) = with_session().await;
    let tab = box_shell(&rig, &id).await;
    let holder = stored(&rig, &tab);
    rig.link_lost(BOX).await;

    assert!(stop(&rig, &tab).await.is_err(), "a stop reached an offline box");
    assert!(end(&rig, &tab).await.is_err(), "an end reached an offline box");

    assert!(holder_answers(&holder), "a failed call touched the box's shell");
}
