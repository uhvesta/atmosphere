//! Reading an agent's conversation file (Claude's JSONL) as entries.

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
fn entries_of(line: &Value, own_tools: &mut OwnTools, out: &mut Vec<Entry>) {
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
