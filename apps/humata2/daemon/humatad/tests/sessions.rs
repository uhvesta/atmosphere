//! SessionService through a real `humatad2` (SE-1..4, FC-11, FC-14, PER-3,
//! PER-23): a session gets its clone on a branch named from its title, its
//! holder keeps the agent's terminal, a reattach replays, and the kill
//! matrix (the app, the daemon, the holder, a reboot) brings the session
//! back in its conversation or says what was lost.

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::{
    Attach, Daemon, World, agent_tab, create_channel, finished, get_session, git, holder_answers,
    humata, kill_holders, last_start, machine_id, open, start_session, start_world, stored_tabs,
    wait_status,
};
use humata2_grpc::humata::v1 as pb;
use pb::SessionStatus as Status;
use tonic::Request;
use tonic::metadata::MetadataValue;

/// A world with one repository, one channel, and one working session.
struct Fixture {
    daemon: Daemon,
    world: World,
    session: pb::Session,
}

impl Fixture {
    async fn new() -> Self {
        let (daemon, world) = start_world().await;
        world.remote("acme", "api");
        let (channel, setup) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
        assert_eq!(
            setup.state,
            pb::OperationState::Succeeded as i32,
            "{:?}",
            setup.error
        );
        let (id, operation) = start_session(&daemon, &channel, "Fix login").await;
        assert_eq!(
            operation.state,
            pb::OperationState::Succeeded as i32,
            "{:?}",
            operation.error
        );
        let session = wait_status(&daemon, &id, Status::Working).await;
        Self {
            daemon,
            world,
            session,
        }
    }

    fn id(&self) -> pb::SessionId {
        self.session.id.clone().unwrap()
    }

    async fn tab(&self) -> pb::TabId {
        agent_tab(&self.daemon, &self.id()).await
    }

    /// Types `line` into the agent and waits for its answer.
    async fn say(&self, line: &str) {
        let mut attach = Attach::start(&self.daemon, open(&self.tab().await, 0)).await;
        attach.type_text(&format!("{line}\r")).await;
        attach.wait_for(&format!("got: {line}")).await;
    }

    fn conversation(&self) -> String {
        std::fs::read_to_string(self.world.conversation(&self.session.conversation_id)).unwrap()
    }

    /// The holders of this session's tabs, as the store has them.
    fn agent_holders(&self) -> Vec<humata2_store::TabRecord> {
        let id = self.id().value;
        stored_tabs(&self.daemon.state)
            .into_iter()
            .filter(|tab| tab.session.as_deref() == Some(id.as_str()))
            .collect()
    }
}

