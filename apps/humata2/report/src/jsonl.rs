//! `events.jsonl`: a report as one JSON object a line, each with a `kind`.
//!
//! The first line is the `context` (with the schema version and whether
//! it is a recording or a snapshot), the second the `start` trees, then
//! the events in order, and last the `end` trees.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::ReadError;
use crate::event::{Context, Event, Kind, Trees};
use crate::recording::Captured;

/// The version of the format this crate writes and reads.
pub(crate) const SCHEMA: u32 = 1;

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Frame {
    Context {
        schema: u32,
        report: Kind,
        context: Context,
    },
    Start {
        at_ms: u64,
        #[serde(default)]
        dropped: usize,
        state: String,
        view: String,
        layout: String,
    },
    End {
        at_ms: u64,
        state: String,
        view: String,
        layout: String,
    },
}

/// `captured` as `events.jsonl`.
pub(crate) fn write(captured: &Captured) -> String {
    let mut out = String::new();
    let mut line = |value: Result<String, serde_json::Error>| {
        // Every field is a string, a number or a list of them, so this
        // can't fail; a line that somehow did is left out rather than
        // losing the report.
        if let Ok(text) = value {
            out.push_str(&text);
            out.push('\n');
        }
    };
    line(serde_json::to_string(&Frame::Context {
        schema: SCHEMA,
        report: captured.kind,
        context: captured.context.clone(),
    }));
    line(serde_json::to_string(&Frame::Start {
        at_ms: captured.start_ms,
        dropped: captured.dropped,
        state: captured.start.state.clone(),
        view: captured.start.view.clone(),
        layout: captured.start.layout.clone(),
    }));
    for event in &captured.events {
        line(serde_json::to_string(event));
    }
    line(serde_json::to_string(&Frame::End {
        at_ms: captured.end_ms,
        state: captured.end.state.clone(),
        view: captured.end.view.clone(),
        layout: captured.end.layout.clone(),
    }));
    out
}

/// Reads a report back from the text of its `events.jsonl`, to replay it
/// or turn it into a flow test.
///
/// # Errors
///
/// [`ReadError`] when a line isn't JSON or isn't an event, the schema is
/// newer than this build reads, or the context, start or end line is
/// missing or out of place.
pub fn read_events(text: &str) -> Result<Captured, ReadError> {
    let mut context: Option<(Kind, Context)> = None;
    let mut start: Option<(u64, usize, Trees)> = None;
    let mut events = Vec::new();
    let mut end: Option<(u64, Trees)> = None;

    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        if raw.trim().is_empty() {
            continue;
        }
        let value: Value =
            serde_json::from_str(raw).map_err(|source| ReadError::Json { line, source })?;
        let is_frame = matches!(
            value.get("kind").and_then(Value::as_str),
            Some("context" | "start" | "end")
        );
        let out_of_place = |what: &'static str| ReadError::OutOfPlace { line, what };
        if end.is_some() {
            return Err(out_of_place("it comes after the end line"));
        }
        if is_frame {
            let frame: Frame =
                serde_json::from_value(value).map_err(|source| ReadError::Json { line, source })?;
            match frame {
                Frame::Context {
                    schema,
                    report,
                    context: read,
                } => {
                    if context.is_some() || start.is_some() {
                        return Err(out_of_place("the context line must come first, once"));
                    }
                    if schema > SCHEMA {
                        return Err(ReadError::Schema {
                            found: schema,
                            supported: SCHEMA,
                        });
                    }
                    context = Some((report, read));
                }
                Frame::Start {
                    at_ms,
                    dropped,
                    state,
                    view,
                    layout,
                } => {
                    if context.is_none() || start.is_some() {
                        return Err(out_of_place("the start line must come once, right after the context line"));
                    }
                    start = Some((at_ms, dropped, Trees { state, view, layout }));
                }
                Frame::End {
                    at_ms,
                    state,
                    view,
                    layout,
                } => {
                    if start.is_none() {
                        return Err(out_of_place("the end line comes before the start line"));
                    }
                    end = Some((at_ms, Trees { state, view, layout }));
                }
            }
        } else {
            if start.is_none() {
                return Err(out_of_place("an event comes before the start line"));
            }
            let event: Event =
                serde_json::from_value(value).map_err(|source| ReadError::Json { line, source })?;
            events.push(event);
        }
    }

    let (kind, context) = context.ok_or(ReadError::Missing { what: "context" })?;
    let (start_ms, dropped, start) = start.ok_or(ReadError::Missing { what: "start" })?;
    let (end_ms, end) = end.ok_or(ReadError::Missing { what: "end" })?;
    Ok(Captured {
        kind,
        context,
        start_ms,
        dropped,
        start,
        events,
        end_ms,
        end,
    })
}
