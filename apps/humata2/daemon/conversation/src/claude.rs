//! Claude Code's JSONL conversations ([`crate::Format::ClaudeJsonl`]).
//!
//! # Where the files are
//!
//! ```text
//! <config>/projects/<dashed folder>/<id>.jsonl   the transcript
//! <config>/projects/<dashed folder>/<id>/...     its companions (side conversations, saved tool output)
//! <config>/projects/<dashed folder>/...          the folder's own files (memory/, other conversations)
//! ```
//!
//! - `<config>` is [`Placement::cli_home`] (the CLI's `CLAUDE_CONFIG_DIR`)
//!   when set, else `<home>/.claude`.
//! - `<dashed folder>` is [`project_name`] of [`Placement::folder`], as
//!   Claude Code 2.1.326 names it (X1, X2) and as `humata2_sessions`'
//!   `{folder_dashed}` fills it: every UTF-16 code unit that isn't an ASCII
//!   letter or digit (`/`, `.`, a space, `_`, an `é`, ...) becomes `-`, so a
//!   character outside the BMP gives two (X1: `/u/me/my_work dir/.x` →
//!   `-u-me-my-work-dir--x`). A name longer than 200 units is cut to its
//!   first 200, then `-` and the base-36 magnitude of the folder's 32-bit
//!   `String.hashCode` (X2). The per-folder record is
//!   `<config>/projects/<dashed folder>`.
//! - [`Adapter::locate`] lists the transcript first, then the conversation's
//!   other files sorted by path: every entry under `<id>/`, and its files in
//!   the config dir (below), as [`crate::FileRole::Companion`], and the
//!   folder's own files (`memory/`) as [`crate::FileRole::Project`]. Another
//!   conversation of the folder (a top-level `*.jsonl` that isn't `<id>`'s,
//!   and a folder with a transcript beside it) is never listed. Links are
//!   listed, never followed. The folder is looked for as given, then in its
//!   canonical form, where the CLI records it (macOS `/var` is
//!   `/private/var`), and the [`Conversation`]'s placement names the one
//!   found (D336).
//! - Its files in the config dir, as [`crate::FileRole::Companion`]:
//!   `file-history/<id>/**` (the person's files, carried byte for byte),
//!   `todos/<id>-agent-*.json`, and `plans/<slug>.md` for the `slug` the
//!   transcript's records carry. Staged as `config/<path under the config
//!   dir>`, and placed under the new config dir. The slug and todo naming
//!   are inferred from the CLI's habits, not yet proven by a capture (X2).
//!   [`Adapter::place`] returns its files in the same order.
//!
//! # The transcript
//!
//! One JSON object per line, each ending in `\n`. A last line without its
//! `\n` is being written: every method ignores it, and a fork or pack
//! leaves it out. Every complete line goes through the **strict parse**,
//! in file order; the first line it refuses decides the error:
//!
//! 1. The line must be a JSON object with a string `type`, else
//!    [`ConversationError::Malformed`] with its 1-based line number.
//! 2. `type` must be one of [`KINDS`].
//! 3. A `user`, `assistant`, `system` or `attachment` record must have a
//!    string `uuid`,
//!    no other record may have the same one, and its `parentUuid` and
//!    `logicalParentUuid`, where present, must be a string or null.
//! 4. A top-level field not in [`FIELDS`] must hold no path: no string in
//!    its value, at any depth, starts with `/` or `~`, or holds the dashed
//!    folder of the conversation's [`Placement`].
//!
//! Steps 2 to 4 refuse with [`ConversationError::Unsupported`], whose
//! `version` is the refused record's top-level `version`, else that of the
//! last record before it that has one, else the CLI version given to
//! [`Adapter::check`] (`None` from every other method). The values of the
//! known fields are free-form (prompts, tool inputs and results): the
//! rewrites below treat them as text. The folder is held by the known
//! top-level fields `cwd`, `attachment` (`snapshot.workingDirectory`),
//! `rendered` (`[].content`) and `message` (`content[].text`), and by any
//! prompt or tool text; the carry rewrites all of them as text, and a fork
//! into another folder rewrites every string of a record (X1).
//!
//! # The current path
//!
//! Records link to the one before them through `parentUuid`. A `/rewind` or
//! an edited prompt writes a new record whose `parentUuid` is an earlier
//! one, and leaves the abandoned branch in the file. The **current path**
//! is the chain from the **leaf**, the last record in the file that has a
//! `uuid` and isn't `"isSidechain":true`, back through `parentUuid`; where
//! that is null, through `logicalParentUuid` (a compaction boundary points
//! at the record before the compaction that way). The chain ends at a
//! record with neither, or one naming a `uuid` the file doesn't hold. A
//! chain that comes back to a record it passed is
//! [`ConversationError::Unsupported`]. The path's records are taken in file
//! order.
//!
//! The path holds `user`, `assistant`, `system` and `attachment` records
//! (X1: a prompt, then the `attachment`s that inject the environment, the
//! date and the skill list, then the reply: user → attachment →
//! assistant). An `attachment` is never a turn.
//!
//! A conversation's **records** are those of the current path, plus:
//! - each `file-history-snapshot` whose `messageId`, and each `summary` or
//!   `last-prompt` whose `leafUuid`, is the `uuid` of one of them;
//! - the **session records**, which have no `uuid` and no parent:
//!   `queue-operation`, `atis-latch`, `api-request`, `api-request-blob`,
//!   `api-request-shape`, `cost-state` and `mode`. A fork keeps those written
//!   before its cut, but not the `queue-operation`s after its last kept path
//!   record (they queue the prompt it cuts before); a carry keeps all of
//!   them.
//!
//! Everything else is left out of forks and packs: abandoned branches
//! (CV-2), side conversations written inline (`"isSidechain":true`; newer
//! CLIs keep them under `<id>/`, carried as companions), and snapshots,
//! summaries and last prompts of left-out records.
//!
//! A **turn** is a record of the current path with `"type":"user"` that is
//! not `"isMeta":true` and not `"isCompactSummary":true`, whose
//! `message.content` is a non-empty string or an array holding a
//! `{"type":"text"}` block with non-empty `text` (the first such block is
//! the prompt). A user record of `tool_result` blocks only is not a turn.
//! Apart from the path and the compact summary this is the rule of
//! `humata2_mirror::turns_of`. [`Turn::first_line`] is the prompt's first
//! line; [`Turn::at`] is the record's `timestamp` string.
//!
//! A turn is **finished** when the last `assistant` record of the path
//! after its prompt (before the next turn, if any) holds no `tool_use`
//! content block and its `message.stop_reason` isn't `"tool_use"`. A turn
//! with no assistant record yet is not finished.
//!
//! # Fork
//!
//! The fork holds the conversation's records that come before turn
//! `before`'s prompt on the current path (with the snapshots, summaries and
//! last prompts that point at them, and the session records before it). For
//! `None` (fork from now) it holds all of them when
//! the last turn is finished, and those before the last turn's prompt when
//! it is still running. Each record is written with:
//! - the folder rewrite below from the parent's [`Placement`] to `to`, when
//!   their folders differ;
//! - `sessionId`, where the record has one, set to the new id.
//!
//! Everything else in each record is kept, and the records keep their file
//! order. The fork is written to `to`'s
//! `<config>/projects/<dashed folder>/<new id>.jsonl`, creating the folders.
//! `before` must be 2 up to the number of turns: 1 is
//! [`ConversationError::NothingBefore`], 0 and past the last turn
//! [`ConversationError::NoSuchTurn`]. From now with no finished turn before
//! the running one (or no turn at all) is
//! [`ConversationError::NothingBefore`]. Every line of the parent is parsed
//! first, so a refused one refuses the fork before anything is written. A
//! fork has no size cap: it rewrites one record at a time.
//!
//! # Carry
//!
//! [`Adapter::pack`] copies every file of the [`Conversation`] to
//! `<stage>/<path under the per-folder record>`: the transcript as its
//! records' lines, unchanged, in file order (the whole current path and every
//! session record, a running turn included: CV-5 leaves it to the CLI on resume), and every
//! other file as it is. [`Adapter::place`] writes each to the same path under
//! `to`'s per-folder record, with every text replaced, in this order:
//! 1. `from`'s per-folder record path → `to`'s;
//! 2. `from.folder` → `to.folder`, as it is;
//! 3. the same, JSON-escaped (`serde_json::to_string` without its quotes),
//!    when that differs from 2;
//! 4. the dashed `from.folder` → the dashed `to.folder`;
//! 5. the same four for `from.folder`'s canonical form, when it differs;
//! 6. `from`'s config dir → `to`'s, then `from.home` → `to.home` (each also
//!    JSON-escaped when that differs), after the folder, which may lie in
//!    the home.
//!
//! `to.folder` is its canonical form when the folder exists. When the place
//! names a head, every record's `gitBranch` is its branch (`None`, a
//! detached head, keeps them).
//!
//! No pairs when the old and new texts are equal. After the rewrite every
//! transcript line must still pass the strict parse (at `to`).
//!
//! Pack and place check every file before they write anything: it must be
//! a plain file, not a link ([`ConversationError::NotAFile`]), at most
//! [`REWRITE_MAX`] bytes ([`ConversationError::TooBig`]), and text: UTF-8
//! with no NUL byte ([`ConversationError::NotText`]). Every transcript line
//! must pass the strict parse. Place also refuses a staged name that is
//! absolute or has a `..` part ([`ConversationError::BadPath`]) and a target
//! that is already there ([`ConversationError::Exists`]).
//!
//! # Versions
//!
//! [`Adapter::check`] runs the strict parse and walks the current path, and
//! says nothing more: the CLI version doesn't decide (CV-7). Claude Code
//! updates itself, so a list of proven versions would turn fork at a turn
//! off after every update; a newer version whose records all parse is
//! known, and one that adds a record kind or a path field is not.
//!
//! # Not yet proven against the real CLI (X2 fixtures)
//!
//! - That [`KINDS`] and [`FIELDS`] cover what a current CLI writes beyond
//!   X1's two plain turns (`sdk-cli`, 2.1.325): a run with tool calls, a
//!   compaction, a subagent, an interactive start and a titled conversation.
//! - Whether `--resume` needs the session records (`queue-operation`,
//!   `atis-latch`, `api-request*`, `cost-state`, `mode`), `last-prompt` and
//!   the `attachment` records. They are kept (the session records before the
//!   cut; the attachments on the path) so that a resume sees what the CLI
//!   wrote; none is known to be required or to be ignored.
//! - That `--resume <new id>` takes a file whose `sessionId`s were
//!   rewritten, and resumes the leaf this module calls the current path.
//! - That a rewind and an edited prompt both branch through `parentUuid`,
//!   and a compaction through `logicalParentUuid`.
//! - The finished-turn rule, if the CLI writes one record per content block
//!   with `stop_reason` null.
//! - That a resumed fork needs no snapshot or summary of left-out records,
//!   and no inline side conversation.
//! - That no file holds two records with one `uuid`.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, BufRead, BufReader, Cursor, Write};
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::files::{
    check_id, complete_lines, holds_files, holds_path, io_error, is_there, json_escape, list_into,
    malformed, open_plain, path_text, read_plain, read_text, relative_name, rewrite_text,
    rewrite_value, stream_complete_lines, under, valid_rel, write_new, write_new_stream,
};
use crate::{
    Adapter, Conversation, ConversationError, ConversationFile, FileRole, Format, GitHead, Packed, PackedFile, Placement,
    Turn,
};

