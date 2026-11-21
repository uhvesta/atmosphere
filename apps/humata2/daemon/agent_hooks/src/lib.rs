//! What each agent's hooks send Humata and read back (HOOKS, D191–D193).
//!
//! - [`parse_permission`] reads a permission hook's stdin payload into the
//!   [`Ask`] the Inbox card shows. Anything it can't read whole is an error,
//!   and an error means "no decision": the agent's TUI asks.
//! - [`render_permission`] writes a [`Decision`] the way that agent reads it
//!   on the hook's stdout. [`Decision::Undecided`] is empty output for all.
//! - [`claude_permission_hook`], [`codex_hooks`] and [`copilot_plugin`] make
//!   the per-session hook config humatad writes into its state folder.
//! - [`claude_mcp_config`], [`copilot_mcp_config`] and [`codex_mcp`] tell
//!   each CLI of Humata's own MCP server for the session (D748).
//! - [`parse_question`] and [`render_question`] do the same for an agent's
//!   own ask-user form, which becomes an Inbox question (D761).
//! - [`parse_conversation`] reads the conversation a Copilot hook says its
//!   CLI is in, so a `/clear` or `/restart` that starts another one in the
//!   same running agent is followed, and a resume takes that one up;
//!   [`parse_transcript`] reads the file Claude's hook says it is kept in.
//! - OpenCode has no hooks: the `humata opencode` wrapper hands each
//!   `permission.asked` event's `data` over as the payload, and posts what
//!   [`render_permission`] writes as the body of OpenCode's reply (D201a).
//!
//! Every hook command ends `|| true` (or prints `{}` for Codex's Stop), so a
//! missing daemon, a missing binary or an older CLI never fails a tool call:
//! Codex and Copilot read exit 2 as a deny.

use std::path::Path;
use std::time::Duration;

mod mcp;
pub use mcp::{MCP_SERVER, agy_mcp_config, claude_mcp_config, codex_mcp, copilot_mcp_config};
mod question;
pub use question::{Question, Reply, parse_question, render_question};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// The timeout written into every agent's permission hook, in seconds (D192).
pub const HOLD_SECONDS: u64 = 3600;
/// When the daemon ends a hold with no decision, so the agent never kills it.
pub const DAEMON_HOLD: Duration = Duration::from_secs(HOLD_SECONDS - 10);
/// The CLI's own cap, above the daemon's, so the daemon always ends first.
pub const CLI_HOLD: Duration = Duration::from_secs(HOLD_SECONDS - 5);
/// The largest payload read; a bigger one raises no card.
pub const PAYLOAD_MAX: usize = 1024 * 1024;

/// An agent whose hooks Humata speaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Agent {
    Claude,
    Codex,
    Copilot,
    OpenCode,
}

impl Agent {
    /// The agent `name` names in a hook command, as [`Agent::name`] wrote it.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            "copilot" => Some(Self::Copilot),
            "opencode" => Some(Self::OpenCode),
            _ => None,
        }
    }

    /// Its name in hook commands: `claude`, `codex`, `copilot`, `opencode`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Copilot => "copilot",
            Self::OpenCode => "opencode",
        }
    }
}

/// One permission ask, as the Inbox card shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ask {
    /// The tool, as the agent names it: `Bash`, `apply_patch`, `mcp__…`.
    pub tool: String,
    /// The whole of what approving runs: the shell command, the patch, or
    /// for any other tool all of its input. Never cut.
    pub command: String,
    /// What the agent says the command is for, if it says.
    pub description: Option<String>,
    /// The tool changes files (an edit, a write, a patch) and doesn't run a
    /// command, so the card says it edits files.
    pub edits_files: bool,
    /// The same for the same ask again, and different for another one: the
    /// Inbox's idempotency key within a session.
    pub key: String,
}

/// How the user answered, or that no one did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    Allow,
    /// `message` is the user's note to the agent; empty if they gave none.
    Deny { message: String },
    /// No answer in time, or the ask couldn't be shown: the TUI asks.
    Undecided,
}

