//! Codex's rollout conversations ([`crate::Format::CodexRollout`]).
//!
//! Each fact below is marked **X1** (seen in the real-CLI layout fixture,
//! `codex_layout_test`, codex-cli 0.169.0: `codex exec --json`, then
//! `codex exec resume --json <id>`) or **X2** (not yet proven; the fork and
//! carry fixtures decide).
//!
//! # Where the files are
//!
//! ```text
//! <codex>/sessions/<YYYY>/<MM>/<DD>/rollout-<YYYY>-<MM>-<DD>T<hh>-<mm>-<ss>-<id>.jsonl   the rollout
//! <codex>/state_5.sqlite (-wal, -shm)          the CLI's thread index
//! <codex>/thread_history_1.sqlite (-wal, -shm) the CLI's projection of each rollout
//! <codex>/goals_1, logs_2, memories_1, queue_1.sqlite   per-thread tables, empty in X1
//! ```
//!
//! - `<codex>` is [`Placement::cli_home`] (the CLI's `CODEX_HOME`) when
//!   set, else `<home>/.codex`. **X1**
//! - The date folders and the time in the name are the *local* time of the
//!   rollout's `session_meta` `payload.timestamp` (X1 wrote `21:48:56.953Z`
//!   as `2026/11/16/rollout-2026-11-16T13-48-56-…` under
//!   `America/Los_Angeles`). **X1**
//! - `codex exec resume <id>` appends to the same rollout; it writes no new
//!   file and no new `session_meta`. **X1**
//! - The id is a lowercase hyphenated UUID (a v7 in X1). [`Adapter::locate`]
//!   and [`Adapter::fork`] take only that form (8-4-4-4-12 lowercase hex),
//!   else [`ConversationError::BadId`]: the CLI resumes by exact full id.
//! - [`Adapter::locate`] lists the one plain file named
//!   `rollout-*-<id>.jsonl` three folders under `<codex>/sessions/`, as
//!   [`crate::FileRole::Transcript`], and nothing else: X1 shows no other
//!   file that is the conversation's own (`shell_snapshots/` stayed empty;
//!   the SQLite files are the CLI's). None is
//!   [`ConversationError::NotFound`]; more than one is
//!   [`ConversationError::Unsupported`]; a link is
//!   [`ConversationError::NotAFile`]. Links are never followed.
//! - `archived_sessions/`: not seen. **X2**
//!
//! # The CLI's SQLite state (CV-4)
//!
//! - `state_5.sqlite` table `threads` has one row per conversation that
//!   names it: `id`, the absolute `rollout_path`, `cwd`, `title`,
//!   `first_user_message`, `git_sha`, `git_branch`, `git_origin_url`,
//!   `cli_version`, times. `thread_read_receipts` has a row by `thread_id`.
//!   Its WAL held the folder 36 times and the id 98 times. **X1**
//! - `thread_history_1.sqlite` is a byte-offset projection of the rollout:
//!   `thread_turns` (one row per turn, `rollout_byte_offset`,
//!   `rollout_end_byte_offset`, `rollout_end_ordinal`), `thread_items` (each
//!   user and agent message's `item_json`) and
//!   `thread_history_projection_state` (`next_rollout_byte_offset`,
//!   `next_rollout_ordinal`). **X1**
//! - `state_5.sqlite` also has `backfill_state`, `rollout_migration_state`
//!   and `rollout_migration_skipped_rollouts`, which suggests the CLI scans
//!   rollouts into its index. **X1** (the tables); what they do is **X2**.
//!
//! This adapter never copies any of them. A fork writes the rollout only,
//! and so does a place onto another Codex dir. A place within one Codex dir
//! (a move on one machine, D336) rewrites the rollout where it is and
//! updates `state_5.sqlite`'s `threads` row of that id (`cwd`,
//! `rollout_path`, and `git_branch` and `git_sha` when the place names a
//! head), no other row; a missing `state_5.sqlite` is left missing.
//! Before that rewrite it deletes the thread's rows from
//! `thread_history_1.sqlite` (`thread_items`, `thread_turns` and
//! `thread_history_projection_state`; the CLI's own trigger drops its
//! `thread_realtime_items`), no other thread's: their byte offsets point
//! into the old rollout, the CLI never repairs them, and with the rows gone
//! it rebuilds them from the rollout on resume (**X2**, 0.169.0). A missing
//! history file is left missing. Whether `codex exec resume <id>` takes a
//! rollout that has no `threads` row (every fork, every placed carry) is
//! **X2**; until the fixture passes, the session's resume check (CV-5) is
//! what catches a refusal.
//!
//! # The rollout
//!
//! One JSON object per line, each ending in `\n`; a last line without its
//! `\n` is being written, and every method ignores it. Every line has the
//! top-level fields `timestamp` (RFC 3339, UTC), `ordinal` (0 on the first
//! line, one more on each), `type` and `payload` (an object); a
//! `response_item` line also has `metadata`. **X1** The kinds X1 wrote:
//!
//! | `type` | `payload.type` | what it is |
//! |---|---|---|
//! | `session_meta` | — | first line, once per file: `id` and `session_id` (both the id), `timestamp`, `cwd`, `runtime_workspace_roots[]`, `cli_version`, `git`, `base_instructions` |
//! | `event_msg` | `task_started` | a turn begins: `turn_id` |
//! | `response_item` | `message` | role `developer` (harness instructions), `user` or `assistant`; `internal_chat_message_metadata_passthrough.turn_id` and `.content_item_kinds` |
//! | `world_state` | — | `state.agents_md.directory`, `state.environments.environments.local.cwd`, `state.environments.filesystem` (XML naming the folder) |
//! | `turn_context` | — | per turn: `turn_id`, `cwd`, model and sandbox settings |
//! | `event_msg` | `thread_settings_applied` | written when a process starts or resumes: `thread_id`, `thread_settings.cwd`, `.runtime_workspace_roots[]` |
//! | `event_msg` | `item_completed` | `thread_id`, `turn_id`, `item.type` `UserMessage` or `AgentMessage` |
//! | `token_usage_record` | — | `thread_id`, `session_id`, `turn_id` |
//! | `event_msg` | `token_count` | usage and rate limits |
//! | `event_msg` | `task_complete` | the turn ended: `turn_id`, `last_agent_message` |
//!
//! - **The id** is in the file 12 times in X1, always as a whole string:
//!   `session_meta`'s `payload.id` and `payload.session_id`, the
//!   `payload.thread_id` of `thread_settings_applied`, `item_completed` and
//!   `token_usage_record`, and `token_usage_record`'s `payload.session_id`.
//!   It is never part of a longer string. **X1**
//! - **The folder** is in `payload.cwd` (`session_meta`, `turn_context`),
//!   `payload.runtime_workspace_roots[]`, `payload.thread_settings.cwd` and
//!   `.runtime_workspace_roots[]`, `world_state`'s three fields above, and
//!   inside free text: the injected `# AGENTS.md instructions for <folder>`
//!   and `<cwd><folder></cwd>`, and any prompt or reply. Never dashed.
//!   **X1**
//! - **The Codex dir** is inside free text: the skill roots
//!   (`<codex>/skills/.system`) in the developer message and in
//!   `world_state`'s `state.host_skills.body`. **X1**
//! - Tool calls, reasoning, compaction, rollback (`/undo`, an edited
//!   prompt) and an interrupted turn: none seen. **X2**
//!
//! # The strict parse
//!
//! Every complete line goes through it, in file order; the first line it
//! refuses decides the error:
//!
//! 1. The line must be a JSON object with a string `type`, else
//!    [`ConversationError::Malformed`] with its 1-based line number.
//! 2. `type` must be one of [`KINDS`] and `payload` an object; a
//!    `response_item`'s `payload.type` one of [`ITEMS`]; an `event_msg`'s
//!    `payload.type` one of [`EVENTS`]; an `item_completed`'s
//!    `payload.item.type` one of [`COMPLETED`].
//! 3. A top-level field not in [`FIELDS`], and a `payload` field not in
//!    that kind's [`PAYLOAD_FIELDS`] entry, must hold no path: no string in
//!    its value, at any depth, starts with `/` or `~` or holds the folder of
//!    the conversation's [`Placement`].
//!
//! Steps 2 and 3 refuse with [`ConversationError::Unsupported`]. Its
//! `version` is the CLI version given to [`Adapter::check`], else the
//! `payload.cli_version` of the last `session_meta` before the refused line,
//! else `None`. The given version comes first because a resume writes no new
//! `session_meta` (X1): the file names only the CLI that started it, and
//! the launch record names the one that ran last. The values of known fields
//! are free-form; the carry rewrites them as text.
//!
//! Every line is on the current path: X1 shows no branch. A rollback,
//! compaction or interruption record is an unknown kind or event today, so
//! step 2 refuses it until the X2 fixtures show how it marks the path.
//!
//! # Turns
//!
//! A **turn** is each distinct `payload.turn_id` of an `event_msg`
//! `item_completed` line whose `item.type` is `UserMessage`, in file order.
//! Its first such line is its prompt: the first `item.content[]` block with
//! `"type":"text"` and non-empty `text`. A second `UserMessage` with the same
//! `turn_id` (a prompt added while the turn runs; **X2**) is not a turn of
//! its own. Injected context (a `response_item` user message whose
//! `content_item_kinds` are `agents_md.*` or `environments.*`) and developer
//! messages have no `item_completed`, so they aren't turns. **X1**
//!
//! - [`Turn::first_line`] is the prompt's first line; [`Turn::at`] is the
//!   prompt line's top-level `timestamp`.
//! - A turn's **start** is the first line whose `payload.turn_id` is its
//!   id: its `task_started` in X1.
//! - A turn is **finished** when an `event_msg` `task_complete` with its
//!   `turn_id` is in the file.
//!
//! # Fork
//!
//! A session forks from now with the CLI's own `codex exec fork <id>`
//! (CV-1). This fork is for fork at a turn, and for a fork into another
//! folder or home.
//!
//! The fork holds the lines before turn `before`'s start. For `None` (from
//! now) it holds every line when the last turn is finished, and those before
//! the last turn's start when it is still running. It keeps a prefix, so its
//! `ordinal`s still count from 0. `before` must be 2 up to the number of
//! turns: 1 is [`ConversationError::NothingBefore`], 0 and past the last turn
//! [`ConversationError::NoSuchTurn`]; from now with no finished turn before
//! the running one (or no turn) is [`ConversationError::NothingBefore`].
//! Each line is written with:
//! - the carry's path rewrite (below) from the parent's [`Placement`] to
//!   `to`, as values (no pairs when they are the same);
//! - every string that is exactly the parent's id, at any depth, set to the
//!   new id.
//!
//! The fork's file is the parent's path under its Codex dir with
//! `-<id>.jsonl` replaced by `-<new id>.jsonl`, under `to`'s Codex dir
//! (the parent's date folders and time, creating the folders). Whether the
//! CLI minds a name whose time isn't the fork's own, and what
//! `codex exec fork` itself writes into `session_meta` (a parent field?), is
//! **X2**. A file already there is [`ConversationError::Exists`]. Every line
//! of the parent is parsed before anything is written. The parent and the
//! SQLite files are never touched. A fork has no size cap.
//!
//! # Carry
//!
//! [`Adapter::pack`] copies the rollout's complete lines, unchanged, to
//! `<stage>/<path under the Codex dir>` (`sessions/<YYYY>/<MM>/<DD>/…`), a
//! running turn included (CV-5 leaves it to the CLI on resume).
//! [`Adapter::place`] writes it to the same path under `to`'s Codex dir,
//! with its text replaced, in this order:
//! 1. `from`'s Codex dir → `to`'s;
//! 2. `from.folder` → `to.folder`, as it is;
//! 3. the same, JSON-escaped, when that differs from 2.
//!
//! [`Adapter::place_on_head`] also sets the first line's `payload.git.branch`
//! (when the head has one) and `commit_hash`, keeping its remote. No pairs when the old and new texts are equal. The id is kept. Pack runs
//! the strict parse on the rollout (at `from`); after the rewrite every line
//! must still pass it (at `to`). Pack and place check before they write
//! anything, as the Claude adapter does: a
//! plain file, not a link ([`ConversationError::NotAFile`]), at most
//! [`REWRITE_MAX`] bytes ([`ConversationError::TooBig`]), and text
//! ([`ConversationError::NotText`]); a staged name that is absolute or has a
//! `..` part is [`ConversationError::BadPath`], a staged set without its
//! rollout (`sessions/…/rollout-…-<id>.jsonl`, as the transcript) is
//! [`ConversationError::NotFound`], and a target that is there is
//! [`ConversationError::Exists`], except within one Codex dir, where the
//! rollout is rewritten in place. Place never creates a SQLite file.
//!
//! # Versions
//!
//! [`Adapter::check`] runs the strict parse and says nothing more: the CLI
//! version doesn't decide (CV-7). A newer version whose lines all parse is
//! known; one that adds a kind, an event or a path field is not.
//!
//! # Not yet proven against the real CLI (X2 fixtures)
//!
//! - That `codex exec resume <fork id>` takes a rollout with no `threads`
//!   row and no projection, and a placed carry whose `rollout_path` the db
//!   doesn't hold.
//! - That a fork's name may keep the parent's time, and what
//!   `codex exec fork` writes (a parent link in `session_meta`?).
//! - The records of tool calls, reasoning, compaction, rollback and an
//!   interrupted turn, and how they mark the current path.
//! - A prompt added to a running turn (one `turn_id`, two `UserMessage`s).
//! - Whether `shell_snapshots/` or `archived_sessions/` hold any of a
//!   conversation's own files.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::Value;

