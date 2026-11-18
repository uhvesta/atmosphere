//! Copilot's session-state conversations ([`crate::Format::CopilotSessionState`]).
//!
//! Each fact below is marked **X1** (seen with the real CLI, GitHub Copilot
//! CLI 1.0.112-0, in temp homes against a loopback model: the layout
//! fixture `copilot_layout.py`, `copilot --prompt`, then
//! `copilot --resume=<id> --prompt`) or **X2** (not yet proven; the adapter
//! refuses what it would need).
//!
//! # Where the files are
//!
//! ```text
//! <copilot>/session-state/<id>/events.jsonl                     the transcript
//! <copilot>/session-state/<id>/workspace.yaml                   the session's summary
//! <copilot>/session-state/<id>/checkpoints/index.md             the checkpoint list
//! <copilot>/session-state/<id>/rewind-file-snapshots/tracking.json
//! <copilot>/session-state/<id>/.workspace-fork.lock             empty
//! <copilot>/session-state/.session-operation-locks/<id>.lock    the CLI's, shared
//! <copilot>/session-store.db (-wal, -shm)                       the CLI's index
//! ```
//!
//! - `<copilot>` is [`Placement::cli_home`] (the CLI's `COPILOT_HOME`) when
//!   set, else `<home>/.copilot`. **X1**
//! - The id is a lowercase hyphenated UUID, and the folder is named for it.
//!   [`Adapter::locate`] and [`Adapter::fork`] take only that form, else
//!   [`ConversationError::BadId`]; `--resume=<id>` resumes by exact id and
//!   refuses an id it has no folder for. **X1**
//! - [`Adapter::locate`] lists every plain file under the id's folder:
//!   `events.jsonl` as [`crate::FileRole::Transcript`] first, the rest as
//!   [`crate::FileRole::Companion`] in path order. The shared lock folder
//!   and `session-store.db` are the CLI's and never listed. The empty
//!   `files/` and `research/` folders a run leaves are not files, and a
//!   resume makes them again. **X1** No folder or no transcript is
//!   [`ConversationError::NotFound`]; a link is
//!   [`ConversationError::NotAFile`]. Links are never followed.
//!
//! # The CLI's index (CV-4)
//!
//! `session-store.db` is SQLite: `sessions` (with `cwd` and `repository`),
//! `turns`, `checkpoints`, `session_files`, `session_refs` and others. **X1**
//! This adapter never reads, copies or writes it. A resume needs no row: a
//! fork written beside its parent, and a folder carried to another
//! `COPILOT_HOME`, both resume and the CLI adds the row. A folder rewritten
//! where it is (a move on one machine) resumes too, and the CLI sets its
//! row's `cwd` to the new folder itself. **X1**
//!
//! # The transcript
//!
//! One JSON object per line, each ending in `\n`; a last line without its
//! `\n` is being written, and every method ignores it. Every record has
//! `type`, `data` (an object), `id` (a UUID), `parentId` (a UUID, or null on
//! the first) and `timestamp` (RFC 3339, UTC). The `parentId`s form a tree,
//! not a chain: the usage record and the reply both name the turn's start.
//! **X1** The records X1 wrote, in order, for two turns in two processes:
//!
//! | `type` | when |
//! |---|---|
//! | `session.start` | first, once: `sessionId` (the id), `version` (1), `producer`, `copilotVersion`, `context` (`cwd`; in a git folder also `gitRoot` and `branch`) |
//! | `instruction.update` | harness instructions, before the first prompt |
//! | `user.message` | a prompt: `content` is what was typed |
//! | `system.message` | the system prompt, once, after the first prompt; its text names the folder |
//! | `assistant.turn_start` | the model is asked |
//! | `session.usage_record` | `usage.accounting.sourceSessionId` is the id |
//! | `assistant.message` | the reply |
//! | `assistant.turn_end` | the turn is done |
//! | `session.shutdown` | the process ends; `usageAccountingWatermarks` is keyed by the id |
//! | `session.resume` | a later process resumes: `context.cwd` |
//!
//! The id is in `sessionId`, `sourceSessionId`, the watermark key, and
//! `workspace.yaml`'s `id:`; the folder in `context.cwd`, `context.gitRoot`,
//! the system prompt's text and `workspace.yaml`'s `cwd:` and `git_root:`.
//! **X1**
//!
//! More records, seen by the round-3 fixture `copilot_roam2.py` (CLI
//! 1.0.112-0 for tools, `/compact` and git, and 1.0.113-1 for all of them
//! and an interactive process with `/rewind`; each fork and carry below
//! resumed with the right history): **X1**
//!
//! | `type` | when |
//! |---|---|
//! | `tool.execution_start`, `tool.execution_complete` | a tool call (`view`, `bash`, `task`); `arguments` and `result` can hold the folder |
//! | `subagent.started`, `.configured`, `.selected`, `.completed` | the `task` tool's agent; each also has a top-level `agentId` |
//! | `session.model_change` | a model is chosen (startup, or a subagent's) |
//! | `session.compaction_start`, `.compaction_complete`, `session.usage_checkpoint` | `/compact`; `summary_count` in `workspace.yaml` goes up |
//! | `session.info` | a message of the CLI's own, such as `infoType: rewind` |
//!
//! The round-3 captures ran signed out against a loopback model. Signed in,
//! every 1.0.113-1 session also writes the records below (seen in Humata's
//! own verification runs: fork, host, PR and repo lanes): **X1**
//!
//! | `type` | when |
//! |---|---|
//! | `instruction.update_delivery` | after `assistant.turn_start`: which `instruction.update`s went with the request; ids and counts only |
//! | `model.turn_started`, `.model_call_started`, `.captured_assignment_context`, `.model_call_success`, `.message`, `.response`, `.turn_ended`, `.messages_snapshot` | a side call the CLI makes to a small model of its own, inside a turn or after its `assistant.turn_end`; `modelInfo` holds endpoints that read as paths, and the request and reply can quote the prompt |
//! | `system.notification` | a background shell the agent started finished (repo lane); the reply it sets off follows in the same turn |
//!
//! None of them is a `user.message` or an `assistant.turn_end`, so they
//! change no turn or cut; a fork and a carry rewrite them like any other line.
//! The same runs add fields to records above (`reasoningEffort`,
//! `agentTaskId`, `providerId`, `reasoning*`, `tokenDetails`), and
//! `tool.execution_complete` names the files a tool wrote in `fileEdits`
//! (paths in the folder). A background shell's end is a `system.notification`
//! (VF-60, amending D746): `content` (the CLI's own note, quoting the
//! command's `description`), `kind` (`type`, `shellId`, `exitCode`,
//! `description`) and, from the schema, `responsesReasoning` (`model`,
//! `initialEffort`, `effort`). Only `kind.type` `shell_completed` (X1) and
//! `shell_detached_completed` (X2, no `exitCode`) are known; the schema's
//! other kinds stay unknown. No field is a path, but `description` and
//! `content` can quote the folder as any text can, and a fork and a carry
//! rewrite it there like any line. It is no turn: the reply it sets off
//! belongs to the turn above, so a cut before the next prompt keeps both,
//! and from now, while that reply runs, leaves out its turn with both. The
//! shell itself is not carried.
//!
//! Records of the owner's real sessions, known from the CLI's own
//! session-event schema (1.0.113-1, `copilot-sdk/generated/session-events.d.ts`)
//! and parsed by the same rules; a fork and a carry rewrite them like any
//! other line: **X2**
//!
//! | `type` | when |
//! |---|---|
//! | `hook.start`, `hook.end` | a configured hook runs; `input` and `output` can hold the folder |
//! | `permission.requested`, `permission.completed` | a permission prompt and its answer; `ownerSessionId` is the id |
//!
//! A tool call splits a turn: the model's reply asks for the tool
//! (`assistant.message` with `toolRequests`), the tool runs, an
//! `assistant.turn_end` comes, and a second `assistant.turn_start` carries the
//! reply that uses the result. A turn is finished only at an
//! `assistant.turn_end` that follows a reply without `toolRequests`. A subagent
//! has its own `user.message` (its `source` names the agent, and its turn
//! records have a `parentToolCallId`); it is not a prompt the person typed and
//! is no turn. `/compact` writes no `user.message` and adds
//! `checkpoints/<n>-*.md` and `session.db`, an SQLite file the CLI makes (only
//! empty tables and a schema row in every run; no id, path or prompt in it). An
//! interactive process writes one `user.message` per prompt with no
//! `session.shutdown` or `session.resume` between, and while it runs the folder
//! also holds `inuse.<pid>.lock` and `.hold`. `/rewind` rewrites `events.jsonl`
//! where it is, dropping the rewound prompt and all after it, and appends a
//! `session.info`.
//!
//! # The strict parse (CV-3, CV-7)
//!
//! A line that isn't a JSON object with a string `type` is
//! [`ConversationError::Malformed`]. It is
//! [`ConversationError::Unsupported`] when its `type` is not one above;
//! when a field the parse doesn't know, at the top or in `data`, holds a
//! path; when the first record is not `session.start`, or a second one
//! comes; and when `session.start` names a `version` other than 1 or a
//! `copilotVersion` older than 1.0.112, the first X1 proved. A newer
//! version whose records all parse is known (CV-7). `workspace.yaml` must
//! be `key: value` lines of the keys X1 saw (`id`, `cwd`, `git_root`,
//! `branch`, `client_name`, `last_client_name`, `name`, `user_named`,
//! `summary_count`, `fork_count`, `created_at`, `updated_at`, and signed in
//! `remote_steerable`, `mc_task_id`, `mc_session_id`, `mc_last_event_id` and
//! `mc_pending_repository_warning`); any other line is unsupported. A
//! multi-line `name` is a block scalar (`name: |-`, then lines indented by
//! two spaces, blank lines allowed); those lines are known.
//! `mc_pending_repository_warning` is a mapping: lines indented by two
//! spaces, each `mc_session_id`, `mc_task_id` or `message` with its value;
//! any other key or a deeper line is unsupported. Every other value is on its
//! line. **X1**
//!
//! # Turns
//!
//! Turn *n* is the *n*th `user.message`; its first line is the first line
//! of its `content` and its time the record's `timestamp`. It starts at
//! that record, or at the `session.shutdown` and `session.resume` right
//! before it when the prompt began a resumed process. It is finished when
//! an `assistant.turn_end` comes after it and before the next turn.
//! (Turns by `turnId` don't work: every process numbers its turns from
//! "0".) **X1**
//!
//! # Fork
//!
//! A fork is a new `session-state/<new id>/events.jsonl` under the target's
//! Copilot dir holding the parent's lines before the cut, as they were, with
//! every occurrence of the parent's id made the new one and the parent's
//! folder the fork's. The cut is turn `before`'s start; from now it is the
//! end when the last turn is finished, else the running last turn's start.
//! Before turn 1, or from now with no turn finished, is
//! [`ConversationError::NothingBefore`]; a turn the conversation hasn't got
//! is [`ConversationError::NoSuchTurn`]. A folder already there for the new
//! id is [`ConversationError::Exists`]. `copilot --resume=<new id>` takes
//! such a fork, sends only the kept turns, and leaves the parent as it was;
//! it makes `workspace.yaml` and the rest itself. **X1**
//!
//! # Carry
//!
//! [`Adapter::pack`] stages every listed file at its path under the Copilot
//! dir (`session-state/<id>/…`), the transcript's complete lines only.
//! [`Adapter::place`] writes them at the same paths under the target's
//! Copilot dir, with the old Copilot dir and the old folder (as it is and
//! as it reads inside a JSON string) made the new ones in every file, and
//! parses the result strictly before writing anything. Copied whole to
//! another `COPILOT_HOME` and folder this way, a session resumes with every
//! turn, and its next request names the new folder, not the old one; so
//! does one rewritten where it is. **X1** A conversation with tool calls, a
//! subagent, a `/compact` (its checkpoint file and `session.db` carried as
//! they are) or an interactive process carries the same way, and a fork cut
//! before or after a compaction resumes, the summary kept. **X1** A file that
//! is not text is carried only when it is an SQLite file that names neither
//! the old folder, the old Copilot dir nor the id (`session.db`); any other
//! is [`ConversationError::NotText`]. Onto another Copilot dir a file
//! already there is [`ConversationError::Exists`]; onto the same one the
//! files are rewritten in place. [`Adapter::place_on_head`] also names the
//! head in `session.start`'s `context.branch` (and `headCommit` where there)
//! and `workspace.yaml`'s `branch:` where they are there: the branch's name,
//! or `detached@` and the first seven characters of the commit for a detached
//! head, as the CLI writes them. A branch YAML would read as another type
//! (`true`, `null`, `123`, `1e3`) or that starts with an indicator (`@`, `{`,
//! `#`, `%`, `!`, `&`, `|`, `>`, `` ` ``) is single-quoted in `workspace.yaml`;
//! any other is plain, and a quoted one reads the same. **X1** The CLI does not read either back: on its next
//! resume it re-reads git, and sets `session.resume`'s `context` and
//! `workspace.yaml`'s `branch:` from the folder's own head, ignoring the
//! recorded ones. **X1**
//!
//! # Not proven
//!
//! Still **X2**, and the adapter refuses what it would need: records the
//! CLI writes only for features no run used (background tasks, workflows,
//! skills, MCP, plan mode), and any other
//! non-text file in a conversation folder.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::files::{
    complete_lines, holds_files, holds_path, io_error, is_there, json_escape, line_starts,
    list_into, malformed, path_text, read_plain, read_text, relative_name, rewrite_text, under,
    valid_rel, write_new,
};
use crate::{
    Adapter, Conversation, ConversationError, ConversationFile, FileRole, Format, GitHead, Packed,
    PackedFile, Placement, Turn,
};

