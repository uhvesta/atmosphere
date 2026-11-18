//! A Desk start whose agent CLI this machine can't find (bug 8, AGENTSET):
//! refused before anything is made, as its own error naming the agent, and
//! never as UNAVAILABLE, which the app reads as the machine not answering.

mod common;

use common::{Build, Daemon, World, error_of, folder};
use humata2_grpc::humata::v1 as pb;
use tonic::Code;

/// A daemon whose only agent runs `program`, which isn't there.
async fn daemon_with_missing_agent(program: &str) -> Daemon {
    let dir = folder();
    let world = World::new(dir.path());
    let agents = serde_json::json!([{
        "name": "copilot-test",
        "program": program,
        "start_args": [],
        "resume_args": [],
        "conversation_file": format!("{}/{{conversation}}.jsonl", world.conv.display()),
    }]);
    std::fs::write(&world.agents, agents.to_string()).unwrap();
    let mut args = world.args();
    let default = args.iter().position(|arg| arg == "--default-agent").unwrap() + 1;
    args[default] = "copilot-test".to_owned();
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    Daemon::start_with(Build::Admin, dir, &refs).await
}

fn request(title: &str) -> pb::StartDeskSessionRequest {
    pb::StartDeskSessionRequest {
        title: title.to_owned(),
        access: Some(pb::DeskAccess {
            boards: true,
            channels_and_machines: false,
        }),
        ..pb::StartDeskSessionRequest::default()
    }
}

async fn start(daemon: &Daemon, title: &str) -> Result<pb::StartDeskSessionResponse, tonic::Status> {
    pb::desk_service_client::DeskServiceClient::new(daemon.channel().await)
        .start_desk_session(request(title))
        .await
        .map(tonic::Response::into_inner)
}

#[tokio::test]
async fn a_desk_start_whose_agent_is_not_on_path_is_refused_as_missing_not_offline() {
    let daemon = daemon_with_missing_agent("humata2-test-agent-that-is-not-installed").await;
    let status = start(&daemon, "Missing").await.expect_err("an agent that isn't there can't start");
    assert_eq!(status.code(), Code::FailedPrecondition, "{status:?}");
    let error = error_of(&status);
    assert!(
        error.message.contains("humata2-test-agent-that-is-not-installed"),
        "the error names the program: {error:?}"
    );
    assert!(!error.fix.is_empty(), "the error says how to fix it: {error:?}");
}

#[tokio::test]
async fn a_desk_start_whose_agent_path_is_gone_is_refused_as_missing_not_offline() {
    let daemon = daemon_with_missing_agent("/nonexistent/humata2/copilot").await;
    let status = start(&daemon, "Gone").await.expect_err("an agent that isn't there can't start");
    assert_eq!(status.code(), Code::FailedPrecondition, "{status:?}");
    assert!(error_of(&status).message.contains("/nonexistent/humata2/copilot"), "{status:?}");
}