use crate::files::{
    complete_lines, holds_files, holds_path, io_error, is_there, json_escape, line_starts,
    malformed, path_text, read_plain, read_text, relative_name, rewrite_text, rewrite_value, under,
    valid_rel, write_new,
};
use crate::{
    Adapter, Conversation, ConversationError, ConversationFile, FileRole, Format, GitHead,
    Packed, PackedFile, Placement, ThreadGit, Turn,
};

/// The biggest file a carry rewrites. A bigger one is refused, never copied
/// as it is (CV-3).
const REWRITE_MAX: u64 = 256 << 20;

/// The folder under the Codex dir that holds the rollouts by date.
const SESSIONS: &str = "sessions";

/// The CLI's thread index under the Codex dir.
const STATE: &str = "state_5.sqlite";

/// The CLI's turn history under the Codex dir: what it projected from each
/// rollout, with byte offsets into it (X2, 0.169.0).
const HISTORY: &str = "thread_history_1.sqlite";

/// The tables of [`HISTORY`] that hold a thread's projection.
const HISTORY_TABLES: [&str; 3] =
    ["thread_items", "thread_turns", "thread_history_projection_state"];

/// The line kinds (`type`) the strict parse knows (CV-7). Add one only once
/// the real-CLI fixtures show what it holds and how a fork and a carry
/// treat it.
const KINDS: &[&str] = &[
    "session_meta",
    "turn_context",
    "world_state",
    "response_item",
    "event_msg",
    "token_usage_record",
];

