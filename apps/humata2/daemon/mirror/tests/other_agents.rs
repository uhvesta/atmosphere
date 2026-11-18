//! Reading Codex rollouts and Copilot session-state events as entries.

mod support;

use humata2_mirror::{Entry, OwnTools, Role, read_from};
use serde_json::json;
use support::*;

fn entry(role: Role, text: &str, at: &str, new_turn: bool) -> Entry {
    Entry {
        role,
        text: text.to_owned(),
        at: Some(at.to_owned()),
        new_turn,
    }
}

fn codex_message(at: &str, kind: &str, block: &str, text: &str) -> String {
    json!({"timestamp": at, "ordinal": 1, "type": "event_msg", "payload": {
        "type": "item_completed", "thread_id": "t", "turn_id": "u",
        "item": {"type": kind, "id": "i", "content": [{"type": block, "text": text}]}
    }})
    .to_string()
}

fn codex_response(at: &str, role: &str, text: &str) -> String {
    json!({"timestamp": at, "ordinal": 2, "type": "response_item", "payload": {
        "type": "message", "role": role,
        "content": [{"type": "input_text", "text": text}]
    }})
    .to_string()
}

fn codex_payload(at: &str, payload: serde_json::Value) -> String {
    json!({"timestamp": at, "ordinal": 3, "type": "response_item", "payload": payload}).to_string()
}

fn copilot(at: &str, kind: &str, data: serde_json::Value) -> String {
    json!({"type": kind, "data": data, "id": "x", "parentId": null, "timestamp": at}).to_string()
}

#[test]
fn a_codex_rollout_gives_prompts_and_replies_without_the_injected_messages() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("rollout.jsonl");
    write_lines(
        &file,
        &[
            json!({"timestamp": "2026-11-16T21:48:57.265Z", "type": "session_meta", "payload": {"id": "t"}}).to_string(),
            json!({"timestamp": "2026-11-16T21:48:57.269Z", "type": "event_msg", "payload": {"type": "task_started", "turn_id": "u"}}).to_string(),
            codex_response("2026-11-16T21:48:57.376Z", "user", "# AGENTS.md instructions for /w\n\n<INSTRUCTIONS>\nRun tests.\n</INSTRUCTIONS>"),
            codex_response("2026-11-16T21:48:57.410Z", "user", "Fix the login redirect"),
            codex_message("2026-11-16T21:48:57.415Z", "UserMessage", "text", "Fix the login redirect"),
            codex_message("2026-11-16T21:48:57.459Z", "AgentMessage", "Text", "The redirect now goes back."),
            codex_response("2026-11-16T21:48:57.461Z", "assistant", "The redirect now goes back."),
            json!({"timestamp": "2026-11-16T21:48:57.463Z", "type": "event_msg", "payload": {"type": "token_count"}}).to_string(),
            codex_message("2026-11-16T21:49:00.000Z", "UserMessage", "text", "Now add a test"),
            codex_message("2026-11-16T21:49:01.000Z", "AgentMessage", "Text", "Added it."),
        ],
    );

    let read = read_from(&file, 0, &mut OwnTools::default()).unwrap();

    assert_eq!(
        read.entries,
        vec![
            entry(Role::User, "Fix the login redirect", "2026-11-16T21:48:57.415Z", true),
            entry(Role::Agent, "The redirect now goes back.", "2026-11-16T21:48:57.459Z", false),
            entry(Role::User, "Now add a test", "2026-11-16T21:49:00.000Z", true),
            entry(Role::Agent, "Added it.", "2026-11-16T21:49:01.000Z", false),
        ]
    );
}

#[test]
fn codex_tool_calls_and_outputs_are_tool_entries_and_humata_calls_are_left_out() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("rollout.jsonl");
    write_lines(
        &file,
        &[
            codex_payload("2026-11-16T10:00:00Z", json!({
                "type": "function_call", "name": "shell", "call_id": "c1",
                "arguments": "{\"command\": [\"bash\", \"-lc\", \"ls\"]}"
            })),
            codex_payload("2026-11-16T10:00:01Z", json!({
                "type": "function_call_output", "call_id": "c1", "output": "a.txt"
            })),
            codex_payload("2026-11-16T10:00:02Z", json!({
                "type": "function_call", "name": "shell", "call_id": "c2",
                "arguments": "{\"command\": [\"bash\", \"-lc\", \"humata session list\"]}"
            })),
            codex_payload("2026-11-16T10:00:03Z", json!({
                "type": "function_call_output", "call_id": "c2", "output": "secret list"
            })),
            codex_payload("2026-11-16T10:00:04Z", json!({
                "type": "function_call", "name": "mcp__humata__inbox", "call_id": "c3", "arguments": "{}"
            })),
            codex_payload("2026-11-16T10:00:05Z", json!({
                "type": "function_call_output", "call_id": "c3", "output": "inbox answer"
            })),
        ],
    );

    let read = read_from(&file, 0, &mut OwnTools::default()).unwrap();

    assert_eq!(
        read.entries,
        vec![
            entry(Role::Tool, r#"shell {"command":["bash","-lc","ls"]}"#, "2026-11-16T10:00:00Z", false),
            entry(Role::Tool, "a.txt", "2026-11-16T10:00:01Z", false),
        ]
    );
}

