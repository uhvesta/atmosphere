//! The tools in `tools/list` and the checks every call meets before it
//! reaches the daemon.

mod support;

use humata2_mcp::Operation;
use serde_json::{Map, Value, json};
use support::{Client, FakeDaemon, minimal};

/// MCPCOMPRESS: `tools/list` exposes only the 3 progressive-disclosure
/// meta-tools within the wire and prompt byte budgets.
#[tokio::test]
async fn tools_list_exposes_three_meta_tools_within_byte_budget() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start(&daemon);

    let reply = client.request(json!("list"), "tools/list", json!({})).await;
    let result = &reply["result"];
    let tools = result["tools"].as_array().expect("tools array");
    let names: Vec<&str> = tools
        .iter()
        .map(|tool| tool["name"].as_str().expect("a name"))
        .collect();
    assert_eq!(names, ["humata_call", "humata_describe", "humata_list"]);

    let wire_bytes = serde_json::to_string(result).unwrap().len();
    assert!(
        wire_bytes <= 5_000,
        "tools/list wire size {wire_bytes} B exceeds 5,000 B budget"
    );

    let mut without_output = result.clone();
    for tool in without_output["tools"].as_array_mut().unwrap() {
        tool.as_object_mut().unwrap().remove("outputSchema");
    }
    let prompt_bytes = serde_json::to_string(&without_output).unwrap().len();
    assert!(
        prompt_bytes <= 2_500,
        "tools/list without outputSchema {prompt_bytes} B exceeds 2,500 B budget"
    );
}

/// MCP.md §6, MCPCOMPRESS: `humata_list` offers every tool of MCP.md as a
/// compact 1-line `<name> [<category>,<effect>](...): <summary>` entry.
#[tokio::test]
async fn tools_list_offers_exactly_the_tools_of_mcp_md() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start(&daemon);

    let listed = client.humata_list(json!({})).await;
    let entries = listed["tools"].as_array().expect("tools");
    let mut names: Vec<String> = entries
        .iter()
        .map(|entry| {
            let line = entry.as_str().expect("1-line string");
            assert!(!line.contains('\n'), "must be 1-line: {line}");
            assert!(line.contains(" [") && line.contains("](") && line.contains("): "), "{line}");
            line.split_whitespace().next().expect("name").to_owned()
        })
        .collect();
    names.sort();

    let expected: Vec<&str> = include_str!("tools.txt").lines().collect();
    assert_eq!(names, expected);
    assert_eq!(listed["count"], expected.len());
}

/// MCPCOMPRESS: `humata_list` filters by category and case-insensitive query.
#[tokio::test]
async fn humata_list_filters_by_category_and_query() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start(&daemon);

    let board = client.humata_list(json!({ "category": "board" })).await;
    let board_tools = board["tools"].as_array().unwrap();
    assert!(!board_tools.is_empty());
    assert!(
        board_tools
            .iter()
            .all(|line| line.as_str().unwrap().contains("[board,")),
        "{board}"
    );

    let filtered = client
        .humata_list(json!({ "category": "board", "query": "ITEM_MOVE" }))
        .await;
    let names: Vec<&str> = filtered["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|line| line.as_str().unwrap().split_whitespace().next().unwrap())
        .collect();
    assert_eq!(names, ["board_item_move"]);
}

/// MCPCOMPRESS: `humata_describe` enforces `minItems: 1`, `maxItems: 20` and
/// returns structured `unavailable` entries for user-only and unknown names.
#[tokio::test]
async fn humata_describe_validates_bounds_and_reports_unavailable() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start(&daemon);

    let empty = client
        .call_raw(json!(1), "humata_describe", json!({ "names": [] }))
        .await;
    assert_eq!(empty["error"]["code"], -32602, "{empty}");

    let too_many: Vec<String> = (0..21).map(|i| format!("t{i}")).collect();
    let over = client
        .call_raw(json!(2), "humata_describe", json!({ "names": too_many }))
        .await;
    assert_eq!(over["error"]["code"], -32602, "{over}");

    let mixed = client
        .humata_describe(&["whoami", "access_set_rule", "no_such_tool"])
        .await;
    assert_eq!(mixed["tools"].as_array().unwrap().len(), 1);
    assert_eq!(mixed["tools"][0]["name"], "whoami");
    let unavailable = mixed["unavailable"].as_array().unwrap();
    assert_eq!(unavailable.len(), 2, "{mixed}");
    assert_eq!(unavailable[0]["name"], "access_set_rule");
    assert_eq!(unavailable[0]["reason"], "user_only");
    assert_eq!(unavailable[0]["capability"], "access_change");
    assert_eq!(unavailable[1]["name"], "no_such_tool");
    assert_eq!(unavailable[1]["reason"], "unknown");
}

/// MCPCOMPRESS: every example from `humata_describe` across `Operation::ALL`
/// validates against `input_schema(op)`.
#[tokio::test]
async fn every_example_from_humata_describe_validates_against_input_schema() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start(&daemon);

    let described = client.all_described_tools().await;
    assert_eq!(described.len(), Operation::ALL.len());
    let mut req_id = 1000_u64;
    for tool in described {
        let name = tool["name"].as_str().expect("name");
        let examples = tool["examples"].as_array().expect("examples");
        assert!(!examples.is_empty(), "{name} must have at least one example");
        if name == "inbox_wait" {
            // inbox_wait starts a wait; just check its example shape directly.
            assert!(examples[0]["request_id"].is_string(), "{name}: {examples:?}");
            continue;
        }
        for example in examples {
            req_id += 1;
            let reply = client
                .humata_call_raw(json!(req_id), name, example.clone())
                .await;
            assert!(
                reply["result"].is_object(),
                "{name} example failed validation: example={example}, reply={reply}"
            );
        }
    }
}

