//! AddRepoToSession (CH-21, FC-11) through a real `humatad2`: a repo of the
//! channel is cloned into a running session on the session's branch name and
//! the agent is told nothing (D743); a repo the channel lacks joins the channel first; the
//! repo library hears of it; adding what the session has changes nothing; an
//! agent may add only what its channel has, and is asked first.

use humatad_test_common as common;

use common::{
    Attach, Daemon, World, agent_tab, create_channel, finished, machine_id, open, repo_ref, shell_token,
    start_request, start_world, wait_status,
};
use humata2_grpc::humata::v1 as pb;
use tonic::Code;

fn request(session: &pb::SessionId, owner: &str, name: &str, key: Option<&str>) -> pb::AddRepoToSessionRequest {
    pb::AddRepoToSessionRequest {
        idempotency_key: key.map(|key| pb::IdempotencyKey { value: key.to_owned() }),
        id: Some(session.clone()),
        repo: Some(repo_ref(owner, name)),
    }
}

async fn add(
    daemon: &Daemon,
    session: &pb::SessionId,
    owner: &str,
    name: &str,
) -> Result<pb::Operation, tonic::Status> {
    daemon
        .session_service()
        .await
        .add_repo_to_session(request(session, owner, name, None))
        .await
        .map(tonic::Response::into_inner)
}

/// A running session of a channel of `acme/api` and `acme/web`, working in
/// `api` only; `acme/docs` is on GitHub and in no channel.
async fn running(daemon: &Daemon, world: &World) -> (pb::Channel, pb::SessionId) {
    for name in ["api", "web", "docs"] {
        world.remote("acme", name);
    }
    let (channel, _) = create_channel(daemon, "Payments", &[("acme", "api"), ("acme", "web")]).await;
    let machine = machine_id(daemon).await;
    let mut start = start_request(channel.id.as_ref().unwrap(), &machine, "Fix login");
    start.repos = vec![pb::RepoChoice { repo: Some(repo_ref("acme", "api")), include: true }];
    let response = daemon
        .session_service()
        .await
        .start_session(start)
        .await
        .unwrap()
        .into_inner();
    finished(daemon, &response.operation.unwrap()).await;
    let session = response.session.unwrap();
    wait_status(daemon, &session, pb::SessionStatus::Working).await;
    (channel, session)
}

fn names(session: &pb::Session) -> Vec<String> {
    session
        .repos
        .iter()
        .filter_map(|repo| repo.repo.as_ref())
        .map(|repo| format!("{}/{}", repo.owner, repo.name))
        .collect()
}

