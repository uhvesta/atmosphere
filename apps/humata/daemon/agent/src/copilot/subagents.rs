//! The subagents Copilot starts with its task tool: their own steps, which
//! carry the subagent's id and the call that started it, and how far each
//! has got.

use std::collections::HashMap;

use serde_json::Value;

use crate::event::{EventKind, SubagentStatus};

/// The tool that starts a subagent.
const STARTER: &str = "task";

/// An event to record under the call that started a subagent.
pub(super) type Tagged = (String, EventKind);

#[derive(Default)]
pub(super) struct Subagents {
    /// The call that started each subagent, by the subagent's id.
    calls: HashMap<String, String>,
    /// Every subagent started, by the call that started it.
    known: HashMap<String, Known>,
}

#[derive(Clone, PartialEq)]
struct Known {
    agent_type: String,
    description: String,
    status: SubagentStatus,
    total_tokens: u64,
    tool_uses: u64,
    duration_ms: u64,
}

/// Whether `event` is a step one of the subagents took, not the main
/// agent's.
pub(super) fn owns(event: &Value) -> bool {
    event["agentId"].is_string() || event["data"]["parentToolCallId"].is_string()
}

/// What an event of Copilot's records, wherever it's from; None for events
/// that record nothing, or nothing here.
pub(super) fn step(event: &Value) -> Option<EventKind> {
    let data = &event["data"];
    let text = |field: &str| data[field].as_str().unwrap_or_default().to_owned();
    let kind = match event["type"].as_str().unwrap_or_default() {
        "assistant.message_delta" => EventKind::AssistantDelta {
            text: text("deltaContent"),
        },
        "assistant.message" => EventKind::AssistantMessage {
            text: text("content"),
        },
        "assistant.reasoning" => EventKind::Reasoning {
            text: text("content"),
        },
        "tool.execution_start" => EventKind::ToolStarted {
            call_id: text("toolCallId"),
            tool: text("toolName"),
            arguments_json: data["arguments"].to_string(),
        },
        "tool.execution_complete" => {
            let output = data["result"]["content"]
                .as_str()
                .or_else(|| data["error"]["message"].as_str())
                .or_else(|| data["error"].as_str())
                .unwrap_or_default();
            EventKind::ToolCompleted {
                call_id: text("toolCallId"),
                success: data["success"].as_bool().unwrap_or(false),
                output: output.to_owned(),
            }
        }
        "assistant.usage" => {
            // Each call says what a premium request of its model costs, but
            // only the person's own prompts are charged one.
            let charged = matches!(data["initiator"].as_str(), None | Some("user"));
            EventKind::Usage {
                model: text("model"),
                input_tokens: data["inputTokens"].as_u64().unwrap_or(0),
                output_tokens: data["outputTokens"].as_u64().unwrap_or(0),
                premium_requests: if charged {
                    data["cost"].as_f64().unwrap_or(0.0)
                } else {
                    0.0
                },
                cache_read_tokens: data["cacheReadTokens"].as_u64().unwrap_or(0),
                cache_write_tokens: data["cacheWriteTokens"].as_u64().unwrap_or(0),
            }
        }
        _ => return None,
    };
    let empty = matches!(
        &kind,
        EventKind::AssistantDelta { text }
            | EventKind::AssistantMessage { text }
            | EventKind::Reasoning { text } if text.is_empty()
    );
    (!empty).then_some(kind)
}

