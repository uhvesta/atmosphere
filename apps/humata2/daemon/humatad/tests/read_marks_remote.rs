//! Read marks of a session on another machine (CH-16, CH-17; D394): the
//! machine marks its own session unread when its agent prints and nobody
//! views it, tells the host in its status report, and the host shows the
//! mark. Viewing the session on the host holds the machine's own view
//! open, which clears the mark there and so here. A host and one box on
//! the multi-machine rig; every wait is on the host's Sessions feed.

use humatad_test_common as common;
use humatad_test_common::remote_rig;

use humata2_grpc::humata::v1 as pb;
use pb::mark_service_client::MarkServiceClient;
use remote_rig::{Rig, is_live};

const REPO: (&str, &str) = ("acme", "api");

/// A host, one box and a live session on the box.
async fn started() -> (Rig, pb::SessionId) {
    let rig = Rig::start(&["box-1"], &[REPO]).await;
    let one = Rig::box_id("box-1");
    let channel = rig.create_channel("Payments", &[REPO], &[rig.host_id().await, one.clone()]).await;
    let id = rig.start_on(&channel, &one, "Fix login").await;
    rig.wait_session(&id, "live", is_live).await;
    (rig, id)
}

/// Types `line` into the box's agent through the host's Type call, which
/// relays it. Typing is not a view: it marks nothing read.
async fn type_line(rig: &Rig, id: &pb::SessionId, line: &str) {
    rig.host
        .terminals()
        .await
        .r#type(pb::TypeRequest {
            idempotency_key: Some(pb::IdempotencyKey {
                value: format!("type-{line}"),
            }),
            session: Some(id.clone()),
            text: format!("{line}\n"),
            when: pb::type_request::When::Now as i32,
            ..Default::default()
        })
        .await
        .expect("the host relays the line to the box's agent");
}

fn is_unread(session: &pb::Session) -> bool {
    session.unread
}

fn is_read(session: &pb::Session) -> bool {
    !session.unread
}

#[tokio::test(flavor = "multi_thread")]
async fn output_nobody_views_on_a_box_shows_as_unread_on_the_host() {
    let (rig, id) = started().await;

    type_line(&rig, &id, "ping").await;

    rig.wait_session(&id, "unread", is_unread).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn viewing_a_box_session_on_the_host_clears_its_mark_on_the_host() {
    let (rig, id) = started().await;
    type_line(&rig, &id, "ping").await;
    rig.wait_session(&id, "unread", is_unread).await;

    let mut viewing = MarkServiceClient::new(rig.channel().await)
        .view_session(pb::ViewSessionRequest { session: Some(id.clone()) })
        .await
        .expect("the host opens the view")
        .into_inner();
    let first = viewing.message().await.unwrap().expect("the view is confirmed at once");

    assert!(first.was_unread, "the host knew the box's mark");
    rig.wait_session(&id, "read", is_read).await;
    drop(viewing);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_host_restart_shows_the_boxs_mark_again_once_it_reports() {
    let (mut rig, id) = started().await;
    type_line(&rig, &id, "ping").await;
    rig.wait_session(&id, "unread", is_unread).await;

    rig.restart_host(&[]).await;

    rig.wait_session(&id, "unread again", is_unread).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_host_restart_shows_the_boxs_mark_before_the_box_links_again() {
    let (mut rig, id) = started().await;
    type_line(&rig, &id, "ping").await;
    rig.wait_session(&id, "unread", is_unread).await;

    // The box stays away across the restart, so only what the host kept
    // can show the mark (D396).
    rig.link_lost("box-1").await;
    rig.restart_host_unlinked(&[]).await;

    rig.wait_session(&id, "unread before the box links", is_unread).await;
    rig.link_back("box-1").await;
}

/// The host's sessions as ListSessions shows them.
async fn listed(rig: &Rig, id: &pb::SessionId) -> pb::Session {
    pb::session_service_client::SessionServiceClient::new(rig.channel().await)
        .list_sessions(pb::ListSessionsRequest::default())
        .await
        .expect("the host lists its sessions")
        .into_inner()
        .sessions
        .into_iter()
        .find(|session| session.id.as_ref() == Some(id))
        .expect("the host lists the box's session")
}

/// NV-24, D437: pins and VIP marks are kept on the host, also for a session
/// placed on a box. What the box reports (status, unread, pointers) is laid
/// over the host's own view of the session and never takes the marks away.
#[tokio::test(flavor = "multi_thread")]
async fn a_box_sessions_pin_and_vip_are_the_hosts_and_survive_its_reports() {
    let (rig, id) = started().await;
    MarkServiceClient::new(rig.channel().await)
        .set_marks(pb::SetMarksRequest {
            idempotency_key: None,
            entity: Some(pb::EntityRef {
                entity: Some(pb::entity_ref::Entity::Session(id.clone())),
            }),
            pinned: Some(true),
            vip: Some(true),
        })
        .await
        .expect("the host sets the marks");

    let got = rig.session(&id).await;
    assert!(got.pinned && got.vip, "GetSession shows both marks: {got:?}");
    let in_list = listed(&rig, &id).await;
    assert!(in_list.pinned && in_list.vip, "ListSessions shows both marks: {in_list:?}");

    // The box reports the unread output: a Session event from its report.
    type_line(&rig, &id, "ping").await;
    let after = rig.wait_session(&id, "unread", is_unread).await;
    assert!(after.pinned && after.vip, "the report keeps the host's marks: {after:?}");
    let in_list = listed(&rig, &id).await;
    assert!(in_list.pinned && in_list.vip && in_list.unread, "so does the list: {in_list:?}");
}
