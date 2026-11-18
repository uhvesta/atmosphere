//! ChannelService.PlanDeleteChannel and DeleteChannel (CH-24, CH-25, NV-27,
//! LD-5 to LD-7): the plan lists every session, board item and pending
//! request of a channel and changes nothing; unpushed work blocks the delete
//! until the user ticks "Delete that work"; deleting stops the agents,
//! withdraws the requests and sends the channel's folder to the trash, and
//! restoring it brings the channel back with its sessions archived. Only the
//! user can delete. Through a real `humatad2`.

mod common;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use common::{
    Attach, Daemon, WAIT, World, create_channel, error_of, finished, folder, holder_answers,
    pending_requests, shell_token, start_session, state_folder, stored_tabs,
};
use humata2_grpc::humata::v1 as pb;
use tonic::metadata::MetadataValue;
use tonic::{Code, Request, Status};

struct Fixture {
    daemon: Daemon,
    world: World,
    channel: pb::Channel,
}

impl Fixture {
    async fn new() -> Self {
        let dir = folder();
        let world = World::new(dir.path());
        let args = world.args();
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let daemon = Daemon::start_with(common::Build::Admin, dir, &refs).await;
        world.remote("acme", "api");
        let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
        Self {
            daemon,
            world,
            channel,
        }
    }

    fn id(&self) -> pb::ChannelId {
        self.channel.id.clone().unwrap()
    }

    /// The channel's folder on this machine.
    fn folder(&self) -> PathBuf {
        self.world.channels.join(&self.channel.folder)
    }

    async fn session(&self, title: &str) -> pb::Session {
        let (id, _) = start_session(&self.daemon, &self.channel, title).await;
        common::wait_running(&self.daemon, &id).await
    }

    async fn plan(&self) -> Result<pb::DeleteChannelPlan, Status> {
        self.daemon
            .channel_service()
            .await
            .plan_delete_channel(pb::PlanDeleteChannelRequest {
                channel: Some(self.id()),
            })
            .await
            .map(tonic::Response::into_inner)
    }

    fn delete_request(plan: &pb::DeleteChannelPlan, delete_work: bool) -> pb::DeleteChannelRequest {
        pb::DeleteChannelRequest {
            idempotency_key: None,
            channel: plan.channel.clone(),
            plan: plan.token.clone(),
            presence: None,
            delete_unpushed_work: delete_work,
        }
    }

    async fn delete(
        &self,
        plan: &pb::DeleteChannelPlan,
        delete_work: bool,
    ) -> Result<pb::DeleteChannelResponse, Status> {
        self.daemon
            .channel_service()
            .await
            .delete_channel(Self::delete_request(plan, delete_work))
            .await
            .map(tonic::Response::into_inner)
    }

    /// Deletes the channel and waits for the operation to succeed.
    async fn delete_all(&self) -> pb::DeleteChannelResponse {
        let plan = self.plan().await.unwrap();
        let deleted = self.delete(&plan, true).await.unwrap();
        let operation = finished(&self.daemon, deleted.operation.as_ref().unwrap()).await;
        assert_eq!(
            operation.state,
            pb::OperationState::Succeeded as i32,
            "{operation:?}"
        );
        deleted
    }

