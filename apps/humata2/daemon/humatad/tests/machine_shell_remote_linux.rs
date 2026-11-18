//! HO-6 on a Linux box, for the Linux VM batch (D784's remote amendment:
//! Linux remotes run the same daemon and holder code). Built for Linux
//! only (`LINUX_VM_TESTS` in BUILD.bazel), so it runs when the batch runs
//! `//apps/humata2/daemon/...` in the VM and is skipped on the Mac.
//!
//! A machine shell opened on the box through the host is a Linux shell
//! there; its holder is a process the box's /proc shows while it runs and
//! not once it is stopped or ended through the host.

mod common;
#[path = "common/remote_rig.rs"]
mod remote_rig;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use common::{Attach, open};
use humata2_grpc::humata::v1 as pb;
use remote_rig::Rig;
use tonic::transport::Channel;

const BOX: &str = "box-1";
const DEADLINE: Duration = Duration::from_secs(30);

async fn machines(rig: &Rig) -> pb::machine_service_client::MachineServiceClient<Channel> {
    pb::machine_service_client::MachineServiceClient::new(rig.channel().await)
}

/// The box's row for `tab`, if it lists one.
async fn row(rig: &Rig, tab: &pb::TabId) -> Option<pb::RunningHolder> {
    machines(rig)
        .await
        .list_running_holders(pb::ListRunningHoldersRequest { machine: Some(Rig::box_id(BOX)) })
        .await
        .expect("the box's holders are listed")
        .into_inner()
        .holders
        .into_iter()
        .find(|row| row.tab.as_ref() == Some(tab))
}

fn proc_of(pid: u32) -> PathBuf {
    PathBuf::from(format!("/proc/{pid}"))
}

async fn wait_gone(pid: u32, what: &str) {
    let deadline = Instant::now() + DEADLINE;
    while proc_of(pid).exists() {
        assert!(Instant::now() < deadline, "{what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn running_pid(rig: &Rig, tab: &pb::TabId) -> u32 {
    let deadline = Instant::now() + DEADLINE;
    loop {
        if let Some(row) = row(rig, tab).await
            && !row.stopped
            && row.pid != 0
        {
            return row.pid;
        }
        assert!(Instant::now() < deadline, "the box never listed the shell as running");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_linux_box_shell_runs_stops_resumes_and_ends_through_the_host() {
    let rig = Rig::start(&[BOX], &[]).await;
    let tab = rig
        .host
        .terminals()
        .await
        .open_machine_shell(pb::OpenMachineShellRequest {
            machine: Some(Rig::box_id(BOX)),
            ..Default::default()
        })
        .await
        .expect("a shell opens on the box")
        .into_inner();
    let id = tab.id.clone().expect("the shell has a tab");
    let attach_open = || pb::AttachOpen { machine: tab.machine.clone(), ..open(&id, 0) };

    let mut attach = Attach::start(&rig.host, attach_open()).await;
    attach.type_text("echo os-$(uname -s)\n").await;
    attach.wait_for("os-Linux").await;
    drop(attach);
    let first = running_pid(&rig, &id).await;
    assert!(proc_of(first).exists(), "the listed holder isn't a process on the box");

    machines(&rig)
        .await
        .stop_running_holder(pb::StopRunningHolderRequest { machine: Some(Rig::box_id(BOX)), tab: Some(id.clone()) })
        .await
        .expect("the shell stops through the host");
    wait_gone(first, "a stopped box shell's holder still runs").await;

    machines(&rig)
        .await
        .resume_running_holder(pb::ResumeRunningHolderRequest { machine: Some(Rig::box_id(BOX)), tab: Some(id.clone()) })
        .await
        .expect("the shell resumes through the host");
    let second = running_pid(&rig, &id).await;
    let mut attach = Attach::start(&rig.host, attach_open()).await;
    attach.wait_for("os-Linux").await;
    drop(attach);

    machines(&rig)
        .await
        .end_running_holder(pb::EndRunningHolderRequest { machine: Some(Rig::box_id(BOX)), tab: Some(id.clone()) })
        .await
        .expect("the shell ends through the host");
    wait_gone(second, "an ended box shell's holder still runs").await;
    assert!(row(&rig, &id).await.is_none(), "an ended box shell is still listed");
}
