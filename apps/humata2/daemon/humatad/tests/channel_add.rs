//! AddRepo and AddMachine through a real `humatad2` (CH-6, CH-7, CH-18,
//! CH-19): a repository added to a channel gets its main clone and the
//! channel lists it; the same key answers the same operation; a repository
//! the channel has runs as a retry and is listed once; a bad owner and an
//! unknown channel are refused with the right kind; naming this machine
//! again succeeds and lists it once; a machine the host never added is
//! refused by AddMachine and CreateChannel (D164); an agent's AddRepo is refused by
//! default and waits in the Inbox when the channel's rule says Ask.

mod common;

use common::{error_of, finished, machine_id, repo_ref, start_world, wait_for_path};
use humata2_grpc::humata::v1 as pb;
use tonic::Code;

fn id_of(channel: &pb::Channel) -> pb::ChannelId {
    channel.id.clone().unwrap()
}

async fn add_repo(
    daemon: &common::Daemon,
    channel: &pb::ChannelId,
    owner: &str,
    name: &str,
    key: Option<&str>,
) -> Result<pb::Operation, tonic::Status> {
    daemon
        .channel_service()
        .await
        .add_repo(pb::AddRepoRequest {
            idempotency_key: key.map(|key| pb::IdempotencyKey {
                value: key.to_owned(),
            }),
            channel: Some(channel.clone()),
            repo: Some(repo_ref(owner, name)),
        })
        .await
        .map(|response| response.into_inner())
}

