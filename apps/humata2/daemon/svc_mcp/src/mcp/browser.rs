//! BROWSER (B2): MCP tool handlers for the 6 Native Browser Co-Pilot tools
//! (`browser_list_tabs`, `browser_read_tab`, `browser_guide_highlight`,
//! `browser_click`, `browser_type`, `browser_navigate`).
//!
//! Strictly gated on `SupportedAgent::parse(agent_id)` (`claude`, `codex`,
//! `copilot`, `agy` only). Every call checks `GetBrowserBridgeStatus`,
//! requests approval via `RequestBrowserApproval` + `WaitForAnswer` (with
//! auto-withdraw on drop/cancel), and then runs `ExecuteBrowserAction`.

use std::sync::atomic::Ordering;

use humata2_browser_copilot::{BrowserAction, SupportedAgent};
use humata2_grpc::humata::v1 as pb;
use humata2_mcp::{Call, ErrorKind, Operation, Reply, RequestId, SessionToken, ToolError};
use serde_json::{Map, Value, json};
use tonic::Request;
use tonic::metadata::{AsciiMetadataValue, MetadataValue};
use tonic::transport::Channel;

use crate::ports::TOKEN_HEADER;

use super::{GrpcDaemon, applied, key, lock, outcome, text};

/// Default approval wait when the tool call doesn't specify `wait_seconds`.
const DEFAULT_WAIT_SECONDS: u32 = 45;
const MAX_WAIT_SECONDS: u32 = 600;

/// Returns `true` for the 6 browser MCP operations.
pub(super) const fn is_browser_op(operation: Operation) -> bool {
    matches!(
        operation,
        Operation::BrowserListTabs
            | Operation::BrowserReadTab
            | Operation::BrowserGuideHighlight
            | Operation::BrowserClick
            | Operation::BrowserType
            | Operation::BrowserNavigate
    )
}

impl GrpcDaemon {
    /// Learns from `WhoAmI` whether this session's agent is in the supported
    /// 4-agent set (`claude`, `codex`, `copilot`, `agy`).
    pub(super) fn learn_browser_agent(&self, agent_id: &str) {
        let supported = SupportedAgent::parse(agent_id).is_ok();
        self.browser_offered.store(supported, Ordering::Relaxed);
    }

    /// Whether `tools/list` and `call()` offer the 6 `browser_*` tools.
    pub(super) fn browser_offered(&self) -> bool {
        self.browser_offered.load(Ordering::Relaxed)
    }

    pub(super) fn browser(
        &self,
    ) -> pb::browser_bridge_service_client::BrowserBridgeServiceClient<Channel> {
        pb::browser_bridge_service_client::BrowserBridgeServiceClient::new(self.channel.clone())
    }

