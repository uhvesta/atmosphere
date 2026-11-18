//! Claude Code's AskUserQuestion and Codex's request_user_input become one
//! Inbox question with all they ask and every option they offer, and the
//! person's answer goes back the way each agent reads it (D796, D795).
//! Claude gets its questions back with the answers, so its dialog never
//! opens; no answer is no output, and the dialog opens. Codex gets the
//! answers keyed by question id.

use humata2_agent_hooks::{Agent, Reply, parse_permission, parse_question, render_question};
use serde_json::Value;

const CLAUDE_ASK: &[u8] = include_bytes!("../testdata/claude/ask_user_question.json");
const CLAUDE_ASK_MULTI: &[u8] = include_bytes!("../testdata/claude/ask_user_question_multi.json");
const CLAUDE_BASH: &[u8] = include_bytes!("../testdata/claude/pre_tool_bash.json");
const CLAUDE_PERMISSION: &[u8] = include_bytes!("../testdata/claude/permission_bash.json");
const CODEX_ASK: &[u8] = include_bytes!("../testdata/codex/request_user_input.json");

fn rendered(agent: Agent, payload: &[u8], reply: &Reply) -> Value {
    serde_json::from_slice(&render_question(agent, payload, reply)).expect("JSON")
}

#[test]
fn a_claude_question_shows_what_it_asks_and_offers_its_options() {
    let question = parse_question(Agent::Claude, CLAUDE_ASK).expect("a question");
    assert!(question.question.contains("Which database should the service use?"));
    assert_eq!(question.choices, ["Postgres", "SQLite"]);
    assert!(!question.key.is_empty());
}

#[test]
fn every_question_claude_asks_at_once_is_on_the_card() {
    let question = parse_question(Agent::Claude, CLAUDE_ASK_MULTI).expect("a question");
    assert!(question.question.contains("Which checks should run before each push?"));
    assert!(question.question.contains("Where should failures be reported?"));
    // Each option is shown, so the person can answer every question.
    for option in ["Lint", "Unit tests", "Build", "Terminal", "Inbox"] {
        assert!(question.question.contains(option), "{option} isn't shown");
    }
}

#[test]
fn the_same_claude_question_has_the_same_key_and_another_another() {
    let first = parse_question(Agent::Claude, CLAUDE_ASK).expect("a question");
    let again = parse_question(Agent::Claude, CLAUDE_ASK).expect("a question");
    let other = parse_question(Agent::Claude, CLAUDE_ASK_MULTI).expect("a question");
    assert_eq!(first.key, again.key);
    assert_ne!(first.key, other.key);
}

#[test]
fn what_is_not_claudes_ask_raises_no_question() {
    for payload in [CLAUDE_BASH, CLAUDE_PERMISSION, b"not json".as_slice(), CODEX_ASK] {
        assert!(parse_question(Agent::Claude, payload).is_err());
    }
    // And the question is no permission ask.
    assert!(parse_permission(Agent::Claude, CLAUDE_ASK).is_err());
}

#[test]
fn an_answer_reaches_claude_as_its_questions_answered_so_the_dialog_stays_closed() {
    let out = rendered(Agent::Claude, CLAUDE_ASK, &Reply::Answer("SQLite".to_owned()));
    let hook = &out["hookSpecificOutput"];
    assert_eq!(hook["hookEventName"], "PreToolUse");
    assert_eq!(hook["permissionDecision"], "allow");
    let input = &hook["updatedInput"];
    // The questions go back as Claude asked them, with the answer to each.
    assert_eq!(
        input["questions"][0]["question"],
        "Which database should the service use?"
    );
    assert_eq!(input["answers"]["Which database should the service use?"], "SQLite");
}

#[test]
fn every_question_asked_at_once_gets_an_answer() {
    let out = rendered(
        Agent::Claude,
        CLAUDE_ASK_MULTI,
        &Reply::Answer("Lint and Build; report in the Inbox".to_owned()),
    );
    let answers = out["hookSpecificOutput"]["updatedInput"]["answers"]
        .as_object()
        .expect("answers is an object");
    for question in [
        "Which checks should run before each push?",
        "Where should failures be reported?",
    ] {
        let answer = answers[question].as_str().expect("an answer");
        assert!(!answer.is_empty(), "{question} has no answer");
    }
}

#[test]
fn a_decline_tells_claude_it_was_declined_without_opening_the_dialog() {
    for message in ["", "not now"] {
        let out = rendered(
            Agent::Claude,
            CLAUDE_ASK,
            &Reply::Declined {
                message: message.to_owned(),
            },
        );
        let hook = &out["hookSpecificOutput"];
        assert_eq!(hook["permissionDecision"], "deny");
        let reason = hook["permissionDecisionReason"].as_str().expect("string");
        assert!(!reason.is_empty());
        assert!(reason.contains(message));
    }
}

#[test]
fn no_answer_to_claude_is_no_output_so_the_dialog_opens() {
    assert!(render_question(Agent::Claude, CLAUDE_ASK, &Reply::Undecided).is_empty());
    // Nor does a payload that can't be read answer anything.
    assert!(render_question(Agent::Claude, b"not json", &Reply::Answer("x".to_owned())).is_empty());
}

#[test]
fn a_codex_question_shows_what_it_asks_and_offers_its_options() {
    let question = parse_question(Agent::Codex, CODEX_ASK).expect("a question");
    assert!(question.question.contains("Which environment should I deploy to?"));
    assert_eq!(question.choices, ["staging", "production"]);
    assert!(parse_question(Agent::Codex, CLAUDE_ASK).is_err());
}

#[test]
fn an_answer_reaches_codex_keyed_by_its_question() {
    let out = rendered(Agent::Codex, CODEX_ASK, &Reply::Answer("staging".to_owned()));
    assert_eq!(out["answers"]["deploy_target"]["answers"][0], "staging");
}

#[test]
fn no_answer_to_codex_is_no_output_so_its_popup_answers() {
    assert!(render_question(Agent::Codex, CODEX_ASK, &Reply::Undecided).is_empty());
}
