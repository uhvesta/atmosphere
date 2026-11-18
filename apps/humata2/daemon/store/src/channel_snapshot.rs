//! A deleted channel's records, kept beside its folder (CH-24, D194).
//!
//! Deleting a channel copies its rows into a side SQLite file of the same
//! schema, `<channel folder>/.humata-deleted.sqlite`, then removes them from
//! the state in one transaction. The folder, with the file in it, goes to
//! the trash. Restoring the folder brings the rows back in one transaction
//! with every session archived, and removes the file.
//!
//! Every table of the schema is either copied (it holds the channel's own
//! rows) or global (it holds the machine's: the journal, default rules,
//! script tokens, board prefixes and counters). A test fails when a table is
//! neither, so a new table has to say which.

use std::fs;
use std::path::{Path, PathBuf};

use prost::Message;
use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, params, params_from_iter};

use crate::proto::AgentLaunch as LaunchProto;
use crate::store::{Inner, bytes_path, check_id, from_sql, to_sql};
use crate::{ChannelRecord, Store, StoreError, blob, schema};

/// The side file's name inside the channel's folder.
pub const SIDE_FILE: &str = ".humata-deleted.sqlite";

const IN_CHANNEL: &str = "session IN (SELECT id FROM sessions WHERE channel = ?1)";
const IN_CHANNEL_ID: &str = "session_id IN (SELECT id FROM sessions WHERE channel = ?1)";
const OF_CHANNEL: &str = "channel = ?1";
const ITEMS: &str = "item IN (SELECT key FROM board_items WHERE channel = ?1)";
const OF_TABS: &str =
    "tab IN (SELECT id FROM tabs WHERE session IN (SELECT id FROM sessions WHERE channel = ?1))";

/// A table whose rows belong to a channel.
struct Copied {
    name: &'static str,
    /// Picks the channel's rows; `?1` is the channel's id.
    scope: &'static str,
    /// Whether restoring brings the rows back. Tabs don't: their terminals
    /// are gone, and a restored session starts new ones when it resumes.
    restored: bool,
    /// `(inline column, file column)` of a blob that may live in a file of
    /// the state folder; the side file keeps it inline.
    blob: Option<(&'static str, &'static str)>,
}

const fn copied(name: &'static str, scope: &'static str) -> Copied {
    Copied {
        name,
        scope,
        restored: true,
        blob: None,
    }
}

/// Parents come before the tables that reference them.
const COPIED: &[Copied] = &[
    copied("channels", "id = ?1"),
    copied("channel_repos", OF_CHANNEL),
    copied("channel_machines", OF_CHANNEL),
    copied("sessions", OF_CHANNEL),
    copied("session_details", IN_CHANNEL),
    copied("session_repos", IN_CHANNEL),
    copied("last_copies", IN_CHANNEL),
    copied("away_left_out", IN_CHANNEL),
    copied("review_sessions", IN_CHANNEL),
    copied("review_session_prs", IN_CHANNEL),
    Copied {
        restored: false,
        ..copied("tabs", IN_CHANNEL)
    },
    Copied {
        restored: false,
        ..copied("tab_program", OF_TABS)
    },
    copied("pointer_reports", IN_CHANNEL),
    copied("repo_pointers", IN_CHANNEL),
    copied("dirty_files", IN_CHANNEL),
    Copied {
        blob: Some(("launch", "launch_file")),
        ..copied("agents", IN_CHANNEL)
    },
    copied("interruptions", IN_CHANNEL),
    Copied {
        blob: Some(("state", "state_file")),
        ..copied("reviews", IN_CHANNEL)
    },
    Copied {
        blob: Some(("view", "view_file")),
        ..copied("views", IN_CHANNEL)
    },
    Copied {
        blob: Some(("entry", "entry_file")),
        ..copied("inbox_requests", IN_CHANNEL)
    },
    copied("comments", IN_CHANNEL),
    copied("comment_deliveries", IN_CHANNEL),
    copied("comment_numbers", IN_CHANNEL),
    copied("doc_comment_records", IN_CHANNEL),
    copied("desk_sessions", IN_CHANNEL_ID),
    copied("previous_conversations", IN_CHANNEL_ID),
    copied("fresh_seeds", IN_CHANNEL_ID),
    copied("fork_origins", IN_CHANNEL_ID),
    copied("archives", IN_CHANNEL_ID),
    copied("remote_session_reports", IN_CHANNEL),
    copied("access_channel_rules", OF_CHANNEL),
    copied("board_items", OF_CHANNEL),
    copied("board_item_sessions", ITEMS),
    copied("board_item_deps", ITEMS),
    copied("board_history", ITEMS),
];

/// Tables of the machine, not of a channel: they stay where they are. The
/// board prefixes and counters stay so an item's key is never used twice.
#[cfg(test)]
const GLOBAL: &[&str] = &[
    "journal",
    "journal_state",
    "access_default_rules",
    "script_tokens",
    "settings",
    "board_prefixes",
    "board_counters",
    "machine_journal",
    "channel_desk_sessions",
    "pr_commit_marks",
    "pr_review_history",
    // A Type call's key belongs to the call, not to a channel.
    "typed_keys",
];

/// Who a side file holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeletedChannel {
    pub id: String,
    pub name: String,
    /// The channel's folder as it was recorded.
    pub folder: String,
}

/// Where a restored channel stands now.
#[derive(Clone, Debug)]
pub struct Placement {
    /// The name and folder it comes back under: the recorded ones, or
    /// the first free `-2`, `-3`, ... of them when something took them.
    pub name: String,
    pub folder: String,
    /// The channel's folder when it was deleted, and where it is now. The
    /// paths of its sessions are moved from one to the other.
    pub old_path: PathBuf,
    pub new_path: PathBuf,
    pub now_unix_ms: i64,
}

pub(crate) fn side_error(path: &Path, doing: &'static str) -> impl FnOnce(rusqlite::Error) -> StoreError {
    let path = path.to_owned();
    move |source| StoreError::Database { path, doing, source }
}

/// The side file's own rows of `table`, all of them.
pub(crate) fn rows_of(connection: &Connection, table: &str, scope: Option<(&str, &str)>) -> rusqlite::Result<(Vec<String>, Vec<Vec<Value>>)> {
    let sql = match scope {
        Some((scope, _)) => format!("SELECT * FROM {table} WHERE {scope}"),
        None => format!("SELECT * FROM {table}"),
    };
    let mut statement = connection.prepare(&sql)?;
    let names: Vec<String> = statement.column_names().into_iter().map(str::to_owned).collect();
    let count = names.len();
    let read = |row: &rusqlite::Row<'_>| (0..count).map(|at| row.get::<_, Value>(at)).collect::<rusqlite::Result<Vec<_>>>();
    let rows = match scope {
        Some((_, channel)) => statement.query_map([channel], read)?.collect::<rusqlite::Result<Vec<_>>>()?,
        None => statement.query_map([], read)?.collect::<rusqlite::Result<Vec<_>>>()?,
    };
    Ok((names, rows))
}

