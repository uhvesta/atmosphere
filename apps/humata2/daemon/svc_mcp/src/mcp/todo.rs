//! The To-do tools of `humatad2 mcp`: an agent asks for the next item,
//! suggests one, or asks to close one through TodoService. The daemon makes
//! an agent's new item suggested and sends a close through the Edit rule,
//! so a close may come back as a pending approval.

use crate::pb;
use humata2_mcp::{Call, ErrorKind, Reply, ToolError};
use serde_json::{Map, Value, json};
use tonic::transport::Channel;

use super::boards::{expected_revision, read_reply, stale_as_of, strings};
use super::{GrpcDaemon, applied, key, lower, outcome, text};

impl GrpcDaemon {
    fn todo_client(&self) -> pb::todo_service_client::TodoServiceClient<Channel> {
        pb::todo_service_client::TodoServiceClient::new(self.channel.clone())
    }

    /// `todo_next`.
    pub(super) async fn todo_next(&self, call: Call) -> Result<Reply, ToolError> {
        let arguments = &call.arguments;
        let channel = text(arguments, "channel");
        let session = text(arguments, "session");
        let request = self.request(pb::NextTodoRequest {
            filter: Some(pb::TodoFilter {
                channel: (!channel.is_empty()).then_some(pb::ChannelId { value: channel }),
                session: (!session.is_empty()).then_some(pb::SessionId { value: session }),
                ..pb::TodoFilter::default()
            }),
        })?;
        let response = self
            .todo_client()
            .next_todo(request)
            .await
            .map_err(|status| self.failed(&status))?;
        let stale_as_of = stale_as_of(response.metadata());
        let next = response.into_inner();
        let result = next.item.as_ref().map(todo_json).unwrap_or(Value::Null);
        Ok(read_reply(result, stale_as_of))
    }

