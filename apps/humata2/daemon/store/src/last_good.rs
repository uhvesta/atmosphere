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

use crate::atomic::{create_dir, sweep_partials};
use crate::store::{DATABASE, check_id};
use crate::{StoreError, fault};
use crate::schema::SCHEMA_VERSION;

/// The copy's name in the state folder.
pub(crate) const LAST_GOOD: &str = "last-good.sqlite";

/// The folder of pre-upgrade copies, one `<name>.sqlite` each.
const UPGRADE_COPIES: &str = "upgrade-copies";

/// Where the folders that keep a database an upgrade replaced are named:
/// `upgraded-<unix ms>`.
const UPGRADED: &str = "upgraded-";

static COPIES: AtomicU64 = AtomicU64::new(0);

/// The copy of the database an upgrade keeps before it changes the schema (D281g).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpgradeCopy {
    pub path: PathBuf,
    /// Its schema version.
    pub schema: i64,
    pub taken_unix_ms: i64,
}

/// What putting a pre-upgrade copy back did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpgradeRestored {
    /// The schema version put back.
    pub schema: i64,
    /// Where the database it replaced was moved.
    pub replaced_kept_at: PathBuf,
}

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

/// Opens a dedicated read-only connection to `database` and writes a
/// snapshot to `root/last-good.sqlite`, so normal writes on the main
/// connection do not block while `VACUUM INTO`, integrity checking, and
/// syncing run. Callers must hold `maintenance_lock` to serialize with
/// `sweep()`.
pub(crate) fn save_from_path(database: &Path, root: &Path) -> Result<LastGood, StoreError> {
    // A plain open, not read-only: a read-only connection to the live WAL
    // database stalls the rollback relaunch (upgrade_cut_test), and
    // `PRAGMA query_only` refuses the `VACUUM INTO` the copy is made with.
    // This connection only reads `database`; it writes the copy.
    let connection = Connection::open(database).map_err(|source| StoreError::Open {
        path: database.to_owned(),
        source,
    })?;
    connection
        .execute_batch(
            "PRAGMA busy_timeout = 5000;
             PRAGMA cache_size = -2000;",
        )
        .map_err(|source| StoreError::Open {
            path: database.to_owned(),
            source,
        })?;
    save(&connection, root)
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
    let saved = save_to(connection, &temp, &path, root, "store.last_good.copied", SCHEMA_VERSION);
    if saved.is_err() {
        let _ = humata2_durable::remove_file(&temp);
    }
    saved?;
    Ok(LastGood {
        taken_unix_ms: mtime_unix_ms(&path).unwrap_or_else(now_unix_ms),
        path,
    })
}

/// Copies `connection`'s database to `temp`, checks the copy against
/// `expected` as its schema version, and renames it to `path` in `root`.
/// `point` names the fault point a crash can hit just before the rename.
fn save_to(
    connection: &Connection,
    temp: &Path,
    path: &Path,
    root: &Path,
    point: &str,
    expected: i64,
) -> Result<(), StoreError> {
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
    let problem = check(temp, expected);
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
        fault::point(point);
        humata2_durable::rename(temp, path)?;
        humata2_durable::sync_dir(root)
    })();
    written.map_err(|source| StoreError::Write {
        path: path.to_owned(),
        source,
    })
}

/// What's wrong with the database at `path`, if anything: SQLite's own
/// check, and a schema version of `expected`.
fn check(path: &Path, expected: i64) -> Option<String> {
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
        Ok(found) if found == expected => None,
        Ok(found) => Some(format!("its schema is version {found}, not {expected}")),
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

/// The blob files the last good copy and each pre-upgrade copy in `root`
/// name, so a sweep keeps them. A copy in progress ends in `.partial` and
/// isn't read.
pub(crate) fn copy_files(root: &Path) -> HashSet<String> {
    let mut files = database_files(&root.join(LAST_GOOD));
    if let Ok(entries) = fs::read_dir(root.join(UPGRADE_COPIES)) {
        for entry in entries.filter_map(Result::ok) {
            let is_copy = entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.ends_with(".sqlite") && !name.starts_with('.'));
            if is_copy {
                files.extend(database_files(&entry.path()));
            }
        }
    }
    files
}

