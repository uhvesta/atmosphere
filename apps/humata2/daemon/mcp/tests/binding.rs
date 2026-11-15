//! Binding the server to its session with the token (MCP.md §1).

mod support;

use humata2_mcp::{ErrorKind, TOKEN_VAR, TokenError};
use serde_json::json;
use support::{Client, FakeDaemon, retryable, structured, tool_error};

/// MCP.md §1
#[tokio::test]
async fn the_server_presents_its_sessions_token_to_the_daemon() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start(&daemon);

    client.call(json!(1), "whoami", json!({})).await;

    let binds = daemon.binds();
    assert!(!binds.is_empty());
    assert!(binds.iter().all(|token| token == "token-1"));
}

/// MCP.md §1
#[tokio::test]
async fn without_a_token_every_call_fails_with_identity_mismatch_and_reaches_no_daemon() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start_with(&daemon, Err(TokenError::Missing { var: TOKEN_VAR }));

    for (n, tool) in ["whoami", "session_info"].into_iter().enumerate() {
        let result = client.call(json!(n), tool, json!({})).await;

        assert_eq!(result["isError"], true);
        let error = &structured(&result)["error"];
        assert_eq!(error["kind"], "identity_mismatch");
        assert!(error.get("retry").is_none());
    }
    assert!(daemon.binds().is_empty());
    assert!(daemon.calls().is_empty());
}

/// MCP.md §1
#[tokio::test]
async fn without_a_token_tools_list_still_works() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start_with(&daemon, Err(TokenError::Empty { var: TOKEN_VAR }));

    assert!(!client.tools().await.is_empty());
}

/// MCP.md §1
#[tokio::test]
async fn a_token_the_daemon_refuses_fails_every_call_and_is_not_tried_again() {
    let daemon = FakeDaemon::new();
    daemon.fail_binds([tool_error(ErrorKind::IdentityMismatch, json!({}))]);
    let mut client = Client::start(&daemon);

    for n in 0..3 {
        let result = client.call(json!(n), "whoami", json!({})).await;

        assert_eq!(structured(&result)["error"]["kind"], "identity_mismatch");
    }
    assert_eq!(daemon.binds().len(), 1);
    assert!(daemon.calls().is_empty());
}

/// MCP.md §1: a daemon that wasn't running yet
#[tokio::test]
async fn a_bind_that_failed_retryably_is_tried_again_so_a_later_call_works() {
    let daemon = FakeDaemon::new();
    daemon.fail_binds([retryable(
        tool_error(ErrorKind::DaemonNotRunning, json!({})),
        None,
    )]);
    let mut client = Client::start(&daemon);

    // The first call may meet the failed bind; the one after it can't.
    client.call(json!(1), "whoami", json!({})).await;
    let result = client.call(json!(2), "whoami", json!({})).await;

    assert_eq!(structured(&result)["outcome"], "applied");
    assert_eq!(daemon.binds().len(), 2);
}
