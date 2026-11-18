//! Reading an agent's conversation file (Claude's JSONL, Codex's rollout or
//! Copilot's session-state events) as entries.

use std::collections::BTreeSet;
use std::fs::File;
use std::io::{BufRead, BufReader, ErrorKind, Seek, SeekFrom};
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::MirrorError;

/// Who an entry is from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Role {
    /// Text the user sent.
    User,
    /// Text the agent wrote.
    Agent,
    /// A tool call the agent made, or what the tool answered.
    Tool,
}

/// One piece of a conversation: a text, a tool call or a tool's answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub role: Role,
    /// The text; for a tool call `<name> <input as compact JSON>`, for a
    /// tool's answer its text.
    pub text: String,
    /// The line's `timestamp` (RFC 3339) as written, if it has one.
    pub at: Option<String>,
    /// True on the first entry of a user line that holds text: the user's
    /// prompt, where a turn starts. A user line with only tool answers
    /// starts none.
    pub new_turn: bool,
}

/// What [`read_from`] found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Read {
    /// The entries of the lines read, in file order.
    pub entries: Vec<Entry>,
    /// The byte offset to read from next time: just after the last complete
    /// line.
    pub next_offset: u64,
}

/// The ids of calls to Humata's own tools that were left out (LD-24,
/// D62.3), so the answer to each is left out too, even when it arrives in a
/// later read. Part of a session's saved mirror state.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnTools {
    dropped: BTreeSet<String>,
}

/// Reads the complete lines of `file` from byte `offset`.
///
/// A last line without `\n` is still being written: it is left for next
/// time and `next_offset` stops before it. Lines that aren't JSON, or that
/// hold no conversation (`summary`, `system`, `file-history-snapshot` and
/// other types that are neither `user` nor `assistant`, user lines with
/// `isMeta: true`, lines with `isSidechain: true`), are skipped. A missing
/// file is `Ok` with no entries and `next_offset` = `offset`.
///
/// Of a line's content blocks, `text` becomes a [`Role::User`] or
/// [`Role::Agent`] entry, `tool_use` a [`Role::Tool`] entry
/// `<name> <input as compact JSON>` and `tool_result` (in user lines; its
/// `content` a string or `[{"type":"text","text":…}]`) a [`Role::Tool`]
/// entry with the answer's text. A user line whose `content` is a string is
/// one text. Other blocks (`thinking`, images) add nothing.
///
/// Calls to Humata's own tools aren't returned: a `tool_use` whose name
/// starts with `mcp__humata`, or a `Bash` one whose `input.command`,
/// trimmed, is `humata` or starts with `humata `. `own_tools` remembers
/// their ids, and the `tool_result` with such a `tool_use_id` isn't
/// returned either.
pub fn read_from(file: &Path, offset: u64, own_tools: &mut OwnTools) -> Result<Read, MirrorError> {
    let io = |source| MirrorError::Io {
        path: file.to_path_buf(),
        source,
    };
    let mut open = match File::open(file) {
        Ok(open) => open,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Ok(Read {
                entries: Vec::new(),
                next_offset: offset,
            });
        }
        Err(error) => return Err(io(error)),
    };
    open.seek(SeekFrom::Start(offset)).map_err(io)?;
    let mut reader = BufReader::new(open);
    let mut entries = Vec::new();
    let mut next_offset = offset;
    let mut line = Vec::new();
    loop {
        line.clear();
        let read = reader.read_until(b'\n', &mut line).map_err(io)?;
        if read == 0 || line.last() != Some(&b'\n') {
            break;
        }
        next_offset += read as u64;
        if let Ok(value) = serde_json::from_slice::<Value>(&line) {
            entries_of(&value, own_tools, &mut entries);
        }
    }
    Ok(Read {
        entries,
        next_offset,
    })
}

