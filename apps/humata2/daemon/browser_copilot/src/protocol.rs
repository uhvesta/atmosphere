//! WebExtension Native Messaging stdio protocol codec and typed messages.
//!
//! Implements the standard 4-byte native-endian length-prefixed JSON framing
//! specified by Chrome and Firefox Native Messaging, with a strict 1 MiB
//! payload bound. Strictly read-only: only `list_tabs`, `read_tab`, and
//! `screenshot` actions exist.

use std::io::{Read, Write};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Maximum allowed Native Messaging payload size (1 MiB, per browser spec).
pub const MAX_NATIVE_MESSAGE_BYTES: usize = 1024 * 1024;

/// Errors produced by the Native Messaging framing codec.
#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("native message size {size} exceeds 1 MiB limit ({max} bytes)")]
    MessageTooLarge { size: usize, max: usize },
    #[error("native message frame had zero length")]
    EmptyFrame,
    #[error("unexpected EOF while reading native message frame")]
    UnexpectedEof,
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON codec error: {0}")]
    Json(#[from] serde_json::Error),
    #[error(
        "browser action JSON is malformed or names an unknown or disallowed action type; expected a JSON object with \"type\" set to one of list_tabs, read_tab, screenshot: {0}"
    )]
    InvalidAction(#[source] serde_json::Error),
}

/// Encodes `message` as JSON and writes a 4-byte native-endian length-prefixed frame.
pub fn write_frame<W: Write, T: Serialize>(
    writer: &mut W,
    message: &T,
) -> Result<(), ProtocolError> {
    let payload = serde_json::to_vec(message)?;
    if payload.is_empty() {
        return Err(ProtocolError::EmptyFrame);
    }
    if payload.len() > MAX_NATIVE_MESSAGE_BYTES {
        return Err(ProtocolError::MessageTooLarge {
            size: payload.len(),
            max: MAX_NATIVE_MESSAGE_BYTES,
        });
    }
    let len = payload.len() as u32;
    writer.write_all(&len.to_ne_bytes())?;
    writer.write_all(&payload)?;
    writer.flush()?;
    Ok(())
}

/// Reads a 4-byte native-endian length-prefixed JSON frame, enforcing the 1 MiB guard.
pub fn read_frame<R: Read, T: DeserializeOwned>(reader: &mut R) -> Result<T, ProtocolError> {
    let mut len_buf = [0u8; 4];
    match reader.read_exact(&mut len_buf) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Err(ProtocolError::UnexpectedEof);
        }
        Err(err) => return Err(ProtocolError::Io(err)),
    }

    let len = u32::from_ne_bytes(len_buf) as usize;
    if len == 0 {
        return Err(ProtocolError::EmptyFrame);
    }
    if len > MAX_NATIVE_MESSAGE_BYTES {
        return Err(ProtocolError::MessageTooLarge {
            size: len,
            max: MAX_NATIVE_MESSAGE_BYTES,
        });
    }

    let mut payload = vec![0u8; len];
    match reader.read_exact(&mut payload) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Err(ProtocolError::UnexpectedEof);
        }
        Err(err) => return Err(ProtocolError::Io(err)),
    }

    Ok(serde_json::from_slice(&payload)?)
}

/// Request envelope sent from the native host to the browser extension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserRequest {
    pub request_id: String,
    pub action: BrowserAction,
}

/// Typed read-only browser actions supported by the Native Browser Co-Pilot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BrowserAction {
    ListTabs,
    ReadTab {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab_id: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_chars: Option<usize>,
    },
    Screenshot {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab_id: Option<u64>,
    },
}

impl BrowserAction {
    /// Exact target URL associated with this action, when applicable.
    pub fn target_url(&self) -> Option<&str> {
        match self {
            BrowserAction::ListTabs
            | BrowserAction::ReadTab { .. }
            | BrowserAction::Screenshot { .. } => None,
        }
    }

    /// Human-readable target preview for Agent Inbox approval prompts.
    pub fn target_preview(&self) -> Option<String> {
        match self {
            BrowserAction::ListTabs => None,
            BrowserAction::ReadTab { tab_id, .. } | BrowserAction::Screenshot { tab_id } => tab_id
                .map(|id| format!("tab #{id}"))
                .or_else(|| Some("active tab".to_string())),
        }
    }

    /// Concise summary of the action for Agent Inbox approval prompts and audit logs.
    pub fn action_summary(&self) -> String {
        match self {
            BrowserAction::ListTabs => "List open browser tabs".to_string(),
            BrowserAction::ReadTab {
                tab_id: Some(id), ..
            } => format!("Read page snapshot from tab #{id}"),
            BrowserAction::ReadTab { tab_id: None, .. } => {
                "Read page snapshot from active browser tab".to_string()
            }
            BrowserAction::Screenshot { tab_id: Some(id) } => {
                format!("Capture screenshot of tab #{id}")
            }
            BrowserAction::Screenshot { tab_id: None } => {
                "Capture screenshot of active browser tab".to_string()
            }
        }
    }

