//! An agent proposes adding a repo to a channel (CH-26, IN-17, D304) through
//! a real `humatad2`: the call is checked against the target channel's "Add
//! a repo to a channel" rule (Ask me by default), approving it adds the
//! repo, a channel agent can't reach for another channel, a Desk agent may
//! propose for any, an approval that finds the repo already there changes
//! nothing, the other channel changes stay Never, and an agent can't name a
//! local path.

use humatad_test_common as common;

use common::{create_channel, error_of, finished, repo_ref, shell_token, start_session, start_world, wait_status};
use humata2_grpc::humata::v1 as pb;
use tonic::Code;

fn id_of(channel: &pb::Channel) -> pb::ChannelId {
    channel.id.clone().unwrap()
}

fn agent_request<T>(message: T, token: &str) -> tonic::Request<T> {
    let mut request = tonic::Request::new(message);
    request.metadata_mut().insert(
        "humata-session-token",
        tonic::metadata::MetadataValue::try_from(token).unwrap(),
    );
    request
}

fn add_repo_request(channel: &pb::ChannelId, owner: &str, name: &str) -> pb::AddRepoRequest {
    pb::AddRepoRequest {
        idempotency_key: None,
        channel: Some(channel.clone()),
        repo: Some(repo_ref(owner, name)),
    }
}

/// The request an agent's call is waiting on, from the pending error.
fn pending_request(refused: &tonic::Status) -> pb::RequestId {
    match error_of(refused).kind {
        Some(pb::error::Kind::ApprovalPending(pending)) => pending.request.unwrap(),
        other => panic!("expected approval_pending, got {other:?}"),
    }
}

