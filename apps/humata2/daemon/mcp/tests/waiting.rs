//! Waiting for the user (MCP.md §4): pending, progress, waiting again,
//! retries and withdrawal.

mod support;

use std::time::Duration;

use humata2_mcp::{ErrorKind, Reply, RequestId, WaitEvent};
use serde_json::{Value, json};
use support::{Client, FakeDaemon, Rule, structured, tool_error};

fn asking() -> FakeDaemon {
    let daemon = FakeDaemon::new();
    daemon.set_rule("board_item_create", Rule::Ask);
    daemon
}

fn item(wait_seconds: Option<u64>) -> Value {
    let mut arguments = json!({ "board": "global", "title": "Ship it" });
    if let Some(seconds) = wait_seconds {
        arguments["wait_seconds"] = json!(seconds);
    }
    arguments
}

fn applied(result: Value) -> WaitEvent {
    WaitEvent::Done(Ok(Reply::Applied {
        result,
        request: None,
        stale_as_of: None,
    }))
}

fn has_reply_to(messages: &[Value], id: &Value) -> bool {
    messages
        .iter()
        .any(|message| message.get("id") == Some(id) && message.get("method").is_none())
}

/// TP-11
#[tokio::test]
async fn a_call_that_asks_with_no_wait_returns_pending_with_its_request_at_once() {
    let daemon = asking();
    let mut client = Client::start(&daemon);

    let result = client.call(json!(1), "board_item_create", item(Some(0))).await;

    assert_eq!(result["isError"], false);
    assert_eq!(
        *structured(&result),
        json!({ "outcome": "pending", "request_id": "req-1" })
    );
}

/// TP-11
#[tokio::test]
async fn a_call_that_asks_waits_45_seconds_unless_told_otherwise() {
    let daemon = asking();
    let mut client = Client::start(&daemon);

    client
        .start_call(json!(1), "board_item_create", json!({ "arguments": item(None) }))
        .await;
    let first = daemon.next_wait().await;
    client
        .start_call(json!(2), "board_item_create", json!({ "arguments": item(Some(120)) }))
        .await;
    let second = daemon.next_wait().await;

    assert_eq!(first.wait, Duration::from_secs(45));
    assert_eq!(second.wait, Duration::from_secs(120));
}

/// TP-11
#[tokio::test]
async fn each_keepalive_is_a_progress_notification_for_the_clients_token() {
    let daemon = asking();
    let mut client = Client::start(&daemon);

    client
        .start_call(
            json!(1),
            "board_item_create",
            json!({ "arguments": item(None), "_meta": { "progressToken": "tok" } }),
        )
        .await;
    let wait = daemon.next_wait().await;
    for event in [WaitEvent::Keepalive, WaitEvent::Keepalive, applied(json!({ "id": "item-1" }))] {
        wait.events.send(event).await.expect("the server waits");
    }
    let reply = client.reply_to(&json!(1)).await;

    let progress: Vec<(Value, Value)> = client
        .notifications
        .iter()
        .filter(|message| message["method"] == "notifications/progress")
        .map(|message| {
            (
                message["params"]["progressToken"].clone(),
                message["params"]["progress"].clone(),
            )
        })
        .collect();
    assert_eq!(progress, vec![(json!("tok"), json!(1)), (json!("tok"), json!(2))]);
    let envelope = structured(&reply["result"]);
    assert_eq!(envelope["outcome"], "applied");
    assert_eq!(envelope["result"], json!({ "id": "item-1" }));
    assert_eq!(envelope["request_id"], "req-1");
}

/// TP-11
#[tokio::test]
async fn without_a_progress_token_keepalives_send_nothing() {
    let daemon = asking();
    let mut client = Client::start(&daemon);

    client
        .start_call(json!(1), "board_item_create", json!({ "arguments": item(None) }))
        .await;
    let wait = daemon.next_wait().await;
    for event in [WaitEvent::Keepalive, applied(json!({}))] {
        wait.events.send(event).await.expect("the server waits");
    }
    client.reply_to(&json!(1)).await;

    assert!(client.notifications.is_empty());
}