/// The `response_item` kinds (`payload.type`) the strict parse knows.
const ITEMS: &[&str] = &["message"];

/// The `event_msg` kinds (`payload.type`) the strict parse knows.
const EVENTS: &[&str] = &[
    "task_started",
    "task_complete",
    "thread_settings_applied",
    "token_count",
    "item_completed",
];

/// The `item_completed` items (`payload.item.type`) the strict parse knows.
const COMPLETED: &[&str] = &["UserMessage", "AgentMessage"];

/// The top-level fields the strict parse knows. An unknown one is accepted
/// only while it holds no path.
const FIELDS: &[&str] = &["timestamp", "ordinal", "type", "payload", "metadata"];

/// The `payload` fields the strict parse knows, by kind: `type` for the
/// payload-less kinds, `type/payload.type` for `response_item` and
/// `event_msg`. An unknown one is accepted only while it holds no path. The
/// values of known ones are rewritten as text by a carry.
const PAYLOAD_FIELDS: &[(&str, &[&str])] = &[
    (
        "session_meta",
        &[
            "account_scoped",
            "session_id",
            "id",
            "timestamp",
            "cwd",
            "runtime_workspace_roots",
            "originator",
            "cli_version",
            "source",
            "thread_source",
            "model_provider",
            "base_instructions",
            "history_mode",
            "context_window",
            "git",
        ],
    ),
    (
        "turn_context",
        &[
            "turn_id",
            "root_turn_id",
            "disabled_plugin_ids",
            "cwd",
            "approval_policy",
            "approvals_reviewer",
            "permission_profile",
            "active_permission_profile",
            "model",
            "collaboration_mode",
            "multi_agent_version",
            "realtime_active",
            "summary",
            "sandbox_policy",
        ],
    ),
    ("world_state", &["full", "state"]),
    (
        "token_usage_record",
        &[
            "thread_id",
            "turn_id",
            "session_id",
            "root_turn_id",
            "response_id",
            "usage",
            "turn_token_usage",
            "thread_token_usage",
        ],
    ),
    (
        "response_item/message",
        &[
            "type",
            "id",
            "role",
            "status",
            "content",
            "phase",
            "internal_chat_message_metadata_passthrough",
        ],
    ),
    (
        "event_msg/task_started",
        &[
            "type",
            "turn_attribution",
            "turn_id",
            "root_turn_id",
            "started_at",
            "model_context_window",
            "collaboration_mode_kind",
        ],
    ),
    (
        "event_msg/task_complete",
        &[
            "type",
            "root_turn_id",
            "turn_id",
            "last_agent_message",
            "started_at",
            "completed_at",
            "duration_ms",
            "time_to_first_token_ms",
        ],
    ),
    (
        "event_msg/thread_settings_applied",
        &["type", "thread_id", "thread_settings"],
    ),
    ("event_msg/token_count", &["type", "info", "rate_limits"]),
    (
        "event_msg/item_completed",
        &[
            "type",
            "thread_id",
            "turn_id",
            "item",
            "started_at_ms",
            "completed_at_ms",
        ],
    ),
];

