//! The server passes on what the daemon decides and never widens it: for
//! every tool, Allow is applied, Ask is pending, and a refusal comes back
//! as the agent's error.

mod support;

use humata2_mcp::{ErrorKind, JournalPosition, Reply, WaitEvent};
use serde_json::{Value, json};
use support::{Client, FakeDaemon, Rule, minimal, structured, tool_error};

/// Each tool with the smallest arguments it takes and, for tools that take
/// `wait_seconds`, a wait of 0 so an Ask returns at once.
async fn every_tool(client: &mut Client) -> Vec<(String, Value, bool)> {
    client
        .tools()
        .await
        .into_iter()
        .filter(|tool| tool["name"] != "inbox_wait")
        .map(|tool| {
            let name = tool["name"].as_str().expect("a name").to_owned();
            let mut arguments = minimal(&tool["inputSchema"]);
            let takes_wait = !tool["inputSchema"]["properties"]["wait_seconds"].is_null();
            if takes_wait {
                arguments["wait_seconds"] = json!(0);
            }
            (name, arguments, takes_wait)
        })
        .collect()
}

/// BOUNDARIES.md §3, MCP.md §6.1
#[tokio::test]
async fn every_tool_the_daemon_allows_is_applied_with_the_daemons_result() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start(&daemon);

    for (n, (name, arguments, _)) in every_tool(&mut client).await.into_iter().enumerate() {
        let result = client.call(json!(n), &name, arguments).await;

        let envelope = structured(&result);
        assert_eq!(result["isError"], false, "{name}");
        assert_eq!(envelope["outcome"], "applied", "{name}");
        assert_eq!(envelope["result"], json!({ "tool": name }), "{name}");
    }
}

/// BOUNDARIES.md §3, MCP.md §6.2, TP-11
#[tokio::test]
async fn every_tool_the_daemon_sends_to_the_inbox_is_pending_never_applied() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start(&daemon);
    let tools = every_tool(&mut client).await;
    for (name, _, _) in &tools {
        daemon.set_rule(name, Rule::Ask);
    }

    for (n, (name, arguments, takes_wait)) in tools.into_iter().enumerate() {
        let id = json!(n);
        client
            .start_call(id.clone(), &name, json!({ "arguments": arguments }))
            .await;
        if !takes_wait {
            // A tool without wait_seconds waits 45 seconds; end the wait
            // the way the daemon does when it passes.
            let wait = daemon.next_wait().await;
            let request = wait.request.clone();
            wait.events
                .send(WaitEvent::Done(Ok(Reply::Pending { request })))
                .await
                .expect("the server waits");
        }
        let reply = client.reply_to(&id).await;

        let envelope = structured(&reply["result"]);
        assert_eq!(envelope["outcome"], "pending", "{name}");
        assert!(envelope["request_id"].is_string(), "{name}");
        assert!(envelope.get("result").is_none(), "{name}");
    }
}

/// BOUNDARIES.md §3, MCP.md §3
#[tokio::test]
async fn every_tool_the_daemon_refuses_returns_the_refusal_as_an_error() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start(&daemon);
    let tools = every_tool(&mut client).await;
    for (name, _, _) in &tools {
        daemon.set_rule(
            name,
            Rule::Refuse(tool_error(
                ErrorKind::Denied,
                json!({ "source": "never_rule" }),
            )),
        );
    }

    for (n, (name, arguments, _)) in tools.into_iter().enumerate() {
        let result = client.call(json!(n), &name, arguments).await;

        assert_eq!(result["isError"], true, "{name}");
        let error = &structured(&result)["error"];
        assert_eq!(error["kind"], "denied", "{name}");
        assert_eq!(error["details"]["source"], "never_rule", "{name}");
        assert!(structured(&result).get("outcome").is_none(), "{name}");
    }
}

/// AW-7, GRPC.md §9
#[tokio::test]
async fn a_write_journaled_while_the_host_is_away_says_where_it_was_journaled() {
    let daemon = FakeDaemon::new();
    daemon.set_rule(
        "board_item_create",
        Rule::Reply(Reply::Journaled {
            result: json!({ "id": "item-1" }),
            journal: JournalPosition {
                machine: "box".to_owned(),
                seq: 7,
            },
        }),
    );
    let mut client = Client::start(&daemon);

    let result = client
        .call(json!(1), "board_item_create", json!({ "board": "session", "title": "t" }))
        .await;

    let envelope = structured(&result);
    assert_eq!(envelope["outcome"], "journaled");
    assert_eq!(envelope["journal"], json!({ "machine": "box", "seq": 7 }));
    assert_eq!(envelope["result"], json!({ "id": "item-1" }));
}

/// AW-8
#[tokio::test]
async fn a_read_from_the_last_copy_says_how_old_it_is() {
    let daemon = FakeDaemon::new();
    daemon.set_rule(
        "board_items_list",
        Rule::Reply(Reply::Applied {
            result: json!({ "items": [] }),
            request: None,
            stale_as_of: Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_794_657_600)),
        }),
    );
    let mut client = Client::start(&daemon);

    let result = client
        .call(json!(1), "board_items_list", json!({ "board": "channel" }))
        .await;

    let envelope = structured(&result);
    assert_eq!(envelope["outcome"], "applied");
    assert_eq!(envelope["stale_as_of"], "2026-11-14T12:00:00Z");
}
