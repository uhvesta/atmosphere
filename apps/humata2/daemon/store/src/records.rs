//! The records the store keeps, as plain data.

use std::ffi::OsString;
use std::path::PathBuf;

/// A session as this machine knows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionRecord {
    pub id: String,
    pub channel: String,
    pub machine: String,
    pub title: String,
    /// The agent as named in Settings › Agents ("claude", "zsh", …).
    pub agent: String,
    /// The session's folder, where its shells start.
    pub folder: PathBuf,
    pub created_unix_ms: i64,
}

/// What runs in a terminal tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabKind {
    Agent,
    Shell,
}

/// A terminal tab and the holder process that keeps its PTY.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabRecord {
    pub id: String,
    /// Unset for a machine shell, which belongs to no session.
    pub session: Option<String>,
    pub kind: TabKind,
    pub title: String,
    /// The holder's own socket.
    pub holder_socket: PathBuf,
    pub holder_pid: u32,
    pub created_unix_ms: i64,
}

/// Where a session's clones stand (SL-3).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pointers {
    pub repos: Vec<RepoPointer>,
    pub reported_unix_ms: i64,
}

/// One clone's position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoPointer {
    pub owner: String,
    pub name: String,
    pub branch: String,
    pub sha: String,
    /// The commit `sha` is on the remote.
    pub pushed: bool,
    /// Repo-relative paths with uncommitted changes.
    pub dirty_files: Vec<String>,
}

/// A session's agent: how it was started, so it can be started again
/// after a crash or reboot (PER-11), and how it stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentRecord {
    /// The session the agent belongs to; one agent per session.
    pub session: String,
    /// The terminal tab the agent runs in.
    pub tab: String,
    pub launch: AgentLaunch,
    pub state: AgentState,
    /// How many times the agent has been started: 1 at launch, one more
    /// for each resume.
    pub generation: u32,
    /// Each time the agent's holder was found gone, oldest first.
    pub interruptions: Vec<Interruption>,
    pub updated_unix_ms: i64,
}

/// A session's review, kept so a restart keeps its review chat (PR-26).
/// The store keeps what the daemon encodes without reading it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReviewRecord {
    pub session: String,
    /// The review chat: its view, notes and switches, as the daemon
    /// encodes them.
    pub chat: Vec<u8>,
    /// What the last diff showed, one encoded diff per repository.
    pub shown: Vec<Vec<u8>>,
    /// The last change the chat numbered, so numbering carries on.
    pub last_seq: u64,
    /// The last note the chat numbered, so a dropped note's id stays used.
    pub last_note: u64,
    /// Whether the user's view was reported, so the chat is a review chat.
    pub view_reported: bool,
    pub updated_unix_ms: i64,
}

/// Everything needed to start an agent again in the same conversation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentLaunch {
    /// The agent as named in Settings › Agents.
    pub agent: String,
    pub folder: PathBuf,
    /// The program and its arguments at launch, the pinned conversation id
    /// among them (PER-12).
    pub argv: Vec<OsString>,
    /// The program and its arguments to resume the same conversation.
    pub resume_argv: Vec<OsString>,
    /// The names of the variables set for the agent. Their values are
    /// never saved: tokens are minted again on resume.
    pub variable_names: Vec<String>,
    pub conversation_id: String,
    /// The file the agent keeps the conversation in, checked on resume
    /// (PER-14).
    pub conversation_file: PathBuf,
    pub holder_socket: PathBuf,
    pub cols: u16,
    pub rows: u16,
}

/// Where an agent stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentState {
    /// Saved, and its holder may or may not have started.
    Launching,
    /// Its holder started and answered.
    Running { holder_pid: u32 },
    /// Found gone and being started again in its conversation.
    Resuming,
    /// The agent ended on its own; nothing is resumed.
    Exited { code: i32 },
    /// The user closed it; nothing is resumed.
    Stopped,
    /// Starting it again in its conversation failed; it is never started
    /// fresh in its place (PER-15).
    ResumeFailed {
        reason: ResumeFailure,
        exit_code: Option<i32>,
    },
}

impl AgentState {
    /// Whether the agent should be running, so a missing holder means it
    /// was interrupted.
    pub fn should_run(self) -> bool {
        matches!(self, Self::Launching | Self::Running { .. } | Self::Resuming)
    }
}

/// Why a resume failed, as `ResumeFailed.Reason` says it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResumeFailure {
    ConversationFileMissing,
    AgentStartedAnotherConversation,
    AgentExited,
    AgentNotInstalled,
    FolderMissing,
}

/// One time an agent's holder was found gone, and what of its output
/// survived.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interruption {
    pub at_unix_ms: i64,
    /// The offset just past the last byte of output saved to disk.
    pub output_saved_through: u64,
    /// The offset the restarted terminal's output starts at; the gap from
    /// `output_saved_through` was never saved.
    pub output_resumes_at: u64,
    /// The end of the saved output was cut short and dropped.
    pub saved_tail_damaged: bool,
    /// Set just before the resumed agent is told it was interrupted, so a
    /// resume redone after a crash knows the note may already be in its
    /// conversation.
    pub told: Option<Told>,
}

/// The resumed agent being told it was interrupted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Told {
    pub at_unix_ms: i64,
    /// The conversation file's length just before: the note, if the agent
    /// wrote it down, is after this.
    pub conversation_len: u64,
}