/// The blob files the database at `path` names. Empty when there's no
/// database or it can't be read.
fn database_files(path: &Path) -> HashSet<String> {
    if !path.exists() {
        return HashSet::new();
    }
    let Ok(connection) = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY) else {
        return HashSet::new();
    };
    // One query per table: a copy taken before a table existed still names
    // the files of the others.
    [
        "SELECT payload_file FROM journal WHERE payload_file IS NOT NULL",
        "SELECT launch_file FROM agents WHERE launch_file IS NOT NULL",
        "SELECT state_file FROM reviews WHERE state_file IS NOT NULL",
        "SELECT view_file FROM views WHERE view_file IS NOT NULL",
        "SELECT entry_file FROM inbox_requests WHERE entry_file IS NOT NULL",
    ]
    .into_iter()
    .flat_map(|query| {
        connection
            .prepare(query)
            .and_then(|mut statement| {
                statement
                    .query_map([], |row| row.get::<_, String>(0))?
                    .collect::<rusqlite::Result<HashSet<String>>>()
            })
            .unwrap_or_default()
    })
    .collect()
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

/// Puts the last good copy in place of the damaged database at
/// `database`, keeping the damaged one (with its -wal and -shm) in a
/// folder of its own under `root`, and lists what differs. The caller
/// opens the store again.
///
/// The copy of the copy is made first, so a crash before the damaged
/// database moves leaves it where it was, to be found damaged again, and
/// a crash after leaves the copy of the copy for [`finish_restore`].
pub(crate) fn restore(root: &Path, database: &Path, damage: String) -> Result<Restored, StoreError> {
    let copy = root.join(LAST_GOOD);
    let name = file_name(database);
    let temp = restore_temp(root, &name);
    copy_of_copy(&copy, &temp, database)?;
    fault::point("store.restore.copied");
    let kept = root.join(format!("{DAMAGED}{}", now_unix_ms()));
    create_dir(&kept)?;
    let damaged = kept.join(&name);
    move_aside(root, database, &damaged)?;
    fault::point("store.restore.moved");
    place(&temp, database, root)?;
    prune_kept(root);
    Ok(restored(&copy, damaged, damage))
}

/// Finishes a restore a crash cut short after the damaged database was
/// moved aside: the database is missing and the copy of the copy is
/// beside it. Anything the damaged one left behind (-wal, -shm) joins it
/// first, since SQLite would otherwise apply it to the restored copy.
/// None when there's no restore to finish.
pub(crate) fn finish_restore(root: &Path, database: &Path) -> Result<Option<Restored>, StoreError> {
    let name = file_name(database);
    let temp = restore_temp(root, &name);
    if database.exists() || !temp.exists() {
        return Ok(None);
    }
    let copy = root.join(LAST_GOOD);
    let kept = match newest_kept(root, DAMAGED) {
        Some(kept) => kept,
        None => {
            let kept = root.join(format!("{DAMAGED}{}", now_unix_ms()));
            create_dir(&kept)?;
            kept
        }
    };
    let damaged = kept.join(&name);
    move_aside(root, database, &damaged)?;
    // Made again: the one there was synced, but making it costs little and
    // leaves no doubt.
    copy_of_copy(&copy, &temp, database)?;
    place(&temp, database, root)?;
    let damage = "a crash cut short restoring the last good copy over a damaged database; the restore was finished"
        .to_owned();
    Ok(Some(restored(&copy, damaged, damage)))
}