/// The adapter for [`crate::Format::CodexRollout`].
pub(crate) struct CodexRollout {
    /// The biggest file [`Adapter::pack`] and [`Adapter::place`] take.
    rewrite_max: u64,
}

impl CodexRollout {
    /// The adapter with the real cap.
    pub(crate) const DEFAULT: Self = Self {
        rewrite_max: REWRITE_MAX,
    };
}

impl Adapter for CodexRollout {
    /// The one rollout `sessions/*/*/*/rollout-*-<id>.jsonl` under the
    /// Codex dir of `at`. The id must be a lowercase hyphenated UUID.
    fn locate(&self, at: &Placement, id: &str) -> Result<Conversation, ConversationError> {
        check_id(id)?;
        let sessions = codex_dir(at).join(SESSIONS);
        let mut found = Vec::new();
        for year in folders_in(&sessions)? {
            for month in folders_in(&year)? {
                for day in folders_in(&month)? {
                    found.extend(
                        children(&day)?
                            .into_iter()
                            .filter(|path| is_rollout_of(path, id)),
                    );
                }
            }
        }
        if found.len() > 1 {
            return Err(ConversationError::Unsupported {
                format: Format::CodexRollout,
                version: None,
            });
        }
        let Some(path) = found.pop() else {
            return Err(ConversationError::NotFound { path: sessions });
        };
        let meta = fs::symlink_metadata(&path)
            .map_err(|source| io_error("read the transcript", &path, source))?;
        if !meta.file_type().is_file() {
            return Err(ConversationError::NotAFile { path });
        }
        Ok(Conversation {
            id: id.to_owned(),
            format: Format::CodexRollout,
            placement: at.clone(),
            files: vec![ConversationFile {
                path,
                role: FileRole::Transcript,
            }],
        })
    }

    /// The prompts of the rollout's turns, in order (module doc, Turns).
    fn turns(&self, conversation: &Conversation) -> Result<Vec<Turn>, ConversationError> {
        let path = transcript_of(conversation)?;
        let bytes = read_plain(&path, u64::MAX)?;
        let lines = parse(&bytes, &path, &conversation.placement, None)?;
        Ok(prompts_of(&lines)
            .into_iter()
            .map(|prompt| prompt.turn)
            .collect())
    }

    /// Where each turn starts: the first line that names its id, the line a
    /// fork before it is cut at (module doc, Fork).
    fn turn_starts(&self, conversation: &Conversation) -> Result<Vec<u64>, ConversationError> {
        let path = transcript_of(conversation)?;
        let bytes = read_plain(&path, u64::MAX)?;
        let lines = parse(&bytes, &path, &conversation.placement, None)?;
        let numbers = prompts_of(&lines).into_iter().map(|prompt| prompt.start);
        Ok(line_starts(&bytes, numbers))
    }

    /// The rollout's lines before turn `before`'s start (or all, from now),
    /// with the id and the paths rewritten, as a new rollout (module doc,
    /// Fork).
    fn fork(
        &self,
        conversation: &Conversation,
        before: Option<u32>,
        new_id: &str,
        to: &Placement,
    ) -> Result<Conversation, ConversationError> {
        check_id(new_id)?;
        let path = transcript_of(conversation)?;
        let bytes = read_plain(&path, u64::MAX)?;
        let lines = parse(&bytes, &path, &conversation.placement, None)?;
        let cut = cut_for(&prompts_of(&lines), before)?;
        let pairs = rewrite_pairs(&conversation.placement, to, false);
        let mut text = String::new();
        for (number, mut record) in lines {
            if number >= cut {
                break;
            }
            rewrite_value(&mut record, &pairs);
            rename_ids(&mut record, &conversation.id, new_id);
            text.push_str(&record.to_string());
            text.push('\n');
        }
        let parent_rel = relative_name(&path, &codex_dir(&conversation.placement))?;
        let parent_end = format!("-{}.jsonl", conversation.id);
        let stem = parent_rel
            .strip_suffix(parent_end.as_str())
            .ok_or_else(|| ConversationError::NotFound { path: path.clone() })?;
        let target = under(&codex_dir(to), &format!("{stem}-{new_id}.jsonl"));
        if is_there(&target)? {
            return Err(ConversationError::Exists { path: target });
        }
        write_new(&target, text.as_bytes())?;
        Ok(Conversation {
            id: new_id.to_owned(),
            format: Format::CodexRollout,
            placement: to.clone(),
            files: vec![ConversationFile {
                path: target,
                role: FileRole::Transcript,
            }],
        })
    }