    async fn listed(&self) -> bool {
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

    async fn sessions(&self, include_archived: bool) -> Vec<pb::Session> {
        self.daemon
            .session_service()
            .await
            .list_sessions(pb::ListSessionsRequest {
                channel: Some(self.id()),
                machine: None,
                include_archived,
                page: None,
            })
            .await
            .unwrap()
            .into_inner()
            .sessions
    }

    async fn trash(&self) -> pb::trash_service_client::TrashServiceClient<tonic::transport::Channel> {
        pb::trash_service_client::TrashServiceClient::new(self.daemon.channel().await)
    }

    /// The user makes an item on the channel's board.
    async fn board_item(&self, title: &str) -> pb::BoardItemId {
        pb::board_service_client::BoardServiceClient::new(self.daemon.channel().await)
            .create_item(pb::CreateItemRequest {
                board: Some(pb::BoardRef {
                    board: Some(pb::board_ref::Board::Channel(self.id())),
                }),
                title: title.to_owned(),
                column: pb::BoardColumn::ToDo as i32,
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
            .item
            .unwrap()
            .id
            .unwrap()
    }

    /// The agent of `session` asks the user something and stops waiting at
    /// once, leaving the request pending.
    async fn ask(&self, session: &pb::SessionId) -> (pb::RequestId, String, Attach) {
        let (token, attach) = shell_token(&self.daemon, session).await;
        let waited = self
            .daemon
            .inbox_service()
            .await
            .ask_user(as_agent(
                pb::AskUserRequest {
                    idempotency_key: None,
                    question: "Which ledger?".to_owned(),
                    choices: Vec::new(),
                    wait_seconds: 1,
                },
                &token,
            ))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(waited.state(), pb::RequestState::Pending, "{waited:?}");
        (waited.id.unwrap(), token, attach)
    }
}

fn as_agent<T>(message: T, token: &str) -> Request<T> {
    let mut request = Request::new(message);
    request.metadata_mut().insert(
        "humata-session-token",
        MetadataValue::try_from(token).unwrap(),
    );
    request
}

fn clone_of(session: &pb::Session) -> PathBuf {
    PathBuf::from(&session.repos[0].path)
}

/// Every path under `root`, sorted.
fn tree(root: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_owned()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if std::fs::symlink_metadata(&path).unwrap().is_dir() {
                stack.push(path.clone());
            }
            out.push(path);
        }
    }
    out.sort();
    out
}

async fn archive(daemon: &Daemon, id: &pb::SessionId) {
    let mut sessions = daemon.session_service().await;
    let plan = sessions
        .plan_archive(pb::PlanArchiveRequest {
            id: Some(id.clone()),
        })
        .await
        .unwrap()
        .into_inner();
    let started = sessions
        .archive_session(pb::ArchiveSessionRequest {
            idempotency_key: None,
            id: Some(id.clone()),
            choice: pb::ArchiveChoice::Archive as i32,
            plan: plan.token,
        })
        .await
        .unwrap()
        .into_inner();
    let operation = finished(daemon, &started).await;
    assert_eq!(
        operation.state,
        pb::OperationState::Succeeded as i32,
        "{operation:?}"
    );
}

/// Waits until `done` holds, polling.
async fn until<F: std::future::Future<Output = bool>>(what: &str, mut done: impl FnMut() -> F) {
    let deadline = Instant::now() + WAIT;
    while !done().await {
        assert!(Instant::now() < deadline, "{what} never happened");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn the_plan_lists_every_session_item_and_request_and_changes_nothing() {
    if !common::in_this_shard(0) {
        return;
    }
    let fixture = Fixture::new().await;
    let dirty = fixture.session("Dirty").await;
    std::fs::write(clone_of(&dirty).join("wip.txt"), "wip\n").unwrap();
    let clean = fixture.session("Clean").await;
    let gone = fixture.session("Archived").await;
    archive(&fixture.daemon, gone.id.as_ref().unwrap()).await;
    let item = fixture.board_item("Reconcile refunds").await;
    let (request, _token, _attach) = fixture.ask(clean.id.as_ref().unwrap()).await;
    let before = tree(&fixture.folder());

    let plan = fixture.plan().await.unwrap();

    assert_eq!(plan.channel, Some(fixture.id()));
    assert!(plan.token.as_ref().is_some_and(|t| !t.value.is_empty()), "{plan:?}");
    let work = |session: &pb::Session| {
        plan.live_sessions
            .iter()
            .find(|w| w.session == session.id)
            .map(|w| (w.dirty_files, w.unpushed_commits))
    };
    assert_eq!(work(&dirty), Some((1, 0)), "{plan:?}");
    assert_eq!(work(&clean), Some((0, 0)), "{plan:?}");
    assert_eq!(plan.live_sessions.len(), 2, "{plan:?}");
    assert!(plan.archived.iter().any(|w| w.session == gone.id), "{plan:?}");
    assert_eq!(plan.archived_sessions, 1);
    assert_eq!(plan.board_item_ids, vec![item]);
    assert_eq!(plan.board_items, 1);
    assert_eq!(plan.pending_request_ids, vec![request]);
    assert_eq!(plan.pending_requests, 1);
    assert!(!plan.machines.is_empty(), "this machine holds the folder: {plan:?}");

    assert!(fixture.listed().await);
    assert_eq!(tree(&fixture.folder()), before);
    assert_eq!(pending_requests(&fixture.daemon).await.len(), 1);
    let after = common::get_session(&fixture.daemon, dirty.id.as_ref().unwrap()).await;
    // Planning stops nothing: the agent still runs. A quiet one may read as
    // probably idle by now, which is as live as working.
    assert!(common::is_running(&after), "planning stopped the session: {after:?}");
}

#[tokio::test]
async fn unpushed_work_blocks_the_delete_until_the_user_ticks_delete_that_work() {
    if !common::in_this_shard(0) {
        return;
    }
    let fixture = Fixture::new().await;
    let dirty = fixture.session("Dirty").await;
    std::fs::write(clone_of(&dirty).join("wip.txt"), "wip\n").unwrap();
    let plan = fixture.plan().await.unwrap();

    let refused = fixture.delete(&plan, false).await.unwrap_err();

    assert_eq!(refused.code(), Code::FailedPrecondition, "{refused:?}");
    assert!(
        matches!(error_of(&refused).kind, Some(pb::error::Kind::UnpushedWork(_))),
        "{refused:?}"
    );
    assert!(fixture.listed().await);
    assert!(clone_of(&dirty).join("wip.txt").exists());
    let after = common::get_session(&fixture.daemon, dirty.id.as_ref().unwrap()).await;
    assert!(common::is_running(&after), "the session runs on: {:?}", after.status());

    // Ticking "Delete that work" lets the same plan through.
    let deleted = fixture.delete(&plan, true).await.unwrap();
    let operation = finished(&fixture.daemon, deleted.operation.as_ref().unwrap()).await;
    assert_eq!(operation.state, pb::OperationState::Succeeded as i32, "{operation:?}");
    assert!(!fixture.listed().await);
}

#[tokio::test]
async fn deleting_stops_the_agents_withdraws_the_requests_and_sends_the_folder_to_trash() {
    if !common::in_this_shard(1) {
        return;
    }
    let fixture = Fixture::new().await;
    let one = fixture.session("One").await;
    fixture.session("Two").await;
    fixture.board_item("Reconcile refunds").await;
    let (request, _token, _attach) = fixture.ask(one.id.as_ref().unwrap()).await;
    let tabs = stored_tabs(&state_folder(fixture.daemon.dir.path()));
    assert!(tabs.iter().any(holder_answers), "the agents run before the delete");
    let folder = fixture.folder();
    assert!(folder.exists());

    let deleted = fixture.delete_all().await;

    assert!(!fixture.listed().await, "the channel leaves the sidebar");
    assert!(fixture.sessions(true).await.is_empty());
    assert!(!folder.exists(), "the channel's folder went to the trash");
    for tab in &tabs {
        assert!(!holder_answers(tab), "tab {} still runs", tab.id);
    }
    assert!(pending_requests(&fixture.daemon).await.is_empty());
    let state = fixture
        .daemon
        .inbox_service()
        .await
        .get_request(pb::GetRequestRequest { id: Some(request) })
        .await
        .map(|request| request.into_inner().state());
    assert!(
        matches!(state, Ok(pb::RequestState::Withdrawn)) || matches!(&state, Err(s) if s.code() == Code::NotFound),
        "the request is withdrawn or gone: {state:?}"
    );
    let item = deleted.trash_item.expect("the trash item");
    let trash = fixture
        .trash()
        .await
        .list_trash(pb::ListTrashRequest::default())
        .await
        .unwrap()
        .into_inner();
    assert!(trash.items.iter().any(|i| i.id.as_ref() == Some(&item)), "{trash:?}");

    // A second delete of the same channel finds nothing.
    let again = fixture.plan().await.unwrap_err();
    assert_eq!(again.code(), Code::NotFound, "{again:?}");
}

#[tokio::test]
async fn restoring_from_the_trash_brings_the_channel_back_with_its_sessions_archived() {
    if !common::in_this_shard(0) {
        return;
    }
    let fixture = Fixture::new().await;
    let one = fixture.session("One").await;
    let clone = clone_of(&one);
    std::fs::write(clone.join("kept.txt"), "kept\n").unwrap();
    let deleted = fixture.delete_all().await;

    fixture
        .trash()
        .await
        .restore_from_trash(pb::RestoreFromTrashRequest {
            idempotency_key: None,
            id: deleted.trash_item.clone(),
        })
        .await
        .unwrap();

    until("the channel coming back", || fixture.listed()).await;
    let sessions = fixture.sessions(true).await;
    assert_eq!(sessions.len(), 1, "{sessions:?}");
    assert_eq!(sessions[0].id, one.id);
    assert!(sessions[0].archive.is_some(), "it comes back archived: {sessions:?}");
    assert!(fixture.sessions(false).await.is_empty());
    assert_eq!(std::fs::read_to_string(clone.join("kept.txt")).unwrap(), "kept\n");
}

#[tokio::test]
async fn a_plan_that_no_longer_matches_does_nothing() {
    if !common::in_this_shard(0) {
        return;
    }
    let fixture = Fixture::new().await;
    fixture.session("One").await;
    let plan = fixture.plan().await.unwrap();
    let later = fixture.session("Started after the plan").await;

    let stale = fixture.delete(&plan, true).await.unwrap_err();

    assert_eq!(stale.code(), Code::FailedPrecondition, "{stale:?}");
    assert!(
        matches!(error_of(&stale).kind, Some(pb::error::Kind::PlanStale(_))),
        "{stale:?}"
    );
    assert!(fixture.listed().await);
    let after = common::get_session(&fixture.daemon, later.id.as_ref().unwrap()).await;
    assert!(common::is_running(&after), "the session runs on: {:?}", after.status());

    // A plan is taken once.
    let fresh = fixture.plan().await.unwrap();
    let deleted = fixture.delete(&fresh, true).await.unwrap();
    finished(&fixture.daemon, deleted.operation.as_ref().unwrap()).await;
    let reused = fixture.delete(&fresh, true).await.unwrap_err();
    assert_ne!(reused.code(), Code::Ok);
}

#[tokio::test]
async fn an_agent_cannot_plan_or_delete_a_channel() {
    if !common::in_this_shard(1) {
        return;
    }
    let fixture = Fixture::new().await;
    let one = fixture.session("One").await;
    let (token, _attach) = shell_token(&fixture.daemon, one.id.as_ref().unwrap()).await;
    let plan = fixture.plan().await.unwrap();
    let mut channels = fixture.daemon.channel_service().await;

    let planned = channels
        .plan_delete_channel(as_agent(
            pb::PlanDeleteChannelRequest {
                channel: Some(fixture.id()),
            },
            &token,
        ))
        .await
        .unwrap_err();
    let deleted = channels
        .delete_channel(as_agent(Fixture::delete_request(&plan, true), &token))
        .await
        .unwrap_err();

    for refused in [planned, deleted] {
        assert_eq!(refused.code(), Code::PermissionDenied, "{refused:?}");
        assert!(
            !matches!(error_of(&refused).kind, Some(pb::error::Kind::ApprovalPending(_))),
            "no Inbox request exists for it (CH-25): {refused:?}"
        );
    }
    assert!(pending_requests(&fixture.daemon).await.is_empty());
    assert!(fixture.listed().await);
}

#[tokio::test]
async fn a_delete_cut_short_finishes_after_a_restart() {
    if !common::in_this_shard(0) {
        return;
    }
    let mut fixture = Fixture::new().await;
    fixture.session("One").await;
    let folder = fixture.folder();
    fixture
        .daemon
        .restart_env(&[("HUMATA2_STOP_AT", "delete_channel.agents_stopped")])
        .await;
    common::last_start(&fixture.daemon).await;
    let plan = fixture.plan().await.unwrap();
    // The daemon kills itself at the stop point, so the reply may not come.
    let _ = fixture.delete(&plan, true).await;

    fixture.daemon.restart().await;

    until("the delete finishing", || async { !fixture.listed().await }).await;
    assert!(!folder.exists());
    assert!(fixture.sessions(true).await.is_empty());
    let trash = fixture
        .trash()
        .await
        .list_trash(pb::ListTrashRequest::default())
        .await
        .unwrap()
        .into_inner();
    assert_eq!(trash.items.len(), 1, "{trash:?}");
}

/// Every step a delete takes, as the points a cut can land on.
const DELETE_POINTS: &[&str] = &[
    "delete_channel.intent",
    "delete_channel.stopped",
    "delete_channel.agents_stopped",
    "delete_channel.snapshotted",
    "delete_channel.trashed",
    "delete_channel.forgotten",
];

/// A delete cut at any step is finished at the next start: the channel and
/// its sessions are gone, the folder is in the trash once, no agent runs,
/// and no note of the delete is left.
#[tokio::test]
async fn a_delete_cut_at_any_point_finishes_at_restart_with_nothing_left() {
    if !common::in_this_shard(1) {
        return;
    }
    for point in DELETE_POINTS {
        let mut fixture = Fixture::new().await;
        fixture.session("One").await;
        fixture.session("Two").await;
        let folder = fixture.folder();
        let tabs = stored_tabs(&state_folder(fixture.daemon.dir.path()));
        assert!(tabs.iter().any(holder_answers), "the agents run before the delete");
        fixture.daemon.restart_env(&[("HUMATA2_STOP_AT", point)]).await;
        common::last_start(&fixture.daemon).await;
        let plan = fixture.plan().await.unwrap();
        let _ = fixture.delete(&plan, true).await;
        let deadline = Instant::now() + WAIT;
        while std::os::unix::net::UnixStream::connect(&fixture.daemon.socket).is_ok() {
            assert!(Instant::now() < deadline, "the daemon never died at {point}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        fixture.daemon.restart().await;
        common::last_start(&fixture.daemon).await;

        until("the delete finishing", || async { !fixture.listed().await }).await;
        assert!(!folder.exists(), "cut at {point}: the folder is in the trash");
        assert!(fixture.sessions(true).await.is_empty(), "cut at {point}");
        let trash = fixture
            .trash()
            .await
            .list_trash(pb::ListTrashRequest::default())
            .await
            .unwrap()
            .into_inner();
        assert_eq!(trash.items.len(), 1, "cut at {point}: {trash:?}");
        for tab in stored_tabs(&state_folder(fixture.daemon.dir.path())) {
            assert!(!holder_answers(&tab), "cut at {point}: tab {} still runs", tab.id);
        }
        for tab in &tabs {
            assert!(!holder_answers(tab), "cut at {point}: tab {} still runs", tab.id);
        }
        assert!(pending_requests(&fixture.daemon).await.is_empty(), "cut at {point}");
        let notes = fixture.world.channels.join(".deleting");
        until("the note of the delete going", || async {
            std::fs::read_dir(&notes).map_or(true, |mut left| left.next().is_none())
        })
        .await;
    }
}
