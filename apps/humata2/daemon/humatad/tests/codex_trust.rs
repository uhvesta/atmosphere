//! Codex's trust step (D202) through a real `humatad2`: while an agent kind
//! carries Codex's hooks and none of them has been heard, Settings › Agents
//! says they need trusting. A turn report from such a session clears it,
//! and a kind without them never shows it.

mod common;

use std::time::{Duration, Instant};

use common::{
    Build, Daemon, WAIT, World, binary, create_channel, folder, shell_token, start_session,
};
use humata2_grpc::humata::v1 as pb;

/// Starts a daemon whose one kind carries `{codex_hooks}` when `codex`,
/// with a session of it running.
async fn daemon(codex: bool) -> (Daemon, pb::SessionId) {
    let dir = folder();
    let world = World::new(dir.path());
    let script = dir.path().join("agent.sh");
    let body = format!(
        "case \"$1\" in --session-id) : > '{}/'\"$2\".jsonl ;; esac\necho up\nexec cat > /dev/null\n",
        world.conv.display()
    );
    std::fs::write(&script, body).expect("write a test file");
    let hooks = if codex {
        "hooks={codex_hooks}"
    } else {
        "plain"
    };
    let agents = serde_json::json!([{
        "name": common::AGENT_NAME,
        "program": "/bin/sh",
        "start_args": [script, "--session-id", "{conversation}", hooks],
        "resume_args": [script, "--resume", "{conversation}", hooks],
        "conversation_file": format!("{}/{{conversation}}.jsonl", world.conv.display()),
        "turn_end": "hooks",
    }]);
    std::fs::write(&world.agents, agents.to_string()).expect("write a test file");
    world.remote("acme", "api");
    let mut args = world.args();
    args.extend([
        "--humata".to_owned(),
        binary("HUMATA").display().to_string(),
    ]);
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let daemon = Daemon::start_with(Build::Admin, dir, &refs).await;
    let (channel, setup) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    assert_eq!(
        setup.state,
        pb::OperationState::Succeeded as i32,
        "{:?}",
        setup.error
    );
    let (session, operation) = start_session(&daemon, &channel, "Fix login").await;
    assert_eq!(
        operation.state,
        pb::OperationState::Succeeded as i32,
        "{:?}",
        operation.error
    );
    (daemon, session)
}

async fn needs_trust(daemon: &Daemon) -> bool {
    pb::settings_service_client::SettingsServiceClient::new(daemon.channel().await)
        .get_settings(pb::GetSettingsRequest {})
        .await
        .expect("read the settings")
        .into_inner()
        .agents
        .is_some_and(|agents| agents.codex_hooks_need_trust)
}

/// The session's agent reports a turn, as Codex's turn hook does.
async fn report_turn(daemon: &Daemon, token: &str) {
    let mut request = tonic::Request::new(pb::ReportTurnRequest {
        state: pb::TurnState::Working as i32,
        reported_at_unix_ns: 0,
    });
    request.metadata_mut().insert(
        "humata-session-token",
        tonic::metadata::MetadataValue::try_from(token).expect("token is valid metadata"),
    );
    pb::session_service_client::SessionServiceClient::new(daemon.channel().await)
        .report_turn(request)
        .await
        .expect("the agent reports its turn");
}

#[tokio::test(flavor = "multi_thread")]
async fn codexs_hooks_need_trust_until_one_of_them_is_heard() {
    let (daemon, session) = daemon(true).await;
    assert!(
        needs_trust(&daemon).await,
        "no hook heard yet, so the step shows"
    );
    let (token, _shell) = shell_token(&daemon, &session).await;
    report_turn(&daemon, &token).await;
    let deadline = Instant::now() + WAIT;
    while needs_trust(&daemon).await {
        assert!(
            Instant::now() < deadline,
            "a heard hook should clear the step"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_kind_without_codexs_hooks_never_shows_the_step() {
    let (daemon, _session) = daemon(false).await;
    assert!(!needs_trust(&daemon).await);
}
