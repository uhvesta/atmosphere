//! An agent's own ask-user form, as an Inbox question (D761).
//!
//! Copilot asks the person through its `ask_user` tool, which shows a form
//! in its TUI and raises no permission ask. Its `preToolUse` hook, matched
//! to `ask_user`, holds while the Inbox question is open:
//! - [`parse_question`] reads the hook's payload into the [`Question`] the
//!   card shows: the whole message, each field's title and description, and
//!   every choice the form offers.
//! - [`render_question`] writes the [`Reply`] the way Copilot reads it: a
//!   `deny` whose reason, shown to the model, carries the person's answer,
//!   so the form never opens. No answer is empty output, and the form opens.
//!
//! Claude Code's `AskUserQuestion` (a `PreToolUse` hook, D796) and Codex's
//! app-server `item/tool/requestUserInput` (D795) are read the same way.
//! Claude is answered with its own questions plus the answers (`updatedInput`),
//! so its dialog stays closed; Codex is answered with `answers` keyed by
//! question id. A question card holds one free answer for all the questions.

use serde_json::{Value, json};

use crate::{Agent, PAYLOAD_MAX, PayloadError, ask_key};

/// Copilot's tool for asking the person.
const ASK_USER: &str = "ask_user";

/// Claude Code's tool for asking the person.
const ASK_USER_QUESTION: &str = "AskUserQuestion";

/// How Claude is given the person's answer. `Updated` hands the questions
/// back answered; `DenyWithReason` is the Copilot-style fallback. Which one
/// Claude honours is owed live check O1 (workflow/milestones/AGENTS2.md).
#[derive(Clone, Copy)]
#[allow(dead_code)]
enum ClaudeAnswer {
    Updated,
    DenyWithReason,
}

const CLAUDE_ANSWER: ClaudeAnswer = ClaudeAnswer::Updated;

/// One question an agent asks the person, as the Inbox card shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    /// All the agent asks, with each field of its form on a line of its own.
    pub question: String,
    /// The one-tap answers the form offers; the person may write their own.
    pub choices: Vec<String>,
    /// The same for the same question again within a session.
    pub key: String,
}

/// How the person answered the question, or that no one did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    /// The person's answer, in their words or one of the choices.
    Answer(String),
    /// The person declined; `message` is their note, empty if none.
    Declined { message: String },
    /// No answer in time, or the question was withdrawn: the form opens.
    Undecided,
}

/// Reads `agent`'s ask-user hook payload: Copilot's `preToolUse` of its
/// `ask_user` tool, Claude's `PreToolUse` of `AskUserQuestion`, or Codex's
/// `requestUserInput` params. Any other payload is an error, which means no
/// card and no decision.
pub fn parse_question(agent: Agent, payload: &[u8]) -> Result<Question, PayloadError> {
    if payload.len() > PAYLOAD_MAX {
        return Err(PayloadError::TooBig {
            size: payload.len(),
        });
    }
    let value: Value = serde_json::from_slice(payload).map_err(PayloadError::NotJson)?;
    let not_a_question = |why: &'static str| PayloadError::NotAQuestion { agent, why };
    match agent {
        Agent::Copilot => {}
        Agent::Claude => {
            if value.get("tool_name").and_then(Value::as_str) != Some(ASK_USER_QUESTION) {
                return Err(not_a_question("tool_name isn't AskUserQuestion"));
            }
            let questions = value
                .get("tool_input")
                .and_then(|input| input.get("questions"));
            return form_question(agent, ASK_USER_QUESTION, questions, &value)
                .ok_or_else(|| not_a_question("AskUserQuestion has no questions"));
        }
        Agent::Codex => {
            return form_question(agent, "requestUserInput", value.get("questions"), &value)
                .ok_or_else(|| not_a_question("no questions to ask"));
        }
        Agent::OpenCode => return Err(not_a_question("this agent's form isn't read")),
    }
    if value.get("toolName").and_then(Value::as_str) != Some(ASK_USER) {
        return Err(not_a_question("toolName isn't ask_user"));
    }
    let args = object_or_json(value.get("toolArgs"))
        .ok_or_else(|| not_a_question("toolArgs isn't an object"))?;
    let message = ["message", "question"]
        .iter()
        .find_map(|key| args.get(*key).and_then(Value::as_str))
        .ok_or_else(|| not_a_question("ask_user has no message"))?;

    let mut lines = vec![message.to_owned()];
    let mut choices: Vec<String> = strings(args.get("choices"));
    let schema = object_or_json(args.get("requestedSchema"));
    let fields = schema
        .as_ref()
        .and_then(|schema| schema.get("properties"))
        .and_then(Value::as_object);
    if let Some(fields) = fields {
        for field in fields.values() {
            let title = field.get("title").and_then(Value::as_str);
            let description = field.get("description").and_then(Value::as_str);
            match (title, description) {
                (Some(title), Some(description)) => lines.push(format!("{title}: {description}")),
                (Some(text), None) | (None, Some(text)) => lines.push(text.to_owned()),
                (None, None) => {}
            }
            choices.extend(field_choices(field));
        }
    }
    let mut seen = std::collections::HashSet::new();
    choices.retain(|choice| seen.insert(choice.clone()));
    Ok(Question {
        question: lines.join("\n"),
        choices,
        key: ask_key(&[agent.name(), ASK_USER, &args.to_string()]),
    })
}

