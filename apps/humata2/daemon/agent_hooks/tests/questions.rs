//! Copilot's ask-user form becomes one Inbox question with all it asks and
//! every choice it offers, and the person's answer goes back the way
//! Copilot reads it (D761).

use std::path::Path;

use humata2_agent_hooks::{
    Agent, HOLD_SECONDS, Reply, copilot_plugin, parse_permission, parse_question, render_question,
};
use serde_json::Value;

const ASK_TEXT: &[u8] = include_bytes!("../testdata/copilot/ask_user_text.json");
const ASK_CHOICES: &[u8] = include_bytes!("../testdata/copilot/ask_user_choices.json");
const PRE_TOOL_BASH: &[u8] = include_bytes!("../testdata/copilot/pre_tool_bash.json");
const COPILOT_SHELL: &[u8] = include_bytes!("../testdata/copilot/permission_shell.json");

fn rendered(reply: &Reply) -> Value {
    serde_json::from_slice(&render_question(Agent::Copilot, reply)).expect("JSON")
}

#[test]
fn a_free_text_form_shows_its_message_and_its_field() {
    let question = parse_question(Agent::Copilot, ASK_TEXT).expect("a question");
    assert!(question.question.contains("What should I draft?"));
    assert!(question.question.contains("What to draft"));
    assert!(question.question.contains("the file name if you want one saved"));
    assert!(question.choices.is_empty());
    assert!(!question.key.is_empty());
}

#[test]
fn a_form_with_choices_offers_each_of_them() {
    let question = parse_question(Agent::Copilot, ASK_CHOICES).expect("a question");
    assert!(question.question.contains("Which environment should I deploy to?"));
    assert_eq!(question.choices, ["staging", "production"]);
}

#[test]
fn the_same_question_has_the_same_key_and_another_another() {
    let first = parse_question(Agent::Copilot, ASK_TEXT).expect("a question");
    let again = parse_question(Agent::Copilot, ASK_TEXT).expect("a question");
    let other = parse_question(Agent::Copilot, ASK_CHOICES).expect("a question");
    assert_eq!(first.key, again.key);
    assert_ne!(first.key, other.key);
}

#[test]
fn what_is_not_an_ask_user_form_raises_no_question() {
    for (agent, payload) in [
        (Agent::Copilot, PRE_TOOL_BASH),
        (Agent::Copilot, COPILOT_SHELL),
        (Agent::Copilot, b"not json".as_slice()),
        (Agent::Claude, ASK_TEXT),
    ] {
        assert!(parse_question(agent, payload).is_err(), "{agent:?}");
    }
    // And the form is no permission ask.
    assert!(parse_permission(Agent::Copilot, ASK_TEXT).is_err());
}

#[test]
fn an_answer_reaches_copilot_as_the_reason_its_form_was_skipped() {
    let out = rendered(&Reply::Answer("a file draft.txt containing draft".to_owned()));
    assert_eq!(out["permissionDecision"], "deny");
    let reason = out["permissionDecisionReason"].as_str().expect("string");
    assert!(reason.contains("a file draft.txt containing draft"));
}

#[test]
fn a_decline_tells_copilot_something_with_or_without_a_note() {
    for message in ["", "not now"] {
        let out = rendered(&Reply::Declined {
            message: message.to_owned(),
        });
        assert_eq!(out["permissionDecision"], "deny");
        let reason = out["permissionDecisionReason"].as_str().expect("string");
        assert!(reason.contains(message));
        assert!(!reason.is_empty());
    }
}

#[test]
fn no_answer_is_empty_output_so_the_form_opens() {
    assert!(render_question(Agent::Copilot, &Reply::Undecided).is_empty());
}

#[test]
fn copilots_plugin_holds_its_ask_user_tool_for_an_hour() {
    let files = copilot_plugin(Path::new("/opt/humata/bin/humata"), Path::new("/tmp/h.sock"));
    let hooks: Value = serde_json::from_str(
        &files.iter().find(|file| file.path == "hooks.json").expect("hooks.json").contents,
    )
    .expect("JSON");
    let list = hooks["hooks"]["preToolUse"].as_array().expect("preToolUse is an array");
    let question = list
        .iter()
        .find(|hook| hook["bash"].as_str().is_some_and(|bash| bash.contains("hook copilot question")))
        .expect("a question hook");
    assert_eq!(question["timeoutSec"], HOLD_SECONDS);
    assert_eq!(question["matcher"], "ask_user");
}
