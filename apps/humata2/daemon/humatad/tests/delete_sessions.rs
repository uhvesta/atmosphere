//! SessionService.PlanDeleteSessions and DeleteSessions (MASSDELETE, LD-5
//! to LD-7, NV-27) through a real `humatad2`: the plan lists the terminals
//! that end, the work that would be lost and the forks that go along or
//! stay; a change after the plan deletes nothing; unpushed work and the
//! typed count gate the delete; a deleted session's folder is in the trash
//! and its records are gone; restoring it brings it back archived; an agent
//! never gets scope all or its own session.

use humatad_test_common as common;

use std::path::PathBuf;

use common::{
    Attach, Daemon, World, create_channel, error_of, finished, folder, get_session, shell_token,
    start_session, wait_running,
};
use humata2_grpc::humata::v1 as pb;
use tonic::Code;

/// Three prompts with answers: enough for a context-only fork.
fn transcript() -> String {
    let lines = [
        r#"{"type":"user","message":{"role":"user","content":"Find the flaky test"},"timestamp":"2026-11-15T09:00:00Z"}"#,
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"It is retry_test."}]},"timestamp":"2026-11-15T09:00:05Z"}"#,
    ];
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

struct Fixture {
    daemon: Daemon,
    world: World,
    channel: pb::Channel,
}

impl Fixture {
    async fn new() -> Self {
        let dir = folder();
        let world = World::new(dir.path());
        let mut agents: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&world.agents).unwrap()).unwrap();
        agents[0]["conversation_format"] = "claude-jsonl".into();
        agents[0]["version_args"] = serde_json::json!([]);
        std::fs::write(&world.agents, agents.to_string()).unwrap();
        let args = world.args();
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let daemon = Daemon::start_with(common::Build::Admin, dir, &refs).await;
        world.remote("acme", "api");
        let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
        Self { daemon, world, channel }
    }

    async fn session(&self, title: &str) -> pb::Session {
        let (id, _) = start_session(&self.daemon, &self.channel, title).await;
        wait_running(&self.daemon, &id).await
    }

    async fn fork(&self, parent: &pb::Session, kind: pb::ForkKind) -> pb::Session {
        let started = self
            .daemon
            .session_service()
            .await
            .fork_session(pb::ForkSessionRequest {
                idempotency_key: None,
                parent: parent.id.clone(),
                kind: kind as i32,
                before_turn: 0,
                title: "Side look".to_owned(),
                agent: None,
                model: String::new(),
                machine: None,
                branch: String::new(),
                reason: String::new(),
            })
            .await
            .unwrap()
            .into_inner();
        let operation = finished(&self.daemon, &started).await;
        let Some(pb::operation::Result::Session(id)) = &operation.result else {
            panic!("the fork names no session: {operation:?}");
        };
        wait_running(&self.daemon, id).await
    }

    async fn plan(&self, scope: pb::delete_scope::Scope) -> Result<pb::DeleteSessionsPlan, tonic::Status> {
        self.daemon
            .session_service()
            .await
            .plan_delete_sessions(pb::PlanDeleteSessionsRequest {
                scope: Some(pb::DeleteScope { scope: Some(scope) }),
            })
            .await
            .map(tonic::Response::into_inner)
    }

    async fn plan_of(&self, sessions: &[&pb::Session]) -> pb::DeleteSessionsPlan {
        self.plan(picked(sessions)).await.unwrap()
    }

    async fn delete(
        &self,
        plan: &pb::DeleteSessionsPlan,
        delete_work: bool,
        typed_count: u32,
    ) -> Result<pb::DeleteSessionsResponse, tonic::Status> {
        self.daemon
            .session_service()
            .await
            .delete_sessions(pb::DeleteSessionsRequest {
                idempotency_key: None,
                plan: plan.token.clone(),
                delete_unpushed_work: delete_work,
                typed_count,
                check_only: false,
            })
            .await
            .map(tonic::Response::into_inner)
    }

    /// Deletes and waits for the operation; the trash items.
    async fn deleted(&self, plan: &pb::DeleteSessionsPlan, typed_count: u32) -> pb::delete_sessions_response::Deleted {
        let answered = self.delete(plan, true, typed_count).await.unwrap();
        let Some(pb::delete_sessions_response::Result::Deleted(done)) = answered.result else {
            panic!("expected a delete, got {answered:?}");
        };
        let operation = finished(&self.daemon, done.operation.as_ref().unwrap()).await;
        assert_eq!(operation.state, pb::OperationState::Succeeded as i32, "{operation:?}");
        done
    }

    async fn listed(&self) -> Vec<pb::Session> {
        self.daemon
            .session_service()
            .await
            .list_sessions(pb::ListSessionsRequest {
                channel: self.channel.id.clone(),
                machine: None,
                include_archived: true,
                page: None,
            })
            .await
            .unwrap()
            .into_inner()
            .sessions
    }

    fn channel_folder(&self) -> PathBuf {
        self.world.channels.join(&self.channel.folder)
    }
}