/// The entries of `data`, complete lines of a conversation file already
/// read (by [`crate::read_lines`] on a machine, for the host's mirror). The
/// parsing is [`read_from`]'s: same skips, same own-tool filtering. A last
/// piece with no `\n` is not a complete line and is left out.
pub fn parse_lines(data: &[u8], own_tools: &mut OwnTools) -> Vec<Entry> {
    let mut entries = Vec::new();
    let mut rest = data;
    while let Some(end) = rest.iter().position(|byte| *byte == b'\n') {
        let (line, after) = rest.split_at(end + 1);
        if let Ok(value) = serde_json::from_slice::<Value>(line) {
            entries_of(&value, own_tools, &mut entries);
        }
        rest = after;
    }
    entries
}

/// Adds the entries of one parsed line to `out`.
///
/// The agent is told by the line's shape: Claude's `user`/`assistant` lines,
/// Codex's `event_msg`/`response_item` lines with a `payload`, Copilot's
/// dotted `type`s (`user.message`) with a `data`. Any other line adds nothing.
fn entries_of(line: &Value, own_tools: &mut OwnTools, out: &mut Vec<Entry>) {
    match line.get("type").and_then(Value::as_str) {
        Some("event_msg" | "response_item") if line.get("payload").is_some() => {
            codex_entries(line, own_tools, out);
        }
        Some(kind) if kind.contains('.') && line.get("data").is_some() => {
            copilot_entries(kind, line, own_tools, out);
        }
        _ => claude_entries(line, own_tools, out),
    }
}