/// What the hook prints for `reply`. `payload` is the hook payload the
/// question was parsed from: Claude's answer carries its questions back, and
/// Codex's is keyed by their ids. An unreadable payload prints nothing, so
/// the agent's own form opens.
pub fn render_question(agent: Agent, payload: &[u8], reply: &Reply) -> Vec<u8> {
    match agent {
        Agent::Claude => return render_claude(CLAUDE_ANSWER, payload, reply),
        Agent::Codex => return render_codex(payload, reply),
        Agent::Copilot | Agent::OpenCode => {}
    }
    let reason = match reply {
        Reply::Undecided => return Vec::new(),
        Reply::Answer(text) => format!(
            "The user answered this question in Humata's Inbox, so the form was not shown. \
             Their answer: {text}\nGo on with this answer; don't ask it again."
        ),
        Reply::Declined { message } if message.is_empty() => {
            "The user declined to answer this question in Humata's Inbox. \
             Go on without the answer, or ask again in words if you can't."
                .to_owned()
        }
        Reply::Declined { message } => format!(
            "The user declined to answer this question in Humata's Inbox, and said: {message}"
        ),
    };
    let answer = match agent {
        Agent::Copilot => json!({"permissionDecision": "deny", "permissionDecisionReason": reason}),
        Agent::Claude | Agent::Codex | Agent::OpenCode => return Vec::new(),
    };
    serde_json::to_vec(&answer).unwrap_or_default()
}

/// The questions array of a Claude or Codex form, as the card shows it.
fn form_question(agent: Agent, tool: &str, questions: Option<&Value>, whole: &Value) -> Option<Question> {
    let questions = questions?.as_array().filter(|list| !list.is_empty())?;
    let mut lines = Vec::new();
    for item in questions {
        let text = item.get("question").and_then(Value::as_str)?;
        match item.get("header").and_then(Value::as_str) {
            Some(header) if !header.is_empty() => lines.push(format!("{header}: {text}")),
            _ => lines.push(text.to_owned()),
        }
        for option in options(item) {
            lines.push(match option.1 {
                Some(description) => format!("- {}: {description}", option.0),
                None => format!("- {}", option.0),
            });
        }
    }
    let single_choice = match questions.as_slice() {
        [only] => only.get("multiSelect").and_then(Value::as_bool) != Some(true),
        _ => false,
    };
    let choices = if single_choice {
        options(&questions[0]).into_iter().map(|option| option.0).collect()
    } else {
        Vec::new()
    };
    let content = match agent {
        Agent::Claude => whole.get("tool_input").unwrap_or(whole),
        _ => whole,
    };
    Some(Question {
        question: lines.join("\n"),
        choices,
        key: ask_key(&[agent.name(), tool, &content.to_string()]),
    })
}

