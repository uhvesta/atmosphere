//! Moving a Desk session into a channel (DK-15..DK-19, D305) through a real
//! `humatad2`. The user plans and moves it, or its own agent asks to and the
//! user approves. The folder lands in the channel's `sessions/`, each repo
//! on the session's branch from where it was, uncommitted files and all. The
//! channel gains the repos it lacked, and the session keeps its id, title
//! and pin. Its Desk row, grants and other requests end. A failed or cut
//! move leaves a Desk session with nothing half-moved; a power cut is
//! desk_into_channel.rs's.

use humatad_test_common as common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use common::{
    Build, Daemon, World, create_channel, error_of, finished, folder, get_session, git, is_running, repo_ref,
    shell_token, wait_running, wait_status,
};
use humata2_grpc::humata::v1 as pb;
use humata2_store::Store;
use pb::SessionStatus as Status;
use tonic::Code;
use tonic::transport::Channel;

/// How long after closing a scratch folder goes, in admin test builds only.
const EXPIRE: &str = "HUMATA2_DESK_EXPIRE_MS";
/// Long enough that no test sees it pass by itself.
const NEVER: &str = "600000";
/// An agent with no turns: a plain terminal (DK-16).
const PLAIN: &str = "plain";

struct Fixture {
    daemon: Daemon,
    world: World,
    /// A channel of `acme/api`; `acme/web` exists on the remote too.
    payments: pb::Channel,
}

impl Fixture {
    async fn new() -> Self {
        let dir = folder();
        let world = World::new(dir.path());
        let mut agents: Vec<serde_json::Value> =
            serde_json::from_str(&std::fs::read_to_string(&world.agents).unwrap()).unwrap();
        let mut plain = agents[0].clone();
        plain["name"] = PLAIN.into();
        plain["turn_end"] = "none".into();
        agents.push(plain);
        std::fs::write(&world.agents, serde_json::to_string(&agents).unwrap()).unwrap();
        world.remote("acme", "api");
        world.remote("acme", "web");
        let args = world.args();
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let daemon = Daemon::start_with_env(Build::Admin, dir, &refs, &[(EXPIRE, NEVER)]).await;
        let (payments, setup) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
        assert_eq!(setup.state, pb::OperationState::Succeeded as i32, "{:?}", setup.error);
        Self {
            daemon,
            world,
            payments,
        }
    }

    async fn desk(&self) -> pb::desk_service_client::DeskServiceClient<Channel> {
        pb::desk_service_client::DeskServiceClient::new(self.daemon.channel().await)
    }