/// The biggest file a carry rewrites. A bigger one is refused, never copied
/// as it is (CV-3).
const REWRITE_MAX: u64 = 256 << 20;

/// The record kinds (`type`) the strict parse knows (CV-7). A record of any
/// other kind makes the conversation unknown. Add one only once the
/// real-CLI fixtures show what it holds and how a fork and a carry treat it.
/// The [`PATH_KINDS`] carry a `uuid` and sit on the current path;
/// `file-history-snapshot`, `summary` and `last-prompt` point at a record of
/// it; the rest are [`SESSION_KINDS`].
const KINDS: &[&str] = &[
    "user",
    "assistant",
    "system",
    "attachment",
    "summary",
    "file-history-snapshot",
    "last-prompt",
    "queue-operation",
    "atis-latch",
    "api-request",
    "api-request-blob",
    "api-request-shape",
    "cost-state",
    "mode",
];

/// The kinds that must have a `uuid`: the records of the current path.
const PATH_KINDS: &[&str] = &["user", "assistant", "system", "attachment"];

/// The kinds written beside the path, with no `uuid` and no parent: the
/// queue, the API request log, and the cost and mode state. A fork keeps
/// those before its cut and a carry keeps all of them.
const SESSION_KINDS: &[&str] = &[
    "queue-operation",
    "atis-latch",
    "api-request",
    "api-request-blob",
    "api-request-shape",
    "cost-state",
    "mode",
];