    /// `todo_add`.
    pub(super) async fn todo_add(&self, call: Call) -> Result<Reply, ToolError> {
        let arguments = &call.arguments;
        let channel = text(arguments, "channel");
        let due = civil_day(arguments, "due")?;
        let effort = effort_of(arguments)?;
        let impact = impact_of(arguments)?;
        let request = self.request(pb::CreateTodoRequest {
            idempotency_key: Some(key(&call)),
            title: text(arguments, "title"),
            channel: (!channel.is_empty()).then_some(pb::ChannelId { value: channel }),
            description: text(arguments, "description"),
            due,
            effort: effort as i32,
            impact: impact as i32,
            tags: strings(arguments, "tags"),
            steps: strings(arguments, "steps")
                .into_iter()
                .map(|text| pb::TodoStep {
                    text,
                    ..pb::TodoStep::default()
                })
                .collect(),
            reason: text(arguments, "reason"),
            evidence: strings(arguments, "evidence"),
            ..pb::CreateTodoRequest::default()
        })?;
        let written = self
            .todo_client()
            .create_todo(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        let mut result = written_json(&written);
        if let Value::Object(map) = &mut result {
            map.insert(
                "note".to_owned(),
                json!("Added as suggested: it waits for the owner's Yes before it joins the list."),
            );
        }
        Ok(write_reply(&written, result))
    }

    /// `todo_close`.
    pub(super) async fn todo_close(&self, call: Call) -> Result<Reply, ToolError> {
        let arguments = &call.arguments;
        let request = self.request(pb::UpdateTodoRequest {
            idempotency_key: Some(key(&call)),
            id: Some(pb::BoardItemId {
                value: text(arguments, "key"),
            }),
            patch: Some(pb::TodoPatch {
                state: Some(pb::TodoState::Closed as i32),
                ..pb::TodoPatch::default()
            }),
            expected_revision: expected_revision(arguments),
            reason: text(arguments, "reason"),
            evidence: strings(arguments, "evidence"),
        })?;
        let written = self
            .todo_client()
            .update_todo(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        Ok(write_reply(&written, written_json(&written)))
    }

    /// `todo_ask_question`.
    pub(super) async fn todo_ask_question(&self, call: Call) -> Result<Reply, ToolError> {
        let ask = ask_question_request(&call.arguments, key(&call))?;
        let request = self.request(ask)?;
        let response = self
            .inbox()
            .ask_user(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        outcome(response)
    }

    /// `todo_request_action`.
    pub(super) async fn todo_request_action(&self, call: Call) -> Result<Reply, ToolError> {
        let ask = request_action_request(&call.arguments, key(&call))?;
        let request = self.request(ask)?;
        let response = self
            .inbox()
            .ask_user(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        outcome(response)
    }

    /// `todo_request_review`.
    pub(super) async fn todo_request_review(&self, call: Call) -> Result<Reply, ToolError> {
        let req = request_local_review_request(&call.arguments, key(&call))?;
        let request = self.request(req)?;
        let resp = self
            .todo_client()
            .request_local_review(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        Ok(applied(
            json!({
                "todo_key": resp.todo_key,
                "attached": resp.attached,
                "attachment_reason": resp.attachment_reason,
                "layers_count": resp.layers_count,
                "notice": resp.notice,
            }),
            None,
        ))
    }

    /// `todo_step_update`.
    pub(super) async fn todo_step_update(&self, call: Call) -> Result<Reply, ToolError> {
        let update = step_update_request(&call.arguments, key(&call))?;
        let request = self.request(update)?;
        let written = self
            .todo_client()
            .update_todo(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        Ok(write_reply(&written, written_json(&written)))
    }

    /// `todo_set_checkpoint`.
    pub(super) async fn todo_set_checkpoint(&self, call: Call) -> Result<Reply, ToolError> {
        let update = set_checkpoint_request(&call.arguments, key(&call))?;
        let request = self.request(update)?;
        let written = self
            .todo_client()
            .update_todo(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        Ok(write_reply(&written, written_json(&written)))
    }

    /// `todo_report_run`.
    pub(super) async fn todo_report_run(&self, call: Call) -> Result<Reply, ToolError> {
        let (comment_req, mut summary) = report_run_request(&call.arguments, key(&call))?;
        let request = self.request(comment_req)?;
        let added = self
            .todo_client()
            .add_todo_comment(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        if let Value::Object(map) = &mut summary {
            if let Some(comment) = added.comment {
                map.insert("comment_id".to_owned(), json!(comment.id));
            }
            if let Some(item) = added.item {
                map.insert("item".to_owned(), todo_json(&item));
            }
        }
        Ok(applied(summary, None))
    }
}

fn require_non_empty(arguments: &Map<String, Value>, field: &str) -> Result<String, ToolError> {
    let value = text(arguments, field);
    if value.trim().is_empty() {
        return Err(invalid(field, &value, "a non-empty string"));
    }
    Ok(value)
}

fn ask_question_request(
    arguments: &Map<String, Value>,
    idempotency_key: pb::IdempotencyKey,
) -> Result<pb::AskUserRequest, ToolError> {
    let item_key = require_non_empty(arguments, "key")?;
    let question = require_non_empty(arguments, "question")?;
    let choices = strings(arguments, "choices");
    if choices.len() > 4 {
        return Err(invalid("choices", &choices.len().to_string(), "at most 4 choices"));
    }
    let multi_select = arguments
        .get("multi_select")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if multi_select && choices.is_empty() {
        return Err(invalid(
            "multi_select",
            "true",
            "false when no choices are provided, or pass 1 to 4 choices",
        ));
    }
    Ok(pb::AskUserRequest {
        idempotency_key: Some(idempotency_key),
        question,
        choices,
        wait_seconds: 0,
        todo_item: Some(pb::BoardItemId { value: item_key }),
        step_id: text(arguments, "step_id"),
        multi_select,
        details: text(arguments, "details"),
    })
}

fn request_action_request(
    arguments: &Map<String, Value>,
    idempotency_key: pb::IdempotencyKey,
) -> Result<pb::AskUserRequest, ToolError> {
    let item_key = require_non_empty(arguments, "key")?;
    let action = require_non_empty(arguments, "action")?;
    Ok(pb::AskUserRequest {
        idempotency_key: Some(idempotency_key),
        question: action,
        choices: vec!["Done".to_owned(), "Can't be done".to_owned()],
        wait_seconds: 0,
        todo_item: Some(pb::BoardItemId { value: item_key }),
        step_id: text(arguments, "step_id"),
        multi_select: false,
        details: text(arguments, "details"),
    })
}

fn request_local_review_request(
    arguments: &Map<String, Value>,
    idempotency_key: pb::IdempotencyKey,
) -> Result<pb::RequestLocalReviewRequest, ToolError> {
    let item_key = text(arguments, "key");
    let channel = text(arguments, "channel");
    let summary_what = {
        let s = text(arguments, "summary_what");
        if s.is_empty() {
            text(arguments, "summary")
        } else {
            s
        }
    };
    let summary_details = {
        let s = text(arguments, "summary_details");
        if s.is_empty() {
            text(arguments, "details")
        } else {
            s
        }
    };
    let layers = match arguments.get("layers") {
        None | Some(Value::Null) => Vec::new(),
        Some(value) => {
            let raw_layers = value
                .as_array()
                .ok_or_else(|| invalid("layers", &value.to_string(), "an array of layer objects"))?;
            let mut parsed = Vec::with_capacity(raw_layers.len());
            for raw in raw_layers {
                let obj = raw.as_object().ok_or_else(|| {
                    invalid("layers", &raw.to_string(), "an object with repo and head_branch")
                })?;
                let repo = require_non_empty(obj, "repo")?;
                let head_branch = require_non_empty(obj, "head_branch")?;
                let pr_number = match obj.get("pr_number") {
                    None | Some(Value::Null) => None,
                    Some(v) => Some(
                        v.as_u64()
                            .and_then(|n| u32::try_from(n).ok())
                            .filter(|n| *n >= 1)
                            .ok_or_else(|| invalid("pr_number", &v.to_string(), "an integer >= 1"))?,
                    ),
                };
                parsed.push(pb::LocalReviewLayer {
                    repo,
                    base_branch: text(obj, "base_branch"),
                    head_branch,
                    title: text(obj, "title"),
                    pr_number,
                });
            }
            parsed
        }
    };
    Ok(pb::RequestLocalReviewRequest {
        idempotency_key: Some(idempotency_key),
        id: (!item_key.is_empty()).then_some(pb::BoardItemId { value: item_key }),
        title: text(arguments, "title"),
        channel: (!channel.is_empty()).then_some(pb::ChannelId { value: channel }),
        layers,
        summary_what,
        summary_why: text(arguments, "summary_why"),
        summary_details,
    })
}

fn step_update_request(
    arguments: &Map<String, Value>,
    idempotency_key: pb::IdempotencyKey,
) -> Result<pb::UpdateTodoRequest, ToolError> {
    let item_key = require_non_empty(arguments, "key")?;
    let raw_steps = arguments
        .get("steps")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("steps", "", "an array of step objects"))?;
    let mut steps = Vec::with_capacity(raw_steps.len());
    for raw in raw_steps {
        let obj = raw
            .as_object()
            .ok_or_else(|| invalid("steps", &raw.to_string(), "an object with text"))?;
        let step_text = require_non_empty(obj, "text")?;
        steps.push(pb::TodoStep {
            id: text(obj, "id"),
            text: step_text,
            done: obj.get("done").and_then(Value::as_bool).unwrap_or(false),
            markdown_context: text(obj, "markdown_context"),
            dropped: obj.get("dropped").and_then(Value::as_bool).unwrap_or(false),
            ..pb::TodoStep::default()
        });
    }
    Ok(pb::UpdateTodoRequest {
        idempotency_key: Some(idempotency_key),
        id: Some(pb::BoardItemId { value: item_key }),
        patch: Some(pb::TodoPatch {
            steps: Some(pb::TodoSteps { steps }),
            ..pb::TodoPatch::default()
        }),
        expected_revision: expected_revision(arguments),
        reason: text(arguments, "reason"),
        evidence: Vec::new(),
    })
}

fn parse_checkpoint(raw: &str) -> Result<pb::TodoCheckpoint, ToolError> {
    match raw {
        "with_context" => Ok(pb::TodoCheckpoint::WithContext),
        "defined" => Ok(pb::TodoCheckpoint::Defined),
        "done" => Ok(pb::TodoCheckpoint::Done),
        other => Err(invalid("checkpoint", other, "with_context, defined or done")),
    }
}

fn set_checkpoint_request(
    arguments: &Map<String, Value>,
    idempotency_key: pb::IdempotencyKey,
) -> Result<pb::UpdateTodoRequest, ToolError> {
    let item_key = require_non_empty(arguments, "key")?;
    let raw = arguments
        .get("checkpoint")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let checkpoint = parse_checkpoint(raw)?;
    Ok(pb::UpdateTodoRequest {
        idempotency_key: Some(idempotency_key),
        id: Some(pb::BoardItemId { value: item_key }),
        patch: Some(pb::TodoPatch {
            checkpoint: Some(checkpoint as i32),
            ..pb::TodoPatch::default()
        }),
        expected_revision: expected_revision(arguments),
        reason: text(arguments, "reason"),
        evidence: Vec::new(),
    })
}

fn format_cost_micros(cost_micros: u64) -> String {
    let cents = (cost_micros + 5_000) / 10_000;
    format!("${}.{:02}", cents / 100, cents % 100)
}

fn report_run_request(
    arguments: &Map<String, Value>,
    idempotency_key: pb::IdempotencyKey,
) -> Result<(pb::AddTodoCommentRequest, Value), ToolError> {
    let item_key = require_non_empty(arguments, "key")?;
    let tool = require_non_empty(arguments, "tool")?;
    let outcome_text = require_non_empty(arguments, "outcome")?;
    let model = text(arguments, "model");
    let cost_micros = arguments
        .get("cost_micros")
        .and_then(Value::as_u64);
    let links: Vec<(String, String)> = arguments
        .get("links")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_object)
                .map(|obj| (text(obj, "label"), text(obj, "target")))
                .filter(|(label, target)| !label.is_empty() && !target.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let mut header = if model.is_empty() {
        format!("Run: {tool} — {outcome_text}")
    } else {
        format!("Run: {tool} ({model}) — {outcome_text}")
    };
    // T6: a cost the agent didn't report stays unknown (null), never $0.
    if let Some(cost) = cost_micros.filter(|c| *c > 0) {
        header.push_str(&format!(" · {}", format_cost_micros(cost)));
    }
    let mut lines = vec![header];
    for (label, target) in &links {
        lines.push(format!("- [{label}]({target})"));
    }
    let body = lines.join("\n");

    let req = pb::AddTodoCommentRequest {
        idempotency_key: Some(idempotency_key),
        id: Some(pb::BoardItemId {
            value: item_key.clone(),
        }),
        body,
        step_id: String::new(),
    };
    let links_json: Vec<Value> = links
        .into_iter()
        .map(|(label, target)| json!({ "label": label, "target": target }))
        .collect();
    let summary = json!({
        "key": item_key,
        "tool": tool,
        "model": if model.is_empty() { Value::Null } else { json!(model) },
        "outcome": outcome_text,
        "cost_micros": cost_micros,
        "links": links_json,
    });
    Ok((req, summary))
}

fn invalid(field: &str, given: &str, accepted: &str) -> ToolError {
    let mut details = Map::new();
    details.insert("field".to_owned(), json!(field));
    details.insert("problem".to_owned(), json!("malformed"));
    ToolError {
        kind: ErrorKind::InvalidField,
        message: format!("{field} {given:?} isn't accepted"),
        fix: format!("write {field} as {accepted}"),
        retry: None,
        details,
    }
}

/// `YYYY-MM-DD` as a real calendar day, or nothing when not given.
fn civil_day(arguments: &Map<String, Value>, field: &str) -> Result<Option<pb::CivilDay>, ToolError> {
    let Some(given) = arguments.get(field).and_then(Value::as_str) else {
        return Ok(None);
    };
    let bad = || invalid(field, given, "a day as YYYY-MM-DD, such as 2026-11-20");
    let mut parts = given.split('-');
    let (Some(year), Some(month), Some(day), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(bad());
    };
    if year.len() != 4 || month.len() != 2 || day.len() != 2 {
        return Err(bad());
    }
    let (Ok(year), Ok(month), Ok(day)) =
        (year.parse::<i32>(), month.parse::<u32>(), day.parse::<u32>())
    else {
        return Err(bad());
    };
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let longest = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return Err(bad()),
    };
    if day == 0 || day > longest {
        return Err(bad());
    }
    Ok(Some(pb::CivilDay { year, month, day }))
}

fn effort_of(arguments: &Map<String, Value>) -> Result<pb::TodoEffort, ToolError> {
    match arguments.get("effort").and_then(Value::as_str) {
        None => Ok(pb::TodoEffort::Unspecified),
        Some("s") => Ok(pb::TodoEffort::S),
        Some("m") => Ok(pb::TodoEffort::M),
        Some("l") => Ok(pb::TodoEffort::L),
        Some(other) => Err(invalid("effort", other, "s, m or l")),
    }
}

fn impact_of(arguments: &Map<String, Value>) -> Result<pb::TodoImpact, ToolError> {
    match arguments.get("impact").and_then(Value::as_str) {
        None => Ok(pb::TodoImpact::Unspecified),
        Some("low") => Ok(pb::TodoImpact::Low),
        Some("medium") => Ok(pb::TodoImpact::Medium),
        Some("high") => Ok(pb::TodoImpact::High),
        Some(other) => Err(invalid("impact", other, "low, medium or high")),
    }
}

fn day_text(day: &pb::CivilDay) -> String {
    format!("{:04}-{:02}-{:02}", day.year, day.month, day.day)
}

fn todo_json(todo: &pb::TodoItem) -> Value {
    let item = todo.item.clone().unwrap_or_default();
    let fields = todo.fields.clone().unwrap_or_default();
    let state = pb::TodoState::try_from(fields.state)
        .map(|state| lower(state.as_str_name(), "TODO_STATE_"))
        .unwrap_or_default();
    let effort = pb::TodoEffort::try_from(fields.effort)
        .ok()
        .filter(|effort| *effort != pb::TodoEffort::Unspecified)
        .map(|effort| lower(effort.as_str_name(), "TODO_EFFORT_"));
    let impact = pb::TodoImpact::try_from(fields.impact)
        .ok()
        .filter(|impact| *impact != pb::TodoImpact::Unspecified)
        .map(|impact| lower(impact.as_str_name(), "TODO_IMPACT_"));
    json!({
        "key": fields.key,
        "id": item.id.as_ref().map(|id| id.value.clone()),
        "title": item.title,
        "description": item.description,
        "state": state,
        "suggested": fields.suggested,
        "due": fields.due.as_ref().map(day_text),
        "effort": effort,
        "impact": impact,
        "tags": fields.tags,
        "steps": fields
            .steps
            .iter()
            .map(|step| json!({ "text": step.text, "done": step.done }))
            .collect::<Vec<_>>(),
        "channel": item.channel.as_ref().map(|channel| channel.value.clone()),
        "sessions": item.sessions.iter().map(|session| session.value.clone()).collect::<Vec<_>>(),
        "revision": item.revision.as_ref().map(|revision| revision.value),
    })
}

/// What a write answered: the item, or, for a journaled write, nothing.
fn written_json(written: &pb::WriteTodoResponse) -> Value {
    match &written.item {
        Some(item) => todo_json(item),
        None => json!({}),
    }
}

fn write_reply(written: &pb::WriteTodoResponse, result: Value) -> Reply {
    match (written.delivery(), &written.journal) {
        (pb::Delivery::Journaled, Some(position)) => Reply::Journaled {
            result,
            journal: humata2_mcp::JournalPosition {
                machine: position
                    .machine
                    .as_ref()
                    .map(|machine| machine.value.clone())
                    .unwrap_or_default(),
                seq: position.seq,
            },
        },
        _ => applied(result, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap_or_default()
    }

    #[test]
    fn a_day_is_read_when_it_exists_on_the_calendar() {
        let day = civil_day(&args(json!({"due": "2024-02-29"})), "due").expect("a leap day");
        assert_eq!(day, Some(pb::CivilDay { year: 2024, month: 2, day: 29 }));
        assert_eq!(civil_day(&args(json!({})), "due").expect("none given"), None);
    }

    #[test]
    fn a_day_that_is_not_a_date_is_refused_with_the_accepted_form() {
        for bad in ["2025-02-29", "2026-13-01", "2026-11-00", "11/20/2026", "2026-1-5", "soon", ""] {
            let error = civil_day(&args(json!({"due": bad})), "due").expect_err(bad);
            assert_eq!(error.kind, ErrorKind::InvalidField, "{bad}");
            assert!(error.fix.contains("YYYY-MM-DD"), "{bad}");
        }
    }

    #[test]
    fn effort_and_impact_take_only_their_named_values() {
        assert_eq!(effort_of(&args(json!({"effort": "m"}))).expect("m"), pb::TodoEffort::M);
        assert_eq!(effort_of(&args(json!({}))).expect("none"), pb::TodoEffort::Unspecified);
        assert_eq!(impact_of(&args(json!({"impact": "high"}))).expect("high"), pb::TodoImpact::High);
        assert_eq!(
            effort_of(&args(json!({"effort": "xl"}))).expect_err("xl").kind,
            ErrorKind::InvalidField
        );
        assert_eq!(
            impact_of(&args(json!({"impact": "huge"}))).expect_err("huge").kind,
            ErrorKind::InvalidField
        );
    }

    #[test]
    fn an_item_is_shown_with_its_to_do_fields() {
        let todo = pb::TodoItem {
            item: Some(pb::BoardItem {
                title: "ship it".to_owned(),
                channel: Some(pb::ChannelId { value: "c1".to_owned() }),
                revision: Some(pb::Revision { value: 4 }),
                ..pb::BoardItem::default()
            }),
            fields: Some(pb::TodoFields {
                key: "PAY-3".to_owned(),
                state: pb::TodoState::InProgress as i32,
                due: Some(pb::CivilDay { year: 2026, month: 11, day: 5 }),
                effort: pb::TodoEffort::L as i32,
                tags: vec!["api".to_owned()],
                steps: vec![pb::TodoStep { text: "a".to_owned(), done: true, ..Default::default() }],
                ..pb::TodoFields::default()
            }),
        };
        let shown = todo_json(&todo);
        assert_eq!(shown["key"], "PAY-3");
        assert_eq!(shown["state"], "in_progress");
        assert_eq!(shown["due"], "2026-11-05");
        assert_eq!(shown["effort"], "l");
        assert_eq!(shown["impact"], Value::Null);
        assert_eq!(shown["steps"][0]["done"], true);
        assert_eq!(shown["channel"], "c1");
        assert_eq!(shown["revision"], 4);
    }

    #[test]
    fn a_write_the_host_has_not_seen_yet_is_journaled_without_an_item() {
        let written = pb::WriteTodoResponse {
            delivery: pb::Delivery::Journaled as i32,
            journal: Some(pb::JournalPosition { machine: None, seq: 9 }),
            ..pb::WriteTodoResponse::default()
        };
        assert!(matches!(
            write_reply(&written, written_json(&written)),
            Reply::Journaled { .. }
        ));
        let done = pb::WriteTodoResponse::default();
        assert!(matches!(write_reply(&done, written_json(&done)), Reply::Applied { .. }));
    }

    fn ikey() -> pb::IdempotencyKey {
        pb::IdempotencyKey {
            value: "k1".to_owned(),
        }
    }

    #[test]
    fn todo_ask_question_builds_free_text_and_multi_choice_requests() {
        let free = ask_question_request(
            &args(json!({
                "key": "PAY-3",
                "question": "Which env?",
                "step_id": "st_1",
                "details": "Needed for **staging** config."
            })),
            ikey(),
        )
        .unwrap();
        assert_eq!(free.todo_item.as_ref().map(|i| i.value.as_str()), Some("PAY-3"));
        assert_eq!(free.step_id, "st_1");
        assert!(!free.multi_select);
        assert!(free.choices.is_empty());
        assert_eq!(free.details, "Needed for **staging** config.");

        let multi = ask_question_request(
            &args(json!({
                "key": "PAY-3",
                "question": "Pick targets",
                "choices": ["mac", "linux", "ci"],
                "multi_select": true
            })),
            ikey(),
        )
        .unwrap();
        assert!(multi.multi_select);
        assert_eq!(multi.choices, vec!["mac", "linux", "ci"]);

        let bad_multi = ask_question_request(
            &args(json!({
                "key": "PAY-3",
                "question": "Pick?",
                "multi_select": true
            })),
            ikey(),
        )
        .unwrap_err();
        assert_eq!(bad_multi.kind, ErrorKind::InvalidField);
    }

    #[test]
    fn todo_request_action_and_review_build_structured_choices() {
        let action = request_action_request(
            &args(json!({
                "key": "PAY-7",
                "action": "Rotate staging API key",
                "step_id": "st_2",
                "details": "In AWS Secrets Manager"
            })),
            ikey(),
        )
        .unwrap();
        assert_eq!(action.todo_item.as_ref().map(|i| i.value.as_str()), Some("PAY-7"));
        assert_eq!(action.step_id, "st_2");
        assert_eq!(action.question, "Rotate staging API key");
        assert_eq!(action.choices, vec!["Done", "Can't be done"]);
        assert!(!action.multi_select);

        let review = request_local_review_request(
            &args(json!({
                "key": "PAY-7",
                "title": "Schema migration",
                "channel": "ch-1",
                "summary": "Schema migration ready",
                "summary_why": "Needed for v2 queries",
                "details": "Addressed round 1 feedback",
                "layers": [
                    {
                        "repo": "acme/api",
                        "base_branch": "main",
                        "head_branch": "feat/schema",
                        "title": "Schema layer",
                        "pr_number": 42
                    }
                ]
            })),
            ikey(),
        )
        .unwrap();
        assert_eq!(review.id.as_ref().map(|i| i.value.as_str()), Some("PAY-7"));
        assert_eq!(review.title, "Schema migration");
        assert_eq!(review.channel.as_ref().map(|c| c.value.as_str()), Some("ch-1"));
        assert_eq!(review.summary_what, "Schema migration ready");
        assert_eq!(review.summary_why, "Needed for v2 queries");
        assert_eq!(review.summary_details, "Addressed round 1 feedback");
        assert_eq!(review.layers.len(), 1);
        assert_eq!(review.layers[0].repo, "acme/api");
        assert_eq!(review.layers[0].base_branch, "main");
        assert_eq!(review.layers[0].head_branch, "feat/schema");
        assert_eq!(review.layers[0].title, "Schema layer");
        assert_eq!(review.layers[0].pr_number, Some(42));

        let bad_layer_pr = request_local_review_request(
            &args(json!({
                "layers": [{ "repo": "acme/api", "head_branch": "feat/x", "pr_number": 0 }]
            })),
            ikey(),
        )
        .unwrap_err();
        assert_eq!(bad_layer_pr.kind, ErrorKind::InvalidField);
    }

    #[test]
    fn todo_step_update_preserves_order_markdown_context_and_drop_restore() {
        let update = step_update_request(
            &args(json!({
                "key": "PAY-9",
                "expected_revision": 3,
                "steps": [
                    { "id": "s2", "text": "Second moved first", "done": false, "markdown_context": "See `a.rs`", "dropped": false },
                    { "id": "s1", "text": "Old step", "done": false, "dropped": true }
                ]
            })),
            ikey(),
        )
        .unwrap();
        assert_eq!(update.id.as_ref().map(|i| i.value.as_str()), Some("PAY-9"));
        assert_eq!(update.expected_revision.as_ref().map(|r| r.value), Some(3));
        let steps = update.patch.unwrap().steps.unwrap().steps;
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].id, "s2");
        assert_eq!(steps[0].markdown_context, "See `a.rs`");
        assert!(!steps[0].dropped);
        assert_eq!(steps[1].id, "s1");
        assert!(steps[1].dropped);
    }

    #[test]
    fn todo_set_checkpoint_maps_all_valid_values_and_rejects_unknown() {
        for (raw, expected) in [
            ("with_context", pb::TodoCheckpoint::WithContext as i32),
            ("defined", pb::TodoCheckpoint::Defined as i32),
            ("done", pb::TodoCheckpoint::Done as i32),
        ] {
            let update = set_checkpoint_request(
                &args(json!({ "key": "PAY-4", "checkpoint": raw })),
                ikey(),
            )
            .unwrap();
            assert_eq!(update.patch.unwrap().checkpoint, Some(expected));
        }
        let err = set_checkpoint_request(
            &args(json!({ "key": "PAY-4", "checkpoint": "unknown" })),
            ikey(),
        )
        .unwrap_err();
        assert_eq!(err.kind, ErrorKind::InvalidField);
    }

    #[test]
    fn todo_report_run_formats_cost_and_links() {
        let (req, summary) = report_run_request(
            &args(json!({
                "key": "PAY-11",
                "tool": "claude",
                "model": "sonnet-4",
                "outcome": "Tests green",
                "cost_micros": 1_250_000,
                "links": [
                    { "label": "PR #42", "target": "https://github.com/acme/api/pull/42" }
                ]
            })),
            ikey(),
        )
        .unwrap();
        assert_eq!(req.id.as_ref().map(|i| i.value.as_str()), Some("PAY-11"));
        assert!(req.body.contains("Run: claude (sonnet-4) — Tests green · $1.25"));
        assert!(req.body.contains("- [PR #42](https://github.com/acme/api/pull/42)"));
        assert_eq!(summary["tool"], "claude");
        assert_eq!(summary["cost_micros"], 1_250_000);
        assert_eq!(summary["links"][0]["label"], "PR #42");
    }
}
