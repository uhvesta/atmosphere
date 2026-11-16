//! OpenCode's permission asks, as the `humata opencode` wrapper hands them
//! to the daemon (D201a): the `data` of a `permission.asked` event becomes a
//! card, and an answer becomes the body OpenCode's reply route reads.

use humata2_agent_hooks::{Agent, Decision, parse_permission, render_permission};
use serde_json::{Value, json};

const SHELL: &[u8] = include_bytes!("../testdata/opencode_asks/permission_shell.json");

fn with(field: &str, value: Value) -> Vec<u8> {
    let mut ask: Value = serde_json::from_slice(SHELL).expect("JSON");
    ask[field] = value;
    serde_json::to_vec(&ask).expect("JSON")
}

#[test]
fn opencode_is_named_as_the_wrapper_names_it() {
    assert_eq!(Agent::parse(Agent::OpenCode.name()), Some(Agent::OpenCode));
}

#[test]
fn a_shell_ask_shows_its_action_and_everything_it_runs() {
    let ask = parse_permission(Agent::OpenCode, SHELL).expect("an ask");
    assert_eq!(ask.tool, "shell");
    assert_eq!(ask.command, "echo hi");
    let two = with("resources", json!(["git status", "git push"]));
    let ask = parse_permission(Agent::OpenCode, &two).expect("an ask");
    assert!(ask.command.contains("git status") && ask.command.contains("git push"));
}

#[test]
fn each_ask_has_its_own_card_and_the_same_ask_keeps_it() {
    let first = parse_permission(Agent::OpenCode, SHELL).expect("an ask");
    let again = parse_permission(Agent::OpenCode, SHELL).expect("an ask");
    assert_eq!(first.key, again.key);
    // The same command asked again later is another ask.
    let later = with("id", json!("per_other"));
    let later = parse_permission(Agent::OpenCode, &later).expect("an ask");
    assert_ne!(first.key, later.key);
}

#[test]
fn an_ask_with_no_id_or_action_raises_no_card() {
    let mut ask: Value = serde_json::from_slice(SHELL).expect("JSON");
    if let Some(fields) = ask.as_object_mut() {
        fields.remove("id");
    }
    let no_id = serde_json::to_vec(&ask).expect("JSON");
    assert!(parse_permission(Agent::OpenCode, &no_id).is_err());
    assert!(parse_permission(Agent::OpenCode, &with("action", json!(7))).is_err());
}

#[test]
fn allow_replies_once_and_deny_rejects_with_a_note() {
    let allow: Value =
        serde_json::from_slice(&render_permission(Agent::OpenCode, &Decision::Allow)).expect("JSON");
    assert_eq!(allow, json!({"decision": "once"}));
    let deny = Decision::Deny { message: "use staging".to_owned() };
    let deny: Value =
        serde_json::from_slice(&render_permission(Agent::OpenCode, &deny)).expect("JSON");
    assert_eq!(deny["decision"], "reject");
    assert_eq!(deny["message"], "use staging");
    // Without a note the agent still hears it was the user.
    let bare = Decision::Deny { message: String::new() };
    let bare: Value =
        serde_json::from_slice(&render_permission(Agent::OpenCode, &bare)).expect("JSON");
    assert!(bare["message"].as_str().is_some_and(|note| !note.is_empty()));
    assert!(render_permission(Agent::OpenCode, &Decision::Undecided).is_empty());
}