/// The biggest file a carry rewrites. A bigger one is refused, never copied
/// as it is (CV-3).
const REWRITE_MAX: u64 = 256 << 20;

/// The folder under the Copilot dir that holds a folder per conversation.
const STATE: &str = "session-state";

/// The transcript in a conversation's folder.
const EVENTS: &str = "events.jsonl";

/// The session's summary in a conversation's folder.
const WORKSPACE: &str = "workspace.yaml";

/// The `session.start` `version` X1 wrote.
const SCHEMA: u64 = 1;

/// The first Copilot version X1 proved; an older one is unsupported.
const FIRST_KNOWN: [u64; 3] = [1, 0, 112];

/// The top-level fields of every record (X1); `agentId` is on a subagent's.
const FIELDS: &[&str] = &["type", "data", "id", "timestamp", "parentId", "agentId"];

/// The record types the strict parse knows (CV-7), each with the `data`
/// fields X1 saw. Add one only once the real CLI shows what it holds and
/// how a fork and a carry treat it.
const DATA_FIELDS: &[(&str, &[&str])] = &[
    (
        "session.start",
        &[
            "sessionId",
            "version",
            "producer",
            "copilotVersion",
            "startTime",
            "selectedModel",
            "reasoningEffort",
            "contextTier",
            "context",
            "alreadyInUse",
            "remoteSteerable",
        ],
    ),
    (
        "instruction.update",
        &["content", "authority", "lifetime", "boundary", "fallback", "name", "postInitial"],
    ),
    (
        "user.message",
        &[
            "content",
            "hasSubmittedTask",
            "transformedContent",
            "messageId",
            "supportedNativeDocumentMimeTypes",
            "delivery",
            "interactionId",
            "turnId",
            "parentAgentTaskId",
            "agentTaskId",
            "source",
            "causalDependencies",
        ],
    ),
    (
        "system.message",
        &["role", "content", "contentBlocks", "metadata", "interactionId"],
    ),
    (
        "assistant.turn_start",
        &["turnId", "interactionId", "agentMode", "parentToolCallId"],
    ),
    ("session.usage_record", &["usage"]),
    (
        "assistant.message",
        &[
            "messageId",
            "originatingMessageId",
            "model",
            "content",
            "toolRequests",
            "interactionId",
            "turnId",
            "rte",
            "apiCallId",
            "providerId",
            "reasoningText",
            "reasoningOpaque",
            "reasoningBlocks",
            "parentToolCallId",
        ],
    ),
    ("assistant.turn_end", &["turnId", "parentToolCallId"]),
    (
        "session.shutdown",
        &[
            "accountingSnapshot",
            "providerModelMetrics",
            "usageAccountingWatermarks",
            "shutdownType",
            "totalPremiumRequests",
            "totalNanoAiu",
            "totalApiDurationMs",
            "sessionStartTime",
            "eventsFileSizeBytes",
            "codeChanges",
            "modelMetrics",
            "agentMetrics",
            "currentModel",
            "currentTokens",
            "systemTokens",
            "conversationTokens",
            "toolDefinitionsTokens",
            "tokenDetails",
        ],
    ),
    (
        "session.resume",
        &[
            "resumeTime",
            "eventCount",
            "eventsFileSizeBytes",
            "selectedModel",
            "reasoningEffort",
            "contextTier",
            "sessionLimits",
            "context",
            "alreadyInUse",
            "remoteSteerable",
            "sessionWasActive",
        ],
    ),
    (
        "tool.execution_start",
        &[
            "toolCallId",
            "toolName",
            "arguments",
            "turnId",
            "model",
            "toolTitle",
            "shellToolInfo",
            "parentToolCallId",
        ],
    ),
    (
        "tool.execution_complete",
        &[
            "toolCallId",
            "model",
            "interactionId",
            "turnId",
            "rte",
            "shellExecution",
            "success",
            "error",
            "result",
            "toolTelemetry",
            // The files the tool wrote, each `path` and `kind` (1.0.113-1, a
            // signed-in run); the paths are in the folder and are rewritten
            // with it.
            "fileEdits",
            "causalDependencies",
            "parentToolCallId",
        ],
    ),
    (
        "subagent.started",
        &[
            "toolCallId",
            "agentName",
            "agentDisplayName",
            "agentDescription",
            "model",
            "taskModelSource",
            "modelSelectionSource",
            "resumable",
            "agentType",
            "executionMode",
        ],
    ),
    ("subagent.configured", &["model", "reasoningEffort", "multiTurn"]),
    ("subagent.selected", &["agentName", "agentDisplayName", "tools"]),
    (
        "subagent.completed",
        &[
            "toolCallId",
            "agentName",
            "agentDisplayName",
            "model",
            "firstDispatchedModel",
            "modelSelectionSource",
            "totalToolCalls",
            "totalTokens",
            "durationMs",
        ],
    ),
    (
        "session.model_change",
        &[
            "source",
            "contextTier",
            "newModel",
            "previousModel",
            "reasoningEffort",
            "previousReasoningEffort",
        ],
    ),
    (
        "session.compaction_start",
        &[
            "compactionAttemptId",
            "model",
            "systemTokens",
            "conversationTokens",
            "toolDefinitionsTokens",
            "currentTokens",
            "tokenLimit",
            "trigger",
            "execution",
        ],
    ),
    (
        "session.compaction_complete",
        &[
            "success",
            "outcome",
            "compactionAttemptId",
            "preCompactionMessagesLength",
            "messagesRemoved",
            "preCompactionTokens",
            "postCompactionTokens",
            "tokensRemoved",
            "tokenAccountingBasis",
            "summaryContent",
            "behaviorModelId",
            "compactionTokensUsed",
            "systemTokens",
            "conversationTokens",
            "toolDefinitionsTokens",
            "tokenLimit",
            "trigger",
            "execution",
            "clientRequestId",
            "attemptMetrics",
            "summarizerProgress",
            "checkpointNumber",
            "checkpointPath",
        ],
    ),
    (
        "session.usage_checkpoint",
        &[
            "totalNanoAiu",
            "totalPremiumRequests",
            "modelCacheState",
            "promptCacheBreakState",
            "accountingSnapshot",
            "providerModelMetrics",
            "usageAccountingWatermarks",
        ],
    ),
    ("session.info", &["infoType", "message"]),
    // A hook the person configured ran around a tool call or a prompt.
    // Fields from the CLI's own session-event schema (1.0.113-1); `input`
    // and `output` are what the hook got and gave, and may name the folder.
    ("hook.start", &["hookInvocationId", "hookType", "input", "parentToolCallId"]),
    (
        "hook.end",
        &["hookInvocationId", "hookType", "output", "error", "success", "parentToolCallId"],
    ),
    // A permission prompt and its answer. `permissionRequest` and
    // `promptRequest` name the command or path asked about;
    // `ownerSessionId` is the id, rewritten with every other occurrence.
    (
        "permission.requested",
        &[
            "agentMode",
            "assistedApprovalAssessment",
            "fusion",
            "ownerAgentId",
            "ownerSessionId",
            "permissionMode",
            "permissionRequest",
            "programToolCallId",
            "promptRequest",
            "recoveryEpisodeId",
            "requestId",
            "resolvedByHook",
            "riskAssessment",
        ],
    ),
    (
        "permission.completed",
        &["blocker", "decisionSource", "recoveryEpisodeId", "requestId", "result", "toolCallId"],
    ),
    // Which harness instructions (`instruction.update`) went with the next
    // model request. Ids, counts and outcome words; no prompt or path.
    (
        "instruction.update_delivery",
        &[
            "deliveryId",
            "firstUpdateEventId",
            "latestUpdateEventId",
            "updateCount",
            "requestGeneration",
            "outcome",
            "capability",
            "reason",
        ],
    ),
    // The CLI's own record of a model call: its lifecycle, the model's
    // description (`modelInfo`, whose `supported_endpoints` read as paths),
    // the request and reply (`requestMessages`, `message`, `response`,
    // `messages`, which can quote the prompt and the system prompt, and so
    // the folder), usage and quota. Seen on a side call the CLI makes in a
    // turn of a signed-in run, and after a turn ends; never a turn of its own.
    ("model.turn_started", &["kind", "model", "modelInfo", "turn", "timestampMs"]),
    ("model.model_call_started", &["kind", "model", "modelInfo", "turn", "timestampMs"]),
    ("model.captured_assignment_context", &["kind", "assignmentContext"]),
    (
        "model.model_call_success",
        &[
            "kind",
            "turn",
            "callId",
            "modelCallDurationMs",
            "ttftMs",
            "outputTtftMs",
            "interTokenLatencyMs",
            "modelCall",
            "responseChunk",
            "responseUsage",
            "requestMessages",
            "requestSettings",
            "quotaSnapshots",
            "requestId",
            "clientRequestId",
            "serviceRequestId",
            "rte",
            "copilotUsage",
            "reasoningEffort",
            "reasoningSummary",
            "maxOutputTokens",
            "toolCount",
            "deferredToolCount",
            "visibleToolCount",
            "provider",
            "customToolCallCount",
            "unmatchedToolResultCount",
            "orphanToolCallCount",
            "modelDisplayName",
        ],
    ),
    ("model.message", &["kind", "turn", "callId", "modelCall", "message"]),
    ("model.response", &["kind", "turn", "callId", "modelCall", "response"]),
    ("model.turn_ended", &["kind", "model", "modelInfo", "turn", "timestampMs"]),
    ("model.messages_snapshot", &["kind", "messages"]),
    // A background shell finished; its `kind` and `responsesReasoning` are
    // checked by `NOTIFICATION_PARTS` and `NOTIFICATION_KINDS`.
    ("system.notification", &["content", "kind", "responsesReasoning"]),
];

