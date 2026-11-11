//! The daemon's own records, in SQLite: projects, and which project and host
//! each session belongs to. A session's history stays with the agent on its
//! host; this only says where to find it.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::StoreError;

/// Each step takes the schema from its index to the next version; the
/// version is kept in `PRAGMA user_version`.
const MIGRATIONS: &[&str] = &["
    CREATE TABLE projects (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        repository TEXT NOT NULL,
        instructions TEXT NOT NULL,
        created_unix_ms INTEGER NOT NULL
    );
    CREATE TABLE sessions (
        id TEXT PRIMARY KEY,
        project_id TEXT REFERENCES projects (id),
        host TEXT NOT NULL
    );
    CREATE INDEX sessions_by_project ON sessions (project_id);
"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Project {
    pub id: String,
    pub name: String,
    /// Its folder on this Mac.
    pub repository: PathBuf,
    pub instructions: String,
    pub created_unix_ms: i64,
}

/// Where a session is, as the daemon records it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placement {
    /// Empty for this Mac.
    pub host: String,
    pub project: Option<String>,
}

pub struct Store {
    path: PathBuf,
    connection: Mutex<Connection>,
}

impl Store {
    /// Opens the store at `path`, creating it and bringing its schema up to
    /// date.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|source| StoreError::CreateDirectory {
                path: dir.to_owned(),
                source,
            })?;
        }
        let opened = |source| StoreError::Open {
            path: path.to_owned(),
            source,
        };
        let mut connection = Connection::open(path).map_err(opened)?;
        connection
            .execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")
            .map_err(opened)?;
        migrate(&mut connection, path)?;
        Ok(Self {
            path: path.to_owned(),
            connection: Mutex::new(connection),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Connection> {
        self.connection
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn failed(&self) -> impl Fn(rusqlite::Error) -> StoreError + '_ {
        |source| StoreError::Query {
            path: self.path.clone(),
            source,
        }
    }

    pub fn add_project(
        &self,
        name: String,
        repository: PathBuf,
        instructions: String,
    ) -> Result<Project, StoreError> {
        let project = Project {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            repository,
            instructions,
            created_unix_ms: now_unix_ms(),
        };
        self.lock()
            .execute(
                "INSERT INTO projects (id, name, repository, instructions, created_unix_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    project.id,
                    project.name,
                    project.repository.to_string_lossy(),
                    project.instructions,
                    project.created_unix_ms,
                ],
            )
            .map_err(self.failed())?;
        Ok(project)
    }

    /// Every project, oldest first.
    pub fn projects(&self) -> Result<Vec<Project>, StoreError> {
        let connection = self.lock();
        let mut statement = connection
            .prepare(
                "SELECT id, name, repository, instructions, created_unix_ms FROM projects
                 ORDER BY created_unix_ms, id",
            )
            .map_err(self.failed())?;
        statement
            .query_map([], project)
            .and_then(Iterator::collect)
            .map_err(self.failed())
    }

    pub fn project(&self, id: &str) -> Result<Project, StoreError> {
        self.lock()
            .query_row(
                "SELECT id, name, repository, instructions, created_unix_ms FROM projects
                 WHERE id = ?1",
                [id],
                project,
            )
            .optional()
            .map_err(self.failed())?
            .ok_or_else(|| StoreError::UnknownProject { id: id.to_owned() })
    }

    /// Records that `session` runs on `host`, and, if given, belongs to
    /// `project`. A session's project, once set, is kept.
    pub fn place(
        &self,
        session: &str,
        host: &str,
        project: Option<&str>,
    ) -> Result<(), StoreError> {
        self.lock()
            .execute(
                "INSERT INTO sessions (id, project_id, host) VALUES (?1, ?2, ?3)
                 ON CONFLICT (id) DO UPDATE SET
                     host = excluded.host,
                     project_id = coalesce(sessions.project_id, excluded.project_id)
                 WHERE sessions.host IS NOT excluded.host
                     OR (sessions.project_id IS NULL AND excluded.project_id IS NOT NULL)",
                params![session, project, host],
            )
            .map_err(|source| match source {
                rusqlite::Error::SqliteFailure(failure, _)
                    if failure.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_FOREIGNKEY =>
                {
                    StoreError::UnknownProject {
                        id: project.unwrap_or_default().to_owned(),
                    }
                }
                source => self.failed()(source),
            })?;
        Ok(())
    }

    /// Where `session` is; None if the daemon has never seen it.
    pub fn placement(&self, session: &str) -> Result<Option<Placement>, StoreError> {
        self.lock()
            .query_row(
                "SELECT host, project_id FROM sessions WHERE id = ?1",
                [session],
                |row| {
                    Ok(Placement {
                        host: row.get(0)?,
                        project: row.get(1)?,
                    })
                },
            )
            .optional()
            .map_err(self.failed())
    }
}

