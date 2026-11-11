//! The daemon's own records, in SQLite: projects, and which project and host
//! each session belongs to. A session's history stays with the agent on its
//! host; this only says where to find it.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use khodr_agent::Repository;
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
", "
    CREATE TABLE project_repositories (
        project_id TEXT NOT NULL REFERENCES projects (id),
        position INTEGER NOT NULL,
        name TEXT NOT NULL,
        path TEXT NOT NULL,
        PRIMARY KEY (project_id, position)
    );
    -- Each project had one repository, named after its folder.
    INSERT INTO project_repositories (project_id, position, name, path)
        SELECT id, 0,
            coalesce(
                nullif(replace(repository, rtrim(repository, replace(repository, '/', '')), ''), ''),
                'repository'
            ),
            repository
        FROM projects;
"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Project {
    pub id: String,
    pub name: String,
    /// The repositories on this Mac its sessions work in; at least one.
    pub repositories: Vec<Repository>,
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
        repositories: Vec<Repository>,
        instructions: String,
    ) -> Result<Project, StoreError> {
        let project = Project {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            repositories,
            instructions,
            created_unix_ms: now_unix_ms(),
        };
        let mut connection = self.lock();
        let added = (|| {
            let transaction = connection.transaction()?;
            // The first repository also goes in the column the first schema
            // had, which is still there.
            let first = project
                .repositories
                .first()
                .map(|repository| repository.path.to_string_lossy().into_owned())
                .unwrap_or_default();
            transaction.execute(
                "INSERT INTO projects (id, name, repository, instructions, created_unix_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    project.id,
                    project.name,
                    first,
                    project.instructions,
                    project.created_unix_ms,
                ],
            )?;
            for (position, repository) in project.repositories.iter().enumerate() {
                transaction.execute(
                    "INSERT INTO project_repositories (project_id, position, name, path)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![
                        project.id,
                        i64::try_from(position).unwrap_or(i64::MAX),
                        repository.name,
                        repository.path.to_string_lossy(),
                    ],
                )?;
            }
            transaction.commit()
        })();
        added.map_err(self.failed())?;
        Ok(project)
    }

    /// Every project, oldest first.
    pub fn projects(&self) -> Result<Vec<Project>, StoreError> {
        let connection = self.lock();
        let listed = (|| -> rusqlite::Result<Vec<Project>> {
            let mut statement = connection.prepare(
                "SELECT id, name, instructions, created_unix_ms FROM projects
                 ORDER BY created_unix_ms, id",
            )?;
            let projects: Vec<Project> = statement
                .query_map([], project)?
                .collect::<Result<_, _>>()?;
            projects
                .into_iter()
                .map(|project| with_repositories(&connection, project))
                .collect()
        })();
        listed.map_err(self.failed())
    }

    pub fn project(&self, id: &str) -> Result<Project, StoreError> {
        let connection = self.lock();
        let found = connection
            .query_row(
                "SELECT id, name, instructions, created_unix_ms FROM projects WHERE id = ?1",
                [id],
                project,
            )
            .optional()
            .and_then(|found| {
                found
                    .map(|project| with_repositories(&connection, project))
                    .transpose()
            })
            .map_err(self.failed())?;
        found.ok_or_else(|| StoreError::UnknownProject { id: id.to_owned() })
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

/// A project's row, without its repositories.
fn project(row: &rusqlite::Row<'_>) -> rusqlite::Result<Project> {
    Ok(Project {
        id: row.get(0)?,
        name: row.get(1)?,
        repositories: Vec::new(),
        instructions: row.get(2)?,
        created_unix_ms: row.get(3)?,
    })
}

fn with_repositories(connection: &Connection, project: Project) -> rusqlite::Result<Project> {
    let mut statement = connection.prepare_cached(
        "SELECT name, path FROM project_repositories WHERE project_id = ?1 ORDER BY position",
    )?;
    let repositories = statement
        .query_map([&project.id], |row| {
            Ok(Repository {
                name: row.get(0)?,
                path: PathBuf::from(row.get::<_, String>(1)?),
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(Project {
        repositories,
        ..project
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

    fn repositories(path: &str) -> Vec<Repository> {
        vec![Repository {
            name: "code".into(),
            path: path.into(),
        }]
    }

    fn store(dir: &Path) -> Store {
        Store::open(&dir.join("khodr.sqlite3")).expect("the store opens")
    }

    #[test]
    fn projects_survive_reopening_in_the_order_they_were_added() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let first = store(dir.path())
            .add_project("one".into(), repositories("/code/one"), "be brief".into())
            .expect("added");
        let second = store(dir.path())
            .add_project("two".into(), repositories("/code/two"), String::new())
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
            .add_project("one".into(), repositories("/code/one"), String::new())
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
    fn a_project_from_the_first_schema_keeps_its_repository() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let path = dir.path().join("khodr.sqlite3");
        let connection = Connection::open(&path).expect("opens");
        connection
            .execute_batch(MIGRATIONS[0])
            .expect("the first schema");
        connection
            .pragma_update(None, "user_version", 1)
            .expect("set");
        connection
            .execute(
                "INSERT INTO projects VALUES ('p', 'one', '/code/app', '', 0)",
                [],
            )
            .expect("inserted");
        drop(connection);
        let project = Store::open(&path)
            .expect("opens")
            .project("p")
            .expect("found");
        assert_eq!(
            project.repositories,
            vec![Repository {
                name: "app".into(),
                path: "/code/app".into(),
            }]
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