/// The objects in a `system.notification`'s `data`, each with the fields a
/// signed-in 1.0.113-1 run wrote or its event schema names. As at the top,
/// a field not listed that holds a path makes the record unknown.
const NOTIFICATION_PARTS: &[(&str, &[&str])] = &[
    ("kind", &["type", "shellId", "exitCode", "description"]),
    ("responsesReasoning", &["model", "initialEffort", "effort"]),
];

/// The `kind.type`s of a `system.notification` that are known:
/// `shell_completed` (X1) and `shell_detached_completed`, the same without
/// `exitCode` (X2). The schema's others (`agent_completed`, `agent_idle`,
/// `new_inbox_message`, `instruction_discovered`) are not.
const NOTIFICATION_KINDS: &[&str] = &["shell_completed", "shell_detached_completed"];

/// The keys of `workspace.yaml` X1 saw; the `mc_*` ones and
/// `remote_steerable` in a signed-in 1.0.113-1 run.
const WORKSPACE_KEYS: &[&str] = &[
    "id",
    "cwd",
    "git_root",
    "branch",
    "client_name",
    "last_client_name",
    "name",
    "user_named",
    "summary_count",
    "fork_count",
    "created_at",
    "updated_at",
    "remote_steerable",
    "mc_task_id",
    "mc_session_id",
    "mc_last_event_id",
    "mc_pending_repository_warning",
];