    /// The rollout's complete lines, unchanged, under `stage` at its path
    /// under the Codex dir (module doc, Carry).
    fn pack(&self, conversation: &Conversation, stage: &Path) -> Result<Packed, ConversationError> {
        let path = transcript_of(conversation)?;
        let text = read_text(&path, self.rewrite_max)?;
        parse(text.as_bytes(), &path, &conversation.placement, None)?;
        if holds_files(stage)? {
            return Err(ConversationError::Exists {
                path: stage.to_path_buf(),
            });
        }
        let rel = relative_name(&path, &codex_dir(&conversation.placement))?;
        let mut body = Vec::with_capacity(text.len());
        for line in complete_lines(text.as_bytes()) {
            body.extend_from_slice(line);
            body.push(b'\n');
        }
        write_new(&under(stage, &rel), &body)?;
        let size = u64::try_from(body.len()).unwrap_or(u64::MAX);
        Ok(Packed {
            id: conversation.id.clone(),
            format: Format::CodexRollout,
            stage: stage.to_path_buf(),
            files: vec![PackedFile {
                rel,
                role: FileRole::Transcript,
                size,
            }],
        })
    }

    /// The staged rollout at the same path under `to`'s Codex dir, its
    /// Codex dir and folder rewritten (module doc, Carry). Writes a SQLite
    /// file only to follow a rollout rewritten in place.
    fn place(
        &self,
        packed: &Packed,
        from: &Placement,
        to: &Placement,
    ) -> Result<Conversation, ConversationError> {
        self.place_on_head(packed, from, to, None)
    }

    /// As [`Adapter::place`], and with `head` the session line's git branch
    /// and commit name it (module doc, Carry). On the same machine (one
    /// Codex dir) the rollout is rewritten where it is and the thread's row
    /// in `state_5.sqlite` follows it.
    fn place_on_head(
        &self,
        packed: &Packed,
        from: &Placement,
        to: &Placement,
        head: Option<&GitHead>,
    ) -> Result<Conversation, ConversationError> {
        ensure_codex(packed.format)?;
        check_id(&packed.id)?;
        if let Some(file) = packed.files.iter().find(|file| !valid_rel(&file.rel)) {
            return Err(ConversationError::BadPath {
                rel: file.rel.clone(),
            });
        }
        let suffix = format!("-{}.jsonl", packed.id);
        let has_rollout = packed.files.iter().any(|file| {
            file.role == FileRole::Transcript
                && file.rel.split('/').next() == Some(SESSIONS)
                && file.rel.ends_with(suffix.as_str())
        });
        if !has_rollout {
            return Err(ConversationError::NotFound {
                path: packed.stage.join(SESSIONS),
            });
        }
        let pairs = rewrite_pairs(from, to, true);
        let to_codex = codex_dir(to);
        let in_place = codex_dir(from) == to_codex;
        let mut writes = Vec::with_capacity(packed.files.len());
        for file in &packed.files {
            let text = read_text(&under(&packed.stage, &file.rel), self.rewrite_max)?;
            let mut rewritten = rewrite_text(&text, &pairs);
            let target = under(&to_codex, &file.rel);
            if is_there(&target)? && !in_place {
                return Err(ConversationError::Exists { path: target });
            }
            if file.role == FileRole::Transcript {
                if let Some(head) = head {
                    rewritten = with_head(&rewritten, head);
                }
                parse(rewritten.as_bytes(), &target, to, None)?;
            }
            writes.push((target, file.role, rewritten));
        }
        if in_place {
            // First, so a refusal leaves the rollout as it was; a rollout
            // write that fails after it leaves a history the CLI rebuilds.
            clear_history(&to_codex, &packed.id)?;
        }
        for (target, _, text) in &writes {
            write_new(target, text.as_bytes())?;
        }
        if in_place {
            let rollout = writes.iter().find(|(_, role, _)| *role == FileRole::Transcript);
            if let Some((rollout, _, _)) = rollout {
                follow_thread(&to_codex, &packed.id, rollout, &to.folder, head)?;
            }
        }
        Ok(Conversation {
            id: packed.id.clone(),
            format: Format::CodexRollout,
            placement: to.clone(),
            files: writes
                .into_iter()
                .map(|(path, role, _)| ConversationFile { path, role })
                .collect(),
        })
    }

    /// The `git_sha` and `git_branch` of the thread `id` in `state_5.sqlite`
    /// under `at`'s Codex dir; `None` without the index or the row.
    fn thread_git(&self, at: &Placement, id: &str) -> Result<Option<ThreadGit>, ConversationError> {
        check_id(id)?;
        let Some((connection, db)) = open_state(&codex_dir(at))? else {
            return Ok(None);
        };
        connection
            .query_row("SELECT git_sha, git_branch FROM threads WHERE id = ?1", [id], |row| {
                Ok(ThreadGit { sha: row.get(0)?, branch: row.get(1)? })
            })
            .optional()
            .map_err(|source| ConversationError::State { path: db, source })
    }

