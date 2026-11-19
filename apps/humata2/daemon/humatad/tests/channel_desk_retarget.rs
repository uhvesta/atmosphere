//! Changing what a channel Desk session looks after (CD-5) through a real
//! `humatad2`: SetDeskTarget moves the session to another session of its
//! channel, or back to the whole channel, and refuses a session of another
//! channel without changing anything. A target that went shows the session's
//! `target_gone`, and a new target runs the stopped session again.

use humatad_test_common as common;

use std::future::Future;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use common::{
    Build, Daemon, World, create_channel, finished, folder, start_session, wait_running,
};
use humata2_grpc::humata::v1 as pb;
use tonic::transport::Channel;
use tonic::{Code, Status};

/// The undo window an archive gets: none, so an archived target is gone
/// at once.
const NO_UNDO: (&str, &str) = ("HUMATA2_ARCHIVE_UNDO_MS", "0");

struct Fixture {
    daemon: Daemon,
    world: World,
    channel: pb::Channel,
    first: pb::SessionId,
    second: pb::SessionId,
}

impl Fixture {
    async fn new() -> Self {
        let dir = folder();
        let world = World::new(dir.path());
        let args = world.args();
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let daemon = Daemon::start_with_env(Build::Admin, dir, &refs, &[NO_UNDO]).await;
        world.remote("acme", "api");
        let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
        let (first, _) = start_session(&daemon, &channel, "Ledger fix").await;
        let (second, _) = start_session(&daemon, &channel, "Docs pass").await;
        wait_running(&daemon, &first).await;
        wait_running(&daemon, &second).await;
        Self {
            daemon,
            world,
            channel,
            first,
            second,
        }
    }

    async fn desk(&self) -> pb::desk_service_client::DeskServiceClient<Channel> {
        pb::desk_service_client::DeskServiceClient::new(self.daemon.channel().await)
    }

