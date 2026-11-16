//! Session and channel changes on the event feed, through a real
//! `humatad2`: a new channel and a started or renamed session reach the user
//! subscribed to them, an agent that exits is told as exited, and an agent
//! hears of no session it may not read.

mod common;

use std::time::Duration;

use common::{WAIT, agent_tab, create_channel, shell_token, start_session, start_world};
use humata2_grpc::humata::v1 as pb;
use tonic::metadata::MetadataValue;
use tonic::{Request, Streaming};

const TOKEN_HEADER: &str = "humata-session-token";

async fn subscribe(
    daemon: &common::Daemon,
    request: Request<pb::SubscribeRequest>,
) -> Streaming<pb::Event> {
    pb::event_service_client::EventServiceClient::new(daemon.channel().await)
        .subscribe(request)
        .await
        .unwrap()
        .into_inner()
}

fn topics() -> pb::SubscribeRequest {
    pb::SubscribeRequest {
        topics: vec![pb::Topic::Sessions as i32, pb::Topic::Channels as i32],
        ..Default::default()
    }
}

/// The next event for which `pick` answers `Some`, within `WAIT`.
async fn next_matching<T>(
    events: &mut Streaming<pb::Event>,
    mut pick: impl FnMut(pb::event::Payload) -> Option<T>,
) -> T {
    tokio::time::timeout(WAIT, async {
        loop {
            let event = events.message().await.unwrap().expect("the feed ended");
            if let Some(found) = event.payload.and_then(&mut pick) {
                return found;
            }
        }
    })
    .await
    .expect("the expected event never arrived")
}

async fn rename(daemon: &common::Daemon, id: &pb::SessionId, title: &str) {
    daemon
        .session_service()
        .await
        .rename_session(pb::RenameSessionRequest {
            id: Some(id.clone()),
            title: title.to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_user_hears_of_a_new_channel_and_a_started_session() {
    let (daemon, world) = start_world().await;
    world.remote("acme", "api");
    let mut events = subscribe(&daemon, Request::new(topics())).await;

    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    let told = next_matching(&mut events, |payload| match payload {
        pb::event::Payload::Channel(told) => Some(told),
        _ => None,
    })
    .await;
    assert_eq!(told.id, channel.id);

    let (session, _) = start_session(&daemon, &channel, "Fix the login").await;
    let told = next_matching(&mut events, |payload| match payload {
        pb::event::Payload::Session(told) if told.id.as_ref() == Some(&session) => Some(told),
        _ => None,
    })
    .await;
    assert_eq!(told.title, "Fix the login");
    assert_eq!(told.channel, channel.id);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_renamed_session_is_told_with_its_new_title() {
    let (daemon, world) = start_world().await;
    world.remote("acme", "api");
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    let (session, _) = start_session(&daemon, &channel, "Before").await;
    let mut events = subscribe(&daemon, Request::new(topics())).await;

    rename(&daemon, &session, "After").await;

    let told = next_matching(&mut events, |payload| match payload {
        pb::event::Payload::Session(told) if told.title == "After" => Some(told),
        _ => None,
    })
    .await;
    assert_eq!(told.id, Some(session));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_hears_of_its_own_session_and_not_one_it_may_not_read() {
    let (daemon, world) = start_world().await;
    world.remote("acme", "api");
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    let (mine, _) = start_session(&daemon, &channel, "Mine").await;
    let (other, _) = start_session(&daemon, &channel, "Other").await;
    // Seeing the channel's other sessions is Default Allow (D127.4), as
    // ListSessions shows them; with the rule at Never the agent reads only
    // its own.
    pb::access_service_client::AccessServiceClient::new(daemon.channel().await)
        .set_channel_rule(pb::SetChannelRuleRequest {
            channel: Some(channel.id.clone().unwrap()),
            capability: pb::Capability::ChannelRead as i32,
            decision: pb::Decision::Never as i32,
            ..Default::default()
        })
        .await
        .unwrap();
    let (token, _attach) = shell_token(&daemon, &mine).await;
    let mut request = Request::new(pb::SubscribeRequest {
        topics: vec![pb::Topic::Sessions as i32],
        ..Default::default()
    });
    request.metadata_mut().insert(
        TOKEN_HEADER,
        MetadataValue::try_from(token.as_str()).unwrap(),
    );
    let mut events = subscribe(&daemon, request).await;

    rename(&daemon, &other, "Other renamed").await;
    rename(&daemon, &mine, "Mine renamed").await;

    // The first session event the agent gets is its own: the other's came
    // first and was not passed on.
    let told = next_matching(&mut events, |payload| match payload {
        pb::event::Payload::Session(told) => Some(told),
        _ => None,
    })
    .await;
    assert_eq!(told.id, Some(mine));
    // And nothing about the other turns up after it.
    let more = tokio::time::timeout(Duration::from_millis(500), async {
        loop {
            let event = events.message().await.unwrap()?;
            if let Some(pb::event::Payload::Session(told)) = event.payload
                && told.id.as_ref() == Some(&other)
            {
                return Some(told);
            }
        }
    })
    .await;
    assert!(!matches!(more, Ok(Some(_))));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_that_exits_is_told_as_exited() {
    let (daemon, world) = start_world().await;
    world.remote("acme", "api");
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    let (session, _) = start_session(&daemon, &channel, "Short lived").await;
    let tab = agent_tab(&daemon, &session).await;
    let mut events = subscribe(&daemon, Request::new(topics())).await;

    // End of input: the test agent's read loop ends and it exits.
    daemon
        .terminals()
        .await
        .r#type(pb::TypeRequest {
            idempotency_key: Some(pb::IdempotencyKey {
                value: "exit-the-agent".to_owned(),
            }),
            session: Some(session.clone()),
            tab: Some(tab),
            text: "\u{4}".to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();

    let told = next_matching(&mut events, |payload| match payload {
        pb::event::Payload::Session(told)
            if told.id.as_ref() == Some(&session)
                && told.status == pb::SessionStatus::Exited as i32 =>
        {
            Some(told)
        }
        _ => None,
    })
    .await;
    assert_eq!(told.channel, channel.id);
}
