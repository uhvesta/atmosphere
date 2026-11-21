//! Typed MCP tool schemas and PermissionGate-backed dispatcher.
//!
//! Exposes ONLY the 3 strictly read-only Native Browser Co-Pilot tools:
//! - `browser_list_tabs` (`Read`)
//! - `browser_read_tab` (`Read`)
//! - `browser_screenshot` (`Read`)

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

use crate::policy::{
    ActionClass, ApprovalOutcome, BrowserInboxAsk, Decision, GateCheck, PermissionGate,
    PolicyError,
};
use crate::protocol::{
    BrowserAction, BrowserReply, BrowserReplyPayload, BrowserRequest, InteractiveElement,
    PageScreenshot, PageSnapshot, TabInfo,
};

/// JSON Schema description of an MCP browser co-pilot tool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpToolSchema {
    pub name: String,
    pub description: String,
    pub action_class: ActionClass,
    pub input_schema: Value,
}

/// Returns the typed read-only MCP tool schemas exposed by the Native Browser Co-Pilot.
pub fn tool_schemas() -> Vec<McpToolSchema> {
    vec![
        McpToolSchema {
            name: "browser_list_tabs".to_string(),
            description: "List open tabs in the user's native desktop browser (read-only)."
                .to_string(),
            action_class: ActionClass::Read,
            input_schema: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
        McpToolSchema {
            name: "browser_read_tab".to_string(),
            description:
                "Read a structured accessibility and visible-text snapshot of a browser tab (read-only)."
                    .to_string(),
            action_class: ActionClass::Read,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "tab_id": { "type": "integer", "minimum": 0 },
                    "max_chars": { "type": "integer", "minimum": 1 }
                },
                "additionalProperties": false
            }),
        },
        McpToolSchema {
            name: "browser_screenshot".to_string(),
            description:
                "Capture a read-only PNG screenshot of the visible area of a browser tab."
                    .to_string(),
            action_class: ActionClass::Read,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "tab_id": { "type": "integer", "minimum": 0 }
                },
                "additionalProperties": false
            }),
        },
    ]
}

/// Transport errors returned when communicating with the browser extension.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TransportError {
    #[error("browser transport disconnected: {0}")]
    Disconnected(String),
    #[error("tab not found: {0}")]
    TabNotFound(u64),
    #[error("no active tab available")]
    NoActiveTab,
}

/// Abstraction over the Native Messaging link to the browser extension.
pub trait BrowserTransport {
    fn send(&mut self, request: BrowserRequest) -> Result<BrowserReply, TransportError>;
}

/// Errors returned by `McpDispatcher`.
#[derive(Debug, Error)]
pub enum DispatchError {
    #[error("unknown MCP browser tool `{0}`")]
    UnknownTool(String),
    #[error("invalid tool arguments for `{tool}`: {reason}")]
    InvalidArguments { tool: String, reason: String },
    #[error(transparent)]
    Policy(#[from] PolicyError),
    #[error(transparent)]
    Transport(#[from] TransportError),
}

/// Outcome of calling or resolving an MCP browser tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchOutcome {
    /// Permission gate requires user approval in the Agent Inbox before executing.
    PendingApproval(BrowserInboxAsk),
    /// User denied the pending approval ask.
    Denied { ask_id: String },
    /// Tool executed against the browser transport and returned a reply.
    Executed(BrowserReply),
}

#[derive(Debug, Deserialize)]
struct ReadTabArgs {
    #[serde(default)]
    tab_id: Option<u64>,
    #[serde(default)]
    max_chars: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ScreenshotArgs {
    #[serde(default)]
    tab_id: Option<u64>,
}

fn parse_tool_action(tool_name: &str, arguments: Value) -> Result<BrowserAction, DispatchError> {
    let parse_err = |err: serde_json::Error| DispatchError::InvalidArguments {
        tool: tool_name.to_string(),
        reason: err.to_string(),
    };

    match tool_name {
        "browser_list_tabs" => Ok(BrowserAction::ListTabs),
        "browser_read_tab" => {
            let args: ReadTabArgs = if arguments.is_null() {
                ReadTabArgs {
                    tab_id: None,
                    max_chars: None,
                }
            } else {
                serde_json::from_value(arguments).map_err(parse_err)?
            };
            Ok(BrowserAction::ReadTab {
                tab_id: args.tab_id,
                max_chars: args.max_chars,
            })
        }
        "browser_screenshot" => {
            let args: ScreenshotArgs = if arguments.is_null() {
                ScreenshotArgs { tab_id: None }
            } else {
                serde_json::from_value(arguments).map_err(parse_err)?
            };
            Ok(BrowserAction::Screenshot {
                tab_id: args.tab_id,
            })
        }
        other => Err(DispatchError::UnknownTool(other.to_string())),
    }
}

/// Dispatches read-only MCP browser tool calls through `PermissionGate` and `BrowserTransport`.
pub struct McpDispatcher<T: BrowserTransport> {
    gate: PermissionGate,
    transport: T,
    pending_actions: HashMap<String, BrowserAction>,
    next_request_seq: u64,
}

impl<T: BrowserTransport> McpDispatcher<T> {
    pub fn new(transport: T) -> Self {
        Self {
            gate: PermissionGate::new(),
            transport,
            pending_actions: HashMap::new(),
            next_request_seq: 0,
        }
    }

