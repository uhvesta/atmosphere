//! The subagents Claude Code starts with its Agent tool, called Task in
//! older versions: their own steps, which the CLI marks with the call that
//! started them, and how far each has got.

use std::collections::HashMap;

use serde_json::Value;

use crate::event::{EventKind, SubagentStatus};

/// The tools that start a subagent.
const STARTERS: [&str; 2] = ["Agent", "Task"];

/// An event to record under the call that started a subagent.
pub(super) type Tagged = (String, EventKind);

#[derive(Default)]
pub(super) struct Subagents {
    /// Every subagent started this turn, by the call that started it.
    known: HashMap<String, Known>,
    /// Per subagent, the reply it's writing: its message id, model and
    /// usage as last said, counted once the next one starts or it ends.
    writing: HashMap<String, Writing>,
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

struct Writing {
    message: String,
    model: String,
    usage: Value,
}

impl Subagents {
    /// What to record for one of the CLI's events, besides what the main
    /// agent's handling records: a subagent's step, under the call that
    /// started it, and news of the subagents started or finished.
    pub(super) fn take(&mut self, event: &Value) -> Vec<Tagged> {
        let mut tagged = Vec::new();
        let parent = event["parent_tool_use_id"].as_str();
        match event["type"].as_str().unwrap_or_default() {
            // The CLI's own message about a failed request.
            "assistant" if !event["error"].is_null() => {}
            "assistant" => {
                if let Some(parent) = parent {
                    self.wrote(parent, event, &mut tagged);
                }
                self.started(event, &mut tagged);
            }
            "user" if event["isReplay"].as_bool() == Some(true) => {}
            "user" => {
                if let Some(parent) = parent {
                    tagged.extend(results(event).map(|kind| (parent.to_owned(), kind)));
                }
                self.returned(event, &mut tagged);
            }
            "system" => self.task(event, &mut tagged),
            _ => {}
        }
        tagged
    }

    /// At the turn's end: the usage of replies not yet counted, and every
    /// subagent still running as stopped, as the CLI stops with the turn.
    pub(super) fn finish(&mut self) -> Vec<Tagged> {
        let mut tagged = Vec::new();
        let mut parents: Vec<String> = self.writing.keys().cloned().collect();
        parents.sort();
        for parent in parents {
            self.count(&parent, &mut tagged);
        }
        let mut running: Vec<String> = self
            .known
            .iter()
            .filter(|(_, known)| known.status == SubagentStatus::Running)
            .map(|(call, _)| call.clone())
            .collect();
        running.sort();
        for call in running {
            self.update(&call, &mut tagged, |known| {
                known.status = SubagentStatus::Stopped;
            });
        }
        tagged
    }

    /// A subagent's reply: its text, thinking and tool calls, and its usage
    /// once the reply is whole.
    fn wrote(&mut self, parent: &str, event: &Value, tagged: &mut Vec<Tagged>) {
        let message = &event["message"];
        let id = message["id"].as_str().unwrap_or_default();
        if self
            .writing
            .get(parent)
            .is_some_and(|writing| writing.message != id)
        {
            self.count(parent, tagged);
        }
        if !message["usage"].is_null() {
            self.writing.insert(
                parent.to_owned(),
                Writing {
                    message: id.to_owned(),
                    model: message["model"].as_str().unwrap_or_default().to_owned(),
                    usage: message["usage"].clone(),
                },
            );
        }
        for block in blocks(event) {
            let text = |field: &str| block[field].as_str().unwrap_or_default().to_owned();
            let kind = match block["type"].as_str().unwrap_or_default() {
                "text" if !text("text").is_empty() => {
                    EventKind::AssistantMessage { text: text("text") }
                }
                "thinking" if !text("thinking").is_empty() => EventKind::Reasoning {
                    text: text("thinking"),
                },
                "tool_use" => EventKind::ToolStarted {
                    call_id: text("id"),
                    tool: text("name"),
                    arguments_json: block["input"].to_string(),
                },
                _ => continue,
            };
            tagged.push((parent.to_owned(), kind));
        }
    }

