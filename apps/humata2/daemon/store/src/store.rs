use std::collections::HashSet;
use std::ffi::OsString;
use std::fs;
use std::io::ErrorKind;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, Row, Transaction, TransactionBehavior, params};

use crate::atomic::{create_dir, sweep_partials, write_atomic};
use crate::journal::Journal;
use crate::last_good::{self, LastGood, Restored};
use crate::{Pointers, RepoPointer, SessionRecord, StoreError, TabKind, TabRecord, blob, fault, schema};

const DATABASE: &str = "state.sqlite";
const BLOBS: &str = "blobs";
const FILES: &str = "files";
const PASTE: &str = "paste";

static PASTES: AtomicU64 = AtomicU64::new(0);

/// One machine's state folder.
///
/// ```text
/// state.sqlite              sessions, tabs, pointers, agents, the journal
/// blobs/<sha256>            blobs too big for their row (see blob.rs)
/// files/<session>/paste/    images pasted into the session's terminals
/// ```
///
/// Folders are 0700 and files 0600. Every write is one SQLite transaction
/// in WAL mode with `synchronous = FULL`: it is on disk when the call
/// returns, and a crash at any moment leaves it whole or not at all.
pub struct Store {
    inner: Arc<Inner>,
    journal: Journal,
}

pub(crate) struct Inner {
    root: PathBuf,
    pub(crate) database: PathBuf,
    pub(crate) blobs: PathBuf,
    connection: Mutex<Connection>,
    /// Told after each committed write, outside the database lock.
    committed: Mutex<Vec<Committed>>,
}

/// What [`Store::on_commit`] is given.
type Committed = Box<dyn Fn() + Send + Sync>;