impl Subagents {
    /// What to record for one of Copilot's events, besides what the main
    /// agent's handling records: a subagent's step, under the call that
    /// started it, and news of the subagents started or finished.
    pub(super) fn take(&mut self, event: &Value) -> Vec<Tagged> {
        let data = &event["data"];
        if let (Some(agent), Some(call)) =
            (event["agentId"].as_str(), data["parentToolCallId"].as_str())
        {
            self.calls.insert(agent.to_owned(), call.to_owned());
        }
        let mut tagged = Vec::new();
        if owns(event)
            && !matches!(
                event["type"].as_str().unwrap_or_default(),
                // Its replies as they stream are left out, as is its prompt.
                "assistant.message_delta" | "user.message"
            )
            && let Some(kind) = step(event)
        {
            tagged.push((self.parent(event), kind));
        }
        let field = |name: &str| data[name].as_str().unwrap_or_default().to_owned();
        let count = |name: &str| data[name].as_u64();
        let Some(call) = data["toolCallId"].as_str().map(str::to_owned) else {
            return tagged;
        };
        match event["type"].as_str().unwrap_or_default() {
            "tool.execution_start" if data["toolName"] == STARTER => {
                let arguments = &data["arguments"];
                let argument = |name: &str| arguments[name].as_str().unwrap_or_default();
                let known = Known {
                    agent_type: argument("agent_type").to_owned(),
                    description: argument("description").to_owned(),
                    status: SubagentStatus::Running,
                    total_tokens: 0,
                    tool_uses: 0,
                    duration_ms: 0,
                };
                tagged.push((call.clone(), known.kind()));
                self.known.insert(call, known);
            }
            "subagent.started" => {
                self.update(&call, &mut tagged, |known| {
                    if known.agent_type.is_empty() {
                        known.agent_type = field("agentName");
                    }
                    if known.description.is_empty() {
                        known.description = field("agentDisplayName");
                    }
                    known.status = SubagentStatus::Running;
                });
            }
            "subagent.completed" => {
                let cancelled = data["cancelled"].as_bool() == Some(true);
                self.update(&call, &mut tagged, |known| {
                    known.status = if cancelled {
                        SubagentStatus::Stopped
                    } else {
                        SubagentStatus::Completed
                    };
                    known.total_tokens = count("totalTokens").unwrap_or(known.total_tokens);
                    known.tool_uses = count("totalToolCalls").unwrap_or(known.tool_uses);
                    known.duration_ms = count("durationMs").unwrap_or(known.duration_ms);
                });
            }
            "subagent.failed" => {
                self.update(&call, &mut tagged, |known| {
                    known.status = SubagentStatus::Failed;
                    known.duration_ms = count("durationMs").unwrap_or(known.duration_ms);
                });
            }
            _ => {}
        }
        tagged
    }

    /// The call that started the subagent `agent`, or its id while that's
    /// not known.
    pub(super) fn call_of(&self, agent: &str) -> String {
        self.calls
            .get(agent)
            .cloned()
            .unwrap_or_else(|| agent.to_owned())
    }

    fn parent(&self, event: &Value) -> String {
        event["data"]["parentToolCallId"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| event["agentId"].as_str().map(|agent| self.call_of(agent)))
            .unwrap_or_default()
    }

    /// Changes what's known of `call`'s subagent, first known now if it
    /// wasn't, and records it if that changed anything.
    fn update(&mut self, call: &str, tagged: &mut Vec<Tagged>, change: impl FnOnce(&mut Known)) {
        let new = !self.known.contains_key(call);
        let known = self.known.entry(call.to_owned()).or_insert(Known {
            agent_type: String::new(),
            description: String::new(),
            status: SubagentStatus::Running,
            total_tokens: 0,
            tool_uses: 0,
            duration_ms: 0,
        });
        let before = known.clone();
        change(known);
        if new || *known != before {
            tagged.push((call.to_owned(), known.kind()));
        }
    }
}