    pub fn gate(&self) -> &PermissionGate {
        &self.gate
    }

    pub fn gate_mut(&mut self) -> &mut PermissionGate {
        &mut self.gate
    }

    pub fn transport(&self) -> &T {
        &self.transport
    }

    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }

    /// Parses and checks an MCP tool call; executes immediately if allowed,
    /// otherwise returns `DispatchOutcome::PendingApproval`.
    pub fn call_tool(
        &mut self,
        agent: &str,
        tool_name: &str,
        arguments: Value,
        now_ms: u64,
    ) -> Result<DispatchOutcome, DispatchError> {
        let action = parse_tool_action(tool_name, arguments)?;
        match self.gate.check(agent, &action, now_ms)? {
            GateCheck::Allowed => self.execute(action),
            GateCheck::NeedsApproval(ask) => {
                self.pending_actions.insert(ask.ask_id.clone(), action);
                Ok(DispatchOutcome::PendingApproval(ask))
            }
        }
    }

    /// Resolves a pending Agent Inbox ask and executes the queued browser action if approved.
    pub fn resolve_and_execute(
        &mut self,
        ask_id: &str,
        decision: Decision,
        now_ms: u64,
    ) -> Result<DispatchOutcome, DispatchError> {
        let outcome = self.gate.resolve_ask(ask_id, decision, now_ms)?;
        let action = self
            .pending_actions
            .remove(ask_id)
            .ok_or_else(|| PolicyError::UnknownAskId(ask_id.to_string()))?;

        match outcome {
            ApprovalOutcome::Denied => Ok(DispatchOutcome::Denied {
                ask_id: ask_id.to_string(),
            }),
            ApprovalOutcome::ApprovedOnce | ApprovalOutcome::ApprovedWithSessionGrant { .. } => {
                self.execute(action)
            }
        }
    }

    fn execute(&mut self, action: BrowserAction) -> Result<DispatchOutcome, DispatchError> {
        self.next_request_seq += 1;
        let request = BrowserRequest {
            request_id: format!("req-{}", self.next_request_seq),
            action,
        };
        let reply = self.transport.send(request)?;
        Ok(DispatchOutcome::Executed(reply))
    }
}

/// In-memory `BrowserTransport` implementation for deterministic end-to-end tests.
#[derive(Debug, Default)]
pub struct FakeBrowserTransport {
    pub tabs: Vec<TabInfo>,
    pub snapshots: HashMap<u64, PageSnapshot>,
    pub screenshots: HashMap<u64, PageScreenshot>,
    pub recorded_requests: Vec<BrowserRequest>,
}

impl FakeBrowserTransport {
    /// Creates a fake transport seeded with one active tab, snapshot, and screenshot.
    pub fn with_sample_tab() -> Self {
        let tab = TabInfo {
            tab_id: 1,
            window_id: 100,
            title: "Humata Dashboard".to_string(),
            url: "https://app.humata.example/projects".to_string(),
            active: true,
            pinned: false,
        };
        let snapshot = PageSnapshot {
            tab_id: 1,
            url: tab.url.clone(),
            title: tab.title.clone(),
            visible_text: "Projects overview. Click New Project to begin.".to_string(),
            interactive_elements: vec![InteractiveElement {
                selector: "button#new-project".to_string(),
                tag: "button".to_string(),
                role: Some("button".to_string()),
                label: "New Project".to_string(),
                input_type: None,
                disabled: false,
            }],
        };
        let screenshot = PageScreenshot {
            tab_id: 1,
            window_id: 100,
            url: tab.url.clone(),
            title: tab.title.clone(),
            format: "png".to_string(),
            data_url: "data:image/png;base64,iVBORw0KGgo=".to_string(),
        };
        let mut snapshots = HashMap::new();
        snapshots.insert(1, snapshot);
        let mut screenshots = HashMap::new();
        screenshots.insert(1, screenshot);
        Self {
            tabs: vec![tab],
            snapshots,
            screenshots,
            recorded_requests: Vec::new(),
        }
    }

