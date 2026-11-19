//! DESKWATCH: the feed tells of Desk sessions as they change, so a client
//! keeps the Desk without listing it again. Through a real `humatad2`.

use humatad_test_common as common;

use common::{Build, Daemon, World, WAIT, create_channel, folder, wait_status};
use humata2_grpc::humata::v1 as pb;
use tonic::transport::Channel;

struct Fixture {
    daemon: Daemon,
    channel: pb::Channel,
}

impl Fixture {
    async fn new() -> Self {
        let dir = folder();
        let world = World::new(dir.path());
        let args = world.args();
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let daemon = Daemon::start_with_env(Build::Admin, dir, &refs, &[]).await;
        world.remote("acme", "api");
        let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
        Self { daemon, channel }
    }

    async fn desk(&self) -> pb::desk_service_client::DeskServiceClient<Channel> {
        pb::desk_service_client::DeskServiceClient::new(self.daemon.channel().await)
    }

    async fn feed(&self) -> tonic::Streaming<pb::Event> {
        pb::event_service_client::EventServiceClient::new(self.daemon.channel().await)
            .subscribe(pb::SubscribeRequest {
                topics: vec![pb::Topic::Sessions as i32],
                ..pb::SubscribeRequest::default()
            })
            .await
            .unwrap()
            .into_inner()
    }

    /// Starts a Desk session of Payments for the whole channel and waits
    /// until its agent is up.
    async fn start(&self, title: &str) -> pb::SessionId {
        let started = self
            .desk()
            .await
            .start_desk_session(pb::StartDeskSessionRequest {
                title: title.to_owned(),
                channel: self.channel.id.clone(),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner();
        let id = started.session.unwrap();
        wait_status(&self.daemon, &id, pb::SessionStatus::Working).await;
        id
    }
}

/// The next Desk event of `session` that `wanted` accepts.
async fn desk_event(
    feed: &mut tonic::Streaming<pb::Event>,
    session: &pb::SessionId,
    wanted: impl Fn(i32, &pb::DeskSession) -> bool,
) -> (i32, pb::DeskSession) {
    tokio::time::timeout(WAIT, async {
        loop {
            let event = feed.message().await.unwrap().expect("the feed ended");
            if let Some(pb::event::Payload::Desk(desk)) = event.payload
                && desk.session.as_ref() == Some(session)
                && wanted(event.change, &desk)
            {
                return (event.change, desk);
            }
        }
    })
    .await
    .expect("no Desk event of the session came on the feed")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_subscriber_hears_another_client_close_a_channel_desk_session() {
    let fixture = Fixture::new().await;
    let id = fixture.start("Triage").await;
    let mut feed = fixture.feed().await;

    // Another client closes it; the subscriber made no call.
    fixture
        .desk()
        .await
        .close_desk_session(pb::CloseDeskSessionRequest {
            idempotency_key: None,
            session: Some(id.clone()),
            ..Default::default()
        })
        .await
        .unwrap();

    let (_, desk) = desk_event(&mut feed, &id, |_, desk| desk.closed_at.is_some()).await;
    let part = desk.channel_desk.expect("a channel Desk session carries its channel part");
    assert_eq!(part.channel, fixture.channel.id);
    assert!(part.target.is_none(), "it looks after the whole channel");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_subscriber_hears_a_channel_desk_session_start() {
    let fixture = Fixture::new().await;
    let mut feed = fixture.feed().await;

    let id = fixture.start("Watch").await;

    let (_, desk) = desk_event(&mut feed, &id, |_, desk| desk.channel_desk.is_some()).await;
    assert!(desk.closed_at.is_none(), "a new session is open");
}
