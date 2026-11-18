//! D786 on a box: a trust box a short screen clips on a session that runs
//! on box-1 grows the pty there only while no client with input is attached,
//! one whose attach the host relays included. With a relayed writer
//! attached, the card offers no choice (D785); once a relayed writer sized
//! the screen and left, the box grows it to 50 rows and the card offers
//! every choice.

mod common;
#[path = "common/remote_rig.rs"]
mod remote_rig;
#[path = "common/trust_box.rs"]
mod trust_box;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use common::{Attach, Daemon, WAIT};
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

fn choices(request: &pb::InboxRequest) -> Vec<String> {
    match request.preview.as_ref().and_then(|preview| preview.preview.as_ref()) {
        Some(pb::request_preview::Preview::Question(question)) => question.choices.clone(),
        other => panic!("not a question card: {other:?}"),
    }
}

/// Waits for a pending card on box-1 that `wanted` takes.
async fn wait_card(daemon: &Daemon, wanted: impl Fn(&pb::InboxRequest) -> bool) -> pb::InboxRequest {
    let deadline = Instant::now() + WAIT * 3;
    loop {
        if let Some(card) = pending(daemon).await.into_iter().find(|card| wanted(card)) {
            return card;
        }
        assert!(Instant::now() < deadline, "no such card came on box-1");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// The session's agent tab, as box-1 keeps it.
fn agent_tab(rig: &Rig, id: &pb::SessionId) -> pb::TabId {
    let agent = rig
        .the(BOX)
        .store()
        .agent(&id.value)
        .unwrap()
        .expect("box-1 keeps the session's agent");
    pb::TabId { value: agent.tab }
}

/// Starts a session on box-1, attaches a writer through the host at 80x24,
/// and returns the session, its tab, the writer and a reader on box-1 once
/// the reader sees that size and that writer.
async fn started_short(rig: &Rig, channel: &pb::Channel, title: &str) -> (pb::SessionId, pb::TabId, Attach, Attach) {
    let id = rig.start_on(channel, &Rig::box_id(BOX), title).await;
    rig.wait_session(&id, "live", is_live).await;
    let tab = agent_tab(rig, &id);
    let writer = Attach::start(&rig.host, trust_box::writer_open(Some(&id), &tab, 80, 24)).await;
    let mut reader = Attach::start(rig.the(BOX).daemon(), trust_box::reader_open(None, &tab)).await;
    trust_box::wait_state(&mut reader, |state| state.rows == 24 && state.other_writers == 1).await;
    (id, tab, writer, reader)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_box_grows_a_clipped_trust_box_only_with_no_relayed_writer_attached() {
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
    let on_box = rig.the(BOX).daemon();

    // A writer the host relays blocks the grow: the card offers no choice.
    let (blocked, _, writer, reader) = started_short(&rig, &channel, "Fix login").await;
    trust_box::go(&conv);
    let card = wait_card(on_box, |_| true).await;
    assert!(choices(&card).is_empty(), "{:?}", choices(&card));
    tokio::time::sleep(SETTLE).await;
    assert_eq!(trust_box::sizes(&conv), ["24 80"]);
    assert_eq!(reader.states.last().map(|state| state.rows), Some(24));
    drop((writer, reader));
    pb::session_service_client::SessionServiceClient::new(rig.channel().await)
        .stop_session(pb::StopSessionRequest {
            idempotency_key: None,
            id: Some(blocked),
            for_move: false,
        })
        .await
        .expect("the first session stops");
    let deadline = Instant::now() + WAIT;
    while !pending(on_box).await.is_empty() {
        assert!(Instant::now() < deadline, "the stopped session's card is still pending");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    std::fs::remove_file(conv.join("go")).unwrap();
    std::fs::remove_file(conv.join("sizes")).unwrap();

    // A relayed writer that sized the screen and left: the box grows it.
    let (_, _, writer, mut reader) = started_short(&rig, &channel, "Fix signup").await;
    drop(writer);
    trust_box::wait_state(&mut reader, |state| state.other_writers == 0).await;
    trust_box::go(&conv);
    let state = trust_box::wait_state(&mut reader, |state| state.rows == 50).await;
    assert_eq!(state.cols, 80);
    let card = wait_card(on_box, |card| choices(card).len() == 3).await;
    assert!(card.on_screen.is_some());
    assert_eq!(trust_box::sizes(&conv), ["24 80", "50 80"]);
}
