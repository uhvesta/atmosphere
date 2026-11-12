//! The agent's types as the protocol's messages.

use std::path::PathBuf;

use humata_agent::{
    Action, ApprovalMode, Changes, Edit, Event, EventKind, FileDiff, FileRef, FileStatus, Hunk,
    HunkAt, Line, LineKind, PermissionKind, Query, Range, Repository, SendMode, SessionRecord,
    SessionState,
};
use humata_wire::pb;

use crate::error::HostError;
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
        EventKind::MessageQueued { queue_id, text } => Kind::MessageQueued(pb::MessageQueued {
            queue_id: queue_id.clone(),
            text: text.clone(),
        }),
        EventKind::Dequeued { queue_id } => Kind::Dequeued(pb::Dequeued {
            queue_id: queue_id.clone(),
        }),
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
        seen_turns: 0,
        pending: pending(history),
        last_active_unix_ms: history.last().map_or(0, |event| event.time_unix_ms),
    }
}

/// The running turn's requests for permission that aren't answered yet.
fn pending(history: &[Event]) -> Vec<pb::PermissionRequested> {
    let mut pending: Vec<pb::PermissionRequested> = Vec::new();
    for event in history {
        match &event.kind {
            EventKind::TurnStarted | EventKind::TurnEnded { .. } => pending.clear(),
            EventKind::PermissionRequested {
                request_id,
                kind,
                summary,
            } => pending.push(pb::PermissionRequested {
                request_id: request_id.clone(),
                kind: permission_kind(*kind).into(),
                summary: summary.clone(),
            }),
            EventKind::PermissionCompleted { request_id, .. } => {
                pending.retain(|request| &request.request_id != request_id);
            }
            _ => {}
        }
    }
    pending
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

/// What a ReadChanges asks for.
pub fn query(read: &pb::ReadChanges) -> Result<Query, HostError> {
    let range = match &read.range {
        None => Range::Session,
        Some(range) => match range.kind() {
            pb::ChangeRangeKind::Session => Range::Session,
            pb::ChangeRangeKind::Unstaged => Range::Unstaged,
            pb::ChangeRangeKind::Staged => Range::Staged,
            pb::ChangeRangeKind::Branch if range.branch.is_empty() => {
                return Err(HostError::NoBranch);
            }
            pb::ChangeRangeKind::Branch => Range::Branch(range.branch.clone()),
            pb::ChangeRangeKind::Turn if range.turn == 0 => return Err(HostError::NoTurn),
            pb::ChangeRangeKind::Turn => Range::Turn(range.turn as usize),
        },
    };
    let only = read.file.as_ref().map(file_ref);
    if read.whole_file && only.is_none() {
        return Err(HostError::NoFile);
    }
    Ok(Query {
        range,
        context: read.context.unwrap_or(3),
        only,
        whole_file: read.whole_file,
    })
}

fn file_ref(file: &pb::FileRef) -> FileRef {
    FileRef {
        repository: file.repository.clone(),
        path: file.path.clone(),
    }
}

/// An EditChanges as the edit it asks for.
pub fn edit(edit: &pb::EditChanges) -> Result<Edit, HostError> {
    let file = edit.file.as_ref().map(file_ref).ok_or(HostError::NoFile)?;
    let action = match edit.action() {
        pb::ChangeAction::Unspecified => return Err(HostError::NoAction),
        pb::ChangeAction::Stage => Action::Stage,
        pb::ChangeAction::Unstage => Action::Unstage,
        pb::ChangeAction::Revert => Action::Revert,
    };
    Ok(Edit {
        file,
        action,
        hunk: edit.hunk.as_ref().map(|hunk| HunkAt {
            old_start: hunk.old_start,
            new_start: hunk.new_start,
            context: hunk.context,
        }),
    })
}

pub fn changes(changes: &Changes) -> pb::RepositoryChanges {
    pb::RepositoryChanges {
        name: changes.name.clone(),
        patch: changes.patch.clone(),
        files: changes.files.iter().map(file_diff).collect(),
    }
}

fn file_diff(file: &FileDiff) -> pb::FileDiff {
    let status = match file.status {
        FileStatus::Modified => pb::FileStatus::Modified,
        FileStatus::Added => pb::FileStatus::Added,
        FileStatus::Deleted => pb::FileStatus::Deleted,
        FileStatus::Renamed => pb::FileStatus::Renamed,
        FileStatus::Unchanged => pb::FileStatus::Unchanged,
    };
    pb::FileDiff {
        path: file.path.clone(),
        old_path: file.old_path.clone().unwrap_or_default(),
        status: status.into(),
        binary: file.binary,
        blob: file.blob.clone().unwrap_or_default(),
        hunks: file.hunks.iter().map(hunk).collect(),
        viewed: false,
    }
}

fn hunk(hunk: &Hunk) -> pb::Hunk {
    pb::Hunk {
        old_start: hunk.old_start,
        old_count: hunk.old_count,
        new_start: hunk.new_start,
        new_count: hunk.new_count,
        header: hunk.header.clone(),
        lines: hunk.lines.iter().map(line).collect(),
    }
}

fn line(line: &Line) -> pb::DiffLine {
    let kind = match line.kind {
        LineKind::Context => pb::DiffLineKind::Context,
        LineKind::Added => pb::DiffLineKind::Added,
        LineKind::Removed => pb::DiffLineKind::Removed,
    };
    pb::DiffLine {
        kind: kind.into(),
        old: line.old.unwrap_or(0),
        new: line.new.unwrap_or(0),
        text: line.text.clone(),
    }
}

#[cfg(test)]
mod tests {
    use humata_agent::EventId;

    use super::*;

    fn event(kind: EventKind) -> Event {
        Event {
            id: EventId::new(format!("{kind:?}")),
            time_unix_ms: 0,
            kind,
        }
    }

    fn asked(request: &str) -> Event {
        event(EventKind::PermissionRequested {
            request_id: request.into(),
            kind: PermissionKind::Write,
            summary: format!("write {request}"),
        })
    }

    fn answered(request: &str) -> Event {
        event(EventKind::PermissionCompleted {
            request_id: request.into(),
            approved: true,
            automatic: false,
        })
    }

    fn ids(history: &[Event]) -> Vec<String> {
        pending(history)
            .into_iter()
            .map(|request| request.request_id)
            .collect()
    }

    #[test]
    fn only_the_running_turns_unanswered_requests_are_pending() {
        let first = [
            event(EventKind::TurnStarted),
            asked("a"),
            asked("b"),
            answered("a"),
        ];
        assert_eq!(ids(&first), ["b"]);
        let mut ended = first.to_vec();
        ended.push(event(EventKind::TurnEnded { aborted: true }));
        assert!(ids(&ended).is_empty());
        ended.extend([event(EventKind::TurnStarted), asked("c")]);
        assert_eq!(ids(&ended), ["c"]);
    }
}
