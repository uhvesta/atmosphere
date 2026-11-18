//! Each agent CLI's own conversation files, read and written the way that
//! CLI keeps them (CV-1–CV-7, D307): list a conversation's turns, fork it
//! before a turn into a new id, and carry it to another home and folder.
//!
//! One [`Adapter`] per [`Format`], each in its own module: Claude Code's
//! JSONL ([`Format::ClaudeJsonl`]), Codex's rollout files and SQLite state
//! ([`Format::CodexRollout`]) and Copilot's session-state folder
//! ([`Format::CopilotSessionState`]). [`adapter`] gives the one for a
//! format.
//!
//! The crate is synchronous, knows nothing about gRPC, sessions or the
//! store, and touches only the files of the [`Placement`]s it is given
//! (and, for a Codex conversation moved on one machine, that thread's row
//! in the CLI's `state_5.sqlite`, D336).
//! It never changes a parent conversation's files, never overwrites a file
//! that is already there, and never copies bytes it can't read: a format it
//! was not proven against, or a record its strict parse doesn't know, is
//! [`ConversationError::Unsupported`] (CV-3, CV-7), and there is no
//! byte-copy fallback.
//!
//! Every method that reads a conversation runs the adapter's strict parse
//! first, so none of them reads past a record it doesn't know. The CLI
//! version doesn't decide by itself: the CLI updates itself, and a newer
//! version whose records all parse is known (CV-7). Callers run
//! [`Adapter::check`] with the CLI version recorded at launch to keep the
//! result beside that version (CV-3).

mod claude;
mod codex;
mod copilot;
mod error;
mod files;

use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub use claude::project_name as claude_project_name;
pub use error::ConversationError;

/// How an agent CLI keeps its conversation on disk. Serialized in
/// kebab-case (`claude-jsonl`, `codex-rollout`, `copilot-session-state`), as
/// an agent kind's `conversation_format` names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Format {
    /// Claude Code: one JSON object per line in
    /// `<config>/projects/<dashed folder>/<id>.jsonl`, where `<config>` is
    /// `CLAUDE_CONFIG_DIR`, or `~/.claude` when that is unset.
    ClaudeJsonl,
    /// Codex: one `rollout-*.jsonl` per conversation under
    /// `sessions/YYYY/MM/DD/` in `CODEX_HOME` (or `~/.codex`), whose first
    /// line (`session_meta`) names its id and folder, and the thread's row
    /// in the CLI's `state_5.sqlite` (CV-7).
    CodexRollout,
    /// Copilot: `session-state/<id>/` under `COPILOT_HOME` (or
    /// `~/.copilot`), as Copilot CLI 1.0.112-0 and later write it (CV-7).
    CopilotSessionState,
}

impl fmt::Display for Format {
    /// The CLI's name, for messages: "Claude Code", "Codex" or "Copilot".
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::ClaudeJsonl => "Claude Code",
            Self::CodexRollout => "Codex",
            Self::CopilotSessionState => "Copilot",
        })
    }
}

/// Where one agent's CLI runs: what the adapter needs to find that CLI's
/// files for a conversation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placement {
    /// The home folder the CLI runs with (`HOME`).
    pub home: PathBuf,
    /// The CLI's own home when its env var is set for it
    /// (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `COPILOT_HOME`). `None` means the
    /// CLI's default under [`Placement::home`] (`~/.claude` for Claude).
    pub cli_home: Option<PathBuf>,
    /// The folder the agent runs in (its cwd). The Claude adapter finds a
    /// conversation under the folder as given, else under its canonical
    /// form (macOS `/var` is `/private/var`), where the CLI recorded it, and
    /// places under the canonical form of a folder that exists (D336).
    pub folder: PathBuf,
}

/// One conversation and every file that belongs to it, as found by
/// [`Adapter::locate`] or written by [`Adapter::fork`] or [`Adapter::place`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conversation {
    /// The CLI's own conversation id: what the agent resumes by (PER-12).
    pub id: String,
    /// How its files are kept.
    pub format: Format,
    /// Where the CLI that owns it runs.
    pub placement: Placement,
    /// Its files: the [`FileRole::Transcript`] first, then the others in
    /// path order. Paths are absolute.
    pub files: Vec<ConversationFile>,
}

/// One file of a [`Conversation`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversationFile {
    /// Where it is.
    pub path: PathBuf,
    /// What it is to the conversation.
    pub role: FileRole,
}