    pub(super) async fn browser_call(&self, call: Call) -> Result<Reply, ToolError> {
        if !self.browser_offered() {
            return Err(ToolError {
                kind: ErrorKind::Denied,
                message: format!(
                    "{} is only available to claude, codex, copilot and agy sessions",
                    call.operation.tool_name()
                ),
                fix: "run this task in a claude, codex, copilot or agy session to use the browser tools"
                    .to_owned(),
                retry: None,
                details: Map::new(),
            });
        }

        let action = browser_action(call.operation, &call.arguments)?;
        let action_json = serde_json::to_string(&action).map_err(|error| ToolError {
            kind: ErrorKind::Internal,
            message: format!("couldn't encode browser action: {error}"),
            fix: "check the tool arguments and try again".to_owned(),
            retry: None,
            details: Map::new(),
        })?;
        let reason = text(&call.arguments, "reason");
        let wait_seconds = call
            .arguments
            .get("wait_seconds")
            .and_then(Value::as_u64)
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(DEFAULT_WAIT_SECONDS)
            .clamp(1, MAX_WAIT_SECONDS);

        // 1. Check that a browser native host is connected before raising an Inbox card.
        let status = self
            .browser()
            .get_browser_bridge_status(self.request(pb::GetBrowserBridgeStatusRequest {})?)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        if !status.connected {
            return Err(browser_disconnected_error());
        }

        // 2. Ask the Inbox for approval (or reuse an active 30-min read grant).
        let waited = self
            .inbox()
            .request_browser_approval(self.request(pb::RequestBrowserApprovalRequest {
                idempotency_key: Some(key(&call)),
                action_json: action_json.clone(),
                reason,
                wait_seconds,
            })?)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();

        let approval_id = match pb::RequestState::try_from(waited.state) {
            Ok(pb::RequestState::Approved) => waited.id,
            Ok(pb::RequestState::Pending) => {
                let Some(req_id) = waited.id else {
                    return Err(ToolError {
                        kind: ErrorKind::Internal,
                        message: "the daemon returned a pending browser approval without a request id"
                            .to_owned(),
                        fix: "try the call again".to_owned(),
                        retry: None,
                        details: Map::new(),
                    });
                };
                let mut guard = WithdrawOnDrop::new(
                    self.channel.clone(),
                    lock(&self.token).clone(),
                    self.bundle.clone(),
                    req_id.value.clone(),
                );
                let mut stream = self
                    .inbox()
                    .wait_for_answer(self.request(pb::WaitForAnswerRequest {
                        id: Some(req_id.clone()),
                        wait_seconds,
                    })?)
                    .await
                    .map_err(|status| self.failed(&status))?
                    .into_inner();
                let mut final_response = None;
                while let Some(msg) = stream
                    .message()
                    .await
                    .map_err(|status| self.failed(&status))?
                {
                    if msg.keepalive {
                        continue;
                    }
                    final_response = Some(msg);
                    break;
                }
                guard.disarm();
                let Some(response) = final_response else {
                    return Ok(Reply::Pending {
                        request: RequestId::new(req_id.value),
                    });
                };
                match pb::RequestState::try_from(response.state) {
                    Ok(pb::RequestState::Approved) if response.error.is_none() => {
                        response.id.or(Some(req_id))
                    }
                    _ => return outcome(response),
                }
            }
            _ => return outcome(waited),
        };

        // 3. Execute the approved action through BrowserBridgeService.
        let executed = self
            .browser()
            .execute_browser_action(self.request(pb::ExecuteBrowserActionRequest {
                approval: approval_id.clone(),
                action_json,
            })?)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();

        let result: Value = serde_json::from_str(&executed.reply_json)
            .unwrap_or_else(|_| json!({ "raw": executed.reply_json }));
        Ok(applied(result, approval_id.map(|id| RequestId::new(id.value))))
    }
}

/// Auto-withdraws a pending browser approval card if the MCP tool call is
/// aborted/dropped (e.g. client cancelled or MCP stdio closed) before settled.
struct WithdrawOnDrop {
    channel: Channel,
    token: Option<SessionToken>,
    bundle: Option<AsciiMetadataValue>,
    request_id: String,
    armed: bool,
}