/// Why a payload raised no card.
#[derive(Debug, thiserror::Error)]
pub enum PayloadError {
    #[error("the hook payload is {size} bytes, over the {PAYLOAD_MAX} read")]
    TooBig { size: usize },
    #[error("the hook payload isn't JSON")]
    NotJson(#[source] serde_json::Error),
    #[error("the hook payload isn't a {agent:?} permission ask: {why}")]
    NotAnAsk { agent: Agent, why: &'static str },
    #[error("the hook payload isn't a {agent:?} question for the user: {why}")]
    NotAQuestion { agent: Agent, why: &'static str },
    #[error("Copilot asks to bypass its sandbox, which an approval from the Inbox doesn't cover")]
    SandboxBypass,
    #[error("the hook payload doesn't name a {agent:?} conversation: {why}")]
    NotAConversation { agent: Agent, why: &'static str },
}

/// The message a deny sends when the user gave no note.
const DENY_WITHOUT_NOTE: &str = "The user denied this in Humata's Inbox.";

/// Reads `agent`'s permission hook payload.
pub fn parse_permission(agent: Agent, payload: &[u8]) -> Result<Ask, PayloadError> {
    if payload.len() > PAYLOAD_MAX {
        return Err(PayloadError::TooBig {
            size: payload.len(),
        });
    }
    let value: Value = serde_json::from_slice(payload).map_err(PayloadError::NotJson)?;
    let not_an_ask = |why: &'static str| PayloadError::NotAnAsk { agent, why };
    if !value.is_object() {
        return Err(not_an_ask("the payload isn't a JSON object"));
    }

    let (tool, input, turn) = match agent {
        Agent::Claude | Agent::Codex => {
            if str_field(&value, "hook_event_name") != Some("PermissionRequest") {
                return Err(not_an_ask("hook_event_name isn't PermissionRequest"));
            }
            let tool = str_field(&value, "tool_name")
                .ok_or_else(|| not_an_ask("there is no tool_name"))?;
            let input = object_field(&value, "tool_input")
                .ok_or_else(|| not_an_ask("tool_input isn't an object"))?;
            let turn = if agent == Agent::Codex {
                str_field(&value, "turn_id").unwrap_or("")
            } else {
                ""
            };
            (tool, input, turn)
        }
        Agent::Copilot => {
            if value.get("tool_name").is_some() {
                return Err(not_an_ask("it has a snake_case tool_name"));
            }
            match value.get("hook_event_name") {
                None => {}
                Some(event) if event.as_str() == Some("permissionRequest") => {}
                Some(_) => return Err(not_an_ask("hook_event_name isn't permissionRequest")),
            }
            let tool = str_field(&value, "toolName")
                .ok_or_else(|| not_an_ask("there is no toolName"))?;
            let input = object_field(&value, "toolInput")
                .ok_or_else(|| not_an_ask("toolInput isn't an object"))?;
            if input.get("requestSandboxBypass") == Some(&Value::Bool(true)) {
                return Err(PayloadError::SandboxBypass);
            }
            (tool, input, "")
        }
        Agent::OpenCode => return opencode_ask(&value).ok_or_else(|| {
            not_an_ask("it has no string id and action, or resources isn't a list of strings")
        }),
    };

    let input_json = input.to_string();
    Ok(Ask {
        tool: tool.to_owned(),
        command: command_of(tool, input),
        description: str_field(input, "description").map(str::to_owned),
        edits_files: edits_files(tool),
        key: ask_key(&[agent.name(), tool, &input_json, turn]),
    })
}

/// The `data` of OpenCode's `permission.asked`: `action` is the tool, its
/// `resources` are what it runs, and its `id` is the ask, so two asks of the
/// same command are two cards and a reopened hold finds the same one.
fn opencode_ask(value: &Value) -> Option<Ask> {
    let id = str_field(value, "id")?;
    let action = str_field(value, "action")?;
    let resources = match value.get("resources") {
        None => Vec::new(),
        Some(resources) => resources
            .as_array()?
            .iter()
            .map(Value::as_str)
            .collect::<Option<Vec<_>>>()?,
    };
    Some(Ask {
        tool: action.to_owned(),
        command: resources.join("\n"),
        description: None,
        edits_files: matches!(action, "edit" | "write" | "patch"),
        key: ask_key(&[Agent::OpenCode.name(), id]),
    })
}

/// Whether `tool`, as any agent names it, changes files: Claude's `Edit`,
/// `Write`; Codex's `apply_patch`; Copilot's `edit`, `create`,
/// `str_replace_editor`.
fn edits_files(tool: &str) -> bool {
    matches!(
        tool.to_ascii_lowercase().as_str(),
        "edit"
            | "multiedit"
            | "write"
            | "create"
            | "notebookedit"
            | "apply_patch"
            | "str_replace"
            | "str_replace_editor"
            | "str_replace_based_edit_tool"
    )
}

fn str_field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key)?.as_str()
}

fn object_field<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.get(key).filter(|field| field.is_object())
}

/// What approving runs, as people read it, with every value of the input on
/// it: a file edit is its path then its change as `-`/`+` lines, a string
/// `command` is itself whole, and any other tool (an MCP call) is one
/// `key: value` line per argument, nothing when it has none.
fn command_of(tool: &str, input: &Value) -> String {
    if edits_files(tool)
        && let Some(edit) = edit_text(input)
    {
        return edit;
    }
    match str_field(input, "command") {
        Some(command) => command.to_owned(),
        None => argument_lines(input, &[]),
    }
}

/// The fields an edit tool names its file with.
const PATH_FIELDS: [&str; 3] = ["file_path", "path", "notebook_path"];
/// The fields that hold what an edit takes out, then what it puts in.
const OLD_FIELDS: [&str; 2] = ["old_string", "old_str"];
const NEW_FIELDS: [&str; 4] = ["new_string", "new_str", "new_source", "insert_text"];
/// The fields that hold a whole file an edit writes.
const CONTENT_FIELDS: [&str; 2] = ["content", "file_text"];

/// An edit tool's input as its path, its change as diff lines, then any
/// other argument as `key: value`; `None` when it names no file.
fn edit_text(input: &Value) -> Option<String> {
    let (field, path) =
        PATH_FIELDS.iter().find_map(|field| Some((*field, str_field(input, field)?)))?;
    let mut lines = vec![path.to_owned()];
    let mut shown = vec![field];
    shown.extend(change_lines(input, &mut lines));
    // MultiEdit: each of its edits, in order.
    if let Some(edits) = input.get("edits").and_then(Value::as_array)
        && edits.iter().all(Value::is_object)
    {
        for edit in edits {
            let used = change_lines(edit, &mut lines);
            let rest = argument_lines(edit, &used);
            if !rest.is_empty() {
                lines.push(rest);
            }
        }
        shown.push("edits");
    }
    let rest = argument_lines(input, &shown);
    if !rest.is_empty() {
        lines.push(rest);
    }
    Some(lines.join("\n"))
}

/// What one edit takes out as `-` lines, then what it puts in as `+`
/// lines; returns the fields it showed.
fn change_lines(edit: &Value, lines: &mut Vec<String>) -> Vec<&'static str> {
    let mut used = Vec::new();
    for (fields, sign) in [(&OLD_FIELDS[..], '-'), (&NEW_FIELDS[..], '+'), (&CONTENT_FIELDS[..], '+')] {
        if let Some((field, text)) =
            fields.iter().find_map(|field| Some((*field, str_field(edit, field)?)))
        {
            lines.extend(text.lines().map(|line| format!("{sign}{line}")));
            used.push(field);
        }
    }
    used
}

