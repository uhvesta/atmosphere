//! Closing a channel's Desk session (CD-4, CD-5, CH-24, D306) through a
//! real `humatad2`: Close sends its folder to the trash at once, as one
//! item naming the session, and restoring that item reopens it; the watch
//! closes it when its target is archived past the undo window, or stops it
//! and marks the target gone when its folder holds Markdown; a restore is
//! refused while its target or its channel is gone; deleting the channel
//! lists its Desk sessions and closes each into its own trash item; a
//! closed one stays closed across a restart.

use humatad_test_common as common;

use std::future::Future;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use common::{
    Build, Daemon, World, create_channel, finished, folder, get_session, start_session, wait_status,
};
use humata2_grpc::humata::v1 as pb;
use tonic::Code;
use tonic::transport::Channel;

/// The undo window an archive gets: none, so an archived target is gone
/// at once.
const NO_UNDO: (&str, &str) = ("HUMATA2_ARCHIVE_UNDO_MS", "0");

struct Fixture {
    daemon: Daemon,
    channel: pb::Channel,
    /// The session the Desk sessions look after, in Payments.
    target: pb::SessionId,
}

impl Fixture {
    async fn new() -> Self {
        Self::with_env(&[]).await
    }

    async fn with_env(env: &[(&str, &str)]) -> Self {
        let dir = folder();
        let world = World::new(dir.path());
        let args = world.args();
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let daemon = Daemon::start_with_env(Build::Admin, dir, &refs, env).await;
        world.remote("acme", "api");
        let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
        let (target, _) = start_session(&daemon, &channel, "Ledger fix").await;
        wait_status(&daemon, &target, pb::SessionStatus::Working).await;
        Self {
            daemon,
            channel,
            target,
        }
    }

    async fn desk(&self) -> pb::desk_service_client::DeskServiceClient<Channel> {
        pb::desk_service_client::DeskServiceClient::new(self.daemon.channel().await)
    }

    async fn trash(&self) -> pb::trash_service_client::TrashServiceClient<Channel> {
        pb::trash_service_client::TrashServiceClient::new(self.daemon.channel().await)
    }

    fn channel_id(&self) -> pb::ChannelId {
        self.channel.id.clone().unwrap()
    }

