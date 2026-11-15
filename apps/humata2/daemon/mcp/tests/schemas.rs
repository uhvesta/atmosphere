//! The tools in `tools/list` and the checks every call meets before it
//! reaches the daemon.

mod support;

use humata2_mcp::Operation;
use serde_json::{Map, Value, json};
use support::{Client, FakeDaemon, minimal};

/// MCP.md §6
#[tokio::test]
async fn tools_list_offers_exactly_the_tools_of_mcp_md() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start(&daemon);

    let mut names: Vec<String> = client
        .tools()
        .await
        .iter()
        .map(|tool| tool["name"].as_str().expect("a name").to_owned())
        .collect();
    names.sort();

    let expected: Vec<&str> = include_str!("tools.txt").lines().collect();
    assert_eq!(names, expected);
}

/// TP-16
#[test]
fn every_tool_is_named_area_verb_and_its_operation_id_is_area_dot_verb() {
    for operation in Operation::ALL {
        let name = operation.tool_name();
        let well_formed = !name.is_empty()
            && name.chars().all(|c| c.is_ascii_lowercase() || c == '_')
            && !name.starts_with('_')
            && !name.ends_with('_')
            && !name.contains("__");
        assert!(well_formed, "{name} is lowercase words joined by _");
        let id = operation.id();
        let (area, verb) = id.split_once('.').expect("the id is area.verb");
        assert!(!area.is_empty() && !verb.is_empty() && !verb.contains('.'), "{id}");
    }
}

/// MCP.md §1, §3
#[tokio::test]
async fn every_tool_takes_an_object_that_allows_no_other_fields_and_no_caller() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start(&daemon);

    for tool in client.tools().await {
        let name = &tool["name"];
        let input = &tool["inputSchema"];
        assert_eq!(input["type"], "object", "{name}");
        assert_eq!(input["additionalProperties"], false, "{name}");
        let properties = input["properties"].as_object().expect("properties");
        for caller in ["requester", "caller", "principal", "caller_session", "caller_channel"] {
            assert!(!properties.contains_key(caller), "{name} takes {caller}");
        }
    }
}

/// MCP.md §3
#[tokio::test]
async fn every_tool_has_an_object_output_schema_and_annotations() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start(&daemon);

    for tool in client.tools().await {
        let name = &tool["name"];
        assert_eq!(tool["outputSchema"]["type"], "object", "{name}");
        assert!(tool["description"].as_str().is_some_and(|text| !text.is_empty()), "{name}");
        let annotations = &tool["annotations"];
        for hint in ["readOnlyHint", "destructiveHint", "idempotentHint", "openWorldHint"] {
            assert!(annotations[hint].is_boolean(), "{name} {hint}");
        }
        let read_only = annotations["readOnlyHint"] == true;
        let destructive = annotations["destructiveHint"] == true;
        assert!(!(read_only && destructive), "{name} is read-only and destructive");
    }
}

/// MCP.md §4: TP-11
#[tokio::test]
async fn every_tool_a_rule_can_send_to_the_inbox_takes_wait_seconds_up_to_600() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start(&daemon);

    for tool in client.tools().await {
        let wait = &tool["inputSchema"]["properties"]["wait_seconds"];
        if wait.is_null() {
            continue;
        }
        let name = &tool["name"];
        assert_eq!(wait["minimum"], 0, "{name}");
        assert_eq!(wait["maximum"], 600, "{name}");
        assert_eq!(wait["default"], 45, "{name}");
    }
}

/// MCP.md §3
#[tokio::test]
async fn the_smallest_arguments_each_schema_describes_are_accepted() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start(&daemon);

    for (n, tool) in client.tools().await.into_iter().enumerate() {
        let name = tool["name"].as_str().expect("a name");
        if name == "inbox_wait" {
            // It waits on the Inbox; tests/waiting.rs covers it.
            continue;
        }
        let reply = client
            .call_raw(json!(n), name, minimal(&tool["inputSchema"]))
            .await;
        assert!(reply["result"].is_object(), "{name}: {reply}");
    }
}