/// The line's `timestamp` as written.
fn at_of(line: &Value) -> Option<String> {
    line.get("timestamp")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn entry_of(role: Role, text: String, at: &Option<String>, new_turn: bool) -> Entry {
    Entry {
        role,
        text,
        at: at.clone(),
        new_turn,
    }
}

/// The text blocks of a Codex message's `content`, joined by newlines.
/// Codex names a prompt's blocks `text` and a reply's `Text`.
fn codex_text(content: Option<&Value>) -> String {
    let Some(Value::Array(blocks)) = content else {
        return String::new();
    };
    blocks
        .iter()
        .filter(|block| {
            block
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind.eq_ignore_ascii_case("text"))
        })
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// A Codex rollout line (codex-cli 0.169.0).
///
/// The prompt and the reply are read from `event_msg` `item_completed`
/// lines (`UserMessage`, `AgentMessage`), not from the `response_item`
/// `message` lines that carry the same text a second time: the `event_msg`
/// ones never hold the injected AGENTS.md and environment messages, which
/// Codex writes as user `response_item`s. Tool calls and their outputs only
/// exist as `response_item`s: `function_call` (`name`, `arguments` a JSON
/// string, `call_id`), `custom_tool_call` (`input`), `local_shell_call`
/// (`action.command`) and their `*_output` lines (`call_id`, `output`). Those
/// shapes are not in the X1 fixtures yet (module docs of the conversation
/// crate list tool calls as unseen) so they follow Codex's documented
/// rollout item names.
fn codex_entries(line: &Value, own_tools: &mut OwnTools, out: &mut Vec<Entry>) {
    let at = at_of(line);
    let Some(payload) = line.get("payload") else {
        return;
    };
    let kind = payload.get("type").and_then(Value::as_str).unwrap_or("");
    if line.get("type").and_then(Value::as_str) == Some("event_msg") {
        if kind != "item_completed" {
            return;
        }
        let Some(item) = payload.get("item") else {
            return;
        };
        let text = codex_text(item.get("content"));
        if text.is_empty() {
            return;
        }
        match item.get("type").and_then(Value::as_str) {
            Some("UserMessage") => out.push(entry_of(Role::User, text, &at, true)),
            Some("AgentMessage") => out.push(entry_of(Role::Agent, text, &at, false)),
            _ => {}
        }
        return;
    }
    let call_id = payload.get("call_id").and_then(Value::as_str);
    match kind {
        "function_call" | "custom_tool_call" | "local_shell_call" => {
            let name = payload.get("name").and_then(Value::as_str).unwrap_or(
                if kind == "local_shell_call" { "shell" } else { "" },
            );
            let input = match kind {
                "local_shell_call" => payload.get("action").cloned(),
                "custom_tool_call" => payload.get("input").map(json_or_text),
                _ => payload.get("arguments").map(json_or_text),
            };
            record_call(name, input, call_id, &at, own_tools, out);
        }
        "function_call_output" | "custom_tool_call_output" | "local_shell_call_output" => {
            let own = call_id.is_some_and(|id| own_tools.dropped.remove(id));
            let text = match payload.get("output") {
                Some(Value::String(text)) => text.clone(),
                Some(other @ Value::Object(_)) => other
                    .get("content")
                    .or_else(|| other.get("output"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
                _ => String::new(),
            };
            if !own && !text.is_empty() {
                out.push(entry_of(Role::Tool, text, &at, false));
            }
        }
        _ => {}
    }
}

/// A string holding JSON as that JSON, any other value or text as it is.
fn json_or_text(value: &Value) -> Value {
    match value {
        Value::String(text) => serde_json::from_str(text).unwrap_or_else(|_| value.clone()),
        other => other.clone(),
    }
}

/// Adds a tool call of Codex or Copilot as `<name> <input as compact JSON>`,
/// or remembers its id and leaves it out when it is a call to Humata's own
/// tools.
fn record_call(
    name: &str,
    input: Option<Value>,
    id: Option<&str>,
    at: &Option<String>,
    own_tools: &mut OwnTools,
    out: &mut Vec<Entry>,
) {
    if name.starts_with("mcp__humata") || is_own_shell(name, input.as_ref()) {
        if let Some(id) = id {
            own_tools.dropped.insert(id.to_owned());
        }
        return;
    }
    let text = match input {
        Some(Value::String(text)) => format!("{name} {text}"),
        Some(input) => format!("{name} {input}"),
        None => name.to_owned(),
    };
    out.push(entry_of(Role::Tool, text, at, false));
}

/// Whether a shell tool call (Codex `shell`, `local_shell_call`,
/// `exec_command`; Copilot `bash`) runs `humata ...`. The command is a
/// string or an argument list, `["bash", "-lc", "humata ..."]`.
fn is_own_shell(name: &str, input: Option<&Value>) -> bool {
    let shell = ["shell", "bash", "exec_command", "shell_command", "local_shell"];
    if !shell.iter().any(|known| name.eq_ignore_ascii_case(known)) {
        return false;
    }
    let Some(input) = input else {
        return false;
    };
    let command = input.get("command").or_else(|| input.get("cmd"));
    let text = match command {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => {
            let parts: Vec<&str> = parts.iter().filter_map(Value::as_str).collect();
            match parts.as_slice() {
                [_, flag, script] if flag.starts_with('-') => (*script).to_owned(),
                _ => parts.join(" "),
            }
        }
        _ => return false,
    };
    text.trim()
        .strip_prefix("humata")
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace))
}

/// A Copilot session-state line (GitHub Copilot CLI 1.0.112-0).
///
/// `user.message` is a prompt (`data.content`, not `transformedContent`,
/// which adds a date), `assistant.message` a reply (`data.content`, maybe
/// empty when it only asks for tools) with its `data.toolRequests`
/// (`toolCallId`, `name`, `arguments`), and `tool.execution_complete` a
/// tool's answer (`toolCallId`, `result.content`). The fixtures only hold
/// empty `toolRequests`, so those shapes follow Copilot's event names.
/// `tool.execution_start` repeats the request and is not read.
fn copilot_entries(kind: &str, line: &Value, own_tools: &mut OwnTools, out: &mut Vec<Entry>) {
    let at = at_of(line);
    let Some(data) = line.get("data") else {
        return;
    };
    let content = data
        .get("content")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty());
    match kind {
        "user.message" => {
            // A subagent's own prompt carries a `source` (Copilot 1.0.113);
            // it is not something the person typed, so it starts no turn.
            let from_subagent = data.get("source").is_some() || line.get("source").is_some();
            if !from_subagent && let Some(text) = content {
                out.push(entry_of(Role::User, text.to_owned(), &at, true));
            }
        }
        "assistant.message" => {
            if let Some(text) = content {
                out.push(entry_of(Role::Agent, text.to_owned(), &at, false));
            }
            let requests = data.get("toolRequests").and_then(Value::as_array);
            for request in requests.into_iter().flatten() {
                let name = request.get("name").and_then(Value::as_str).unwrap_or("");
                let input = request.get("arguments").map(json_or_text);
                let id = request.get("toolCallId").and_then(Value::as_str);
                record_call(name, input, id, &at, own_tools, out);
            }
        }
        "tool.execution_complete" => {
            let own = data
                .get("toolCallId")
                .and_then(Value::as_str)
                .is_some_and(|id| own_tools.dropped.remove(id));
            let text = match data.pointer("/result/content") {
                Some(Value::String(text)) => text.clone(),
                _ => String::new(),
            };
            if !own && !text.is_empty() {
                out.push(entry_of(Role::Tool, text, &at, false));
            }
        }
        _ => {}
    }
}

