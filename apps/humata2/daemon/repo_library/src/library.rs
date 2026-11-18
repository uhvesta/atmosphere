//! The repo library on the host (RL-1, RL-2, RL-4): every repo the user has
//! added, in one SQLite file of its own. It is user data and nothing else
//! rebuilds it, so the file is in WAL mode with synchronous FULL, every
//! change is one immediate transaction, and a file that isn't SQLite is
//! refused before anything is written to it.
//!
//! One number, the revision, counts the changes ever made. It starts at 0
//! in a new file, goes up with every change, stays as it is for a call that
//! changes nothing, and survives closing and reopening the file.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rusqlite::{Connection, Transaction, TransactionBehavior, params};

use crate::name::RepoName;

/// Where a repo was last used: in a channel, or on the Desk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Place {
    /// A channel, by its id.
    Channel(String),
    /// The Desk.
    Desk,
}

impl Place {
    /// The text the library stores: `desk`, or `channel:` and the id.
    fn to_text(&self) -> String {
        match self {
            Place::Desk => "desk".to_owned(),
            Place::Channel(id) => format!("channel:{id}"),
        }
    }
}

fn place_from_text(text: &str) -> Result<Place, rusqlite::Error> {
    if text == "desk" {
        return Ok(Place::Desk);
    }
    match text.strip_prefix("channel:") {
        Some(id) => Ok(Place::Channel(id.to_owned())),
        None => Err(rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            format!("unknown place {text:?} in the repo library").into(),
        )),
    }
}

/// One use of a repo: it was added, or used again, at `at`, in `place`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Use {
    /// The repo.
    pub repo: RepoName,
    /// The clone URL the repo was added with. An empty one keeps the stored
    /// URL.
    pub remote: String,
    /// The repo's default branch. An empty one keeps the stored branch.
    pub default_branch: String,
    /// When the use happened, in Unix milliseconds. Only a later `at` moves
    /// the repo's last use forward.
    pub at: i64,
    /// Where the use happened.
    pub place: Place,
    /// Whether `repo` is spelled as GitHub spells it (its `full_name`, from
    /// a search hit, the repo listing or `gh repo view`). A canonical name
    /// replaces the stored owner and name casing; a name the user typed
    /// only sets it for a repo the library doesn't have yet.
    pub canonical: bool,
}

/// A repo in the library, as the library lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The repo.
    pub repo: RepoName,
    /// The clone URL the repo was last added with.
    pub remote: String,
    /// The repo's default branch.
    pub default_branch: String,
    /// When the repo was first added (or added again after a remove).
    pub added_at: i64,
    /// When the repo was last used, in Unix milliseconds.
    pub last_used_at: i64,
    /// Where the repo was last used.
    pub last_used_in: Place,
}

/// What went wrong reading or changing the repo library.
#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    /// The library file couldn't be opened, read or written, or it isn't a
    /// repo library. The file is left as it was.
    #[error(
        "couldn't read or write the repo library at {}. If the disk is unmounted or full, mount it or free some space, then try again. If the file isn't a Humata repo library, move it aside: Humata then makes a new, empty library, and the repos in the old file are not recovered. Don't delete a library that holds repos you use",
        path.display()
    )]
    Database {
        /// The library file.
        path: PathBuf,
        /// What SQLite reported.
        #[source]
        source: rusqlite::Error,
    },
}

/// The repo library, open. Clones share the one file and can be used from
/// any thread.
#[derive(Clone)]
pub struct Library {
    path: PathBuf,
    db: Arc<Mutex<Connection>>,
}

const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS repos (
        repo_key TEXT PRIMARY KEY,
        owner TEXT NOT NULL,
        name TEXT NOT NULL,
        remote TEXT NOT NULL,
        default_branch TEXT NOT NULL,
        added_at INTEGER NOT NULL,
        last_used_at INTEGER NOT NULL,
        last_used_in TEXT NOT NULL
    ) WITHOUT ROWID;
    CREATE INDEX IF NOT EXISTS repos_by_last_use ON repos (last_used_at, repo_key);
    CREATE TABLE IF NOT EXISTS meta (
        key TEXT PRIMARY KEY,
        value INTEGER NOT NULL
    ) WITHOUT ROWID;
    INSERT OR IGNORE INTO meta (key, value) VALUES ('revision', 0);
";

/// Adds a repo, or updates it when its key is already there. The update
/// keeps `added_at`, takes the owner and name casing only from a canonical
/// name (?8), keeps a blank
/// remote or branch, and moves the last use only for a later `at`. The
/// WHERE clause changes nothing, and so counts no row, when the values
/// would stay the same.
const RECORD: &str = "
    INSERT INTO repos (repo_key, owner, name, remote, default_branch, added_at, last_used_at, last_used_in)
    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7)
    ON CONFLICT (repo_key) DO UPDATE SET
        owner = CASE WHEN ?8 THEN excluded.owner ELSE repos.owner END,
        name = CASE WHEN ?8 THEN excluded.name ELSE repos.name END,
        remote = CASE WHEN excluded.remote = '' THEN repos.remote ELSE excluded.remote END,
        default_branch = CASE WHEN excluded.default_branch = '' THEN repos.default_branch ELSE excluded.default_branch END,
        last_used_at = MAX(repos.last_used_at, excluded.last_used_at),
        last_used_in = CASE WHEN excluded.last_used_at > repos.last_used_at THEN excluded.last_used_in ELSE repos.last_used_in END
    WHERE (?8 AND (repos.owner != excluded.owner OR repos.name != excluded.name))
        OR (excluded.remote != '' AND repos.remote != excluded.remote)
        OR (excluded.default_branch != '' AND repos.default_branch != excluded.default_branch)
        OR excluded.last_used_at > repos.last_used_at