    /// Starts a Desk session "Plan Q4" of `agent` with clones of acme/api and
    /// acme/web, and waits until its agent is up. Its folder and id.
    async fn start_as(&self, agent: Option<&str>) -> (pb::SessionId, PathBuf) {
        let started = self
            .desk()
            .await
            .start_desk_session(pb::StartDeskSessionRequest {
                idempotency_key: None,
                title: "Plan Q4".to_owned(),
                agent: agent.map(|name| pb::AgentId {
                    value: name.to_owned(),
                }),
                model: String::new(),
                machine: None,
                picked_path: String::new(),
                repos: vec![repo_ref("acme", "api"), repo_ref("acme", "web")],
                access: Some(pb::DeskAccess {
                    boards: true,
                    channels_and_machines: false,
                }),
                first_message: String::new(),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner();
        let id = started.session.unwrap();
        // An agent with turns is up once Working, or Probably idle when its
        // output has been quiet 3 s (D169); one without turns stays unknown.
        if agent == Some(PLAIN) {
            wait_status(&self.daemon, &id, Status::RunningUnknownTurn).await;
        } else {
            wait_running(&self.daemon, &id).await;
        }
        (id, PathBuf::from(started.desk.unwrap().folder.unwrap().path))
    }

    async fn start(&self) -> (pb::SessionId, PathBuf) {
        self.start_as(None).await
    }

    async fn plan(&self, id: &pb::SessionId) -> Result<pb::DeskMovePlan, tonic::Status> {
        self.desk()
            .await
            .plan_desk_move(pb::PlanDeskMoveRequest {
                session: Some(id.clone()),
                channel: self.payments.id.clone(),
                branch: String::new(),
            })
            .await
            .map(tonic::Response::into_inner)
    }

    fn move_request(&self, id: &pb::SessionId, plan: &pb::DeskMovePlan) -> pb::MoveDeskSessionToChannelRequest {
        pb::MoveDeskSessionToChannelRequest {
            idempotency_key: None,
            session: Some(id.clone()),
            channel: self.payments.id.clone(),
            plan: plan.token.clone(),
            branch: String::new(),
            reason: String::new(),
            // Its agent is in a turn: the user moves it anyway.
            confirm_running: true,
            ..Default::default()
        }
    }

    /// The user's move of `id` by `plan`, as it started.
    async fn start_move(&self, id: &pb::SessionId, plan: &pb::DeskMovePlan) -> Result<pb::Operation, tonic::Status> {
        self.desk()
            .await
            .move_desk_session_to_channel(self.move_request(id, plan))
            .await
            .map(tonic::Response::into_inner)
    }

    /// The user's move of `id` by `plan`, as it ended.
    async fn move_by(&self, id: &pb::SessionId, plan: &pb::DeskMovePlan) -> Result<pb::Operation, tonic::Status> {
        let started = self.start_move(id, plan).await?;
        Ok(finished(&self.daemon, &started).await)
    }

    /// The agent's own call to move its session into `channel`.
    async fn agent_asks(&self, token: &str, channel: &pb::Channel) -> Result<pb::Operation, tonic::Status> {
        self.desk()
            .await
            .move_desk_session_to_channel(agent_request(
                pb::MoveDeskSessionToChannelRequest {
                    idempotency_key: None,
                    session: None,
                    channel: channel.id.clone(),
                    plan: None,
                    branch: String::new(),
                    reason: "the plan became work on the payments code".to_owned(),
                    ..Default::default()
                },
                token,
            ))
            .await
            .map(tonic::Response::into_inner)
    }

    async fn on_desk(&self, id: &pb::SessionId) -> bool {
        self.desk()
            .await
            .list_desk_sessions(pb::ListDeskSessionsRequest::default())
            .await
            .unwrap()
            .into_inner()
            .sessions
            .iter()
            .any(|desk| desk.session.as_ref() == Some(id))
    }

    async fn channel_repos(&self) -> Vec<pb::RepoRef> {
        self.daemon
            .channel_service()
            .await
            .get_channel(pb::GetChannelRequest {
                id: self.payments.id.clone(),
            })
            .await
            .unwrap()
            .into_inner()
            .repos
    }

    /// Where a session named "Plan Q4" lands in Payments.
    fn landing(&self) -> PathBuf {
        self.world
            .channels
            .join(&self.payments.folder)
            .join("sessions")
            .join("plan-q4")
    }

    async fn set_rule(&self, capability: pb::Capability, decision: pb::Decision) {
        pb::access_service_client::AccessServiceClient::new(self.daemon.channel().await)
            .set_channel_rule(pb::SetChannelRuleRequest {
                channel: self.payments.id.clone(),
                capability: capability as i32,
                decision: decision as i32,
                ..Default::default()
            })
            .await
            .unwrap();
    }

    async fn pin(&self, id: &pb::SessionId) {
        pb::mark_service_client::MarkServiceClient::new(self.daemon.channel().await)
            .set_marks(pb::SetMarksRequest {
                idempotency_key: None,
                entity: Some(pb::EntityRef {
                    entity: Some(pb::entity_ref::Entity::Session(id.clone())),
                }),
                pinned: Some(true),
                vip: None,
            })
            .await
            .unwrap();
    }

    async fn grants_of(&self, id: &pb::SessionId) -> Vec<pb::Grant> {
        pb::access_service_client::AccessServiceClient::new(self.daemon.channel().await)
            .list_grants(pb::ListGrantsRequest {
                requester: Some(id.clone()),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
            .grants
    }

    /// The agent of `token` asks ahead to reach a port; the request.
    async fn ask_network(&self, token: &str, port: u32) -> pb::RequestId {
        self.daemon
            .inbox_service()
            .await
            .request_access(agent_request(
                pb::RequestAccessRequest {
                    capability: pb::Capability::NetworkReach as i32,
                    target: Some(network("localhost", port)),
                    reason: "to run the tests".to_owned(),
                    ..Default::default()
                },
                token,
            ))
            .await
            .unwrap()
            .into_inner()
            .id
            .unwrap()
    }

    /// The user approves `id` with the "Always allow" it offers.
    async fn always(&self, id: &pb::RequestId) {
        let mut inbox = self.daemon.inbox_service().await;
        let offered = inbox
            .get_request(pb::GetRequestRequest { id: Some(id.clone()) })
            .await
            .unwrap()
            .into_inner()
            .always
            .and_then(|always| always.target);
        inbox
            .answer_request(pb::AnswerRequestRequest {
                id: Some(id.clone()),
                answer: Some(pb::RequestAnswer {
                    answer: Some(pb::request_answer::Answer::Always(pb::AlwaysLet { target: offered })),
                }),
                ..Default::default()
            })
            .await
            .unwrap();
    }

    async fn request(&self, id: &pb::RequestId) -> pb::InboxRequest {
        inbox(&self.daemon)
            .await
            .into_iter()
            .find(|request| request.id.as_ref() == Some(id))
            .expect("the request is listed")
    }

    /// Waits until the session is back up as a channel session of Payments.
    async fn wait_moved(&self, id: &pb::SessionId) -> pb::Session {
        let deadline = std::time::Instant::now() + Duration::from_secs(90);
        loop {
            let session = get_session(&self.daemon, id).await;
            if session.channel == self.payments.id && is_running(&session) {
                return session;
            }
            assert!(std::time::Instant::now() < deadline, "the session never came up moved: {session:?}");
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }
}

fn agent_request<T>(message: T, token: &str) -> tonic::Request<T> {
    let mut request = tonic::Request::new(message);
    request.metadata_mut().insert(
        "humata-session-token",
        tonic::metadata::MetadataValue::try_from(token).unwrap(),
    );
    request
}

fn network(host: &str, port: u32) -> pb::GrantTarget {
    pb::GrantTarget {
        target: Some(pb::grant_target::Target::Network(pb::NetworkTarget {
            host: host.to_owned(),
            port,
            machine: None,
        })),
    }
}

/// The request an agent's call is waiting on, from the pending error.
fn pending_request(refused: &tonic::Status) -> pb::RequestId {
    match error_of(refused).kind {
        Some(pb::error::Kind::ApprovalPending(pending)) => pending.request.unwrap(),
        other => panic!("expected approval_pending, got {other:?}"),
    }
}

fn is_plan_stale(error: &pb::Error) -> bool {
    matches!(error.kind, Some(pb::error::Kind::PlanStale(_)))
}

/// Every Inbox request the daemon holds, answered ones too.
async fn inbox(daemon: &Daemon) -> Vec<pb::InboxRequest> {
    daemon
        .inbox_service()
        .await
        .list_requests(pb::ListRequestsRequest {
            include_answered: true,
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner()
        .requests
}

/// The user approves `id`, and the agent waits for what the approved call
/// returned.
async fn approve_and_wait(daemon: &Daemon, token: &str, id: &pb::RequestId) -> pb::WaitResponse {
    let mut inbox = daemon.inbox_service().await;
    inbox
        .answer_request(pb::AnswerRequestRequest {
            id: Some(id.clone()),
            answer: Some(pb::RequestAnswer {
                answer: Some(pb::request_answer::Answer::Approve(pb::Approve {})),
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    let mut stream = inbox
        .wait_for_answer(agent_request(
            pb::WaitForAnswerRequest {
                id: Some(id.clone()),
                wait_seconds: 10,
            },
            token,
        ))
        .await
        .unwrap()
        .into_inner();
    loop {
        let message = tokio::time::timeout(common::WAIT, stream.message())
            .await
            .expect("the wait ended in time")
            .unwrap()
            .expect("the wait ended with an answer");
        if !message.keepalive {
            return message;
        }
    }
}

fn head(clone: &Path) -> String {
    git(&["-C", &clone.display().to_string(), "rev-parse", "HEAD"])
}

fn branch_of(clone: &Path) -> String {
    git(&["-C", &clone.display().to_string(), "symbolic-ref", "--short", "HEAD"])
}

fn succeeded(operation: &pb::Operation) -> bool {
    operation.state == pb::OperationState::Succeeded as i32
}

#[tokio::test(flavor = "multi_thread")]
async fn a_moved_session_lands_in_the_channel_with_its_work_on_a_new_branch() {
    let fixture = Fixture::new().await;
    let (id, desk) = fixture.start().await;
    fixture.pin(&id).await;
    std::fs::write(desk.join("api/wip.txt"), "half done\n").unwrap();
    std::fs::write(desk.join("notes.md"), "# Q4\n").unwrap();
    let api_head = head(&desk.join("api"));
    let web_head = head(&desk.join("web"));

    let plan = fixture.plan(&id).await.unwrap();
    assert_eq!(PathBuf::from(&plan.folder), fixture.landing());
    assert_eq!(plan.adds, vec![repo_ref("acme", "web")]);
    assert!(!plan.branch.is_empty());
    let moved = fixture.move_by(&id, &plan).await.unwrap();
    assert!(succeeded(&moved), "{:?}", moved.error);

    let session = fixture.wait_moved(&id).await;
    assert_eq!(session.title, "Plan Q4");
    assert!(session.pinned);
    let to = fixture.landing();
    assert_eq!(std::fs::read_to_string(to.join("api/wip.txt")).unwrap(), "half done\n");
    assert_eq!(std::fs::read_to_string(to.join("notes.md")).unwrap(), "# Q4\n");
    assert_eq!(head(&to.join("api")), api_head);
    assert_eq!(head(&to.join("web")), web_head);
    for name in ["api", "web"] {
        assert_eq!(branch_of(&to.join(name)), plan.branch, "{name}");
        let repo = session
            .repos
            .iter()
            .find(|repo| repo.repo == Some(repo_ref("acme", name)))
            .unwrap_or_else(|| panic!("the session lists {name}"));
        assert_eq!(repo.branch, plan.branch);
        assert_eq!(PathBuf::from(&repo.path), to.join(name));
    }
    let repos = fixture.channel_repos().await;
    assert!(repos.contains(&repo_ref("acme", "web")), "{repos:?}");
    assert!(!fixture.on_desk(&id).await);
    // D801 E: the scratch folder was renamed, not copied, and its agent runs
    // on; its old name is a link to the new folder while the agent runs.
    assert!(std::fs::symlink_metadata(&desk).unwrap().file_type().is_symlink());
    assert_eq!(std::fs::read_to_string(desk.join("notes.md")).unwrap(), "# Q4\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_ends_the_sessions_grants_and_withdraws_its_requests() {
    let fixture = Fixture::new().await;
    let (id, _) = fixture.start().await;
    let (token, _attach) = shell_token(&fixture.daemon, &id).await;
    let granted = fixture.ask_network(&token, 5432).await;
    fixture.always(&granted).await;
    assert_eq!(fixture.grants_of(&id).await.len(), 1);
    let waiting = fixture.ask_network(&token, 5433).await;

    let plan = fixture.plan(&id).await.unwrap();
    let moved = fixture.move_by(&id, &plan).await.unwrap();
    assert!(succeeded(&moved), "{:?}", moved.error);

    assert!(fixture.grants_of(&id).await.is_empty());
    let request = fixture.request(&waiting).await;
    assert_eq!(request.state(), pb::RequestState::Withdrawn);
    assert_eq!(request.withdraw_reason(), pb::WithdrawReason::SessionMoved);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agents_move_asks_under_the_target_channels_rule_and_runs_once_approved() {
    let fixture = Fixture::new().await;
    let (id, _) = fixture.start().await;
    let (token, _attach) = shell_token(&fixture.daemon, &id).await;

    let pending = fixture.agent_asks(&token, &fixture.payments).await.unwrap_err();
    let asked = pending_request(&pending);
    let request = fixture.request(&asked).await;
    assert_eq!(request.capability(), pb::Capability::DeskMoveToChannel);
    assert_eq!(request.channel, fixture.payments.id);
    assert!(request.always.is_none(), "no Always allow is offered: {request:?}");
    assert!(fixture.on_desk(&id).await, "nothing moves before the answer");

    let waited = approve_and_wait(&fixture.daemon, &token, &asked).await;
    assert_eq!(waited.result_type, "humata.v1.Operation", "{:?}", waited.error);
    let operation = <pb::Operation as prost::Message>::decode(waited.result.as_slice()).unwrap();
    let moved = finished(&fixture.daemon, &operation).await;
    assert!(succeeded(&moved), "{:?}", moved.error);
    fixture.wait_moved(&id).await;
    assert!(fixture.landing().join("api").exists());
    assert!(!fixture.on_desk(&id).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agents_move_that_adds_repos_is_refused_when_the_target_never_takes_one() {
    let fixture = Fixture::new().await;
    fixture.set_rule(pb::Capability::ChannelRepoAdd, pb::Decision::Never).await;
    let (id, desk) = fixture.start().await;
    let (token, _attach) = shell_token(&fixture.daemon, &id).await;

    let refused = fixture.agent_asks(&token, &fixture.payments).await.unwrap_err();

    assert_eq!(refused.code(), Code::PermissionDenied, "{refused:?}");
    assert!(
        inbox(&fixture.daemon)
            .await
            .iter()
            .all(|request| request.capability() != pb::Capability::DeskMoveToChannel),
        "nothing was asked"
    );
    assert!(fixture.on_desk(&id).await);
    assert!(desk.join("web").exists());
    assert!(!fixture.channel_repos().await.contains(&repo_ref("acme", "web")));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_can_not_move_a_plain_terminal_session() {
    let fixture = Fixture::new().await;
    let (id, desk) = fixture.start_as(Some(PLAIN)).await;
    let (token, _attach) = shell_token(&fixture.daemon, &id).await;

    let refused = fixture.agent_asks(&token, &fixture.payments).await.unwrap_err();

    assert_eq!(refused.code(), Code::FailedPrecondition, "{refused:?}");
    assert!(fixture.on_desk(&id).await);
    assert!(desk.exists());
    assert!(!fixture.landing().exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_closed_session_is_not_moved() {
    let fixture = Fixture::new().await;
    let (id, desk) = fixture.start().await;
    let plan = fixture.plan(&id).await.unwrap();
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

    let refused = fixture.move_by(&id, &plan).await.unwrap_err();

    assert_eq!(refused.code(), Code::FailedPrecondition, "{refused:?}");
    assert!(fixture.on_desk(&id).await);
    assert!(desk.exists());
    assert!(!fixture.landing().exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_by_a_plan_the_folder_has_outgrown_moves_nothing() {
    let fixture = Fixture::new().await;
    let (id, desk) = fixture.start().await;
    let plan = fixture.plan(&id).await.unwrap();
    std::fs::write(desk.join("api/late.txt"), "written after the plan\n").unwrap();

    let refused = fixture.move_by(&id, &plan).await.unwrap_err();

    assert_eq!(refused.code(), Code::FailedPrecondition, "{refused:?}");
    assert!(is_plan_stale(&error_of(&refused)), "{refused:?}");
    assert!(fixture.on_desk(&id).await);
    assert!(!fixture.landing().exists());
    assert!(!fixture.channel_repos().await.contains(&repo_ref("acme", "web")));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_approval_after_the_folder_changed_moves_nothing() {
    let fixture = Fixture::new().await;
    let (id, desk) = fixture.start().await;
    let (token, _attach) = shell_token(&fixture.daemon, &id).await;
    let asked = pending_request(&fixture.agent_asks(&token, &fixture.payments).await.unwrap_err());
    std::fs::write(desk.join("api/late.txt"), "written after the ask\n").unwrap();

    let waited = approve_and_wait(&fixture.daemon, &token, &asked).await;

    let error = waited.error.expect("the approved call failed");
    assert!(is_plan_stale(&error), "{error:?}");
    assert!(fixture.on_desk(&id).await);
    assert!(!fixture.landing().exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_move_request_withdraws_the_first() {
    let fixture = Fixture::new().await;
    let (search, _) = create_channel(&fixture.daemon, "Search", &[("acme", "api")]).await;
    let (id, _) = fixture.start().await;
    let (token, _attach) = shell_token(&fixture.daemon, &id).await;
    let first = pending_request(&fixture.agent_asks(&token, &fixture.payments).await.unwrap_err());

    let second = pending_request(&fixture.agent_asks(&token, &search).await.unwrap_err());

    assert_ne!(first, second);
    let withdrawn = fixture.request(&first).await;
    assert_eq!(withdrawn.state(), pb::RequestState::Withdrawn);
    assert_eq!(withdrawn.withdraw_reason(), pb::WithdrawReason::Superseded);
    assert_eq!(fixture.request(&second).await.state(), pb::RequestState::Pending);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_that_fails_leaves_a_running_desk_session_and_nothing_half_moved() {
    let fixture = Fixture::new().await;
    let (id, desk) = fixture.start().await;
    std::fs::write(desk.join("api/wip.txt"), "half done\n").unwrap();
    let plan = fixture.plan(&id).await.unwrap();
    // The Desk clone already has a branch of the plan's name, which the
    // plan doesn't look at, so putting its clone on that branch fails after
    // the folder was renamed into the channel: the rename is undone.
    git(&["-C", &desk.join("api").display().to_string(), "branch", &plan.branch]);

    let ended = fixture.move_by(&id, &plan).await;

    assert!(!ended.as_ref().is_ok_and(succeeded), "{ended:?}");
    wait_running(&fixture.daemon, &id).await;
    let session = get_session(&fixture.daemon, &id).await;
    assert_ne!(session.channel, fixture.payments.id);
    assert!(fixture.on_desk(&id).await);
    assert_eq!(branch_of(&desk.join("api")), "main");
    assert_eq!(std::fs::read_to_string(desk.join("api/wip.txt")).unwrap(), "half done\n");
    assert!(!fixture.landing().exists());
}

/// DV-10: moving a Desk session into a channel keeps its versions: the
/// folder's repository lands with the folder, on the same commits, still
/// leaving its clones out, and the comments recorded for them stay with the
/// session, whose id the move keeps.
#[tokio::test(flavor = "multi_thread")]
async fn a_desk_session_moved_into_a_channel_keeps_its_versions() {
    let fixture = Fixture::new().await;
    let (id, desk) = fixture.start().await;
    let there = desk.display().to_string();
    std::fs::write(desk.join("notes.md"), "# Q4\n").unwrap();
    git(&["-C", &there, "add", "notes.md"]);
    git(&["-C", &there, "commit", "-q", "-m", "First notes"]);
    let head = git(&["-C", &there, "rev-parse", "HEAD"]);
    let versions = git(&["-C", &there, "log", "--format=%H", "--", "notes.md"]);
    let recorded: &[u8] = b"a comment as it was";
    let store = Store::open(fixture.daemon.state.join("store")).unwrap();
    assert!(store.put_doc_comments(&id.value, "feedface00", "notes.md", recorded, 1).unwrap());

    let plan = fixture.plan(&id).await.unwrap();
    let moved = fixture.move_by(&id, &plan).await.unwrap();
    assert!(succeeded(&moved), "{:?}", moved.error);
    fixture.wait_moved(&id).await;

    let to = fixture.landing();
    let here = to.display().to_string();
    assert_eq!(git(&["-C", &here, "rev-parse", "HEAD"]), head);
    assert_eq!(git(&["-C", &here, "log", "--format=%H", "--", "notes.md"]), versions);
    let status = git(&["-C", &here, "status", "--porcelain", "--untracked-files=all"]);
    assert!(
        !status.lines().any(|line| line.contains("api/") || line.contains("web/")),
        "the clones stay out of the versions: {status}"
    );
    assert_eq!(
        store.doc_comments(&id.value, "feedface00", "notes.md").unwrap().as_deref(),
        Some(recorded)
    );
}
