//! A review on a machine whose sessions are stopped (D261, D287), through a
//! real `humatad2`.
//!
//! A stopped machine asks GitHub nothing on its own: no heads poll and no
//! settling of a submit cut short. Its Start (the daemon starting itself
//! again, running) does both. GitHub is the fake `gh` of
//! `common/pr_github.rs`.
//!
//! The PR milestone's Start hook.

#[allow(dead_code)]
mod common;
#[allow(dead_code)]
#[path = "common/mcp_client.rs"]
mod mcp_client;
#[allow(dead_code)]
#[path = "common/pr_github.rs"]
mod pr_github;

use std::time::{Duration, Instant};

use common::wait_running;
use humata2_grpc::humata::v1 as pb;
use hyper_util::rt::TokioIo;
use pr_github::{API, Fixture, LEDGER, SLOW, TWO_COMMITS};
use tokio::net::UnixStream;
use tonic::transport::{Endpoint, Uri};

const STOP: &str = "9b1e4c2a-3f60-4d7e-8a15-6c0d2e7f4a01";

/// Long enough for a 200 ms poll (1 s when the review isn't open) to have
/// asked several times over.
const QUIET: Duration = Duration::from_secs(3);

/// The daemon service on a connection of its own, or `None` while the
/// daemon is between two runs of itself.
async fn daemon_service(
    fixture: &Fixture,
) -> Option<pb::daemon_service_client::DaemonServiceClient<tonic::transport::Channel>> {
    let socket = fixture.daemon.socket.clone();
    let channel = Endpoint::try_from("http://[::]:50051")
        .ok()?
        .connect_with_connector(tower::service_fn(move |_: Uri| {
            let socket = socket.clone();
            async move { Ok::<_, std::io::Error>(TokioIo::new(UnixStream::connect(socket).await?)) }
        }))
        .await
        .ok()?;
    Some(pb::daemon_service_client::DaemonServiceClient::new(channel))
}

async fn last_start(fixture: &Fixture) -> Option<pb::LastStart> {
    daemon_service(fixture)
        .await?
        .get_server_info(pb::GetServerInfoRequest {})
        .await
        .ok()?
        .into_inner()
        .last_start
}

/// The user stops this machine's sessions; returns once the sweep has run.
async fn stop(fixture: &Fixture) {
    daemon_service(fixture)
        .await
        .expect("the daemon is up")
        .stop_sessions(pb::StopSessionsRequest {
            stop_id: STOP.to_owned(),
            reason: "away for the night".to_owned(),
            via: pb::StopVia::App as i32,
            ..Default::default()
        })
        .await
        .expect("the stop is taken");
    let deadline = Instant::now() + SLOW;
    loop {
        let swept = last_start(fixture)
            .await
            .and_then(|start| start.sessions_stopped)
            .is_some_and(|stopped| stopped.ended.is_some());
        if swept {
            return;
        }
        assert!(Instant::now() < deadline, "the daemon never said its sessions are stopped");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// The user starts the sessions again; returns once the start has run.
async fn start(fixture: &Fixture) {
    daemon_service(fixture)
        .await
        .expect("the daemon is up")
        .start_sessions(pb::StartSessionsRequest {})
        .await
        .expect("the start is taken");
    let deadline = Instant::now() + SLOW;
    loop {
        if last_start(fixture).await.is_some_and(|start| start.after_stop.is_some()) {
            return;
        }
        assert!(Instant::now() < deadline, "the daemon never started its sessions again");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn id_of(comment: &pb::Comment) -> &str {
    comment.id.as_ref().map_or("", |id| id.value.as_str())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stopped_machine_stops_following_its_reviews_and_its_start_follows_them_again() {
    let fixture = Fixture::with_env(&[("HUMATA2_PR_POLL_MS", "200")]).await;
    fixture.start_review(&[API]).await;
    stop(&fixture).await;

    let asked = fixture.gh.graphql_calls();
    fixture.move_head(API, &TWO_COMMITS);
    tokio::time::sleep(QUIET).await;
    assert_eq!(fixture.gh.graphql_calls(), asked, "a stopped machine asked GitHub for heads");

    start(&fixture).await;
    let deadline = Instant::now() + SLOW;
    while fixture.gh.graphql_calls() == asked {
        assert!(Instant::now() < deadline, "the started machine never followed its review again");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_submit_cut_short_on_a_stopped_machine_is_settled_at_its_start() {
    let mut fixture = Fixture::new().await;
    let session = fixture.start_review(&[API, LEDGER]).await;
    let mut agent = fixture.agent(&session).await;
    let api = fixture.prs[API].clone();
    let draft = agent.draft(&api, "refund.rs", 3, 3, "Half-even here?").await;
    drop(agent);
    stop(&fixture).await;

    // Submitting asks nothing of the sessions, so a stopped machine still
    // posts; this one dies right after GitHub took the review.
    fixture.restart_with(&[("HUMATA2_STOP_AT", "pr.posted")]).await;
    let plan = fixture.plan(&session).await;
    let reviews = vec![pb::ReviewSubmission {
        pr: Some(api.reference()),
        verdict: pb::Verdict::Comment as i32,
        summary: "Looked".to_owned(),
        drafts: vec![pb::CommentId { value: draft.clone() }],
    }];
    assert!(
        fixture.submit(&session, &plan, "cut", reviews).await.is_err(),
        "the daemon died before it answered"
    );
    fixture.daemon.stopped(SLOW).await;
    let posted = fixture.gh.posted(&api);
    assert_eq!(posted.len(), 1, "{posted:?}");
    let (id, body) = posted.into_iter().next().unwrap();
    fixture.gh.list_review(&api, id, &body, "2099-01-01T00:00:00Z");

    // Still stopped: the cut submit waits, neither posted nor unconfirmed.
    fixture.restart().await;
    tokio::time::sleep(QUIET).await;
    let waiting = fixture.list(&session).await;
    let waiting = waiting.iter().find(|c| id_of(c) == draft).expect("the draft is listed");
    assert_ne!(waiting.status, pb::CommentStatus::Posted as i32, "{waiting:?}");
    assert!(!waiting.post_unconfirmed, "{waiting:?}");

    start(&fixture).await;
    wait_running(&fixture.daemon, session.id.as_ref().unwrap()).await;
    fixture
        .comments_until(&session, |comments| {
            comments
                .iter()
                .any(|c| id_of(c) == draft && c.status == pb::CommentStatus::Posted as i32)
        })
        .await;
    assert_eq!(fixture.gh.posted(&api).len(), 1, "never posted twice");
}
