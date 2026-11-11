//! The agent's types as the protocol's messages.

use khodr_agent::{Event, EventKind, PermissionKind, SendMode, SessionRecord, SessionState};
use khodr_wire::pb;

pub fn event(event: &Event) -> pb::SessionEvent {
    use pb::session_event::Kind;
    let kind = match &event.kind {
        EventKind::UserMessage { text } => {
            Kind::UserMessage(pb::UserMessage { text: text.clone() })
        }
        EventKind::TurnStarted => Kind::TurnStarted(pb::TurnStarted {}),
        EventKind::AssistantDelta { text } => {
            Kind::AssistantDelta(pb::AssistantDelta { text: text.clone() })
        }
        EventKind::AssistantMessage { text } => {
            Kind::AssistantMessage(pb::AssistantMessage { text: text.clone() })
        }
        EventKind::Reasoning { text } => Kind::Reasoning(pb::Reasoning { text: text.clone() }),
        EventKind::ToolStarted {
            call_id,
            tool,
            arguments_json,
        } => Kind::ToolStarted(pb::ToolStarted {
            call_id: call_id.clone(),
            tool: tool.clone(),
            arguments_json: arguments_json.clone(),
        }),
        EventKind::ToolCompleted {
            call_id,
            success,
            output,
        } => Kind::ToolCompleted(pb::ToolCompleted {
            call_id: call_id.clone(),
            success: *success,
            output: output.clone(),
        }),
        EventKind::PermissionRequested {
            request_id,
            kind,
            summary,
        } => Kind::PermissionRequested(pb::PermissionRequested {
            request_id: request_id.clone(),
            kind: permission_kind(*kind).into(),
            summary: summary.clone(),
        }),
        EventKind::PermissionCompleted {
            request_id,
            approved,
        } => Kind::PermissionCompleted(pb::PermissionCompleted {
            request_id: request_id.clone(),
            approved: *approved,
        }),
        EventKind::TurnEnded { aborted } => Kind::TurnEnded(pb::TurnEnded { aborted: *aborted }),
        EventKind::Idle => Kind::Idle(pb::SessionIdle {}),
        EventKind::Failed { message } => Kind::Failed(pb::SessionFailed {
            message: message.clone(),
        }),
        EventKind::Usage {
            model,
            input_tokens,
            output_tokens,
            premium_requests,
        } => Kind::Usage(pb::Usage {
            model: model.clone(),
            input_tokens: *input_tokens,
            output_tokens: *output_tokens,
            premium_requests: *premium_requests,
        }),
    };
    pb::SessionEvent {
        id: event.id.to_string(),
        time_unix_ms: event.time_unix_ms,
        kind: Some(kind),
    }
}

fn permission_kind(kind: PermissionKind) -> pb::PermissionKind {
    match kind {
        PermissionKind::Shell => pb::PermissionKind::Shell,
        PermissionKind::Write => pb::PermissionKind::Write,
        PermissionKind::Read => pb::PermissionKind::Read,
        PermissionKind::Other => pb::PermissionKind::Other,
    }
}

pub fn session(record: &SessionRecord, backend: pb::Backend, history: &[Event]) -> pb::SessionInfo {
    let state = match SessionState::after(history) {
        SessionState::Idle => pb::SessionState::Idle,
        SessionState::Working => pb::SessionState::Working,
        SessionState::WaitingForApproval => pb::SessionState::WaitingForApproval,
        SessionState::Failed => pb::SessionState::Failed,
    };
    let parent = record.parent.as_ref();
    pb::SessionInfo {
        id: record.id.to_string(),
        name: record.name.clone(),
        backend: backend.into(),
        working_directory: record.working_directory.display().to_string(),
        model: record.model.clone().unwrap_or_default(),
        state: state.into(),
        parent_id: parent
            .map(|parent| parent.session.to_string())
            .unwrap_or_default(),
        forked_before_event_id: parent
            .and_then(|parent| parent.before.as_ref())
            .map(ToString::to_string)
            .unwrap_or_default(),
        host: String::new(),
    }
}

pub fn send_mode(mode: pb::SendMode) -> SendMode {
    match mode {
        pb::SendMode::Immediate => SendMode::Immediate,
        pb::SendMode::Enqueue | pb::SendMode::Unspecified => SendMode::Enqueue,
    }
}

/// None for an empty string, which proto3 sends for an unset one.
pub fn optional(text: String) -> Option<String> {
    Some(text).filter(|text| !text.is_empty())
}
