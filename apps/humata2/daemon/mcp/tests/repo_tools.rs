//! The repo tools (RL-11, D479): `repo_search` and `repo_library` read,
//! and nothing an agent can call saves the scope or removes a repo.

mod support;

use serde_json::json;
use support::{Client, FakeDaemon, structured};

#[tokio::test]
async fn both_repo_tools_are_listed_as_reads() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start(&daemon);

    let described = client.humata_describe(&["repo_search", "repo_library"]).await;
    let tools = described["tools"].as_array().expect("tools");

    for name in ["repo_search", "repo_library"] {
        let tool = tools
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap_or_else(|| panic!("{name} isn't described"));
        assert_eq!(tool["annotations"]["readOnlyHint"], true, "{name}");
        assert_eq!(tool["annotations"]["destructiveHint"], false, "{name}");
    }
}

#[tokio::test]
async fn a_search_can_widen_one_call_but_has_no_way_to_save_a_scope() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start(&daemon);

    let described = client.humata_describe(&["repo_search"]).await;
    let search = &described["tools"][0];

    let input = &search["inputSchema"];
    assert_eq!(input["required"], json!(["query"]));
    let properties = input["properties"].as_object().expect("properties");
    assert!(properties.contains_key("owners"));
    assert!(properties.contains_key("all_of_github"));
    for saving in ["save", "save_as_default", "default", "scope"] {
        assert!(!properties.contains_key(saving), "{saving}");
    }
}

#[tokio::test]
async fn removing_forgetting_and_saving_the_scope_are_refused_and_never_reach_the_daemon() {
    let daemon = FakeDaemon::new();
    let mut client = Client::start(&daemon);
    let listed = client.humata_list(json!({})).await;
    let entries = listed["tools"].as_array().unwrap();

    for (n, name) in ["repo_remove", "repo_forget", "repo_scope_update"]
        .into_iter()
        .enumerate()
    {
        assert!(
            entries
                .iter()
                .all(|line| !line.as_str().unwrap().starts_with(&format!("{name} "))),
            "{name}"
        );
        let result = client
            .humata_call(json!(n), name, json!({ "repo": "acme/api" }))
            .await;

        assert_eq!(result["isError"], true, "{name}");
        assert_eq!(structured(&result)["error"]["kind"], "user_only", "{name}");
    }
    assert!(daemon.calls().is_empty());
}