impl Store {
    /// Opens the state in `root`, making it if it's missing, brings its
    /// schema up to this build's, and removes files a crash left: writes
    /// it cut short, and blobs no record names. A restore of the last good
    /// copy a crash cut short is finished first.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, StoreError> {
        let root = root.into();
        create_dir(&root)?;
        let blobs = root.join(BLOBS);
        create_dir(&blobs)?;
        create_dir(&root.join(FILES))?;
        let database = root.join(DATABASE);
        last_good::finish_restore(&root, &database)?;
        sweep_partials(&root)?;
        Self::sweep_paste_partials(&root.join(FILES))?;
        let connection = schema::open(&database)?;
        let inner = Arc::new(Inner {
            root,
            database,
            blobs,
            connection: Mutex::new(connection),
            committed: Mutex::new(Vec::new()),
        });
        let store = Self {
            journal: Journal::new(inner.clone()),
            inner,
        };
        store.sweep()?;
        Ok(store)
    }

    /// Opens the state in `root` as [`Store::open`] does, and if its
    /// database is damaged (it won't open, or SQLite's quick check fails),
    /// moves it aside and restores the last good copy, saying what that
    /// lost (PER-31). Fails as before when there's no copy, or when the
    /// database is unreachable rather than damaged.
    pub fn open_or_restore(root: impl Into<PathBuf>) -> Result<(Self, Option<Restored>), StoreError> {
        let root = root.into();
        let database = root.join(DATABASE);
        if let Some(finished) = last_good::finish_restore(&root, &database)? {
            return Ok((Self::open(root)?, Some(finished)));
        }
        let damage = match Self::open(root.clone()) {
            Ok(store) => {
                let problem = store.inner.read_with(|connection| Ok(last_good::integrity(connection, "quick_check")))?;
                match problem {
                    None => return Ok((store, None)),
                    Some(problem) => problem,
                }
            }
            Err(error) if last_good::is_damage(&error) => match std::error::Error::source(&error) {
                Some(source) => format!("{error}: {source}"),
                None => error.to_string(),
            },
            Err(error) => return Err(error),
        };
        if !root.join(last_good::LAST_GOOD).exists() {
            return Err(StoreError::DamagedWithoutCopy { path: database, damage });
        }
        let restored = last_good::restore(&root, &database, damage)?;
        Ok((Self::open(root)?, Some(restored)))
    }

    /// What SQLite's quick check finds wrong with the database now, or
    /// None when it finds nothing: the check [`Store::open_or_restore`]
    /// makes at start, so a store this calls damaged is restored from its
    /// last good copy when the daemon starts again (PER-30, PER-31).
    pub fn damage(&self) -> Result<Option<String>, StoreError> {
        self.inner
            .read_with(|connection| Ok(last_good::integrity(connection, "quick_check")))
    }

    /// Whether a last good copy is kept beside the database to restore.
    pub fn has_last_good(&self) -> bool {
        self.inner.root.join(last_good::LAST_GOOD).exists()
    }

    /// Saves a last good copy of the database beside it, replacing the one
    /// before once the new one checks out (PER-31). Holds the database
    /// while it copies, so run it off the async runtime, when writes are
    /// few.
    pub fn save_last_good(&self) -> Result<LastGood, StoreError> {
        let connection = self.inner.lock();
        last_good::save(&connection, &self.inner.root)
    }

    /// Calls `then` after every write this store commits, on the thread
    /// that wrote, once the database is free again. Keep it quick: a send
    /// on a channel, say. This is how the daemon learns of writes without
    /// watching the files or polling (TR-7).
    pub fn on_commit(&self, then: impl Fn() + Send + Sync + 'static) {
        self.inner
            .committed
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(Box::new(then));
    }

    pub fn root(&self) -> &Path {
        &self.inner.root
    }

    pub fn journal(&self) -> &Journal {
        &self.journal
    }

    /// Removes blob files no record names: left by a crash between writing
    /// one and committing its row, or by records since removed. Returns
    /// how many went.
    pub fn sweep(&self) -> Result<usize, StoreError> {
        self.inner.sweep()
    }

    pub fn put_session(&self, session: &SessionRecord) -> Result<(), StoreError> {
        check_id(&session.id)?;
        self.inner.write("session.put", "save a session", |transaction| {
            transaction
                .execute(
                    "INSERT INTO sessions (id, channel, machine, title, agent, folder, created_unix_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                     ON CONFLICT (id) DO UPDATE SET
                         channel = excluded.channel, machine = excluded.machine,
                         title = excluded.title, agent = excluded.agent,
                         folder = excluded.folder, created_unix_ms = excluded.created_unix_ms",
                    params![
                        session.id,
                        session.channel,
                        session.machine,
                        session.title,
                        session.agent,
                        path_bytes(&session.folder),
                        session.created_unix_ms,
                    ],
                )
                .map(drop)
        })
    }

    pub fn session(&self, id: &str) -> Result<Option<SessionRecord>, StoreError> {
        check_id(id)?;
        self.inner.read("read a session", |connection| {
            connection
                .query_row(
                    "SELECT id, channel, machine, title, agent, folder, created_unix_ms
                     FROM sessions WHERE id = ?1",
                    [id],
                    session_row,
                )
                .optional()
        })
    }

    /// Every session, oldest first.
    pub fn sessions(&self) -> Result<Vec<SessionRecord>, StoreError> {
        self.inner.read("list sessions", |connection| {
            connection
                .prepare(
                    "SELECT id, channel, machine, title, agent, folder, created_unix_ms
                     FROM sessions ORDER BY created_unix_ms, id",
                )?
                .query_map([], session_row)?
                .collect()
        })
    }

    /// Removes a session with its tabs, pointers, agent and pasted files.
    /// The files go first, so a crash part way leaves the session to
    /// remove again.
    pub fn remove_session(&self, id: &str) -> Result<(), StoreError> {
        check_id(id)?;
        let files = self.inner.root.join(FILES).join(id);
        match fs::remove_dir_all(&files) {
            Err(source) if source.kind() != ErrorKind::NotFound => {
                return Err(StoreError::Remove {
                    path: files,
                    source,
                });
            }
            _ => {}
        }
        self.inner.write("session.remove", "remove a session", |transaction| {
            transaction
                .execute("DELETE FROM sessions WHERE id = ?1", [id])
                .and_then(|_| {
                    transaction.execute(
                        "DELETE FROM previous_conversations WHERE session_id = ?1",
                        [id],
                    )
                })
                .and_then(|_| {
                    transaction.execute("DELETE FROM fresh_seeds WHERE session_id = ?1", [id])
                })
                .and_then(|_| {
                    transaction.execute("DELETE FROM fork_origins WHERE session_id = ?1", [id])
                })
                .and_then(|_| {
                    transaction.execute("DELETE FROM archives WHERE session_id = ?1", [id])
                })
                .map(drop)
        })?;
        self.inner.sweep().map(drop)
    }

    /// Saves a tab. A tab of a session needs the session saved first.
    pub fn put_tab(&self, tab: &TabRecord) -> Result<(), StoreError> {
        check_id(&tab.id)?;
        let doing = "save a tab";
        self.inner
            .write_for_session("tab.put", doing, tab.session.as_deref(), |transaction| {
                put_tab(transaction, tab).map_err(self.inner.failed(doing))
            })
    }

    pub fn tab(&self, id: &str) -> Result<Option<TabRecord>, StoreError> {
        check_id(id)?;
        self.inner.read("read a tab", |connection| {
            connection
                .query_row(
                    "SELECT id, session, kind, title, holder_socket, holder_pid, created_unix_ms
                     FROM tabs WHERE id = ?1",
                    [id],
                    tab_row,
                )
                .optional()
        })
    }

    /// Every tab, oldest first.
    pub fn tabs(&self) -> Result<Vec<TabRecord>, StoreError> {
        self.inner.read("list tabs", |connection| {
            connection
                .prepare(
                    "SELECT id, session, kind, title, holder_socket, holder_pid, created_unix_ms
                     FROM tabs ORDER BY created_unix_ms, id",
                )?
                .query_map([], tab_row)?
                .collect()
        })
    }

    pub fn remove_tab(&self, id: &str) -> Result<(), StoreError> {
        check_id(id)?;
        self.inner.write("tab.remove", "remove a tab", |transaction| {
            transaction
                .execute("DELETE FROM tabs WHERE id = ?1", [id])
                .map(drop)
        })
    }

    /// Replaces where a session's clones stand. The session must be saved.
    pub fn put_pointers(&self, session: &str, pointers: &Pointers) -> Result<(), StoreError> {
        let doing = "save where a session's clones stand";
        self.inner
            .write_for_session("pointers.put", doing, Some(session), |transaction| {
                put_pointers(transaction, session, pointers).map_err(self.inner.failed(doing))
            })
    }

    pub fn pointers(&self, session: &str) -> Result<Option<Pointers>, StoreError> {
        check_id(session)?;
        self.inner.read("read where a session's clones stand", |connection| {
            let Some(reported_unix_ms) = connection
                .query_row(
                    "SELECT reported_unix_ms FROM pointer_reports WHERE session = ?1",
                    [session],
                    |row| row.get(0),
                )
                .optional()?
            else {
                return Ok(None);
            };
            let mut repos: Vec<RepoPointer> = connection
                .prepare(
                    "SELECT owner, name, branch, sha, pushed FROM repo_pointers
                     WHERE session = ?1 ORDER BY position",
                )?
                .query_map([session], |row| {
                    Ok(RepoPointer {
                        owner: row.get(0)?,
                        name: row.get(1)?,
                        branch: row.get(2)?,
                        sha: row.get(3)?,
                        pushed: row.get(4)?,
                        dirty_files: Vec::new(),
                    })
                })?
                .collect::<rusqlite::Result<_>>()?;
            let mut dirty = connection.prepare(
                "SELECT position, path FROM dirty_files WHERE session = ?1 ORDER BY position, n",
            )?;
            let rows = dirty.query_map([session], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?;
            for row in rows {
                let (position, path) = row?;
                if let Some(repo) = usize::try_from(position).ok().and_then(|at| repos.get_mut(at)) {
                    repo.dirty_files.push(path);
                }
            }
            Ok(Some(Pointers {
                repos,
                reported_unix_ms,
            }))
        })
    }

    /// Saves an image pasted into a session's terminal under the session's
    /// own folder, readable only by this user, and returns where. The file
    /// goes when the session does.
    pub fn save_paste(
        &self,
        session: &str,
        extension: &str,
        bytes: &[u8],
    ) -> Result<PathBuf, StoreError> {
        check_id(extension)?;
        if self.session(session)?.is_none() {
            return Err(StoreError::NoSession {
                id: session.to_owned(),
            });
        }
        let folder = self.inner.root.join(FILES).join(session).join(PASTE);
        create_dir(&folder)?;
        let path = folder.join(format!(
            "paste-{}-{}.{extension}",
            unix_ms(),
            PASTES.fetch_add(1, Ordering::Relaxed),
        ));
        write_atomic(&path, bytes)?;
        Ok(path)
    }

    /// Removes the pastes a crash cut short in each session's folder under
    /// `files`.
    fn sweep_paste_partials(files: &Path) -> Result<(), StoreError> {
        let read_failed = |source| StoreError::Read {
            path: files.to_owned(),
            source,
        };
        for entry in fs::read_dir(files).map_err(read_failed)? {
            let entry = entry.map_err(read_failed)?;
            sweep_partials(&entry.path().join(PASTE))?;
        }
        Ok(())
    }

    pub(crate) fn inner(&self) -> &Inner {
        &self.inner
    }
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, Connection> {
        // A transaction a panic cut short rolled back when it dropped.
        self.connection
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn failed(&self, doing: &'static str) -> impl Fn(rusqlite::Error) -> StoreError + '_ {
        move |source| StoreError::Database {
            path: self.database.clone(),
            doing,
            source,
        }
    }

    /// Runs `work` as one transaction, committed before this returns.
    /// `op` names the fault points either side of the commit.
    pub(crate) fn write<T>(
        &self,
        op: &str,
        doing: &'static str,
        work: impl FnOnce(&Transaction<'_>) -> rusqlite::Result<T>,
    ) -> Result<T, StoreError> {
        self.write_with(op, doing, |transaction| {
            work(transaction).map_err(self.failed(doing))
        })
    }

    /// As [`Inner::write`], for work that also fails in the store's own
    /// terms, such as writing a blob's file.
    pub(crate) fn write_with<T>(
        &self,
        op: &str,
        doing: &'static str,
        work: impl FnOnce(&Transaction<'_>) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        let mut connection = self.lock();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(self.failed(doing))?;
        let value = work(&transaction)?;
        fault::commit_point(op, "committing");
        transaction.commit().map_err(self.failed(doing))?;
        fault::commit_point(op, "committed");
        drop(connection);
        for then in self.committed.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).iter() {
            then();
        }
        Ok(value)
    }

    /// As [`Inner::write`], saying a missing session is missing rather than
    /// a failure.
    pub(crate) fn write_for_session<T>(
        &self,
        op: &str,
        doing: &'static str,
        session: Option<&str>,
        work: impl FnOnce(&Transaction<'_>) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        if let Some(session) = session {
            check_id(session)?;
        }
        self.write_with(op, doing, work).map_err(|error| match (error, session) {
            (StoreError::Database { source, .. }, Some(session)) if names_no_row(&source) => {
                StoreError::NoSession {
                    id: session.to_owned(),
                }
            }
            (error, _) => error,
        })
    }

    pub(crate) fn read<T>(
        &self,
        doing: &'static str,
        work: impl FnOnce(&Connection) -> rusqlite::Result<T>,
    ) -> Result<T, StoreError> {
        work(&self.lock()).map_err(self.failed(doing))
    }

    pub(crate) fn read_with<T>(
        &self,
        work: impl FnOnce(&Connection) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        work(&self.lock())
    }

    /// Holds the database while it lists the blobs rows name, so no write
    /// can place a file the listing misses.
    pub(crate) fn sweep(&self) -> Result<usize, StoreError> {
        let connection = self.lock();
        let mut referenced = connection
            .prepare(
                "SELECT payload_file FROM journal WHERE payload_file IS NOT NULL
                 UNION SELECT launch_file FROM agents WHERE launch_file IS NOT NULL
                 UNION SELECT state_file FROM reviews WHERE state_file IS NOT NULL
                 UNION SELECT view_file FROM views WHERE view_file IS NOT NULL
                 UNION SELECT entry_file FROM inbox_requests WHERE entry_file IS NOT NULL",
            )
            .and_then(|mut statement| {
                statement
                    .query_map([], |row| row.get::<_, String>(0))?
                    .collect::<rusqlite::Result<HashSet<String>>>()
            })
            .map_err(self.failed("list the files records name"))?;
        // The last good copy's too, so a restored row finds its file.
        referenced.extend(last_good::copy_files(&self.root));
        blob::sweep(&self.blobs, &referenced)
    }
}

pub(crate) fn put_tab(transaction: &Transaction<'_>, tab: &TabRecord) -> rusqlite::Result<()> {
    let kind = match tab.kind {
        TabKind::Agent => "agent",
        TabKind::Shell => "shell",
    };
    transaction
        .execute(
            "INSERT INTO tabs (id, session, kind, title, holder_socket, holder_pid, created_unix_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (id) DO UPDATE SET
                 session = excluded.session, kind = excluded.kind, title = excluded.title,
                 holder_socket = excluded.holder_socket, holder_pid = excluded.holder_pid,
                 created_unix_ms = excluded.created_unix_ms",
            params![
                tab.id,
                tab.session,
                kind,
                tab.title,
                path_bytes(&tab.holder_socket),
                tab.holder_pid,
                tab.created_unix_ms,
            ],
        )
        .map(drop)
}

fn put_pointers(
    transaction: &Transaction<'_>,
    session: &str,
    pointers: &Pointers,
) -> rusqlite::Result<()> {
    transaction.execute("DELETE FROM pointer_reports WHERE session = ?1", [session])?;
    transaction.execute(
        "INSERT INTO pointer_reports (session, reported_unix_ms) VALUES (?1, ?2)",
        params![session, pointers.reported_unix_ms],
    )?;
    for (position, repo) in pointers.repos.iter().enumerate() {
        transaction.execute(
            "INSERT INTO repo_pointers (session, position, owner, name, branch, sha, pushed)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                session,
                position as i64,
                repo.owner,
                repo.name,
                repo.branch,
                repo.sha,
                repo.pushed,
            ],
        )?;
        for (n, path) in repo.dirty_files.iter().enumerate() {
            transaction.execute(
                "INSERT INTO dirty_files (session, position, n, path) VALUES (?1, ?2, ?3, ?4)",
                params![session, position as i64, n as i64, path],
            )?;
        }
    }
    Ok(())
}

/// A write that broke a foreign key: the session it names isn't saved.
fn names_no_row(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(failure, _)
            if failure.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_FOREIGNKEY
    )
}

fn session_row(row: &Row<'_>) -> rusqlite::Result<SessionRecord> {
    Ok(SessionRecord {
        id: row.get(0)?,
        channel: row.get(1)?,
        machine: row.get(2)?,
        title: row.get(3)?,
        agent: row.get(4)?,
        folder: bytes_path(row.get(5)?),
        created_unix_ms: row.get(6)?,
    })
}

fn tab_row(row: &Row<'_>) -> rusqlite::Result<TabRecord> {
    let kind: String = row.get(2)?;
    Ok(TabRecord {
        id: row.get(0)?,
        session: row.get(1)?,
        // The table's CHECK allows only these two.
        kind: if kind == "agent" {
            TabKind::Agent
        } else {
            TabKind::Shell
        },
        title: row.get(3)?,
        holder_socket: bytes_path(row.get(4)?),
        holder_pid: row.get(5)?,
        created_unix_ms: row.get(6)?,
    })
}

pub(crate) fn path_bytes(path: &Path) -> &[u8] {
    path.as_os_str().as_bytes()
}

pub(crate) fn bytes_path(bytes: Vec<u8>) -> PathBuf {
    PathBuf::from(OsString::from_vec(bytes))
}

/// Ids name folders, so they keep to characters that can't climb out of
/// their folder or hide.
pub(crate) fn check_id(id: &str) -> Result<(), StoreError> {
    let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.');
    if id.is_empty() || id.len() > 128 || id.starts_with('.') || !id.chars().all(allowed) {
        return Err(StoreError::InvalidId { id: id.to_owned() });
    }
    Ok(())
}

pub(crate) fn unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| i64::try_from(since.as_millis()).unwrap_or(i64::MAX))
}

/// An offset as SQLite's signed integers keep it.
pub(crate) fn to_sql(offset: u64) -> i64 {
    i64::try_from(offset).unwrap_or(i64::MAX)
}

pub(crate) fn from_sql(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}
