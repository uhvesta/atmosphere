//! Refusals as the agent gets them (MCP.md §3), and the user-only
//! operations that have no tool (MCP.md §7).

mod support;

use std::time::{Duration, UNIX_EPOCH};

use humata2_access::{Capability, Tier};
use serde_json::json;
use support::{Client, FakeDaemon, Rule, retryable, structured, tool_error};

use humata2_mcp::ErrorKind;

/// One name per user-only capability, and the capability's proto name.
const USER_ONLY: &[(Capability, &str, &str)] = &[
    (Capability::InboxAnswer, "inbox_answer", "inbox_answer"),
    (Capability::AccessChange, "access_set_rule", "access_change"),
    (Capability::MarksChange, "marks_set", "marks_change"),
    (Capability::PrSubmit, "review_submit", "pr_submit"),
    (Capability::UserReviewState, "diff_mark_reviewed", "user_review_state"),
    (Capability::TrashEmpty, "trash_empty", "trash_empty"),
    (Capability::HostSecrets, "host_set_variable", "host_secrets"),
    (Capability::ChannelDelete, "channel_delete", "channel_delete"),
    (Capability::MachineSettings, "machine_change_root", "machine_settings"),
    (Capability::TerminalInput, "terminal_attach", "terminal_input"),
];

/// User-only operations that no capability names.
const UNNAMED: &[&str] = &[
    "bridge_close",
    "events_subscribe",
    "audit_list",
    "transfer_start",
    "view_state_save",
];

/// MCP.md §3, D25
#[tokio::test]
async fn a_never_rule_refusal_reaches_the_agent_as_denied_with_its_source_and_capability() {
    let daemon = FakeDaemon::new();
    daemon.set_rule(
        "session_stop",
        Rule::Refuse(tool_error(
            ErrorKind::Denied,
            json!({ "source": "never_rule", "capability": "session_stop_other" }),
        )),
    );
    let mut client = Client::start(&daemon);

    let result = client
        .call(json!(1), "session_stop", json!({ "session": "s-2" }))
        .await;

    assert_eq!(result["isError"], true);
    let error = &structured(&result)["error"];
    assert_eq!(error["kind"], "denied");
    assert_eq!(error["details"]["source"], "never_rule");
    assert_eq!(error["details"]["capability"], "session_stop_other");
    assert!(error["message"].as_str().is_some_and(|text| !text.is_empty()));
    assert!(error["fix"].as_str().is_some_and(|text| !text.is_empty()));
    assert!(error.get("retry").is_none());
}

/// MCP.md §3
#[tokio::test]
async fn a_rate_limit_says_when_to_retry_and_with_the_same_key() {
    let after = UNIX_EPOCH + Duration::from_secs(1_794_657_600);
    let daemon = FakeDaemon::new();
    daemon.set_rule(
        "pr_review_start",
        Rule::Refuse(retryable(
            tool_error(ErrorKind::RateLimited, json!({})),
            Some(after),
        )),
    );
    let mut client = Client::start(&daemon);
    let described = client.humata_describe(&["pr_review_start"]).await;
    let tool = &described["tools"][0];

    let result = client
        .humata_call(json!(1), "pr_review_start", support::minimal(&tool["inputSchema"]))
        .await;

    let error = &structured(&result)["error"];
    assert_eq!(error["kind"], "rate_limited");
    assert_eq!(
        error["retry"],
        json!({ "retryable": true, "after": "2026-11-14T12:00:00Z", "same_key": true })
    );
}

/// MCP.md §7, SEC-7, IN-23
#[tokio::test]
async fn no_user_only_operation_is_in_tools_list() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start(&daemon);

    let tools = client.tools().await;
    let listed = client.humata_list(json!({})).await;
    let entries = listed["tools"].as_array().unwrap();

    let named = USER_ONLY.iter().map(|(_, name, _)| *name).chain(UNNAMED.iter().copied());
    for name in named {
        assert!(
            tools.iter().all(|tool| tool["name"] != name),
            "{name} is offered in tools/list"
        );
        assert!(
            entries
                .iter()
                .all(|line| !line.as_str().unwrap().starts_with(&format!("{name} "))),
            "{name} is offered in humata_list"
        );
    }
}

/// MCP.md §7, SEC-7, IN-23
#[tokio::test]
async fn calling_a_user_only_operation_by_name_is_refused_and_never_reaches_the_daemon() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start(&daemon);

    let named = USER_ONLY.iter().map(|(_, name, _)| *name).chain(UNNAMED.iter().copied());
    for (n, name) in named.enumerate() {
        let direct = client.call(json!(n * 2), name, json!({ "anything": true })).await;
        let wrapped = client
            .humata_call(json!(n * 2 + 1), name, json!({ "anything": true }))
            .await;

        for result in [direct, wrapped] {
            assert_eq!(result["isError"], true, "{name}");
            let error = &structured(&result)["error"];
            assert_eq!(error["kind"], "user_only", "{name}");
            assert!(error.get("retry").is_none(), "{name}");
        }
    }
    assert!(daemon.calls().is_empty());
    assert!(daemon.withdrawals().is_empty());
}

/// MCP.md §7, SEC-7
#[tokio::test]
async fn each_user_only_refusal_names_its_capability() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start(&daemon);

    for (n, (_, name, capability)) in USER_ONLY.iter().enumerate() {
        let result = client.call(json!(n), name, json!({})).await;

        let error = &structured(&result)["error"];
        assert_eq!(error["details"]["capability"], *capability, "{name}");
    }
}

/// SEC-7: the table above covers every user-only capability.
#[test]
fn every_user_only_capability_has_a_refused_name() {
    for capability in Capability::ALL {
        if capability.tier() != Tier::UserOnly {
            continue;
        }
        assert!(
            USER_ONLY.iter().any(|(covered, _, _)| *covered == capability),
            "{capability:?} has no user-only name"
        );
    }
}

/// MCP.md §7, MCP.md §1
#[tokio::test]
async fn a_user_only_operation_is_refused_even_without_a_session_token() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start_with(
        &daemon,
        Err(humata2_mcp::TokenError::Missing {
            var: humata2_mcp::TOKEN_VAR,
        }),
    );

    let result = client.call(json!(1), "access_set_rule", json!({})).await;

    assert_eq!(structured(&result)["error"]["kind"], "user_only");
}

/// SETTINGSFIX Slice 3: settings and agent mutations are no longer hard
/// `user_only` refusals in MCP.
#[tokio::test]
async fn settings_and_agent_mutations_are_not_refused_as_user_only() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start(&daemon);

    for (n, name) in [
        "settings_update",
        "channel_settings_update",
        "agents_add",
        "agents_update",
    ]
    .iter()
    .enumerate()
    {
        let reply = client.call_raw(json!(n + 1), name, json!({})).await;
        let kind = reply
            .get("result")
            .and_then(|r| r.get("structuredContent"))
            .and_then(|s| s.get("error"))
            .and_then(|e| e.get("kind"))
            .and_then(|k| k.as_str());
        assert_ne!(
            kind,
            Some("user_only"),
            "{name} should no longer be refused as user_only, got {reply}"
        );
    }
}