fn project(row: &rusqlite::Row<'_>) -> rusqlite::Result<Project> {
    Ok(Project {
        id: row.get(0)?,
        name: row.get(1)?,
        repository: PathBuf::from(row.get::<_, String>(2)?),
        instructions: row.get(3)?,
        created_unix_ms: row.get(4)?,
    })
}

fn migrate(connection: &mut Connection, path: &Path) -> Result<(), StoreError> {
    let failed = |source| StoreError::Migrate {
        path: path.to_owned(),
        source,
    };
    let found: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(failed)?;
    let mut version = 0_i64;
    for step in MIGRATIONS {
        version += 1;
        if version <= found {
            continue;
        }
        let transaction = connection.transaction().map_err(failed)?;
        transaction.execute_batch(step).map_err(failed)?;
        transaction
            .pragma_update(None, "user_version", version)
            .map_err(failed)?;
        transaction.commit().map_err(failed)?;
    }
    if found > version {
        return Err(StoreError::NewerSchema {
            path: path.to_owned(),
            found,
            known: version,
        });
    }
    Ok(())
}

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(dir: &Path) -> Store {
        Store::open(&dir.join("khodr.sqlite3")).expect("the store opens")
    }

    #[test]
    fn projects_survive_reopening_in_the_order_they_were_added() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let first = store(dir.path())
            .add_project("one".into(), "/code/one".into(), "be brief".into())
            .expect("added");
        let second = store(dir.path())
            .add_project("two".into(), "/code/two".into(), String::new())
            .expect("added");
        let reopened = store(dir.path());
        assert_eq!(
            reopened.projects().expect("listed"),
            vec![first.clone(), second]
        );
        assert_eq!(reopened.project(&first.id).expect("found"), first);
    }

    #[test]
    fn an_unknown_project_is_an_error() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let store = store(dir.path());
        assert!(matches!(
            store.project("nope"),
            Err(StoreError::UnknownProject { .. })
        ));
        assert!(matches!(
            store.place("s", "", Some("nope")),
            Err(StoreError::UnknownProject { .. })
        ));
    }

    #[test]
    fn a_session_keeps_its_project_when_placed_again() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let store = store(dir.path());
        let project = store
            .add_project("one".into(), "/code/one".into(), String::new())
            .expect("added");
        assert_eq!(store.placement("s").expect("read"), None);
        store.place("s", "", Some(&project.id)).expect("placed");
        store.place("s", "build", None).expect("placed again");
        assert_eq!(
            store.placement("s").expect("read"),
            Some(Placement {
                host: "build".into(),
                project: Some(project.id),
            })
        );
    }

    #[test]
    fn a_store_from_a_newer_khodrd_is_refused() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let path = dir.path().join("khodr.sqlite3");
        drop(Store::open(&path).expect("the store opens"));
        let connection = Connection::open(&path).expect("opens");
        connection
            .pragma_update(None, "user_version", 1_000)
            .expect("set");
        drop(connection);
        assert!(matches!(
            Store::open(&path),
            Err(StoreError::NewerSchema { .. })
        ));
    }
}
