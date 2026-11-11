//! What a session is made of: its record and its events.

use std::fmt;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use humata_workspace::{Checkout, Repository, Snapshot};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(String);

impl SessionId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub(crate) fn random() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Unique within a session; a fork's copies keep their ids.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EventId(String);

impl EventId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub(crate) fn random() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for EventId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub id: EventId,
    /// Milliseconds since 1970 in UTC.
    pub time_unix_ms: i64,
    pub kind: EventKind,
}

impl Event {
    pub(crate) fn now(kind: EventKind) -> Self {
        let time_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| {
                i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
            });
        Self {
            id: EventId::random(),
            time_unix_ms,
            kind,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventKind {
    UserMessage {
        text: String,
    },
    TurnStarted,
    /// Part of the reply as it streams; the AssistantMessage after holds all of it.
    AssistantDelta {
        text: String,
    },
    AssistantMessage {
        text: String,
    },
    Reasoning {
        text: String,
    },
    ToolStarted {
        call_id: String,
        tool: String,
        arguments_json: String,
    },
    ToolCompleted {
        call_id: String,
        success: bool,
        output: String,
    },
    PermissionRequested {
        request_id: String,
        kind: PermissionKind,
        summary: String,
    },
    PermissionCompleted {
        request_id: String,
        approved: bool,
        /// Answered by the session's [`ApprovalMode`], not by a person.
        #[serde(default)]
        automatic: bool,
    },
    TurnEnded {
        aborted: bool,
    },
    Idle,
    Failed {
        message: String,
    },
    Usage {
        model: String,
        input_tokens: u64,
        output_tokens: u64,
        premium_requests: f64,
    },
    /// The session's code at this point: each repository's snapshot, taken
    /// when its copy is made and at the end of every turn. A fork with the
    /// code starts from the last one before where it forks.
    Checkpoint {
        snapshots: Vec<Snapshot>,
    },
}

impl EventKind {
    /// The snapshots of a checkpoint; None for any other event.
    pub fn snapshots(&self) -> Option<&[Snapshot]> {
        match self {
            Self::Checkpoint { snapshots } => Some(snapshots),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionKind {
    Shell,
    Write,
    Read,
    Other,
}

/// What a session may do without asking.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalMode {
    #[default]
    AskEveryTime,
    /// Reads and file writes go ahead; commands and anything else still ask.
    AllowEdits,
    AllowEverything,
}

impl ApprovalMode {
    /// Whether a request of `kind` goes ahead without asking.
    pub fn allows(self, kind: PermissionKind) -> bool {
        match self {
            Self::AskEveryTime => false,
            Self::AllowEdits => matches!(kind, PermissionKind::Read | PermissionKind::Write),
            Self::AllowEverything => true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendMode {
    /// Start a turn now, or after the running one.
    Enqueue,
    /// Steer the running turn; start one if none is running.
    Immediate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionState {
    Idle,
    Working,
    WaitingForApproval,
    Failed,
}

impl SessionState {
    /// The state a session is in after `events`.
    pub fn after(events: &[Event]) -> Self {
        let mut state = Self::Idle;
        let mut failed = false;
        let mut pending = 0_usize;
        for event in events {
            match &event.kind {
                EventKind::TurnStarted => {
                    state = Self::Working;
                    failed = false;
                    pending = 0;
                }
                EventKind::PermissionRequested { .. } => pending += 1,
                EventKind::PermissionCompleted { .. } => pending = pending.saturating_sub(1),
                EventKind::Failed { .. } => failed = true,
                EventKind::TurnEnded { .. } => {
                    state = if failed { Self::Failed } else { Self::Idle };
                    pending = 0;
                }
                _ => {}
            }
        }
        if state == Self::Working && pending > 0 {
            Self::WaitingForApproval
        } else {
            state
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct StartOptions {
    pub name: String,
    /// The folder the agent works in.
    pub working_directory: PathBuf,
    /// None for the backend's default.
    pub model: Option<String>,
    /// Instructions for the agent, for the life of the session.
    pub instructions: Option<String>,
    /// Settings only the backend reads, as JSON: the scripted backend's script.
    pub backend_config: Option<String>,
    pub approval: ApprovalMode,
    /// Repositories the session gets its own copy of, before its first turn;
    /// it then works in that copy, not in `working_directory`.
    pub repositories: Vec<Repository>,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ForkOptions {
    /// Keep the events before this one, not including it. None keeps them all.
    pub before: Option<EventId>,
    /// None takes the parent's name.
    pub name: Option<String>,
    /// Keep working in the parent's copy of the code, not a copy of the code
    /// as it was at `before`.
    pub chat_only: bool,
}

/// Changes to a session's record; None leaves a field as it is.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct SessionUpdate {
    pub name: Option<String>,
    /// An archived session's copy of the code is removed, after a snapshot,
    /// and it takes no messages; unarchived, its copy comes back as it was.
    pub archived: Option<bool>,
    /// Marks the session whose changes are the ones to keep.
    pub keeper: Option<bool>,
}

/// Where a fork came from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Parent {
    pub session: SessionId,
    pub before: Option<EventId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: SessionId,
    pub name: String,
    pub working_directory: PathBuf,
    pub model: Option<String>,
    pub instructions: Option<String>,
    #[serde(default)]
    pub backend_config: Option<String>,
    /// None for a root session.
    pub parent: Option<Parent>,
    #[serde(default)]
    pub approval: ApprovalMode,
    #[serde(default)]
    pub repositories: Vec<Repository>,
    /// Its worktrees, once made; a fork of the chat only shares its
    /// parent's.
    #[serde(default)]
    pub checkouts: Vec<Checkout>,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub keeper: bool,
}
