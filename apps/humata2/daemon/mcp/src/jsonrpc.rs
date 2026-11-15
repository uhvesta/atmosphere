//! JSON-RPC 2.0 messages, one per line on stdio (the MCP stdio transport).

use serde_json::{Map, Value, json};

pub(crate) const PARSE_ERROR: i64 = -32700;
pub(crate) const INVALID_REQUEST: i64 = -32600;
pub(crate) const METHOD_NOT_FOUND: i64 = -32601;
pub(crate) const INVALID_PARAMS: i64 = -32602;

/// A JSON-RPC error. MCP keeps these for calls the protocol or a tool's
/// schema rejects (MCP.md §3); everything else is a tool result.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RpcError {
    pub(crate) code: i64,
    pub(crate) message: String,
    pub(crate) data: Option<Value>,
}

impl RpcError {
    pub(crate) fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    pub(crate) fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }
}

/// One line from the client.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Incoming {
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    Notification {
        method: String,
        params: Value,
    },
    /// A reply to a request of ours. The server sends none, so these are
    /// dropped.
    Response,
    /// Not a message we can act on; answered with this error.
    Invalid { id: Value, error: RpcError },
}

pub(crate) fn parse(line: &str) -> Incoming {
    let value: Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(error) => {
            return Incoming::Invalid {
                id: Value::Null,
                error: RpcError::new(PARSE_ERROR, format!("not JSON: {error}")),
            };
        }
    };
    let Value::Object(mut message) = value else {
        let reason = if value.is_array() {
            "batches aren't supported; send one JSON-RPC message per line"
        } else {
            "a JSON-RPC message must be an object"
        };
        return Incoming::Invalid {
            id: Value::Null,
            error: RpcError::new(INVALID_REQUEST, reason),
        };
    };
    let id = message.remove("id");
    if let Some(id) = &id
        && !(id.is_string() || id.is_number())
    {
        return invalid(Value::Null, "the id must be a string or a number");
    }
    if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return invalid(id.unwrap_or(Value::Null), "jsonrpc must be \"2.0\"");
    }
    let method = match message.remove("method") {
        Some(Value::String(method)) => method,
        Some(_) => return invalid(id.unwrap_or(Value::Null), "method must be a string"),
        None if message.contains_key("result") || message.contains_key("error") => {
            return Incoming::Response;
        }
        None => return invalid(id.unwrap_or(Value::Null), "the message has no method"),
    };
    let params = match message.remove("params") {
        None => Value::Object(Map::new()),
        Some(params @ (Value::Object(_) | Value::Array(_))) => params,
        Some(_) => return invalid(id.unwrap_or(Value::Null), "params must be an object"),
    };
    match id {
        Some(id) => Incoming::Request { id, method, params },
        None => Incoming::Notification { method, params },
    }
}

fn invalid(id: Value, message: &str) -> Incoming {
    Incoming::Invalid {
        id,
        error: RpcError::new(INVALID_REQUEST, message),
    }
}

pub(crate) fn result(id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

pub(crate) fn error(id: &Value, error: &RpcError) -> Value {
    let mut body = Map::new();
    body.insert("code".to_owned(), Value::from(error.code));
    body.insert("message".to_owned(), Value::from(error.message.clone()));
    if let Some(data) = &error.data {
        body.insert("data".to_owned(), data.clone());
    }
    json!({ "jsonrpc": "2.0", "id": id, "error": body })
}

pub(crate) fn notification(method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "method": method, "params": params })
}
