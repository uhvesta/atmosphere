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

use serde_json::{Value, json};

use crate::{Agent, PAYLOAD_MAX, PayloadError, ask_key};

/// Copilot's tool for asking the person.
const ASK_USER: &str = "ask_user";

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

/// Reads `agent`'s ask-user hook payload. Only Copilot's `preToolUse` of
/// its `ask_user` tool is a question; any other payload is an error, which
/// means no card and no decision.
pub fn parse_question(agent: Agent, payload: &[u8]) -> Result<Question, PayloadError> {
    if payload.len() > PAYLOAD_MAX {
        return Err(PayloadError::TooBig {
            size: payload.len(),
        });
    }
    let value: Value = serde_json::from_slice(payload).map_err(PayloadError::NotJson)?;
    let not_a_question = |why: &'static str| PayloadError::NotAQuestion { agent, why };
    if agent != Agent::Copilot {
        return Err(not_a_question("only Copilot's ask_user form is read"));
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

/// What the hook prints for `reply`.
pub fn render_question(agent: Agent, reply: &Reply) -> Vec<u8> {
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
        // No other agent's form is read, so none is answered.
        Agent::Claude | Agent::Codex | Agent::OpenCode => return Vec::new(),
    };
    serde_json::to_vec(&answer).unwrap_or_default()
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
