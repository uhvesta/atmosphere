//! The store's last good copy (PER-31): a `VACUUM INTO` snapshot, checked
//! before it replaces the one before it, so a database damaged by a disk
//! fault or a bug is restored rather than lost, with what changed since
//! the copy listed.
//!
//! The blob files rows name are kept while the copy names them too (see
//! [`copy_files`]), so a restored row's file is still there.

use std::collections::{BTreeMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::ErrorKind;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OpenFlags};

use crate::StoreError;
use crate::atomic::create_dir;
use crate::schema::SCHEMA_VERSION;

/// The copy's name in the state folder.
pub(crate) const LAST_GOOD: &str = "last-good.sqlite";

static COPIES: AtomicU64 = AtomicU64::new(0);

/// A last good copy on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LastGood {
    pub path: PathBuf,
    pub taken_unix_ms: i64,
}

/// What restoring the last good copy brought back and what it lost.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Restored {
    /// When the copy now in use was taken.
    pub copy_taken_unix_ms: i64,
    /// Where the damaged database was moved, kept for a closer look.
    pub damaged_kept_at: PathBuf,
    /// What SQLite said was wrong with it.
    pub damage: String,
    /// Rows that differ between the damaged database and the copy, by
    /// table and primary key.
    pub changes: Vec<RowChange>,
    /// Tables of the damaged database that couldn't be read, so what they
    /// held since the copy isn't known.
    pub unreadable_tables: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowChange {
    pub table: String,
    /// The row's primary key, as SQL literals joined by commas.
    pub key: String,
    pub change: Change,
}

/// How a row differs from the copy, seen from the damaged database.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// Written after the copy: gone now.
    AddedSinceCopy,
    /// Changed after the copy: back to the copy's version.
    ChangedSinceCopy,
    /// Removed after the copy: back again.
    RemovedSinceCopy,
}

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| i64::try_from(since.as_millis()).unwrap_or(i64::MAX))
}

fn mtime_unix_ms(path: &Path) -> Option<i64> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    let since = modified.duration_since(UNIX_EPOCH).ok()?;
    i64::try_from(since.as_millis()).ok()
}

/// Writes a snapshot of `connection`'s database to `root/last-good.sqlite`:
/// to a temp file, checked whole, fsynced, renamed over the one before,
/// and the folder fsynced. A copy that doesn't check out is removed and the
/// one before kept.
pub(crate) fn save(connection: &Connection, root: &Path) -> Result<LastGood, StoreError> {
    let path = root.join(LAST_GOOD);
    let temp = root.join(format!(
        ".{LAST_GOOD}.{}-{}.partial",
        std::process::id(),
        COPIES.fetch_add(1, Ordering::Relaxed)
    ));
    let saved = save_to(connection, &temp, &path, root);
    if saved.is_err() {
        let _ = humata2_durable::remove_file(&temp);
    }
    saved?;
    Ok(LastGood {
        taken_unix_ms: mtime_unix_ms(&path).unwrap_or_else(now_unix_ms),
        path,
    })
}

fn save_to(connection: &Connection, temp: &Path, path: &Path, root: &Path) -> Result<(), StoreError> {
    let Some(temp_text) = temp.to_str() else {
        return Err(StoreError::Snapshot {
            path: path.to_owned(),
            problem: "the state folder's path isn't UTF-8, which VACUUM INTO needs".to_owned(),
        });
    };
    connection
        .execute("VACUUM INTO ?1", [temp_text])
        .map_err(|source| StoreError::Database {
            path: path.to_owned(),
            doing: "copy the database",
            source,
        })?;
    let problem = check(temp);
    if let Some(problem) = problem {
        return Err(StoreError::Snapshot {
            path: path.to_owned(),
            problem,
        });
    }
    let written = (|| {
        let mut options = OpenOptions::new();
        options.read(true).write(true).mode(0o600);
        humata2_durable::File::open(&options, temp)?.sync_all()?;
        humata2_durable::rename(temp, path)?;
        humata2_durable::sync_dir(root)
    })();
    written.map_err(|source| StoreError::Write {
        path: path.to_owned(),
        source,
    })
}