async fn preview(daemon: &Daemon, channel: &pb::Channel, title: &str) -> Vec<String> {
    daemon
        .session_service()
        .await
        .preview_branch_names(pb::PreviewBranchNamesRequest {
            channel: channel.id.clone(),
            title: title.to_owned(),
            machine: Some(machine_id(daemon).await),
            repos: Vec::new(),
        })
        .await
        .unwrap()
        .into_inner()
        .names
        .into_iter()
        .map(|name| name.branch)
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_started_session_has_its_clone_on_a_branch_named_from_its_title() {
    let (daemon, world) = start_world().await;
    world.remote("acme", "api");
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    assert_eq!(preview(&daemon, &channel, "Fix login").await, ["fix-login"]);

    let (id, operation) = start_session(&daemon, &channel, "Fix login").await;

    assert_eq!(
        operation.state,
        pb::OperationState::Succeeded as i32,
        "{:?}",
        operation.error
    );
    assert!(
        operation
            .steps
            .iter()
            .all(|step| step.state == pb::StepState::Done as i32),
        "{:?}",
        operation.steps
    );
    let session = wait_status(&daemon, &id, Status::Working).await;
    assert_eq!(session.title, "Fix login");
    assert!(!session.conversation_id.is_empty());
    let repo = &session.repos[0];
    assert_eq!(repo.branch, "fix-login");
    let clone: PathBuf = world
        .channels
        .join(&channel.folder)
        .join("sessions/fix-login/api");
    assert_eq!(
        std::fs::canonicalize(&repo.path).unwrap(),
        std::fs::canonicalize(&clone).unwrap()
    );
    assert_eq!(
        git(&[
            "-C",
            &clone.display().to_string(),
            "rev-parse",
            "--abbrev-ref",
            "HEAD"
        ]),
        "fix-login"
    );

    let listed = daemon
        .session_service()
        .await
        .list_sessions(pb::ListSessionsRequest::default())
        .await
        .unwrap()
        .into_inner()
        .sessions;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, session.id);
    assert_eq!(listed[0].status, Status::Working as i32);
    // The same title again would now get the numbered name (FC-14).
    assert_eq!(
        preview(&daemon, &channel, "Fix login").await,
        ["fix-login-2"]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_started_session_tells_its_own_folder_and_the_clones_inside_it() {
    let (daemon, world) = start_world().await;
    world.remote("acme", "api");
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    let (id, _) = start_session(&daemon, &channel, "Fix login").await;
    let session = wait_status(&daemon, &id, Status::Working).await;

    let folder = PathBuf::from(&session.folder);
    assert!(folder.is_absolute(), "{folder:?}");
    assert!(folder.is_dir(), "{folder:?}");
    let clone = PathBuf::from(&session.repos[0].path);
    assert!(clone.is_absolute(), "{clone:?}");
    assert!(clone.starts_with(&folder), "{clone:?} is outside {folder:?}");
    assert!(clone.is_dir(), "{clone:?}");
    // GetSession says the same as the list.
    assert_eq!(get_session(&daemon, &id).await.folder, session.folder);
}

#[tokio::test(flavor = "multi_thread")]
async fn typed_input_reaches_the_agent_and_a_reattach_replays_it() {
    let fix = Fixture::new().await;
    let tab = fix.tab().await;
    let mut first = Attach::start(&fix.daemon, open(&tab, 0)).await;
    first.type_text("ping-1\r").await;
    first.wait_for("got: ping-1").await;
    drop(first);

    let mut again = Attach::start(&fix.daemon, open(&tab, 0)).await;
    again.wait_for("got: ping-1").await;

    assert_eq!(again.snapshots, 1);
    assert!(fix.conversation().contains("ping-1"));
}

#[tokio::test(flavor = "multi_thread")]
async fn dropping_the_app_leaves_the_session_working_and_replayable() {
    let fix = Fixture::new().await;
    let tab = fix.tab().await;
    let mut app = Attach::start(&fix.daemon, open(&tab, 0)).await;
    app.type_text("before\r").await;
    app.wait_for("got: before").await;
    // The app goes away mid-stream, with its connection.
    drop(app);

    assert_eq!(
        get_session(&fix.daemon, &fix.id()).await.status,
        Status::Working as i32
    );
    let mut back = Attach::start(&fix.daemon, open(&tab, 0)).await;
    back.wait_for("got: before").await;
    back.type_text("after\r").await;
    back.wait_for("got: after").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn killing_the_daemon_leaves_the_holder_and_the_conversation_alone() {
    let mut fix = Fixture::new().await;
    fix.say("one").await;
    let id = fix.id();

    fix.daemon.kill();
    let before = fix.agent_holders();
    assert!(!before.is_empty());
    assert!(
        before.iter().all(holder_answers),
        "a holder died with the daemon"
    );
    fix.daemon.restart().await;

    let start = last_start(&fix.daemon).await;
    assert!(
        start.not_brought_back.is_empty(),
        "{:?}",
        start.not_brought_back
    );
    let session = wait_status(&fix.daemon, &id, Status::Working).await;
    assert_eq!(session.conversation_id, fix.session.conversation_id);
    fix.say("two").await;
    let conversation = fix.conversation();
    assert!(conversation.contains("one") && conversation.contains("two"));
    fix.daemon.kill();
    let after = fix.agent_holders();
    let pids = |tabs: &[humata2_store::TabRecord]| {
        tabs.iter().map(|tab| tab.holder_pid).collect::<Vec<_>>()
    };
    assert_eq!(pids(&before), pids(&after));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_lost_holder_is_resumed_in_its_conversation_after_a_restart() {
    let mut fix = Fixture::new().await;
    fix.say("one").await;
    let id = fix.id();

    fix.daemon.kill();
    kill_holders(&fix.agent_holders()).await;
    fix.daemon.restart().await;

    let start = last_start(&fix.daemon).await;
    assert!(
        start.not_brought_back.is_empty(),
        "{:?}",
        start.not_brought_back
    );
    let session = wait_status(&fix.daemon, &id, Status::Working).await;
    assert_eq!(session.conversation_id, fix.session.conversation_id);
    let mut attach = Attach::start(&fix.daemon, open(&fix.tab().await, 0)).await;
    attach.wait_for("agent up --resume").await;
    assert!(fix.conversation().contains("one"));
    fix.say("two").await;
    assert!(fix.conversation().contains("two"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reboot_brings_back_the_session_and_a_shell() {
    let mut fix = Fixture::new().await;
    fix.say("one").await;
    let id = fix.id();
    let shell = fix
        .daemon
        .terminals()
        .await
        .open_machine_shell(pb::OpenMachineShellRequest::default())
        .await
        .unwrap()
        .into_inner()
        .id
        .unwrap();

    // Everything goes at once: the daemon and every holder.
    fix.daemon.kill();
    kill_holders(&stored_tabs(&fix.daemon.state)).await;
    fix.daemon.restart().await;

    let start = last_start(&fix.daemon).await;
    assert!(
        start.not_brought_back.is_empty(),
        "{:?}",
        start.not_brought_back
    );
    let session = wait_status(&fix.daemon, &id, Status::Working).await;
    assert_eq!(session.conversation_id, fix.session.conversation_id);
    assert!(fix.conversation().contains("one"));
    fix.say("two").await;
    let mut after = Attach::start(&fix.daemon, open(&shell, 0)).await;
    after.type_text("echo bac''k\n").await;
    after.wait_for("back").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reboot_that_lost_the_conversation_says_the_resume_failed() {
    let mut fix = Fixture::new().await;
    fix.say("one").await;
    let id = fix.id();

    fix.daemon.kill();
    kill_holders(&fix.agent_holders()).await;
    std::fs::remove_file(fix.world.conversation(&fix.session.conversation_id)).unwrap();
    fix.daemon.restart().await;

    let start = last_start(&fix.daemon).await;
    let line = start
        .not_brought_back
        .iter()
        .find(|line| line.session == id.value)
        .unwrap_or_else(|| {
            panic!(
                "the start doesn't name the session: {:?}",
                start.not_brought_back
            )
        });
    assert!(!line.problem.is_empty());
    let session = wait_status(&fix.daemon, &id, Status::ResumeFailed).await;
    assert_eq!(session.conversation_id, fix.session.conversation_id);
    // Nothing was started in its place.
    assert!(fix.agent_holders().iter().all(|tab| !holder_answers(tab)));
}

#[tokio::test(flavor = "multi_thread")]
async fn stopping_ends_the_agent_and_resuming_brings_it_back_in_its_conversation() {
    let fix = Fixture::new().await;
    fix.say("one").await;
    let id = fix.id();
    let holders = fix.agent_holders();
    assert!(holders.iter().all(holder_answers));

    let stopped = fix
        .daemon
        .session_service()
        .await
        .stop_session(pb::StopSessionRequest {
            idempotency_key: None,
            id: Some(id.clone()),
        })
        .await
        .unwrap()
        .into_inner();

    // A stopped agent reads as stopped, not exited, and offers Resume
    // (D784, session_status in session_service.rs).
    assert_eq!(stopped.status, Status::Stopped as i32);
    assert_eq!(
        get_session(&fix.daemon, &id).await.status,
        Status::Stopped as i32
    );
    let deadline = tokio::time::Instant::now() + common::WAIT;
    while holders.iter().any(holder_answers) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the holder never went"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    let operation = fix
        .daemon
        .session_service()
        .await
        .resume_session(pb::ResumeSessionRequest {
            idempotency_key: None,
            id: Some(id.clone()),
            mode: pb::resume_session_request::Mode::Resume as i32,
        })
        .await
        .unwrap()
        .into_inner();
    let operation = finished(&fix.daemon, &operation).await;

    assert_eq!(
        operation.state,
        pb::OperationState::Succeeded as i32,
        "{:?}",
        operation.error
    );
    let session = wait_status(&fix.daemon, &id, Status::Working).await;
    assert_eq!(session.conversation_id, fix.session.conversation_id);
    let mut attach = Attach::start(&fix.daemon, open(&fix.tab().await, 0)).await;
    attach.wait_for("agent up --resume").await;
    assert!(fix.conversation().contains("one"));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_cli_starts_a_session_and_lists_it() {
    let (daemon, world) = start_world().await;
    world.remote("acme", "api");
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    let channel_id = channel.id.clone().unwrap().value;
    let machine = machine_id(&daemon).await.value;

    let (code, json) = humata(
        &daemon,
        &[
            "--channel",
            &channel_id,
            "--machine",
            &machine,
            "session",
            "start",
            "--title",
            "Fix login",
        ],
    );

    assert_eq!(code, 0, "{json}");
    let listed = daemon
        .session_service()
        .await
        .list_sessions(pb::ListSessionsRequest::default())
        .await
        .unwrap()
        .into_inner()
        .sessions;
    let session = listed
        .iter()
        .find(|session| session.title == "Fix login")
        .unwrap();
    let id = session.id.clone().unwrap();
    wait_status(&daemon, &id, Status::Working).await;
    let (code, json) = humata(&daemon, &["ls"]);
    assert_eq!(code, 0, "{json}");
    assert!(json.to_string().contains(&id.value), "{json}");
}

/// `path`'s contents once something non-empty is written there.
async fn read_when_written(path: &Path) -> String {
    let deadline = Instant::now() + common::WAIT;
    loop {
        match std::fs::read_to_string(path) {
            Ok(text) if !text.is_empty() => return text,
            _ => {}
        }
        assert!(
            Instant::now() < deadline,
            "{} was never written",
            path.display()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// The session token a shell tab of the session was given (SEC-8: a
/// command run there acts as the session's agent).
async fn shell_token(fix: &Fixture) -> String {
    let tab = fix
        .daemon
        .terminals()
        .await
        .open_shell_tab(pb::OpenShellTabRequest {
            idempotency_key: None,
            session: Some(fix.id()),
            repo: None,
            folder: String::new(),
        })
        .await
        .unwrap()
        .into_inner()
        .id
        .unwrap();
    let file = fix.daemon.dir.path().join("token");
    let mut attach = Attach::start(&fix.daemon, open(&tab, 0)).await;
    attach
        .type_text(&format!(
            "printf '%s' \"$HUMATA_SESSION_TOKEN\" > {}\r",
            file.display()
        ))
        .await;
    read_when_written(&file).await
}

fn as_agent<T>(message: T, token: &str) -> Request<T> {
    let mut request = Request::new(message);
    request.metadata_mut().insert(
        "humata-session-token",
        MetadataValue::try_from(token).unwrap(),
    );
    request
}

#[tokio::test(flavor = "multi_thread")]
async fn stopping_a_session_withdraws_its_pending_requests() {
    let fix = Fixture::new().await;
    let token = shell_token(&fix).await;
    let mut inbox = pb::inbox_service_client::InboxServiceClient::new(fix.daemon.channel().await);
    let asked = inbox
        .ask_user(as_agent(
            pb::AskUserRequest {
                question: "Which branch?".to_owned(),
                ..Default::default()
            },
            &token,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(asked.state(), pb::RequestState::Pending);

    fix.daemon
        .session_service()
        .await
        .stop_session(pb::StopSessionRequest {
            idempotency_key: None,
            id: Some(fix.id()),
        })
        .await
        .unwrap();

    let mut stream = inbox
        .wait_for_answer(as_agent(
            pb::WaitForAnswerRequest {
                id: asked.id.clone(),
                wait_seconds: 10,
            },
            &token,
        ))
        .await
        .unwrap()
        .into_inner();
    let ended = loop {
        let message = tokio::time::timeout(common::WAIT, stream.message())
            .await
            .expect("the wait ended in time")
            .unwrap()
            .expect("the wait ended with a state");
        if !message.keepalive {
            break message;
        }
    };
    assert_eq!(ended.state(), pb::RequestState::Withdrawn);
}

/// The user answers `id` with an approval, and the agent waits for what the
/// approved call returned: its type name and encoded response.
async fn approve_and_wait(
    daemon: &common::Daemon,
    token: &str,
    id: &pb::RequestId,
) -> pb::WaitResponse {
    let mut inbox = pb::inbox_service_client::InboxServiceClient::new(daemon.channel().await);
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

/// The request an agent's call is waiting on, from the pending error.
fn pending_request(refused: &tonic::Status) -> pb::RequestId {
    match common::error_of(refused).kind {
        Some(pb::error::Kind::ApprovalPending(pending)) => pending.request.unwrap(),
        other => panic!("expected approval_pending, got {other:?}"),
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

/// Makes the channel's rule for `capability` Ask me.
async fn ask_me(daemon: &common::Daemon, channel: &pb::ChannelId, capability: pb::Capability) {
    pb::access_service_client::AccessServiceClient::new(daemon.channel().await)
        .set_channel_rule(pb::SetChannelRuleRequest {
            channel: Some(channel.clone()),
            capability: capability as i32,
            decision: pb::Decision::Ask as i32,
            ..Default::default()
        })
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agents_rename_that_a_rule_sends_to_ask_runs_once_the_user_approves() {
    let fix = Fixture::new().await;
    let token = shell_token(&fix).await;
    ask_me(
        &fix.daemon,
        fix.session.channel.as_ref().unwrap(),
        pb::Capability::OwnSessionManage,
    )
    .await;

    let refused = fix
        .daemon
        .session_service()
        .await
        .rename_session(agent_request(
            pb::RenameSessionRequest {
                id: Some(fix.id()),
                title: "Renamed by agent".to_owned(),
                ..Default::default()
            },
            &token,
        ))
        .await
        .unwrap_err();
    let waited = approve_and_wait(&fix.daemon, &token, &pending_request(&refused)).await;

    assert_eq!(waited.result_type, "humata.v1.Session");
    let renamed = <pb::Session as prost::Message>::decode(waited.result.as_slice()).unwrap();
    assert_eq!(renamed.title, "Renamed by agent");
}

/// D757: the after-move mode is the move's own. A caller that sends it for
/// a session that never moved gets the strict resume: a lost conversation
/// is a failed resume, never a fresh agent in its place.
#[tokio::test(flavor = "multi_thread")]
async fn an_after_move_resume_of_a_session_that_never_moved_does_not_start_it_fresh() {
    let (mut daemon, world) = start_world().await;
    world.remote("acme", "api");
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    let (id, _) = start_session(&daemon, &channel, "Fix login").await;
    let session = wait_status(&daemon, &id, Status::Working).await;
    let tabs: Vec<_> = stored_tabs(&daemon.state)
        .into_iter()
        .filter(|tab| tab.session.as_deref() == Some(id.value.as_str()))
        .collect();
    daemon.kill();
    kill_holders(&tabs).await;
    std::fs::remove_file(world.conversation(&session.conversation_id)).unwrap();
    daemon.restart().await;
    wait_status(&daemon, &id, Status::ResumeFailed).await;

    let asked = daemon
        .session_service()
        .await
        .resume_session(pb::ResumeSessionRequest {
            idempotency_key: None,
            id: Some(id.clone()),
            mode: pb::resume_session_request::Mode::AfterMove as i32,
        })
        .await;

    if let Ok(answer) = asked {
        let operation = finished(&daemon, &answer.into_inner()).await;
        assert_ne!(operation.state, pb::OperationState::Succeeded as i32, "{operation:?}");
    }
    let after = get_session(&daemon, &id).await;
    assert_eq!(after.status, Status::ResumeFailed as i32, "{after:?}");
    assert!(
        !world.conversation(&session.conversation_id).exists(),
        "a fresh agent was started for a session that never moved"
    );
}