/// What a file is to its conversation, which decides whether a fork or a
/// carry takes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FileRole {
    /// The file the turns are read from and a fork cuts. A conversation has
    /// exactly one.
    Transcript,
    /// A file kept beside the transcript for this conversation alone, such
    /// as Claude's `<id>/` folder (side conversations, saved tool output).
    /// Carried; not copied by a fork.
    Companion,
    /// A file the CLI keeps per folder rather than per conversation, such as
    /// Claude's `memory/`. Carried by a move (MV-13); not copied by a fork.
    /// Another conversation of the folder is never listed (MV-16).
    Project,
}

/// One turn of a conversation: a prompt the user sent on the conversation's
/// current path, not injected context, a tool result, a side conversation
/// or a prompt abandoned by a rewind or an edit (CV-2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Turn {
    /// The turn's number, from 1, in the order of the current path.
    pub index: u32,
    /// The first line of the prompt's text.
    pub first_line: String,
    /// When it was sent, as the CLI wrote it (RFC 3339), if it did.
    pub at: Option<String>,
}

/// A conversation's files copied into a stage folder for a carry (CV-4,
/// MV-14), as [`Adapter::pack`] wrote them. It travels in the move manifest;
/// on the new machine, set [`Packed::stage`] to where the files arrived
/// before calling [`Adapter::place`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Packed {
    /// The conversation's id, kept on the new machine.
    pub id: String,
    /// How its files are kept.
    pub format: Format,
    /// The folder the files were copied into.
    pub stage: PathBuf,
    /// Every staged file.
    pub files: Vec<PackedFile>,
}

/// One staged file of a [`Packed`] conversation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackedFile {
    /// Its path under [`Packed::stage`], with `/` between parts. It is
    /// also its path under the CLI's per-folder record on the new machine
    /// (for Claude, under `projects/<dashed folder>/`). Never absolute and
    /// never holds `..`; [`Adapter::place`] refuses one that does.
    pub rel: String,
    /// What it is to the conversation.
    pub role: FileRole,
    /// Its size in bytes as staged.
    pub size: u64,
}

/// What Humata can do with one CLI's conversation files.
///
/// Every method leaves the files it reads as they were, writes new files
/// through a `.partial` beside them that is renamed into place, and refuses
/// rather than overwrite a file that is already there.
pub trait Adapter: Send + Sync {
    /// The conversation `id` of the CLI running at `at`, with all its files.
    ///
    /// Refuses an `id` that isn't one plain file name
    /// ([`ConversationError::BadId`]) and a conversation whose transcript
    /// isn't there ([`ConversationError::NotFound`]).
    fn locate(&self, at: &Placement, id: &str) -> Result<Conversation, ConversationError>;

    /// The conversation's turns on its current path, in order (CV-2): a
    /// prompt abandoned by a rewind or replaced by an edited prompt is not
    /// one. A record still being written is left out. A complete line that
    /// isn't a record of the format is [`ConversationError::Malformed`],
    /// naming the line; a record the strict parse doesn't know is
    /// [`ConversationError::Unsupported`], and the caller lists no turns.
    fn turns(&self, conversation: &Conversation) -> Result<Vec<Turn>, ConversationError>;

    /// Where each of [`Adapter::turns`]'s turns starts in the transcript, in
    /// bytes and in the same order, for a format whose fork keeps the lines
    /// before a turn's start: the bytes before turn n's start are what a fork
    /// before turn n holds, as the parent has them. A format whose fork keeps
    /// only some of those lines (Claude's current path) has no such starts
    /// and is [`ConversationError::Unsupported`].
    fn turn_starts(&self, conversation: &Conversation) -> Result<Vec<u64>, ConversationError> {
        Err(ConversationError::Unsupported {
            format: conversation.format,
            version: None,
        })
    }

    /// Writes a new conversation `new_id` for the agent running at `to` that
    /// holds exactly the turns of the current path before turn `before`, in
    /// the CLI's own format (CV-2). With `before` `None` (fork from now) it
    /// holds every finished turn: while a turn runs, the fork ends after the
    /// last finished one. Records name `new_id` as their conversation and
    /// `to`'s folder in place of the parent's. Only the transcript is
    /// copied, and only complete records.
    ///
    /// Never changes the parent's files. Refuses a fork that would hold no
    /// turn ([`ConversationError::NothingBefore`]: `before` 1, or from now
    /// while the first turn runs), a turn the conversation doesn't have
    /// ([`ConversationError::NoSuchTurn`]), and a fork whose file is
    /// already there ([`ConversationError::Exists`]).
    fn fork(
        &self,
        conversation: &Conversation,
        before: Option<u32>,
        new_id: &str,
        to: &Placement,
    ) -> Result<Conversation, ConversationError>;

