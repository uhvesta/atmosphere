//! JSON-RPC and the MCP handshake, as an agent's client sees them.

mod support;

use serde_json::{Value, json};
use support::{Client, FakeDaemon};

/// MCP.md §1
#[tokio::test]
async fn initialize_names_the_server_humata_and_offers_tools() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start(&daemon);

    let reply = client
        .request(
            json!(1),
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "test", "version": "1" },
            }),
        )
        .await;

    let result = &reply["result"];
    assert_eq!(result["serverInfo"]["name"], "humata");
    assert_eq!(result["protocolVersion"], "2025-06-18");
    assert!(result["capabilities"]["tools"].is_object());
}

/// MCP.md §1
#[tokio::test]
async fn initialize_with_an_unknown_version_answers_with_the_newest_one_it_speaks() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start(&daemon);

    let reply = client
        .request(json!(1), "initialize", json!({ "protocolVersion": "1999-01-01" }))
        .await;

    assert_eq!(reply["result"]["protocolVersion"], "2025-11-25");
}

/// MCP.md §1
#[tokio::test]
async fn ping_answers_with_an_empty_result() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start(&daemon);

    let reply = client.request(json!("p"), "ping", json!({})).await;

    assert_eq!(reply["result"], json!({}));
}

/// MCP.md §1
#[tokio::test]
async fn a_line_that_isnt_json_is_a_parse_error() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start(&daemon);

    client.send_line("{not json").await;
    let reply = client.receive().await.expect("a reply");

    assert_eq!(reply["error"]["code"], -32700);
    assert_eq!(reply["id"], Value::Null);
}

/// MCP.md §1
#[tokio::test]
async fn a_batch_is_an_invalid_request() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start(&daemon);

    client
        .send(json!([{ "jsonrpc": "2.0", "id": 1, "method": "ping" }]))
        .await;
    let reply = client.receive().await.expect("a reply");

    assert_eq!(reply["error"]["code"], -32600);
}

/// MCP.md §1
#[tokio::test]
async fn a_message_without_jsonrpc_2_0_is_an_invalid_request() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start(&daemon);

    client.send(json!({ "id": 4, "method": "ping" })).await;
    let reply = client.reply_to(&json!(4)).await;

    assert_eq!(reply["error"]["code"], -32600);
}

/// MCP.md §1
#[tokio::test]
async fn a_method_the_server_doesnt_offer_is_method_not_found() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start(&daemon);

    let reply = client.request(json!(2), "resources/list", json!({})).await;

    assert_eq!(reply["error"]["code"], -32601);
}

/// MCP.md §1
#[tokio::test]
async fn notifications_get_no_reply_and_blank_lines_are_skipped() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start(&daemon);

    client
        .send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
        .await;
    client.send_line("").await;
    let reply = client.request(json!(3), "ping", json!({})).await;

    assert_eq!(reply["result"], json!({}));
    assert!(client.notifications.is_empty());
}

/// MCP.md §1
#[tokio::test]
async fn the_server_stops_cleanly_when_the_agent_closes_its_side() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start(&daemon);
    client.request(json!(1), "ping", json!({})).await;

    let (served, rest) = client.close().await;

    assert!(served.is_ok());
    assert!(rest.is_empty());
}

/// MCP.md §3
#[tokio::test]
async fn a_tool_result_carries_the_same_json_as_text_and_as_structured_content() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start(&daemon);

    let result = client.call(json!(1), "whoami", json!({})).await;

    let text = result["content"][0]["text"].as_str().expect("text content");
    let parsed: Value = serde_json::from_str(text).expect("the text is JSON");
    assert_eq!(parsed, result["structuredContent"]);
    assert_eq!(result["isError"], false);
}
