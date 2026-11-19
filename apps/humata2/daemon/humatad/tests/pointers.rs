//! A session's thin pointers (SL-3, SL-4): each machine reports, for each
//! repo of each session, its path, branch, HEAD, upstream, whether it is
//! pushed, its unpushed commits and its dirty files; the host keeps the
//! last report, so a machine that is gone for good still has them on the
//! host (what Forget and Rebuild list as lost).
//!
//! Every wait is on the host's event feed with the rig's LONG as a safety
//! net; nothing depends on how often a machine looks at its clones.

use humatad_test_common as common;
use humatad_test_common::remote_rig;

use std::path::Path;

use common::git;
use humata2_grpc::humata::v1 as pb;
use remote_rig::Rig;

/// Where "Cache warmup"'s clone sits in its channel's folder.
const CLONE: &str = "sessions/cache-warmup/api";

fn text(path: &Path) -> String {
    path.display().to_string()
}

/// The pointer of `repo` (`owner/name`), if the session has one.
fn pointer<'a>(session: &'a pb::Session, name: &str) -> Option<&'a pb::Pointer> {
    session
        .pointers
        .iter()
        .find(|pointer| pointer.repo.as_ref().is_some_and(|repo| repo.name == name))
}

/// Two dirty files and one commit the remote doesn't have.
fn make_work(clone: &Path) {
    std::fs::write(clone.join("README.md"), "changed on the box\n").unwrap();
    std::fs::write(clone.join("new.txt"), "brand new\n").unwrap();
    std::fs::write(clone.join("committed.txt"), "kept\n").unwrap();
    git(&["-C", &text(clone), "add", "committed.txt"]);
    git(&["-C", &text(clone), "commit", "-m", "work on the box"]);
}

fn shows_the_work(session: &pb::Session, head: &str) -> bool {
    pointer(session, "api").is_some_and(|pointer| {
        pointer.dirty_files == 2
            && pointer.unpushed_commits == 1
            && !pointer.pushed
            && pointer.head_sha == head
            && pointer.reported_at.is_some()
    })
}

/// SL-3: work done in a box's clone shows on the host, with the branch,
/// HEAD and counts the box's clone has.
#[tokio::test(flavor = "multi_thread")]
async fn a_box_sessions_pointers_reach_the_host_with_dirty_and_unpushed_counts() {
    let rig = Rig::start(&["box-1"], &[("acme", "api")]).await;
    let box_1 = Rig::box_id("box-1");
    let channel = rig.create_channel("Payments", &[("acme", "api")], &[rig.host_id().await, box_1.clone()]).await;
    let id = rig.start_on(&channel, &box_1, "Cache warmup").await;
    let session = rig.wait_session(&id, "with pointers", |session| pointer(session, "api").is_some()).await;
    let first = pointer(&session, "api").unwrap().clone();
    assert_eq!(first.branch, session.repos[0].branch);
    assert_eq!(first.dirty_files, 0);
    assert_eq!(first.unpushed_commits, 0);

    let clone = rig.the("box-1").path(&channel, CLONE);
    make_work(&clone);
    let head = git(&["-C", &text(&clone), "rev-parse", "HEAD"]);

    let shown = rig.wait_session(&id, "showing the box's work", |session| shows_the_work(session, &head)).await;
    let pointer = pointer(&shown, "api").unwrap();
    assert_eq!(pointer.branch, first.branch);
    assert_eq!(std::fs::canonicalize(&pointer.path).unwrap(), std::fs::canonicalize(&clone).unwrap());
}

/// SL-4: the host keeps a box's last pointers through the box going away
/// for good and the host restarting.
#[tokio::test(flavor = "multi_thread")]
async fn the_host_keeps_a_gone_boxs_last_pointers_through_its_own_restart() {
    let mut rig = Rig::start(&["box-1"], &[("acme", "api")]).await;
    let box_1 = Rig::box_id("box-1");
    let channel = rig.create_channel("Payments", &[("acme", "api")], &[rig.host_id().await, box_1.clone()]).await;
    let id = rig.start_on(&channel, &box_1, "Cache warmup").await;
    rig.wait_session(&id, "with pointers", |session| pointer(session, "api").is_some()).await;
    let clone = rig.the("box-1").path(&channel, CLONE);
    make_work(&clone);
    let head = git(&["-C", &text(&clone), "rev-parse", "HEAD"]);
    rig.wait_session(&id, "showing the box's work", |session| shows_the_work(session, &head)).await;

    rig.link_lost("box-1").await;
    rig.the_mut("box-1").kill();
    rig.restart_host(&[]).await;

    let after = rig.session(&id).await;
    assert!(shows_the_work(&after, &head), "the host lost box-1's last pointers: {:?}", after.pointers);
}

/// SL-3 on the host's own sessions: the same pointers, read from its clone.
#[tokio::test(flavor = "multi_thread")]
async fn a_session_on_this_mac_shows_its_own_pointers() {
    let rig = Rig::start(&[], &[("acme", "api")]).await;
    let host = rig.host_id().await;
    let channel = rig.create_channel("Payments", &[("acme", "api")], &[host.clone()]).await;
    let id = rig.start_on(&channel, &host, "Cache warmup").await;
    rig.wait_session(&id, "with pointers", |session| pointer(session, "api").is_some()).await;
    let clone = rig.host_path(&channel, CLONE);
    make_work(&clone);
    let head = git(&["-C", &text(&clone), "rev-parse", "HEAD"]);
    rig.wait_session(&id, "showing its work", |session| shows_the_work(session, &head)).await;
}

/// BUG 23: a file an agent makes in its session's clone is told on the
/// feed as created, without anyone asking for the list again.
#[tokio::test(flavor = "multi_thread")]
async fn a_file_made_in_a_live_sessions_clone_is_told_on_the_feed() {
    let rig = Rig::start(&[], &[("acme", "api")]).await;
    let host = rig.host_id().await;
    let channel = rig.create_channel("Payments", &[("acme", "api")], &[host.clone()]).await;
    let id = rig.start_on(&channel, &host, "Cache warmup").await;
    // The pointers come from the first look, which also remembers the clone's files.
    rig.wait_session(&id, "with pointers", |session| pointer(session, "api").is_some()).await;
    let mut events = pb::event_service_client::EventServiceClient::new(rig.channel().await)
        .subscribe(pb::SubscribeRequest {
            sessions: vec![id.clone()],
            topics: vec![pb::Topic::Sessions as i32],
            ..pb::SubscribeRequest::default()
        })
        .await
        .unwrap()
        .into_inner();

    let clone = rig.host_path(&channel, CLONE);
    std::fs::write(clone.join("agent_made.txt"), "from the agent\n").unwrap();

    let deadline = tokio::time::Instant::now() + remote_rig::LONG;
    loop {
        let event = tokio::time::timeout_at(deadline, events.message())
            .await
            .expect("the new file was never told on the feed")
            .unwrap()
            .expect("the feed ended");
        if let Some(pb::event::Payload::FileChanged(changed)) = event.payload
            && changed.path == "agent_made.txt"
        {
            assert_eq!(changed.kind, pb::file_changed::Kind::Created as i32);
            return;
        }
    }
}
