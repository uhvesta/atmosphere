//! A plain start cut short on a box (D150, PER-27, PER-29; SAFE): the box is
//! killed while it launches a session's agent, and the restart can't take the
//! session back, so it names the start stuck. The host's start was answered
//! as running before the cut, and its restart finds the box's operation lost
//! and the session still starting. The host's start ends Failed, with the
//! box's own words, instead of waiting for a report that never comes; it
//! keeps its record of the session, which the box still has. Waits are
//! bounded by `LONG`.
//!
//! As in `desk_stuck_remote`, the undo is made to fail by taking the write
//! permission off the session's folder, which only a user other than root is
//! held to, so the test says it is skipped when it runs as root. Finishing
//! the start is made to fail by removing the channel's record from the box's
//! store while the box is down, so the restart can't launch the agent.

mod common;
#[path = "common/remote_rig.rs"]
mod remote_rig;

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use humata2_grpc::humata::v1 as pb;
use remote_rig::Rig;

const BOX: &str = "box-1";
const REPO: (&str, &str) = ("acme", "api");
/// Where the box is killed: its session recorded, its agent not launched.
const KILLED_AT: &str = "sessions.start.launching";

/// Gives a folder its write permission back when dropped, so the test's
/// temporary folder can be removed even when an assertion fails.
struct Writable(PathBuf);

impl Drop for Writable {
    fn drop(&mut self) {
        let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plain_start_cut_on_a_box_and_named_stuck_ends_failed_on_the_host() {
    if rustix::process::geteuid().is_root() {
        eprintln!("skipped: root can write to a folder that has no write permission, so the undo this test needs to fail would succeed");
        return;
    }
    // The box is the `--admin` test daemon, so its restart's account is
    // read from its server info as the user.
    let mut rig = Rig::start_seen_with_env(&[BOX], &[REPO], &[("HUMATA2_STOP_AT", KILLED_AT)]).await;
    let on = Rig::box_id(BOX);
    let channel = rig.create_channel("stuck-start", &[REPO], std::slice::from_ref(&on)).await;

    // The host's start is answered as running, then the box dies launching.
    let started = pb::session_service_client::SessionServiceClient::new(rig.channel().await)
        .start_session(common::start_request(channel.id.as_ref().expect("a channel has an id"), &on, "Stuck start"))
        .await
        .expect("the host answers a start the box has recorded")
        .into_inner();
    let session = started.session.expect("a start names its session");
    let operation = started.operation.expect("a start has an operation");
    rig.box_stopped(BOX).await;

    // The box kept the one session it was killed starting.
    let kept: Vec<_> = rig
        .the(BOX)
        .store()
        .sessions()
        .expect("box-1's store reads")
        .into_iter()
        .filter(|kept| kept.id == session.value)
        .collect();
    assert_eq!(kept.len(), 1, "box-1 kept the session it was killed starting: {kept:?}");
    let (folder, box_channel) = (kept[0].folder.clone(), kept[0].channel.clone());

    // The restart can't take it back: the channel is gone from the box's
    // store, so finishing the start fails, and a folder that isn't writable
    // can't be removed, so the undo fails too.
    rig.the(BOX).store().remove_channel(&box_channel).expect("the channel's record is removed while box-1 is down");
    let _writable = Writable(folder.clone());
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o500)).unwrap();
    rig.restart_box_env(BOX, &[]).await;
    let last = common::last_start(rig.the(BOX).daemon()).await;
    assert!(
        last.not_brought_back.iter().any(|line| line.session == session.value),
        "the restart names {} as one it couldn't finish or remove: {:?}",
        session.value,
        last.not_brought_back
    );

    // The host's start ends Failed within LONG (`ended` bounds the wait),
    // naming what to do, and the host still shows the session.
    let ended = rig.ended(&operation).await;
    assert_eq!(ended.state, pb::OperationState::Failed as i32, "the host's start of a stuck session ended: {ended:?}");
    assert!(ended.error.is_some_and(|error| !error.fix.is_empty()), "the failure tells the person what to do");
    let shown = rig.session(&session).await;
    assert_eq!(shown.id.map(|id| id.value), Some(session.value), "the host kept its record of the session");
}
