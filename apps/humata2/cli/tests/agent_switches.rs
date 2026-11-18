//! `humata agents edit --asks-in-inbox`: the per-kind switch for where an
//! agent's own questions go (D782), set alone or with other fields.

mod support;

use humata2_cli::{exit, rpc};
use humata2_proto::humata::v1 as pb;
use support::{FakeConsole, FakeDaemon, run};

fn listed(daemon: &FakeDaemon) {
    daemon.reply(
        rpc::agents::LIST_AGENTS,
        &pb::ListAgentsResponse {
            agents: vec![pb::AgentDefinition {
                id: Some(pb::AgentId { value: "ag_1".into() }),
                name: "claude".into(),
                ..Default::default()
            }],
        },
    );
}

#[tokio::test]
async fn asks_in_inbox_false_switches_that_agent_alone() {
    let daemon = FakeDaemon::new();
    listed(&daemon);
    daemon.reply(rpc::agents::SET_AGENT_ENABLED, &pb::AgentDefinition::default());
    let console = FakeConsole::pipe();
    let code = run(&daemon, &console, &["--json", "agents", "edit", "claude", "--asks-in-inbox", "false"]).await;
    assert_eq!(code, exit::OK, "{}", console.stderr());
    let sent: pb::SetAgentEnabledRequest = daemon.only(rpc::agents::SET_AGENT_ENABLED);
    assert_eq!(
        (sent.agent.map(|agent| agent.value), sent.asks_in_inbox, sent.enabled, sent.install_when_missing),
        (Some("ag_1".to_owned()), Some(false), None, None)
    );
    assert!(!daemon.called(rpc::agents::UPDATE_AGENT));
}

#[tokio::test]
async fn asks_in_inbox_true_with_another_field_updates_and_switches() {
    let daemon = FakeDaemon::new();
    listed(&daemon);
    daemon.reply(rpc::agents::UPDATE_AGENT, &pb::AgentDefinition::default());
    daemon.reply(rpc::agents::SET_AGENT_ENABLED, &pb::AgentDefinition::default());
    let console = FakeConsole::pipe();
    let args = ["--json", "agents", "edit", "claude", "--launch", "x", "--asks-in-inbox", "true"];
    let code = run(&daemon, &console, &args).await;
    assert_eq!(code, exit::OK, "{}", console.stderr());
    assert!(daemon.called(rpc::agents::UPDATE_AGENT));
    let sent: pb::SetAgentEnabledRequest = daemon.only(rpc::agents::SET_AGENT_ENABLED);
    assert_eq!(sent.asks_in_inbox, Some(true));
}

#[tokio::test]
async fn a_value_that_is_not_true_or_false_is_a_usage_error() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let code = run(&daemon, &console, &["agents", "edit", "claude", "--asks-in-inbox", "maybe"]).await;
    assert_eq!(code, exit::USAGE);
    assert!(!daemon.called(rpc::agents::SET_AGENT_ENABLED));
}