/// What's wrong with the database at `path`, if anything: SQLite's own
/// check, and a schema this build reads.
fn check(path: &Path) -> Option<String> {
    let opened = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY);
    let connection = match opened {
        Ok(connection) => connection,
        Err(error) => return Some(format!("it doesn't open: {error}")),
    };
    let problems = integrity(&connection, "integrity_check");
    if problems.is_some() {
        return problems;
    }
    match connection.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0)) {
        Ok(SCHEMA_VERSION) => None,
        Ok(found) => Some(format!("its schema is version {found}, not {SCHEMA_VERSION}")),
        Err(error) => Some(format!("its schema version is unreadable: {error}")),
    }
}

/// What `PRAGMA <pragma>` reports wrong, or None when it says "ok".
pub(crate) fn integrity(connection: &Connection, pragma: &str) -> Option<String> {
    let rows = connection
        .prepare(&format!("PRAGMA {pragma}"))
        .and_then(|mut statement| {
            statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<String>>>()
        });
    match rows {
        Ok(rows) if rows == ["ok"] => None,
        Ok(rows) => Some(rows.join("; ")),
        Err(error) => Some(error.to_string()),
    }
}

/// The blob files the last good copy in `root` names, so a sweep keeps
/// them. Empty when there's no copy or it can't be read.
pub(crate) fn copy_files(root: &Path) -> HashSet<String> {
    let path = root.join(LAST_GOOD);
    if !path.exists() {
        return HashSet::new();
    }
    let Ok(connection) = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY) else {
        return HashSet::new();
    };
    connection
        .prepare(
            "SELECT payload_file FROM journal WHERE payload_file IS NOT NULL
             UNION SELECT launch_file FROM agents WHERE launch_file IS NOT NULL",
        )
        .and_then(|mut statement| {
            statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<HashSet<String>>>()
        })
        .unwrap_or_default()
}

/// Whether `error`, from opening the store, says its database is damaged,
/// as opposed to unreachable (permissions, a full disk) or too new.
pub(crate) fn is_damage(error: &StoreError) -> bool {
    let source = match error {
        StoreError::Open { source, .. }
        | StoreError::Migrate { source, .. }
        | StoreError::Database { source, .. } => source,
        StoreError::Damaged { .. } => return true,
        _ => return false,
    };
    matches!(
        source.sqlite_error_code(),
        Some(rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase)
    )
}

/// Moves the damaged database at `database` (and its -wal and -shm) into
/// a folder of its own under `root`, puts the last good copy in its
/// place, and lists what differs. The caller opens the store again.
pub(crate) fn restore(root: &Path, database: &Path, damage: String) -> Result<Restored, StoreError> {
    let copy = root.join(LAST_GOOD);
    let copy_taken_unix_ms = mtime_unix_ms(&copy).unwrap_or(0);
    let kept = root.join(format!("damaged-{}", now_unix_ms()));
    create_dir(&kept)?;
    let moved = |from: &Path, to: &Path| {
        humata2_durable::rename(from, to).or_else(|error| match error.kind() {
            ErrorKind::NotFound => Ok(()),
            _ => Err(StoreError::Write {
                path: from.to_owned(),
                source: error,
            }),
        })
    };
    let name = database.file_name().map_or_else(|| "state.sqlite".into(), ToOwned::to_owned);
    let damaged = kept.join(&name);
    moved(database, &damaged)?;
    for suffix in ["-wal", "-shm"] {
        let mut side = database.as_os_str().to_owned();
        side.push(suffix);
        let mut to = damaged.as_os_str().to_owned();
        to.push(suffix);
        moved(Path::new(&side), Path::new(&to))?;
    }
    let sync = |folder: &Path| {
        humata2_durable::sync_dir(folder).map_err(|source| StoreError::Write {
            path: folder.to_owned(),
            source,
        })
    };
    sync(&kept)?;
    sync(root)?;

    // A copy of the copy, so the copy stays for a database damaged again.
    let temp = root.join(format!(".{}.restore.partial", name.to_string_lossy()));
    let copied = (|| {
        fs::copy(&copy, &temp)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true);
        humata2_durable::File::open(&options, &temp)?.sync_all()?;
        humata2_durable::rename(&temp, database)?;
        humata2_durable::sync_dir(root)
    })();
    if let Err(source) = copied {
        let _ = humata2_durable::remove_file(&temp);
        return Err(StoreError::Write {
            path: database.to_owned(),
            source,
        });
    }
    let (changes, unreadable_tables) = differences(&damaged, &copy);
    Ok(Restored {
        copy_taken_unix_ms,
        damaged_kept_at: damaged,
        damage,
        changes,
        unreadable_tables,
    })
}