    /// A Desk session of Payments looking after `target`; answers its id and
    /// folder.
    async fn start(&self, target: Option<&pb::SessionId>) -> (pb::SessionId, PathBuf) {
        let started = self
            .desk()
            .await
            .start_desk_session(pb::StartDeskSessionRequest {
                title: "Watch it".to_owned(),
                channel: self.channel.id.clone(),
                target: target.cloned(),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner();
        let id = started.session.unwrap();
        wait_running(&self.daemon, &id).await;
        (id, PathBuf::from(started.desk.unwrap().folder.unwrap().path))
    }

    async fn retarget(
        &self,
        session: &pb::SessionId,
        target: Option<&pb::SessionId>,
    ) -> Result<pb::DeskSession, Status> {
        self.desk()
            .await
            .set_desk_target(pb::SetDeskTargetRequest {
                session: Some(session.clone()),
                target: target.cloned(),
            })
            .await
            .map(tonic::Response::into_inner)
    }

    async fn listed(&self) -> Vec<pb::DeskSession> {
        self.desk()
            .await
            .list_desk_sessions(pb::ListDeskSessionsRequest {
                channel: self.channel.id.clone(),
                target: None,
            })
            .await
            .unwrap()
            .into_inner()
            .sessions
    }

    async fn looking_after(&self, target: &pb::SessionId) -> Vec<pb::SessionId> {
        self.desk()
            .await
            .list_desk_sessions(pb::ListDeskSessionsRequest {
                channel: None,
                target: Some(target.clone()),
            })
            .await
            .unwrap()
            .into_inner()
            .sessions
            .into_iter()
            .filter_map(|desk| desk.session)
            .collect()
    }

    async fn target_gone(&self, id: &pb::SessionId) -> bool {
        self.listed().await.iter().any(|desk| {
            desk.session.as_ref() == Some(id)
                && desk.channel_desk.as_ref().is_some_and(|part| part.target_gone)
        })
    }

    /// The user archives `id` and waits for the archive to finish; with no
    /// undo window the session is gone at once.
    async fn archive(&self, id: &pb::SessionId) {
        let mut sessions = self.daemon.session_service().await;
        let plan = sessions
            .plan_archive(pb::PlanArchiveRequest { id: Some(id.clone()) })
            .await
            .unwrap()
            .into_inner();
        let choice = if plan.choices.contains(&(pb::ArchiveChoice::Archive as i32)) {
            pb::ArchiveChoice::Archive
        } else {
            pb::ArchiveChoice::ArchiveAndDeleteWork
        };
        let started = sessions
            .archive_session(pb::ArchiveSessionRequest {
                idempotency_key: None,
                id: Some(id.clone()),
                choice: choice as i32,
                plan: plan.token,
                end: Some(true),
            })
            .await
            .unwrap()
            .into_inner();
        let done = finished(&self.daemon, &started).await;
        assert_eq!(done.state, pb::OperationState::Succeeded as i32, "{done:?}");
    }
}

/// Waits up to 30 seconds for `done` to hold.
async fn until<F, Fut>(what: &str, done: F)
where
    F: Fn() -> Fut,
    Fut: Future<Output = bool>,
{
    let deadline = Instant::now() + Duration::from_secs(30);
    while !done().await {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_desk_session_looks_after_the_session_it_was_moved_to() {
    let fixture = Fixture::new().await;
    let (desk, _) = fixture.start(Some(&fixture.first)).await;
    let answered = fixture
        .retarget(&desk, Some(&fixture.second))
        .await
        .unwrap();
    assert_eq!(
        answered.channel_desk.and_then(|part| part.target),
        Some(fixture.second.clone())
    );
    assert_eq!(fixture.looking_after(&fixture.second).await, vec![desk]);
    assert!(fixture.looking_after(&fixture.first).await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn clearing_the_target_leaves_the_session_looking_after_the_whole_channel() {
    let fixture = Fixture::new().await;
    let (desk, _) = fixture.start(Some(&fixture.first)).await;
    let answered = fixture.retarget(&desk, None).await.unwrap();
    assert_eq!(answered.channel_desk.and_then(|part| part.target), None);
    assert!(fixture.looking_after(&fixture.first).await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_of_another_channel_is_refused_and_the_target_stays() {
    let fixture = Fixture::new().await;
    fixture.world.remote("acme", "web");
    let (search, _) = create_channel(&fixture.daemon, "Search", &[("acme", "web")]).await;
    let (elsewhere, _) = start_session(&fixture.daemon, &search, "Index tune").await;
    let (desk, _) = fixture.start(Some(&fixture.first)).await;

    let refused = fixture.retarget(&desk, Some(&elsewhere)).await.unwrap_err();
    assert!(
        matches!(refused.code(), Code::FailedPrecondition | Code::InvalidArgument),
        "refused as the caller's mistake, not {refused:?}"
    );
    assert_eq!(fixture.looking_after(&fixture.first).await, vec![desk]);
}

/// CD-5: the daemon says the target is gone, and a new target runs the
/// session it stopped again.
#[tokio::test(flavor = "multi_thread")]
async fn a_gone_target_shows_on_the_desk_session_and_a_new_one_runs_it_again() {
    let fixture = Fixture::new().await;
    let (desk, folder) = fixture.start(Some(&fixture.first)).await;
    std::fs::write(folder.join("findings.md"), "# Findings\n").unwrap();

    fixture.archive(&fixture.first).await;
    until("the target showing as gone", || fixture.target_gone(&desk)).await;
    assert!(fixture.looking_after(&fixture.first).await.contains(&desk));

    let answered = fixture
        .retarget(&desk, Some(&fixture.second))
        .await
        .unwrap();
    assert!(!answered.channel_desk.is_some_and(|part| part.target_gone));
    wait_running(&fixture.daemon, &desk).await;
    assert!(!fixture.target_gone(&desk).await);
}
