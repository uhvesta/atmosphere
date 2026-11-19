//! Read marks through a real `humatad2` (CH-16, CH-17; D392): the agent's
//! output while no client views its session marks it unread, a restart
//! keeps the mark, and MarkService.ViewSession clears it. Only the user
//! views; a session the host doesn't know is not found. A repaint after a
//! resize of the agent's tab marks nothing, but new words in it do (D393).

use humatad_test_common as common;

use common::{
    Attach, Daemon, WAIT, World, agent_tab, create_channel, error_of, get_session, open,
    shell_token, start_redrawing_world, start_session, start_world,
};
use humata2_grpc::humata::v1 as pb;
use pb::mark_service_client::MarkServiceClient;
use tonic::metadata::MetadataValue;
use tonic::{Code, Request};

const TOKEN_HEADER: &str = "humata-session-token";

/// A daemon with one running session, and the session.
async fn started() -> (Daemon, pb::SessionId) {
    let (daemon, world) = start_world().await;
    let session = started_in(&daemon, &world).await;
    (daemon, session)
}

/// Starts a session of a channel on `daemon`, which serves `world`, and
/// returns it once it runs.
async fn started_in(daemon: &Daemon, world: &World) -> pb::SessionId {
    world.remote("acme", "api");
    let (channel, setup) = create_channel(daemon, "Payments", &[("acme", "api")]).await;
    assert_eq!(setup.state, pb::OperationState::Succeeded as i32, "{:?}", setup.error);
    let (session, operation) = start_session(daemon, &channel, "Fix login").await;
    assert_eq!(operation.state, pb::OperationState::Succeeded as i32, "{:?}", operation.error);
    session
}

/// A resize of a tab to a size no tab starts at, so the program in it is
/// told of it and repaints.
fn resize() -> pb::attach_client_message::Message {
    pb::attach_client_message::Message::Resize(pb::TerminalResize {
        cols: 133,
        rows: 37,
        ..Default::default()
    })
}