pub(crate) fn insert_rows(connection: &Connection, table: &str, names: &[String], rows: Vec<Vec<Value>>) -> rusqlite::Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let marks = vec!["?"; names.len()].join(", ");
    let mut statement = connection.prepare(&format!("INSERT INTO {table} ({}) VALUES ({marks})", names.join(", ")))?;
    for row in rows {
        statement.execute(params_from_iter(row))?;
    }
    Ok(())
}

/// Removes the side file and what SQLite keeps beside it.
pub(crate) fn remove_side(side: &Path) -> Result<(), StoreError> {
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let mut name = side.as_os_str().to_owned();
        name.push(suffix);
        let path = PathBuf::from(name);
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => return Err(StoreError::Remove { path, source }),
        }
    }
    Ok(())
}

/// The longest key an agent keeps whole: its folder's path at 200 units.
const KEY_UNITS: usize = 200;

/// A conversation file that has to move with its channel: where it is, and
/// where its session's launch now names it.
pub type ConversationMove = (PathBuf, PathBuf);

/// How an agent that keeps a conversation per folder writes `path`: the
/// CLI's own rule, long folders included (D347).
fn dashed(path: &str) -> String {
    humata2_conversation::claude_project_name(std::path::Path::new(path))
}

/// `path` as JSON writes it inside a string.
fn escaped(path: &str) -> String {
    path.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Whether `rest`, what follows a folder's path in some text, shows that
/// the path was the whole of it and not the start of another folder's name
/// (`payments` against `payments-2`).
fn ends_path(rest: &str) -> bool {
    let mut chars = rest.chars();
    match chars.next() {
        None => true,
        Some('.') => !chars.next().is_some_and(char::is_alphanumeric),
        Some(c) => !(c.is_alphanumeric() || matches!(c, '-' | '_')),
    }
}

/// The same for a folder as an agent writes it in a conversation's key:
/// what follows is the end, a separator, or the name of one of a channel's
/// folders (`-sessions-`, `-desk-`, `-main-`).
fn ends_key(rest: &str) -> bool {
    match rest.strip_prefix('-') {
        Some(after) => ["sessions-", "desk-", "main-", "shared"]
            .iter()
            .any(|inside| after.starts_with(inside)),
        None => rest.chars().next().is_none_or(|c| !(c.is_alphanumeric() || c == '_')),
    }
}

/// `text` with, at each place, the longest of `keys` that its `ends`
/// accepts replaced by its replacement.
fn replace_keys(text: &str, keys: &[(String, String, fn(&str) -> bool)]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while at < text.len() {
        let mut best: Option<(usize, usize, &str)> = None;
        for (from, to, ends) in keys.iter().filter(|(from, ..)| !from.is_empty()) {
            let mut from_at = at;
            while let Some(found) = text[from_at..].find(from.as_str()) {
                let start = from_at + found;
                let end = start + from.len();
                if ends(&text[end..]) {
                    let better = best.is_none_or(|(s, e, _)| start < s || (start == s && end > e));
                    if better {
                        best = Some((start, end, to.as_str()));
                    }
                    break;
                }
                // `from` is never empty and starts a char, so this stays on a boundary.
                from_at = start + text[start..].chars().next().map_or(1, char::len_utf8);
            }
        }
        let Some((start, end, to)) = best else { break };
        out.push_str(&text[at..start]);
        out.push_str(to);
        at = end;
    }
    out.push_str(&text[at..]);
    out
}

/// Whether `rest` follows a whole conversation key: a key cut at 200 units
/// ends in its hash, so only a character no key holds can follow it.
fn ends_whole_key(rest: &str) -> bool {
    rest.chars().next().is_none_or(|c| !(c.is_alphanumeric() || matches!(c, '-' | '_')))
}

/// `text` with each `from` that `ends` accepts replaced by `to`.
fn replace_where(text: &str, from: &str, to: &str, ends: fn(&str) -> bool) -> String {
    if from.is_empty() {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while let Some(found) = text[at..].find(from) {
        let start = at + found;
        let end = start + from.len();
        out.push_str(&text[at..start]);
        out.push_str(if ends(&text[end..]) { to } else { from });
        at = end;
    }
    out.push_str(&text[at..]);
    out
}

impl Placement {
    /// `text` with the old folder's path made the new one's: the path as it
    /// is, as JSON writes it, and as an agent names a conversation's folder.
    /// `None` when nothing changes. A path that only starts like the old one
    /// (`payments-2` against `payments`) is left alone.
    pub fn rewritten(&self, text: &str) -> Option<String> {
        let (old, new) = (self.old_path.to_str()?, self.new_path.to_str()?);
        if old == new {
            return None;
        }
        let mut now = replace_where(text, old, new, ends_path);
        if escaped(old) != old {
            now = replace_where(&now, &escaped(old), &escaped(new), ends_path);
        }
        now = replace_where(&now, &dashed(old), &dashed(new), ends_key);
        (now != text).then_some(now)
    }

    /// [`Placement::rewritten`], with the keys of `known` folders (the ones
    /// under the old path that the records name) made exact. A key over 200
    /// units is the folder's first 200 and a hash of the whole path, so it
    /// is no old key plus a suffix and the prefix replace would miss it,
    /// or cut a long one wrong. Each such key becomes the new path's own;
    /// the prefix replace stays for the folders whose keys are at most 200
    /// units on both sides.
    pub fn rewritten_among(&self, text: &str, known: &[PathBuf]) -> Option<String> {
        let (old, new) = (self.old_path.to_str()?, self.new_path.to_str()?);
        if old == new {
            return None;
        }
        let mut now = replace_where(text, old, new, ends_path);
        if escaped(old) != old {
            now = replace_where(&now, &escaped(old), &escaped(new), ends_path);
        }
        now = replace_keys(&now, &self.keys(known));
        (now != text).then_some(now)
    }

    /// The keys to replace: the folder's own and each known folder's under
    /// it, with the replacement of each, the longest first.
    fn keys(&self, known: &[PathBuf]) -> Vec<(String, String, fn(&str) -> bool)> {
        let name = humata2_conversation::claude_project_name;
        let (old, new) = (name(&self.old_path), name(&self.new_path));
        let own: fn(&str) -> bool = if old.len() <= KEY_UNITS && new.len() <= KEY_UNITS {
            ends_key
        } else {
            ends_whole_key
        };
        let mut keys = vec![(old, new, own)];
        for folder in known {
            let Ok(rest) = folder.strip_prefix(&self.old_path) else { continue };
            let from = name(folder);
            // strip_prefix goes by components and drops a trailing '/', which
            // the folder's record (rewritten as text) keeps, and so does its key.
            let mut moved = self.new_path.join(rest);
            if folder.as_os_str().as_encoded_bytes().ends_with(b"/") {
                moved.push("");
            }
            let to = name(&moved);
            if from != to && !keys.iter().any(|(have, ..)| *have == from) {
                keys.push((from, to, ends_whole_key));
            }
        }
        keys.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
        keys
    }

    /// [`Placement::rewritten_among`], and the keys the conversation `moves`
    /// name made exact too: a move from `<dir>/<old key>/..` to
    /// `<dir>/<new key>/..` says what the key of a folder the records don't
    /// otherwise name becomes. Only a key cut at 200 units needs it.
    pub fn rewritten_moved(&self, text: &str, known: &[PathBuf], moves: &[ConversationMove]) -> Option<String> {
        let (old, new) = (self.old_path.to_str()?, self.new_path.to_str()?);
        if old == new {
            return None;
        }
        let mut now = replace_where(text, old, new, ends_path);
        if escaped(old) != old {
            now = replace_where(&now, &escaped(old), &escaped(new), ends_path);
        }
        let key_of = |file: &Path| file.parent().and_then(Path::file_name).and_then(|name| name.to_str()).map(str::to_owned);
        let mut keys = self.keys(known);
        for (was, then) in moves {
            if let (Some(from), Some(to)) = (key_of(was), key_of(then))
                && from != to
                && (from.len() > KEY_UNITS || to.len() > KEY_UNITS)
                && !keys.iter().any(|(have, ..)| *have == from)
            {
                keys.push((from, to, ends_whole_key));
            }
        }
        keys.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
        now = replace_keys(&now, &keys);
        (now != text).then_some(now)
    }

    fn bytes(&self, bytes: &[u8], known: &[PathBuf]) -> Option<Vec<u8>> {
        let text = std::str::from_utf8(bytes).ok()?;
        self.rewritten_among(text, known).map(String::into_bytes)
    }
}

/// The sessions whose places are under a channel's folder: its own, and its
/// Desk sessions, which are filed under the Desk's channel. With `?2` set,
/// only those of that machine (D348: the host's records of a machine that
/// hasn't renamed its folder yet keep their old paths).
const OF_FOLDER: &str = "SELECT id FROM sessions WHERE channel = ?1 AND (?2 IS NULL OR machine = ?2)
    UNION SELECT session_id FROM channel_desk_sessions WHERE channel = ?1
        AND (?2 IS NULL OR NOT EXISTS (SELECT 1 FROM sessions WHERE id = session_id AND machine <> ?2))";

/// Points the records of `channel`'s sessions at `placement`'s new folder:
/// each session's folder, its clones, its agent's launch and its previous
/// conversations. Writes only when `write` is set, so the same walk can
/// list the conversation files that have to move. Answers those, each once.
/// `on` set keeps to that machine's sessions.
fn rebase_rows(
    inner: &Inner,
    connection: &Connection,
    channel: &str,
    on: Option<&str>,
    placement: &Placement,
    write: bool,
) -> Result<Vec<ConversationMove>, StoreError> {
    let failed = inner.failed("move a channel's records to its new folder");
    let mut moved: Vec<ConversationMove> = Vec::new();
    if placement.old_path == placement.new_path {
        return Ok(moved);
    }
    let sessions = connection
        .prepare(OF_FOLDER)
        .and_then(|mut statement| {
            statement
                .query_map(params![channel, on], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(&failed)?;
    for session in &sessions {
        let folder = connection
            .query_row("SELECT folder FROM sessions WHERE id = ?1", [session], |row| row.get::<_, Vec<u8>>(0))
            .optional()
            .map_err(&failed)?;
        let clones = connection
            .prepare("SELECT owner, name, path FROM session_repos WHERE session = ?1")
            .and_then(|mut statement| {
                statement
                    .query_map([session], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, Vec<u8>>(2)?))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .map_err(&failed)?;
        // The folders this session names: where the keys of its agent's
        // conversations come from.
        let mut known: Vec<PathBuf> = folder
            .iter()
            .cloned()
            .chain(clones.iter().map(|(.., path)| path.clone()))
            .map(bytes_path)
            .collect();
        if write && let Some(now) = folder.and_then(|folder| placement.bytes(&folder, &known)) {
            connection
                .execute("UPDATE sessions SET folder = ?2 WHERE id = ?1", params![session, now])
                .map_err(&failed)?;
        }
        for (owner, name, path) in clones {
            if write && let Some(now) = placement.bytes(&path, &known) {
                connection
                    .execute(
                        "UPDATE session_repos SET path = ?4 WHERE session = ?1 AND owner = ?2 AND name = ?3",
                        params![session, owner, name, now],
                    )
                    .map_err(&failed)?;
            }
        }
        // The agent's own folder joins them: its previous conversations
        // are keyed by it too, even two levels under a clone.
        rebase_launch(inner, connection, session, placement, &mut known, write, &mut moved)?;
        let previous = connection
            .prepare("SELECT conversation_id, path FROM previous_conversations WHERE session_id = ?1")
            .and_then(|mut statement| {
                statement
                    .query_map([session], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?)))?
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .map_err(&failed)?;
        for (conversation, path) in previous {
            let Some(now) = placement.bytes(&path, &known) else { continue };
            if write {
                connection
                    .execute(
                        "UPDATE previous_conversations SET path = ?3 WHERE session_id = ?1 AND conversation_id = ?2",
                        params![session, conversation, now],
                    )
                    .map_err(&failed)?;
            }
            moved.push((bytes_path(path), bytes_path(now)));
        }
    }
    moved.sort();
    moved.dedup();
    Ok(moved)
}

/// [`rebase_rows`] for one session's agent: its folder, its commands and
/// its conversation file. Adds the agent's folder (as it was) to `known`.
fn rebase_launch(
    inner: &Inner,
    connection: &Connection,
    session: &str,
    placement: &Placement,
    known: &mut Vec<PathBuf>,
    write: bool,
    moved: &mut Vec<ConversationMove>,
) -> Result<(), StoreError> {
    let failed = inner.failed("move a channel's records to its new folder");
    let row = connection
        .query_row("SELECT launch, launch_file FROM agents WHERE session = ?1", [session], |row| {
            Ok((row.get::<_, Option<Vec<u8>>>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .optional()
        .map_err(&failed)?;
    let Some((inline, file)) = row else { return Ok(()) };
    let what = || format!("the agent of session {session}");
    let (version, body) = blob::read(&inner.blobs, &inner.database, inline, file, blob::Type::AgentLaunch, what)?;
    let mut launch = LaunchProto::decode(body.as_slice()).map_err(|_| StoreError::Damaged {
        path: inner.database.clone(),
        what: what(),
        reason: "how it was launched is unreadable",
    })?;
    let before = launch.conversation_file.clone();
    let mut changed = false;
    known.push(bytes_path(launch.folder.clone()));
    let known = &*known;
    let mut point = |field: &mut Vec<u8>| {
        if let Some(now) = placement.bytes(field, known) {
            *field = now;
            changed = true;
        }
    };
    point(&mut launch.folder);
    for arg in launch.argv.iter_mut().chain(launch.resume_argv.iter_mut()) {
        point(arg);
    }
    point(&mut launch.conversation_file);
    if !changed {
        return Ok(());
    }
    if launch.conversation_file != before {
        moved.push((bytes_path(before), bytes_path(launch.conversation_file.clone())));
    }
    if write {
        let encoded = blob::encode(blob::Type::AgentLaunch, version, &launch.encode_to_vec())?;
        let placed = blob::place(&inner.blobs, encoded)?;
        connection
            .execute(
                "UPDATE agents SET launch = ?2, launch_file = ?3 WHERE session = ?1",
                params![session, placed.inline(), placed.file()],
            )
            .map_err(&failed)?;
    }
    Ok(())
}

impl Store {
    /// Copies the rows of `channel` and of its sessions into a new side
    /// file at `side`, replacing one already there, and leaves the state
    /// as it was. A blob kept in a file of the state folder goes inline.
    pub fn export_channel(&self, channel: &str, side: &Path) -> Result<(), StoreError> {
        check_id(channel)?;
        remove_side(side)?;
        let target = schema::open(side)?;
        let (database, blobs) = (self.inner().database.clone(), self.inner().blobs.clone());
        let copied = self.inner().read_with(|source| {
            let write = side_error(side, "copy the channel's records");
            let transaction = target.unchecked_transaction().map_err(write)?;
            for table in COPIED {
                let (names, mut rows) = rows_of(source, table.name, Some((table.scope, channel)))
                    .map_err(|source| StoreError::Database {
                        path: database.clone(),
                        doing: "read the channel's records",
                        source,
                    })?;
                if let Some((inline, file)) = table.blob {
                    let at = |wanted: &str| names.iter().position(|name| name == wanted);
                    if let (Some(inline), Some(file)) = (at(inline), at(file)) {
                        for row in &mut rows {
                            if let Value::Text(hash) = &row[file] {
                                let path = blobs.join(hash);
                                let bytes = fs::read(&path).map_err(|source| StoreError::Read { path, source })?;
                                row[inline] = Value::Blob(bytes);
                                row[file] = Value::Null;
                            }
                        }
                    }
                }
                insert_rows(&transaction, table.name, &names, rows).map_err(side_error(side, "copy the channel's records"))?;
            }
            transaction.commit().map_err(side_error(side, "save the channel's records"))
        });
        let finished = copied.and_then(|()| {
            // One file, so the folder holds everything it needs.
            target
                .execute_batch("PRAGMA journal_mode = DELETE")
                .map_err(side_error(side, "close the side file"))
        });
        drop(target);
        if finished.is_err() {
            let _ = remove_side(side);
        }
        finished
    }

    /// Removes the rows [`Store::export_channel`] copies, in one
    /// transaction, and the files the sessions pasted. The channel's board
    /// prefix and counters stay. Returns the ids of the sessions removed.
    pub fn forget_channel(&self, channel: &str) -> Result<Vec<String>, StoreError> {
        check_id(channel)?;
        let sessions = self.inner().write("channel.forget", "remove a channel's records", |transaction| {
            let sessions = transaction
                .prepare("SELECT id FROM sessions WHERE channel = ?1")?
                .query_map([channel], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for table in COPIED.iter().rev() {
                transaction.execute(&format!("DELETE FROM {} WHERE {}", table.name, table.scope), [channel])?;
            }
            Ok(sessions)
        })?;
        for session in &sessions {
            let files = self.root().join("files").join(session);
            match fs::remove_dir_all(&files) {
                Err(source) if source.kind() != std::io::ErrorKind::NotFound => {
                    return Err(StoreError::Remove { path: files, source });
                }
                _ => {}
            }
        }
        self.inner().sweep()?;
        Ok(sessions)
    }

    /// Who the side file at `side` holds. It is brought up to this build's
    /// schema first, as the state is.
    pub fn deleted_channel(side: &Path) -> Result<DeletedChannel, StoreError> {
        let connection = schema::open(side)?;
        let found = connection
            .query_row("SELECT id, name, folder FROM channels", [], |row| {
                Ok(DeletedChannel {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    folder: row.get(2)?,
                })
            })
            .optional()
            .map_err(side_error(side, "read the deleted channel"))?;
        found.ok_or_else(|| StoreError::NoDeletedChannel { path: side.to_owned() })
    }

    /// Brings the channel in the side file back in one transaction, under
    /// `placement`, with every one of its sessions archived, and removes
    /// the side file. The channel and its sessions must not be in the state.
    ///
    /// A channel that comes back elsewhere has the paths in its sessions'
    /// records, and in its Desk sessions', made the new folder's (the
    /// records of its Desk sessions stay in the state, where they were). The
    /// conversation files that must move with them are answered, each as
    /// where it is and where the launch now names it. Its archived
    /// sessions keep their folders until `free_after_unix_ms` at least: a
    /// restore gives them an undo window of their own.
    pub fn import_channel(
        &self,
        side: &Path,
        placement: &Placement,
        free_after_unix_ms: i64,
    ) -> Result<Vec<ConversationMove>, StoreError> {
        let source = schema::open(side)?;
        let mut tables = Vec::new();
        for table in COPIED.iter().filter(|table| table.restored) {
            let (names, rows) = rows_of(&source, table.name, None).map_err(side_error(side, "read the deleted channel"))?;
            tables.push((table.name, names, rows));
        }
        let channel = Self::deleted_channel(side)?;
        drop(source);
        let doing = "bring a deleted channel back";
        let moved = self.inner().write_with("channel.import", doing, |transaction| {
            let failed = self.inner().failed(doing);
            for (table, names, rows) in tables {
                insert_rows(transaction, table, &names, rows).map_err(&failed)?;
            }
            transaction
                .execute(
                    "UPDATE channels SET name = ?2, folder = ?3 WHERE id = ?1",
                    rusqlite::params![channel.id, placement.name, placement.folder],
                )
                .map_err(&failed)?;
            let moved = rebase_rows(self.inner(), transaction, &channel.id, None, placement, true)?;
            // Every session comes back archived: the ones that were not
            // get a fresh archive row, and one cut short counts as done.
            transaction
                .execute(
                    "UPDATE archives SET state = 1 WHERE state = 0 AND session_id IN (SELECT id FROM sessions WHERE channel = ?1)",
                    [&channel.id],
                )
                .map_err(&failed)?;
            transaction
                .execute(
                    "INSERT OR IGNORE INTO archives (session_id, archived_unix_ms, state, summary, free_after_unix_ms)
                     SELECT id, ?2, 1, X'', ?3 FROM sessions WHERE channel = ?1",
                    rusqlite::params![channel.id, placement.now_unix_ms, free_after_unix_ms],
                )
                .map_err(&failed)?;
            // One archived before the delete keeps its folder through the
            // restore's window too, not only its own, long past.
            transaction
                .execute(
                    "UPDATE archives SET free_after_unix_ms = MAX(free_after_unix_ms, ?2)
                     WHERE state = 1 AND session_id IN (SELECT id FROM sessions WHERE channel = ?1)",
                    rusqlite::params![channel.id, free_after_unix_ms],
                )
                .map_err(&failed)?;
            Ok(moved)
        })?;
        remove_side(side)?;
        Ok(moved)
    }

    /// Renames channel `id` and moves it to `placement`'s folder in the
    /// records, in one transaction: its name and folder, and the folders,
    /// clones, launches and previous conversations of its sessions and of
    /// its Desk sessions, closed ones too. `expected_revision` is checked
    /// as a rename's is. The conversation files that go with the folder
    /// are [`Store::conversation_moves`]'. `on` set moves only that
    /// machine's sessions: the others follow with
    /// [`Store::move_machine_sessions`] once their machine moved its folder.
    pub fn move_channel(
        &self,
        id: &str,
        on: Option<&str>,
        placement: &Placement,
        expected_revision: Option<u64>,
    ) -> Result<ChannelRecord, StoreError> {
        check_id(id)?;
        let doing = "rename a channel and move its folder";
        self.inner().write_with("channel.move", doing, |transaction| {
            let failed = self.inner().failed(doing);
            let found = transaction
                .query_row("SELECT revision FROM channels WHERE id = ?1", [id], |row| row.get::<_, i64>(0))
                .optional()
                .map_err(&failed)?;
            let Some(revision) = found.map(from_sql) else {
                return Err(StoreError::NoChannel { id: id.to_owned() });
            };
            if let Some(expected) = expected_revision
                && expected != revision
            {
                return Err(StoreError::StaleChannel {
                    id: id.to_owned(),
                    expected,
                    actual: revision,
                });
            }
            transaction
                .execute(
                    "UPDATE channels SET name = ?2, folder = ?3, revision = ?4 WHERE id = ?1",
                    params![id, placement.name, placement.folder, to_sql(revision + 1)],
                )
                .map_err(&failed)?;
            rebase_rows(self.inner(), transaction, id, on, placement, true)?;
            Ok(())
        })?;
        self.channel(id)?.ok_or_else(|| StoreError::NoChannel { id: id.to_owned() })
    }

    /// The conversation files that move when `placement` is applied to
    /// channel `id`: where each is, and where its session's launch will
    /// name it. Changes nothing, so it can be asked before the folder moves.
    pub fn conversation_moves(
        &self,
        id: &str,
        on: Option<&str>,
        placement: &Placement,
    ) -> Result<Vec<ConversationMove>, StoreError> {
        check_id(id)?;
        self.inner()
            .read_with(|connection| rebase_rows(self.inner(), connection, id, on, placement, false))
    }

    /// D348: points the records of `machine`'s sessions of channel `id` at
    /// `placement`'s new folder, once that machine moved its own folder.
    /// The channel's own record is left as it is.
    pub fn move_machine_sessions(&self, id: &str, machine: &str, placement: &Placement) -> Result<(), StoreError> {
        check_id(id)?;
        self.inner().write_with(
            "channel.move",
            "point a machine's sessions at its channel's new folder",
            |transaction| rebase_rows(self.inner(), transaction, id, Some(machine), placement, true).map(|_| ()),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::{
        BoardKey, ChannelRecord, ChannelRepo, MainClone, NewBoardItem, SavedRules, SessionDetail, SessionRecord, SessionRepo,
        StartStep, TabKind, TabRecord,
    };

    fn channel(id: &str, folder: &str) -> ChannelRecord {
        ChannelRecord {
            id: id.to_owned(),
            name: "Payments".to_owned(),
            folder: folder.to_owned(),
            branch_prefix: None,
            default_agent: "claude".to_owned(),
            repos: vec![ChannelRepo {
                owner: "acme".to_owned(),
                name: "api".to_owned(),
                main: MainClone::Ready,
            }],
            machines: vec!["ma_1".to_owned()],
            revision: 1,
            created_unix_ms: 1,
        }
    }

    fn session(id: &str, channel: &str, folder: &Path) -> SessionRecord {
        SessionRecord {
            id: id.to_owned(),
            channel: channel.to_owned(),
            machine: "ma_1".to_owned(),
            title: "Fix".to_owned(),
            agent: "claude".to_owned(),
            folder: folder.to_owned(),
            created_unix_ms: 1,
        }
    }

    /// A store with channel `ch_1` at `root/payments` holding one session
    /// with its detail, a tab, a board item and rules, and channel `ch_2`
    /// with a session of its own.
    fn filled(root: &Path) -> Store {
        let store = Store::open(root.join("state")).unwrap();
        let folder = root.join("payments");
        store.put_channel(&channel("ch_1", "payments")).unwrap();
        store.put_channel(&channel("ch_2", "other")).unwrap();
        store.put_session(&session("se_1", "ch_1", &folder.join("sessions/fix"))).unwrap();
        store.put_session(&session("se_2", "ch_2", &root.join("other/sessions/x"))).unwrap();
        store
            .put_session_detail(&SessionDetail {
                session: "se_1".to_owned(),
                repos: vec![SessionRepo {
                    owner: "acme".to_owned(),
                    name: "api".to_owned(),
                    branch: "fix".to_owned(),
                    path: folder.join("sessions/fix/api"),
                }],
                model: None,
                first_message: Some("hello".to_owned()),
                start: StartStep::Done,
                operation: None,
                revision: 1,
            })
            .unwrap();
        store
            .put_tab(&TabRecord {
                id: "tab_1".to_owned(),
                session: Some("se_1".to_owned()),
                kind: TabKind::Shell,
                title: "sh".to_owned(),
                holder_socket: PathBuf::from("/tmp/h"),
                holder_pid: 7,
                created_unix_ms: 1,
                holder_generation: 0,
            })
            .unwrap();
        store.claim_board_prefix("ch_1", "PAY").unwrap();
        store
            .create_board_item(&NewBoardItem {
                board: BoardKey::Channel("ch_1".to_owned()),
                channel: "ch_1".to_owned(),
                title: "Refunds".to_owned(),
                description: String::new(),
                column: 1,
                rank: "m".to_owned(),
                created_by: String::new(),
                at_unix_ms: 1,
                sessions: vec!["se_1".to_owned()],
                depends_on: Vec::new(),
            })
            .unwrap();
        store
            .put_channel_rules("ch_1", &SavedRules { revision: 1, rules: vec![1, 2, 3] })
            .unwrap();
        store
    }

    fn placement(name: &str, folder: &str, old: &Path, new: &Path) -> Placement {
        Placement {
            name: name.to_owned(),
            folder: folder.to_owned(),
            old_path: old.to_owned(),
            new_path: new.to_owned(),
            now_unix_ms: 99,
        }
    }

    #[test]
    fn every_table_is_copied_or_global() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("state")).unwrap();
        let tables: BTreeSet<String> = store
            .inner()
            .read("list the tables", |connection| {
                connection
                    .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")?
                    .query_map([], |row| row.get::<_, String>(0))?
                    .collect()
            })
            .unwrap();
        let known: BTreeSet<String> = COPIED
            .iter()
            .map(|table| table.name)
            .chain(GLOBAL.iter().copied())
            .map(str::to_owned)
            .collect();
        assert_eq!(tables, known, "a table was added to the schema: say whether a deleted channel copies it or it is global");
        let copied: BTreeSet<&str> = COPIED.iter().map(|table| table.name).collect();
        assert!(GLOBAL.iter().all(|name| !copied.contains(name)));
    }

    #[test]
    fn deleting_and_bringing_back_returns_the_channel_with_its_sessions_archived() {
        let dir = tempfile::tempdir().unwrap();
        let store = filled(dir.path());
        let folder = dir.path().join("payments");
        fs::create_dir_all(&folder).unwrap();
        let side = folder.join(SIDE_FILE);

        store.export_channel("ch_1", &side).unwrap();
        let gone = store.forget_channel("ch_1").unwrap();

        assert_eq!(gone, ["se_1"]);
        assert!(store.channel("ch_1").unwrap().is_none());
        assert!(store.session("se_1").unwrap().is_none());
        assert!(store.tabs().unwrap().is_empty());
        assert!(store.board_items_in_channel("ch_1").unwrap().is_empty());
        assert!(store.channel_rules().unwrap().is_empty());
        assert!(store.channel("ch_2").unwrap().is_some(), "another channel is untouched");
        assert!(store.session("se_2").unwrap().is_some());
        assert_eq!(store.board_prefix("ch_1").unwrap().as_deref(), Some("PAY"), "prefixes stay");
        assert_eq!(Store::deleted_channel(&side).unwrap().id, "ch_1");

        store
            .import_channel(&side, &placement("Payments", "payments", &folder, &folder), 77)
            .unwrap();

        assert_eq!(store.channel("ch_1").unwrap().unwrap().folder, "payments");
        let back = store.session("se_1").unwrap().unwrap();
        assert_eq!(back.folder, folder.join("sessions/fix"));
        assert_eq!(store.session_detail("se_1").unwrap().unwrap().first_message.as_deref(), Some("hello"));
        let archive = store.archive("se_1").unwrap().unwrap();
        assert_eq!(archive.state, crate::ArchiveState::Archived);
        assert_eq!(archive.free_after_unix_ms, 77, "its folder is kept until the restore's window closes");
        assert!(store.tabs().unwrap().is_empty(), "terminals do not come back");
        let items = store.board_items_in_channel("ch_1").unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].sessions, ["se_1"]);
        assert_eq!(store.channel_rules().unwrap().len(), 1);
        assert!(!side.exists(), "the side file is used up");
        assert!(store.archive("se_2").unwrap().is_none());
    }

    #[test]
    fn a_channel_that_comes_back_elsewhere_moves_its_sessions_paths() {
        let dir = tempfile::tempdir().unwrap();
        let store = filled(dir.path());
        let old = dir.path().join("payments");
        let new = dir.path().join("payments-2");
        fs::create_dir_all(&new).unwrap();
        let side = new.join(SIDE_FILE);
        store.export_channel("ch_1", &side).unwrap();
        store.forget_channel("ch_1").unwrap();

        store
            .import_channel(&side, &placement("Payments-2", "payments-2", &old, &new), 0)
            .unwrap();

        let back = store.channel("ch_1").unwrap().unwrap();
        assert_eq!((back.name.as_str(), back.folder.as_str()), ("Payments-2", "payments-2"));
        assert_eq!(store.session("se_1").unwrap().unwrap().folder, new.join("sessions/fix"));
        assert_eq!(
            store.session_detail("se_1").unwrap().unwrap().repos[0].path,
            new.join("sessions/fix/api")
        );
    }

    /// How an agent that keeps its conversations by folder names `folder`.
    fn dashed(folder: &Path) -> String {
        humata2_conversation::claude_project_name(folder)
    }

    /// Folder `sessions/ddd...` under `at`, with a path of `units`
    /// characters.
    fn sub_of(at: &Path, units: usize) -> String {
        let used = at.to_string_lossy().len() + "/sessions/".len();
        format!("sessions/{}", "d".repeat(units.saturating_sub(used)))
    }

    /// Renames `old_name` to `new_name` with an agent in a subfolder whose
    /// path is `units` long under `at_name`'s folder: its conversation
    /// file and arguments move to the new path's own key, and a sibling
    /// folder's key is left alone.
    fn renames_a_subfolder(old_name: &str, new_name: &str, at_name: &str, units: usize, long: (bool, bool)) {
        let dir = tempfile::tempdir().unwrap();
        let store = filled(dir.path());
        let (old, new) = (dir.path().join(old_name), dir.path().join(new_name));
        let sub = sub_of(&dir.path().join(at_name), units);
        let conv = dir.path().join("conv");
        assert_eq!((dashed(&old.join(&sub)).len() > 200, dashed(&new.join(&sub)).len() > 200), long);
        store.put_agent(&agent("se_1", &old.join(&sub), &conv)).unwrap();
        let placement = placement("Billing", "billing", &old, &new);

        let listed = store.conversation_moves("ch_1", None, &placement).unwrap();
        store.move_channel("ch_1", None, &placement, None).unwrap();

        let was = conv.join(dashed(&old.join(&sub))).join("c1.jsonl");
        let now = conv.join(dashed(&new.join(&sub))).join("c1.jsonl");
        assert_eq!(listed, vec![(was, now.clone())]);
        let launch = store.agent("se_1").unwrap().unwrap().launch;
        assert_eq!(launch, agent("se_1", &new.join(&sub), &conv).launch);
        assert_eq!(launch.conversation_file, now);

        let sibling = dir.path().join(format!("{old_name}-2")).join(&sub);
        let text = format!("{} {}", dashed(&old.join(&sub)), dashed(&sibling));
        let among = placement_of(&old, &new).rewritten_among(&text, &[old.join(&sub), sibling.clone()]);
        assert_eq!(among, Some(format!("{} {}", dashed(&new.join(&sub)), dashed(&sibling))));
    }

    fn placement_of(old: &Path, new: &Path) -> Placement {
        placement("Billing", "billing", old, new)
    }

    #[test]
    fn a_subfolder_whose_key_is_cut_on_both_sides_gets_the_new_paths_own_key() {
        renames_a_subfolder("payments", "billing", "payments", 260, (true, true));
    }

    #[test]
    fn a_subfolder_whose_key_grows_past_the_cut_gets_the_new_paths_own_key() {
        renames_a_subfolder("payments", "payments-and-billing", "payments", 200, (false, true));
    }

    #[test]
    fn a_subfolder_whose_key_shrinks_under_the_cut_gets_the_new_paths_own_key() {
        renames_a_subfolder("payments-and-billing", "payments", "payments", 200, (true, false));
    }

    #[test]
    fn a_short_subfolder_key_keeps_the_prefix_rewrite() {
        renames_a_subfolder("payments", "billing", "payments", 40, (false, false));
        let (old, new) = (Path::new("/r/payments"), Path::new("/r/billing"));
        let moved = placement_of(old, new);
        let sibling = dashed(Path::new("/r/payments-2/sessions/x"));

        assert_eq!(
            moved.rewritten_among(&format!("{} {sibling}", dashed(Path::new("/r/payments/sessions/x"))), &[]),
            Some(format!("{} {sibling}", dashed(Path::new("/r/billing/sessions/x"))))
        );
    }

    /// A folder under `old`'s session `fix`, two levels below its clone
    /// `api`, with a path of at least `units` characters.
    fn deep_folder(old: &Path, units: usize) -> PathBuf {
        let base = old.join("sessions/fix/api/src/pkg");
        let used = base.to_string_lossy().len() + 1;
        base.join("d".repeat(units.saturating_sub(used).max(1)))
    }

    /// Renames `old_name` to `new_name` with a Claude agent (and one
    /// previous conversation) in a folder two levels under its clone: both
    /// conversations move to the new path's own key, and the arguments
    /// follow.
    fn renames_a_deep_folder(old_name: &str, new_name: &str, units: usize, long: (bool, bool)) {
        let dir = tempfile::tempdir().unwrap();
        let store = filled(dir.path());
        let (old, new) = (dir.path().join(old_name), dir.path().join(new_name));
        let deep = deep_folder(&old, units);
        let now_deep = new.join(deep.strip_prefix(&old).unwrap());
        let conv = dir.path().join("conv");
        assert_eq!((dashed(&deep).len() > 200, dashed(&now_deep).len() > 200), long);
        store.put_agent(&agent("se_1", &deep, &conv)).unwrap();
        let earlier = conv.join(dashed(&deep)).join("c0.jsonl");
        store.push_previous_conversation("se_1", "c0", &earlier, 3).unwrap();
        let placement = placement("Billing", "billing", &old, &new);

        let listed = store.conversation_moves("ch_1", None, &placement).unwrap();
        store.move_channel("ch_1", None, &placement, None).unwrap();

        let (was, now) = (conv.join(dashed(&deep)), conv.join(dashed(&now_deep)));
        let mut want = vec![(was.join("c1.jsonl"), now.join("c1.jsonl")), (was.join("c0.jsonl"), now.join("c0.jsonl"))];
        want.sort();
        assert_eq!(listed, want);
        assert_eq!(store.agent("se_1").unwrap().unwrap().launch, agent("se_1", &now_deep, &conv).launch);
        assert_eq!(store.previous_conversations("se_1").unwrap()[0].path, now.join("c0.jsonl"));
    }

    #[test]
    fn a_session_two_levels_under_its_clone_with_a_cut_key_keeps_its_conversations() {
        renames_a_deep_folder("payments", "billing", 300, (true, true));
    }

    #[test]
    fn a_session_two_levels_under_its_clone_whose_key_grows_past_the_cut_keeps_its_conversations() {
        renames_a_deep_folder("payments", "payments-and-billing", 200, (false, true));
    }

    #[test]
    fn a_session_two_levels_under_its_clone_whose_key_shrinks_under_the_cut_keeps_its_conversations() {
        // Over 200 units before and at most 200 after: the old name is 12
        // longer.
        renames_a_deep_folder("payments-and-billing", "payments", 205, (true, false));
    }

    #[test]
    fn a_session_two_levels_under_its_clone_with_a_short_key_keeps_its_conversations() {
        renames_a_deep_folder("payments", "billing", 40, (false, false));
    }

    /// A conversation key of a deep folder that no record names is made
    /// exact from the move that carries its file.
    #[test]
    fn a_cut_key_of_a_folder_no_record_names_is_made_exact_from_the_move_that_names_it() {
        let (old, new) = (Path::new("/r/payments"), Path::new("/r/billing"));
        let deep = |root: &str| Path::new(root).join(format!("sessions/fix/api/src/{}", "d".repeat(240)));
        let (was, now) = (dashed(&deep("/r/payments")), dashed(&deep("/r/billing")));
        let sibling = dashed(&deep("/r/payments-2"));
        assert!(was.len() > 200 && now.len() > 200);
        let moves = vec![(Path::new("/c").join(&was).join("c0.jsonl"), Path::new("/c").join(&now).join("c0.jsonl"))];

        let moved = placement_of(old, new).rewritten_moved(&format!("{was} {sibling}"), &[], &moves);

        assert_eq!(moved, Some(format!("{now} {sibling}")));
    }

    /// The agent of `session` in `folder`, keeping conversation `c1` by
    /// folder under `conv`, as `kind.launch` fills it in.
    fn agent(session: &str, folder: &Path, conv: &Path) -> crate::AgentRecord {
        let place = folder.display().to_string();
        let key = dashed(folder);
        let argv = |mode: &str| -> Vec<std::ffi::OsString> {
            ["/bin/agent", mode, "c1", place.as_str(), key.as_str()]
                .into_iter()
                .map(Into::into)
                .collect()
        };
        crate::AgentRecord {
            session: session.to_owned(),
            tab: format!("tab_{session}"),
            launch: crate::AgentLaunch {
                agent: "claude".to_owned(),
                folder: folder.to_owned(),
                argv: argv("--session-id"),
                resume_argv: argv("--resume"),
                variable_names: Vec::new(),
                conversation_id: "c1".to_owned(),
                conversation_file: conv.join(dashed(folder)).join("c1.jsonl"),
                holder_socket: PathBuf::from("/tmp/h"),
                cols: 80,
                rows: 24,
                cli_version: None,
            },
            state: crate::AgentState::Stopped,
            generation: 1,
            interruptions: Vec::new(),
            updated_unix_ms: 1,
        }
    }

    /// Channel `channel`'s Desk row for whole-channel Desk session
    /// `session`, closed into trash item `trash`.
    fn desk_row(session: &str, channel: &str, trash: &str) -> crate::ChannelDeskRow {
        crate::ChannelDeskRow {
            session_id: session.to_owned(),
            channel: channel.to_owned(),
            target_session: None,
            read_files: true,
            send_notes: true,
            type_into_terminal: false,
            start_and_fork: false,
            stop_and_archive: false,
            change_board_items: false,
            last_used_unix_ms: 5,
            closed_unix_ms: Some(6),
            target_gone: false,
            trash_item: Some(trash.to_owned()),
        }
    }

    /// MV-16, D314: a channel that comes back at `<old>-2` brings its
    /// agents' launches and its previous conversations with it: folder,
    /// arguments and conversation file all name the new place.
    #[test]
    fn a_channel_that_comes_back_elsewhere_moves_its_agents_and_previous_conversations() {
        let dir = tempfile::tempdir().unwrap();
        let store = filled(dir.path());
        let old = dir.path().join("payments");
        let new = dir.path().join("payments-2");
        let conv = dir.path().join("conv");
        store.put_agent(&agent("se_1", &old.join("sessions/fix"), &conv)).unwrap();
        let before = conv.join(dashed(&old.join("sessions/fix"))).join("c0.jsonl");
        store.push_previous_conversation("se_1", "c0", &before, 3).unwrap();
        fs::create_dir_all(&new).unwrap();
        let side = new.join(SIDE_FILE);
        store.export_channel("ch_1", &side).unwrap();
        store.forget_channel("ch_1").unwrap();

        store
            .import_channel(&side, &placement("Payments-2", "payments-2", &old, &new), 0)
            .unwrap();

        let moved = new.join("sessions/fix");
        let want = agent("se_1", &moved, &conv).launch;
        let launch = store.agent("se_1").unwrap().expect("the agent comes back").launch;
        assert_eq!(launch.folder, want.folder);
        assert_eq!(launch.conversation_file, want.conversation_file);
        assert_eq!(launch.argv, want.argv);
        assert_eq!(launch.resume_argv, want.resume_argv);
        let previous = store.previous_conversations("se_1").unwrap();
        assert_eq!(previous.len(), 1);
        assert_eq!(previous[0].conversation_id, "c0");
        assert_eq!(previous[0].path, conv.join(dashed(&moved)).join("c0.jsonl"));
    }

    /// MV-16, CH-24: a channel's Desk sessions come back with it under its
    /// new folder: their records, their rows (still the channel's, still in
    /// the trash) and their launches; another channel's are left alone.
    #[test]
    fn a_channels_desk_sessions_come_back_with_it_under_its_new_folder() {
        let dir = tempfile::tempdir().unwrap();
        let store = filled(dir.path());
        let old = dir.path().join("payments");
        let new = dir.path().join("payments-2");
        let conv = dir.path().join("conv");
        let other = dir.path().join("other/desk/x");
        store.put_session(&session("se_3", "desk", &old.join("desk/plan"))).unwrap();
        store.put_channel_desk(&desk_row("se_3", "ch_1", "tr_1")).unwrap();
        store.put_agent(&agent("se_3", &old.join("desk/plan"), &conv)).unwrap();
        store.put_session(&session("se_4", "desk", &other)).unwrap();
        store.put_channel_desk(&desk_row("se_4", "ch_2", "tr_2")).unwrap();
        store.put_agent(&agent("se_4", &other, &conv)).unwrap();
        fs::create_dir_all(&new).unwrap();
        let side = new.join(SIDE_FILE);
        store.export_channel("ch_1", &side).unwrap();
        store.forget_channel("ch_1").unwrap();

        store
            .import_channel(&side, &placement("Payments-2", "payments-2", &old, &new), 0)
            .unwrap();

        let back = store.session("se_3").unwrap().expect("the Desk session's record is there");
        assert_eq!(back.folder, new.join("desk/plan"));
        assert_eq!(back.channel, "desk");
        assert_eq!(store.channel_desk("se_3").unwrap(), Some(desk_row("se_3", "ch_1", "tr_1")));
        let launch = store.agent("se_3").unwrap().expect("its agent is there").launch;
        assert_eq!(launch, agent("se_3", &new.join("desk/plan"), &conv).launch);
        assert_eq!(store.session("se_4").unwrap().unwrap().folder, other, "another channel's is untouched");
        assert_eq!(store.channel_desk("se_4").unwrap(), Some(desk_row("se_4", "ch_2", "tr_2")));
        assert_eq!(store.agent("se_4").unwrap().unwrap().launch, agent("se_4", &other, &conv).launch);
    }

    /// D314: renaming a channel points its sessions' and its Desk sessions'
    /// records at the new folder in one step, and names the conversation
    /// files that follow.
    #[test]
    fn moving_a_channel_points_its_records_at_its_new_folder_and_lists_the_conversations() {
        let dir = tempfile::tempdir().unwrap();
        let store = filled(dir.path());
        let old = dir.path().join("payments");
        let new = dir.path().join("billing");
        let conv = dir.path().join("conv");
        store.put_agent(&agent("se_1", &old.join("sessions/fix"), &conv)).unwrap();
        store.put_session(&session("se_3", "desk", &old.join("desk/plan"))).unwrap();
        store.put_channel_desk(&desk_row("se_3", "ch_1", "tr_1")).unwrap();
        store.put_agent(&agent("se_3", &old.join("desk/plan"), &conv)).unwrap();
        let placement = placement("Billing", "billing", &old, &new);

        let listed = store.conversation_moves("ch_1", None, &placement).unwrap();
        let moved = store.move_channel("ch_1", None, &placement, Some(1)).unwrap();

        assert_eq!((moved.name.as_str(), moved.folder.as_str()), ("Billing", "billing"));
        assert_eq!(moved.revision, 2);
        let key = |folder: &str| conv.join(dashed(&new.join(folder))).join("c1.jsonl");
        assert_eq!(store.session("se_1").unwrap().unwrap().folder, new.join("sessions/fix"));
        assert_eq!(store.session_detail("se_1").unwrap().unwrap().repos[0].path, new.join("sessions/fix/api"));
        assert_eq!(store.agent("se_1").unwrap().unwrap().launch, agent("se_1", &new.join("sessions/fix"), &conv).launch);
        assert_eq!(store.session("se_3").unwrap().unwrap().folder, new.join("desk/plan"));
        assert_eq!(store.agent("se_3").unwrap().unwrap().launch.conversation_file, key("desk/plan"));
        assert_eq!(listed.len(), 2);
        let was = conv.join(dashed(&old.join("sessions/fix"))).join("c1.jsonl");
        assert!(listed.contains(&(was, key("sessions/fix"))), "{listed:?}");
        assert_eq!(store.session("se_2").unwrap().unwrap().folder, dir.path().join("other/sessions/x"));
    }

    /// D348: a rename moves one machine's records; another machine's keep
    /// their paths until that machine moved its folder.
    #[test]
    fn moving_a_channel_leaves_another_machine_s_sessions_until_that_machine_moves() {
        let dir = tempfile::tempdir().unwrap();
        let store = filled(dir.path());
        let (old, new) = (dir.path().join("payments"), dir.path().join("billing"));
        let far = SessionRecord {
            machine: "ma_2".to_owned(),
            ..session("se_4", "ch_1", &old.join("sessions/far"))
        };
        store.put_session(&far).unwrap();
        let placement = placement("Billing", "billing", &old, &new);

        store.move_channel("ch_1", Some("ma_1"), &placement, Some(1)).unwrap();

        assert_eq!(store.session("se_1").unwrap().unwrap().folder, new.join("sessions/fix"));
        assert_eq!(store.session("se_4").unwrap().unwrap().folder, old.join("sessions/far"));

        store.move_machine_sessions("ch_1", "ma_2", &placement).unwrap();

        assert_eq!(store.session("se_4").unwrap().unwrap().folder, new.join("sessions/far"));
    }

    /// D314: a rename is refused, and changes nothing, against another
    /// revision than the channel's.
    #[test]
    fn moving_a_channel_against_another_revision_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let store = filled(dir.path());
        let (old, new) = (dir.path().join("payments"), dir.path().join("billing"));

        let refused = store.move_channel("ch_1", None, &placement("Billing", "billing", &old, &new), Some(7));

        assert!(matches!(refused, Err(StoreError::StaleChannel { .. })), "{refused:?}");
        let kept = store.channel("ch_1").unwrap().unwrap();
        assert_eq!((kept.name.as_str(), kept.folder.as_str(), kept.revision), ("Payments", "payments", 1));
        assert_eq!(store.session("se_1").unwrap().unwrap().folder, old.join("sessions/fix"));
    }

    /// D314: a folder that only starts like the old one is not moved.
    #[test]
    fn a_folder_that_only_starts_like_the_old_one_is_left_alone() {
        let (old, new) = (Path::new("/r/payments"), Path::new("/r/billing"));
        let moved = placement("Billing", "billing", old, new);

        assert_eq!(moved.rewritten("cd /r/payments/sessions/x").as_deref(), Some("cd /r/billing/sessions/x"));
        assert_eq!(moved.rewritten("cd /r/payments-2/sessions/x"), None);
        assert_eq!(
            moved.rewritten("-r-payments-sessions-x -r-payments-2-sessions-x").as_deref(),
            Some("-r-billing-sessions-x -r-payments-2-sessions-x")
        );
    }

    #[test]
    fn an_empty_side_file_holds_no_channel() {
        let dir = tempfile::tempdir().unwrap();
        let side = dir.path().join(SIDE_FILE);
        drop(schema::open(&side).unwrap());

        let found = Store::deleted_channel(&side);

        assert!(matches!(found, Err(StoreError::NoDeletedChannel { .. })), "{found:?}");
    }
}
