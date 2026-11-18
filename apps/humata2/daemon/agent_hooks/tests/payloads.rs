//! Each agent's recorded permission payload becomes one card, and each
//! answer goes back the way that agent reads it (HOOKS, D191).

use humata2_agent_hooks::{Agent, Decision, parse_permission, render_permission};
use serde_json::Value;

const CLAUDE_BASH: &[u8] = include_bytes!("../testdata/claude/permission_bash.json");
const CLAUDE_EDIT: &[u8] = include_bytes!("../testdata/claude/permission_edit.json");
const CLAUDE_MCP: &[u8] = include_bytes!("../testdata/claude/permission_mcp.json");
const CLAUDE_STOP: &[u8] = include_bytes!("../testdata/claude/stop.json");
const CODEX_BASH: &[u8] = include_bytes!("../testdata/codex/permission_bash.json");
const CODEX_PATCH: &[u8] = include_bytes!("../testdata/codex/permission_patch.json");
const CODEX_STOP: &[u8] = include_bytes!("../testdata/codex/stop.json");
const COPILOT_SHELL: &[u8] = include_bytes!("../testdata/copilot/permission_shell.json");
const COPILOT_BYPASS: &[u8] = include_bytes!("../testdata/copilot/permission_sandbox_bypass.json");
const COPILOT_STOP: &[u8] = include_bytes!("../testdata/copilot/agent_stop.json");

/// What the hook prints for `decision`, as JSON.
fn rendered(agent: Agent, decision: &Decision) -> Value {
    serde_json::from_slice(&render_permission(agent, decision)).expect("JSON")
}

#[test]
fn agents_are_named_as_hook_commands_name_them() {
    for agent in [Agent::Claude, Agent::Codex, Agent::Copilot] {
        assert_eq!(Agent::parse(agent.name()), Some(agent));
    }
    assert_eq!(Agent::parse("gemini"), None);
}

#[test]
fn a_shell_ask_shows_the_whole_command() {
    let cases = [
        (Agent::Claude, CLAUDE_BASH, "npm test -- --watch=false", Some("Run the test suite")),
        (Agent::Codex, CODEX_BASH, "cargo publish --dry-run", Some("Check the crate packages")),
        (Agent::Copilot, COPILOT_SHELL, "rm -rf build", Some("Clean the build folder")),
    ];
    for (agent, payload, command, description) in cases {
        let ask = parse_permission(agent, payload).expect("an ask");
        assert_eq!(ask.command, command, "{agent:?}");
        assert_eq!(ask.description.as_deref(), description, "{agent:?}");
        assert!(!ask.tool.is_empty());
        assert!(!ask.key.is_empty());
    }
}

#[test]
fn a_patch_ask_shows_the_whole_patch() {
    let ask = parse_permission(Agent::Codex, CODEX_PATCH).expect("an ask");
    assert!(ask.command.starts_with("*** Begin Patch"));
    assert!(ask.command.contains("+let a = 2;"));
    assert!(ask.command.trim_end().ends_with("*** End Patch"));
}

#[test]
fn another_tool_shows_all_of_its_input() {
    for payload in [CLAUDE_EDIT, CLAUDE_MCP] {
        let ask = parse_permission(Agent::Claude, payload).expect("an ask");
        let input = serde_json::from_slice::<Value>(payload).expect("JSON")["tool_input"].clone();
        // Every value of the input is on the card, so approving it approves
        // what was shown.
        for value in input.as_object().expect("JSON object").values() {
            assert!(ask.command.contains(value.as_str().expect("string")), "{}", ask.command);
        }
    }
    let mcp = parse_permission(Agent::Claude, CLAUDE_MCP).expect("an ask");
    assert!(mcp.tool.contains("create_issue"));
}