    /// Sets the thread `id`'s `git_sha` and `git_branch` as `git` says, NULLs
    /// too; no other column or row changes, and a missing index or row stays
    /// missing.
    fn restore_thread_git(&self, at: &Placement, id: &str, git: &ThreadGit) -> Result<(), ConversationError> {
        check_id(id)?;
        let Some((connection, db)) = open_state(&codex_dir(at))? else {
            return Ok(());
        };
        connection
            .execute(
                "UPDATE threads SET git_sha = ?2, git_branch = ?3 WHERE id = ?1",
                rusqlite::params![id, git.sha, git.branch],
            )
            .map(drop)
            .map_err(|source| ConversationError::State { path: db, source })
    }

    /// The strict parse of every complete line (module doc, The strict
    /// parse); the CLI version only names the refusal.
    fn check(
        &self,
        conversation: &Conversation,
        cli_version: Option<&str>,
    ) -> Result<(), ConversationError> {
        let path = transcript_of(conversation)?;
        let bytes = read_plain(&path, u64::MAX)?;
        parse(&bytes, &path, &conversation.placement, cli_version).map(drop)
    }
}

/// A conversation id is a lowercase hyphenated UUID: 8-4-4-4-12 hex digits.
fn check_id(id: &str) -> Result<(), ConversationError> {
    let bytes = id.as_bytes();
    let shaped = bytes.len() == 36
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => *byte == b'-',
            _ => byte.is_ascii_digit() || (b'a'..=b'f').contains(byte),
        });
    if shaped {
        Ok(())
    } else {
        Err(ConversationError::BadId { id: id.to_owned() })
    }
}

/// The Codex dir of `at`: its `CODEX_HOME`, else `<home>/.codex` (X1).
fn codex_dir(at: &Placement) -> PathBuf {
    at.cli_home
        .clone()
        .unwrap_or_else(|| at.home.join(".codex"))
}

fn ensure_codex(format: Format) -> Result<(), ConversationError> {
    match format {
        Format::CodexRollout => Ok(()),
        other => Err(ConversationError::Unsupported {
            format: other,
            version: None,
        }),
    }
}

/// The transcript file of `conversation`, which a method of this adapter
/// reads.
fn transcript_of(conversation: &Conversation) -> Result<PathBuf, ConversationError> {
    ensure_codex(conversation.format)?;
    conversation
        .files
        .iter()
        .find(|file| file.role == FileRole::Transcript)
        .map(|file| file.path.clone())
        .ok_or_else(|| ConversationError::NotFound {
            path: codex_dir(&conversation.placement).join(SESSIONS),
        })
}

/// The entries of `dir`; a folder that isn't there has none.
fn children(dir: &Path) -> Result<Vec<PathBuf>, ConversationError> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(io_error("list the folder", dir, source)),
    };
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| io_error("list the folder", dir, source))?;
        paths.push(entry.path());
    }
    Ok(paths)
}

/// The folders under `dir`. A link is not followed, so it is not one.
fn folders_in(dir: &Path) -> Result<Vec<PathBuf>, ConversationError> {
    let mut folders = Vec::new();
    for path in children(dir)? {
        let meta = fs::symlink_metadata(&path)
            .map_err(|source| io_error("list the folder", &path, source))?;
        if meta.file_type().is_dir() {
            folders.push(path);
        }
    }
    Ok(folders)
}

/// Whether `path` is a rollout of conversation `id`: `rollout-*-<id>.jsonl`.
fn is_rollout_of(path: &Path, id: &str) -> bool {
    let suffix = format!("-{id}.jsonl");
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("rollout-") && name.ends_with(suffix.as_str()))
}

/// The complete lines of `bytes`, each with its 1-based number, through the
/// strict parse (module doc, The strict parse): the first line it refuses
/// decides the error. `cli_version` names the refusal first; `None` in every
/// method but check, which takes the version the rollout's last
/// `session_meta` names.
fn parse(
    bytes: &[u8],
    path: &Path,
    at: &Placement,
    cli_version: Option<&str>,
) -> Result<Vec<(u64, Value)>, ConversationError> {
    let folder = path_text(&at.folder);
    let mut lines = Vec::new();
    let mut last_version: Option<String> = None;
    for (number, raw) in (1_u64..).zip(complete_lines(bytes)) {
        let value: Value =
            serde_json::from_slice(raw).map_err(|source| ConversationError::Malformed {
                path: path.to_path_buf(),
                line: number,
                source: Some(source),
            })?;
        if !value.is_object() || str_at(&value, "/type").is_none() {
            return Err(malformed(path, number));
        }
        if !is_known(&value, &folder) {
            return Err(unsupported(cli_version, last_version.as_deref()));
        }
        if let Some(version) = session_version(&value) {
            last_version = Some(version.to_owned());
        }
        lines.push((number, value));
    }
    Ok(lines)
}

/// The refusal for a record the strict parse doesn't know.
fn unsupported(cli_version: Option<&str>, last_version: Option<&str>) -> ConversationError {
    ConversationError::Unsupported {
        format: Format::CodexRollout,
        version: cli_version.or(last_version).map(str::to_owned),
    }
}

