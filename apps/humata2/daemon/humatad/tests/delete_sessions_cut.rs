//! A bulk session delete cut short finishes at the next start (MASSDELETE,
//! D876): at each point of a session's delete the daemon is cut, restarted,
//! and the session ends up deleted: its folder is in the trash once, its
//! records are gone, and the note of the delete is removed. Through a real
//! `humatad2` built with fault points.

use humatad_test_common as common;

use std::time::{Duration, Instant};

use common::{Daemon, WAIT, World, create_channel, folder, start_session, wait_running};
use humata2_grpc::humata::v1 as pb;

/// Every step of one session's delete, as the points a cut can land on.
const POINTS: &[&str] = &[
    "delete_sessions.intent",
    "delete_sessions.stopped",
    "delete_sessions.snapshotted",
    "delete_sessions.trashed",
    "delete_sessions.forgotten",
];

async fn until<F: std::future::Future<Output = bool>>(what: &str, mut done: impl FnMut() -> F) {
    let deadline = Instant::now() + WAIT;
    while !done().await {
        assert!(Instant::now() < deadline, "{what} never happened");
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

async fn sessions_of(daemon: &Daemon, channel: &pb::Channel) -> Vec<pb::Session> {
    daemon
        .session_service()
        .await
        .list_sessions(pb::ListSessionsRequest {
            channel: channel.id.clone(),
            machine: None,
            include_archived: true,
            page: None,
        })
        .await
        .unwrap()
        .into_inner()
        .sessions
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_delete_cut_at_any_point_finishes_at_restart_with_nothing_left() {
    for point in POINTS {
        let dir = folder();
        let world = World::new(dir.path());
        let args = world.args();
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let mut daemon = Daemon::start_with(common::Build::Admin, dir, &refs).await;
        world.remote("acme", "api");
        let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
        let (id, _) = start_session(&daemon, &channel, "One").await;
        let session = wait_running(&daemon, &id).await;
        let clone = std::path::PathBuf::from(&session.repos[0].path);

        daemon.restart_env(&[("HUMATA2_STOP_AT", point)]).await;
        common::last_start(&daemon).await;
        let plan = daemon
            .session_service()
            .await
            .plan_delete_sessions(pb::PlanDeleteSessionsRequest {
                scope: Some(pb::DeleteScope {
                    scope: Some(pb::delete_scope::Scope::Sessions(pb::delete_scope::Picked {
                        ids: vec![id.clone()],
                    })),
                }),
            })
            .await
            .unwrap()
            .into_inner();
        // The daemon ends itself at the point, so the reply may not come.
        let _ = daemon
            .session_service()
            .await
            .delete_sessions(pb::DeleteSessionsRequest {
                idempotency_key: None,
                plan: plan.token.clone(),
                delete_unpushed_work: true,
                typed_count: 0,
                check_only: false,
            })
            .await;
        let deadline = Instant::now() + WAIT;
        while std::os::unix::net::UnixStream::connect(&daemon.socket).is_ok() {
            assert!(Instant::now() < deadline, "the daemon never died at {point}");
            tokio::time::sleep(Duration::from_millis(2)).await;
        }

        daemon.restart().await;
        common::last_start(&daemon).await;

        until("the delete finishing", || async { sessions_of(&daemon, &channel).await.is_empty() }).await;
        assert!(!clone.exists(), "cut at {point}: the folder is in the trash");
        let trash = pb::trash_service_client::TrashServiceClient::new(daemon.channel().await)
            .list_trash(pb::ListTrashRequest::default())
            .await
            .unwrap()
            .into_inner();
        assert_eq!(trash.items.len(), 1, "cut at {point}: {trash:?}");
        let notes = world.channels.join(".deleting-sessions");
        until("the note of the delete going", || async {
            std::fs::read_dir(&notes).map_or(true, |mut left| left.next().is_none())
        })
        .await;
    }
}