    /// The usage of the reply `parent`'s subagent last wrote, once.
    fn count(&mut self, parent: &str, tagged: &mut Vec<Tagged>) {
        let Some(writing) = self.writing.remove(parent) else {
            return;
        };
        let count = |field: &str| writing.usage[field].as_u64().unwrap_or(0);
        tagged.push((
            parent.to_owned(),
            EventKind::Usage {
                model: writing.model,
                input_tokens: count("input_tokens")
                    + count("cache_creation_input_tokens")
                    + count("cache_read_input_tokens"),
                output_tokens: count("output_tokens"),
                premium_requests: 0.0,
                cache_read_tokens: count("cache_read_input_tokens"),
                cache_write_tokens: count("cache_creation_input_tokens"),
            },
        ));
    }

    /// The subagents a reply's tool calls start.
    fn started(&mut self, event: &Value, tagged: &mut Vec<Tagged>) {
        for block in blocks(event) {
            let name = block["name"].as_str().unwrap_or_default();
            if block["type"] != "tool_use" || !STARTERS.contains(&name) {
                continue;
            }
            let Some(call) = block["id"].as_str() else {
                continue;
            };
            let input = &block["input"];
            let field = |key: &str| input[key].as_str().unwrap_or_default().to_owned();
            let known = Known {
                agent_type: field("subagent_type"),
                description: field("description"),
                status: SubagentStatus::Running,
                total_tokens: 0,
                tool_uses: 0,
                duration_ms: 0,
            };
            tagged.push((call.to_owned(), known.kind()));
            self.known.insert(call.to_owned(), known);
        }
    }

    /// The subagents whose calls returned: finished, unless the call only
    /// started one in the background.
    fn returned(&mut self, event: &Value, tagged: &mut Vec<Tagged>) {
        for block in blocks(event) {
            let Some(call) = block["tool_use_id"].as_str() else {
                continue;
            };
            if block["type"] != "tool_result" || !self.known.contains_key(call) {
                continue;
            }
            // Started in the background, it says when it's done as a task.
            let result = &event["tool_use_result"];
            if result["status"] == "async_launched" || result["isAsync"].as_bool() == Some(true) {
                continue;
            }
            self.count(call, tagged);
            let failed = block["is_error"].as_bool() == Some(true);
            self.update(call, tagged, |known| {
                known.status = if failed {
                    SubagentStatus::Failed
                } else {
                    SubagentStatus::Completed
                };
                set(&mut known.total_tokens, &result["totalTokens"]);
                set(&mut known.tool_uses, &result["totalToolUseCount"]);
                set(&mut known.duration_ms, &result["totalDurationMs"]);
            });
        }
    }

    /// The CLI's news of a task: started, how far it's got, or done. Tasks
    /// that aren't subagents, such as a command left running, are skipped.
    fn task(&mut self, event: &Value, tagged: &mut Vec<Tagged>) {
        let Some(call) = event["tool_use_id"].as_str() else {
            return;
        };
        if !self.known.contains_key(call) {
            return;
        }
        let subtype = event["subtype"].as_str().unwrap_or_default();
        if !matches!(
            subtype,
            "task_started" | "task_progress" | "task_notification"
        ) {
            return;
        }
        let call = call.to_owned();
        let usage = &event["usage"];
        if subtype == "task_notification" {
            self.count(&call, tagged);
        }
        self.update(&call, tagged, |known| {
            if let Some(kind) = event["subagent_type"]
                .as_str()
                .filter(|kind| !kind.is_empty())
            {
                kind.clone_into(&mut known.agent_type);
            }
            if known.description.is_empty()
                && let Some(description) = event["description"].as_str()
            {
                description.clone_into(&mut known.description);
            }
            set(&mut known.total_tokens, &usage["total_tokens"]);
            set(&mut known.tool_uses, &usage["tool_uses"]);
            set(&mut known.duration_ms, &usage["duration_ms"]);
            if subtype == "task_notification" {
                known.status = match event["status"].as_str().unwrap_or_default() {
                    "completed" => SubagentStatus::Completed,
                    "failed" => SubagentStatus::Failed,
                    _ => SubagentStatus::Stopped,
                };
            }
        });
    }