/// The top-level record fields the strict parse knows (CV-7). Of these
/// `cwd`, `attachment`, `rendered` and `message` hold the folder, and the
/// carry rewrite handles it; the values of the others are rewritten as text
/// too. An unknown field is accepted only while it holds no path. Add one
/// only once the fixtures show it.
const FIELDS: &[&str] = &[
    "type",
    "uuid",
    "parentUuid",
    "logicalParentUuid",
    "isSidechain",
    "userType",
    "cwd",
    "sessionId",
    "version",
    "gitBranch",
    "message",
    "requestId",
    "timestamp",
    "isMeta",
    "isCompactSummary",
    "isVisibleInTranscriptOnly",
    "toolUseResult",
    "subtype",
    "content",
    "level",
    "compactMetadata",
    "messageId",
    "snapshot",
    "isSnapshotUpdate",
    "summary",
    "leafUuid",
    // user (X1, 2.1.325)
    "promptId",
    "permissionMode",
    "promptSource",
    "turnOrigin",
    "turnPosition",
    "entrypoint",
    // attachment
    "attachment",
    "rendered",
    "renderedRole",
    "renderedBesideToolResult",
    // assistant
    "apiBlockIndex",
    "requestRef",
    "requestedModel",
    "promptCacheRequested",
    "effort",
    "perTurnEffort",
    "madeBy",
    // queue-operation, atis-latch, last-prompt, mode
    "operation",
    "atis",
    "lastPrompt",
    "mode",
    // api-request, api-request-blob, api-request-shape
    "id",
    "querySource",
    "shapeHash",
    "params",
    "keep",
    "tail",
    "cacheControl",
    "requestDigest",
    "hash",
    "shape",
    // cost-state
    "totalCostUSD",
    "totalAPIDuration",
    "totalAPIDurationWithoutRetries",
    "totalToolDuration",
    "totalLinesAdded",
    "totalLinesRemoved",
    "totalDuration",
    "startTime",
    "modelUsage",
    "hasUnknownModelCost",
];

/// The adapter for [`crate::Format::ClaudeJsonl`].
pub(crate) struct ClaudeJsonl {
    /// The biggest file [`Adapter::pack`] and [`Adapter::place`] take.
    rewrite_max: u64,
}

impl ClaudeJsonl {
    /// The adapter with the real cap.
    pub(crate) const DEFAULT: Self = Self {
        rewrite_max: REWRITE_MAX,
    };
}

impl Adapter for ClaudeJsonl {
    fn locate(&self, at: &Placement, id: &str) -> Result<Conversation, ConversationError> {
        check_id(id)?;
        let (at, transcript) = locate_transcript(at, id)?;
        let at = &at;
        let project = project_dir(at);
        let meta = fs::symlink_metadata(&transcript).map_err(|source| match source.kind() {
            io::ErrorKind::NotFound => ConversationError::NotFound {
                path: transcript.clone(),
            },
            _ => io_error("read the transcript", &transcript, source),
        })?;
        if !meta.file_type().is_file() {
            return Err(ConversationError::NotAFile { path: transcript });
        }
        let mut listed = Vec::new();
        list_into(&project, &mut listed)?;
        let mut others = Vec::new();
        for path in listed {
            if path != transcript && !is_another_conversations(&path, &project, id)? {
                others.push(path);
            }
        }
        let companions = project.join(id);
        let mut own = side_files(at, id, &transcript)?;
        own.extend(others.iter().filter(|path| path.starts_with(&companions)).cloned());
        others.retain(|path| !path.starts_with(&companions));
        let mut rest: Vec<ConversationFile> = own
            .into_iter()
            .map(|path| ConversationFile { path, role: FileRole::Companion })
            .chain(others.into_iter().map(|path| ConversationFile { path, role: FileRole::Project }))
            .collect();
        rest.sort_by(|a, b| a.path.cmp(&b.path));
        let mut files = vec![ConversationFile {
            path: transcript,
            role: FileRole::Transcript,
        }];
        files.extend(rest);
        Ok(Conversation {
            id: id.to_owned(),
            format: Format::ClaudeJsonl,
            placement: at.clone(),
            files,
        })
    }

    fn turns(&self, conversation: &Conversation) -> Result<Vec<Turn>, ConversationError> {
        ensure_claude(conversation.format)?;
        let path = transcript_path_of(conversation)?;
        let file = open_plain(&path, u64::MAX)?;
        let transcript = parse_transcript_reader(BufReader::new(file), &path, &conversation.placement, None)?;
        let on_path = current_path(&transcript, None)?;
        Ok((1_u32..)
            .zip(prompts_on(&transcript, &on_path))
            .map(|(index, prompt)| Turn {
                index,
                first_line: prompt.first_line,
                at: prompt.at,
            })
            .collect())
    }

    fn fork(
        &self,
        conversation: &Conversation,
        before: Option<u32>,
        new_id: &str,
        to: &Placement,
    ) -> Result<Conversation, ConversationError> {
        ensure_claude(conversation.format)?;
        check_id(new_id)?;
        let path = transcript_path_of(conversation)?;
        let file = open_plain(&path, u64::MAX)?;
        let transcript = parse_transcript_reader(BufReader::new(file), &path, &conversation.placement, None)?;
        let on_path = current_path(&transcript, None)?;
        let prompts = prompts_on(&transcript, &on_path);
        let cut = cut_for(&prompts, before)?;
        if !prompts.iter().any(|prompt| prompt.line < cut) {
            return Err(ConversationError::NothingBefore);
        }
        let target = project_dir(to).join(format!("{new_id}.jsonl"));
        if is_there(&target)? {
            return Err(ConversationError::Exists { path: target });
        }
        let pairs = if conversation.placement.folder == to.folder {
            Vec::new()
        } else {
            rewrite_pairs(&conversation.placement, to, false)
        };
        let keep = kept_lines(&transcript, &on_path, cut);
        let pass2 = open_plain(&path, u64::MAX)?;
        write_new_stream(&target, |writer| {
            stream_complete_lines(BufReader::new(pass2), &path, |number, _, raw| {
                let idx = usize::try_from(number - 1).unwrap_or(usize::MAX);
                if !keep.get(idx).copied().unwrap_or(false) {
                    return Ok(());
                }
                let mut record: Value = serde_json::from_slice(raw).map_err(|source| {
                    ConversationError::Malformed {
                        path: path.clone(),
                        line: number,
                        source: Some(source),
                    }
                })?;
                rewrite_value(&mut record, &pairs);
                if let Some(session) = record.get_mut("sessionId") {
                    *session = Value::from(new_id);
                }
                serde_json::to_writer(&mut *writer, &record)
                    .map_err(|source| io_error("write", &target, io::Error::other(source)))?;
                writer
                    .write_all(b"\n")
                    .map_err(|source| io_error("write", &target, source))?;
                Ok(())
            })
        })?;
        Ok(Conversation {
            id: new_id.to_owned(),
            format: Format::ClaudeJsonl,
            placement: to.clone(),
            files: vec![ConversationFile {
                path: target,
                role: FileRole::Transcript,
            }],
        })
    }