";

impl Library {
    /// Opens the repo library at `path`, creating an empty one at revision
    /// 0 if there is no file there. A file that exists but isn't a
    /// repo library is refused and left exactly as it was.
    pub fn open(path: &Path) -> Result<Library, LibraryError> {
        let fail = |source| LibraryError::Database {
            path: path.to_owned(),
            source,
        };
        let conn = Connection::open(path).map_err(fail)?;
        // Read the file before anything is written to it, so a file that
        // isn't SQLite fails here and is left alone.
        conn.query_row("PRAGMA schema_version", [], |row| row.get::<_, i64>(0))
            .map_err(fail)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             PRAGMA fullfsync = ON;
             PRAGMA checkpoint_fullfsync = ON;
             PRAGMA busy_timeout = 5000;",
        )
        .map_err(fail)?;
        conn.execute_batch(SCHEMA).map_err(fail)?;
        Ok(Library {
            path: path.to_owned(),
            db: Arc::new(Mutex::new(conn)),
        })
    }

    /// Records a use of a repo: adds it if it is new, or moves its last use
    /// forward if `at` is later than the last one. Returns the revision
    /// after the call. A use that changes nothing returns the revision
    /// unchanged.
    pub fn record(&self, usage: &Use) -> Result<u64, LibraryError> {
        let key = usage.repo.key();
        let place = usage.place.to_text();
        self.write(|tx| {
            let changed = tx.execute(
                RECORD,
                params![
                    key,
                    usage.repo.owner(),
                    usage.repo.name(),
                    usage.remote,
                    usage.default_branch,
                    usage.at,
                    place,
                    usage.canonical
                ],
            )?;
            if changed > 0 {
                bump(tx)
            } else {
                revision_of(tx)
            }
        })
    }

    /// The repos in the library, most recently used first (ties sorted by
    /// name), with the revision they were read at.
    pub fn list(&self) -> Result<(Vec<Entry>, u64), LibraryError> {
        let mut conn = self.lock();
        let read = |conn: &mut Connection| -> Result<(Vec<Entry>, u64), rusqlite::Error> {
            let tx = conn.transaction()?;
            let revision = revision_of(&tx)?;
            let entries = entries_in(&tx)?;
            Ok((entries, revision))
        };
        read(&mut conn).map_err(|source| self.fail(source))
    }

    /// Takes one repo out of the library. Channels, sessions and clones that
    /// use it are not touched, and it comes back when it is added again.
    /// Returns whether the repo was in the library.
    pub fn remove(&self, repo: &RepoName) -> Result<bool, LibraryError> {
        let key = repo.key();
        self.write(|tx| {
            let removed = tx.execute("DELETE FROM repos WHERE repo_key = ?1", params![key])?;
            if removed > 0 {
                bump(tx)?;
            }
            Ok(removed > 0)
        })
    }

    /// Empties the library and returns how many repos it forgot. Nothing on
    /// disk or on GitHub is touched. Forgetting nothing changes nothing.
    pub fn forget_all(&self) -> Result<usize, LibraryError> {
        self.write(|tx| {
            let removed = tx.execute("DELETE FROM repos", [])?;
            if removed > 0 {
                bump(tx)?;
            }
            Ok(removed)
        })
    }

    fn lock(&self) -> MutexGuard<'_, Connection> {
        self.db.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn fail(&self, source: rusqlite::Error) -> LibraryError {
        LibraryError::Database {
            path: self.path.clone(),
            source,
        }
    }

    /// Runs `body` in one immediate transaction and commits it. If `body`
    /// fails, the transaction is rolled back.
    fn write<T>(
        &self,
        body: impl FnOnce(&Transaction<'_>) -> Result<T, rusqlite::Error>,
    ) -> Result<T, LibraryError> {
        let mut conn = self.lock();
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|source| self.fail(source))?;
        let value = body(&tx).map_err(|source| self.fail(source))?;
        tx.commit().map_err(|source| self.fail(source))?;
        Ok(value)
    }
}

fn revision_of(conn: &Connection) -> Result<u64, rusqlite::Error> {
    let value: i64 = conn.query_row(
        "SELECT value FROM meta WHERE key = 'revision'",
        [],
        |row| row.get(0),
    )?;
    Ok(u64::try_from(value).unwrap_or(0))
}

/// Raises the revision by one and returns it.
fn bump(tx: &Transaction<'_>) -> Result<u64, rusqlite::Error> {
    tx.execute("UPDATE meta SET value = value + 1 WHERE key = 'revision'", [])?;
    revision_of(tx)
}

/// Every repo in list order: most recently used first, then by key.
fn entries_in(conn: &Connection) -> Result<Vec<Entry>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT owner, name, remote, default_branch, added_at, last_used_at, last_used_in
         FROM repos ORDER BY last_used_at DESC, repo_key ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, i64>(4)?,
            row.get::<_, i64>(5)?,
            row.get::<_, String>(6)?,
        ))
    })?;
    let mut entries = Vec::new();
    for row in rows {
        let (owner, name, remote, default_branch, added_at, last_used_at, last_used_in) = row?;
        entries.push(Entry {
            repo: RepoName::from_parts(owner, name),
            remote,
            default_branch,
            added_at,
            last_used_at,
            last_used_in: place_from_text(&last_used_in)?,
        });
    }
    Ok(entries)
}
