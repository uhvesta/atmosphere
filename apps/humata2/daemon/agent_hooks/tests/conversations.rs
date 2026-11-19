//! A Copilot, Claude Code or Codex hook names the conversation its CLI is
//! in, so a `/clear`, `/restart` or `/new` that starts another one in the
//! same running agent can be followed (D799); and the plugin runs that hook
//! when a conversation starts, at each prompt and at the end of each turn.

use std::path::Path;

use humata2_agent_hooks::{Agent, PayloadError, copilot_plugin, parse_conversation, parse_transcript};
use serde_json::Value;

const SESSION_START: &[u8] = include_bytes!("../testdata/copilot/session_start_new.json");
const AGENT_STOP: &[u8] = include_bytes!("../testdata/copilot/agent_stop.json");
const SUBAGENT_PROMPT: &[u8] = include_bytes!("../testdata/copilot/user_prompt_subagent.json");
const CLAUDE_CLEARED: &[u8] = include_bytes!("../testdata/claude/session_start_clear.json");
const CLAUDE_STOP: &[u8] = include_bytes!("../testdata/claude/stop.json");
const CLAUDE_SUBAGENT: &[u8] = include_bytes!("../testdata/claude/stop_subagent.json");
const CODEX_THREAD: &[u8] = include_bytes!("../testdata/codex/thread_started.json");
const CODEX_STOP: &[u8] = include_bytes!("../testdata/codex/stop.json");

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
fn opencodes_hooks_name_no_conversation() {
    for payload in [SESSION_START, CLAUDE_CLEARED, CODEX_THREAD] {
        assert!(parse_conversation(Agent::OpenCode, payload).is_err());
    }
}

#[test]
fn one_agents_payload_names_no_conversation_for_another() {
    // Copilot's `sessionId` isn't Claude's or Codex's `session_id`.
    for agent in [Agent::Claude, Agent::Codex] {
        assert!(parse_conversation(agent, SESSION_START).is_err(), "{agent:?}");
    }
    assert!(parse_conversation(Agent::Copilot, CLAUDE_CLEARED).is_err());
}

#[test]
fn a_claude_session_start_after_clear_names_the_new_conversation() {
    let id = parse_conversation(Agent::Claude, CLAUDE_CLEARED).expect("a conversation");
    assert_eq!(id, "7c1e2a90-5b3d-4f6e-9a21-0d8c4b7e3f15");
}

#[test]
fn the_end_of_a_claude_turn_names_its_conversation() {
    let id = parse_conversation(Agent::Claude, CLAUDE_STOP).expect("a conversation");
    assert_eq!(id, "abc123");
}

#[test]
fn a_claude_subagents_hook_names_no_conversation_of_the_session() {
    assert!(matches!(
        parse_conversation(Agent::Claude, CLAUDE_SUBAGENT),
        Err(PayloadError::NotAConversation { .. })
    ));
}

#[test]
fn a_codex_thread_names_its_conversation() {
    let id = parse_conversation(Agent::Codex, CODEX_THREAD).expect("a conversation");
    assert_eq!(id, "019b2c4d-7e8f-7a01-b234-56789abcdef0");
    let id = parse_conversation(Agent::Codex, CODEX_STOP).expect("a conversation");
    assert_eq!(id, "019a-thread");
}

#[test]
fn a_claude_or_codex_id_that_could_name_a_path_is_refused() {
    for agent in [Agent::Claude, Agent::Codex] {
        for id in ["../other", "a/b", "", "with space"] {
            let payload = serde_json::json!({ "session_id": id }).to_string();
            assert!(
                parse_conversation(agent, payload.as_bytes()).is_err(),
                "{agent:?} took {id:?}"
            );
        }
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

/// Claude names its project folder from the session folder's real path, so
/// the file its hook names is the one to follow, not one built from the
/// folder as Humata was given it.
#[test]
fn a_claude_hook_names_its_transcript_file_and_a_subagents_names_none() {
    assert_eq!(
        parse_transcript(Agent::Claude, CLAUDE_CLEARED).as_deref(),
        Some(Path::new(
            "/home/u/.claude/projects/-work-api/7c1e2a90-5b3d-4f6e-9a21-0d8c4b7e3f15.jsonl"
        ))
    );
    assert_eq!(parse_transcript(Agent::Claude, CLAUDE_SUBAGENT), None);
    assert_eq!(parse_transcript(Agent::Codex, CODEX_THREAD), None);
    assert_eq!(parse_transcript(Agent::Claude, br#"{"session_id":"a","transcript_path":"rel/a.jsonl"}"#), None);
    assert_eq!(parse_transcript(Agent::Claude, b"not json"), None);
}
