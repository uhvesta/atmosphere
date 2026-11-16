//! One session's folder under `<root>/mirror/`: its entries, its saved state
//! and the names its hits show. This is the user data; the index is built
//! from it.

use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::raw::Raw;
use crate::{MirrorEntry, MirrorError, OwnTools, Role, SessionMeta};

const LOG: &str = "entries.log";
const STATE: &str = "state.json";
const META: &str = "meta.json";

/// Where a session's mirror stopped. Saved after the entries it counts are
/// in `entries.log`; bytes of the log past `committed_len` are a cut-off
/// append.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct State {
    /// Bytes of `entries.log` that count.
    pub committed_len: u64,
    /// Where reading the agent's file goes on, in bytes.
    pub offset: u64,
    /// The `seq` of the next entry.
    pub next_seq: u64,
    /// The turn of the last entry.
    pub turn: u32,
    pub own_tools: OwnTools,
    /// When the last catch-up finished (RFC 3339).
    pub as_of: String,
    /// For a session on another machine: what that machine said its
    /// conversation file was at the last read (D189), `offset` counting
    /// that file's bytes. Empty for this host's own sessions.
    #[serde(default)]
    pub source: String,
    /// The raw lines of a session on another machine (D195), kept beside
    /// the entries; see `raw.rs`.
    #[serde(default)]
    pub raw: Raw,
}

/// The names saved by `describe`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct SavedMeta {
    pub channel: String,
    pub channel_name: String,
    pub title: String,
    pub machine: String,
    pub machine_name: String,
    pub agent: String,
    pub archived: bool,
}

impl From<&SessionMeta> for SavedMeta {
    fn from(meta: &SessionMeta) -> Self {
        SavedMeta {
            channel: meta.channel.clone(),
            channel_name: meta.channel_name.clone(),
            title: meta.title.clone(),
            machine: meta.machine.clone(),
            machine_name: meta.machine_name.clone(),
            agent: meta.agent.clone(),
            archived: meta.archived,
        }
    }
}

/// One line of `entries.log`.
#[derive(Serialize, Deserialize)]
struct Stored {
    seq: u64,
    turn: u32,
    role: Role,
    text: String,
    at: Option<String>,
}

/// The folder name for a session id: letters, digits, `_` and `-` as they
/// are, every other byte `%XX`; an empty id is a lone `%`.
pub(crate) fn escape(session: &str) -> String {
    if session.is_empty() {
        return "%".to_owned();
    }
    let mut name = String::with_capacity(session.len());
    for byte in session.bytes() {
        if byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-' {
            name.push(char::from(byte));
        } else {
            name.push_str(&format!("%{byte:02X}"));
        }
    }
    name
}

/// The session id of a folder name, if the name is one [`escape`] makes.
pub(crate) fn unescape(name: &str) -> Option<String> {
    let session = if name == "%" {
        String::new()
    } else {
        let bytes = name.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut at = 0;
        while at < bytes.len() {
            if bytes[at] == b'%' {
                let hex = name.get(at + 1..at + 3)?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                at += 3;
            } else {
                out.push(bytes[at]);
                at += 1;
            }
        }
        String::from_utf8(out).ok()?
    };
    (escape(&session) == name).then_some(session)
}

fn io(path: &Path) -> impl Fn(std::io::Error) -> MirrorError + '_ {
    |source| MirrorError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn corrupt(path: &Path, reason: impl std::fmt::Display) -> MirrorError {
    MirrorError::Corrupt {
        path: path.to_path_buf(),
        reason: reason.to_string(),
    }
}

/// Reads a JSON file of the session folder; `None` when it isn't there.
fn load<T: for<'de> Deserialize<'de>>(dir: &Path, name: &str) -> Result<Option<T>, MirrorError> {
    let path = dir.join(name);
    match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|error| corrupt(&path, error)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io(&path)(error)),
    }
}

/// Replaces a JSON file of the session folder in one step: a crash leaves
/// the old file or the new one.
fn save<T: Serialize>(dir: &Path, name: &str, value: &T) -> Result<(), MirrorError> {
    fs::create_dir_all(dir).map_err(io(dir))?;
    let path = dir.join(name);
    let temp = dir.join(format!("{name}.tmp"));
    let bytes = serde_json::to_vec(value).map_err(|error| corrupt(&path, error))?;
    let mut file = File::create(&temp).map_err(io(&temp))?;
    file.write_all(&bytes).map_err(io(&temp))?;
    file.sync_all().map_err(io(&temp))?;
    fs::rename(&temp, &path).map_err(io(&path))?;
    if let Ok(folder) = File::open(dir) {
        // Best effort: makes the rename itself durable.
        let _ = folder.sync_all();
    }
    Ok(())
}

