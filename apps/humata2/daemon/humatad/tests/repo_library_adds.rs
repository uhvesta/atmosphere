//! The repo library on the host (RL-1, RL-2) through a real `humatad2`: a
//! repo whose add succeeded is recorded in `repos.db` with where it was used;
//! a clone that fails records nothing. The library file is opened here
//! beside the daemon, which is fine for a WAL database.

mod common;

use common::{
    Daemon, create_channel, error_of, finished, repo_ref, shell_token, start_session, start_world, wait_status,
};
use humata2_grpc::humata::v1 as pb;
use humata2_repo_library::{Entry, Library, Place};

/// What the library holds now.
fn library(daemon: &Daemon) -> Vec<Entry> {
    let library = Library::open(&daemon.state.join("repos.db")).expect("the repo library opens");
    library.list().expect("the repo library lists").0
}

/// Where `repo` was last used, when the library has it.
fn used_in(daemon: &Daemon, repo: &str) -> Option<Place> {
    library(daemon)
        .into_iter()
        .find(|entry| entry.repo.to_string() == repo)
        .map(|entry| entry.last_used_in)
}

async fn add_repo(daemon: &Daemon, channel: &pb::ChannelId, name: &str) -> pb::Operation {
    daemon
        .channel_service()
        .await
        .add_repo(pb::AddRepoRequest {
            idempotency_key: None,
            channel: Some(channel.clone()),
            repo: Some(repo_ref("acme", name)),
        })
        .await
        .unwrap()
        .into_inner()
}

fn channel_place(channel: &pb::Channel) -> Place {
    Place::Channel(channel.id.clone().unwrap().value)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_channel_created_with_a_repo_leaves_the_repo_in_the_library() {
    let (daemon, world) = start_world().await;
    world.remote("acme", "api");

    let (channel, setup) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;

    assert_eq!(setup.state, pb::OperationState::Succeeded as i32, "{:?}", setup.error);
    assert_eq!(used_in(&daemon, "acme/api"), Some(channel_place(&channel)));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_clone_that_fails_leaves_the_library_empty() {
    let (daemon, _world) = start_world().await;

    let (_channel, setup) = create_channel(&daemon, "Payments", &[("acme", "gone")]).await;

    assert_eq!(setup.state, pb::OperationState::Failed as i32);
    assert!(library(&daemon).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_repo_added_to_a_channel_is_recorded_and_a_repo_that_fails_is_not() {
    let (daemon, world) = start_world().await;
    world.remote("acme", "api");
    world.remote("acme", "web");
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    let id = channel.id.clone().unwrap();

    let web = finished(&daemon, &add_repo(&daemon, &id, "web").await).await;
    let gone = finished(&daemon, &add_repo(&daemon, &id, "gone").await).await;

    assert_eq!(web.state, pb::OperationState::Succeeded as i32, "{:?}", web.error);
    assert_eq!(gone.state, pb::OperationState::Failed as i32);
    assert_eq!(used_in(&daemon, "acme/web"), Some(channel_place(&channel)));
    assert_eq!(used_in(&daemon, "acme/gone"), None);
}

fn agent_request<T>(message: T, token: &str) -> tonic::Request<T> {
    let mut request = tonic::Request::new(message);
    request.metadata_mut().insert(
        "humata-session-token",
        tonic::metadata::MetadataValue::try_from(token).unwrap(),
    );
    request
}

#[tokio::test(flavor = "multi_thread")]
async fn an_approved_agent_proposal_is_recorded_once_the_user_says_yes() {
    let (daemon, world) = start_world().await;
    world.remote("acme", "api");
    world.remote("acme", "web");
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    let (session, _) = start_session(&daemon, &channel, "Fix login").await;
    wait_status(&daemon, &session, pb::SessionStatus::Working).await;
    let (token, _attach) = shell_token(&daemon, &session).await;
    let pending = daemon
        .channel_service()
        .await
        .add_repo(agent_request(
            pb::AddRepoRequest {
                idempotency_key: None,
                channel: channel.id.clone(),
                repo: Some(repo_ref("acme", "web")),
            },
            &token,
        ))
        .await
        .unwrap_err();
    let request = match error_of(&pending).kind {
        Some(pb::error::Kind::ApprovalPending(pending)) => pending.request.unwrap(),
        other => panic!("expected approval_pending, got {other:?}"),
    };
    assert_eq!(used_in(&daemon, "acme/web"), None, "asking is not adding");

    let mut inbox = daemon.inbox_service().await;
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
    let result = loop {
        let message = tokio::time::timeout(common::WAIT, stream.message())
            .await
            .expect("the wait ended in time")
            .unwrap()
            .expect("the wait ended with an answer");
        if !message.keepalive {
            break message;
        }
    };
    let op = <pb::Operation as prost::Message>::decode(result.result.as_slice()).unwrap();
    let ended = finished(&daemon, &op).await;

    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    assert_eq!(used_in(&daemon, "acme/web"), Some(channel_place(&channel)));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_repo_added_to_a_desk_session_is_recorded_as_used_on_the_desk() {
    let (daemon, world) = start_world().await;
    world.remote("acme", "api");
    let mut desk = pb::desk_service_client::DeskServiceClient::new(daemon.channel().await);
    let started = desk
        .start_desk_session(pb::StartDeskSessionRequest {
            idempotency_key: None,
            title: "Scratch".to_owned(),
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
    let session = started.session.unwrap();
    wait_status(&daemon, &session, pb::SessionStatus::Working).await;

    desk.add_desk_repo(pb::AddDeskRepoRequest {
        session: Some(session.clone()),
        repo: Some(repo_ref("acme", "api")),
    })
    .await
    .unwrap();
    let failed = desk
        .add_desk_repo(pb::AddDeskRepoRequest {
            session: Some(session),
            repo: Some(repo_ref("acme", "gone")),
        })
        .await;

    assert!(failed.is_err());
    assert_eq!(used_in(&daemon, "acme/api"), Some(Place::Desk));
    assert_eq!(used_in(&daemon, "acme/gone"), None);
}