    fn pack(&self, conversation: &Conversation, stage: &Path) -> Result<Packed, ConversationError> {
        ensure_claude(conversation.format)?;
        let project = project_dir(&conversation.placement);
        let config = config_dir(&conversation.placement);
        let mut staged = Vec::with_capacity(conversation.files.len());
        for file in &conversation.files {
            let rel = if file.path.starts_with(&project) {
                relative_name(&file.path, &project)?
            } else {
                format!("{CONFIG_REL}/{}", relative_name(&file.path, &config)?)
            };
            if is_backup(&rel) {
                staged.push((rel, file.role, read_plain(&file.path, self.rewrite_max)?));
                continue;
            }
            let text = read_text(&file.path, self.rewrite_max)?;
            let body = if file.role == FileRole::Transcript {
                let transcript = parse_transcript_reader(
                    Cursor::new(text.as_bytes()),
                    &file.path,
                    &conversation.placement,
                    None,
                )?;
                let on_path = current_path(&transcript, None)?;
                let keep = kept_lines(&transcript, &on_path, usize::MAX);
                let mut body = Vec::new();
                for (raw, keep) in complete_lines(text.as_bytes()).into_iter().zip(keep) {
                    if keep {
                        body.extend_from_slice(raw);
                        body.push(b'\n');
                    }
                }
                body
            } else {
                text.into_bytes()
            };
            staged.push((rel, file.role, body));
        }
        if holds_files(stage)? {
            return Err(ConversationError::Exists {
                path: stage.to_path_buf(),
            });
        }
        let mut files = Vec::with_capacity(staged.len());
        for (rel, role, body) in staged {
            let target = under(stage, &rel);
            write_new(&target, &body)?;
            let size = u64::try_from(body.len()).unwrap_or(u64::MAX);
            files.push(PackedFile { rel, role, size });
        }
        Ok(Packed {
            id: conversation.id.clone(),
            format: Format::ClaudeJsonl,
            stage: stage.to_path_buf(),
            files,
        })
    }

    fn place(&self, packed: &Packed, from: &Placement, to: &Placement) -> Result<Conversation, ConversationError> {
        self.place_on_head(packed, from, to, None)
    }

    fn place_on_head(
        &self,
        packed: &Packed,
        from: &Placement,
        to: &Placement,
        head: Option<&GitHead>,
    ) -> Result<Conversation, ConversationError> {
        ensure_claude(packed.format)?;
        let to = &canonical(to);
        check_id(&packed.id)?;
        if let Some(file) = packed.files.iter().find(|file| !valid_rel(&file.rel)) {
            return Err(ConversationError::BadPath {
                rel: file.rel.clone(),
            });
        }
        let transcript_name = format!("{}.jsonl", packed.id);
        if !packed
            .files
            .iter()
            .any(|file| file.role == FileRole::Transcript && file.rel == transcript_name)
        {
            return Err(ConversationError::NotFound {
                path: under(&packed.stage, &transcript_name),
            });
        }
        let pairs = carry_pairs(from, to);
        let to_project = project_dir(to);
        let to_config = config_dir(to);
        let mut files = Vec::with_capacity(packed.files.len());
        let mut writes = Vec::with_capacity(packed.files.len());
        for file in &packed.files {
            let target = match file.rel.strip_prefix(CONFIG_PREFIX) {
                Some(rest) => under(&to_config, rest),
                None => under(&to_project, &file.rel),
            };
            let source = under(&packed.stage, &file.rel);
            let body = if is_backup(&file.rel) {
                read_plain(&source, self.rewrite_max)?
            } else {
                let text = read_text(&source, self.rewrite_max)?;
                let mut rewritten = rewrite_text(&text, &pairs);
                if file.role == FileRole::Transcript
                    && let Some(branch) = head.and_then(|head| head.branch.as_deref())
                {
                    rewritten = with_branch(&rewritten, branch);
                }
                if file.role == FileRole::Transcript {
                    parse_transcript_reader(Cursor::new(rewritten.as_bytes()), &target, to, None)?;
                }
                rewritten.into_bytes()
            };
            if is_there(&target)? {
                return Err(ConversationError::Exists { path: target });
            }
            files.push(ConversationFile {
                path: target.clone(),
                role: file.role,
            });
            writes.push((target, body));
        }
        for (target, body) in &writes {
            write_new(target, body)?;
        }
        Ok(Conversation {
            id: packed.id.clone(),
            format: Format::ClaudeJsonl,
            placement: to.clone(),
            files,
        })
    }

    fn check(&self, conversation: &Conversation, cli_version: Option<&str>) -> Result<(), ConversationError> {
        ensure_claude(conversation.format)?;
        let path = transcript_path_of(conversation)?;
        let file = open_plain(&path, u64::MAX)?;
        let transcript = parse_transcript_reader(BufReader::new(file), &path, &conversation.placement, cli_version)?;
        current_path(&transcript, cli_version).map(drop)
    }
}

/// Lightweight per-line metadata extracted while streaming a transcript,
/// avoiding retaining full `serde_json::Value` DOMs in memory.
struct LineMeta {
    kind: String,
    uuid: Option<String>,
    parent_uuid: Option<String>,
    is_sidechain: bool,
    version: Option<String>,
    /// Target uuid for `file-history-snapshot` (`messageId`), `summary` / `last-prompt` (`leafUuid`).
    points_to: Option<String>,
    /// `(first_line, timestamp)` when this line starts a turn.
    prompt: Option<(String, Option<String>)>,
    /// Whether an `assistant` record finishes a reply.
    finished_reply: bool,
}

