//! D845 on a box: a trust box on the screen of a session that runs on
//! box-1 raises no Inbox card, on the box or the host, though the session
//! needs you while it shows.

use humatad_test_common as common;
use humatad_test_common::remote_rig;
use humatad_test_common::trust_box;

use std::path::PathBuf;
use std::time::Duration;

use common::Daemon;
use humata2_grpc::humata::v1 as pb;
use remote_rig::{Rig, is_live};

const REPO: (&str, &str) = ("acme", "api");
const BOX: &str = "box-1";
/// Copilot's trust box needs about 26 rows to show its whole list.
const COPILOT_ROWS: u16 = 30;
/// Past a quiet spell and a look.
const SETTLE: Duration = Duration::from_secs(5);

async fn pending(daemon: &Daemon) -> Vec<pb::InboxRequest> {
    pb::inbox_service_client::InboxServiceClient::new(daemon.channel().await)
        .list_requests(pb::ListRequestsRequest::default())
        .await
        .expect("list box-1's Inbox requests")
        .into_inner()
        .requests
        .into_iter()
        .filter(|request| request.state == pb::RequestState::Pending as i32)
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_trust_box_on_a_box_raises_no_card_but_the_session_needs_you() {
    let rig = Rig::start_seen(&[BOX], &[REPO]).await;
    let conv: PathBuf = rig.the(BOX).home().join("conv");
    std::fs::write(
        rig.the(BOX).home().join("agent.sh"),
        trust_box::agent(&conv, COPILOT_ROWS, true),
    )
    .unwrap();
    let channel = rig
        .create_channel("Payments", &[REPO], &[rig.host_id().await, Rig::box_id(BOX)])
        .await;
    let id = rig.start_on(&channel, &Rig::box_id(BOX), "Fix login").await;
    rig.wait_session(&id, "live", is_live).await;
    trust_box::go(&conv);
    rig.wait_session(&id, "needing you", |session| {
        session.status() == pb::SessionStatus::NeedsInput
    })
    .await;
    tokio::time::sleep(SETTLE).await;
    assert!(pending(rig.the(BOX).daemon()).await.is_empty(), "a card was raised on box-1");
    assert!(pending(&rig.host).await.is_empty(), "a card was raised on the host");
}