impl WithdrawOnDrop {
    fn new(
        channel: Channel,
        token: Option<SessionToken>,
        bundle: Option<AsciiMetadataValue>,
        request_id: String,
    ) -> Self {
        Self {
            channel,
            token,
            bundle,
            request_id,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for WithdrawOnDrop {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let channel = self.channel.clone();
        let token = self.token.clone();
        let bundle = self.bundle.clone();
        let request_id = std::mem::take(&mut self.request_id);
        handle.spawn(async move {
            let mut req = Request::new(pb::WithdrawRequestRequest {
                idempotency_key: None,
                id: Some(pb::RequestId { value: request_id }),
            });
            if let Some(token) = token
                && let Ok(mut value) = MetadataValue::try_from(token.expose())
            {
                value.set_sensitive(true);
                req.metadata_mut().insert(TOKEN_HEADER, value);
            }
            if let Some(bundle) = bundle {
                req.metadata_mut()
                    .insert(humata2_daemon_ports::release::BUNDLE_HEADER, bundle);
            }
            let _ = pb::inbox_service_client::InboxServiceClient::new(channel)
                .withdraw_request(req)
                .await;
        });
    }
}

fn browser_disconnected_error() -> ToolError {
    ToolError {
        kind: ErrorKind::AgentLacksFeature,
        message: "no browser is connected to Humata's browser bridge right now".to_owned(),
        fix: "open Settings › Browser in Humata to set up the native messaging host and load the browser extension, then try again".to_owned(),
        retry: None,
        details: Map::new(),
    }
}

fn opt_u64(arguments: &Map<String, Value>, field: &str) -> Option<u64> {
    arguments.get(field).and_then(Value::as_u64)
}

fn opt_string(arguments: &Map<String, Value>, field: &str) -> Option<String> {
    arguments
        .get(field)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn browser_action(
    operation: Operation,
    arguments: &Map<String, Value>,
) -> Result<BrowserAction, ToolError> {
    match operation {
        Operation::BrowserListTabs => Ok(BrowserAction::ListTabs),
        Operation::BrowserReadTab => Ok(BrowserAction::ReadTab {
            tab_id: opt_u64(arguments, "tab_id"),
            max_chars: opt_u64(arguments, "max_chars").and_then(|v| usize::try_from(v).ok()),
        }),
        Operation::BrowserGuideHighlight => Ok(BrowserAction::HighlightGuide {
            tab_id: opt_u64(arguments, "tab_id"),
            url: text(arguments, "url"),
            selector: text(arguments, "selector"),
            title: text(arguments, "title"),
            body: text(arguments, "body"),
            step_label: opt_string(arguments, "step_label"),
        }),
        Operation::BrowserClick => Ok(BrowserAction::Click {
            tab_id: opt_u64(arguments, "tab_id"),
            url: text(arguments, "url"),
            selector: text(arguments, "selector"),
            element_preview: opt_string(arguments, "element_preview"),
        }),
        Operation::BrowserType => Ok(BrowserAction::Type {
            tab_id: opt_u64(arguments, "tab_id"),
            url: text(arguments, "url"),
            selector: text(arguments, "selector"),
            text: text(arguments, "text"),
            clear_first: arguments
                .get("clear_first")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            element_preview: opt_string(arguments, "element_preview"),
        }),
        Operation::BrowserNavigate => Ok(BrowserAction::Navigate {
            tab_id: opt_u64(arguments, "tab_id"),
            url: text(arguments, "url"),
        }),
        other => Err(ToolError {
            kind: ErrorKind::Internal,
            message: format!("{} is not a browser operation", other.tool_name()),
            fix: String::new(),
            retry: None,
            details: Map::new(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use humata2_mcp::Daemon;

    use super::*;

    fn daemon() -> GrpcDaemon {
        GrpcDaemon::new(PathBuf::from("/nonexistent/humatad.sock"))
    }

    #[tokio::test]
    async fn browser_tools_are_offered_only_for_claude_codex_copilot_and_agy() {
        let d = daemon();
        assert!(!d.serves(Operation::BrowserListTabs));

        for supported in ["claude", "codex", "copilot", "agy"] {
            d.learn_browser_agent(supported);
            assert!(d.serves(Operation::BrowserListTabs), "{supported}");
            assert!(d.serves(Operation::BrowserReadTab), "{supported}");
            assert!(d.serves(Operation::BrowserGuideHighlight), "{supported}");
            assert!(d.serves(Operation::BrowserClick), "{supported}");
            assert!(d.serves(Operation::BrowserType), "{supported}");
            assert!(d.serves(Operation::BrowserNavigate), "{supported}");
        }

        for unsupported in ["opencode", "zsh", "custom", ""] {
            d.learn_browser_agent(unsupported);
            assert!(!d.serves(Operation::BrowserListTabs), "{unsupported}");
            assert!(!d.serves(Operation::BrowserClick), "{unsupported}");
        }
    }

    #[test]
    fn maps_all_six_browser_operations_to_typed_actions() {
        let empty = Map::new();
        assert_eq!(
            browser_action(Operation::BrowserListTabs, &empty).unwrap(),
            BrowserAction::ListTabs
        );

        let read_args = json!({ "tab_id": 4, "max_chars": 500 })
            .as_object()
            .cloned()
            .unwrap();
        assert_eq!(
            browser_action(Operation::BrowserReadTab, &read_args).unwrap(),
            BrowserAction::ReadTab {
                tab_id: Some(4),
                max_chars: Some(500),
            }
        );

        let click_args = json!({
            "tab_id": 2,
            "url": "https://example.com",
            "selector": "button#go",
            "element_preview": "Go"
        })
        .as_object()
        .cloned()
        .unwrap();
        assert_eq!(
            browser_action(Operation::BrowserClick, &click_args).unwrap(),
            BrowserAction::Click {
                tab_id: Some(2),
                url: "https://example.com".to_owned(),
                selector: "button#go".to_owned(),
                element_preview: Some("Go".to_owned()),
            }
        );

        let nav_args = json!({ "url": "https://example.com/next" })
            .as_object()
            .cloned()
            .unwrap();
        assert_eq!(
            browser_action(Operation::BrowserNavigate, &nav_args).unwrap(),
            BrowserAction::Navigate {
                tab_id: None,
                url: "https://example.com/next".to_owned(),
            }
        );
    }
}
