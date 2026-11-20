//! Typed MCP tool schemas and PermissionGate-backed dispatcher.
//!
//! Exposes the Native Browser Co-Pilot tools:
//! - `browser_list_tabs` (`Read`)
//! - `browser_read_tab` (`Read`)
//! - `browser_click` (`Write`)
//! - `browser_type` (`Write`)
//! - `browser_navigate` (`Write`)
//! - `browser_guide_highlight` (`Write`)

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

use crate::policy::{
    ActionClass, ApprovalOutcome, BrowserInboxAsk, Decision, GateCheck, PermissionGate,
    PolicyError,
};
use crate::protocol::{
    BrowserAction, BrowserReply, BrowserReplyPayload, BrowserRequest, GuideHighlight,
    InteractiveElement, PageSnapshot, TabInfo,
};

/// JSON Schema description of an MCP browser co-pilot tool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpToolSchema {
    pub name: String,
    pub description: String,
    pub action_class: ActionClass,
    pub input_schema: Value,
}

/// Returns the typed MCP tool schemas exposed by the Native Browser Co-Pilot.
pub fn tool_schemas() -> Vec<McpToolSchema> {
    vec![
        McpToolSchema {
            name: "browser_list_tabs".to_string(),
            description: "List open tabs in the user's native desktop browser.".to_string(),
            action_class: ActionClass::Read,
            input_schema: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
        McpToolSchema {
            name: "browser_read_tab".to_string(),
            description: "Read a structured accessibility and visible-text snapshot of a browser tab.".to_string(),
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
            name: "browser_click".to_string(),
            description: "Click an element on a browser tab (requires per-action Agent Inbox approval).".to_string(),
            action_class: ActionClass::Write,
            input_schema: json!({
                "type": "object",
                "required": ["url", "selector"],
                "properties": {
                    "tab_id": { "type": "integer", "minimum": 0 },
                    "url": { "type": "string" },
                    "selector": { "type": "string" },
                    "element_preview": { "type": "string" }
                },
                "additionalProperties": false
            }),
        },
        McpToolSchema {
            name: "browser_type".to_string(),
            description: "Type text into an input element on a browser tab (requires per-action Agent Inbox approval).".to_string(),
            action_class: ActionClass::Write,
            input_schema: json!({
                "type": "object",
                "required": ["url", "selector", "text"],
                "properties": {
                    "tab_id": { "type": "integer", "minimum": 0 },
                    "url": { "type": "string" },
                    "selector": { "type": "string" },
                    "text": { "type": "string" },
                    "clear_first": { "type": "boolean" },
                    "element_preview": { "type": "string" }
                },
                "additionalProperties": false
            }),
        },
        McpToolSchema {
            name: "browser_navigate".to_string(),
            description: "Navigate a browser tab to a target URL (requires per-action Agent Inbox approval).".to_string(),
            action_class: ActionClass::Write,
            input_schema: json!({
                "type": "object",
                "required": ["url"],
                "properties": {
                    "tab_id": { "type": "integer", "minimum": 0 },
                    "url": { "type": "string" }
                },
                "additionalProperties": false
            }),
        },
        McpToolSchema {
            name: "browser_guide_highlight".to_string(),
            description: "Highlight an on-page element with a dismissible callout bubble to visually guide the user (requires per-action Agent Inbox approval).".to_string(),
            action_class: ActionClass::Write,
            input_schema: json!({
                "type": "object",
                "required": ["url", "selector", "title", "body"],
                "properties": {
                    "tab_id": { "type": "integer", "minimum": 0 },
                    "url": { "type": "string" },
                    "selector": { "type": "string" },
                    "title": { "type": "string" },
                    "body": { "type": "string" },
                    "step_label": { "type": "string" }
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
struct ClickArgs {
    #[serde(default)]
    tab_id: Option<u64>,
    url: String,
    selector: String,
    #[serde(default)]
    element_preview: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TypeArgs {
    #[serde(default)]
    tab_id: Option<u64>,
    url: String,
    selector: String,
    text: String,
    #[serde(default)]
    clear_first: bool,
    #[serde(default)]
    element_preview: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NavigateArgs {
    #[serde(default)]
    tab_id: Option<u64>,
    url: String,
}

#[derive(Debug, Deserialize)]
struct SubmitArgs {
    #[serde(default)]
    tab_id: Option<u64>,
    url: String,
    selector: String,
    #[serde(default)]
    element_preview: Option<String>,
}

#[derive(Debug, Deserialize)]
struct HighlightGuideArgs {
    #[serde(default)]
    tab_id: Option<u64>,
    url: String,
    selector: String,
    title: String,
    body: String,
    #[serde(default)]
    step_label: Option<String>,
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
        "browser_click" => {
            let args: ClickArgs = serde_json::from_value(arguments).map_err(parse_err)?;
            Ok(BrowserAction::Click {
                tab_id: args.tab_id,
                url: args.url,
                selector: args.selector,
                element_preview: args.element_preview,
            })
        }
        "browser_type" => {
            let args: TypeArgs = serde_json::from_value(arguments).map_err(parse_err)?;
            Ok(BrowserAction::Type {
                tab_id: args.tab_id,
                url: args.url,
                selector: args.selector,
                text: args.text,
                clear_first: args.clear_first,
                element_preview: args.element_preview,
            })
        }
        "browser_navigate" => {
            let args: NavigateArgs = serde_json::from_value(arguments).map_err(parse_err)?;
            Ok(BrowserAction::Navigate {
                tab_id: args.tab_id,
                url: args.url,
            })
        }
        "browser_submit" => {
            let args: SubmitArgs = serde_json::from_value(arguments).map_err(parse_err)?;
            Ok(BrowserAction::Submit {
                tab_id: args.tab_id,
                url: args.url,
                selector: args.selector,
                element_preview: args.element_preview,
            })
        }
        "browser_guide_highlight" => {
            let args: HighlightGuideArgs = serde_json::from_value(arguments).map_err(parse_err)?;
            Ok(BrowserAction::HighlightGuide {
                tab_id: args.tab_id,
                url: args.url,
                selector: args.selector,
                title: args.title,
                body: args.body,
                step_label: args.step_label,
            })
        }
        other => Err(DispatchError::UnknownTool(other.to_string())),
    }
}

/// Dispatches MCP browser tool calls through `PermissionGate` and `BrowserTransport`.
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
    pub recorded_requests: Vec<BrowserRequest>,
    pub displayed_highlights: Vec<GuideHighlight>,
}

impl FakeBrowserTransport {
    /// Creates a fake transport seeded with one active tab and snapshot.
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
        let mut snapshots = HashMap::new();
        snapshots.insert(1, snapshot);
        Self {
            tabs: vec![tab],
            snapshots,
            recorded_requests: Vec::new(),
            displayed_highlights: Vec::new(),
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
            BrowserAction::Click {
                tab_id,
                url,
                selector,
                ..
            } => {
                let id = self.resolve_tab_id(tab_id)?;
                Ok(BrowserReply::ok(
                    req_id,
                    BrowserReplyPayload::ActionAck {
                        tab_id: id,
                        url,
                        detail: format!("clicked {selector}"),
                    },
                ))
            }
            BrowserAction::Type {
                tab_id,
                url,
                selector,
                text,
                ..
            } => {
                let id = self.resolve_tab_id(tab_id)?;
                Ok(BrowserReply::ok(
                    req_id,
                    BrowserReplyPayload::ActionAck {
                        tab_id: id,
                        url,
                        detail: format!("typed {} chars into {selector}", text.chars().count()),
                    },
                ))
            }
            BrowserAction::Navigate { tab_id, url } => {
                let id = self.resolve_tab_id(tab_id)?;
                if let Some(tab) = self.tabs.iter_mut().find(|t| t.tab_id == id) {
                    tab.url = url.clone();
                }
                Ok(BrowserReply::ok(
                    req_id,
                    BrowserReplyPayload::ActionAck {
                        tab_id: id,
                        url: url.clone(),
                        detail: format!("navigated to {url}"),
                    },
                ))
            }
            BrowserAction::Submit {
                tab_id,
                url,
                selector,
                ..
            } => {
                let id = self.resolve_tab_id(tab_id)?;
                Ok(BrowserReply::ok(
                    req_id,
                    BrowserReplyPayload::ActionAck {
                        tab_id: id,
                        url,
                        detail: format!("submitted {selector}"),
                    },
                ))
            }
            BrowserAction::HighlightGuide {
                tab_id,
                selector,
                title,
                body,
                step_label,
                ..
            } => {
                let id = self.resolve_tab_id(tab_id)?;
                let highlight = GuideHighlight {
                    tab_id: id,
                    selector,
                    title,
                    body,
                    step_label,
                    displayed: true,
                };
                self.displayed_highlights.push(highlight.clone());
                Ok(BrowserReply::ok(
                    req_id,
                    BrowserReplyPayload::Highlight { highlight },
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_schemas_cover_all_six_mcp_tools() {
        let schemas = tool_schemas();
        let names: Vec<&str> = schemas.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "browser_list_tabs",
                "browser_read_tab",
                "browser_click",
                "browser_type",
                "browser_navigate",
                "browser_guide_highlight",
            ]
        );
    }

    #[test]
    fn end_to_end_read_session_and_strict_write_approval_flow() {
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

        // 2. Follow-up Read tool succeeds immediately within the 30-min window.
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

        // 3. Write tool (`browser_guide_highlight`) MUST still ask for per-action approval.
        let guide_ask = match dispatcher
            .call_tool(
                "claude",
                "browser_guide_highlight",
                json!({
                    "tab_id": 1,
                    "url": "https://app.humata.example/projects",
                    "selector": "button#new-project",
                    "title": "Create your project",
                    "body": "Click this button to initialize a new workspace.",
                    "step_label": "Step 1"
                }),
                t0 + 61_000,
            )
            .unwrap()
        {
            DispatchOutcome::PendingApproval(ask) => ask,
            other => panic!("Write must require approval, got {other:?}"),
        };
        assert_eq!(guide_ask.action_class, ActionClass::Write);
        assert_eq!(
            guide_ask.allowed_decisions,
            vec![Decision::AllowOnce, Decision::Deny]
        );

        // AllowFor30Min is rejected on write.
        assert!(matches!(
            dispatcher.resolve_and_execute(
                &guide_ask.ask_id,
                Decision::AllowFor30Min,
                t0 + 61_500
            ),
            Err(DispatchError::Policy(
                PolicyError::WriteRequiresPerActionApproval
            ))
        ));

        // AllowOnce executes the visual guide highlight.
        let guide_exec = dispatcher
            .resolve_and_execute(&guide_ask.ask_id, Decision::AllowOnce, t0 + 62_000)
            .unwrap();
        assert!(matches!(
            guide_exec,
            DispatchOutcome::Executed(BrowserReply {
                ok: true,
                payload: Some(BrowserReplyPayload::Highlight { .. }),
                ..
            })
        ));
        assert_eq!(dispatcher.transport().displayed_highlights.len(), 1);

        // 4. Unsupported agent is rejected before touching transport.
        assert!(matches!(
            dispatcher.call_tool("unknown-agent", "browser_list_tabs", json!({}), t0 + 63_000),
            Err(DispatchError::Policy(PolicyError::UnsupportedAgent(_)))
        ));
    }
}