impl Known {
    fn kind(&self) -> EventKind {
        EventKind::Subagent {
            agent_type: self.agent_type.clone(),
            description: self.description.clone(),
            status: self.status,
            total_tokens: self.total_tokens,
            tool_uses: self.tool_uses,
            duration_ms: self.duration_ms,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn statuses(tagged: &[Tagged]) -> Vec<(String, SubagentStatus)> {
        tagged
            .iter()
            .filter_map(|(call, kind)| match kind {
                EventKind::Subagent { status, .. } => Some((call.clone(), *status)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_subagents_steps_are_tagged_with_the_call_that_started_it() {
        let mut subagents = Subagents::default();
        let events = [
            json!({"type": "tool.execution_start", "data": {"toolCallId": "call-1", "toolName": "task",
                "arguments": {"agent_type": "explore", "description": "Find the parser", "prompt": "..."}}}),
            json!({"type": "subagent.started", "data": {"toolCallId": "call-1", "agentName": "explore",
                "agentDisplayName": "Find the parser"}}),
            json!({"type": "user.message", "agentId": "sub-1", "data": {"content": "..."}}),
            json!({"type": "assistant.message_delta", "agentId": "sub-1",
                "data": {"deltaContent": "Look", "parentToolCallId": "call-1"}}),
            json!({"type": "tool.execution_start", "agentId": "sub-1",
                "data": {"toolCallId": "call-2", "toolName": "grep", "arguments": {"pattern": "parse"},
                    "parentToolCallId": "call-1"}}),
            json!({"type": "assistant.usage", "agentId": "sub-1", "ephemeral": true,
                "data": {"model": "gpt-5-mini", "inputTokens": 100, "outputTokens": 10, "cost": 1.0,
                    "initiator": "sub-agent"}}),
            json!({"type": "assistant.message", "agentId": "sub-1",
                "data": {"content": "It's in src/parse.rs.", "parentToolCallId": "call-1"}}),
            json!({"type": "subagent.completed", "data": {"toolCallId": "call-1", "agentName": "explore",
                "totalTokens": 110, "totalToolCalls": 1, "durationMs": 800}}),
        ];
        let tagged: Vec<Tagged> = events
            .iter()
            .flat_map(|event| subagents.take(event))
            .collect();

        let steps: Vec<&EventKind> = tagged
            .iter()
            .filter(|(_, kind)| !matches!(kind, EventKind::Subagent { .. }))
            .map(|(call, kind)| {
                assert_eq!(call, "call-1");
                kind
            })
            .collect();
        assert_eq!(
            steps,
            [
                &EventKind::ToolStarted {
                    call_id: "call-2".into(),
                    tool: "grep".into(),
                    arguments_json: json!({"pattern": "parse"}).to_string(),
                },
                &EventKind::Usage {
                    model: "gpt-5-mini".into(),
                    input_tokens: 100,
                    output_tokens: 10,
                    premium_requests: 0.0,
                    cache_read_tokens: 0,
                    cache_write_tokens: 0,
                },
                &EventKind::AssistantMessage {
                    text: "It's in src/parse.rs.".into()
                },
            ]
        );
        assert_eq!(
            tagged.last().map(|(_, kind)| kind),
            Some(&EventKind::Subagent {
                agent_type: "explore".into(),
                description: "Find the parser".into(),
                status: SubagentStatus::Completed,
                total_tokens: 110,
                tool_uses: 1,
                duration_ms: 800,
            })
        );
    }

    #[test]
    fn the_main_agents_own_steps_add_nothing() {
        let mut subagents = Subagents::default();
        let events = [
            json!({"type": "assistant.message", "data": {"content": "Running it."}}),
            json!({"type": "tool.execution_start", "data": {"toolCallId": "call-1", "toolName": "bash",
                "arguments": {"command": "ls"}}}),
            json!({"type": "assistant.usage", "data": {"model": "gpt-5", "inputTokens": 1}}),
        ];
        for event in &events {
            assert!(!owns(event));
            assert!(subagents.take(event).is_empty());
        }
    }

    #[test]
    fn a_subagent_known_only_from_its_lifecycle_is_still_listed() {
        let mut subagents = Subagents::default();
        let started = subagents.take(&json!({"type": "subagent.started",
            "data": {"toolCallId": "call-1", "agentName": "explore", "agentDisplayName": "Look around"}}));
        let failed = subagents.take(&json!({"type": "subagent.failed",
            "data": {"toolCallId": "call-1", "error": "boom", "durationMs": 5}}));
        assert_eq!(
            statuses(&started),
            [("call-1".to_owned(), SubagentStatus::Running)]
        );
        assert_eq!(
            statuses(&failed),
            [("call-1".to_owned(), SubagentStatus::Failed)]
        );
    }

    #[test]
    fn a_step_naming_only_its_subagent_goes_under_the_call_that_started_it() {
        let mut subagents = Subagents::default();
        subagents.take(&json!({"type": "assistant.message", "agentId": "sub-1",
            "data": {"content": "First.", "parentToolCallId": "call-1"}}));
        let tagged = subagents.take(&json!({"type": "assistant.reasoning", "agentId": "sub-1",
            "data": {"content": "Then this."}}));
        assert_eq!(
            tagged,
            [(
                "call-1".to_owned(),
                EventKind::Reasoning {
                    text: "Then this.".into()
                }
            )]
        );
    }

    #[test]
    fn only_the_persons_own_prompts_are_charged_a_premium_request() {
        let usage = |initiator: Option<&str>| {
            let mut data = json!({"model": "gpt-5", "inputTokens": 1, "cost": 1.0});
            if let Some(initiator) = initiator {
                data["initiator"] = json!(initiator);
            }
            match step(&json!({"type": "assistant.usage", "data": data})) {
                Some(EventKind::Usage {
                    premium_requests, ..
                }) => premium_requests,
                _ => f64::NAN,
            }
        };
        assert!((usage(None) - 1.0).abs() < f64::EPSILON);
        assert!((usage(Some("user")) - 1.0).abs() < f64::EPSILON);
        assert!(usage(Some("agent")).abs() < f64::EPSILON);
        assert!(usage(Some("sub-agent")).abs() < f64::EPSILON);
    }
}