/// A transcript that passed the strict parse, every complete line of it.
struct TranscriptIndex {
    lines: Vec<LineMeta>,
    /// Each record's `uuid`, to its index in `lines`.
    by_uuid: HashMap<String, usize>,
}

/// A turn on the current path, as the fork cut and the turn list need it.
struct Prompt {
    /// Its record's index in `lines`.
    line: usize,
    first_line: String,
    at: Option<String>,
    /// Whether the reply after it on the path is finished.
    finished: bool,
}

/// The strict parse (the module's steps 1 to 4) of every complete line of
/// `reader`, streamed in file order. The first line it refuses decides the error.
fn parse_transcript_reader<R: BufRead>(
    reader: R,
    path: &Path,
    at: &Placement,
    cli_version: Option<&str>,
) -> Result<TranscriptIndex, ConversationError> {
    let folder = project_name(&at.folder);
    let mut lines: Vec<LineMeta> = Vec::new();
    let mut by_uuid = HashMap::new();
    let mut last_version: Option<String> = None;
    stream_complete_lines(reader, path, |number, _, raw| {
        let value: Value = serde_json::from_slice(raw).map_err(|source| ConversationError::Malformed {
            path: path.to_path_buf(),
            line: number,
            source: Some(source),
        })?;
        let Some(record) = value.as_object() else {
            return Err(malformed(path, number));
        };
        let Some(kind) = record.get("type").and_then(Value::as_str) else {
            return Err(malformed(path, number));
        };
        let own = record.get("version").and_then(Value::as_str);
        let last = last_version.as_deref();
        if !KINDS.contains(&kind) {
            return Err(unsupported(own, last, cli_version));
        }
        let uuid = match record.get("uuid") {
            Some(uuid) => {
                let Some(uuid) = uuid.as_str() else {
                    return Err(unsupported(own, last, cli_version));
                };
                if by_uuid.insert(uuid.to_owned(), lines.len()).is_some() {
                    return Err(unsupported(own, last, cli_version));
                }
                Some(uuid.to_owned())
            }
            None if PATH_KINDS.contains(&kind) => {
                return Err(unsupported(own, last, cli_version));
            }
            None => None,
        };
        for key in ["parentUuid", "logicalParentUuid"] {
            if record
                .get(key)
                .is_some_and(|field| !(field.is_string() || field.is_null()))
            {
                return Err(unsupported(own, last, cli_version));
            }
        }
        if record
            .iter()
            .any(|(key, field)| !FIELDS.contains(&key.as_str()) && holds_path(field, &folder))
        {
            return Err(unsupported(own, last, cli_version));
        }
        if let Some(version) = own {
            last_version = Some(version.to_owned());
        }
        let points_to = match kind {
            "file-history-snapshot" => record.get("messageId").and_then(Value::as_str).map(str::to_owned),
            "summary" | "last-prompt" => record.get("leafUuid").and_then(Value::as_str).map(str::to_owned),
            _ => None,
        };
        let prompt = prompt_of(&value).map(|text| {
            (
                text.lines().next().unwrap_or_default().to_owned(),
                record.get("timestamp").and_then(Value::as_str).map(str::to_owned),
            )
        });
        let finished = kind == "assistant" && finished_reply(&value);
        lines.push(LineMeta {
            kind: kind.to_owned(),
            uuid,
            parent_uuid: parent_uuid(&value).map(str::to_owned),
            is_sidechain: is_sidechain(&value),
            version: own.map(str::to_owned),
            points_to,
            prompt,
            finished_reply: finished,
        });
        Ok(())
    })?;
    Ok(TranscriptIndex { lines, by_uuid })
}

/// The refusal for a record the strict parse doesn't know. The version is
/// the record's own, else the last record's before it, else the CLI's.
fn unsupported(own: Option<&str>, last: Option<&str>, cli_version: Option<&str>) -> ConversationError {
    ConversationError::Unsupported {
        format: Format::ClaudeJsonl,
        version: own.or(last).or(cli_version).map(str::to_owned),
    }
}

/// The refusal for a chain that loops, naming the version as of its record.
fn unsupported_at(transcript: &TranscriptIndex, index: usize, cli_version: Option<&str>) -> ConversationError {
    let version = transcript
        .lines
        .iter()
        .take(index.saturating_add(1))
        .rev()
        .find_map(|line| line.version.as_deref())
        .or(cli_version);
    ConversationError::Unsupported {
        format: Format::ClaudeJsonl,
        version: version.map(str::to_owned),
    }
}

/// The current path (CV-2): the chain from the leaf back through the
/// parents, as a mask over `transcript.lines`.
fn current_path(transcript: &TranscriptIndex, cli_version: Option<&str>) -> Result<Vec<bool>, ConversationError> {
    let mut on_path = vec![false; transcript.lines.len()];
    let leaf = transcript
        .lines
        .iter()
        .rposition(|line| line.uuid.is_some() && !line.is_sidechain);
    let mut next = leaf;
    while let Some(index) = next {
        let Some(seen) = on_path.get_mut(index) else {
            break;
        };
        if *seen {
            return Err(unsupported_at(transcript, index, cli_version));
        }
        *seen = true;
        next = transcript
            .lines
            .get(index)
            .and_then(|line| line.parent_uuid.as_deref())
            .and_then(|uuid| transcript.by_uuid.get(uuid).copied());
    }
    Ok(on_path)
}

/// The turns on the current path, in order, each with whether its reply is
/// finished (the last assistant record after it holds no tool call and
/// doesn't stop for one).
fn prompts_on(transcript: &TranscriptIndex, on_path: &[bool]) -> Vec<Prompt> {
    let mut prompts: Vec<Prompt> = Vec::new();
    for (index, (line, on)) in transcript.lines.iter().zip(on_path).enumerate() {
        if !*on {
            continue;
        }
        if let Some((first_line, at)) = &line.prompt {
            prompts.push(Prompt {
                line: index,
                first_line: first_line.clone(),
                at: at.clone(),
                finished: false,
            });
        } else if line.kind == "assistant" {
            if let Some(prompt) = prompts.last_mut() {
                prompt.finished = line.finished_reply;
            }
        }
    }
    prompts
}

