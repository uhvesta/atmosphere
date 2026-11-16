//! Tool results (MCP.md §3): the envelope for what a call did, and the
//! error shape for what it couldn't do. Each result carries the same JSON
//! as `structuredContent` and, on one line, as text for agents that read
//! only text.

use serde_json::{Map, Value, json};

use crate::daemon::{Reply, RequestId, ToolError};
use crate::time::rfc3339;

/// The envelope, `https://humata.dev/mcp/v1/envelope.json`.
fn envelope_schema() -> Value {
    json!({
        "type": "object",
        "required": ["outcome"],
        "properties": {
            "outcome": {
                "enum": ["applied", "journaled", "pending"],
                "description": "applied: done on the host or this machine. journaled: the host is away; written to this machine's journal and applied when the host replays it. pending: waiting for the user; call inbox_wait with request_id."
            },
            "result": {
                "description": "The tool's output; set when applied, and for journaled writes what was journaled. A journaled new item has no id until the host replays it."
            },
            "request_id": {
                "type": "string",
                "description": "Set when pending, and when an applied call went through the Inbox."
            },
            "journal": {
                "type": "object",
                "properties": { "machine": { "type": "string" }, "seq": { "type": "integer" } }
            },
            "stale_as_of": {
                "type": "string",
                "format": "date-time",
                "description": "Reads only: the host is away and this is the last copy, from this time."
            }
        }
    })
}

/// The error shape, `https://humata.dev/mcp/v1/error.json`.
fn error_schema() -> Value {
    json!({
        "type": "object",
        "required": ["error"],
        "properties": {
            "error": {
                "type": "object",
                "required": ["kind", "message", "fix"],
                "properties": {
                    "kind": {
                        "type": "string",
                        "description": "errors.proto kind, e.g. denied, user_only, revision_conflict, host_unreachable, rate_limited. Branch on this, never on the text."
                    },
                    "message": { "type": "string", "description": "What failed." },
                    "fix": { "type": "string", "description": "How to fix it, or why the obvious fix is unsafe." },
                    "retry": {
                        "type": "object",
                        "properties": {
                            "retryable": { "type": "boolean" },
                            "after": { "type": "string", "format": "date-time" },
                            "same_key": { "type": "boolean" }
                        }
                    },
                    "details": { "type": "object", "description": "The kind's fields, e.g. source and capability for denied." }
                }
            }
        }
    })
}

/// Every tool's `outputSchema`: the envelope, or the error shape for a
/// result with `isError`. Inline, with no `$id` or `$ref`: clients compile
/// each tool's schema on its own and can't fetch a reference, and some
/// refuse two schemas with one `$id`.
pub(crate) fn output_schema() -> Value {
    json!({
        "type": "object",
        "anyOf": [envelope_schema(), error_schema()]
    })
}

/// The result for a call that did something or waits. `waited_on` is the
/// request the server waited for, named in an applied result that doesn't
/// name it already.
pub(crate) fn success(reply: Reply, waited_on: Option<&RequestId>) -> Value {
    let mut envelope = Map::new();
    match reply {
        Reply::Applied {
            result,
            request,
            stale_as_of,
        } => {
            envelope.insert("outcome".to_owned(), json!("applied"));
            envelope.insert("result".to_owned(), result);
            let request = request.or_else(|| waited_on.cloned());
            if let Some(request) = request {
                envelope.insert("request_id".to_owned(), json!(request.as_str()));
            }
            if let Some(time) = stale_as_of {
                envelope.insert("stale_as_of".to_owned(), json!(rfc3339(time)));
            }
        }
        Reply::Journaled { result, journal } => {
            envelope.insert("outcome".to_owned(), json!("journaled"));
            envelope.insert("result".to_owned(), result);
            envelope.insert(
                "journal".to_owned(),
                json!({ "machine": journal.machine, "seq": journal.seq }),
            );
        }
        Reply::Pending { request } => {
            envelope.insert("outcome".to_owned(), json!("pending"));
            envelope.insert("request_id".to_owned(), json!(request.as_str()));
        }
    }
    tool_result(Value::Object(envelope), false)
}

/// The result for a call that failed or was refused.
pub(crate) fn failure(error: &ToolError) -> Value {
    let mut body = Map::new();
    body.insert("kind".to_owned(), json!(error.kind.as_str()));
    body.insert("message".to_owned(), json!(error.message));
    body.insert("fix".to_owned(), json!(error.fix));
    if let Some(retry) = &error.retry {
        let mut advice = Map::new();
        advice.insert("retryable".to_owned(), json!(retry.retryable));
        if let Some(after) = retry.after {
            advice.insert("after".to_owned(), json!(rfc3339(after)));
        }
        advice.insert("same_key".to_owned(), json!(retry.same_key));
        body.insert("retry".to_owned(), Value::Object(advice));
    }
    if !error.details.is_empty() {
        body.insert("details".to_owned(), Value::Object(error.details.clone()));
    }
    tool_result(json!({ "error": body }), true)
}

fn tool_result(structured: Value, is_error: bool) -> Value {
    json!({
        "content": [{ "type": "text", "text": structured.to_string() }],
        "structuredContent": structured,
        "isError": is_error,
    })
}
