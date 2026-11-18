//! `humata session fork <s> --channel <other>` through a real `humatad2`:
//! a fork stays in its parent's channel, so naming another channel is a
//! usage error that points at `session continue --channel` (CV-8), and no
//! fork is made anywhere. Naming the parent's own channel forks as usual.

mod common;

use std::time::{Duration, Instant};

use common::{create_channel, humata, start_session, start_world};
use humata2_grpc::humata::v1 as pb;

async fn sessions(daemon: &common::Daemon) -> Vec<pb::Session> {
    daemon
        .session_service()
        .await
        .list_sessions(pb::ListSessionsRequest {
            include_archived: true,
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner()
        .sessions
}

#[tokio::test(flavor = "multi_thread")]
async fn a_fork_into_another_channel_is_refused_and_makes_no_fork() {
    let (daemon, world) = start_world().await;
    world.remote("acme", "api");
    let (payments, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    let (other, _) = create_channel(&daemon, "Billing", &[("acme", "api")]).await;
    let (parent, _) = start_session(&daemon, &payments, "Fix login").await;
    common::wait_running(&daemon, &parent).await;
    let before = sessions(&daemon).await.len();

    let other_id = other.id.clone().unwrap().value;
    let (code, json) = humata(
        &daemon,
        &["session", "fork", &parent.value, "--kind", "code-and-context", "--channel", &other_id],
    );

    assert_eq!(code, 2, "a usage error: {json}");
    assert_eq!(json["error"]["kind"], "invalid_field", "{json}");
    assert!(
        json["error"]["fix"].as_str().is_some_and(|fix| fix.contains("session continue")),
        "the fix names the command that carries a session into another channel: {json}"
    );
    let after = sessions(&daemon).await;
    assert_eq!(after.len(), before, "no fork is made in any channel: {after:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_fork_naming_its_parents_own_channel_is_made_there() {
    let (daemon, world) = start_world().await;
    world.remote("acme", "api");
    let (payments, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    let (parent, _) = start_session(&daemon, &payments, "Fix login").await;
    common::wait_running(&daemon, &parent).await;

    let channel = payments.id.clone().unwrap();
    let (code, json) = humata(
        &daemon,
        &["session", "fork", &parent.value, "--kind", "code-only", "--channel", &channel.value],
    );

    assert_eq!(code, 0, "{json}");
    let deadline = Instant::now() + Duration::from_secs(60);
    let forks = loop {
        let forks: Vec<_> = sessions(&daemon)
            .await
            .into_iter()
            .filter(|session| session.id.as_ref() != Some(&parent))
            .collect();
        if !forks.is_empty() {
            break forks;
        }
        assert!(Instant::now() < deadline, "the fork was never made: {json}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(forks.len(), 1, "{forks:?}");
    assert_eq!(forks[0].channel.as_ref(), Some(&channel));
}
