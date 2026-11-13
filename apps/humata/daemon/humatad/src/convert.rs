//! The agent's types as the protocol's messages.

use std::path::PathBuf;

use humata_agent::{
    Action, ApprovalMode, Changes, Edit, Event, EventKind, FileDiff, FileRef, FileStatus, Hunk,
    HunkAt, Image, Line, LineKind, PermissionKind, Query, Range, Repository, SendMode,
    SessionRecord, SessionState, Summary,
};
use humata_wire::pb;

use crate::error::HostError;
use crate::store::{Project, Skill};

pub fn event(event: &Event) -> pb::SessionEvent {
    use pb::session_event::Kind;
    let kind = match &event.kind {
        EventKind::UserMessage { text, images } => Kind::UserMessage(pb::UserMessage {
            text: text.clone(),
            images: images.clone(),
        }),
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
            judged,
        } => Kind::PermissionCompleted(pb::PermissionCompleted {
            request_id: request_id.clone(),
            approved: *approved,
            automatic: *automatic,
            judged: *judged,
        }),
        EventKind::TurnEnded { aborted } => Kind::TurnEnded(pb::TurnEnded { aborted: *aborted }),
        EventKind::MessageQueued {
            queue_id,
            text,
            images,
        } => Kind::MessageQueued(pb::MessageQueued {
            queue_id: queue_id.clone(),
            text: text.clone(),
            images: images.clone(),
        }),
        EventKind::Dequeued { queue_id, dropped } => Kind::Dequeued(pb::Dequeued {
            queue_id: queue_id.clone(),
            dropped: *dropped,
        }),
        EventKind::Idle => Kind::Idle(pb::SessionIdle {}),
        EventKind::Failed { message } => Kind::Failed(pb::SessionFailed {
            message: message.clone(),
        }),
        EventKind::Notice { text } => Kind::Notice(pb::Notice { text: text.clone() }),
        EventKind::Rewound {
            text,
            images,
            files,
            ..
        } => Kind::Rewound(pb::Rewound {
            text: text.clone(),
            images: images.clone(),
            files: *files,
        }),
        EventKind::RewindUndone => Kind::RewindUndone(pb::RewindUndoneEvent {}),
        EventKind::Compacted => Kind::Compacted(pb::Compacted {}),
        EventKind::Waiting { reason } => Kind::Waiting(pb::Waiting {
            reason: reason.clone(),
        }),
        EventKind::Usage {
            model,
            input_tokens,
            output_tokens,
            premium_requests,
            cache_read_tokens,
            cache_write_tokens,
        } => Kind::Usage(pb::Usage {
            model: model.clone(),
            input_tokens: *input_tokens,
            output_tokens: *output_tokens,
            premium_requests: *premium_requests,
            cache_read_tokens: *cache_read_tokens,
            cache_write_tokens: *cache_write_tokens,
        }),
        EventKind::Checkpoint { snapshots } => Kind::Checkpoint(pb::Checkpoint {
            snapshots: wire_snapshots(snapshots),
        }),
        EventKind::Saved { snapshots } => Kind::Saved(pb::Saved {
            snapshots: wire_snapshots(snapshots),
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

pub fn session(record: &SessionRecord, backend: pb::Backend, summary: &Summary) -> pb::SessionInfo {
    let state = match summary.state() {
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
            ApprovalMode::Auto => pb::ApprovalMode::Auto,
        }
        .into(),
        project_id: String::new(),
        planning: false,
        asking: false,
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
        item_title: String::new(),
        effort: record.effort.clone().unwrap_or_default(),
        experimental: record.experimental,
        turns: summary.turns,
        seen_turns: 0,
        pending: summary
            .pending
            .iter()
            .map(|request| pb::PermissionRequested {
                request_id: request.request_id.clone(),
                kind: permission_kind(request.kind).into(),
                summary: request.summary.clone(),
            })
            .collect(),
        last_active_unix_ms: summary.last_active_unix_ms,
        last_message: summary.last_message.clone(),
    }
}

pub fn approval_mode(mode: pb::ApprovalMode) -> ApprovalMode {
    match mode {
        pb::ApprovalMode::AskEveryTime | pb::ApprovalMode::Unspecified => {
            ApprovalMode::AskEveryTime
        }
        pb::ApprovalMode::AllowEdits => ApprovalMode::AllowEdits,
        pb::ApprovalMode::AllowEverything => ApprovalMode::AllowEverything,
        pb::ApprovalMode::Auto => ApprovalMode::Auto,
    }
}

pub fn send_mode(mode: pb::SendMode) -> SendMode {
    match mode {
        pb::SendMode::Immediate => SendMode::Immediate,
        pb::SendMode::EnqueueOnce => SendMode::EnqueueOnce,
        pb::SendMode::Enqueue | pb::SendMode::Unspecified => SendMode::Enqueue,
    }
}

/// None for an empty string, which proto3 sends for an unset one.
pub fn image(image: pb::Image) -> Image {
    Image {
        media_type: image.media_type,
        data: image.data,
    }
}

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
        url: String::new(),
    }
}

pub fn project(project: Project) -> pb::Project {
    pb::Project {
        id: project.id,
        name: project.name,
        repositories: project
            .repositories
            .iter()
            .map(|repository| pb::Repository {
                url: project
                    .urls
                    .get(&repository.name)
                    .cloned()
                    .unwrap_or_default(),
                ..wire_repository(repository)
            })
            .collect(),
        instructions: project.instructions,
        created_unix_ms: project.created_unix_ms,
        skills: project
            .skills
            .into_iter()
            .map(|skill| pb::Skill {
                name: skill.name,
                text: skill.text,
            })
            .collect(),
        plugins: project.plugins,
        defaults: project.defaults,
    }
}

pub fn skills(skills: Vec<pb::Skill>) -> Vec<Skill> {
    skills
        .into_iter()
        .map(|skill| Skill {
            name: skill.name.trim().to_owned(),
            text: skill.text,
        })
        .filter(|skill| !skill.name.is_empty() || !skill.text.trim().is_empty())
        .collect()
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

fn wire_snapshots(snapshots: &[humata_agent::Snapshot]) -> Vec<pb::Snapshot> {
    snapshots
        .iter()
        .map(|snapshot| pb::Snapshot {
            name: snapshot.name.clone(),
            head: snapshot.head.clone(),
            commit: snapshot.commit.clone(),
        })
        .collect()
}
