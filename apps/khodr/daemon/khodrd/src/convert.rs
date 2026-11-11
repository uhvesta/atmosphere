//! The agent's types as the protocol's messages.

use std::path::PathBuf;

use khodr_agent::{
    ApprovalMode, Event, EventKind, PermissionKind, Repository, SendMode, SessionRecord,
    SessionState,
};
use khodr_wire::pb;

use crate::store::Project;

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
            automatic,
        } => Kind::PermissionCompleted(pb::PermissionCompleted {
            request_id: request_id.clone(),
            approved: *approved,
            automatic: *automatic,
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
        EventKind::Checkpoint { snapshots } => Kind::Checkpoint(pb::Checkpoint {
            snapshots: snapshots
                .iter()
                .map(|snapshot| pb::Snapshot {
                    name: snapshot.name.clone(),
                    head: snapshot.head.clone(),
                    commit: snapshot.commit.clone(),
                })
                .collect(),
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
        approval_mode: match record.approval {
            ApprovalMode::AskEveryTime => pb::ApprovalMode::AskEveryTime,
            ApprovalMode::AllowEdits => pb::ApprovalMode::AllowEdits,
            ApprovalMode::AllowEverything => pb::ApprovalMode::AllowEverything,
        }
        .into(),
        project_id: String::new(),
        host_offline: false,
        checkouts: record
            .checkouts
            .iter()
            .map(|checkout| pb::Checkout {
                name: checkout.name.clone(),
                path: checkout.path.display().to_string(),
                branch: checkout.branch.clone(),
            })
            .collect(),
        archived: record.archived,
        keeper: record.keeper,
        item_id: String::new(),
        turns: u32::try_from(
            history
                .iter()
                .filter(|event| matches!(event.kind, EventKind::TurnEnded { .. }))
                .count(),
        )
        .unwrap_or(u32::MAX),
    }
}

pub fn approval_mode(mode: pb::ApprovalMode) -> ApprovalMode {
    match mode {
        pb::ApprovalMode::AskEveryTime | pb::ApprovalMode::Unspecified => {
            ApprovalMode::AskEveryTime
        }
        pb::ApprovalMode::AllowEdits => ApprovalMode::AllowEdits,
        pb::ApprovalMode::AllowEverything => ApprovalMode::AllowEverything,
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

pub fn repository(repository: pb::Repository) -> Repository {
    Repository {
        name: repository.name,
        path: PathBuf::from(repository.path),
    }
}

pub fn wire_repository(repository: &Repository) -> pb::Repository {
    pb::Repository {
        name: repository.name.clone(),
        path: repository.path.display().to_string(),
    }
}

pub fn project(project: Project) -> pb::Project {
    pb::Project {
        id: project.id,
        name: project.name,
        repositories: project.repositories.iter().map(wire_repository).collect(),
        instructions: project.instructions,
        created_unix_ms: project.created_unix_ms,
    }
}