/// TP-11
#[tokio::test]
async fn a_wait_that_passes_returns_pending() {
    let daemon = asking();
    let mut client = Client::start(&daemon);

    client
        .start_call(json!(1), "board_item_create", json!({ "arguments": item(None) }))
        .await;
    let wait = daemon.next_wait().await;
    let request = wait.request.clone();
    wait.events
        .send(WaitEvent::Done(Ok(Reply::Pending { request })))
        .await
        .expect("the server waits");
    let reply = client.reply_to(&json!(1)).await;

    assert_eq!(
        *structured(&reply["result"]),
        json!({ "outcome": "pending", "request_id": "req-1" })
    );
}

/// TP-11
#[tokio::test]
async fn other_calls_are_answered_while_one_waits() {
    let daemon = asking();
    let mut client = Client::start(&daemon);

    client
        .start_call(json!(1), "board_item_create", json!({ "arguments": item(None) }))
        .await;
    let wait = daemon.next_wait().await;
    let other = client.call(json!(2), "whoami", json!({})).await;
    wait.events.send(applied(json!({}))).await.expect("the server waits");
    let first = client.reply_to(&json!(1)).await;

    assert_eq!(structured(&other)["outcome"], "applied");
    assert_eq!(structured(&first["result"])["outcome"], "applied");
}

/// TP-12
#[tokio::test]
async fn inbox_wait_returns_what_the_original_call_would_have() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start(&daemon);

    client
        .start_call(
            json!(1),
            "inbox_wait",
            json!({ "arguments": { "request_id": "req-9", "wait_seconds": 30 } }),
        )
        .await;
    let wait = daemon.next_wait().await;
    wait.events
        .send(applied(json!({ "id": "item-1" })))
        .await
        .expect("the server waits");
    let reply = client.reply_to(&json!(1)).await;

    assert_eq!(wait.request, RequestId::new("req-9"));
    assert_eq!(wait.wait, Duration::from_secs(30));
    assert_eq!(
        *structured(&reply["result"]),
        json!({ "outcome": "applied", "result": { "id": "item-1" }, "request_id": "req-9" })
    );
    assert!(daemon.calls().is_empty());
}

/// TP-12
#[tokio::test]
async fn a_denial_while_waiting_comes_back_as_the_calls_error() {
    let daemon = asking();
    let mut client = Client::start(&daemon);

    client
        .start_call(json!(1), "board_item_create", json!({ "arguments": item(None) }))
        .await;
    let wait = daemon.next_wait().await;
    let denied = tool_error(ErrorKind::Denied, json!({ "source": "user_denied" }));
    wait.events
        .send(WaitEvent::Done(Err(denied)))
        .await
        .expect("the server waits");
    let reply = client.reply_to(&json!(1)).await;

    assert_eq!(reply["result"]["isError"], true);
    let error = &structured(&reply["result"])["error"];
    assert_eq!(error["kind"], "denied");
    assert_eq!(error["details"]["source"], "user_denied");
}

/// TP-14
#[tokio::test]
async fn a_client_retry_of_the_same_request_gets_the_same_inbox_request() {
    let daemon = asking();
    let mut client = Client::start(&daemon);

    let first = client.call(json!(7), "board_item_create", item(Some(0))).await;
    let retry = client.call(json!(7), "board_item_create", item(Some(0))).await;

    assert_eq!(structured(&first)["request_id"], "req-1");
    assert_eq!(structured(&retry)["request_id"], "req-1");
    let calls = daemon.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].idempotency_key, calls[1].idempotency_key);
}