/// The entries of a Claude Code line.
fn claude_entries(line: &Value, own_tools: &mut OwnTools, out: &mut Vec<Entry>) {
    let is_user = match line.get("type").and_then(Value::as_str) {
        Some("user") => true,
        Some("assistant") => false,
        _ => return,
    };
    if line.get("isSidechain").and_then(Value::as_bool) == Some(true)
        || (is_user && line.get("isMeta").and_then(Value::as_bool) == Some(true))
    {
        return;
    }
    let at = line
        .get("timestamp")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let text_role = if is_user { Role::User } else { Role::Agent };
    let first = out.len();
    let mut has_text = false;
    let mut push = |role: Role, text: String| {
        out.push(Entry {
            role,
            text,
            at: at.clone(),
            new_turn: false,
        });
    };
    match line.pointer("/message/content") {
        Some(Value::String(text)) => {
            if !text.is_empty() {
                has_text = is_user;
                push(text_role, text.clone());
            }
        }
        Some(Value::Array(blocks)) => {
            for block in blocks {
                match block.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        if let Some(text) = block.get("text").and_then(Value::as_str)
                            && !text.is_empty()
                        {
                            has_text |= is_user;
                            push(text_role, text.to_owned());
                        }
                    }
                    Some("tool_use") if !is_user => {
                        let name = block.get("name").and_then(Value::as_str).unwrap_or("");
                        if is_own_call(name, block.get("input")) {
                            if let Some(id) = block.get("id").and_then(Value::as_str) {
                                own_tools.dropped.insert(id.to_owned());
                            }
                            continue;
                        }
                        let text = match block.get("input") {
                            Some(input) => format!("{name} {input}"),
                            None => name.to_owned(),
                        };
                        push(Role::Tool, text);
                    }
                    Some("tool_result") if is_user => {
                        let own = block
                            .get("tool_use_id")
                            .and_then(Value::as_str)
                            .is_some_and(|id| own_tools.dropped.remove(id));
                        let text = result_text(block.get("content"));
                        if !own && !text.is_empty() {
                            push(Role::Tool, text);
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
    if has_text && let Some(entry) = out.get_mut(first) {
        entry.new_turn = true;
    }
}

/// Whether a `tool_use` is a call to Humata's own tools (LD-24).
fn is_own_call(name: &str, input: Option<&Value>) -> bool {
    if name.starts_with("mcp__humata") {
        return true;
    }
    if name != "Bash" {
        return false;
    }
    let command = input
        .and_then(|input| input.get("command"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    command
        .strip_prefix("humata")
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace))
}

/// The text of a `tool_result`'s `content`: a string or a list of text blocks.
fn result_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}