    /// Starts a Desk session of Payments, looking after `target` or the
    /// whole channel, and waits until its agent is up. Answers its id and
    /// folder.
    async fn start(&self, title: &str, target: Option<&pb::SessionId>) -> (pb::SessionId, PathBuf) {
        let started = self
            .desk()
            .await
            .start_desk_session(pb::StartDeskSessionRequest {
                title: title.to_owned(),
                channel: Some(self.channel_id()),
                target: target.cloned(),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner();
        let id = started.session.unwrap();
        wait_status(&self.daemon, &id, pb::SessionStatus::Working).await;
        let path = PathBuf::from(started.desk.unwrap().folder.unwrap().path);
        (id, path)
    }

    async fn close(&self, id: &pb::SessionId) -> pb::DeskSession {
        self.desk()
            .await
            .close_desk_session(pb::CloseDeskSessionRequest {
                idempotency_key: None,
                session: Some(id.clone()),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
    }

    /// The open Desk sessions of Payments.
    async fn listed(&self) -> Vec<pb::DeskSession> {
        self.desk()
            .await
            .list_desk_sessions(pb::ListDeskSessionsRequest {
                channel: Some(self.channel_id()),
                target: None,
            })
            .await
            .unwrap()
            .into_inner()
            .sessions
    }

    async fn is_listed(&self, id: &pb::SessionId) -> bool {
        self.listed()
            .await
            .iter()
            .any(|desk| desk.session.as_ref() == Some(id))
    }

    async fn trash_items(&self) -> Vec<pb::TrashItem> {
        self.trash()
            .await
            .list_trash(pb::ListTrashRequest::default())
            .await
            .unwrap()
            .into_inner()
            .items
    }

    /// The trash item that is Desk session `id`'s folder, if there is one.
    async fn item_of(&self, id: &pb::SessionId) -> Option<pb::TrashItem> {
        self.trash_items()
            .await
            .into_iter()
            .find(|item| item.was == Some(session_ref(id)))
    }

    async fn restore(&self, item: &pb::TrashItemId) -> Result<pb::EntityRef, tonic::Status> {
        self.trash()
            .await
            .restore_from_trash(pb::RestoreFromTrashRequest {
                idempotency_key: None,
                id: Some(item.clone()),
            })
            .await
            .map(|restored| restored.into_inner().restored.unwrap_or_default())
    }

    /// The user archives `id` and waits for the archive to finish.
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

    async fn plan_delete(&self) -> pb::DeleteChannelPlan {
        self.daemon
            .channel_service()
            .await
            .plan_delete_channel(pb::PlanDeleteChannelRequest {
                channel: Some(self.channel_id()),
            })
            .await
            .unwrap()
            .into_inner()
    }

    /// Deletes Payments, its sessions' work included, and waits for it.
    async fn delete_channel(&self) -> pb::DeleteChannelResponse {
        let plan = self.plan_delete().await;
        let deleted = self
            .daemon
            .channel_service()
            .await
            .delete_channel(pb::DeleteChannelRequest {
                idempotency_key: None,
                channel: plan.channel.clone(),
                plan: plan.token.clone(),
                presence: None,
                delete_unpushed_work: true,
                end_live: Some(true),
            })
            .await
            .unwrap()
            .into_inner();
        let done = finished(&self.daemon, deleted.operation.as_ref().unwrap()).await;
        assert_eq!(done.state, pb::OperationState::Succeeded as i32, "{done:?}");
        deleted
    }

    async fn channel_listed(&self) -> bool {
        self.daemon
            .channel_service()
            .await
            .list_channels(pb::ListChannelsRequest::default())
            .await
            .unwrap()
            .into_inner()
            .channels
            .iter()
            .any(|channel| channel.id == self.channel.id)
    }
}

fn session_ref(id: &pb::SessionId) -> pb::EntityRef {
    pb::EntityRef {
        entity: Some(pb::entity_ref::Entity::Session(id.clone())),
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

/// CD-4: Close sends the folder to the trash at once, as one item that is
/// the session; the session leaves the channel's list. A second Close is
/// no second item.
#[tokio::test(flavor = "multi_thread")]
async fn close_sends_the_folder_to_the_trash_at_once_as_the_session() {
    let fixture = Fixture::new().await;
    let (id, path) = fixture.start("Triage the queue", Some(&fixture.target)).await;

    let closed = fixture.close(&id).await;
    assert!(closed.closed_at.is_some());
    let item = closed
        .channel_desk
        .and_then(|part| part.trash_item)
        .expect("the closed session names its trash item");
    assert!(!path.exists(), "the folder is in the trash, not in the channel");
    let listed = fixture.item_of(&id).await.expect("the trash holds the session");
    assert_eq!(listed.id, Some(item.clone()));
    assert!(!fixture.is_listed(&id).await);

    let again = fixture.close(&id).await;
    assert_eq!(again.channel_desk.and_then(|part| part.trash_item), Some(item));
    let items = fixture
        .trash_items()
        .await
        .into_iter()
        .filter(|item| item.was == Some(session_ref(&id)))
        .count();
    assert_eq!(items, 1);
}

/// CD-4: restoring the item puts the folder back and reopens the session.
#[tokio::test(flavor = "multi_thread")]
async fn restoring_a_closed_desk_session_reopens_it() {
    let fixture = Fixture::new().await;
    let (id, path) = fixture.start("Triage the queue", Some(&fixture.target)).await;
    std::fs::write(path.join("plan.md"), "# Plan\n").unwrap();
    let item = fixture
        .close(&id)
        .await
        .channel_desk
        .and_then(|part| part.trash_item)
        .unwrap();

    let restored = fixture.restore(&item).await.expect("the restore goes through");
    assert_eq!(restored, session_ref(&id));
    assert_eq!(std::fs::read_to_string(path.join("plan.md")).unwrap(), "# Plan\n");
    wait_status(&fixture.daemon, &id, pb::SessionStatus::Working).await;
    let back = fixture
        .listed()
        .await
        .into_iter()
        .find(|desk| desk.session.as_ref() == Some(&id))
        .expect("the channel lists it again");
    assert!(back.closed_at.is_none());
    assert_eq!(back.channel_desk.and_then(|part| part.trash_item), None);
    assert!(fixture.item_of(&id).await.is_none());
}

/// CD-5: a target archived past its undo window closes a Desk session
/// whose folder holds no Markdown.
#[tokio::test(flavor = "multi_thread")]
async fn a_target_archived_past_its_undo_window_closes_the_desk_session() {
    let fixture = Fixture::with_env(&[NO_UNDO]).await;
    let (id, path) = fixture.start("Watch the ledger", Some(&fixture.target)).await;

    fixture.archive(&fixture.target).await;

    until("the Desk session closing", || async {
        fixture.item_of(&id).await.is_some()
    })
    .await;
    assert!(!fixture.is_listed(&id).await);
    assert!(!path.exists());
}

/// CD-5: with Markdown in its folder the session stops instead, shows its
/// target gone and keeps its folder.
#[tokio::test(flavor = "multi_thread")]
async fn with_markdown_in_its_folder_the_desk_session_stops_and_keeps_it() {
    let fixture = Fixture::with_env(&[NO_UNDO]).await;
    let (id, path) = fixture.start("Watch the ledger", Some(&fixture.target)).await;
    std::fs::write(path.join("findings.md"), "# Findings\n").unwrap();

    fixture.archive(&fixture.target).await;

    until("the target showing as gone", || async {
        fixture
            .listed()
            .await
            .iter()
            .any(|desk| desk.session.as_ref() == Some(&id) && desk.channel_desk.as_ref().is_some_and(|part| part.target_gone))
    })
    .await;
    assert_eq!(
        std::fs::read_to_string(path.join("findings.md")).unwrap(),
        "# Findings\n"
    );
    assert!(fixture.item_of(&id).await.is_none());
    let status = get_session(&fixture.daemon, &id).await.status;
    assert_ne!(status, pb::SessionStatus::Working as i32, "its agent is stopped");
}

/// CD-5: a closed Desk session can't be restored while its target is gone;
/// it stays in the trash.
#[tokio::test(flavor = "multi_thread")]
async fn a_restore_is_refused_while_the_target_is_gone() {
    let fixture = Fixture::with_env(&[NO_UNDO]).await;
    let (id, _) = fixture.start("Watch the ledger", Some(&fixture.target)).await;
    let item = fixture
        .close(&id)
        .await
        .channel_desk
        .and_then(|part| part.trash_item)
        .unwrap();
    fixture.archive(&fixture.target).await;

    let refused = fixture.restore(&item).await.unwrap_err();
    assert_eq!(refused.code(), Code::FailedPrecondition, "{refused:?}");
    assert!(fixture.item_of(&id).await.is_some());
}

/// CH-24: the delete's plan lists the channel's Desk sessions, and the
/// delete closes each into its own trash item, apart from the channel's.
/// None of them can come back while the channel is gone.
#[tokio::test(flavor = "multi_thread")]
async fn deleting_the_channel_closes_each_desk_session_into_its_own_trash_item() {
    let fixture = Fixture::new().await;
    let (whole, _) = fixture.start("Plan the week", None).await;
    let (watching, _) = fixture.start("Watch the ledger", Some(&fixture.target)).await;

    let plan = fixture.plan_delete().await;
    assert_eq!(plan.desk_sessions.len(), 2);
    assert!(plan.desk_sessions.contains(&whole) && plan.desk_sessions.contains(&watching));

    let deleted = fixture.delete_channel().await;
    let whole_item = fixture.item_of(&whole).await.expect("its own trash item");
    let watching_item = fixture.item_of(&watching).await.expect("its own trash item");
    assert_ne!(whole_item.id, watching_item.id);
    assert_ne!(whole_item.id, deleted.trash_item);
    assert_ne!(watching_item.id, deleted.trash_item);

    let refused = fixture.restore(whole_item.id.as_ref().unwrap()).await.unwrap_err();
    assert_eq!(refused.code(), Code::FailedPrecondition, "{refused:?}");
    assert!(fixture.item_of(&whole).await.is_some());
}

/// CD-4, CH-24: once the channel is back, its Desk session can be restored
/// too, into the channel's folder.
#[tokio::test(flavor = "multi_thread")]
async fn a_desk_session_comes_back_after_its_channel_does() {
    let fixture = Fixture::new().await;
    let (whole, path) = fixture.start("Plan the week", None).await;
    let deleted = fixture.delete_channel().await;
    let item = fixture.item_of(&whole).await.unwrap().id.unwrap();

    fixture
        .restore(deleted.trash_item.as_ref().unwrap())
        .await
        .expect("the channel comes back");
    until("the channel coming back", || fixture.channel_listed()).await;
    fixture.restore(&item).await.expect("then its Desk session does");

    assert!(path.is_dir());
    wait_status(&fixture.daemon, &whole, pb::SessionStatus::Working).await;
    assert!(fixture.is_listed(&whole).await);
}

/// Persistence: a closed Desk session is still closed after a restart:
/// not listed, its folder still in the trash, its agent not brought back.
#[tokio::test(flavor = "multi_thread")]
async fn a_closed_desk_session_stays_closed_after_a_restart() {
    let mut fixture = Fixture::new().await;
    let (id, path) = fixture.start("Triage the queue", Some(&fixture.target)).await;
    fixture.close(&id).await;

    fixture.daemon.restart().await;

    assert!(!fixture.is_listed(&id).await);
    assert!(!path.exists());
    assert!(fixture.item_of(&id).await.is_some());
    let status = get_session(&fixture.daemon, &id).await.status;
    assert_ne!(status, pb::SessionStatus::Working as i32, "nothing resumed it");
}