    fn resolve_tab_id(&self, requested: Option<u64>) -> Result<u64, TransportError> {
        if let Some(id) = requested {
            if self.tabs.iter().any(|t| t.tab_id == id) {
                return Ok(id);
            }
            return Err(TransportError::TabNotFound(id));
        }
        self.tabs
            .iter()
            .find(|t| t.active)
            .or_else(|| self.tabs.first())
            .map(|t| t.tab_id)
            .ok_or(TransportError::NoActiveTab)
    }
}

impl BrowserTransport for FakeBrowserTransport {
    fn send(&mut self, request: BrowserRequest) -> Result<BrowserReply, TransportError> {
        self.recorded_requests.push(request.clone());
        let req_id = request.request_id;

        match request.action {
            BrowserAction::ListTabs => Ok(BrowserReply::ok(
                req_id,
                BrowserReplyPayload::Tabs {
                    tabs: self.tabs.clone(),
                },
            )),
            BrowserAction::ReadTab { tab_id, max_chars } => {
                let id = self.resolve_tab_id(tab_id)?;
                let mut snap = self
                    .snapshots
                    .get(&id)
                    .cloned()
                    .ok_or(TransportError::TabNotFound(id))?;
                if let Some(limit) = max_chars {
                    snap.visible_text = snap.visible_text.chars().take(limit).collect();
                }
                Ok(BrowserReply::ok(
                    req_id,
                    BrowserReplyPayload::Snapshot { snapshot: snap },
                ))
            }
            BrowserAction::Screenshot { tab_id } => {
                let id = self.resolve_tab_id(tab_id)?;
                let screenshot = self
                    .screenshots
                    .get(&id)
                    .cloned()
                    .ok_or(TransportError::TabNotFound(id))?;
                Ok(BrowserReply::ok(
                    req_id,
                    BrowserReplyPayload::Screenshot { screenshot },
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_schemas_expose_only_three_read_only_tools() {
        let schemas = tool_schemas();
        let names: Vec<&str> = schemas.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["browser_list_tabs", "browser_read_tab", "browser_screenshot"]
        );
        assert!(schemas.iter().all(|s| s.action_class == ActionClass::Read));
    }

    #[test]
    fn end_to_end_read_only_flow_and_rejection_of_execute_tools() {
        let mut dispatcher = McpDispatcher::new(FakeBrowserTransport::with_sample_tab());
        let t0 = 100_000;

        // 1. Read tool asks for approval; grant 30-min session.
        let ask = match dispatcher
            .call_tool("claude", "browser_list_tabs", json!({}), t0)
            .unwrap()
        {
            DispatchOutcome::PendingApproval(ask) => ask,
            other => panic!("expected PendingApproval, got {other:?}"),
        };
        let executed = dispatcher
            .resolve_and_execute(&ask.ask_id, Decision::AllowFor30Min, t0)
            .unwrap();
        assert!(matches!(
            executed,
            DispatchOutcome::Executed(BrowserReply {
                ok: true,
                payload: Some(BrowserReplyPayload::Tabs { .. }),
                ..
            })
        ));

        // 2. Follow-up ReadTab succeeds immediately within the 30-min window.
        let snap_outcome = dispatcher
            .call_tool(
                "claude",
                "browser_read_tab",
                json!({ "tab_id": 1, "max_chars": 20 }),
                t0 + 60_000,
            )
            .unwrap();
        assert!(matches!(
            snap_outcome,
            DispatchOutcome::Executed(BrowserReply {
                ok: true,
                payload: Some(BrowserReplyPayload::Snapshot { .. }),
                ..
            })
        ));

        // 3. Follow-up Screenshot succeeds immediately within the 30-min window.
        let shot_outcome = dispatcher
            .call_tool("claude", "browser_screenshot", json!({ "tab_id": 1 }), t0 + 61_000)
            .unwrap();
        assert!(matches!(
            shot_outcome,
            DispatchOutcome::Executed(BrowserReply {
                ok: true,
                payload: Some(BrowserReplyPayload::Screenshot { .. }),
                ..
            })
        ));

        // 4. All execute/write/mutate tools are rejected as UnknownTool.
        for disallowed in [
            "browser_click",
            "browser_type",
            "browser_navigate",
            "browser_submit",
            "browser_guide_highlight",
        ] {
            assert!(
                matches!(
                    dispatcher.call_tool("claude", disallowed, json!({}), t0 + 62_000),
                    Err(DispatchError::UnknownTool(_))
                ),
                "{disallowed} must be rejected"
            );
        }

        // 5. Unsupported agent is rejected before touching transport.
        assert!(matches!(
            dispatcher.call_tool("unknown-agent", "browser_list_tabs", json!({}), t0 + 63_000),
            Err(DispatchError::Policy(PolicyError::UnsupportedAgent(_)))
        ));
    }
}