/// Whether the strict parse knows a record: its kind, its payload's kind
/// and item, and that no field it doesn't know holds a path (steps 2 and 3).
fn is_known(value: &Value, folder: &str) -> bool {
    let Some(kind) = str_at(value, "/type") else {
        return false;
    };
    let Some(payload) = value.get("payload").and_then(Value::as_object) else {
        return false;
    };
    let Some(key) = payload_key(kind, payload.get("type").and_then(Value::as_str)) else {
        return false;
    };
    if key == "event_msg/item_completed"
        && !str_at(value, "/payload/item/type").is_some_and(|item| COMPLETED.contains(&item))
    {
        return false;
    }
    let known = PAYLOAD_FIELDS
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(_, fields)| *fields)
        .unwrap_or(&[]);
    let payload_known = payload
        .iter()
        .all(|(field, held)| known.contains(&field.as_str()) || !holds_path(held, folder));
    let top_known = value.as_object().is_some_and(|record| {
        record
            .iter()
            .all(|(field, held)| FIELDS.contains(&field.as_str()) || !holds_path(held, folder))
    });
    payload_known && top_known
}

/// The `PAYLOAD_FIELDS` key of a record: its kind, or `response_item/<item>`
/// and `event_msg/<event>` for the two kinds that name one. `None` when the
/// kind or that name is unknown.
fn payload_key(kind: &str, payload_type: Option<&str>) -> Option<String> {
    if !KINDS.contains(&kind) {
        return None;
    }
    match (kind, payload_type) {
        ("response_item", Some(item)) if ITEMS.contains(&item) => {
            Some(format!("response_item/{item}"))
        }
        ("event_msg", Some(event)) if EVENTS.contains(&event) => Some(format!("event_msg/{event}")),
        ("response_item" | "event_msg", _) => None,
        _ => Some(kind.to_owned()),
    }
}

/// The CLI version a `session_meta` names, if this record is one.
fn session_version(value: &Value) -> Option<&str> {
    if str_at(value, "/type") == Some("session_meta") {
        str_at(value, "/payload/cli_version")
    } else {
        None
    }
}

/// The string at JSON `pointer` of `value`, if there is one.
fn str_at<'a>(value: &'a Value, pointer: &str) -> Option<&'a str> {
    value.pointer(pointer).and_then(Value::as_str)
}

/// Whether `value` is an `event_msg` of payload type `kind`.
fn event_is(value: &Value, kind: &str) -> bool {
    str_at(value, "/type") == Some("event_msg") && str_at(value, "/payload/type") == Some(kind)
}

/// The turn id a record names in its payload.
fn turn_of(value: &Value) -> Option<&str> {
    str_at(value, "/payload/turn_id")
}

/// The first non-empty text block of a `UserMessage`'s content.
fn prompt_text(value: &Value) -> Option<&str> {
    value
        .pointer("/payload/item/content")?
        .as_array()?
        .iter()
        .filter(|block| str_at(block, "/type") == Some("text"))
        .find_map(|block| str_at(block, "/text").filter(|text| !text.is_empty()))
}

/// A turn of the rollout, with what a fork needs beside its listing.
struct Prompt {
    /// The id its lines name it by.
    turn_id: String,
    turn: Turn,
    /// The number of the first line that names the id: its start.
    start: u64,
    /// Whether a `task_complete` with the id is in the file.
    finished: bool,
}

/// The turns of the rollout, in order: the first `UserMessage` of each
/// distinct turn id (module doc, Turns).
fn prompts_of(lines: &[(u64, Value)]) -> Vec<Prompt> {
    let mut prompts: Vec<Prompt> = Vec::new();
    for (_, value) in lines {
        if !event_is(value, "item_completed")
            || str_at(value, "/payload/item/type") != Some("UserMessage")
        {
            continue;
        }
        let Some(turn_id) = turn_of(value) else {
            continue;
        };
        if prompts.iter().any(|prompt| prompt.turn_id == turn_id) {
            continue;
        }
        let first_line = prompt_text(value)
            .and_then(|text| text.lines().next())
            .unwrap_or_default()
            .to_owned();
        let start = lines
            .iter()
            .find(|(_, line)| turn_of(line) == Some(turn_id))
            .map_or(0, |(number, _)| *number);
        let finished = lines
            .iter()
            .any(|(_, line)| event_is(line, "task_complete") && turn_of(line) == Some(turn_id));
        prompts.push(Prompt {
            turn_id: turn_id.to_owned(),
            turn: Turn {
                index: u32::try_from(prompts.len() + 1).unwrap_or(u32::MAX),
                first_line,
                at: str_at(value, "/timestamp").map(str::to_owned),
            },
            start,
            finished,
        });
    }
    prompts
}

/// The line a fork keeps lines before (module doc, Fork): turn `before`'s
/// start, the end of the rollout when the last turn is finished (`None`
/// from now), else the running last turn's start.
fn cut_for(prompts: &[Prompt], before: Option<u32>) -> Result<u64, ConversationError> {
    let turns = u32::try_from(prompts.len()).unwrap_or(u32::MAX);
    match before {
        Some(1) => Err(ConversationError::NothingBefore),
        Some(turn) => usize::try_from(turn)
            .ok()
            .and_then(|position| position.checked_sub(1))
            .and_then(|index| prompts.get(index))
            .map(|prompt| prompt.start)
            .ok_or(ConversationError::NoSuchTurn { turn, turns }),
        None => match prompts.last() {
            None => Err(ConversationError::NothingBefore),
            Some(last) if last.finished => Ok(u64::MAX),
            Some(_) if !prompts.iter().any(|prompt| prompt.finished) => {
                Err(ConversationError::NothingBefore)
            }
            Some(last) => Ok(last.start),
        },
    }
}

