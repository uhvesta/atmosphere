//! DV-9: the agent of a Desk session lists and reads the versions of a file
//! through the MCP tools `doc_versions_list` and `doc_version_read`. Its own
//! session's folder needs no rule; another Desk session's is refused, since
//! Desk sessions are not channel-mates (D182.3). There is no tool that saves
//! or restores.

use humatad_test_common as common;
use humatad_test_common::mcp_client;

use std::path::PathBuf;

use common::{Build, Daemon, World, folder, git, shell_token, wait_status};
use humata2_grpc::humata::v1 as pb;
use mcp_client::Mcp;
use serde_json::json;

async fn daemon() -> Daemon {
    let dir = folder();
    let world = World::new(dir.path());
    let args = world.args();
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    Daemon::start_with(Build::Admin, dir, &refs).await
}

/// A plain Desk session, up and running; returns it and its folder.
async fn desk_session(daemon: &Daemon, title: &str) -> (pb::SessionId, PathBuf) {
    let started = pb::desk_service_client::DeskServiceClient::new(daemon.channel().await)
        .start_desk_session(pb::StartDeskSessionRequest {
            title: title.to_owned(),
            ..Default::default()
        })
        .await
        .expect("start a desk session")
        .into_inner();
    let id = started.session.expect("the session id");
    wait_status(daemon, &id, pb::SessionStatus::Working).await;
    let path = started.desk.and_then(|desk| desk.folder).expect("its folder").path;
    (id, PathBuf::from(path))
}

/// What the agent does to save a version: a plain `git commit`.
fn agent_commit(folder: &std::path::Path, file: &str, text: &str, message: &str) {
    std::fs::write(folder.join(file), text).expect("write the file");
    let dir = folder.display().to_string();
    git(&["-C", &dir, "add", "-A"]);
    git(&["-C", &dir, "-c", "core.hooksPath=/dev/null", "commit", "-q", "-m", message]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_desk_agent_lists_and_reads_the_versions_of_its_own_folders_file() {
    let daemon = daemon().await;
    let (id, folder) = desk_session(&daemon, "Mine").await;
    agent_commit(&folder, "rfc.md", "one\n", "First");
    agent_commit(&folder, "rfc.md", "one\ntwo\n", "Second");
    let (token, _shell) = shell_token(&daemon, &id).await;
    let mut agent = Mcp::start(&daemon.socket, &token).await;

    let listed = agent.call(1, "doc_versions_list", json!({ "path": "rfc.md" })).await;
    let versions = listed["structuredContent"]["result"]["versions"]
        .as_array()
        .unwrap_or_else(|| panic!("a list of versions: {listed}"))
        .clone();
    let oldest = versions.last().expect("the oldest version")["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    let read = agent
        .call(2, "doc_version_read", json!({ "path": "rfc.md", "version": oldest }))
        .await;

    assert_eq!(listed["isError"], false, "{listed}");
    assert_eq!(versions.len(), 2, "{listed}");
    assert_eq!(versions[0]["number"], 2, "newest first: {listed}");
    assert_eq!(read["structuredContent"]["result"]["text"], "one\n", "{read}");
}

#[tokio::test(flavor = "multi_thread")]
async fn another_desk_sessions_versions_are_refused() {
    let daemon = daemon().await;
    let (mine, _) = desk_session(&daemon, "Mine").await;
    let (other, other_folder) = desk_session(&daemon, "Other").await;
    agent_commit(&other_folder, "plan.md", "plan\n", "Plan");
    let (token, _shell) = shell_token(&daemon, &mine).await;
    let mut agent = Mcp::start(&daemon.socket, &token).await;

    // Two Desk sessions are not channel-mates (D182.3), so the file-read
    // rules DV-9 reads under refuse it outright; nothing is asked.
    let refused = agent
        .call(1, "doc_versions_list", json!({ "path": "plan.md", "location": { "session": other.value } }))
        .await;

    assert_eq!(refused["isError"], true, "{refused}");
    assert_eq!(refused["structuredContent"]["error"]["kind"], "denied", "{refused}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_version_is_an_error_result() {
    let daemon = daemon().await;
    let (id, folder) = desk_session(&daemon, "Mine").await;
    agent_commit(&folder, "rfc.md", "one\n", "First");
    let (token, _shell) = shell_token(&daemon, &id).await;
    let mut agent = Mcp::start(&daemon.socket, &token).await;

    let read = agent
        .call(
            1,
            "doc_version_read",
            json!({ "path": "rfc.md", "version": "0000000000000000000000000000000000000000" }),
        )
        .await;

    assert_eq!(read["isError"], true, "{read}");
}