/// Every Inbox request the daemon holds, answered ones too.
async fn inbox(daemon: &common::Daemon) -> Vec<pb::InboxRequest> {
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

/// The user answers `id` with an approval, and the agent waits for what the
/// approved call returned.
async fn approve_and_wait(
    daemon: &common::Daemon,
    token: &str,
    id: &pb::RequestId,
) -> pb::WaitResponse {
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

/// A channel of `acme/api` with a running session, and the token of its
/// agent (kept alive with the attach).
async fn agent_in_channel(
    daemon: &common::Daemon,
    name: &str,
) -> (pb::Channel, String, common::Attach) {
    let (channel, _) = create_channel(daemon, name, &[("acme", "api")]).await;
    let (session, _) = start_session(daemon, &channel, "Fix login").await;
    wait_status(daemon, &session, pb::SessionStatus::Working).await;
    let (token, attach) = shell_token(daemon, &session).await;
    (channel, token, attach)
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agents_add_repo_becomes_an_inbox_request_under_the_repo_add_rule() {
    let (daemon, world) = start_world().await;
    world.remote("acme", "api");
    world.remote("acme", "web");
    let (channel, token, _attach) = agent_in_channel(&daemon, "Payments").await;

    let pending = daemon
        .channel_service()
        .await
        .add_repo(agent_request(add_repo_request(&id_of(&channel), "acme", "web"), &token))
        .await
        .unwrap_err();

    let id = pending_request(&pending);
    let held = inbox(&daemon).await;
    let request = held.iter().find(|request| request.id.as_ref() == Some(&id)).unwrap();
    assert_eq!(request.capability(), pb::Capability::ChannelRepoAdd);
    assert_eq!(held.len(), 1);
    assert_eq!(get(&daemon, &id_of(&channel)).await.repos, vec![repo_ref("acme", "api")]);
}

#[tokio::test(flavor = "multi_thread")]
async fn approving_the_proposal_adds_the_repo() {
    let (daemon, world) = start_world().await;
    world.remote("acme", "api");
    world.remote("acme", "web");
    let (channel, token, _attach) = agent_in_channel(&daemon, "Payments").await;
    let pending = daemon
        .channel_service()
        .await
        .add_repo(agent_request(add_repo_request(&id_of(&channel), "acme", "web"), &token))
        .await
        .unwrap_err();

    let waited = approve_and_wait(&daemon, &token, &pending_request(&pending)).await;

    assert_eq!(waited.result_type, "humata.v1.Operation");
    let op = <pb::Operation as prost::Message>::decode(waited.result.as_slice()).unwrap();
    let ended = finished(&daemon, &op).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    assert_eq!(
        get(&daemon, &id_of(&channel)).await.repos,
        vec![repo_ref("acme", "api"), repo_ref("acme", "web")]
    );
    assert!(world.channels.join(&channel.folder).join("main/web/.git").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_channel_agent_asking_for_another_channel_is_denied_and_adds_nothing_to_the_inbox() {
    let (daemon, world) = start_world().await;
    world.remote("acme", "api");
    world.remote("acme", "web");
    let (_own, token, _attach) = agent_in_channel(&daemon, "Payments").await;
    let (other, _) = create_channel(&daemon, "Billing", &[("acme", "api")]).await;

    let refused = daemon
        .channel_service()
        .await
        .add_repo(agent_request(add_repo_request(&id_of(&other), "acme", "web"), &token))
        .await
        .unwrap_err();

    assert_eq!(refused.code(), Code::PermissionDenied, "{refused:?}");
    assert!(inbox(&daemon).await.is_empty());
    assert_eq!(get(&daemon, &id_of(&other)).await.repos, vec![repo_ref("acme", "api")]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_desk_agents_proposal_for_any_channel_is_asked() {
    let (daemon, world) = start_world().await;
    world.remote("acme", "api");
    world.remote("acme", "web");
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    let started = pb::desk_service_client::DeskServiceClient::new(daemon.channel().await)
        .start_desk_session(pb::StartDeskSessionRequest {
            idempotency_key: None,
            title: "Plan Q4".to_owned(),
            agent: None,
            model: String::new(),
            machine: None,
            picked_path: String::new(),
            repos: Vec::new(),
            access: Some(pb::DeskAccess {
                boards: true,
                channels_and_machines: false,
            }),
            first_message: String::new(),
            channel: None,
            target: None,
            switches: None,
        })
        .await
        .unwrap()
        .into_inner();
    let desk = started.session.unwrap();
    wait_status(&daemon, &desk, pb::SessionStatus::Working).await;
    let (token, _attach) = shell_token(&daemon, &desk).await;

    let pending = daemon
        .channel_service()
        .await
        .add_repo(agent_request(add_repo_request(&id_of(&channel), "acme", "web"), &token))
        .await
        .unwrap_err();

    let id = pending_request(&pending);
    let held = inbox(&daemon).await;
    let request = held.iter().find(|request| request.id.as_ref() == Some(&id)).unwrap();
    assert_eq!(request.capability(), pb::Capability::ChannelRepoAdd);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_approval_after_the_user_added_the_repo_succeeds_without_a_second_clone() {
    let (daemon, world) = start_world().await;
    world.remote("acme", "api");
    world.remote("acme", "web");
    let (channel, token, _attach) = agent_in_channel(&daemon, "Payments").await;
    let pending = daemon
        .channel_service()
        .await
        .add_repo(agent_request(add_repo_request(&id_of(&channel), "acme", "web"), &token))
        .await
        .unwrap_err();
    // The user adds it themselves before answering the card.
    let own = daemon
        .channel_service()
        .await
        .add_repo(add_repo_request(&id_of(&channel), "acme", "web"))
        .await
        .unwrap()
        .into_inner();
    let ended = finished(&daemon, &own).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    let clone = world.channels.join(&channel.folder).join("main/web");
    std::fs::write(clone.join("MARKER"), "not cloned again\n").unwrap();
    let head_path = git_head_path(&clone);
    let head = std::fs::read(&head_path).unwrap();

    let waited = approve_and_wait(&daemon, &token, &pending_request(&pending)).await;

    let op = <pb::Operation as prost::Message>::decode(waited.result.as_slice()).unwrap();
    let ended = finished(&daemon, &op).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    assert!(clone.join("MARKER").exists(), "the clone was made again");
    assert_eq!(std::fs::read(&head_path).unwrap(), head);
    assert_eq!(
        get(&daemon, &id_of(&channel)).await.repos,
        vec![repo_ref("acme", "api"), repo_ref("acme", "web")]
    );
}

fn git_head_path(repo: &std::path::Path) -> std::path::PathBuf {
    let dot_git = repo.join(".git");
    if dot_git.is_file()
        && let Ok(raw) = std::fs::read_to_string(&dot_git)
        && let Some(dir) = raw.lines().find_map(|l| l.trim().strip_prefix("gitdir:"))
    {
        let p = std::path::Path::new(dir.trim());
        return if p.is_absolute() { p.join("HEAD") } else { repo.join(p).join("HEAD") };
    }
    dot_git.join("HEAD")
}

fn step_keys(operation: &pb::Operation) -> Vec<&str> {
    operation.steps.iter().map(|step| step.key.as_str()).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_proposal_for_a_repo_the_channel_has_resolves_as_already_in_the_channel_with_no_card() {
    let (daemon, world) = start_world().await;
    world.remote("acme", "api");
    let (channel, token, _attach) = agent_in_channel(&daemon, "Payments").await;

    let answered = daemon
        .channel_service()
        .await
        .add_repo(agent_request(add_repo_request(&id_of(&channel), "acme", "api"), &token))
        .await
        .unwrap()
        .into_inner();

    let ended = finished(&daemon, &answered).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    assert_eq!(step_keys(&ended), vec!["already_in_channel"]);
    assert!(inbox(&daemon).await.is_empty());
    assert_eq!(get(&daemon, &id_of(&channel)).await.repos, vec![repo_ref("acme", "api")]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_card_answered_after_the_repo_came_in_resolves_as_already_added() {
    let (daemon, world) = start_world().await;
    world.remote("acme", "api");
    world.remote("acme", "web");
    let (channel, token, _attach) = agent_in_channel(&daemon, "Payments").await;
    let pending = daemon
        .channel_service()
        .await
        .add_repo(agent_request(add_repo_request(&id_of(&channel), "acme", "web"), &token))
        .await
        .unwrap_err();
    let own = daemon
        .channel_service()
        .await
        .add_repo(add_repo_request(&id_of(&channel), "acme", "web"))
        .await
        .unwrap()
        .into_inner();
    finished(&daemon, &own).await;

    let waited = approve_and_wait(&daemon, &token, &pending_request(&pending)).await;

    let op = <pb::Operation as prost::Message>::decode(waited.result.as_slice()).unwrap();
    let ended = finished(&daemon, &op).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    assert_eq!(step_keys(&ended), vec!["already_added"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn other_channel_changes_by_an_agent_are_still_never() {
    let (daemon, world) = start_world().await;
    world.remote("acme", "api");
    let (channel, token, _attach) = agent_in_channel(&daemon, "Payments").await;

    let rename = daemon
        .channel_service()
        .await
        .rename_channel(agent_request(
            pb::RenameChannelRequest {
                id: channel.id.clone(),
                name: "Payments v2".to_owned(),
                ..Default::default()
            },
            &token,
        ))
        .await
        .unwrap_err();
    let machine = daemon
        .channel_service()
        .await
        .add_machine(agent_request(
            pb::AddChannelMachineRequest {
                idempotency_key: None,
                channel: channel.id.clone(),
                machine: Some(common::machine_id(&daemon).await),
            },
            &token,
        ))
        .await
        .unwrap_err();

    assert_eq!(rename.code(), Code::PermissionDenied, "{rename:?}");
    assert_eq!(machine.code(), Code::PermissionDenied, "{machine:?}");
    assert!(inbox(&daemon).await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_naming_a_local_path_is_refused_and_asks_nothing() {
    let (daemon, world) = start_world().await;
    world.remote("acme", "api");
    let (channel, token, _attach) = agent_in_channel(&daemon, "Payments").await;

    for (owner, name) in [("/Users/me/code", "api"), ("~", "api"), ("acme", "../api")] {
        let refused = daemon
            .channel_service()
            .await
            .add_repo(agent_request(add_repo_request(&id_of(&channel), owner, name), &token))
            .await
            .unwrap_err();
        assert_eq!(refused.code(), Code::InvalidArgument, "{owner}/{name}: {refused:?}");
    }

    assert!(inbox(&daemon).await.is_empty());
    assert_eq!(get(&daemon, &id_of(&channel)).await.repos, vec![repo_ref("acme", "api")]);
}
