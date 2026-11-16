//! Where each turn of an agent's conversation file starts (FK-3, D181): the
//! places a fork before a turn is cut at.

use std::fs::File;
use std::io::{BufRead, BufReader, ErrorKind};
use std::path::Path;

use serde_json::Value;

use crate::MirrorError;

/// One turn of a conversation: a prompt the user sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnMark {
    /// The turn's number, from 1.
    pub index: u32,
    /// The first line of the prompt.
    pub first_line: String,
    /// The line's `timestamp` (RFC 3339) as written, if it has one.
    pub at: Option<String>,
    /// Where the turn's line starts in the file. The bytes before it are the
    /// conversation before the turn.
    pub offset: u64,
}

/// The turns of the conversation file at `file`, in file order.
///
/// A turn is a user line whose text (a string, or a `text` block) isn't
/// empty, that isn't `isMeta` or `isSidechain`: the same rule
/// [`crate::read_from`] marks entries with. A user line of tool answers
/// only is not a turn. Only complete lines count, as they do there, and a
/// missing file has no turns.
pub fn turns_of(file: &Path) -> Result<Vec<TurnMark>, MirrorError> {
    let io = |source| MirrorError::Io {
        path: file.to_path_buf(),
        source,
    };
    let open = match File::open(file) {
        Ok(open) => open,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(io(error)),
    };
    let mut reader = BufReader::new(open);
    let mut turns = Vec::new();
    let mut offset = 0_u64;
    let mut line = Vec::new();
    loop {
        line.clear();
        let read = reader.read_until(b'\n', &mut line).map_err(io)?;
        if read == 0 || line.last() != Some(&b'\n') {
            break;
        }
        if let Ok(value) = serde_json::from_slice::<Value>(&line)
            && let Some(text) = prompt_of(&value)
        {
            turns.push(TurnMark {
                index: u32::try_from(turns.len() + 1).unwrap_or(u32::MAX),
                first_line: text.lines().next().unwrap_or_default().to_owned(),
                at: value
                    .get("timestamp")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                offset,
            });
        }
        offset += read as u64;
    }
    Ok(turns)
}

/// The text of a line that starts a turn, if it is one.
fn prompt_of(line: &Value) -> Option<&str> {
    if line.get("type").and_then(Value::as_str) != Some("user")
        || line.get("isSidechain").and_then(Value::as_bool) == Some(true)
        || line.get("isMeta").and_then(Value::as_bool) == Some(true)
    {
        return None;
    }
    match line.pointer("/message/content")? {
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