    /// Parses one read-only browser action from its wire JSON.
    ///
    /// Malformed JSON and any unknown or disallowed `"type"` tag (including
    /// `click`, `type`, `navigate`, `submit`, `highlight_guide`) return
    /// `ProtocolError::InvalidAction`.
    pub fn from_json(json: &str) -> Result<BrowserAction, ProtocolError> {
        serde_json::from_str(json).map_err(ProtocolError::InvalidAction)
    }

    /// Serde tag name of this action, as written in the wire JSON `"type"` field.
    pub fn kind(&self) -> &'static str {
        match self {
            BrowserAction::ListTabs => "list_tabs",
            BrowserAction::ReadTab { .. } => "read_tab",
            BrowserAction::Screenshot { .. } => "screenshot",
        }
    }

    /// Stable identity of this action: the lowercase hex SHA-256 of its canonical JSON.
    ///
    /// Equal actions produce equal keys. If the canonical JSON cannot be produced,
    /// the key falls back to hashing the `Debug` representation.
    pub fn action_key(&self) -> String {
        let input = match serde_json::to_vec(self) {
            Ok(bytes) => bytes,
            Err(_) => format!("{self:?}").into_bytes(),
        };
        Sha256::digest(&input)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}

/// Metadata describing an open browser tab.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TabInfo {
    pub tab_id: u64,
    pub window_id: u64,
    pub title: String,
    pub url: String,
    pub active: bool,
    pub pinned: bool,
}

/// Interactive element entry extracted by the read-only content script.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InteractiveElement {
    pub selector: String,
    pub tag: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_type: Option<String>,
    #[serde(default)]
    pub disabled: bool,
}

/// Accessibility-oriented read-only snapshot of a browser page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageSnapshot {
    pub tab_id: u64,
    pub url: String,
    pub title: String,
    pub visible_text: String,
    pub interactive_elements: Vec<InteractiveElement>,
}

/// Read-only screenshot of the visible area of a browser tab.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageScreenshot {
    pub tab_id: u64,
    pub window_id: u64,
    pub url: String,
    pub title: String,
    pub format: String,
    pub data_url: String,
}

/// Reply payload variants returned by the read-only browser extension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BrowserReplyPayload {
    Tabs { tabs: Vec<TabInfo> },
    Snapshot { snapshot: PageSnapshot },
    Screenshot { screenshot: PageScreenshot },
}

