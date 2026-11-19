//! `DaemonService.RepairRestart` (PER-30, D242) through a real `humatad2`:
//! what the last start didn't bring back is brought back again, and a
//! store with nothing wrong isn't touched.

use humatad_test_common as common;

use common::{
    Attach, Daemon, create_channel, finished, kill_holders, last_start, open, start_session,
    start_world, stored_tabs, wait_status,
};
use humata2_grpc::humata::v1 as pb;
use pb::SessionStatus as Status;
use pb::repair_restart_request::Target;

async fn repair(daemon: &Daemon, target: Target) -> Result<pb::RepairRestartResponse, tonic::Status> {
    pb::daemon_service_client::DaemonServiceClient::new(daemon.channel().await)
        .repair_restart(pb::RepairRestartRequest {
            target: Some(target),
        })
        .await
        .map(tonic::Response::into_inner)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_that_was_not_brought_back_is_repaired_once_what_it_needed_is_back() {
    let (mut daemon, world) = start_world().await;
    world.remote("acme", "api");
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    let (id, _) = start_session(&daemon, &channel, "Fix login").await;
    let session = wait_status(&daemon, &id, Status::Working).await;
    let conversation = world.conversation(&session.conversation_id);
    let kept = std::fs::read(&conversation).unwrap();
    let tabs: Vec<_> = stored_tabs(&daemon.state)
        .into_iter()
        .filter(|tab| tab.session.as_deref() == Some(id.value.as_str()))
        .collect();
    // A reboot that also lost the conversation file.
    daemon.kill();
    kill_holders(&tabs).await;
    std::fs::remove_file(&conversation).unwrap();
    daemon.restart().await;
    wait_status(&daemon, &id, Status::ResumeFailed).await;
    std::fs::write(&conversation, &kept).unwrap();

    let response = repair(&daemon, Target::Session(id.clone())).await.unwrap();

    let operation = finished(&daemon, &response.operation.unwrap()).await;
    assert_eq!(operation.state, pb::OperationState::Succeeded as i32, "{:?}", operation.error);
    let after = wait_status(&daemon, &id, Status::Working).await;
    assert!(after.resume_failed.is_none());
    assert_eq!(after.conversation_id, session.conversation_id);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_shell_tab_that_was_not_brought_back_is_started_again() {
    let mut daemon = Daemon::start().await;
    let tab = daemon
        .terminals()
        .await
        .open_machine_shell(pb::OpenMachineShellRequest::default())
        .await
        .unwrap()
        .into_inner()
        .id
        .unwrap();
    let tabs = stored_tabs(&daemon.state);
    daemon.kill();
    kill_holders(&tabs).await;
    // What the holder saved can't be read back, so the start gives up on it.
    let saved = tabs[0].holder_socket.with_extension("scrollback");
    let _ = std::fs::remove_file(&saved);
    std::fs::create_dir(&saved).unwrap();
    daemon.restart().await;
    let start = last_start(&daemon).await;
    assert_eq!(start.not_brought_back.len(), 1, "{:?}", start.not_brought_back);
    std::fs::remove_dir(&saved).unwrap();

    repair(&daemon, Target::Tab(tab.clone())).await.unwrap();

    let mut after = Attach::start(&daemon, open(&tab, 0)).await;
    after.type_text("echo bac''k\n").await;
    after.wait_for("back").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_store_with_nothing_wrong_is_refused() {
    let daemon = Daemon::start().await;

    let refused = repair(&daemon, Target::Store(true)).await.unwrap_err();

    assert_eq!(refused.code(), tonic::Code::FailedPrecondition);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_repair_with_no_target_is_refused() {
    let daemon = Daemon::start().await;

    let refused = pb::daemon_service_client::DaemonServiceClient::new(daemon.channel().await)
        .repair_restart(pb::RepairRestartRequest { target: None })
        .await
        .unwrap_err();

    assert_eq!(refused.code(), tonic::Code::InvalidArgument);
}