/// Everything the test agent was typed.
fn typed(world: &World) -> String {
    std::fs::read_dir(&world.conv)
        .unwrap()
        .filter_map(Result::ok)
        .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_repo_of_the_channel_is_cloned_into_the_session_on_its_branch_and_the_agent_is_told_nothing() {
    let (daemon, world) = start_world().await;
    let (_channel, session) = running(&daemon, &world).await;
    let branch = common::get_session(&daemon, &session).await.repos[0].branch.clone();

    let operation = add(&daemon, &session, "acme", "web").await.unwrap();
    let ended = finished(&daemon, &operation).await;

    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    let after = common::get_session(&daemon, &session).await;
    assert_eq!(names(&after), ["acme/api", "acme/web"]);
    let web = &after.repos[1];
    assert_eq!(web.branch, branch, "the repo takes the session's branch name");
    assert!(std::path::Path::new(&web.path).join(".git").exists());
    // The agent reads its input line by line, so once a line typed after the
    // add is in its conversation, anything typed before it is there too.
    let mut attach = Attach::start(&daemon, open(&agent_tab(&daemon, &session).await, 0)).await;
    attach.type_text("after the add\r").await;
    attach.wait_for("got: after the add").await;
    let deadline = std::time::Instant::now() + common::WAIT;
    while !typed(&world).contains("after the add") {
        assert!(std::time::Instant::now() < deadline, "the agent never kept the typed line");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let typed = typed(&world);
    assert!(!typed.contains("acme/web"), "the agent was told about the repo: {typed:?}");
    assert!(!typed.contains(&web.path), "the agent was told the repo's path: {typed:?}");
    assert!(!typed.contains(&web.branch), "the agent was told the repo's branch: {typed:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_repo_the_channel_lacks_joins_the_channel_and_the_library() {
    let (daemon, world) = start_world().await;
    let (channel, session) = running(&daemon, &world).await;

    let operation = add(&daemon, &session, "acme", "docs").await.unwrap();
    let ended = finished(&daemon, &operation).await;

    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    assert!(names(&common::get_session(&daemon, &session).await).contains(&"acme/docs".to_owned()));
    let got = daemon
        .channel_service()
        .await
        .get_channel(pb::GetChannelRequest { id: channel.id.clone() })
        .await
        .unwrap()
        .into_inner();
    assert!(got.repos.contains(&repo_ref("acme", "docs")));
    let library = humata2_repo_library::Library::open(&daemon.state.join("repos.db")).unwrap();
    let entries = library.list().unwrap().0;
    assert!(entries.iter().any(|entry| entry.repo.to_string() == "acme/docs"));
}

#[tokio::test(flavor = "multi_thread")]
async fn adding_a_repo_the_session_has_changes_nothing() {
    let (daemon, world) = start_world().await;
    let (_channel, session) = running(&daemon, &world).await;
    let before = names(&common::get_session(&daemon, &session).await);

    let operation = add(&daemon, &session, "acme", "api").await.unwrap();
    let ended = finished(&daemon, &operation).await;

    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    assert_eq!(names(&common::get_session(&daemon, &session).await), before);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_repo_that_does_not_exist_fails_the_operation_and_leaves_the_session_as_it_was() {
    let (daemon, world) = start_world().await;
    let (_channel, session) = running(&daemon, &world).await;
    let before = names(&common::get_session(&daemon, &session).await);

    let operation = add(&daemon, &session, "acme", "nowhere").await.unwrap();
    let ended = finished(&daemon, &operation).await;

    assert_eq!(ended.state, pb::OperationState::Failed as i32);
    assert!(ended.error.is_some());
    assert_eq!(names(&common::get_session(&daemon, &session).await), before);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bad_name_and_an_unknown_session_are_refused() {
    let (daemon, world) = start_world().await;
    let (_channel, session) = running(&daemon, &world).await;

    let bad = add(&daemon, &session, "acme corp", "web").await.unwrap_err();
    let unknown = add(&daemon, &pb::SessionId { value: "se_nope".to_owned() }, "acme", "web")
        .await
        .unwrap_err();

    assert_eq!(common::invalid_field(&bad).0, "repo.owner");
    assert_eq!(unknown.code(), Code::NotFound);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_is_asked_first_and_may_not_bring_in_a_repo_its_channel_lacks() {
    let (daemon, world) = start_world().await;
    let (_channel, session) = running(&daemon, &world).await;
    let (token, _attach) = shell_token(&daemon, &session).await;

    let asked = daemon
        .session_service()
        .await
        .add_repo_to_session(as_script_agent(request(&session, "acme", "web", None), &token))
        .await
        .unwrap_err();
    let lacks = daemon
        .session_service()
        .await
        .add_repo_to_session(as_script_agent(request(&session, "acme", "docs", None), &token))
        .await
        .unwrap_err();

    assert!(
        matches!(common::error_of(&asked).kind, Some(pb::error::Kind::ApprovalPending(_))),
        "{asked:?}"
    );
    assert_eq!(lacks.code(), Code::FailedPrecondition, "{lacks:?}");
}

fn as_script_agent<T>(message: T, token: &str) -> tonic::Request<T> {
    let mut request = tonic::Request::new(message);
    request.metadata_mut().insert(
        "humata-session-token",
        tonic::metadata::MetadataValue::try_from(token).unwrap(),
    );
    request
}
