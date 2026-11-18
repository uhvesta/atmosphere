//! DESKWATCH: a Desk session on a box is told on the host's feed when the
//! box changes it (the agent saves a version), with no call to the host
//! and no poll. A host and one box on the multi-machine rig.

mod common;
#[path = "common/remote_rig.rs"]
mod remote_rig;

use common::{WAIT, git};
use humata2_grpc::humata::v1 as pb;
use remote_rig::{Rig, is_live};

const BOX: &str = "box-1";
const REPO: (&str, &str) = ("acme", "api");

#[tokio::test(flavor = "multi_thread")]
async fn a_version_saved_on_a_box_is_told_on_the_hosts_feed() {
    let rig = Rig::start(&[BOX], &[REPO]).await;
    let started = pb::desk_service_client::DeskServiceClient::new(rig.channel().await)
        .start_desk_session(pb::StartDeskSessionRequest {
            idempotency_key: Some(pb::IdempotencyKey { value: "desk-watch".to_owned() }),
            title: "Plan".to_owned(),
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
    let folder = started.desk.and_then(|desk| desk.folder).expect("a start says where it works").path;

    let mut feed = pb::event_service_client::EventServiceClient::new(rig.channel().await)
        .subscribe(pb::SubscribeRequest {
            topics: vec![pb::Topic::Sessions as i32],
            ..pb::SubscribeRequest::default()
        })
        .await
        .unwrap()
        .into_inner();

    // What the agent does on the box: write a file and commit it.
    std::fs::write(std::path::Path::new(&folder).join("plan.md"), "one\n").unwrap();
    git(&["-C", &folder, "add", "-A"]);
    git(&["-C", &folder, "-c", "core.hooksPath=/dev/null", "commit", "-q", "-m", "First"]);
    let saved = git(&["-C", &folder, "rev-parse", "HEAD"]);

    // Other changes of the session (its agent going live, idle) may be told
    // between the write and the commit, naming plan.md with no version yet;
    // the one that matters carries the version just saved.
    let told = tokio::time::timeout(WAIT, async {
        loop {
            let event = feed.message().await.unwrap().expect("the feed ended");
            if let Some(pb::event::Payload::Desk(desk)) = event.payload
                && desk.session.as_ref() == Some(&id)
                && desk.documents.iter().any(|name| name == "plan.md")
                && !desk.doc_head.is_empty()
            {
                return desk;
            }
        }
    })
    .await
    .expect("the host's feed never told of the box's new version");
    assert_eq!(told.doc_head, saved, "the newest version came with it");
}