/// TP-14
#[tokio::test]
async fn each_new_request_id_is_a_new_intent() {
    let daemon = asking();
    let mut client = Client::start(&daemon);

    let mut requests = Vec::new();
    for id in [json!(7), json!(8), json!("7")] {
        let result = client.call(id, "board_item_create", item(Some(0))).await;
        requests.push(structured(&result)["request_id"].clone());
    }

    assert_eq!(requests, vec![json!("req-1"), json!("req-2"), json!("req-3")]);
    let keys: Vec<_> = daemon.calls().into_iter().map(|call| call.idempotency_key).collect();
    assert_ne!(keys[0], keys[1]);
    assert_ne!(keys[0], keys[2]);
}

/// TP-14, D29
#[tokio::test]
async fn a_restarted_server_makes_new_keys_for_the_same_request_ids() {
    let daemon = asking();
    let mut first = Client::start(&daemon);
    first.call(json!(1), "board_item_create", item(Some(0))).await;
    first.close().await.0.expect("the server stops cleanly");
    let mut second = Client::start(&daemon);
    second.call(json!(1), "board_item_create", item(Some(0))).await;

    let calls = daemon.calls();
    assert_eq!(calls.len(), 2);
    assert_ne!(calls[0].idempotency_key, calls[1].idempotency_key);
}

/// TP-15
#[tokio::test]
async fn cancelling_a_waiting_call_withdraws_its_request_and_sends_no_reply() {
    let daemon = asking();
    let mut client = Client::start(&daemon);

    client
        .start_call(json!(1), "board_item_create", json!({ "arguments": item(None) }))
        .await;
    let _wait = daemon.next_wait().await;
    client
        .send(json!({
            "jsonrpc": "2.0",
            "method": "notifications/cancelled",
            "params": { "requestId": 1, "reason": "user pressed escape" },
        }))
        .await;
    let (served, rest) = client.close().await;

    assert!(served.is_ok());
    assert_eq!(daemon.withdrawals(), vec![RequestId::new("req-1")]);
    assert!(!has_reply_to(&rest, &json!(1)));
}

/// TP-15
#[tokio::test]
async fn the_agent_exiting_while_a_call_waits_withdraws_its_request() {
    let daemon = asking();
    let mut client = Client::start(&daemon);

    client
        .start_call(json!(1), "board_item_create", json!({ "arguments": item(None) }))
        .await;
    let _wait = daemon.next_wait().await;
    let (served, rest) = client.close().await;

    assert!(served.is_ok());
    assert_eq!(daemon.withdrawals(), vec![RequestId::new("req-1")]);
    assert!(!has_reply_to(&rest, &json!(1)));
}

/// TP-15
#[tokio::test]
async fn a_request_that_returned_pending_survives_the_agent_exiting() {
    let daemon = asking();
    let mut client = Client::start(&daemon);

    client.call(json!(1), "board_item_create", item(Some(0))).await;
    client
        .start_call(json!(2), "board_item_create", json!({ "arguments": item(None) }))
        .await;
    let wait = daemon.next_wait().await;
    let request = wait.request.clone();
    wait.events
        .send(WaitEvent::Done(Ok(Reply::Pending { request })))
        .await
        .expect("the server waits");
    client.reply_to(&json!(2)).await;
    let (served, _) = client.close().await;

    assert!(served.is_ok());
    assert!(daemon.withdrawals().is_empty());
}

/// TP-15
#[tokio::test]
async fn inbox_wait_cancelled_leaves_the_request_answerable() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start(&daemon);

    client
        .start_call(
            json!(1),
            "inbox_wait",
            json!({ "arguments": { "request_id": "req-9" } }),
        )
        .await;
    let _wait = daemon.next_wait().await;
    client
        .send(json!({
            "jsonrpc": "2.0",
            "method": "notifications/cancelled",
            "params": { "requestId": 1 },
        }))
        .await;
    let (served, _) = client.close().await;

    assert!(served.is_ok());
    assert!(daemon.withdrawals().is_empty());
}