async fn get(daemon: &common::Daemon, channel: &pb::ChannelId) -> pb::Channel {
    daemon
        .channel_service()
        .await
        .get_channel(pb::GetChannelRequest {
            id: Some(channel.clone()),
        })
        .await
        .unwrap()
        .into_inner()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_repo_added_to_a_channel_gets_a_main_clone_and_the_first_is_untouched() {
    let (daemon, world) = start_world().await;
    world.remote("acme", "api");
    world.remote("acme", "web");
    let (channel, _) = common::create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    let folder = world.channels.join(&channel.folder);
    let first_head = std::fs::read(folder.join("main/api/.git/HEAD")).unwrap();

    let op = add_repo(&daemon, &id_of(&channel), "acme", "web", None)
        .await
        .unwrap();
    let ended = finished(&daemon, &op).await;

    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    assert!(folder.join("main/web/.git").exists());
    assert_eq!(std::fs::read(folder.join("main/api/.git/HEAD")).unwrap(), first_head);
    let got = get(&daemon, &id_of(&channel)).await;
    assert_eq!(got.repos, vec![repo_ref("acme", "api"), repo_ref("acme", "web")]);
}

#[tokio::test(flavor = "multi_thread")]
async fn repeating_a_call_with_its_key_answers_the_same_operation() {
    let (daemon, world) = start_world().await;
    world.remote("acme", "api");
    world.remote("acme", "web");
    let (channel, _) = common::create_channel(&daemon, "Payments", &[("acme", "api")]).await;

    let first = add_repo(&daemon, &id_of(&channel), "acme", "web", Some("add-1"))
        .await
        .unwrap();
    let again = add_repo(&daemon, &id_of(&channel), "acme", "web", Some("add-1"))
        .await
        .unwrap();

    assert_eq!(first.id, again.id);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_repo_the_channel_has_runs_as_a_retry_and_is_listed_once() {
    let (daemon, world) = start_world().await;
    world.remote("acme", "api");
    let (channel, _) = common::create_channel(&daemon, "Payments", &[("acme", "api")]).await;

    let op = add_repo(&daemon, &id_of(&channel), "acme", "api", None)
        .await
        .unwrap();
    let ended = finished(&daemon, &op).await;

    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    assert_eq!(get(&daemon, &id_of(&channel)).await.repos, vec![repo_ref("acme", "api")]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bad_owner_and_an_unknown_channel_are_refused() {
    let (daemon, world) = start_world().await;
    world.remote("acme", "api");
    let (channel, _) = common::create_channel(&daemon, "Payments", &[("acme", "api")]).await;

    let bad = add_repo(&daemon, &id_of(&channel), "acme corp", "web", None)
        .await
        .unwrap_err();
    let (field, problem) = common::invalid_field(&bad);
    assert_eq!(field, "repo.owner");
    assert_eq!(problem, pb::invalid_field::Problem::BadFormat);

    let unknown = pb::ChannelId {
        value: "ch_nope".to_owned(),
    };
    let missing = add_repo(&daemon, &unknown, "acme", "web", None)
        .await
        .unwrap_err();
    assert_eq!(missing.code(), Code::NotFound);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_machine_the_host_never_added_is_refused_at_once() {
    let (daemon, world) = start_world().await;
    world.remote("acme", "api");
    let (channel, _) = common::create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    let nowhere = pb::MachineId {
        value: "nowhere".to_owned(),
    };

    let added = daemon
        .channel_service()
        .await
        .add_machine(pb::AddChannelMachineRequest {
            idempotency_key: None,
            channel: Some(id_of(&channel)),
            machine: Some(nowhere.clone()),
        })
        .await
        .unwrap_err();
    let created = daemon
        .channel_service()
        .await
        .create_channel(pb::CreateChannelRequest {
            idempotency_key: None,
            name: "Billing".to_owned(),
            repos: vec![repo_ref("acme", "api")],
            machines: vec![machine_id(&daemon).await, nowhere],
        })
        .await
        .unwrap_err();

    assert_eq!(common::not_found(&added), pb::EntityKind::Machine);
    assert_eq!(common::not_found(&created), pb::EntityKind::Machine);
    assert_eq!(get(&daemon, &id_of(&channel)).await.machines, vec![machine_id(&daemon).await]);
    assert!(!world.channels.join("billing").exists(), "nothing is made for a refused channel");
}

#[tokio::test(flavor = "multi_thread")]
async fn naming_this_machine_again_succeeds_and_lists_it_once() {
    let (daemon, world) = start_world().await;
    world.remote("acme", "api");
    let (channel, _) = common::create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    let machine = machine_id(&daemon).await;

    let op = daemon
        .channel_service()
        .await
        .add_machine(pb::AddChannelMachineRequest {
            idempotency_key: None,
            channel: Some(id_of(&channel)),
            machine: Some(machine.clone()),
        })
        .await
        .unwrap()
        .into_inner();
    let ended = finished(&daemon, &op).await;

    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    assert_eq!(get(&daemon, &id_of(&channel)).await.machines, vec![machine]);
}

fn agent_request<T>(message: T, token: &str) -> tonic::Request<T> {
    let mut request = tonic::Request::new(message);
    request.metadata_mut().insert(
        "humata-session-token",
        tonic::metadata::MetadataValue::try_from(token).unwrap(),
    );
    request
}

/// The session token a shell tab of `session` was given.
async fn session_token(daemon: &common::Daemon, session: &pb::SessionId) -> String {
    let tab = daemon
        .terminals()
        .await
        .open_shell_tab(pb::OpenShellTabRequest {
            idempotency_key: None,
            session: Some(session.clone()),
            repo: None,
            folder: String::new(),
        })
        .await
        .unwrap()
        .into_inner()
        .id
        .unwrap();
    let file = daemon.dir.path().join(format!("token-{}", session.value));
    let mut attach = common::Attach::start(daemon, common::open(&tab, 0)).await;
    attach
        .type_text(&format!(
            "printf '%s' \"$HUMATA_SESSION_TOKEN\" > {}\r",
            file.display()
        ))
        .await;
    let deadline = std::time::Instant::now() + common::WAIT;
    loop {
        if let Ok(text) = std::fs::read_to_string(&file)
            && !text.is_empty()
        {
            return text;
        }
        assert!(std::time::Instant::now() < deadline, "the token was never written");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

/// An agent of a new session in a channel of `acme/api`, and its token.
async fn agent_in_channel(
    daemon: &common::Daemon,
    world: &common::World,
) -> (pb::Channel, String) {
    world.remote("acme", "api");
    world.remote("acme", "web");
    let (channel, _) = common::create_channel(daemon, "Payments", &[("acme", "api")]).await;
    let (session, _) = common::start_session(daemon, &channel, "Fix login").await;
    common::wait_status(daemon, &session, pb::SessionStatus::Working).await;
    let token = session_token(daemon, &session).await;
    (channel, token)
}

fn add_web(channel: &pb::Channel) -> pb::AddRepoRequest {
    pb::AddRepoRequest {
        idempotency_key: None,
        channel: channel.id.clone(),
        repo: Some(repo_ref("acme", "web")),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agents_add_repo_is_refused_when_the_repo_add_rule_is_never() {
    let (daemon, world) = start_world().await;
    let (channel, token) = agent_in_channel(&daemon, &world).await;
    pb::access_service_client::AccessServiceClient::new(daemon.channel().await)
        .set_channel_rule(pb::SetChannelRuleRequest {
            channel: channel.id.clone(),
            capability: pb::Capability::ChannelRepoAdd as i32,
            decision: pb::Decision::Never as i32,
            ..Default::default()
        })
        .await
        .unwrap();

    let refused = daemon
        .channel_service()
        .await
        .add_repo(agent_request(add_web(&channel), &token))
        .await
        .unwrap_err();

    assert_eq!(refused.code(), Code::PermissionDenied, "{refused:?}");
    let folder = world.channels.join(&channel.folder);
    assert!(!folder.join("main/web").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agents_add_repo_that_a_rule_sends_to_ask_runs_once_the_user_approves() {
    let (daemon, world) = start_world().await;
    let (channel, token) = agent_in_channel(&daemon, &world).await;
    pb::access_service_client::AccessServiceClient::new(daemon.channel().await)
        .set_channel_rule(pb::SetChannelRuleRequest {
            channel: channel.id.clone(),
            capability: pb::Capability::ChannelRepoAdd as i32,
            decision: pb::Decision::Ask as i32,
            ..Default::default()
        })
        .await
        .unwrap();

    let pending = daemon
        .channel_service()
        .await
        .add_repo(agent_request(add_web(&channel), &token))
        .await
        .unwrap_err();
    let request = match error_of(&pending).kind {
        Some(pb::error::Kind::ApprovalPending(pending)) => pending.request.unwrap(),
        other => panic!("expected approval_pending, got {other:?}"),
    };
    let mut inbox = pb::inbox_service_client::InboxServiceClient::new(daemon.channel().await);
    inbox
        .answer_request(pb::AnswerRequestRequest {
            id: Some(request.clone()),
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
                id: Some(request),
                wait_seconds: 10,
            },
            &token,
        ))
        .await
        .unwrap()
        .into_inner();
    let waited = loop {
        let message = tokio::time::timeout(common::WAIT, stream.message())
            .await
            .expect("the wait ended in time")
            .unwrap()
            .expect("the wait ended with an answer");
        if !message.keepalive {
            break message;
        }
    };

    assert_eq!(waited.result_type, "humata.v1.Operation");
    let op = <pb::Operation as prost::Message>::decode(waited.result.as_slice()).unwrap();
    let ended = finished(&daemon, &op).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    wait_for_path(&world.channels.join(&channel.folder).join("main/web/.git")).await;
}
