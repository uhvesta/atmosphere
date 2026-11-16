//! What one event from OpenCode's stream says, for the turn and the Inbox.

use serde_json::Value;

/// The name an event gets in errors when its frame has no `type`.
const UNTYPED: &str = "(no type)";

/// One event from OpenCode's event stream, reduced to what Humata uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// The stream is open.
    Connected,
    /// A session started a run.
    Started { session: String },
    /// A session's run ended, by success, interruption or failure.
    Ended { session: String },
    /// A session asked permission to act.
    Asked(Asked),
    /// An ask was answered.
    Replied { session: String, request: String },
    /// An event Humata doesn't use.
    Other,
}

/// A permission ask, as the Inbox shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asked {
    /// The ask's id, which the reply names.
    pub id: String,
    /// The session that asked.
    pub session: String,
    /// What it wants to do, such as `shell`.
    pub action: String,
    /// What it wants to do, such as the command line.
    pub resources: Vec<String>,
}

/// Why an event could not be read.
#[derive(Debug, thiserror::Error)]
pub enum EventError {
    /// The frame is not JSON at all.
    #[error(
        "OpenCode's event stream sent a frame that is not JSON. Check that the server is \
         OpenCode by running `opencode --version`; if it is older than 2.3.0, upgrade it, and \
         if it is newer, update Humata."
    )]
    NotJson {
        #[source]
        source: serde_json::Error,
    },
    /// A known event has no field Humata needs.
    #[error(
        "OpenCode event `{event}` has no `{field}` field. This OpenCode may differ from the \
         2.3.0 or later version Humata reads: update Humata if OpenCode is newer than that, or \
         upgrade OpenCode to 2.3.0 or later if it is older."
    )]
    Missing { event: String, field: &'static str },
    /// A known event has a field of the wrong type.
    #[error(
        "OpenCode event `{event}` has a `{field}` field of the wrong type. This OpenCode may \
         differ from the 2.3.0 or later version Humata reads: update Humata if OpenCode is \
         newer than that, or upgrade OpenCode to 2.3.0 or later if it is older."
    )]
    Malformed {
        event: String,
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
}

impl Event {
    /// Reads one frame's data as an event.
    ///
    /// Events Humata doesn't use come back as [`Event::Other`]. A known event
    /// with a missing or mistyped field is an error.
    pub fn parse(frame: &str) -> Result<Self, EventError> {
        let value: Value =
            serde_json::from_str(frame).map_err(|source| EventError::NotJson { source })?;
        let kind =
            value
                .get("type")
                .and_then(Value::as_str)
                .ok_or_else(|| EventError::Missing {
                    event: UNTYPED.to_owned(),
                    field: "type",
                })?;
        match kind {
            "server.connected" => Ok(Self::Connected),
            "session.execution.started" => Ok(Self::Started {
                session: data_text(&value, kind, "sessionID")?,
            }),
            "session.execution.succeeded"
            | "session.execution.interrupted"
            | "session.execution.failed" => Ok(Self::Ended {
                session: data_text(&value, kind, "sessionID")?,
            }),
            "permission.asked" => asked(&value).map(Self::Asked),
            "permission.replied" => Ok(Self::Replied {
                session: data_text(&value, kind, "sessionID")?,
                request: data_text(&value, kind, "requestID")?,
            }),
            _ => Ok(Self::Other),
        }
    }
}

/// The text at `data.<key>` of an event, or the error that it is missing.
fn data_text(value: &Value, event: &str, key: &'static str) -> Result<String, EventError> {
    value
        .get("data")
        .and_then(|data| data.get(key))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| EventError::Missing {
            event: event.to_owned(),
            field: key,
        })
}

/// The permission ask in a `permission.asked` event.
fn asked(value: &Value) -> Result<Asked, EventError> {
    const EVENT: &str = "permission.asked";
    let resources = match value.get("data").and_then(|data| data.get("resources")) {
        None | Some(Value::Null) => Vec::new(),
        Some(list) => serde_json::from_value::<Vec<String>>(list.clone()).map_err(|source| {
            EventError::Malformed {
                event: EVENT.to_owned(),
                field: "resources",
                source,
            }
        })?,
    };
    Ok(Asked {
        id: data_text(value, EVENT, "id")?,
        session: data_text(value, EVENT, "sessionID")?,
        action: data_text(value, EVENT, "action")?,
        resources,
    })
}