/// Waits until the session's agent has exited, as the feed tells it or as it
/// now stands. The exit is recorded only after the watch of the agent has
/// judged every output before it (the holder passes on all of a program's
/// output before its exit), so a session that shows as exited has had its
/// last output judged for marks.
async fn wait_exited(daemon: &Daemon, session: &pb::SessionId) {
    let exited = pb::SessionStatus::Exited as i32;
    let mut events = pb::event_service_client::EventServiceClient::new(daemon.channel().await)
        .subscribe(pb::SubscribeRequest {
            topics: vec![pb::Topic::Sessions as i32],
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    if get_session(daemon, session).await.status == exited {
        return;
    }
    tokio::time::timeout(WAIT, async {
        loop {
            let event = events.message().await.unwrap().expect("the feed ended");
            if let Some(pb::event::Payload::Session(shown)) = event.payload
                && shown.id.as_ref() == Some(session)
                && shown.status == exited
            {
                return;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("the feed never told the session had exited"));
}

/// Views `session` and drops the view, so the session is read. Says whether
/// it was unread before the view.
async fn view_and_drop(daemon: &Daemon, session: &pb::SessionId) -> bool {
    let mut viewing = marks(daemon)
        .await
        .view_session(view_request(session))
        .await
        .unwrap()
        .into_inner();
    viewing.message().await.unwrap().expect("the view is confirmed").was_unread
}

/// Types a line to the session's agent and waits for its answer. The
/// terminal attach is not a view: it marks nothing read.
async fn agent_answers(daemon: &Daemon, session: &pb::SessionId, line: &str) {
    let tab = agent_tab(daemon, session).await;
    let mut attach = Attach::start(daemon, open(&tab, 0)).await;
    attach.type_text(&format!("{line}\r")).await;
    attach.wait_for(&format!("got: {line}")).await;
}

/// Waits until the session's read mark is `unread`: as it is now, else
/// as the feed tells it.
async fn wait_unread(daemon: &Daemon, session: &pb::SessionId, unread: bool) {
    let mut events = pb::event_service_client::EventServiceClient::new(daemon.channel().await)
        .subscribe(pb::SubscribeRequest {
            topics: vec![pb::Topic::Sessions as i32],
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    if get_session(daemon, session).await.unread == unread {
        return;
    }
    tokio::time::timeout(WAIT, async {
        loop {
            let event = events.message().await.unwrap().expect("the feed ended");
            if let Some(pb::event::Payload::Session(shown)) = event.payload
                && shown.id.as_ref() == Some(session)
                && shown.unread == unread
            {
                return;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("the feed never told the session with unread = {unread}"));
}

fn view_request(session: &pb::SessionId) -> pb::ViewSessionRequest {
    pb::ViewSessionRequest {
        session: Some(session.clone()),
    }
}

async fn marks(daemon: &Daemon) -> MarkServiceClient<tonic::transport::Channel> {
    MarkServiceClient::new(daemon.channel().await)
}

#[tokio::test(flavor = "multi_thread")]
async fn output_nobody_views_marks_the_session_unread_and_a_restart_keeps_it() {
    let (mut daemon, session) = started().await;
    agent_answers(&daemon, &session, "ping").await;
    wait_unread(&daemon, &session, true).await;

    daemon.restart().await;
    assert!(get_session(&daemon, &session).await.unread, "the restart kept the mark");
}

#[tokio::test(flavor = "multi_thread")]
async fn viewing_a_session_clears_its_mark_and_a_restart_keeps_it_read() {
    let (mut daemon, session) = started().await;
    agent_answers(&daemon, &session, "ping").await;
    wait_unread(&daemon, &session, true).await;

    let mut viewing = marks(&daemon)
        .await
        .view_session(view_request(&session))
        .await
        .unwrap()
        .into_inner();
    let first = viewing.message().await.unwrap().expect("the view is confirmed");
    assert!(first.was_unread, "it was unread before it was viewed");
    assert!(!get_session(&daemon, &session).await.unread, "viewing it read it");
    drop(viewing);

    daemon.restart().await;
    assert!(!get_session(&daemon, &session).await.unread, "the restart kept it read");
}

#[tokio::test(flavor = "multi_thread")]
async fn viewing_a_read_session_says_it_was_read() {
    let (daemon, session) = started().await;
    let mut client = marks(&daemon).await;
    let mut viewing = client.view_session(view_request(&session)).await.unwrap().into_inner();
    viewing.message().await.unwrap().expect("the first view is confirmed");
    // Read now, whatever the agent printed as it started.
    let mut again = client.view_session(view_request(&session)).await.unwrap().into_inner();
    let second = again.message().await.unwrap().expect("the second view is confirmed");
    assert!(!second.was_unread);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_cannot_view_a_session() {
    let (daemon, session) = started().await;
    let (token, _attach) = shell_token(&daemon, &session).await;
    let mut request = Request::new(view_request(&session));
    request
        .metadata_mut()
        .insert(TOKEN_HEADER, MetadataValue::try_from(token.as_str()).unwrap());
    let refused = marks(&daemon).await.view_session(request).await.unwrap_err();
    assert_eq!(refused.code(), Code::PermissionDenied, "{refused:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn viewing_an_unknown_session_is_not_found() {
    let (daemon, _session) = started().await;
    let missing = pb::SessionId {
        value: "se_missing".to_owned(),
    };
    let refused = marks(&daemon)
        .await
        .view_session(view_request(&missing))
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::NotFound, "{refused:?}");
    assert!(matches!(error_of(&refused).kind, Some(pb::error::Kind::NotFound(_))));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_repaint_after_a_resize_leaves_a_read_session_read() {
    let (daemon, world) = start_redrawing_world().await;
    let session = started_in(&daemon, &world).await;
    agent_answers(&daemon, &session, "ping").await;
    wait_unread(&daemon, &session, true).await;
    assert!(view_and_drop(&daemon, &session).await, "it was unread before it was viewed");

    // The agent exits once it has repainted. Its exit is recorded after the
    // watch has judged the repaint, so once the session shows as exited, a
    // view says whether the repaint marked it (see `wait_exited`).
    std::fs::write(world.conv.join("exit-after-repaint"), "").unwrap();
    let tab = agent_tab(&daemon, &session).await;
    let mut attach = Attach::start(&daemon, open(&tab, 0)).await;
    attach.send(resize()).await;
    attach.wait_for("**").await;
    wait_exited(&daemon, &session).await;

    assert!(
        !view_and_drop(&daemon, &session).await,
        "the repaint after the resize marked the session unread"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn new_words_after_a_resize_mark_the_session_unread() {
    let (daemon, world) = start_redrawing_world().await;
    let session = started_in(&daemon, &world).await;
    agent_answers(&daemon, &session, "ping").await;
    wait_unread(&daemon, &session, true).await;
    assert!(view_and_drop(&daemon, &session).await, "it was unread before it was viewed");

    // The repaint shows words the session never printed, which is new output
    // even while the program is repainting.
    std::fs::write(world.conv.join("repaint-text"), "fresh words").unwrap();
    let tab = agent_tab(&daemon, &session).await;
    let mut attach = Attach::start(&daemon, open(&tab, 0)).await;
    attach.send(resize()).await;
    attach.wait_for("fresh words").await;
    wait_unread(&daemon, &session, true).await;
}