/// The line a fork cuts at, as an index into `lines` (records before it are
/// kept): the start of turn `before`, or with `None` the end of the path, or
/// the running turn's start.
fn cut_for(prompts: &[Prompt], before: Option<u32>) -> Result<usize, ConversationError> {
    match before {
        Some(turn) => {
            let wanted = turn
                .checked_sub(1)
                .and_then(|n| usize::try_from(n).ok())
                .and_then(|n| prompts.get(n));
            match wanted {
                Some(prompt) => Ok(prompt.line),
                None => Err(ConversationError::NoSuchTurn {
                    turn,
                    turns: u32::try_from(prompts.len()).unwrap_or(u32::MAX),
                }),
            }
        }
        None => Ok(match prompts.last() {
            None => 0,
            Some(last) if last.finished => usize::MAX,
            Some(last) => last.line,
        }),
    }
}

/// Which records a fork before `cut` (or a carry, with `usize::MAX`) keeps:
/// the current path's records before `cut` that aren't side conversations,
/// each snapshot, summary and last prompt that points at one of them, and
/// the session records before `cut` (CV-2).
fn kept_lines(transcript: &TranscriptIndex, on_path: &[bool], cut: usize) -> Vec<bool> {
    let base: Vec<bool> = transcript
        .lines
        .iter()
        .zip(on_path)
        .enumerate()
        .map(|(index, (line, on))| *on && index < cut && !line.is_sidechain)
        .collect();
    let kept_uuids: HashSet<&str> = transcript
        .lines
        .iter()
        .zip(&base)
        .filter(|(_, keep)| **keep)
        .filter_map(|(line, _)| line.uuid.as_deref())
        .collect();
    let last_kept = base.iter().rposition(|keep| *keep);
    transcript
        .lines
        .iter()
        .zip(base)
        .enumerate()
        .map(|(index, (line, keep))| {
            keep || line.points_to.as_deref().is_some_and(|uuid| kept_uuids.contains(uuid))
                || session_record_kept(&line.kind, index, cut, last_kept)
        })
        .collect()
}

/// Whether a session record (no `uuid`, no parent) is kept: it comes before
/// `cut`, and a fork (`cut` short of the end) leaves out the queue entries
/// after the last kept path record, which queue the prompt it cuts before.
fn session_record_kept(kind: &str, index: usize, cut: usize, last_kept: Option<usize>) -> bool {
    if !SESSION_KINDS.contains(&kind) || index >= cut {
        return false;
    }
    let queues_the_cut_prompt =
        kind == "queue-operation" && cut != usize::MAX && last_kept.is_none_or(|last| index > last);
    !queues_the_cut_prompt
}

/// Whether an assistant record ends its reply: no tool call, and not
/// stopped for one.
fn finished_reply(value: &Value) -> bool {
    !has_tool_use(value) && value.pointer("/message/stop_reason").and_then(Value::as_str) != Some("tool_use")
}

fn has_tool_use(value: &Value) -> bool {
    value
        .pointer("/message/content")
        .and_then(Value::as_array)
        .is_some_and(|blocks| {
            blocks
                .iter()
                .any(|block| block.get("type").and_then(Value::as_str) == Some("tool_use"))
        })
}

/// The text of a record that starts a turn, if it is one: the mirror's rule
/// (a user record that is neither a side conversation nor `isMeta`, whose
/// content is a non-empty string or holds a non-empty `text` block), and
/// not a compact summary.
fn prompt_of(value: &Value) -> Option<&str> {
    if kind_of(value) != Some("user")
        || value.get("isSidechain").and_then(Value::as_bool) == Some(true)
        || value.get("isMeta").and_then(Value::as_bool) == Some(true)
        || value.get("isCompactSummary").and_then(Value::as_bool) == Some(true)
    {
        return None;
    }
    match value.pointer("/message/content")? {
        Value::String(text) if !text.is_empty() => Some(text.as_str()),
        Value::Array(blocks) => blocks.iter().find_map(text_block),
        _ => None,
    }
}

/// The text of a `text` content block, if it isn't empty.
fn text_block(block: &Value) -> Option<&str> {
    if block.get("type").and_then(Value::as_str) != Some("text") {
        return None;
    }
    block
        .get("text")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
}

fn kind_of(value: &Value) -> Option<&str> {
    value.get("type").and_then(Value::as_str)
}

/// The record a record continues from: its `parentUuid`, else (where that is
/// null or absent) its `logicalParentUuid`.
fn parent_uuid(value: &Value) -> Option<&str> {
    value
        .get("parentUuid")
        .and_then(Value::as_str)
        .or_else(|| value.get("logicalParentUuid").and_then(Value::as_str))
}

fn is_sidechain(value: &Value) -> bool {
    value.get("isSidechain").and_then(Value::as_bool) == Some(true)
}

/// The folder a Claude conversation of `at` lives under: its config dir's
/// `projects/<dashed folder>`.
fn project_dir(at: &Placement) -> PathBuf {
    config_dir(at).join("projects").join(project_name(&at.folder))
}

/// The CLI's config dir at `at`: `CLAUDE_CONFIG_DIR`, else `<home>/.claude`.
fn config_dir(at: &Placement) -> PathBuf {
    match &at.cli_home {
        Some(config) => config.clone(),
        None => at.home.join(".claude"),
    }
}

/// Where a conversation's files outside the per-folder record are staged:
/// `config/<path under the config dir>`.
const CONFIG_REL: &str = "config";
const CONFIG_PREFIX: &str = "config/";

/// Whether a staged name is a backup under `file-history/`, which is copied
/// byte for byte: it is the person's file, not the CLI's text.
fn is_backup(rel: &str) -> bool {
    rel.starts_with("config/file-history/")
}

/// `at` with its folder as the CLI records it (D336): the real path when the
/// folder exists (macOS `/var` is `/private/var`), else as given.
fn canonical(at: &Placement) -> Placement {
    match fs::canonicalize(&at.folder) {
        Ok(folder) => Placement { folder, ..at.clone() },
        Err(_) => at.clone(),
    }
}

/// The placement and transcript path of conversation `id` for `at`: the
/// folder as given when its record is there, else its canonical form.
fn locate_transcript(at: &Placement, id: &str) -> Result<(Placement, PathBuf), ConversationError> {
    let given = project_dir(at).join(format!("{id}.jsonl"));
    if is_there(&given)? {
        return Ok((at.clone(), given));
    }
    let real = canonical(at);
    let recorded = project_dir(&real).join(format!("{id}.jsonl"));
    if real.folder != at.folder && is_there(&recorded)? {
        Ok((real, recorded))
    } else {
        Ok((at.clone(), given))
    }
}