    /// Changes what's known of `call`'s subagent, and records it if that
    /// changed anything.
    fn update(&mut self, call: &str, tagged: &mut Vec<Tagged>, change: impl FnOnce(&mut Known)) {
        let Some(known) = self.known.get_mut(call) else {
            return;
        };
        let before = known.clone();
        change(known);
        if *known != before {
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

/// Sets `to` from `value` if it's a count.
fn set(to: &mut u64, value: &Value) {
    if let Some(value) = value.as_u64() {
        *to = value;
    }
}

fn blocks(event: &Value) -> &[Value] {
    event["message"]["content"]
        .as_array()
        .map_or(&[][..], Vec::as_slice)
}

/// The tool results in a message the CLI passes a subagent; the prompt it
/// starts with is skipped.
fn results(event: &Value) -> impl Iterator<Item = EventKind> + '_ {
    blocks(event)
        .iter()
        .filter(|block| block["type"] == "tool_result")
        .map(|block| EventKind::ToolCompleted {
            call_id: block["tool_use_id"].as_str().unwrap_or_default().to_owned(),
            success: block["is_error"].as_bool() != Some(true),
            output: match &block["content"] {
                Value::String(text) => text.clone(),
                Value::Array(parts) => parts
                    .iter()
                    .filter_map(|part| part["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
                _ => String::new(),
            },
        })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn agent_call(id: &str, parent: Option<&str>) -> Value {
        json!({
            "type": "assistant",
            "parent_tool_use_id": parent,
            "message": {"id": format!("msg-{id}"), "model": "claude-fable", "content": [{
                "type": "tool_use", "id": id, "name": "Agent",
                "input": {"subagent_type": "Explore", "description": "Find the parser", "prompt": "..."},
            }], "usage": {"input_tokens": 10, "output_tokens": 5}},
        })
    }

    fn running(description: &str) -> EventKind {
        EventKind::Subagent {
            agent_type: "Explore".into(),
            description: description.into(),
            status: SubagentStatus::Running,
            total_tokens: 0,
            tool_uses: 0,
            duration_ms: 0,
        }
    }

    #[test]
    fn a_subagents_steps_are_tagged_with_the_call_that_started_it() {
        let mut subagents = Subagents::default();
        let mut tagged = subagents.take(&agent_call("toolu_a", None));
        tagged.extend(subagents.take(&json!({
            "type": "user",
            "parent_tool_use_id": "toolu_a",
            "message": {"content": [{"type": "text", "text": "Find the parser"}]},
        })));
        tagged.extend(subagents.take(&json!({
            "type": "assistant",
            "parent_tool_use_id": "toolu_a",
            "message": {"id": "msg-1", "model": "claude-haiku", "content": [
                {"type": "thinking", "thinking": "Grep for it."},
                {"type": "tool_use", "id": "toolu_b", "name": "Grep", "input": {"pattern": "parse"}},
            ], "usage": {"input_tokens": 100, "cache_read_input_tokens": 50, "output_tokens": 20}},
        })));
        tagged.extend(subagents.take(&json!({
            "type": "user",
            "parent_tool_use_id": "toolu_a",
            "message": {"content": [
                {"type": "tool_result", "tool_use_id": "toolu_b", "content": "src/parse.rs"},
            ]},
        })));
        tagged.extend(subagents.take(&json!({
            "type": "assistant",
            "parent_tool_use_id": "toolu_a",
            "message": {"id": "msg-2", "model": "claude-haiku", "content": [
                {"type": "text", "text": "It's in src/parse.rs."},
            ], "usage": {"input_tokens": 200, "output_tokens": 10}},
        })));
        tagged.extend(subagents.take(&json!({
            "type": "user",
            "parent_tool_use_id": null,
            "message": {"content": [
                {"type": "tool_result", "tool_use_id": "toolu_a", "content": "It's in src/parse.rs."},
            ]},
            "tool_use_result": {"status": "completed", "totalTokens": 380, "totalToolUseCount": 1, "totalDurationMs": 900},
        })));

        let a = "toolu_a".to_owned();
        assert_eq!(
            tagged,
            [
                (a.clone(), running("Find the parser")),
                (
                    a.clone(),
                    EventKind::Reasoning {
                        text: "Grep for it.".into()
                    }
                ),
                (
                    a.clone(),
                    EventKind::ToolStarted {
                        call_id: "toolu_b".into(),
                        tool: "Grep".into(),
                        arguments_json: json!({"pattern": "parse"}).to_string(),
                    }
                ),
                (
                    a.clone(),
                    EventKind::ToolCompleted {
                        call_id: "toolu_b".into(),
                        success: true,
                        output: "src/parse.rs".into(),
                    }
                ),
                (
                    a.clone(),
                    EventKind::Usage {
                        model: "claude-haiku".into(),
                        input_tokens: 150,
                        output_tokens: 20,
                        premium_requests: 0.0,
                        cache_read_tokens: 50,
                        cache_write_tokens: 0,
                    }
                ),
                (
                    a.clone(),
                    EventKind::AssistantMessage {
                        text: "It's in src/parse.rs.".into()
                    }
                ),
                (
                    a.clone(),
                    EventKind::Usage {
                        model: "claude-haiku".into(),
                        input_tokens: 200,
                        output_tokens: 10,
                        premium_requests: 0.0,
                        cache_read_tokens: 0,
                        cache_write_tokens: 0,
                    }
                ),
                (
                    a,
                    EventKind::Subagent {
                        agent_type: "Explore".into(),
                        description: "Find the parser".into(),
                        status: SubagentStatus::Completed,
                        total_tokens: 380,
                        tool_uses: 1,
                        duration_ms: 900,
                    }
                ),
            ]
        );
        assert!(subagents.finish().is_empty());
    }

    #[test]
    fn the_main_agents_own_steps_add_nothing() {
        let mut subagents = Subagents::default();
        let steps = [
            json!({"type": "assistant", "parent_tool_use_id": null, "message": {"id": "m", "content": [
                {"type": "text", "text": "Running it."},
                {"type": "tool_use", "id": "toolu_x", "name": "Bash", "input": {"command": "ls"}},
            ], "usage": {"input_tokens": 1, "output_tokens": 1}}}),
            json!({"type": "user", "parent_tool_use_id": null, "message": {"content": [
                {"type": "tool_result", "tool_use_id": "toolu_x", "content": "a b"},
            ]}}),
            json!({"type": "system", "subtype": "task_started", "tool_use_id": "toolu_x", "task_type": "local_bash"}),
        ];
        for step in &steps {
            assert!(subagents.take(step).is_empty());
        }
        assert!(subagents.finish().is_empty());
    }

    #[test]
    fn a_background_subagent_runs_until_its_task_says_it_finished() {
        let mut subagents = Subagents::default();
        subagents.take(&agent_call("toolu_a", None));
        let launched = subagents.take(&json!({
            "type": "user",
            "parent_tool_use_id": null,
            "message": {"content": [{"type": "tool_result", "tool_use_id": "toolu_a", "content": "Started."}]},
            "tool_use_result": {"isAsync": true, "status": "async_launched", "agentId": "a1"},
        }));
        assert!(launched.is_empty());

        let progress = json!({
            "type": "system", "subtype": "task_progress", "tool_use_id": "toolu_a",
            "usage": {"total_tokens": 1200, "tool_uses": 3, "duration_ms": 4000},
        });
        let progressed = subagents.take(&progress);
        assert_eq!(progressed.len(), 1);
        assert!(subagents.take(&progress).is_empty());

        let done = subagents.take(&json!({
            "type": "system", "subtype": "task_notification", "tool_use_id": "toolu_a",
            "status": "completed",
            "usage": {"total_tokens": 1500, "tool_uses": 4, "duration_ms": 5000},
        }));
        assert_eq!(
            done,
            [(
                "toolu_a".to_owned(),
                EventKind::Subagent {
                    agent_type: "Explore".into(),
                    description: "Find the parser".into(),
                    status: SubagentStatus::Completed,
                    total_tokens: 1500,
                    tool_uses: 4,
                    duration_ms: 5000,
                }
            )]
        );
    }

    #[test]
    fn subagents_still_running_when_the_turn_ends_are_stopped() {
        let mut subagents = Subagents::default();
        subagents.take(&agent_call("toolu_a", None));
        subagents.take(&agent_call("toolu_b", Some("toolu_a")));

        let finished = subagents.finish();
        let statuses: Vec<(String, SubagentStatus)> = finished
            .into_iter()
            .filter_map(|(call, kind)| match kind {
                EventKind::Subagent { status, .. } => Some((call, status)),
                _ => None,
            })
            .collect();
        assert_eq!(
            statuses,
            [
                ("toolu_a".to_owned(), SubagentStatus::Stopped),
                ("toolu_b".to_owned(), SubagentStatus::Stopped),
            ]
        );
    }
}