/// The keys of `workspace.yaml` that hold a mapping, each with the keys of
/// its lines (indented by two spaces, one value each) a signed-in 1.0.113-1
/// run wrote. `mc_session_id` is the id, rewritten with every other
/// occurrence.
const WORKSPACE_MAPPINGS: &[(&str, &[&str])] =
    &[("mc_pending_repository_warning", &["mc_session_id", "mc_task_id", "message"])];

/// The adapter for [`Format::CopilotSessionState`].
pub(crate) struct CopilotSessionState;

impl Adapter for CopilotSessionState {
    /// The files of `session-state/<id>/` under the Copilot dir of `at`,
    /// the transcript first. The id must be a lowercase hyphenated UUID.
    fn locate(&self, at: &Placement, id: &str) -> Result<Conversation, ConversationError> {
        check_id(id)?;
        let folder = copilot_dir(at).join(STATE).join(id);
        let meta = match fs::symlink_metadata(&folder) {
            Ok(meta) => meta,
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                return Err(ConversationError::NotFound { path: folder });
            }
            Err(source) => return Err(io_error("find the conversation's folder", &folder, source)),
        };
        if !meta.file_type().is_dir() {
            return Err(ConversationError::NotAFile { path: folder });
        }
        let transcript = folder.join(EVENTS);
        if !is_there(&transcript)? {
            return Err(ConversationError::NotFound { path: transcript });
        }
        let meta = fs::symlink_metadata(&transcript)
            .map_err(|source| io_error("read the transcript", &transcript, source))?;
        if !meta.file_type().is_file() {
            return Err(ConversationError::NotAFile { path: transcript });
        }
        let mut found = Vec::new();
        list_into(&folder, &mut found)?;
        found.retain(|path| *path != transcript);
        found.sort();
        let files = std::iter::once(ConversationFile {
            path: transcript,
            role: FileRole::Transcript,
        })
        .chain(found.into_iter().map(|path| ConversationFile {
            path,
            role: FileRole::Companion,
        }))
        .collect();
        Ok(Conversation {
            id: id.to_owned(),
            format: Format::CopilotSessionState,
            placement: at.clone(),
            files,
        })
    }

    /// The prompts of the transcript's turns, in order (module doc, Turns).
    fn turns(&self, conversation: &Conversation) -> Result<Vec<Turn>, ConversationError> {
        let path = transcript_of(conversation)?;
        let bytes = read_plain(&path, u64::MAX)?;
        let lines = parse(&bytes, &path, &conversation.placement)?;
        Ok(prompts_of(&lines).into_iter().map(|prompt| prompt.turn).collect())
    }

    /// Where each turn starts: its prompt's line, or the resume or shutdown
    /// records just before it, the line a fork before it is cut at (module
    /// doc, Fork).
    fn turn_starts(&self, conversation: &Conversation) -> Result<Vec<u64>, ConversationError> {
        let path = transcript_of(conversation)?;
        let bytes = read_plain(&path, u64::MAX)?;
        let lines = parse(&bytes, &path, &conversation.placement)?;
        let numbers = prompts_of(&lines).into_iter().map(|prompt| prompt.start);
        Ok(line_starts(&bytes, numbers))
    }

    /// The transcript's lines before the cut, renamed for `new_id` and
    /// `to`'s folder, as a new conversation folder (module doc, Fork).
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
        let lines = parse(&bytes, &path, &conversation.placement)?;
        let cut = cut_for(&prompts_of(&lines), before)?;
        let mut kept = Vec::with_capacity(bytes.len());
        for (number, line) in (1_u64..).zip(complete_lines(&bytes)) {
            if number >= cut {
                break;
            }
            kept.extend_from_slice(line);
            kept.push(b'\n');
        }
        let kept = String::from_utf8(kept).map_err(|_| ConversationError::NotText { path: path.clone() })?;
        let mut pairs = rewrite_pairs(&conversation.placement, to);
        push_pair(&mut pairs, conversation.id.clone(), new_id.to_owned());
        let text = rewrite_text(&kept, &pairs);
        let folder = copilot_dir(to).join(STATE).join(new_id);
        if is_there(&folder)? {
            return Err(ConversationError::Exists { path: folder });
        }
        let target = folder.join(EVENTS);
        parse(text.as_bytes(), &target, to)?;
        write_new(&target, text.as_bytes())?;
        Ok(Conversation {
            id: new_id.to_owned(),
            format: Format::CopilotSessionState,
            placement: to.clone(),
            files: vec![ConversationFile {
                path: target,
                role: FileRole::Transcript,
            }],
        })
    }

    /// Every file of the conversation's folder, unchanged (the transcript's
    /// complete lines), under `stage` at its path under the Copilot dir
    /// (module doc, Carry).
    fn pack(&self, conversation: &Conversation, stage: &Path) -> Result<Packed, ConversationError> {
        let transcript = transcript_of(conversation)?;
        let base = copilot_dir(&conversation.placement);
        let mut staged = Vec::with_capacity(conversation.files.len());
        for file in &conversation.files {
            if file.path != transcript {
                if let Some(bytes) = sqlite_bytes(&file.path, &conversation.placement, &conversation.id)? {
                    staged.push((relative_name(&file.path, &base)?, file.role, bytes));
                    continue;
                }
            }
            let text = read_text(&file.path, REWRITE_MAX)?;
            let rel = relative_name(&file.path, &base)?;
            let body = if file.path == transcript {
                parse(text.as_bytes(), &file.path, &conversation.placement)?;
                let mut body = Vec::with_capacity(text.len());
                for line in complete_lines(text.as_bytes()) {
                    body.extend_from_slice(line);
                    body.push(b'\n');
                }
                body
            } else {
                if is_workspace(&rel, &conversation.id) {
                    parse_workspace(&text)?;
                }
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
            write_new(&under(stage, &rel), &body)?;
            files.push(PackedFile {
                size: u64::try_from(body.len()).unwrap_or(u64::MAX),
                rel,
                role,
            });
        }
        Ok(Packed {
            id: conversation.id.clone(),
            format: Format::CopilotSessionState,
            stage: stage.to_path_buf(),
            files,
        })
    }

    /// The staged files at the same paths under `to`'s Copilot dir, the
    /// Copilot dir and folder rewritten (module doc, Carry). Never writes
    /// the CLI's index.
    fn place(&self, packed: &Packed, from: &Placement, to: &Placement) -> Result<Conversation, ConversationError> {
        self.place_on_head(packed, from, to, None)
    }

    /// As [`Adapter::place`], and with `head`'s branch named where the
    /// session recorded one. On one Copilot dir the files are rewritten
    /// where they are.
    fn place_on_head(
        &self,
        packed: &Packed,
        from: &Placement,
        to: &Placement,
        head: Option<&GitHead>,
    ) -> Result<Conversation, ConversationError> {
        ensure_copilot(packed.format)?;
        check_id(&packed.id)?;
        let own = format!("{STATE}/{}/", packed.id);
        if let Some(file) = packed
            .files
            .iter()
            .find(|file| !valid_rel(&file.rel) || !file.rel.starts_with(own.as_str()))
        {
            return Err(ConversationError::BadPath {
                rel: file.rel.clone(),
            });
        }
        let transcript = format!("{own}{EVENTS}");
        let has_transcript = packed
            .files
            .iter()
            .any(|file| file.role == FileRole::Transcript && file.rel == transcript);
        if !has_transcript {
            return Err(ConversationError::NotFound {
                path: under(&packed.stage, &transcript),
            });
        }
        let pairs = rewrite_pairs(from, to);
        let to_dir = copilot_dir(to);
        let in_place = copilot_dir(from) == to_dir;
        let mut writes = Vec::with_capacity(packed.files.len());
        for file in &packed.files {
            if file.rel != transcript {
                if let Some(bytes) = sqlite_bytes(&under(&packed.stage, &file.rel), from, &packed.id)? {
                    let target = under(&to_dir, &file.rel);
                    if !in_place && is_there(&target)? {
                        return Err(ConversationError::Exists { path: target });
                    }
                    writes.push((target, file.role, bytes));
                    continue;
                }
            }
            let text = read_text(&under(&packed.stage, &file.rel), REWRITE_MAX)?;
            let mut rewritten = rewrite_text(&text, &pairs);
            let target = under(&to_dir, &file.rel);
            if !in_place && is_there(&target)? {
                return Err(ConversationError::Exists { path: target });
            }
            if file.rel == transcript {
                if let Some(head) = head {
                    rewritten = events_on_head(&rewritten, head);
                }
                parse(rewritten.as_bytes(), &target, to)?;
            } else if is_workspace(&file.rel, &packed.id) {
                if let Some(head) = head {
                    rewritten = workspace_on_head(&rewritten, head);
                }
                parse_workspace(&rewritten)?;
            }
            writes.push((target, file.role, rewritten.into_bytes()));
        }
        for (target, _, bytes) in &writes {
            write_new(target, bytes)?;
        }
        Ok(Conversation {
            id: packed.id.clone(),
            format: Format::CopilotSessionState,
            placement: to.clone(),
            files: writes
                .into_iter()
                .map(|(path, role, _)| ConversationFile { path, role })
                .collect(),
        })
    }

    /// The strict parse of the transcript and `workspace.yaml` (module doc,
    /// The strict parse), and `cli_version`, the launching CLI's own
    /// `--version` text or its number, no older than the first proven.
    fn check(&self, conversation: &Conversation, cli_version: Option<&str>) -> Result<(), ConversationError> {
        let path = transcript_of(conversation)?;
        let older = cli_version.filter(|text| !version_number(text).is_some_and(|version| version >= FIRST_KNOWN));
        if let Some(text) = older {
            return Err(unsupported(Some(text.lines().next().unwrap_or(text).trim())));
        }
        let bytes = read_plain(&path, u64::MAX)?;
        parse(&bytes, &path, &conversation.placement)?;
        let workspace = copilot_dir(&conversation.placement)
            .join(STATE)
            .join(&conversation.id)
            .join(WORKSPACE);
        if let Some(file) = conversation.files.iter().find(|file| file.path == workspace) {
            parse_workspace(&read_text(&file.path, REWRITE_MAX)?)?;
        }
        Ok(())
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

/// The Copilot dir of `at`: its `COPILOT_HOME`, else `<home>/.copilot` (X1).
fn copilot_dir(at: &Placement) -> PathBuf {
    at.cli_home.clone().unwrap_or_else(|| at.home.join(".copilot"))
}

fn ensure_copilot(format: Format) -> Result<(), ConversationError> {
    match format {
        Format::CopilotSessionState => Ok(()),
        other => Err(ConversationError::Unsupported {
            format: other,
            version: None,
        }),
    }
}

/// The refusal for what the strict parse doesn't know, naming `version`
/// when it is known.
fn unsupported(version: Option<&str>) -> ConversationError {
    ConversationError::Unsupported {
        format: Format::CopilotSessionState,
        version: version.map(str::to_owned),
    }
}

/// The transcript file of `conversation`, which a method of this adapter
/// reads.
fn transcript_of(conversation: &Conversation) -> Result<PathBuf, ConversationError> {
    ensure_copilot(conversation.format)?;
    conversation
        .files
        .iter()
        .find(|file| file.role == FileRole::Transcript)
        .map(|file| file.path.clone())
        .ok_or_else(|| ConversationError::NotFound {
            path: copilot_dir(&conversation.placement)
                .join(STATE)
                .join(&conversation.id)
                .join(EVENTS),
        })
}

/// Whether `rel` (under the Copilot dir) is conversation `id`'s
/// `workspace.yaml`.
fn is_workspace(rel: &str, id: &str) -> bool {
    rel == format!("{STATE}/{id}/{WORKSPACE}")
}

/// The bytes of the SQLite file at `path` when it names neither the folder,
/// the Copilot dir of `at` nor the conversation's `id` (`session.db`, module
/// doc, Carry); `None` for any other file, which is text or refused.
fn sqlite_bytes(path: &Path, at: &Placement, id: &str) -> Result<Option<Vec<u8>>, ConversationError> {
    let bytes = read_plain(path, REWRITE_MAX)?;
    if !bytes.starts_with(b"SQLite format 3\0") {
        return Ok(None);
    }
    let names = [path_text(&at.folder), path_text(&copilot_dir(at)), id.to_owned()];
    let named = names
        .iter()
        .filter(|name| !name.is_empty())
        .any(|name| bytes.windows(name.len()).any(|window| window == name.as_bytes()));
    if named {
        Err(ConversationError::NotText { path: path.to_path_buf() })
    } else {
        Ok(Some(bytes))
    }
}

/// The strict parse of every complete line (module doc, The strict parse).
fn parse(bytes: &[u8], path: &Path, at: &Placement) -> Result<Vec<(u64, Value)>, ConversationError> {
    let folder = path_text(&at.folder);
    let mut lines: Vec<(u64, Value)> = Vec::new();
    for (number, raw) in (1_u64..).zip(complete_lines(bytes)) {
        let value: Value = serde_json::from_slice(raw).map_err(|source| ConversationError::Malformed {
            path: path.to_path_buf(),
            line: number,
            source: Some(source),
        })?;
        let Some(kind) = value.get("type").and_then(Value::as_str) else {
            return Err(malformed(path, number));
        };
        let starts = kind == "session.start";
        if starts != lines.is_empty() {
            return Err(unsupported(None));
        }
        if !is_known(&value, kind, &folder) {
            return Err(unsupported(lines.first().and_then(|(_, first)| copilot_version(first))));
        }
        if starts {
            known_start(&value)?;
        }
        lines.push((number, value));
    }
    Ok(lines)
}

/// Whether the strict parse knows a record: its type, and that no field it
/// doesn't know, at the top or in `data`, holds a path.
fn is_known(value: &Value, kind: &str, folder: &str) -> bool {
    let Some(known) = DATA_FIELDS
        .iter()
        .find(|(name, _)| *name == kind)
        .map(|(_, fields)| *fields)
    else {
        return false;
    };
    let Some(data) = value.get("data").and_then(Value::as_object) else {
        return false;
    };
    let data_known = data
        .iter()
        .all(|(field, held)| known.contains(&field.as_str()) || !holds_path(held, folder));
    let top_known = value.as_object().is_some_and(|record| {
        record
            .iter()
            .all(|(field, held)| FIELDS.contains(&field.as_str()) || !holds_path(held, folder))
    });
    let parts_known = kind != "system.notification" || is_known_notification(data, folder);
    data_known && top_known && parts_known
}

/// Whether a `system.notification`'s `data` is one the parse knows: a
/// `kind` of a known type, and no field it doesn't know in `kind` or
/// `responsesReasoning` that holds a path.
fn is_known_notification(data: &serde_json::Map<String, Value>, folder: &str) -> bool {
    let kind_known = data
        .get("kind")
        .and_then(|kind| kind.get("type"))
        .and_then(Value::as_str)
        .is_some_and(|kind| NOTIFICATION_KINDS.contains(&kind));
    let parts_known = NOTIFICATION_PARTS.iter().all(|(part, known)| match data.get(*part) {
        None => *part != "kind",
        Some(Value::Object(fields)) => fields
            .iter()
            .all(|(field, held)| known.contains(&field.as_str()) || !holds_path(held, folder)),
        Some(_) => false,
    });
    kind_known && parts_known
}

/// Refuses a `session.start` of a schema or a Copilot version X1 didn't
/// prove.
fn known_start(value: &Value) -> Result<(), ConversationError> {
    let version = copilot_version(value);
    let schema = value.pointer("/data/version").and_then(Value::as_u64);
    let proven = version
        .and_then(version_number)
        .is_some_and(|number| number >= FIRST_KNOWN);
    if schema == Some(SCHEMA) && proven {
        Ok(())
    } else {
        Err(unsupported(version))
    }
}

/// The `copilotVersion` a `session.start` names.
fn copilot_version(value: &Value) -> Option<&str> {
    value.pointer("/data/copilotVersion").and_then(Value::as_str)
}

/// The first `major.minor.patch` in `text`: `1.0.112-0` and
/// `GitHub Copilot CLI 1.0.112-0.` are both 1.0.112.
fn version_number(text: &str) -> Option<[u64; 3]> {
    text.split_whitespace().find_map(|word| {
        let digits: String = word
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        let mut parts = digits.trim_end_matches('.').split('.').map(|part| part.parse::<u64>().ok());
        let number = [parts.next()??, parts.next()??, parts.next()??];
        parts.next().is_none().then_some(number)
    })
}

/// Refuses a `workspace.yaml` that isn't `key: value` lines of the keys X1
/// saw.
fn parse_workspace(text: &str) -> Result<(), ConversationError> {
    let mut block = false;
    let mut mapping: Option<&[&str]> = None;
    for line in text.lines() {
        if block && (line.is_empty() || line.starts_with(' ')) {
            continue;
        }
        if let Some(keys) = mapping.filter(|_| line.starts_with(' ')) {
            let known = line
                .strip_prefix("  ")
                .filter(|inner| !inner.starts_with(' '))
                .and_then(|inner| inner.split_once(':'))
                .is_some_and(|(key, rest)| keys.contains(&key) && rest.starts_with(' '));
            if known {
                continue;
            }
            return Err(unsupported(None));
        }
        let known = line.split_once(':').filter(|(key, rest)| {
            WORKSPACE_KEYS.contains(key) && (rest.is_empty() || rest.starts_with(' '))
        });
        let Some((key, rest)) = known else {
            return Err(unsupported(None));
        };
        block = matches!(rest.trim(), "|" | "|-" | "|+" | ">" | ">-" | ">+");
        mapping = rest
            .is_empty()
            .then(|| WORKSPACE_MAPPINGS.iter().find(|(name, _)| *name == key).map(|(_, keys)| *keys))
            .flatten();
    }
    Ok(())
}

/// A turn of the transcript and where it starts (module doc, Turns).
struct Prompt {
    turn: Turn,
    /// The number of its first line.
    start: u64,
    /// Whether an `assistant.turn_end` follows it before the next turn.
    finished: bool,
}

fn kind_of(value: &Value) -> Option<&str> {
    value.get("type").and_then(Value::as_str)
}

/// Whether `value` is a prompt the person typed: a `user.message` no
/// subagent sent (a subagent's names the agent in `source`).
fn is_typed(value: &Value) -> bool {
    kind_of(value) == Some("user.message") && value.pointer("/data/source").and_then(Value::as_str).is_none()
}

/// Whether the main agent's last `assistant.turn_end` in `span` follows a
/// reply that asked for no tool. A turn that called a tool has an end after
/// the tool ran and a second reply after that; a subagent's records (they
/// have a `parentToolCallId`) are not the main agent's.
fn ends_in_answer(span: &[(u64, Value)]) -> bool {
    let mut answered = true;
    span.iter().any(|(_, value)| {
        if value.pointer("/data/parentToolCallId").is_some() {
            return false;
        }
        match kind_of(value) {
            Some("assistant.message") => {
                answered = value
                    .pointer("/data/toolRequests")
                    .and_then(Value::as_array)
                    .is_none_or(Vec::is_empty);
                false
            }
            Some("assistant.turn_end") => answered,
            _ => false,
        }
    })
}

/// The turns of the transcript, in order: one per `user.message`.
fn prompts_of(lines: &[(u64, Value)]) -> Vec<Prompt> {
    let prompts: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, (_, value))| is_typed(value))
        .map(|(position, _)| position)
        .collect();
    let mut found = Vec::with_capacity(prompts.len());
    for (n, &position) in prompts.iter().enumerate() {
        let mut first = position;
        while first > 0
            && matches!(
                kind_of(&lines[first - 1].1),
                Some("session.resume" | "session.shutdown")
            )
        {
            first -= 1;
        }
        let end = prompts.get(n + 1).copied().unwrap_or(lines.len());
        let finished = ends_in_answer(&lines[position..end]);
        let value = &lines[position].1;
        let first_line = value
            .pointer("/data/content")
            .and_then(Value::as_str)
            .and_then(|text| text.lines().next())
            .unwrap_or_default()
            .to_owned();
        found.push(Prompt {
            turn: Turn {
                index: u32::try_from(n + 1).unwrap_or(u32::MAX),
                first_line,
                at: value.get("timestamp").and_then(Value::as_str).map(str::to_owned),
            },
            start: lines[first].0,
            finished,
        });
    }
    found
}