/// Each table's rows by primary key, each row as one text of SQL literals.
type Rows = BTreeMap<String, String>;

/// The rows that differ between `damaged` and `copy`, and the damaged
/// database's tables that couldn't be read.
fn differences(damaged: &Path, copy: &Path) -> (Vec<RowChange>, Vec<String>) {
    let open = |path: &Path| Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).ok();
    let Some(copy) = open(copy) else {
        return (Vec::new(), Vec::new());
    };
    let tables = tables(&copy).unwrap_or_default();
    let damaged = open(damaged);
    let mut changes = Vec::new();
    let mut unreadable = Vec::new();
    for table in tables {
        let Ok(before) = rows(&copy, &table) else { continue };
        let after = damaged.as_ref().and_then(|damaged| rows(damaged, &table).ok());
        let Some(after) = after else {
            unreadable.push(table);
            continue;
        };
        let change = |key: &String, change| RowChange {
            table: table.clone(),
            key: key.clone(),
            change,
        };
        for (key, row) in &after {
            match before.get(key) {
                None => changes.push(change(key, Change::AddedSinceCopy)),
                Some(old) if old != row => changes.push(change(key, Change::ChangedSinceCopy)),
                Some(_) => {}
            }
        }
        for key in before.keys().filter(|key| !after.contains_key(*key)) {
            changes.push(change(key, Change::RemovedSinceCopy));
        }
    }
    (changes, unreadable)
}

fn tables(connection: &Connection) -> rusqlite::Result<Vec<String>> {
    connection
        .prepare("SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name")?
        .query_map([], |row| row.get(0))?
        .collect()
}

fn rows(connection: &Connection, table: &str) -> rusqlite::Result<Rows> {
    let quoted = |name: &str| format!("\"{}\"", name.replace('"', "\"\""));
    let mut columns: Vec<(i64, String)> = Vec::new();
    let mut all = Vec::new();
    connection
        .prepare(&format!("PRAGMA table_info({})", quoted(table)))?
        .query_map([], |row| Ok((row.get::<_, String>(1)?, row.get::<_, i64>(5)?)))?
        .try_for_each(|column| {
            let (name, pk) = column?;
            if pk > 0 {
                columns.push((pk, name.clone()));
            }
            all.push(name);
            Ok::<_, rusqlite::Error>(())
        })?;
    columns.sort();
    let literals = |names: Vec<&String>| {
        names
            .into_iter()
            .map(|name| format!("quote({})", quoted(name)))
            .collect::<Vec<_>>()
            .join(" || ',' || ")
    };
    // A table without a declared key is keyed by its rowid.
    let key = if columns.is_empty() {
        "quote(rowid)".to_owned()
    } else {
        literals(columns.iter().map(|(_, name)| name).collect())
    };
    let row = if all.is_empty() { "''".to_owned() } else { literals(all.iter().collect()) };
    connection
        .prepare(&format!("SELECT {key}, {row} FROM {}", quoted(table)))?
        .query_map([], |found| Ok((found.get::<_, String>(0)?, found.get::<_, String>(1)?)))?
        .collect()
}