/// MCPCOMPRESS: `humata_call` validation failure returns exact violation AND
/// compact inline schema in both `error.message` and `error.data.schema` for
/// 1-turn self-correction.
#[tokio::test]
async fn validation_error_returns_inline_schema_for_one_turn_self_correction() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start(&daemon);

    let bad = client
        .humata_call_raw(json!(1), "board_item_create", json!({ "board": "session" }))
        .await;
    assert_eq!(bad["error"]["code"], -32602, "{bad}");
    assert_eq!(bad["error"]["data"]["field"], "/title");
    assert_eq!(bad["error"]["data"]["problem"], "missing");
    let schema_text = bad["error"]["data"]["schema"]
        .as_str()
        .expect("error.data.schema");
    assert!(schema_text.contains("title*:"), "{schema_text}");
    assert!(
        bad["error"]["message"]
            .as_str()
            .unwrap()
            .contains(schema_text),
        "{bad}"
    );

    // 1-turn self-correction using the inline schema succeeds.
    let fixed = client
        .humata_call(
            json!(2),
            "board_item_create",
            json!({ "board": "session", "title": "Fixed in one turn" }),
        )
        .await;
    assert_eq!(fixed["structuredContent"]["outcome"], "applied", "{fixed}");
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

    let mut all = client.tools().await;
    all.extend(client.all_described_tools().await);
    for tool in all {
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

    for tool in client.all_described_tools().await {
        let name = &tool["name"];
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

    for tool in client.all_described_tools().await {
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

    for (n, tool) in client.all_described_tools().await.into_iter().enumerate() {
        let name = tool["name"].as_str().expect("a name");
        if name == "inbox_wait" {
            // It waits on the Inbox; tests/waiting.rs covers it.
            continue;
        }
        let reply = client
            .humata_call_raw(json!(n), name, minimal(&tool["inputSchema"]))
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
    let described = client.humata_describe(&["file_write"]).await;
    let file_write = &described["tools"][0];
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

/// MCP.md §6: D382
#[tokio::test]
async fn a_session_folder_location_is_accepted_and_reaches_the_daemon_as_given() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start(&daemon);
    let location = json!({ "root": "session_folder", "session": "se_desk", "path": "rfc.md" });

    let reply = client
        .call_raw(json!(1), "file_read", json!({ "location": location }))
        .await;

    assert!(reply["result"].is_object(), "{reply}");
    let calls = daemon.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].operation, Operation::FileRead);
    assert_eq!(calls[0].arguments["location"], location);
}

/// MCP.md §6: D382, MCPCOMPRESS `OneOf` closest-shape regression: a session
/// folder location with `repo` matches the `session_folder` discriminator and
/// reports `/location/repo` as `not_allowed_here` (not `/location/root`).
#[tokio::test]
async fn a_session_folder_location_takes_no_repo() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start(&daemon);

    let reply = client
        .humata_call_raw(
            json!(1),
            "file_read",
            json!({ "location": {
                "root": "session_folder",
                "repo": "o/n",
                "path": "rfc.md",
            } }),
        )
        .await;

    assert_eq!(reply["error"]["code"], -32602, "{reply}");
    assert_eq!(reply["error"]["data"]["field"], "/location/repo", "{reply}");
    assert_eq!(reply["error"]["data"]["problem"], "not_allowed_here", "{reply}");
    assert!(daemon.calls().is_empty());
}

/// MCP.md §3, §6: browser tools are strictly read-only (`browser_list_tabs`,
/// `browser_read_tab`, `browser_screenshot`); execute/mutate tools are unavailable.
#[tokio::test]
async fn browser_tools_are_strictly_read_only_and_reject_execute_tools() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start(&daemon);
    let described = client
        .humata_describe(&[
            "browser_list_tabs",
            "browser_read_tab",
            "browser_screenshot",
            "browser_click",
            "browser_type",
            "browser_navigate",
            "browser_guide_highlight",
        ])
        .await;
    let tools = described["tools"].as_array().expect("described tools");
    let described_names: Vec<&str> = tools
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert_eq!(
        described_names,
        vec!["browser_list_tabs", "browser_read_tab", "browser_screenshot"]
    );

    let unavailable = described["unavailable"]
        .as_array()
        .expect("unavailable array");
    let unavailable_names: Vec<&str> = unavailable
        .iter()
        .filter_map(|u| u["name"].as_str())
        .collect();
    for removed in [
        "browser_click",
        "browser_type",
        "browser_navigate",
        "browser_guide_highlight",
    ] {
        assert!(
            unavailable_names.contains(&removed),
            "{removed} must be unavailable: {described}"
        );
    }

    let reply = client.call_raw(json!(2), "browser_list_tabs", json!({})).await;
    assert!(reply["result"].is_object(), "{reply}");
    let reply = client.call_raw(json!(3), "browser_screenshot", json!({})).await;
    assert!(reply["result"].is_object(), "{reply}");
    let calls = daemon.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].operation, Operation::BrowserListTabs);
    assert!(calls[0].arguments.is_empty());
    assert_eq!(calls[1].operation, Operation::BrowserScreenshot);
    assert!(calls[1].arguments.is_empty());
}