#[test]
fn an_mcp_call_shows_one_line_per_argument_and_nothing_for_none() {
    let mcp = parse_permission(Agent::Claude, CLAUDE_MCP).expect("an ask");
    let lines: Vec<&str> = mcp.command.lines().collect();
    assert_eq!(lines.len(), 2, "{}", mcp.command);
    assert!(lines.iter().any(|line| line.starts_with("title") && line.contains("Login fails")));
    assert!(lines.iter().any(|line| line.starts_with("body") && line.contains("Steps…")));
    assert!(!mcp.command.contains('{'), "{}", mcp.command);

    let none = br#"{"hook_event_name":"PermissionRequest","tool_name":"mcp__humata2__whoami","tool_input":{}}"#;
    let none = parse_permission(Agent::Claude, none).expect("an ask");
    assert!(none.command.is_empty(), "{}", none.command);
    assert!(none.tool.contains("whoami"));
}

#[test]
fn a_file_edit_shows_its_path_then_its_change_as_lines() {
    const COPILOT_EDIT: &[u8] = include_bytes!("../testdata/copilot/permission_edit.json");
    for (agent, payload) in [(Agent::Claude, CLAUDE_EDIT), (Agent::Copilot, COPILOT_EDIT)] {
        let ask = parse_permission(agent, payload).expect("an ask");
        let lines: Vec<&str> = ask.command.lines().collect();
        assert_eq!(lines, ["/work/api/src/login.rs", "-let a = 1;", "+let a = 2;"], "{agent:?}");
    }
}

#[test]
fn a_multi_line_edit_or_write_keeps_each_line_and_every_other_argument() {
    let multi = br#"{"hook_event_name":"PermissionRequest","tool_name":"MultiEdit","tool_input":{"file_path":"/w/a.rs","edits":[{"old_string":"a\nb","new_string":"c","replace_all":true}]}}"#;
    let multi = parse_permission(Agent::Claude, multi).expect("an ask");
    let lines: Vec<&str> = multi.command.lines().collect();
    assert_eq!(lines[..4], ["/w/a.rs", "-a", "-b", "+c"], "{}", multi.command);
    assert!(lines[4..].iter().any(|line| line.contains("replace_all") && line.contains("true")));

    let write = br##"{"hook_event_name":"PermissionRequest","tool_name":"Write","tool_input":{"file_path":"/w/new.md","content":"# Title\n\nBody"}}"##;
    let write = parse_permission(Agent::Claude, write).expect("an ask");
    let lines: Vec<&str> = write.command.lines().collect();
    assert_eq!(lines, ["/w/new.md", "+# Title", "+", "+Body"]);
}

#[test]
fn the_same_ask_again_has_the_same_key_and_another_ask_another() {
    let first = parse_permission(Agent::Claude, CLAUDE_BASH).expect("an ask");
    let again = parse_permission(Agent::Claude, CLAUDE_BASH).expect("an ask");
    assert_eq!(first.key, again.key);
    let other = parse_permission(Agent::Claude, CLAUDE_EDIT).expect("an ask");
    assert_ne!(first.key, other.key);
    let codex_one = parse_permission(Agent::Codex, CODEX_BASH).expect("an ask");
    let codex_two = parse_permission(Agent::Codex, CODEX_PATCH).expect("an ask");
    assert_ne!(codex_one.key, codex_two.key);
}

#[test]
fn what_is_not_a_permission_ask_raises_no_card() {
    assert!(parse_permission(Agent::Claude, CLAUDE_STOP).is_err());
    assert!(parse_permission(Agent::Codex, CODEX_STOP).is_err());
    assert!(parse_permission(Agent::Copilot, COPILOT_STOP).is_err());
    assert!(parse_permission(Agent::Claude, b"not json").is_err());
    assert!(parse_permission(Agent::Claude, b"").is_err());
    // Another agent's shape is not guessed at.
    assert!(parse_permission(Agent::Copilot, CLAUDE_BASH).is_err());
    assert!(parse_permission(Agent::Claude, COPILOT_SHELL).is_err());
}