/// `text` with the session line's `payload.git` naming `head`: its `branch`
/// (when the head has one) and `commit_hash`; its remote is kept.
fn with_head(text: &str, head: &GitHead) -> String {
    let mut out = String::with_capacity(text.len());
    for piece in text.split_inclusive('\n') {
        let rewritten = piece
            .strip_suffix('\n')
            .filter(|line| line.contains("\"session_meta\""))
            .and_then(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|record| record.get("type").and_then(Value::as_str) == Some("session_meta"))
            .and_then(|mut record| {
                let git = record.pointer_mut("/payload/git")?.as_object_mut()?;
                if let Some(branch) = &head.branch {
                    git.insert("branch".to_owned(), Value::from(branch.as_str()));
                }
                git.insert("commit_hash".to_owned(), Value::from(head.sha.as_str()));
                Some(record.to_string())
            });
        match rewritten {
            Some(line) => {
                out.push_str(&line);
                out.push('\n');
            }
            None => out.push_str(piece),
        }
    }
    out
}

/// The CLI's thread index under `codex` opened for reading and writing, and
/// its path; `None` when there is none.
fn open_state(codex: &Path) -> Result<Option<(Connection, PathBuf)>, ConversationError> {
    let db = codex.join(STATE);
    if !is_there(&db)? {
        return Ok(None);
    }
    let state_error = |source| ConversationError::State { path: db.clone(), source };
    let connection = Connection::open_with_flags(&db, OpenFlags::SQLITE_OPEN_READ_WRITE).map_err(state_error)?;
    connection.busy_timeout(Duration::from_secs(5)).map_err(state_error)?;
    Ok(Some((connection, db)))
}

/// Drops what the CLI projected from thread `id`'s rollout into its turn
/// history ([`HISTORY`] in `codex`): the thread's rows in each of
/// [`HISTORY_TABLES`], whose byte offsets a rewrite makes stale. The CLI
/// never repairs stale offsets, and rebuilds a thread's rows from its
/// rollout when they are gone (X2, 0.169.0); its own trigger drops the
/// thread's `thread_realtime_items` with the projection state. No other
/// thread's rows change, and no history file is no error.
fn clear_history(codex: &Path, id: &str) -> Result<(), ConversationError> {
    let db = codex.join(HISTORY);
    if !is_there(&db)? {
        return Ok(());
    }
    let history_error = |source| ConversationError::History { path: db.clone(), source };
    let mut connection =
        Connection::open_with_flags(&db, OpenFlags::SQLITE_OPEN_READ_WRITE).map_err(history_error)?;
    connection.busy_timeout(Duration::from_secs(5)).map_err(history_error)?;
    let transaction = connection.transaction().map_err(history_error)?;
    for table in HISTORY_TABLES {
        transaction
            .execute(&format!("DELETE FROM {table} WHERE thread_id = ?1"), [id])
            .map_err(history_error)?;
    }
    transaction.commit().map_err(history_error)
}

/// Points the thread `id` of the CLI's index (`state_5.sqlite` in `codex`)
/// at its rewritten `rollout` and the new `folder`, and at `head` when
/// given. Only that row changes. No index file is no error: the CLI has
/// nothing to follow.
fn follow_thread(
    codex: &Path,
    id: &str,
    rollout: &Path,
    folder: &Path,
    head: Option<&GitHead>,
) -> Result<(), ConversationError> {
    let db = codex.join(STATE);
    let state_error = |source| ConversationError::State { path: db.clone(), source };
    if !is_there(&db)? {
        return Ok(());
    }
    let connection = Connection::open_with_flags(&db, OpenFlags::SQLITE_OPEN_READ_WRITE).map_err(state_error)?;
    connection.busy_timeout(Duration::from_secs(5)).map_err(state_error)?;
    let (rollout, folder) = (path_text(rollout), path_text(folder));
    connection
        .execute(
            "UPDATE threads SET cwd = ?2, rollout_path = ?3 WHERE id = ?1",
            rusqlite::params![id, folder, rollout],
        )
        .map_err(state_error)?;
    if let Some(head) = head {
        connection
            .execute(
                "UPDATE threads SET git_sha = ?2, git_branch = coalesce(?3, git_branch) WHERE id = ?1",
                rusqlite::params![id, head.sha, head.branch],
            )
            .map_err(state_error)?;
    }
    Ok(())
}

/// The text pairs of a rewrite, each old text to its new one, in order: the
/// Codex dir, then the folder, and with `escaped` the folder JSON-escaped
/// when that differs (CV-4). Pairs whose texts are equal are left out.
fn rewrite_pairs(from: &Placement, to: &Placement, escaped: bool) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    push_pair(
        &mut pairs,
        path_text(&codex_dir(from)),
        path_text(&codex_dir(to)),
    );
    let (old, new) = (path_text(&from.folder), path_text(&to.folder));
    let (old_escaped, new_escaped) = (json_escape(&old), json_escape(&new));
    let escaped_differs =
        escaped && (old_escaped.as_str(), new_escaped.as_str()) != (old.as_str(), new.as_str());
    push_pair(&mut pairs, old, new);
    if escaped_differs {
        push_pair(&mut pairs, old_escaped, new_escaped);
    }
    pairs
}

fn push_pair(pairs: &mut Vec<(String, String)>, old: String, new: String) {
    if old != new && !old.is_empty() {
        pairs.push((old, new));
    }
}

/// Every string of `value` that is exactly `old` becomes `new`, at any depth
/// (CV-2): a fork's conversation id.
fn rename_ids(value: &mut Value, old: &str, new: &str) {
    match value {
        Value::String(text) if text.as_str() == old => *text = new.to_owned(),
        Value::Array(items) => items.iter_mut().for_each(|item| rename_ids(item, old, new)),
        Value::Object(fields) => fields
            .values_mut()
            .for_each(|field| rename_ids(field, old, new)),
        Value::String(_) | Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}