impl crate::Store {
    /// Keeps a copy of the database in `root` as `name` before an upgrade
    /// changes its schema, so a failed upgrade can put it back (D281g).
    /// None when there's no database, or it's already at this build's
    /// schema, so nothing needs keeping. A database newer than this build
    /// is refused.
    ///
    /// The store must not be open while this runs: the copy is of the
    /// database as it is on disk.
    pub fn copy_before_upgrade(root: impl AsRef<Path>, name: &str) -> Result<Option<UpgradeCopy>, StoreError> {
        let root = root.as_ref();
        check_id(name)?;
        let database = root.join(DATABASE);
        if !database.exists() {
            return Ok(None);
        }
        let connection = Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_WRITE).map_err(
            |source| StoreError::Open {
                path: database.clone(),
                source,
            },
        )?;
        let schema: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(|source| StoreError::Database {
                path: database.clone(),
                doing: "read the schema version",
                source,
            })?;
        if schema == SCHEMA_VERSION {
            return Ok(None);
        }
        if schema > SCHEMA_VERSION {
            return Err(StoreError::SchemaTooNew {
                path: database,
                found: schema,
                supported: SCHEMA_VERSION,
            });
        }
        let folder = root.join(UPGRADE_COPIES);
        create_dir(&folder)?;
        sweep_partials(&folder)?;
        let path = upgrade_copy_path(root, name);
        let temp = folder.join(format!(
            ".{name}.sqlite.{}-{}.partial",
            std::process::id(),
            COPIES.fetch_add(1, Ordering::Relaxed)
        ));
        let saved = save_to(&connection, &temp, &path, &folder, "store.upgrade_copy.copied", schema);
        if saved.is_err() {
            let _ = humata2_durable::remove_file(&temp);
        }
        saved.map_err(|error| match error {
            StoreError::Snapshot { path, problem } => StoreError::UpgradeCopyCheck { path, problem },
            other => other,
        })?;
        let taken_unix_ms = mtime_unix_ms(&path).unwrap_or_else(now_unix_ms);
        Ok(Some(UpgradeCopy {
            path,
            schema,
            taken_unix_ms,
        }))
    }

    /// Puts the pre-upgrade copy `name` back in place of the database in
    /// `root`, keeping the database it replaces in an `upgraded-<ms>`
    /// folder (D281g). A copy a crash cut short after the database moved
    /// aside is finished instead. If the last good copy is of another
    /// schema, it's replaced by this copy first, so the older build
    /// restores its own schema and not the newer one's. Fails with
    /// [`StoreError::NoUpgradeCopy`] when `name` was never taken or was
    /// dropped.
    ///
    /// The store must not be open while this runs. Running it again after
    /// it succeeded puts the copy back again.
    pub fn restore_upgrade_copy(root: impl AsRef<Path>, name: &str) -> Result<UpgradeRestored, StoreError> {
        let root = root.as_ref();
        check_id(name)?;
        let copy = upgrade_copy_path(root, name);
        if !copy.exists() {
            return Err(StoreError::NoUpgradeCopy { path: copy });
        }
        let schema = schema_of(&copy)?;
        let database = root.join(DATABASE);
        let database_name = file_name(&database);
        let temp = upgrade_restore_temp(root, &database_name);
        if !database.exists() && temp.exists() {
            // A cut came after the database moved aside: finish the put-back.
            place(&temp, &database, root)?;
            let kept = match newest_kept(root, UPGRADED) {
                Some(kept) => kept,
                None => {
                    let kept = root.join(format!("{UPGRADED}{}", now_unix_ms()));
                    create_dir(&kept)?;
                    kept
                }
            };
            return Ok(UpgradeRestored {
                schema,
                replaced_kept_at: kept.join(&database_name),
            });
        }
        let last_good = root.join(LAST_GOOD);
        if last_good.exists() && schema_of(&last_good).ok() != Some(schema) {
            let temp_last = root.join(format!(".{LAST_GOOD}.upgrade.partial"));
            copy_of_copy(&copy, &temp_last, &last_good)?;
            place(&temp_last, &last_good, root)?;
        }
        copy_of_copy(&copy, &temp, &database)?;
        fault::point("store.upgrade_restore.copied");
        let kept = root.join(format!("{UPGRADED}{}", now_unix_ms()));
        create_dir(&kept)?;
        let replaced_kept_at = kept.join(&database_name);
        move_aside(root, &database, &replaced_kept_at)?;
        fault::point("store.upgrade_restore.moved");
        place(&temp, &database, root)?;
        prune_kept(root);
        Ok(UpgradeRestored {
            schema,
            replaced_kept_at,
        })
    }

    /// Removes the pre-upgrade copy `name` in `root` once the upgrade has
    /// settled, so a later put-back is refused. Dropping a copy that's
    /// already gone does nothing. The blob files it named go at the next
    /// open.
    ///
    /// The store must not be open while this runs.
    pub fn drop_upgrade_copy(root: impl AsRef<Path>, name: &str) -> Result<(), StoreError> {
        let root = root.as_ref();
        check_id(name)?;
        let copy = upgrade_copy_path(root, name);
        match humata2_durable::remove_file(&copy) {
            Ok(()) => {}
            Err(source) if source.kind() == ErrorKind::NotFound => {}
            Err(source) => return Err(StoreError::Remove { path: copy, source }),
        }
        let folder = root.join(UPGRADE_COPIES);
        if folder.exists() {
            humata2_durable::sync_dir(&folder).map_err(|source| StoreError::Remove {
                path: folder,
                source,
            })?;
        }
        Ok(())
    }
}

/// Where the damaged databases restores moved aside are kept, by when.
const DAMAGED: &str = "damaged-";

/// How many newest `upgraded-*` backup folders to retain (at least 2).
const KEEP_BACKUPS: usize = 2;

