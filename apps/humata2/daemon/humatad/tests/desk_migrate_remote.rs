//! Desk folders on another machine get their versions repository (D458,
//! SAFE): the host posts one "Migrate the Desk folders" item for each
//! machine whose folders lack it, asked over the link; Migrate there makes
//! each a repository with no commit; a machine that is away leaves its item
//! as it was, never holds up the host's start, and is asked again when its
//! link comes up. A host and one box on the multi-machine rig. Waits for the
//! host's Inbox are bounded by `LONG`, the rig's safety net.

use humatad_test_common as common;
use humatad_test_common::remote_rig;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use common::approve;
use humata2_grpc::humata::v1 as pb;
use remote_rig::{LONG, Rig, is_live};
use tonic::transport::Channel;

const BOX: &str = "box-1";
const REPO: (&str, &str) = ("acme", "api");

async fn desks(rig: &Rig) -> pb::desk_service_client::DeskServiceClient<Channel> {
    pb::desk_service_client::DeskServiceClient::new(rig.channel().await)
}

/// A plain Desk session on the box, live; the folder it works in there.
async fn on_box(rig: &Rig) -> PathBuf {
    let started = desks(rig)
        .await
        .start_desk_session(pb::StartDeskSessionRequest {
            idempotency_key: Some(pb::IdempotencyKey { value: "desk-old-notes".to_owned() }),
            title: "Old notes".to_owned(),
            machine: Some(Rig::box_id(BOX)),
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
    rig.wait_session(&id, "live", is_live).await;
    PathBuf::from(started.desk.and_then(|desk| desk.folder).expect("a start says where it works").path)
}

/// The Desk folders items waiting for the user, for the machine `name`.
async fn pending_for(rig: &Rig, name: &str) -> Vec<pb::InboxRequest> {
    rig.host
        .inbox_service()
        .await
        .list_requests(pb::ListRequestsRequest::default())
        .await
        .expect("the host lists its Inbox")
        .into_inner()
        .requests
        .into_iter()
        .filter(|request| request.state() == pb::RequestState::Pending)
        .filter(|request| match request.preview.as_ref().and_then(|preview| preview.preview.as_ref()) {
            Some(pb::request_preview::Preview::Migrate(card)) => {
                card.thing() == pb::MigrateThing::DeskFolders
                    && card.machine.as_ref().is_some_and(|machine| machine.value == name)
            }
            _ => false,
        })
        .collect()
}

/// The Inbox has no push a test can hold, so it is read again until `want`
/// holds, up to `LONG`.
async fn until(rig: &Rig, what: &str, want: impl Fn(&[pb::InboxRequest]) -> bool) -> Vec<pb::InboxRequest> {
    let deadline = Instant::now() + LONG;
    loop {
        let items = pending_for(rig, BOX).await;
        if want(&items) {
            return items;
        }
        assert!(Instant::now() < deadline, "{what}; the host's items for {BOX}: {items:?}");
        tokio::task::yield_now().await;
    }
}

fn has_commit(folder: &Path) -> bool {
    Command::new("git")
        .args(["-C", &folder.display().to_string(), "rev-parse", "--verify", "-q", "HEAD"])
        .output()
        .expect("git runs")
        .status
        .success()
}

/// A folder as an older release made it: no repository.
fn without_repo(folder: &Path) {
    std::fs::remove_dir_all(folder.join(".git")).expect("the box's folder has a repository to remove");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_box_desk_folder_without_a_repository_gets_the_host_a_migrate_item_and_one_on_migrate() {
    let rig = Rig::start(&[BOX], &[REPO]).await;
    let folder = on_box(&rig).await;
    assert!(pending_for(&rig, BOX).await.is_empty(), "a folder made now is a repository: no item");
    without_repo(&folder);
    // The host asks a machine when its link comes up.
    rig.link_lost(BOX).await;
    rig.link_back(BOX).await;

    let items = until(&rig, "no Migrate item for the box's Desk folders reached the host's Inbox", |items| !items.is_empty()).await;
    assert_eq!(items.len(), 1);
    assert!(items[0].vip, "a Migrate item is always VIP");
    assert!(!folder.join(".git").exists(), "nothing changes before the user migrates");

    approve(&rig.host, items[0].id.as_ref().expect("an item has an id")).await;

    until(&rig, "the item stayed pending after Migrate", |items| items.is_empty()).await;
    assert!(folder.join(".git").is_dir(), "the box's folder became a repository");
    assert!(!has_commit(&folder), "migrating saves no version");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_box_that_is_away_never_holds_up_the_host_and_is_asked_when_its_link_comes_up() {
    let mut rig = Rig::start(&[BOX], &[REPO]).await;
    let folder = on_box(&rig).await;
    without_repo(&folder);
    rig.link_lost(BOX).await;

    rig.restart_host_unlinked(&[]).await;
    desks(&rig)
        .await
        .list_desk_sessions(pb::ListDeskSessionsRequest::default())
        .await
        .expect("the host serves while the box is away");
    assert!(pending_for(&rig, BOX).await.is_empty(), "nothing is known of the box's folders while it is away");

    rig.link_back(BOX).await;
    until(&rig, "the box's link came up but its Migrate item never reached the Inbox", |items| !items.is_empty()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_migrated_box_posts_no_item_again() {
    let rig = Rig::start(&[BOX], &[REPO]).await;
    let folder = on_box(&rig).await;
    without_repo(&folder);
    rig.link_lost(BOX).await;
    rig.link_back(BOX).await;
    let items = until(&rig, "no Migrate item reached the Inbox", |items| !items.is_empty()).await;
    approve(&rig.host, items[0].id.as_ref().expect("an item has an id")).await;
    until(&rig, "the item stayed pending after Migrate", |items| items.is_empty()).await;

    rig.link_lost(BOX).await;
    rig.link_back(BOX).await;
    assert!(pending_for(&rig, BOX).await.is_empty(), "a migrated folder posts nothing when the link comes back");
}