/// Whether `path`, under the per-folder record `project`, belongs to another
/// conversation of the folder: a top-level `*.jsonl` that isn't `id`'s, or a
/// folder named for one that has its own transcript beside it.
fn is_another_conversations(path: &Path, project: &Path, id: &str) -> Result<bool, ConversationError> {
    let Ok(rest) = path.strip_prefix(project) else {
        return Ok(false);
    };
    let mut parts = rest.components();
    let (Some(first), second) = (parts.next(), parts.next()) else {
        return Ok(false);
    };
    let name = first.as_os_str().to_string_lossy();
    if second.is_none() {
        return Ok(name.ends_with(".jsonl") && name != format!("{id}.jsonl"));
    }
    Ok(name != id && is_there(&project.join(format!("{name}.jsonl")))?)
}

/// The conversation's own files in the config dir (D336): its
/// `file-history/<id>/` backups, its `todos/<id>-agent-*.json` and the plan
/// named by the transcript's `slug`.
fn side_files(at: &Placement, id: &str, transcript: &Path) -> Result<Vec<PathBuf>, ConversationError> {
    let config = config_dir(at);
    let mut found = Vec::new();
    let history = config.join("file-history").join(id);
    if is_there(&history)? {
        list_into(&history, &mut found)?;
    }
    let todos = config.join("todos");
    let prefix = format!("{id}-agent-");
    match fs::read_dir(&todos) {
        Ok(entries) => {
            for entry in entries {
                let entry = entry.map_err(|source| io_error("list the folder", &todos, source))?;
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with(&prefix) && name.ends_with(".json") {
                    found.push(entry.path());
                }
            }
        }
        Err(source) if source.kind() == io::ErrorKind::NotFound => {}
        Err(source) => return Err(io_error("list the folder", &todos, source)),
    }
    if let Some(slug) = slug_of(transcript)? {
        let plan = config.join("plans").join(format!("{slug}.md"));
        if is_there(&plan)? {
            found.push(plan);
        }
    }
    Ok(found)
}