#[test]
fn a_copilot_events_file_gives_prompts_replies_and_tools() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("events.jsonl");
    write_lines(
        &file,
        &[
            copilot("2026-11-17T02:42:43.111Z", "session.start", json!({"sessionId": "s"})),
            copilot("2026-11-17T02:42:43.612Z", "instruction.update", json!({"content": "<system_reminder>x</system_reminder>"})),
            copilot("2026-11-17T02:42:43.615Z", "user.message", json!({
                "content": "Fix the failing payments test",
                "transformedContent": "<current_datetime>now</current_datetime>\n\nFix the failing payments test"
            })),
            copilot("2026-11-17T02:42:43.650Z", "user.message", json!({
                "content": "subagent task", "source": "subagent"
            })),
            copilot("2026-11-17T02:42:43.657Z", "system.message", json!({"content": "You are the Copilot CLI"})),
            copilot("2026-11-17T02:42:43.700Z", "assistant.message", json!({
                "content": "Looking.",
                "toolRequests": [
                    {"toolCallId": "t1", "name": "bash", "arguments": {"command": "cat a.txt"}},
                    {"toolCallId": "t2", "name": "bash", "arguments": {"command": "humata inbox list"}}
                ]
            })),
            copilot("2026-11-17T02:42:43.710Z", "tool.execution_complete", json!({
                "toolCallId": "t1", "success": true, "result": {"content": "hello"}
            })),
            copilot("2026-11-17T02:42:43.720Z", "tool.execution_complete", json!({
                "toolCallId": "t2", "success": true, "result": {"content": "private"}
            })),
            copilot("2026-11-17T02:42:43.815Z", "assistant.message", json!({
                "content": "Fixed the assertion.", "toolRequests": []
            })),
            copilot("2026-11-17T02:42:43.828Z", "assistant.turn_end", json!({"turnId": "0"})),
            copilot("2026-11-17T02:42:47.001Z", "session.resume", json!({})),
            copilot("2026-11-17T02:42:48.413Z", "user.message", json!({"content": "Now the refund path"})),
        ],
    );

    let read = read_from(&file, 0, &mut OwnTools::default()).unwrap();

    assert_eq!(
        read.entries,
        vec![
            entry(Role::User, "Fix the failing payments test", "2026-11-17T02:42:43.615Z", true),
            entry(Role::Agent, "Looking.", "2026-11-17T02:42:43.700Z", false),
            entry(Role::Tool, r#"bash {"command":"cat a.txt"}"#, "2026-11-17T02:42:43.700Z", false),
            entry(Role::Tool, "hello", "2026-11-17T02:42:43.710Z", false),
            entry(Role::Agent, "Fixed the assertion.", "2026-11-17T02:42:43.815Z", false),
            entry(Role::User, "Now the refund path", "2026-11-17T02:42:48.413Z", true),
        ]
    );
}

#[test]
fn lines_of_unknown_shape_are_skipped_among_known_ones() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("mixed.jsonl");
    write_lines(
        &file,
        &[
            "not json at all".to_owned(),
            json!({"type": "something.else", "data": {"content": "x"}}).to_string(),
            json!({"type": "event_msg", "payload": {"type": "item_completed", "item": {"type": "Plan"}}}).to_string(),
            json!({"type": "response_item", "payload": {"type": "reasoning"}}).to_string(),
            json!({"hello": "world"}).to_string(),
            copilot("2026-11-17T02:42:48.413Z", "user.message", json!({"content": "Hi"})),
        ],
    );

    let read = read_from(&file, 0, &mut OwnTools::default()).unwrap();

    assert_eq!(
        read.entries,
        vec![entry(Role::User, "Hi", "2026-11-17T02:42:48.413Z", true)]
    );
}