/// A question's options as (label, description).
fn options(question: &Value) -> Vec<(String, Option<String>)> {
    question
        .get("options")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|option| {
                    let label = option.get("label").and_then(Value::as_str)?.to_owned();
                    let description = option
                        .get("description")
                        .and_then(Value::as_str)
                        .filter(|text| !text.is_empty())
                        .map(str::to_owned);
                    Some((label, description))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Claude's answer: its questions back with the answers, or a deny.
fn render_claude(mode: ClaudeAnswer, payload: &[u8], reply: &Reply) -> Vec<u8> {
    let deny = |reason: String| {
        json!({"hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": reason,
        }})
    };
    let answer = match reply {
        Reply::Undecided => return Vec::new(),
        Reply::Declined { message } if message.is_empty() => deny(
            "The user declined to answer this question in Humata's Inbox. \
             Go on without the answer, or ask again in words if you can't."
                .to_owned(),
        ),
        Reply::Declined { message } => deny(format!(
            "The user declined to answer this question in Humata's Inbox, and said: {message}"
        )),
        Reply::Answer(text) => {
            let Some(questions) = claude_questions(payload) else {
                return Vec::new();
            };
            match mode {
                ClaudeAnswer::Updated => {
                    let answers: serde_json::Map<String, Value> = questions
                        .iter()
                        .filter_map(|item| item.get("question").and_then(Value::as_str))
                        .map(|question| (question.to_owned(), Value::String(text.clone())))
                        .collect();
                    json!({"hookSpecificOutput": {
                        "hookEventName": "PreToolUse",
                        "permissionDecision": "allow",
                        "updatedInput": {"questions": questions, "answers": answers},
                    }})
                }
                ClaudeAnswer::DenyWithReason => deny(format!(
                    "The user answered this question in Humata's Inbox, so the dialog was not shown. \
                     Their answer: {text}\nGo on with this answer; don't ask it again."
                )),
            }
        }
    };
    serde_json::to_vec(&answer).unwrap_or_default()
}

fn claude_questions(payload: &[u8]) -> Option<Vec<Value>> {
    let value: Value = serde_json::from_slice(payload).ok()?;
    value
        .get("tool_input")?
        .get("questions")?
        .as_array()
        .filter(|list| !list.is_empty())
        .cloned()
}

/// Codex's answer: `answers` keyed by question id; otherwise its popup asks.
fn render_codex(payload: &[u8], reply: &Reply) -> Vec<u8> {
    let Reply::Answer(text) = reply else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_slice::<Value>(payload) else {
        return Vec::new();
    };
    let answers: serde_json::Map<String, Value> = value
        .get("questions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("id").and_then(Value::as_str))
        .map(|id| (id.to_owned(), json!({"answers": [text]})))
        .collect();
    if answers.is_empty() {
        return Vec::new();
    }
    serde_json::to_vec(&json!({"answers": answers})).unwrap_or_default()
}

/// An object, or a JSON string holding one: Copilot's docs give `toolArgs`
/// as a string, and its records show an object.
fn object_or_json(value: Option<&Value>) -> Option<Value> {
    match value? {
        object @ Value::Object(_) => Some(object.clone()),
        Value::String(text) => serde_json::from_str::<Value>(text).ok().filter(Value::is_object),
        _ => None,
    }
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).map(str::to_owned).collect())
        .unwrap_or_default()
}

/// The values a form field offers: an `enum`, `oneOf`/`anyOf` of `const`s,
/// or an array whose `items` do.
fn field_choices(field: &Value) -> Vec<String> {
    let mut choices = strings(field.get("enum"));
    for key in ["oneOf", "anyOf"] {
        if let Some(options) = field.get(key).and_then(Value::as_array) {
            choices.extend(
                options
                    .iter()
                    .filter_map(|option| option.get("const").and_then(Value::as_str))
                    .map(str::to_owned),
            );
        }
    }
    if let Some(items) = field.get("items") {
        choices.extend(field_choices(items));
    }
    choices
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAYLOAD: &str = r#"{"tool_name":"AskUserQuestion","tool_input":{"questions":[{"question":"Which?","header":"H","multiSelect":false,"options":[{"label":"A","description":"a"}]}]}}"#;

    fn claude(mode: ClaudeAnswer) -> Value {
        let reply = Reply::Answer("A".to_owned());
        serde_json::from_slice(&render_claude(mode, PAYLOAD.as_bytes(), &reply)).expect("JSON")
    }

    #[test]
    fn updated_hands_the_questions_back_answered() {
        let out = claude(ClaudeAnswer::Updated);
        let hook = &out["hookSpecificOutput"];
        assert_eq!(hook["permissionDecision"], "allow");
        assert_eq!(hook["updatedInput"]["answers"]["Which?"], "A");
        assert_eq!(hook["updatedInput"]["questions"][0]["question"], "Which?");
    }

    #[test]
    fn deny_with_reason_carries_the_answer() {
        let out = claude(ClaudeAnswer::DenyWithReason);
        let hook = &out["hookSpecificOutput"];
        assert_eq!(hook["permissionDecision"], "deny");
        assert!(hook["permissionDecisionReason"].as_str().expect("string").contains("A"));
    }
}
