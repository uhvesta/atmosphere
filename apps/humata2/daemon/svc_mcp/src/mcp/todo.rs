//! The To-do tools of `humatad2 mcp`: an agent asks for the next item,
//! suggests one, or asks to close one through TodoService. The daemon makes
//! an agent's new item suggested and sends a close through the Edit rule,
//! so a close may come back as a pending approval.

use humata2_grpc::humata::v1 as pb;
use humata2_mcp::{Call, ErrorKind, Reply, ToolError};
use serde_json::{Map, Value, json};
use tonic::transport::Channel;

use super::boards::{expected_revision, read_reply, stale_as_of, strings};
use super::{GrpcDaemon, applied, key, lower, text};

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
}
