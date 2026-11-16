//! A machine reading its conversation file for the host's mirror (D189).
//!
//! The host asks from the byte offset its mirror reached; the machine, which
//! alone holds the file and its host variable values, answers with the
//! complete lines from there, each scrubbed (LD-21) and written back as one
//! compact JSON line. The machine keeps no state: the offset and the file's
//! identity are the host's to remember and send again.

use std::collections::BTreeSet;
use std::fs::File;
use std::io::{BufRead, BufReader, ErrorKind, Seek, SeekFrom};
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::UNIX_EPOCH;

use serde_json::Value;

use crate::{MirrorError, Scrubber};

/// The most `data` of one answer holds (TR-11): the cap, and what a
/// `max_bytes` of 0 or more than this means.
pub const MAX_CHUNK: usize = 1 << 20;

/// A longer string in a line over [`MAX_CHUNK`] is cut to this many bytes.
const MAX_STRING: usize = 64 << 10;

/// What [`read_lines`] found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lines {
    /// The scrubbed lines, each one compact JSON object ended by `\n`; at
    /// most [`MAX_CHUNK`] bytes unless a single line was all there was.
    pub data: Vec<u8>,
    /// Where the next read starts, in the file's own bytes: just past the
    /// last line read. Past `offset` whenever `more` is set.
    pub next_offset: u64,
    /// Another complete line follows `next_offset`.
    pub more: bool,
    /// The file isn't the one `source` named, or is shorter than `offset`,
    /// so `data` is from byte 0 of the file as it is now.
    pub rewritten: bool,
    /// The file's identity now: the machine, the device, the inode and the
    /// birth time when the file system has one. Never a size or a time that
    /// moves when the file grows.
    pub source: String,
    /// The names of the variables whose values were replaced; never the values.
    pub scrubbed_names: Vec<String>,
    /// The sizes in bytes of lines that were left out because they stayed
    /// over [`MAX_CHUNK`] after long strings were cut. The offset moved past them.
    pub left_out: Vec<u64>,
}

/// The complete lines of `file` from byte `offset`, scrubbed with `scrubber`.
///
/// `machine` is the machine's id, `source` what the last answer called the
/// file (empty the first time, which matches only offset 0), and `max_bytes`
/// the most to read (0, or more than [`MAX_CHUNK`], means [`MAX_CHUNK`]).
/// A last line with no `\n` is still being written and is left for later.
/// A line that isn't JSON is left out and the offset moves past it. A read
/// always takes the first complete line however long it is, so `more` never
/// comes without progress; a line over [`MAX_CHUNK`] once scrubbed has each
/// string over 64 KiB cut with a note of the bytes left out, and is itself
/// left out (see `left_out`) if that isn't enough.
///
/// A file swapped in place for a longer one with the same identity is not
/// noticed: the bytes before `offset` are skipped. Agents only append, so
/// that takes a tool outside them.
///
/// A missing file is `Ok` with no data and `next_offset` = `offset`.
pub fn read_lines(
    file: &Path,
    machine: &str,
    offset: u64,
    max_bytes: u32,
    source: &str,
    scrubber: &Scrubber,
) -> Result<Lines, MirrorError> {
    let io = |source| MirrorError::Io {
        path: file.to_path_buf(),
        source,
    };
    let mut open = match File::open(file) {
        Ok(open) => open,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Ok(Lines {
                next_offset: offset,
                source: source.to_owned(),
                ..Lines::default()
            });
        }
        Err(error) => return Err(io(error)),
    };
    let meta = open.metadata().map_err(io)?;
    let identity = identity_of(machine, &meta);
    let rewritten = if source.is_empty() {
        offset != 0
    } else {
        source != identity || offset > meta.len()
    };
    let start = if rewritten { 0 } else { offset };
    let limit = match usize::try_from(max_bytes) {
        Ok(0) | Err(_) => MAX_CHUNK,
        Ok(max) => max.min(MAX_CHUNK),
    };
    open.seek(SeekFrom::Start(start)).map_err(io)?;
    let mut reader = BufReader::new(open);

    let mut found = Lines {
        rewritten,
        source: identity,
        next_offset: start,
        ..Lines::default()
    };
    let mut names = BTreeSet::new();
    let mut raw = Vec::new();
    let mut consumed = 0u64;
    loop {
        raw.clear();
        let read = reader.read_until(b'\n', &mut raw).map_err(io)?;
        if read == 0 || raw.last() != Some(&b'\n') {
            break;
        }
        // Past the first line, one that doesn't fit waits for the next read.
        if consumed > 0 && consumed + read as u64 > limit as u64 {
            found.more = true;
            break;
        }
        let piece = match prepare(&raw, scrubber, &mut names) {
            Prepared::Line(piece) => Some(piece),
            Prepared::NotJson => None,
            Prepared::TooBig => {
                found.left_out.push(read as u64);
                None
            }
        };
        if let Some(piece) = piece {
            if !found.data.is_empty() && found.data.len() + piece.len() > limit {
                found.more = true;
                break;
            }
            found.data.extend_from_slice(&piece);
        }
        consumed += read as u64;
    }
    found.next_offset = start + consumed;
    found.scrubbed_names = names.into_iter().collect();
    Ok(found)
}

enum Prepared {
    /// The scrubbed line and its `\n`.
    Line(Vec<u8>),
    NotJson,
    TooBig,
}

fn prepare(raw: &[u8], scrubber: &Scrubber, names: &mut BTreeSet<String>) -> Prepared {
    let Ok(text) = std::str::from_utf8(raw) else {
        return Prepared::NotJson;
    };
    let Some((mut value, hit)) = scrubber.scrub_json(text) else {
        return Prepared::NotJson;
    };
    names.extend(hit);
    let mut encoded = match serde_json::to_vec(&value) {
        Ok(encoded) => encoded,
        Err(_) => return Prepared::NotJson,
    };
    if encoded.len() + 1 > MAX_CHUNK {
        cut_long_strings(&mut value);
        encoded = match serde_json::to_vec(&value) {
            Ok(encoded) => encoded,
            Err(_) => return Prepared::TooBig,
        };
        if encoded.len() + 1 > MAX_CHUNK {
            return Prepared::TooBig;
        }
    }
    encoded.push(b'\n');
    Prepared::Line(encoded)
}

/// Cuts every string over [`MAX_STRING`] bytes to its first [`MAX_STRING`]
/// bytes (at a character boundary) and adds a note of what was left out.
fn cut_long_strings(value: &mut Value) {
    match value {
        Value::String(text) if text.len() > MAX_STRING => {
            let mut keep = MAX_STRING;
            while !text.is_char_boundary(keep) {
                keep -= 1;
            }
            let left = text.len() - keep;
            text.truncate(keep);
            text.push_str(&format!(" … [{left} bytes left out]"));
        }
        Value::Array(items) => items.iter_mut().for_each(cut_long_strings),
        Value::Object(fields) => fields.values_mut().for_each(cut_long_strings),
        _ => {}
    }
}

fn identity_of(machine: &str, meta: &std::fs::Metadata) -> String {
    let birth = meta
        .created()
        .ok()
        .and_then(|born| born.duration_since(UNIX_EPOCH).ok())
        .map(|since| format!("{}.{:09}", since.as_secs(), since.subsec_nanos()))
        .unwrap_or_default();
    format!("{machine}:{}:{}:{birth}", meta.dev(), meta.ino())
}
