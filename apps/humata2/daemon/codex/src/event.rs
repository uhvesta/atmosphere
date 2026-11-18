//! What one message of Codex's app-server (JSON-RPC 2.0) means for Humata.

use std::path::PathBuf;

use serde_json::Value;

/// A JSON-RPC request id, which Codex may send as a number or a string.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RequestId(Id);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Id {
    Number(i64),
    Text(String),
}

impl RequestId {
    fn from_value(value: &Value) -> Option<Self> {
        match value {
            Value::Number(n) => n.as_i64().map(|n| Self(Id::Number(n))),
            Value::String(s) => Some(Self(Id::Text(s.clone()))),
            _ => None,
        }
    }

    fn to_value(&self) -> Value {
        match &self.0 {
            Id::Number(n) => Value::from(*n),
            Id::Text(s) => Value::from(s.as_str()),
        }
    }
}

/// What an ask wants answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AskKind {
    /// Approval to run a command.
    Command,
    /// Approval to change files.
    FileChange,
    /// A question for the person.
    Question,
}

/// One message from the app-server, reduced to what Humata uses.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// A thread (a conversation) started; a subagent's thread is not reported.
    ThreadStarted { thread: String, path: Option<PathBuf> },
    /// A turn started.
    TurnStarted { thread: String },
    /// A turn ended; an interrupted one was stopped by the person.
    TurnCompleted { thread: String, interrupted: bool },
    /// Codex waits for an approval or an answer.
    Ask {
        request: RequestId,
        thread: String,
        item: String,
        kind: AskKind,
        /// The request's whole `params`.
        params: Value,
    },
    /// An ask was answered elsewhere.
    Resolved { request: RequestId, thread: String },
}

/// Why a message could not be read.
#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    /// The message is not JSON.
    #[error(
        "Codex's app-server sent a message that is not JSON. Check that the socket belongs to \
         `codex app-server` by running `codex --version`, and update Humata if Codex is newer."
    )]
    NotJson {
        #[source]
        source: serde_json::Error,
    },
    /// The message is JSON but not a JSON-RPC object.
    #[error(
        "Codex's app-server sent JSON that is not a JSON-RPC message object. Check that the \
         socket belongs to `codex app-server`, and update Humata if Codex is newer."
    )]
    NotAnObject,
    /// A known message lacks a field Humata needs.
    #[error(
        "Codex message `{method}` has no `{field}` field. This Codex may differ from the one \
         Humata reads: update Humata if Codex is newer, or upgrade Codex if it is older."
    )]
    Missing { method: String, field: &'static str },
    /// A known message has a field of the wrong type.
    #[error(
        "Codex message `{method}` has a `{field}` field of the wrong type. This Codex may \
         differ from the one Humata reads: update Humata if Codex is newer, or upgrade Codex if \
         it is older."
    )]
    Malformed { method: String, field: &'static str },
}

struct Fields<'a> {
    method: &'a str,
}

impl Fields<'_> {
    fn get<'v>(&self, from: &'v Value, field: &'static str) -> Result<&'v Value, FrameError> {
        match from.get(field) {
            Some(v) if !v.is_null() => Ok(v),
            _ => Err(FrameError::Missing { method: self.method.to_owned(), field }),
        }
    }

    fn text(&self, from: &Value, field: &'static str) -> Result<String, FrameError> {
        self.get(from, field)?
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| FrameError::Malformed { method: self.method.to_owned(), field })
    }

    fn id(&self, from: &Value, field: &'static str) -> Result<RequestId, FrameError> {
        RequestId::from_value(self.get(from, field)?)
            .ok_or_else(|| FrameError::Malformed { method: self.method.to_owned(), field })
    }
}

/// Reads one message. Messages Humata doesn't use (unknown methods, replies
/// to its own requests, subagent threads) are `Ok(None)`.
pub fn read_frame(text: &str) -> Result<Option<Event>, FrameError> {
    let value: Value =
        serde_json::from_str(text).map_err(|source| FrameError::NotJson { source })?;
    if !value.is_object() {
        return Err(FrameError::NotAnObject);
    }
    let Some(method) = value.get("method").and_then(Value::as_str) else {
        return Ok(None);
    };
    let f = Fields { method };
    let empty = Value::Null;
    let params = value.get("params").unwrap_or(&empty);
    let ask = |kind: AskKind| -> Result<Option<Event>, FrameError> {
        Ok(Some(Event::Ask {
            request: f.id(&value, "id")?,
            thread: f.text(params, "threadId")?,
            item: f.text(params, "itemId")?,
            kind,
            params: params.clone(),
        }))
    };
    match method {
        "thread/started" => {
            let thread = f.get(params, "thread")?;
            if thread.get("parentThreadId").is_some_and(|p| !p.is_null()) {
                return Ok(None);
            }
            let path = match thread.get("path") {
                None | Some(Value::Null) => None,
                Some(Value::String(p)) => Some(PathBuf::from(p)),
                Some(_) => return Err(FrameError::Malformed { method: method.to_owned(), field: "path" }),
            };
            Ok(Some(Event::ThreadStarted { thread: f.text(thread, "id")?, path }))
        }
        "turn/started" => Ok(Some(Event::TurnStarted { thread: f.text(params, "threadId")? })),
        "turn/completed" => {
            let turn = f.get(params, "turn")?;
            Ok(Some(Event::TurnCompleted {
                thread: f.text(params, "threadId")?,
                interrupted: f.text(turn, "status")? == "interrupted",
            }))
        }
        "item/commandExecution/requestApproval" => ask(AskKind::Command),
        "item/fileChange/requestApproval" => ask(AskKind::FileChange),
        "item/tool/requestUserInput" => ask(AskKind::Question),
        "serverRequest/resolved" => Ok(Some(Event::Resolved {
            request: f.id(params, "requestId")?,
            thread: f.text(params, "threadId")?,
        })),
        _ => Ok(None),
    }
}

/// The reply that answers a server request with `result`.
pub fn answer_frame(request: &RequestId, result: Value) -> String {
    serde_json::json!({"jsonrpc": "2.0", "id": request.to_value(), "result": result}).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answer_echoes_a_numeric_or_text_id() {
        let n = RequestId::from_value(&Value::from(7)).unwrap();
        let s = RequestId::from_value(&Value::from("abc")).unwrap();
        let a: Value = serde_json::from_str(&answer_frame(&n, serde_json::json!({"decision": "accept"}))).unwrap();
        assert_eq!(a, serde_json::json!({"jsonrpc":"2.0","id":7,"result":{"decision":"accept"}}));
        let b: Value = serde_json::from_str(&answer_frame(&s, serde_json::json!({"answers": {}}))).unwrap();
        assert_eq!(b["id"], "abc");
    }
}