/// Best-effort pruning of old `upgraded-*` folders in `root`, keeping at
/// least the newest [`KEEP_BACKUPS`]. `damaged-*` folders are never pruned
/// here: a damaged database may hold the only copy of recent work, so only
/// the owner removes them.
pub(crate) fn prune_kept(root: &Path) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    let mut kept: Vec<(u64, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name();
            let at: u64 = name.to_str()?.strip_prefix(UPGRADED)?.parse().ok()?;
            Some((at, entry.path()))
        })
        .collect();
    if kept.len() <= KEEP_BACKUPS {
        return;
    }
    kept.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    for (_, path) in kept.into_iter().skip(KEEP_BACKUPS) {
        let _ = fs::remove_dir_all(&path);
    }
}

fn file_name(database: &Path) -> std::ffi::OsString {
    database.file_name().map_or_else(|| "state.sqlite".into(), ToOwned::to_owned)
}

fn restore_temp(root: &Path, name: &std::ffi::OsStr) -> PathBuf {
    root.join(format!(".{}.restore.partial", name.to_string_lossy()))
}

fn upgrade_restore_temp(root: &Path, name: &std::ffi::OsStr) -> PathBuf {
    root.join(format!(".{}.upgrade-restore.partial", name.to_string_lossy()))
}

/// Where the pre-upgrade copy called `name` is kept.
fn upgrade_copy_path(root: &Path, name: &str) -> PathBuf {
    root.join(UPGRADE_COPIES).join(format!("{name}.sqlite"))
}

/// The schema version of the database at `path`, read without changing it.
fn schema_of(path: &Path) -> Result<i64, StoreError> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|source| {
        StoreError::Open {
            path: path.to_owned(),
            source,
        }
    })?;
    connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|source| StoreError::Database {
            path: path.to_owned(),
            doing: "read the schema version",
            source,
        })
}

/// Finishes putting a pre-upgrade copy back that a crash cut short after
/// the database was moved aside: the database is missing and the copy of
/// the copy is beside it, so it's placed. [`Store::open`](crate::Store::open)
/// calls this before it sweeps the folder's partial writes, so an open
/// between the cut and the retry doesn't create an empty database. Does
/// nothing otherwise.
pub(crate) fn finish_upgrade_restore(root: &Path, database: &Path) -> Result<(), StoreError> {
    let temp = upgrade_restore_temp(root, &file_name(database));
    if database.exists() || !temp.exists() {
        return Ok(());
    }
    place(&temp, database, root)
}

/// The newest folder under `root` named with `prefix` and a time, which a
/// restore kept a database in.
fn newest_kept(root: &Path, prefix: &str) -> Option<PathBuf> {
    fs::read_dir(root)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name();
            let at: u64 = name.to_str()?.strip_prefix(prefix)?.parse().ok()?;
            Some((at, entry.path()))
        })
        .max_by_key(|(at, _)| *at)
        .map(|(_, path)| path)
}

/// Copies the last good copy to `temp`, synced, keeping the copy itself.
fn copy_of_copy(copy: &Path, temp: &Path, database: &Path) -> Result<(), StoreError> {
    let copied = (|| {
        fs::copy(copy, temp)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true);
        humata2_durable::File::open(&options, temp)?.sync_all()
    })();
    copied.map_err(|source| {
        let _ = humata2_durable::remove_file(temp);
        StoreError::Write {
            path: database.to_owned(),
            source,
        }
    })
}

/// Moves `database` and its -wal and -shm, those still there, to
/// `damaged`, and syncs both folders.
fn move_aside(root: &Path, database: &Path, damaged: &Path) -> Result<(), StoreError> {
    let moved = |from: &Path, to: &Path| {
        humata2_durable::rename(from, to).or_else(|error| match error.kind() {
            ErrorKind::NotFound => Ok(()),
            _ => Err(StoreError::Write {
                path: from.to_owned(),
                source: error,
            }),
        })
    };
    moved(database, damaged)?;
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
    if let Some(kept) = damaged.parent() {
        sync(kept)?;
    }
    sync(root)
}

/// Renames the copy of the copy over `database`, and syncs the folder.
fn place(temp: &Path, database: &Path, root: &Path) -> Result<(), StoreError> {
    let placed = humata2_durable::rename(temp, database).and_then(|()| humata2_durable::sync_dir(root));
    placed.map_err(|source| {
        let _ = humata2_durable::remove_file(temp);
        StoreError::Write {
            path: database.to_owned(),
            source,
        }
    })
}

fn restored(copy: &Path, damaged: PathBuf, damage: String) -> Restored {
    let (changes, unreadable_tables) = differences(&damaged, copy);
    Restored {
        copy_taken_unix_ms: mtime_unix_ms(copy).unwrap_or(0),
        damaged_kept_at: damaged,
        damage,
        changes,
        unreadable_tables,
    }
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