pub(crate) fn load_state(dir: &Path) -> Result<State, MirrorError> {
    Ok(load(dir, STATE)?.unwrap_or_default())
}

pub(crate) fn save_state(dir: &Path, state: &State) -> Result<(), MirrorError> {
    save(dir, STATE, state)
}

pub(crate) fn load_meta(dir: &Path) -> Result<Option<SavedMeta>, MirrorError> {
    load(dir, META)
}

pub(crate) fn save_meta(dir: &Path, meta: &SavedMeta) -> Result<(), MirrorError> {
    save(dir, META, meta)
}

/// Drops what a crash left after the last committed append.
pub(crate) fn trim_log(dir: &Path, committed: u64) -> Result<(), MirrorError> {
    let path = dir.join(LOG);
    let len = match fs::metadata(&path) {
        Ok(meta) => meta.len(),
        Err(error) if error.kind() == ErrorKind::NotFound => 0,
        Err(error) => return Err(io(&path)(error)),
    };
    if len < committed {
        return Err(corrupt(
            &path,
            format!("it holds {len} bytes but {committed} were saved"),
        ));
    }
    if len > committed {
        let file = OpenOptions::new().write(true).open(&path).map_err(io(&path))?;
        file.set_len(committed).map_err(io(&path))?;
        file.sync_all().map_err(io(&path))?;
    }
    Ok(())
}

/// Adds `entries` after the first `committed` bytes of the log and returns
/// how many bytes that is. The append counts only once the state that
/// covers it is saved.
pub(crate) fn append(dir: &Path, committed: u64, entries: &[MirrorEntry]) -> Result<u64, MirrorError> {
    fs::create_dir_all(dir).map_err(io(dir))?;
    if entries.is_empty() {
        return Ok(0);
    }
    let path = dir.join(LOG);
    let mut bytes = Vec::new();
    for entry in entries {
        let stored = Stored {
            seq: entry.seq,
            turn: entry.turn,
            role: entry.role,
            text: entry.text.clone(),
            at: entry.at.clone(),
        };
        serde_json::to_writer(&mut bytes, &stored).map_err(|error| corrupt(&path, error))?;
        bytes.push(b'\n');
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(io(&path))?;
    file.set_len(committed).map_err(io(&path))?;
    file.seek(SeekFrom::Start(committed)).map_err(io(&path))?;
    file.write_all(&bytes).map_err(io(&path))?;
    file.sync_data().map_err(io(&path))?;
    Ok(bytes.len() as u64)
}

/// The entries in the first `committed` bytes of the log.
pub(crate) fn read_log(dir: &Path, committed: u64) -> Result<Vec<MirrorEntry>, MirrorError> {
    if committed == 0 {
        return Ok(Vec::new());
    }
    let path = dir.join(LOG);
    let bytes = fs::read(&path).map_err(io(&path))?;
    let Some(bytes) = usize::try_from(committed)
        .ok()
        .and_then(|len| bytes.get(..len))
    else {
        return Err(corrupt(&path, "it is shorter than the saved length"));
    };
    bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| {
            let stored: Stored =
                serde_json::from_slice(line).map_err(|error| corrupt(&path, error))?;
            Ok(MirrorEntry {
                seq: stored.seq,
                turn: stored.turn,
                role: stored.role,
                text: stored.text,
                at: stored.at,
            })
        })
        .collect()
}

/// The session folders under `mirror_dir`, as session ids.
pub(crate) fn sessions(mirror_dir: &Path) -> Result<Vec<String>, MirrorError> {
    let mut found = Vec::new();
    for item in fs::read_dir(mirror_dir).map_err(io(mirror_dir))? {
        let item = item.map_err(io(mirror_dir))?;
        let is_dir = item.file_type().map_err(io(&item.path()))?.is_dir();
        if let Some(session) = item.file_name().to_str().and_then(unescape)
            && is_dir
        {
            found.push(session);
        }
    }
    Ok(found)
}

pub(crate) fn remove_dir(dir: &Path) -> Result<(), MirrorError> {
    match fs::remove_dir_all(dir) {
        Err(error) if error.kind() != ErrorKind::NotFound => Err(io(dir)(error)),
        _ => Ok(()),
    }
}

pub(crate) fn dir_of(mirror_dir: &Path, session: &str) -> PathBuf {
    mirror_dir.join(escape(session))
}

/// The current time as RFC 3339, UTC, to the millisecond.
pub(crate) fn now() -> String {
    let since = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = since.as_secs();
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    // Civil date from days since 1970-01-01.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let rest = secs % 86_400;
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60,
        since.subsec_millis()
    )
}
