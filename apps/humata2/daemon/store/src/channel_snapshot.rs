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

use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, params_from_iter};

use crate::store::{bytes_path, check_id, path_bytes};
use crate::{Store, StoreError, schema};

/// The side file's name inside the channel's folder.
pub const SIDE_FILE: &str = ".humata-deleted.sqlite";

const IN_CHANNEL: &str = "session IN (SELECT id FROM sessions WHERE channel = ?1)";
const IN_CHANNEL_ID: &str = "session_id IN (SELECT id FROM sessions WHERE channel = ?1)";
const OF_CHANNEL: &str = "channel = ?1";
const ITEMS: &str = "item IN (SELECT key FROM board_items WHERE channel = ?1)";

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
    Copied {
        restored: false,
        ..copied("tabs", IN_CHANNEL)
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

/// `path` under `new` instead of `old`, when it was under `old`.
fn rebased(path: &Path, old: &Path, new: &Path) -> Option<PathBuf> {
    path.strip_prefix(old).ok().map(|rest| new.join(rest))
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
    pub fn import_channel(&self, side: &Path, placement: &Placement) -> Result<(), StoreError> {
        let source = schema::open(side)?;
        let mut tables = Vec::new();
        for table in COPIED.iter().filter(|table| table.restored) {
            let (names, rows) = rows_of(&source, table.name, None).map_err(side_error(side, "read the deleted channel"))?;
            tables.push((table.name, names, rows));
        }
        let channel = Self::deleted_channel(side)?;
        drop(source);
        let doing = "bring a deleted channel back";
        self.inner().write_with("channel.import", doing, |transaction| {
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
            if placement.old_path != placement.new_path {
                let folders = transaction
                    .prepare("SELECT id, folder FROM sessions WHERE channel = ?1")
                    .and_then(|mut statement| {
                        statement
                            .query_map([&channel.id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?)))?
                            .collect::<rusqlite::Result<Vec<_>>>()
                    })
                    .map_err(&failed)?;
                for (id, folder) in folders {
                    if let Some(moved) = rebased(&bytes_path(folder), &placement.old_path, &placement.new_path) {
                        transaction
                            .execute("UPDATE sessions SET folder = ?2 WHERE id = ?1", rusqlite::params![id, path_bytes(&moved)])
                            .map_err(&failed)?;
                    }
                }
                let paths = transaction
                    .prepare("SELECT session, owner, name, path FROM session_repos WHERE session IN (SELECT id FROM sessions WHERE channel = ?1)")
                    .and_then(|mut statement| {
                        statement
                            .query_map([&channel.id], |row| {
                                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, Vec<u8>>(3)?))
                            })?
                            .collect::<rusqlite::Result<Vec<_>>>()
                    })
                    .map_err(&failed)?;
                for (session, owner, name, path) in paths {
                    if let Some(moved) = rebased(&bytes_path(path), &placement.old_path, &placement.new_path) {
                        transaction
                            .execute(
                                "UPDATE session_repos SET path = ?4 WHERE session = ?1 AND owner = ?2 AND name = ?3",
                                rusqlite::params![session, owner, name, path_bytes(&moved)],
                            )
                            .map_err(&failed)?;
                    }
                }
            }
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
                     SELECT id, ?2, 1, X'', ?2 FROM sessions WHERE channel = ?1",
                    rusqlite::params![channel.id, placement.now_unix_ms],
                )
                .map_err(&failed)?;
            Ok(())
        })?;
        remove_side(side)
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
            .import_channel(&side, &placement("Payments", "payments", &folder, &folder))
            .unwrap();

        assert_eq!(store.channel("ch_1").unwrap().unwrap().folder, "payments");
        let back = store.session("se_1").unwrap().unwrap();
        assert_eq!(back.folder, folder.join("sessions/fix"));
        assert_eq!(store.session_detail("se_1").unwrap().unwrap().first_message.as_deref(), Some("hello"));
        let archive = store.archive("se_1").unwrap().unwrap();
        assert_eq!(archive.state, crate::ArchiveState::Archived);
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
            .import_channel(&side, &placement("Payments-2", "payments-2", &old, &new))
            .unwrap();

        let back = store.channel("ch_1").unwrap().unwrap();
        assert_eq!((back.name.as_str(), back.folder.as_str()), ("Payments-2", "payments-2"));
        assert_eq!(store.session("se_1").unwrap().unwrap().folder, new.join("sessions/fix"));
        assert_eq!(
            store.session_detail("se_1").unwrap().unwrap().repos[0].path,
            new.join("sessions/fix/api")
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