fn picked(sessions: &[&pb::Session]) -> pb::delete_scope::Scope {
    pb::delete_scope::Scope::Sessions(pb::delete_scope::Picked {
        ids: sessions.iter().filter_map(|session| session.id.clone()).collect(),
    })
}

fn clone_of(session: &pb::Session) -> PathBuf {
    PathBuf::from(&session.repos[0].path)
}

fn as_agent<T>(message: T, token: &str) -> tonic::Request<T> {
    let mut request = tonic::Request::new(message);
    request.metadata_mut().insert(
        "humata-session-token",
        tonic::metadata::MetadataValue::try_from(token).unwrap(),
    );
    request
}

#[tokio::test(flavor = "multi_thread")]
async fn the_plan_lists_terminals_work_and_the_context_fork_and_leaves_out_code_forks_and_the_desk() {
    let fixture = Fixture::new().await;
    let parent = fixture.session("Parent").await;
    std::fs::write(fixture.world.conversation(&parent.conversation_id), transcript()).unwrap();
    std::fs::write(clone_of(&parent).join("wip.txt"), "wip\n").unwrap();
    let (_token, _attach): (String, Attach) = shell_token(&fixture.daemon, parent.id.as_ref().unwrap()).await;
    let context = fixture.fork(&parent, pb::ForkKind::ContextOnly).await;
    let code = fixture.fork(&parent, pb::ForkKind::CodeOnly).await;
    let desk = pb::desk_service_client::DeskServiceClient::new(fixture.daemon.channel().await)
        .start_desk_session(pb::StartDeskSessionRequest {
            title: "Scratch".to_owned(),
            access: Some(pb::DeskAccess { boards: true, channels_and_machines: false }),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    assert!(desk.session.is_some());

    let plan = fixture.plan_of(&[&parent]).await;

    let find = |session: &pb::Session| plan.sessions.iter().find(|s| s.session == session.id);
    let planned = find(&parent).expect("the picked session is in the plan");
    assert_eq!(planned.unpushed.len(), 1, "{plan:?}");
    assert_eq!(planned.unpushed[0].dirty_files, 1);
    assert!(planned.running_terminals >= 2, "agent and shell: {plan:?}");
    assert_eq!(planned.running_terminals as usize, planned.terminals.len());
    assert_eq!(find(&context).expect("the context fork goes along").goes_with, parent.id);
    assert!(find(&code).is_none(), "a code fork is not deleted: {plan:?}");
    assert!(plan.forks_staying.iter().any(|work| work.session == code.id), "{plan:?}");
    assert!(!plan.needs_count);

    let all = fixture.plan(pb::delete_scope::Scope::All(true)).await.unwrap();
    assert_eq!(all.desk_sessions_left_out, 1, "{all:?}");
    assert!(all.needs_count);
    assert!(all.sessions.iter().all(|s| s.channel == fixture.channel.id));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_change_after_the_plan_answers_changed_and_deletes_nothing() {
    let fixture = Fixture::new().await;
    let one = fixture.session("One").await;
    let two = fixture.session("Two").await;
    let plan = fixture.plan_of(&[&one, &two]).await;
    let (_token, _attach) = shell_token(&fixture.daemon, one.id.as_ref().unwrap()).await;

    let answered = fixture.delete(&plan, true, 0).await.unwrap();

    let Some(pb::delete_sessions_response::Result::Changed(changed)) = answered.result else {
        panic!("expected changed, got {answered:?}");
    };
    assert!(!changed.changes.is_empty());
    let fresh = changed.plan.expect("a fresh plan");
    assert_ne!(fresh.token, plan.token);
    assert_eq!(fresh.sessions.len(), 2);
    assert_eq!(fixture.listed().await.len(), 2, "nothing was deleted");
    assert!(clone_of(&one).exists());
    // The fresh plan deletes.
    fixture.deleted(&fresh, 0).await;
    assert!(fixture.listed().await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn unpushed_work_blocks_the_delete_until_it_is_ticked() {
    let fixture = Fixture::new().await;
    let dirty = fixture.session("Dirty").await;
    std::fs::write(clone_of(&dirty).join("wip.txt"), "wip\n").unwrap();
    let plan = fixture.plan_of(&[&dirty]).await;

    let refused = fixture.delete(&plan, false, 0).await.unwrap_err();

    assert_eq!(refused.code(), Code::FailedPrecondition, "{refused:?}");
    assert!(matches!(error_of(&refused).kind, Some(pb::error::Kind::UnpushedWork(_))), "{refused:?}");
    assert_eq!(fixture.listed().await.len(), 1);
    fixture.deleted(&plan, 0).await;
    assert!(fixture.listed().await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn deleting_every_session_needs_the_number_typed() {
    let fixture = Fixture::new().await;
    fixture.session("One").await;
    fixture.session("Two").await;
    let plan = fixture.plan(pb::delete_scope::Scope::All(true)).await.unwrap();
    assert_eq!(plan.sessions.len(), 2);

    let wrong = fixture.delete(&plan, true, 1).await.unwrap_err();

    assert_eq!(wrong.code(), Code::InvalidArgument, "{wrong:?}");
    assert_eq!(fixture.listed().await.len(), 2);
    let done = fixture.deleted(&plan, 2).await;
    assert_eq!(done.sessions, 2);
    assert!(fixture.listed().await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_deleted_session_is_in_the_trash_and_restoring_it_brings_it_back_archived() {
    let fixture = Fixture::new().await;
    let one = fixture.session("One").await;
    let keep = fixture.session("Keep").await;
    let clone = clone_of(&one);
    std::fs::write(clone.join("kept.txt"), "kept\n").unwrap();
    let plan = fixture.plan_of(&[&one]).await;

    let done = fixture.deleted(&plan, 0).await;

    assert!(!clone.exists(), "the folder went to the trash");
    let gone = fixture
        .daemon
        .session_service()
        .await
        .get_session(pb::GetSessionRequest { id: one.id.clone() })
        .await
        .unwrap_err();
    assert_eq!(gone.code(), Code::NotFound, "{gone:?}");
    assert_eq!(fixture.listed().await.len(), 1);
    assert!(get_session(&fixture.daemon, keep.id.as_ref().unwrap()).await.id.is_some());
    let item = done.trash_items.first().expect("a trash item").clone();
    let mut trash = pb::trash_service_client::TrashServiceClient::new(fixture.daemon.channel().await);
    let listed = trash.list_trash(pb::ListTrashRequest::default()).await.unwrap().into_inner();
    assert!(listed.items.iter().any(|i| i.id.as_ref() == Some(&item)), "{listed:?}");

    trash
        .restore_from_trash(pb::RestoreFromTrashRequest { idempotency_key: None, id: Some(item) })
        .await
        .unwrap();

    let back = fixture.listed().await;
    let restored = back.iter().find(|s| s.id == one.id).expect("the session is back");
    assert_eq!(restored.status, pb::SessionStatus::Archived as i32, "{restored:?}");
    assert_eq!(std::fs::read_to_string(clone.join("kept.txt")).unwrap(), "kept\n");
    assert!(fixture.channel_folder().exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_never_gets_scope_all_or_its_own_session() {
    let fixture = Fixture::new().await;
    let mine = fixture.session("Mine").await;
    let other = fixture.session("Other").await;
    let (token, _attach) = shell_token(&fixture.daemon, mine.id.as_ref().unwrap()).await;
    pb::access_service_client::AccessServiceClient::new(fixture.daemon.channel().await)
        .set_channel_rule(pb::SetChannelRuleRequest {
            channel: fixture.channel.id.clone(),
            capability: pb::Capability::SessionsDeleteMany as i32,
            decision: pb::Decision::Allow as i32,
            ..Default::default()
        })
        .await
        .expect("the user sets the rule");
    let mut sessions = fixture.daemon.session_service().await;
    let plan = |scope: pb::delete_scope::Scope| {
        as_agent(
            pb::PlanDeleteSessionsRequest { scope: Some(pb::DeleteScope { scope: Some(scope) }) },
            &token,
        )
    };

    let all = sessions.plan_delete_sessions(plan(pb::delete_scope::Scope::All(true))).await.unwrap_err();
    let own = sessions.plan_delete_sessions(plan(picked(&[&mine]))).await.unwrap_err();
    let allowed = sessions
        .plan_delete_sessions(plan(pb::delete_scope::Scope::Channel(fixture.channel.id.clone().unwrap())))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(all.code(), Code::PermissionDenied, "{all:?}");
    assert_eq!(own.code(), Code::InvalidArgument, "{own:?}");
    let ids: Vec<_> = allowed.sessions.iter().map(|s| s.session.clone()).collect();
    assert_eq!(ids, vec![other.id.clone()], "a channel scope leaves the agent's own session out");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_whose_channel_was_deleted_is_not_restored_and_stays_in_the_trash() {
    let fixture = Fixture::new().await;
    let one = fixture.session("One").await;
    let plan = fixture.plan_of(&[&one]).await;
    let done = fixture.deleted(&plan, 0).await;
    let item = done.trash_items.first().expect("a trash item").clone();
    let mut channels = fixture.daemon.channel_service().await;
    let channel_plan = channels
        .plan_delete_channel(pb::PlanDeleteChannelRequest { channel: fixture.channel.id.clone() })
        .await
        .unwrap()
        .into_inner();
    let ended = channels
        .delete_channel(pb::DeleteChannelRequest {
            idempotency_key: None,
            channel: channel_plan.channel.clone(),
            plan: channel_plan.token.clone(),
            presence: None,
            delete_unpushed_work: true,
            end_live: Some(true),
        })
        .await
        .unwrap()
        .into_inner();
    finished(&fixture.daemon, ended.operation.as_ref().unwrap()).await;
    let mut trash = pb::trash_service_client::TrashServiceClient::new(fixture.daemon.channel().await);

    let refused = trash
        .restore_from_trash(pb::RestoreFromTrashRequest { idempotency_key: None, id: Some(item.clone()) })
        .await
        .unwrap_err();

    assert_eq!(refused.code(), Code::FailedPrecondition, "{refused:?}");
    assert!(!error_of(&refused).fix.is_empty(), "{refused:?}");
    let listed = trash.list_trash(pb::ListTrashRequest::default()).await.unwrap().into_inner();
    assert!(listed.items.iter().any(|i| i.id.as_ref() == Some(&item)), "the item is untouched: {listed:?}");
    let gone = fixture
        .daemon
        .session_service()
        .await
        .get_session(pb::GetSessionRequest { id: one.id.clone() })
        .await
        .unwrap_err();
    assert_eq!(gone.code(), Code::NotFound, "nothing was brought back: {gone:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_with_allow_deletes_a_plan_without_terminals_and_asks_for_one_that_ends_a_terminal() {
    let fixture = Fixture::new().await;
    let mine = fixture.session("Mine").await;
    let quiet = fixture.session("Quiet").await;
    let busy = fixture.session("Busy").await;
    let (token, _attach) = shell_token(&fixture.daemon, mine.id.as_ref().unwrap()).await;
    pb::access_service_client::AccessServiceClient::new(fixture.daemon.channel().await)
        .set_channel_rule(pb::SetChannelRuleRequest {
            channel: fixture.channel.id.clone(),
            capability: pb::Capability::SessionsDeleteMany as i32,
            decision: pb::Decision::Allow as i32,
            ..Default::default()
        })
        .await
        .expect("the user sets the rule");
    fixture
        .daemon
        .session_service()
        .await
        .stop_session(pb::StopSessionRequest { idempotency_key: None, id: quiet.id.clone(), for_move: false })
        .await
        .expect("the session stops");
    common::wait_status(&fixture.daemon, quiet.id.as_ref().unwrap(), pb::SessionStatus::Stopped).await;
    let mut sessions = fixture.daemon.session_service().await;
    let mut as_the_agent = async |session: &pb::Session| {
        let planned = sessions
            .plan_delete_sessions(as_agent(
                pb::PlanDeleteSessionsRequest {
                    scope: Some(pb::DeleteScope { scope: Some(picked(&[session])) }),
                },
                &token,
            ))
            .await
            .unwrap()
            .into_inner();
        sessions
            .delete_sessions(as_agent(
                pb::DeleteSessionsRequest {
                    idempotency_key: None,
                    plan: planned.token.clone(),
                    delete_unpushed_work: true,
                    typed_count: 0,
                    check_only: false,
                },
                &token,
            ))
            .await
    };

    let straight = as_the_agent(&quiet).await;
    let asked = as_the_agent(&busy).await.unwrap_err();

    let done = straight.expect("a plan that ends no terminal goes straight through").into_inner();
    assert!(matches!(done.result, Some(pb::delete_sessions_response::Result::Deleted(_))), "{done:?}");
    assert!(
        matches!(error_of(&asked).kind, Some(pb::error::Kind::ApprovalPending(_))),
        "a plan that ends a terminal asks the owner: {asked:?}"
    );
    assert_eq!(common::pending_requests(&fixture.daemon).await.len(), 1);
    let left: Vec<_> = fixture.listed().await.into_iter().map(|s| s.id).collect();
    assert!(left.contains(&busy.id), "the busy session is untouched");
    assert!(!left.contains(&quiet.id), "the quiet session is deleted");
}