    /// Copies the conversation into `stage` for a carry (CV-4): its other
    /// files as they are, and its transcript's records of the current path
    /// as they are, so an abandoned branch is not carried (CV-2). Checks
    /// every file before copying any, so a refusal leaves `stage` holding no
    /// files: a file over the rewrite cap ([`ConversationError::TooBig`]),
    /// one that isn't text ([`ConversationError::NotText`]), a malformed or
    /// unknown transcript record, or a `stage` that already holds files
    /// ([`ConversationError::Exists`]).
    fn pack(&self, conversation: &Conversation, stage: &Path) -> Result<Packed, ConversationError>;

    /// Places a packed conversation where the CLI running at `to` looks for
    /// it, rewriting `from`'s folder to `to`'s in the text that holds it: the
    /// path as it is, as JSON escapes it, and dashed as the CLI names its
    /// per-folder record, plus the old per-folder record's own path. The
    /// conversation keeps its id. Refuses before writing anything when a
    /// file would land on one that is there ([`ConversationError::Exists`])
    /// or a staged name leaves its folder ([`ConversationError::BadPath`]).
    fn place(&self, packed: &Packed, from: &Placement, to: &Placement) -> Result<Conversation, ConversationError>;

    /// Whether this adapter can read the conversation (CV-3, CV-7): its
    /// format was proven against the real CLI and its strict parse accepts
    /// every record. `cli_version` is the version recorded at launch
    /// (`None` when none was); it doesn't decide by itself, and names the
    /// version in the refusal when the records name none. A record the
    /// parse doesn't know is [`ConversationError::Unsupported`]; a
    /// malformed line is [`ConversationError::Malformed`].
    fn check(&self, conversation: &Conversation, cli_version: Option<&str>) -> Result<(), ConversationError>;

    /// As [`Adapter::place`], and when `head` is given, the conversation's
    /// own record of its git branch and commit names `head`: a carry onto a
    /// new clone of the session's repo (MV-16, D336). For Codex that is the
    /// session line's `payload.git.branch` and `commit_hash` (its remote is
    /// kept) and, on the same machine, the thread index row's `git_branch`
    /// and `git_sha`; Claude's records name only the branch, as do Copilot's
    /// session start and `workspace.yaml` where they recorded one. An
    /// adapter with no head to name places as `place` does.
    fn place_on_head(
        &self,
        packed: &Packed,
        from: &Placement,
        to: &Placement,
        head: Option<&GitHead>,
    ) -> Result<Conversation, ConversationError> {
        let _ = head;
        self.place(packed, from, to)
    }

    /// The git commit and branch the CLI's own index names for conversation
    /// `id` at `at`: `None` when the CLI keeps no such index or row. Only
    /// Codex does; any other adapter has none.
    fn thread_git(&self, at: &Placement, id: &str) -> Result<Option<ThreadGit>, ConversationError> {
        let _ = (at, id);
        Ok(None)
    }

    /// Puts `git` back as the CLI's index names it for conversation `id` at
    /// `at`, as [`Adapter::thread_git`] read it. A row or index that isn't
    /// there stays missing.
    fn restore_thread_git(&self, at: &Placement, id: &str, git: &ThreadGit) -> Result<(), ConversationError> {
        let _ = (at, id, git);
        Ok(())
    }
}

/// The git fields of a conversation's row in the CLI's index (Codex's
/// `threads.git_sha` and `git_branch`), each `None` when the row holds NULL.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadGit {
    pub sha: Option<String>,
    pub branch: Option<String>,
}

/// The git branch and commit a carried session's clone is on at its new
/// place (MV-16): a carry onto a new clone passes it to
/// [`Adapter::place_on_head`] so the conversation names that clone's head.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHead {
    /// None for a detached head.
    pub branch: Option<String>,
    /// The full hex commit id.
    pub sha: String,
}

static CLAUDE: claude::ClaudeJsonl = claude::ClaudeJsonl::DEFAULT;
static CODEX: codex::CodexRollout = codex::CodexRollout::DEFAULT;
static COPILOT: copilot::CopilotSessionState = copilot::CopilotSessionState;

/// The adapter for `format`.
pub fn adapter(format: Format) -> &'static dyn Adapter {
    match format {
        Format::ClaudeJsonl => &CLAUDE,
        Format::CodexRollout => &CODEX,
        Format::CopilotSessionState => &COPILOT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_are_named_as_agent_kinds_name_them() {
        for (format, name) in [
            (Format::ClaudeJsonl, "\"claude-jsonl\""),
            (Format::CodexRollout, "\"codex-rollout\""),
            (Format::CopilotSessionState, "\"copilot-session-state\""),
        ] {
            assert_eq!(serde_json::to_string(&format).unwrap(), name);
            assert_eq!(serde_json::from_str::<Format>(name).unwrap(), format);
        }
    }
}