/// The plan name the transcript's records carry (`slug`), when they have
/// one and it is one plain name.
fn slug_of(transcript: &Path) -> Result<Option<String>, ConversationError> {
    let file = fs::File::open(transcript).map_err(|source| io_error("read the transcript", transcript, source))?;
    for line in io::BufReader::new(file).lines() {
        let line = line.map_err(|source| io_error("read the transcript", transcript, source))?;
        if !line.contains("\"slug\"") {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some(slug) = value.get("slug").and_then(Value::as_str) {
            return Ok(check_id(slug).is_ok().then(|| slug.to_owned()));
        }
    }
    Ok(None)
}

/// `text` with every record's top-level `gitBranch` set to `branch`.
fn with_branch(text: &str, branch: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for piece in text.split_inclusive('\n') {
        let rewritten = piece
            .strip_suffix('\n')
            .filter(|line| line.contains("\"gitBranch\""))
            .and_then(|line| serde_json::from_str::<Value>(line).ok())
            .and_then(|mut record| {
                *record.get_mut("gitBranch")? = Value::from(branch);
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

/// The longest per-folder record name, in UTF-16 code units, the CLI keeps
/// as it is; a longer one is cut to this many and hashed (X2, 2.1.326).
const PROJECT_NAME_MAX: usize = 200;

/// The name Claude Code gives the per-folder record of `folder` (X1, X2,
/// 2.1.326), and what `humata2_sessions` fills `{folder_dashed}` with. The
/// folder's text is taken as UTF-16, the way the CLI holds it: each code
/// unit that isn't an ASCII letter or digit becomes `-`, so `é` gives one
/// and a character outside the BMP two. A name of at most 200 units is
/// that; a longer one is its first 200, `-`, and the magnitude of the
/// folder's 32-bit `String.hashCode` in lowercase base 36.
pub fn project_name(folder: &Path) -> String {
    let text = folder.to_string_lossy();
    let mut name = String::with_capacity(text.len());
    let mut hash = 0_i32;
    for unit in text.encode_utf16() {
        hash = hash.wrapping_mul(31).wrapping_add(i32::from(unit));
        name.push(match u8::try_from(unit) {
            Ok(byte) if byte.is_ascii_alphanumeric() => char::from(byte),
            _ => '-',
        });
    }
    // Every character of `name` is ASCII, so its bytes are its units.
    if name.len() <= PROJECT_NAME_MAX {
        return name;
    }
    name.truncate(PROJECT_NAME_MAX);
    name.push('-');
    name.push_str(&base36(hash.unsigned_abs()));
    name
}

/// `n` in lowercase base 36, as the CLI's `toString(36)` writes it.
fn base36(mut n: u32) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut digits = Vec::new();
    loop {
        digits.push(char::from(DIGITS[(n % 36) as usize]));
        n /= 36;
        if n == 0 {
            break;
        }
    }
    digits.iter().rev().collect()
}

fn transcript_path_of(conversation: &Conversation) -> Result<PathBuf, ConversationError> {
    match conversation.files.iter().find(|file| file.role == FileRole::Transcript) {
        Some(file) => Ok(file.path.clone()),
        None => Err(ConversationError::NotFound {
            path: project_dir(&conversation.placement).join(format!("{}.jsonl", conversation.id)),
        }),
    }
}

fn ensure_claude(format: Format) -> Result<(), ConversationError> {
    match format {
        Format::ClaudeJsonl => Ok(()),
        other => Err(ConversationError::Unsupported {
            format: other,
            version: None,
        }),
    }
}

/// The folder rewrite (CV-4): the old per-folder record's path, the old
/// folder, its JSON-escaped form when that differs (`escaped`), and the
/// dashed folder, each to its new text. Pairs whose texts are equal are left
/// out.
fn rewrite_pairs(from: &Placement, to: &Placement, escaped: bool) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    push_pair(
        &mut pairs,
        path_text(&project_dir(from)),
        path_text(&project_dir(to)),
    );
    let (old, new) = (path_text(&from.folder), path_text(&to.folder));
    push_pair(&mut pairs, old.clone(), new.clone());
    if escaped {
        let (escaped_old, escaped_new) = (json_escape(&old), json_escape(&new));
        if (escaped_old.as_str(), escaped_new.as_str()) != (old.as_str(), new.as_str()) {
            push_pair(&mut pairs, escaped_old, escaped_new);
        }
    }
    push_pair(&mut pairs, project_name(&from.folder), project_name(&to.folder));
    pairs
}

/// The carry's rewrite (CV-4, D336): the folder rewrite for `from` and, when
/// it differs, its canonical form, then the old config dir and home to the
/// new ones. The folder's texts come first, as the folder may lie in the
/// home, and the config dir before the home, as it may too.
fn carry_pairs(from: &Placement, to: &Placement) -> Vec<(String, String)> {
    let mut pairs = rewrite_pairs(from, to, true);
    let real = canonical(from);
    if real.folder != from.folder {
        pairs.extend(rewrite_pairs(&real, to, true));
    }
    for (old, new) in [
        (config_dir(from), config_dir(to)),
        (from.home.clone(), to.home.clone()),
    ] {
        let (old, new) = (path_text(&old), path_text(&new));
        let (escaped_old, escaped_new) = (json_escape(&old), json_escape(&new));
        push_pair(&mut pairs, old, new);
        push_pair(&mut pairs, escaped_old, escaped_new);
    }
    let mut seen = HashSet::new();
    pairs.retain(|pair| seen.insert(pair.clone()));
    pairs
}

fn push_pair(pairs: &mut Vec<(String, String)>, old: String, new: String) {
    if old != new && !old.is_empty() {
        pairs.push((old, new));
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    /// A conversation of one prompt at a placement under `root`.
    fn one_turn(root: &Path) -> Placement {
        let at = Placement {
            home: root.join("home"),
            cli_home: None,
            folder: root.join("work/fix"),
        };
        let project = at.home.join(".claude/projects").join(project_name(&at.folder));
        fs::create_dir_all(&project).unwrap();
        let line = serde_json::json!({
            "type": "user",
            "uuid": "7c0e5a10-0001-4a8e-9b1d-3f2a6c8d0001",
            "parentUuid": null,
            "sessionId": "c-1",
            "cwd": at.folder,
            "version": "2.1.325",
            "message": {"role": "user", "content": "Fix the login redirect"},
            "timestamp": "2026-11-16T09:01:00.000Z",
        });
        fs::write(project.join("c-1.jsonl"), format!("{line}\n")).unwrap();
        at
    }

    /// How many files are under `dir`, in any folder; none when it is absent.
    fn files_under(dir: &Path) -> usize {
        let Ok(entries) = fs::read_dir(dir) else { return 0 };
        entries
            .map(|entry| entry.unwrap().path())
            .map(|path| if path.is_dir() { files_under(&path) } else { 1 })
            .sum()
    }

    #[test]
    fn a_file_over_the_cap_is_refused_and_nothing_is_staged() {
        let root = tempfile::tempdir().unwrap();
        let at = one_turn(root.path());
        let small = ClaudeJsonl { rewrite_max: 64 };
        let conversation = small.locate(&at, "c-1").unwrap();
        let stage = root.path().join("stage");

        assert!(matches!(
            small.pack(&conversation, &stage),
            Err(ConversationError::TooBig { .. })
        ));
        assert_eq!(files_under(&stage), 0);
    }

    #[test]
    fn a_staged_file_over_the_cap_is_refused_and_nothing_is_placed() {
        let root = tempfile::tempdir().unwrap();
        let from = one_turn(root.path());
        let conversation = ClaudeJsonl::DEFAULT.locate(&from, "c-1").unwrap();
        let packed = ClaudeJsonl::DEFAULT
            .pack(&conversation, &root.path().join("stage"))
            .unwrap();
        let to = Placement {
            home: root.path().join("other"),
            cli_home: None,
            folder: root.path().join("there/fix"),
        };

        let small = ClaudeJsonl { rewrite_max: 64 };
        assert!(matches!(
            small.place(&packed, &from, &to),
            Err(ConversationError::TooBig { .. })
        ));
        assert!(!to.home.join(".claude").exists());
    }

    /// Where X2 ran Claude Code 2.1.326, and so where each name below is
    /// the per-folder record the CLI made.
    const X2: &str = "/private/tmp/claude-501/h2x2fix";
    const X2_DASHED: &str = "-private-tmp-claude-501-h2x2fix";

    fn named(folder: &str) -> String {
        project_name(Path::new(folder))
    }

    #[test]
    fn a_folder_of_at_most_200_units_is_named_whole_with_each_other_unit_a_dash() {
        assert_eq!(named("/u/me/my_work dir/.x"), "-u-me-my-work-dir--x");
        assert_eq!(named(&format!("{X2}/s😀t")), format!("{X2_DASHED}-s--t"));
        assert_eq!(
            named(&format!("{X2}/w/{}", "a".repeat(166))),
            format!("{X2_DASHED}-w-{}", "a".repeat(166))
        );
        // 200 UTF-16 units, 199 characters: the emoji is two units.
        assert_eq!(
            named(&format!("{X2}/e😀/{}", "h".repeat(164))),
            format!("{X2_DASHED}-e---{}", "h".repeat(164))
        );
    }

    #[test]
    fn a_longer_folder_is_cut_to_200_units_and_named_by_its_hash() {
        assert_eq!(
            named(&format!("{X2}/w/{}", "b".repeat(167))),
            format!("{X2_DASHED}-w-{}-svwx2v", "b".repeat(166))
        );
        assert_eq!(
            named(&format!("{X2}/my_work dir/{}/.x/{}", "c".repeat(120), "d".repeat(100))),
            format!("{X2_DASHED}-my-work-dir-{}--x-{}-vgkxsp", "c".repeat(120), "d".repeat(32))
        );
        assert_eq!(
            named(&format!("{X2}/acc/{}/café b_c.d/{}", "a".repeat(150), "g".repeat(60))),
            format!("{X2_DASHED}-acc-{}-caf--b-c-d-gg-3zpaas", "a".repeat(150))
        );
    }

    #[test]
    fn a_folder_whose_hash_is_negative_is_named_by_its_magnitude() {
        assert_eq!(
            named(&format!("{X2}/neg/{}/m0/{}", "n".repeat(150), "k".repeat(80))),
            format!("{X2_DASHED}-neg-{}-m0-{}-ems0wh", "n".repeat(150), "k".repeat(10))
        );
    }
}