/// One `key: value` line per field of `input` not in `skip`: a string as
/// it is, anything else as compact JSON.
fn argument_lines(input: &Value, skip: &[&str]) -> String {
    let Some(fields) = input.as_object() else {
        return input.to_string();
    };
    fields
        .iter()
        .filter(|(key, _)| !skip.contains(&key.as_str()))
        .map(|(key, value)| match value {
            Value::String(text) => format!("{key}: {text}"),
            other => format!("{key}: {other}"),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Hex SHA-256 over the parts, each followed by a NUL so no two splits collide.
fn ask_key(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update(&[0u8]);
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The longest conversation id taken from a hook.
const CONVERSATION_ID_MAX: usize = 128;

/// The conversation `agent`'s hook payload says its CLI is in now. Copilot
/// names it in `sessionId`, Claude Code and Codex in `session_id`: the id
/// of the conversation, the name of its `session-state/<id>` folder for
/// Copilot. A Claude subagent's hook (one with a non-null `agent_id`) names
/// the child's conversation, not the session's, so it names none here, and
/// a Copilot subagent's (one with a non-null `agentId`) names none either.
/// OpenCode's hooks name none. An id is letters, digits, `-` and `_` only,
/// so it never names a path of its own.
pub fn parse_conversation(agent: Agent, payload: &[u8]) -> Result<String, PayloadError> {
    let none = |why: &'static str| PayloadError::NotAConversation { agent, why };
    let key = match agent {
        Agent::Copilot => "sessionId",
        Agent::Claude | Agent::Codex => "session_id",
        Agent::OpenCode => {
            return Err(none("OpenCode's hooks name no conversation"));
        }
    };
    if payload.len() > PAYLOAD_MAX {
        return Err(PayloadError::TooBig {
            size: payload.len(),
        });
    }
    let value: Value = serde_json::from_slice(payload).map_err(PayloadError::NotJson)?;
    if !value.is_object() {
        return Err(none("the payload isn't a JSON object"));
    }
    let subagent = match agent {
        Agent::Copilot => "agentId",
        _ => "agent_id",
    };
    if value.get(subagent).is_some_and(|id| !id.is_null()) {
        return Err(none("it is a subagent's hook"));
    }
    let id = str_field(&value, key).ok_or_else(|| none("there is no session id"))?;
    let plain = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
    if id.is_empty() || id.len() > CONVERSATION_ID_MAX || !id.chars().all(plain) {
        return Err(none("the session id isn't a plain id"));
    }
    Ok(id.to_owned())
}

/// The file Claude Code's hook says its conversation is kept in, its
/// `transcript_path`: Claude names its project folder from the session
/// folder's real path, which a symlink above it (`/tmp`) makes differ from
/// the path Humata was given. None for any other agent, a subagent's hook
/// (whose file is the parent's), and a path that isn't absolute; the file
/// itself is checked by whoever uses it.
#[must_use]
pub fn parse_transcript(agent: Agent, payload: &[u8]) -> Option<std::path::PathBuf> {
    if agent != Agent::Claude || payload.len() > PAYLOAD_MAX {
        return None;
    }
    let value: Value = serde_json::from_slice(payload).ok()?;
    if value.get("agent_id").is_some_and(|id| !id.is_null()) {
        return None;
    }
    let path = Path::new(str_field(&value, "transcript_path")?);
    path.is_absolute().then(|| path.to_owned())
}

/// What the permission hook prints for `decision`.
pub fn render_permission(agent: Agent, decision: &Decision) -> Vec<u8> {
    let answer = match (agent, decision) {
        (_, Decision::Undecided) => return Vec::new(),
        (Agent::Claude | Agent::Codex, Decision::Allow) => json!({
            "hookSpecificOutput": {
                "hookEventName": "PermissionRequest",
                "decision": {"behavior": "allow"},
            },
        }),
        (Agent::Claude | Agent::Codex, Decision::Deny { message }) => json!({
            "hookSpecificOutput": {
                "hookEventName": "PermissionRequest",
                "decision": {"behavior": "deny", "message": note(message)},
            },
        }),
        (Agent::Copilot, Decision::Allow) => json!({"behavior": "allow"}),
        (Agent::Copilot, Decision::Deny { message }) => {
            json!({"behavior": "deny", "message": note(message)})
        }
        // "Always allow" stays deferred: an approval answers this ask only.
        (Agent::OpenCode, Decision::Allow) => json!({"decision": "once"}),
        (Agent::OpenCode, Decision::Deny { message }) => {
            json!({"decision": "reject", "message": note(message)})
        }
    };
    serde_json::to_vec(&answer).unwrap_or_default()
}

fn note(message: &str) -> &str {
    if message.is_empty() {
        DENY_WITHOUT_NOTE
    } else {
        message
    }
}

/// Single-quotes `text` for `sh`, writing each `'` as `'\''`.
fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// The `humata` call a hook makes, with the socket it talks to.
fn humata_call(humata: &Path, socket: &Path) -> String {
    format!(
        "HUMATA_SOCKET={} {}",
        shell_quote(&socket.to_string_lossy()),
        shell_quote(&humata.to_string_lossy()),
    )
}

/// The shell command an agent's permission hook runs.
pub fn permission_command(agent: Agent, humata: &Path, socket: &Path) -> String {
    format!(
        "{} hook {} permission 2>/dev/null || true",
        humata_call(humata, socket),
        agent.name(),
    )
}

/// The shell command an agent's ask-user hook runs (D761).
fn question_command(agent: Agent, humata: &Path, socket: &Path) -> String {
    format!(
        "{} hook {} question 2>/dev/null || true",
        humata_call(humata, socket),
        agent.name(),
    )
}

/// The shell command an agent's hook runs to say which conversation its
/// CLI is in: the payload on its stdin names it.
fn conversation_command(agent: Agent, humata: &Path, socket: &Path) -> String {
    format!(
        "{} hook {} conversation >/dev/null 2>&1 || true",
        humata_call(humata, socket),
        agent.name(),
    )
}

/// The shell command Claude's Notification hook runs to tell Humata the
/// agent is waiting on a prompt: the payload on its stdin names which.
fn notification_command(agent: Agent, humata: &Path, socket: &Path) -> String {
    format!(
        "{} hook {} notification >/dev/null 2>&1 || true",
        humata_call(humata, socket),
        agent.name(),
    )
}

/// The shell command that tells Humata the session is `state`: working,
/// idle or needs-input. Its output is discarded and it never fails.
fn turn_command(humata: &Path, socket: &Path, state: &str) -> String {
    format!(
        "{} turn {state} >/dev/null 2>&1 || true",
        humata_call(humata, socket),
    )
}

/// The blocking hook added to Claude's `PermissionRequest` group, beside
/// D169's async `turn needs-input` one.
pub fn claude_permission_hook(humata: &Path, socket: &Path) -> serde_json::Value {
    json!({
        "type": "command",
        "command": permission_command(Agent::Claude, humata, socket),
        "timeout": HOLD_SECONDS,
    })
}

/// The Claude hook entry that says which conversation the CLI is in, run
/// in the background on each session start, prompt and turn end (D799).
pub fn claude_conversation_hook(humata: &Path, socket: &Path) -> serde_json::Value {
    json!({
        "type": "command",
        "command": conversation_command(Agent::Claude, humata, socket),
        "async": true,
        "timeout": 10,
    })
}

/// The Claude hook entry that tells Humata a prompt is waiting in its
/// terminal (D798), run in the background so it never blocks Claude.
pub fn claude_notification_hook(humata: &Path, socket: &Path) -> serde_json::Value {
    json!({
        "type": "command",
        "command": notification_command(Agent::Claude, humata, socket),
        "async": true,
        "timeout": 10,
    })
}

/// The blocking Claude hook that answers an ask-user form in the Inbox and
/// holds for the answer, the same as Copilot's `ask_user` one (D761).
pub fn claude_question_hook(humata: &Path, socket: &Path) -> serde_json::Value {
    json!({
        "type": "command",
        "command": question_command(Agent::Claude, humata, socket),
        "timeout": HOLD_SECONDS,
    })
}

/// The TOML value Codex is launched with as `-c hooks=<value>`.
pub fn codex_hooks(humata: &Path, socket: &Path) -> String {
    let command_hook = |command: &str, timeout: u64| {
        format!(
            "{{ type = \"command\", command = {}, timeout = {timeout} }}",
            toml_string(command),
        )
    };
    let group = |hooks: &[String]| format!("{{ hooks = [{}] }}", hooks.join(", "));
    let event = |groups: &[String]| format!("[{}]", groups.join(", "));

    let working = command_hook(&turn_command(humata, socket, "working"), 10);
    let stop_idle = command_hook(
        &format!("{}; echo '{{}}'", turn_command(humata, socket, "idle")),
        10,
    );
    let interrupt_idle = command_hook(&turn_command(humata, socket, "idle"), 3);
    let needs_input = command_hook(&turn_command(humata, socket, "needs-input"), 10);
    let permission = command_hook(
        &permission_command(Agent::Codex, humata, socket),
        HOLD_SECONDS,
    );

    let events = [
        ("UserPromptSubmit", event(&[group(&[working.clone()])])),        ("PostToolUse", event(&[group(&[working])])),
        ("Stop", event(&[group(&[stop_idle])])),
        ("Interrupt", event(&[group(&[interrupt_idle])])),
        (
            "PermissionRequest",
            event(&[group(&[needs_input, permission])]),
        ),
    ];
    let entries: Vec<String> = events
        .iter()
        .map(|(name, value)| format!("{name} = {value}"))
        .collect();
    format!("{{ {} }}", entries.join(", "))
}

/// A TOML basic string for `text`, escaped by the `toml` crate.
fn toml_string(text: &str) -> String {
    toml::Value::String(text.to_owned()).to_string()
}

/// One file of Copilot's session plugin, relative to its folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginFile {
    pub path: &'static str,
    pub contents: String,
}

/// The files of the plugin Copilot loads with `--plugin-dir`.
pub fn copilot_plugin(humata: &Path, socket: &Path) -> Vec<PluginFile> {
    let turn = |state: &str, timeout: u64| {
        json!({
            "type": "command",
            "bash": turn_command(humata, socket, state),
            "timeoutSec": timeout,
        })
    };
    let permission = json!({
        "type": "command",
        "bash": permission_command(Agent::Copilot, humata, socket),
        "timeoutSec": HOLD_SECONDS,
    });
    // Matched to `ask_user` only, so no other tool call waits on a CLI run;
    // the daemon reads the tool name all the same (D761).
    let question = json!({
        "type": "command",
        "matcher": "ask_user",
        "bash": question_command(Agent::Copilot, humata, socket),
        "timeoutSec": HOLD_SECONDS,
    });
    // `/clear` and `/restart` start another conversation in the running
    // agent: its session start says so, and so do the prompt and the end of
    // each turn, by when the conversation's file is written.
    let conversation = json!({
        "type": "command",
        "bash": conversation_command(Agent::Copilot, humata, socket),
        "timeoutSec": 10,
    });
    let hooks = json!({
        "version": 1,
        "hooks": {
            "sessionStart": [conversation.clone()],
            "preToolUse": [question],
            "userPromptSubmitted": [turn("working", 10), conversation.clone()],
            "postToolUse": [turn("working", 10)],
            "agentStop": [turn("idle", 10), conversation],
            "errorOccurred": [turn("idle", 10)],
            "permissionRequest": [permission, turn("needs-input", 10)],
        },
    });
    let plugin = json!({
        "name": "humata",
        "description": "Humata session hooks",
        "version": "1.0.0",
    });
    vec![
        PluginFile {
            path: "plugin.json",
            contents: serde_json::to_string_pretty(&plugin).unwrap_or_default(),
        },
        PluginFile {
            path: "hooks.json",
            contents: serde_json::to_string_pretty(&hooks).unwrap_or_default(),
        },
    ]
}