#[test]
fn a_copilot_sandbox_bypass_raises_no_card() {
    // Copilot's allow doesn't cover a sandbox bypass, so an Inbox approval
    // couldn't answer it: the TUI asks.
    assert!(parse_permission(Agent::Copilot, COPILOT_BYPASS).is_err());
}

#[test]
fn a_file_edit_ask_is_an_edit_and_a_shell_ask_is_a_command() {
    const COPILOT_EDIT: &[u8] = include_bytes!("../testdata/copilot/permission_edit.json");
    for (agent, payload) in [
        (Agent::Copilot, COPILOT_EDIT),
        (Agent::Claude, CLAUDE_EDIT),
        (Agent::Codex, CODEX_PATCH),
    ] {
        assert!(parse_permission(agent, payload).expect("an ask").edits_files, "{agent:?}");
    }
    for (agent, payload) in [
        (Agent::Copilot, COPILOT_SHELL),
        (Agent::Claude, CLAUDE_BASH),
        (Agent::Codex, CODEX_BASH),
        (Agent::Claude, CLAUDE_MCP),
    ] {
        assert!(!parse_permission(agent, payload).expect("an ask").edits_files, "{agent:?}");
    }
}

#[test]
fn a_payload_over_the_cap_raises_no_card() {
    let mut huge = br#"{"hook_event_name":"PermissionRequest","tool_name":"Bash","tool_input":{"command":""#.to_vec();
    huge.extend(std::iter::repeat_n(b'a', 2 * 1024 * 1024));
    huge.extend(br#""}}"#);
    assert!(parse_permission(Agent::Claude, &huge).is_err());
}

#[test]
fn claude_and_codex_read_allow_and_deny_from_hook_specific_output() {
    for agent in [Agent::Claude, Agent::Codex] {
        let allow = rendered(agent, &Decision::Allow);
        let out = &allow["hookSpecificOutput"];
        assert_eq!(out["hookEventName"], "PermissionRequest");
        assert_eq!(out["decision"]["behavior"], "allow");

        let deny = Decision::Deny {
            message: "use the staging database".into(),
        };
        let deny = rendered(agent, &deny);
        let decision = &deny["hookSpecificOutput"]["decision"];
        assert_eq!(decision["behavior"], "deny");
        assert_eq!(decision["message"], "use the staging database");
    }
}

#[test]
fn codex_never_gets_fields_it_fails_closed_on() {
    for decision in [
        Decision::Allow,
        Decision::Deny {
            message: "no".into(),
        },
    ] {
        let out = rendered(Agent::Codex, &decision);
        let decision = out["hookSpecificOutput"]["decision"].as_object().expect("JSON object");
        for field in ["updatedInput", "updatedPermissions", "interrupt"] {
            assert!(!decision.contains_key(field));
        }
    }
}

#[test]
fn copilot_reads_a_flat_behavior() {
    let allow = rendered(Agent::Copilot, &Decision::Allow);
    assert_eq!(allow["behavior"], "allow");
    let deny = Decision::Deny {
        message: "not on main".into(),
    };
    let deny = rendered(Agent::Copilot, &deny);
    assert_eq!(deny["behavior"], "deny");
    assert_eq!(deny["message"], "not on main");
}

#[test]
fn a_deny_without_a_note_still_tells_the_agent_something() {
    let deny = Decision::Deny {
        message: String::new(),
    };
    let claude = rendered(Agent::Claude, &deny);
    let message = &claude["hookSpecificOutput"]["decision"]["message"];
    assert!(!message.as_str().expect("string").is_empty());
    let copilot = rendered(Agent::Copilot, &deny);
    assert!(!copilot["message"].as_str().expect("string").is_empty());
}

#[test]
fn no_decision_is_empty_output_for_every_agent() {
    // Empty stdout on exit 0 is "no decision" for all three: the TUI asks.
    for agent in [Agent::Claude, Agent::Codex, Agent::Copilot] {
        assert!(render_permission(agent, &Decision::Undecided).is_empty());
    }
}
