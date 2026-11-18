//! A Copilot hook names the conversation its CLI is in, so a `/clear` or
//! `/restart` that starts another one in the same running agent can be
//! followed; and the plugin runs that hook when a conversation starts, at
//! each prompt and at the end of each turn.

use std::path::Path;

use humata2_agent_hooks::{Agent, PayloadError, copilot_plugin, parse_conversation};
use serde_json::Value;

const SESSION_START: &[u8] = include_bytes!("../testdata/copilot/session_start_new.json");
const AGENT_STOP: &[u8] = include_bytes!("../testdata/copilot/agent_stop.json");
const SUBAGENT_PROMPT: &[u8] = include_bytes!("../testdata/copilot/user_prompt_subagent.json");

#[test]
fn a_session_start_names_the_new_conversation() {
    let id = parse_conversation(Agent::Copilot, SESSION_START).expect("a conversation");
    assert_eq!(id, "f78588d5-b8ac-40f0-8448-6af7d59c8a9c");
}

#[test]
fn the_end_of_a_turn_names_its_conversation() {
    let id = parse_conversation(Agent::Copilot, AGENT_STOP).expect("a conversation");
    assert_eq!(id, "c0p1-session");
}

#[test]
fn a_subagents_hook_names_no_conversation_of_the_session() {
    assert!(matches!(
        parse_conversation(Agent::Copilot, SUBAGENT_PROMPT),
        Err(PayloadError::NotAConversation { .. })
    ));
}

#[test]
fn an_id_that_could_name_a_path_is_refused() {
    for id in ["../other", "a/b", "", "with space"] {
        let payload = serde_json::json!({ "sessionId": id }).to_string();
        assert!(
            parse_conversation(Agent::Copilot, payload.as_bytes()).is_err(),
            "{id:?} was taken"
        );
    }
}

#[test]
fn a_payload_with_no_id_or_not_json_names_none() {
    assert!(parse_conversation(Agent::Copilot, br#"{"timestamp":1}"#).is_err());
    assert!(matches!(
        parse_conversation(Agent::Copilot, b"not json"),
        Err(PayloadError::NotJson(_))
    ));
}

#[test]
fn other_agents_hooks_name_no_conversation() {
    for agent in [Agent::Claude, Agent::Codex, Agent::OpenCode] {
        assert!(parse_conversation(agent, SESSION_START).is_err(), "{agent:?}");
    }
}

#[test]
fn the_plugin_reports_the_conversation_when_it_starts_at_each_prompt_and_turn_end() {
    let files = copilot_plugin(Path::new("/opt/humata/bin/humata"), Path::new("/run/h.sock"));
    let hooks: Value = serde_json::from_str(
        &files.iter().find(|file| file.path == "hooks.json").expect("hooks.json").contents,
    )
    .expect("hooks.json is JSON");
    for event in ["sessionStart", "userPromptSubmitted", "agentStop"] {
        let reports = hooks["hooks"][event]
            .as_array()
            .expect("JSON array")
            .iter()
            .filter(|hook| {
                hook["bash"].as_str().is_some_and(|bash| bash.contains("hook copilot conversation"))
            })
            .count();
        assert_eq!(reports, 1, "{event}");
    }
}