/// Reply envelope sent from the browser extension back to the native host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserReply {
    pub request_id: String,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<BrowserReplyPayload>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl BrowserReply {
    pub fn ok(request_id: impl Into<String>, payload: BrowserReplyPayload) -> Self {
        Self {
            request_id: request_id.into(),
            ok: true,
            payload: Some(payload),
            error: None,
        }
    }

    pub fn err(request_id: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            request_id: request_id.into(),
            ok: false,
            payload: None,
            error: Some(message.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn native_endian_framing_round_trips_request_and_reply() {
        let req = BrowserRequest {
            request_id: "req-42".to_string(),
            action: BrowserAction::Screenshot { tab_id: Some(7) },
        };

        let mut wire = Vec::new();
        write_frame(&mut wire, &req).expect("write request");

        // Verify 4-byte native-endian prefix matches exact payload byte count.
        let expected_len = (wire.len() - 4) as u32;
        assert_eq!(&wire[..4], &expected_len.to_ne_bytes());

        let mut cursor = Cursor::new(wire);
        let decoded: BrowserRequest = read_frame(&mut cursor).expect("read request");
        assert_eq!(decoded, req);

        let reply = BrowserReply::ok(
            "req-42",
            BrowserReplyPayload::Screenshot {
                screenshot: PageScreenshot {
                    tab_id: 7,
                    window_id: 100,
                    url: "https://console.example.com/billing".to_string(),
                    title: "Billing".to_string(),
                    format: "png".to_string(),
                    data_url: "data:image/png;base64,iVBORw0KGgo=".to_string(),
                },
            },
        );
        let mut reply_wire = Vec::new();
        write_frame(&mut reply_wire, &reply).expect("write reply");
        let decoded_reply: BrowserReply =
            read_frame(&mut Cursor::new(reply_wire)).expect("read reply");
        assert_eq!(decoded_reply, reply);
    }

    #[test]
    fn rejects_oversize_frame_on_write_and_read() {
        let huge = BrowserReply::ok(
            "req-big",
            BrowserReplyPayload::Screenshot {
                screenshot: PageScreenshot {
                    tab_id: 1,
                    window_id: 1,
                    url: "https://example.com".to_string(),
                    title: "Huge".to_string(),
                    format: "png".to_string(),
                    data_url: "x".repeat(MAX_NATIVE_MESSAGE_BYTES + 64),
                },
            },
        );
        let mut out = Vec::new();
        assert!(matches!(
            write_frame(&mut out, &huge),
            Err(ProtocolError::MessageTooLarge { .. })
        ));

        // Craft a wire header claiming 1 MiB + 1 byte without allocating that payload.
        let oversize_len = (MAX_NATIVE_MESSAGE_BYTES as u32) + 1;
        let mut bad_wire = Vec::from(oversize_len.to_ne_bytes());
        bad_wire.extend_from_slice(b"{}");
        let err = read_frame::<_, BrowserRequest>(&mut Cursor::new(bad_wire)).unwrap_err();
        assert!(matches!(
            err,
            ProtocolError::MessageTooLarge {
                size,
                max: MAX_NATIVE_MESSAGE_BYTES
            } if size == MAX_NATIVE_MESSAGE_BYTES + 1
        ));
    }

    #[test]
    fn rejects_zero_length_and_truncated_frames() {
        let zero = 0u32.to_ne_bytes();
        assert!(matches!(
            read_frame::<_, BrowserRequest>(&mut Cursor::new(zero)),
            Err(ProtocolError::EmptyFrame)
        ));

        let mut truncated = Vec::from(100u32.to_ne_bytes());
        truncated.extend_from_slice(b"{\"partial\":true");
        assert!(matches!(
            read_frame::<_, BrowserRequest>(&mut Cursor::new(truncated)),
            Err(ProtocolError::UnexpectedEof)
        ));
    }

    fn one_of_each_action() -> Vec<BrowserAction> {
        vec![
            BrowserAction::ListTabs,
            BrowserAction::ReadTab {
                tab_id: Some(3),
                max_chars: Some(200),
            },
            BrowserAction::Screenshot { tab_id: Some(3) },
        ]
    }

    #[test]
    fn equal_actions_have_equal_action_keys() {
        let a = BrowserAction::ReadTab {
            tab_id: Some(1),
            max_chars: Some(500),
        };
        let b = BrowserAction::ReadTab {
            tab_id: Some(1),
            max_chars: Some(500),
        };
        assert_eq!(a.action_key(), b.action_key());
    }

    #[test]
    fn changing_tab_or_max_chars_changes_action_key() {
        let base = BrowserAction::ReadTab {
            tab_id: Some(1),
            max_chars: Some(500),
        };
        let diff_tab = BrowserAction::ReadTab {
            tab_id: Some(2),
            max_chars: Some(500),
        };
        let diff_chars = BrowserAction::ReadTab {
            tab_id: Some(1),
            max_chars: Some(1000),
        };
        let screenshot = BrowserAction::Screenshot { tab_id: Some(1) };

        assert_ne!(base.action_key(), diff_tab.action_key());
        assert_ne!(base.action_key(), diff_chars.action_key());
        assert_ne!(base.action_key(), screenshot.action_key());
    }

    #[test]
    fn action_key_is_64_lowercase_hex_characters() {
        for action in one_of_each_action() {
            let key = action.action_key();
            assert_eq!(key.len(), 64);
            assert!(
                key.chars()
                    .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
            );
        }
    }

    #[test]
    fn from_json_round_trips_every_read_only_action() {
        for action in one_of_each_action() {
            let wire = serde_json::to_string(&action).expect("serialize action");
            let parsed = BrowserAction::from_json(&wire).expect("parse action");
            assert_eq!(parsed, action);
        }
    }

    #[test]
    fn from_json_rejects_malformed_unknown_and_execute_actions() {
        let malformed = [r#"{"type":"read_tab","tab_id":"#, "not json", ""];
        for input in malformed {
            assert!(matches!(
                BrowserAction::from_json(input),
                Err(ProtocolError::InvalidAction(_))
            ));
        }

        let disallowed = [
            r#"{"type":"click","url":"https://example.com","selector":"button"}"#,
            r#"{"type":"type","url":"https://example.com","selector":"input","text":"hi"}"#,
            r#"{"type":"navigate","url":"https://example.com"}"#,
            r#"{"type":"submit","url":"https://example.com","selector":"form"}"#,
            r#"{"type":"highlight_guide","url":"https://example.com","selector":"a","title":"t","body":"b"}"#,
            r#"{"type":"execute_script","code":"alert(1)"}"#,
            r#"{"type":"explode"}"#,
            r#"{"url":"https://example.com"}"#,
        ];
        for input in disallowed {
            assert!(
                matches!(
                    BrowserAction::from_json(input),
                    Err(ProtocolError::InvalidAction(_))
                ),
                "disallowed payload must fail deserialization: {input}"
            );
        }
    }

    #[test]
    fn kind_matches_json_type_tag() {
        for action in one_of_each_action() {
            let value = serde_json::to_value(&action).expect("serialize action to value");
            assert_eq!(
                value.get("type").and_then(serde_json::Value::as_str),
                Some(action.kind())
            );
        }
    }
}