/// The line a fork keeps lines before (module doc, Fork): turn `before`'s
/// start, the end when the last turn is finished (`None`, from now), else
/// the running last turn's start.
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
            Some(_) if !prompts.iter().any(|prompt| prompt.finished) => Err(ConversationError::NothingBefore),
            Some(last) => Ok(last.start),
        },
    }
}

/// The branch the CLI records for `head`: its name, or `detached@` and the
/// first seven characters of the commit.
fn recorded_branch(head: &GitHead) -> String {
    head.branch
        .clone()
        .unwrap_or_else(|| format!("detached@{}", head.sha.chars().take(7).collect::<String>()))
}

/// `text` with `session.start`'s `context.branch` and `context.headCommit`,
/// where it has them, set to `head`; every other line as it was.
fn events_on_head(text: &str, head: &GitHead) -> String {
    let branch = recorded_branch(head);
    let mut out = String::with_capacity(text.len());
    for piece in text.split_inclusive('\n') {
        let rewritten = piece
            .strip_suffix('\n')
            .filter(|line| line.contains("\"session.start\""))
            .and_then(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|record| kind_of(record) == Some("session.start"))
            .and_then(|mut record| {
                let context = record.pointer_mut("/data/context")?.as_object_mut()?;
                let held = context.get_mut("branch")?;
                *held = Value::from(branch.as_str());
                if let Some(commit) = context.get_mut("headCommit") {
                    *commit = Value::from(head.sha.as_str());
                }
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

/// `branch` as `workspace.yaml` holds it after `branch:`: plain when YAML
/// reads it back as the same text, else single-quoted. `None` for a branch
/// with a control character, which is left as it was.
fn yaml_branch(branch: &str) -> Option<String> {
    if branch.is_empty() || branch.chars().any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}')) {
        return None;
    }
    let indicator = branch.starts_with(|c: char| {
        matches!(c, '@' | '#' | '{' | '}' | '[' | ']' | '%' | '!' | '&' | '*' | '|' | '>' | '\'' | '"' | '`' | ',' | '?' | ':' | '-' | '~' | '=')
    });
    let number_like = branch.starts_with(|c: char| c.is_ascii_digit() || matches!(c, '+' | '.'))
        && branch.chars().all(|c| c.is_ascii_hexdigit() || matches!(c, '.' | '_' | '+' | '-' | 'x' | 'X' | 'o' | 'O'));
    let word = matches!(
        branch.to_ascii_lowercase().as_str(),
        "true" | "false" | "null" | "yes" | "no" | "on" | "off" | "y" | "n"
    );
    if indicator || number_like || word {
        Some(format!("'{}'", branch.replace('\'', "''")))
    } else {
        Some(branch.to_owned())
    }
}

/// `text` with its `branch:` line, where it has one, naming `head` as the CLI
/// writes it (module doc, Carry).
fn workspace_on_head(text: &str, head: &GitHead) -> String {
    let Some(branch) = yaml_branch(&recorded_branch(head)) else {
        return text.to_owned();
    };
    text.split_inclusive('\n')
        .map(|piece| {
            let line = piece.strip_suffix('\n').unwrap_or(piece);
            if line.starts_with("branch:") {
                let end = if piece.ends_with('\n') { "\n" } else { "" };
                format!("branch: {branch}{end}")
            } else {
                piece.to_owned()
            }
        })
        .collect()
}

/// The text a carry or fork replaces, in order: the old Copilot dir, then
/// the old folder as it is and as it reads inside a JSON string.
fn rewrite_pairs(from: &Placement, to: &Placement) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    push_pair(&mut pairs, path_text(&copilot_dir(from)), path_text(&copilot_dir(to)));
    let (old, new) = (path_text(&from.folder), path_text(&to.folder));
    let (old_escaped, new_escaped) = (json_escape(&old), json_escape(&new));
    let escaped_differs = (old_escaped.as_str(), new_escaped.as_str()) != (old.as_str(), new.as_str());
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