/// MCP.md §3
#[tokio::test]
async fn a_title_longer_than_200_characters_is_refused_before_the_daemon_sees_it() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start(&daemon);

    let reply = client
        .call_raw(
            json!(1),
            "board_item_create",
            json!({ "board": "session", "title": "x".repeat(201) }),
        )
        .await;

    assert_eq!(reply["error"]["code"], -32602);
    assert_eq!(reply["error"]["data"]["field"], "/title");
    assert_eq!(reply["error"]["data"]["problem"], "too_long");
    assert!(daemon.calls().is_empty());
}

/// MCP.md §3
#[tokio::test]
async fn a_title_of_exactly_200_characters_is_accepted() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start(&daemon);

    let result = client
        .call(
            json!(1),
            "board_item_create",
            json!({ "board": "session", "title": "x".repeat(200) }),
        )
        .await;

    assert_eq!(result["structuredContent"]["outcome"], "applied");
}

/// MCP.md §4: TP-11
#[tokio::test]
async fn a_wait_over_600_seconds_is_out_of_range() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start(&daemon);

    let reply = client
        .call_raw(
            json!(1),
            "board_item_create",
            json!({ "board": "session", "title": "t", "wait_seconds": 601 }),
        )
        .await;

    assert_eq!(reply["error"]["code"], -32602);
    assert_eq!(reply["error"]["data"]["field"], "/wait_seconds");
    assert_eq!(reply["error"]["data"]["problem"], "out_of_range");
    assert!(daemon.calls().is_empty());
}

/// MCP.md §1
#[tokio::test]
async fn a_requester_argument_is_refused_as_not_a_field() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start(&daemon);

    let reply = client
        .call_raw(json!(1), "whoami", json!({ "requester": "another-session" }))
        .await;

    assert_eq!(reply["error"]["code"], -32602);
    assert_eq!(reply["error"]["data"]["field"], "/requester");
    assert_eq!(reply["error"]["data"]["problem"], "not_allowed_here");
    assert!(daemon.calls().is_empty());
}

/// MCP.md §3
#[tokio::test]
async fn a_missing_required_field_is_named() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start(&daemon);

    let reply = client
        .call_raw(json!(1), "board_item_create", json!({ "board": "session" }))
        .await;

    assert_eq!(reply["error"]["code"], -32602);
    assert_eq!(reply["error"]["data"]["field"], "/title");
    assert_eq!(reply["error"]["data"]["problem"], "missing");
}

/// MCP.md §3
#[tokio::test]
async fn arguments_that_arent_an_object_are_invalid_params() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start(&daemon);

    let reply = client.call_raw(json!(1), "whoami", json!(["x"])).await;

    assert_eq!(reply["error"]["code"], -32602);
    assert!(daemon.calls().is_empty());
}

/// MCP.md §6
#[tokio::test]
async fn a_tool_that_doesnt_exist_is_invalid_params() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start(&daemon);

    let reply = client.call_raw(json!(1), "board_items_frobnicate", json!({})).await;

    assert_eq!(reply["error"]["code"], -32602);
    assert!(daemon.calls().is_empty());
}

/// MCP.md §4: TP-11
#[tokio::test]
async fn the_daemon_gets_the_tool_and_its_arguments_without_wait_seconds() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start(&daemon);

    client
        .call(
            json!(1),
            "board_item_create",
            json!({ "board": "channel", "title": "Fix it", "wait_seconds": 5 }),
        )
        .await;

    let calls = daemon.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].operation, Operation::BoardItemCreate);
    let mut expected = Map::new();
    expected.insert("board".to_owned(), json!("channel"));
    expected.insert("title".to_owned(), json!("Fix it"));
    assert_eq!(calls[0].arguments, expected);
}

/// MCP.md §3
#[tokio::test]
async fn one_field_of_a_choice_is_required_and_two_are_refused() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start(&daemon);
    let tools = client.tools().await;
    let file_write = tools
        .iter()
        .find(|tool| tool["name"] == "file_write")
        .expect("file_write is a tool");
    let mut arguments = minimal(&file_write["inputSchema"]);
    let Value::Object(fields) = &mut arguments else {
        panic!("file_write takes an object");
    };
    fields.insert("text".to_owned(), json!("a"));
    fields.insert("base64".to_owned(), json!("YQ=="));

    let reply = client.call_raw(json!(1), "file_write", arguments).await;

    assert_eq!(reply["error"]["code"], -32602);
    assert!(daemon.calls().is_empty());
}
