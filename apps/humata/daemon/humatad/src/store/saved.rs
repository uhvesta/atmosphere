//! Saved repositories, to pick from when making or changing a project: the
//! ones projects were made or changed with, and those of the GitHub owners
//! the person added, as last listed. Projects keep their own copies, so
//! nothing here changes a project.

use std::collections::{BTreeMap, HashMap, HashSet};

use humata_agent::Repository;
use rusqlite::{OptionalExtension, Row, Transaction, params};

use super::{Store, now_unix_ms};
use crate::error::StoreError;
use crate::github::Repo;

/// A repository to pick when making or changing a project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedRepository {
    pub id: i64,
    pub name: String,
    /// A folder on this Mac, or a URL to clone.
    pub location: String,
    pub source: Source,
    /// The owner it was listed for, for one from GitHub.
    pub owner: Option<String>,
    pub added_unix_ms: i64,
    /// When a project was last made or changed with it.
    pub last_used_unix_ms: Option<i64>,
}

/// Where a saved repository came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// A project was made or changed with it.
    Used,
    /// It was listed on GitHub for one of the owners.
    GitHub,
}

impl Source {
    fn column(self) -> &'static str {
        match self {
            Self::Used => "used",
            Self::GitHub => "github",
        }
    }
}

/// A GitHub user or organization whose repositories are saved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Owner {
    pub login: String,
    pub added_unix_ms: i64,
    /// When its repositories were last listed.
    pub refreshed_unix_ms: Option<i64>,
    /// Why the last listing failed, for the person.
    pub error: Option<String>,
    /// How many of its repositories are saved and shown.
    pub repositories: u32,
}

/// One of an owner's repositories, as GitHub lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnedRepository {
    pub name: String,
    /// Its URL, to clone.
    pub location: String,
}

impl Store {
    /// Every saved repository the person hasn't removed, last used first,
    /// then by name; those never used last.
    pub fn saved_repositories(&self) -> Result<Vec<SavedRepository>, StoreError> {
        let connection = self.lock();
        let listed = (|| -> rusqlite::Result<Vec<SavedRepository>> {
            let mut statement = connection.prepare_cached(
                "SELECT id, name, location, source, owner, added_unix_ms, last_used_unix_ms
                 FROM repositories WHERE hidden = 0
                 ORDER BY last_used_unix_ms IS NULL, last_used_unix_ms DESC,
                     name COLLATE NOCASE, id",
            )?;
            statement.query_map([], saved_row)?.collect()
        })();
        listed.map_err(self.failed())
    }

    pub fn rename_saved_repository(&self, id: i64, name: &str) -> Result<(), StoreError> {
        let renamed = self
            .lock()
            .execute(
                "UPDATE repositories SET name = ?2 WHERE id = ?1 AND hidden = 0",
                params![id, name],
            )
            .map_err(self.failed())?;
        if renamed == 0 {
            return Err(StoreError::UnknownRepository { id });
        }
        Ok(())
    }

    /// Takes `id` out of the list. One from GitHub is kept hidden, so
    /// listing its owner again doesn't bring it back; one a project used
    /// is saved again when a project is next made or changed with it.
    pub fn remove_saved_repository(&self, id: i64) -> Result<(), StoreError> {
        let connection = self.lock();
        let removed = (|| -> rusqlite::Result<usize> {
            let hidden = connection.execute(
                "UPDATE repositories SET hidden = 1
                 WHERE id = ?1 AND source = 'github' AND hidden = 0",
                [id],
            )?;
            if hidden > 0 {
                return Ok(hidden);
            }
            connection.execute(
                "DELETE FROM repositories WHERE id = ?1 AND source = 'used'",
                [id],
            )
        })();
        if removed.map_err(self.failed())? == 0 {
            return Err(StoreError::UnknownRepository { id });
        }
        Ok(())
    }

    /// The owners, by login, each with how many of its repositories show.
    pub fn repository_owners(&self) -> Result<Vec<Owner>, StoreError> {
        let connection = self.lock();
        let listed = (|| -> rusqlite::Result<Vec<Owner>> {
            let mut statement = connection.prepare_cached(
                "SELECT owners.login, owners.added_unix_ms, owners.refreshed_unix_ms,
                     owners.error,
                     (SELECT count(*) FROM repositories
                      WHERE repositories.source = 'github' AND repositories.hidden = 0
                          AND repositories.owner = owners.login COLLATE NOCASE)
                 FROM repository_owners AS owners
                 ORDER BY owners.login COLLATE NOCASE",
            )?;
            statement
                .query_map([], |row| {
                    Ok(Owner {
                        login: row.get(0)?,
                        added_unix_ms: row.get(1)?,
                        refreshed_unix_ms: row.get(2)?,
                        error: row.get(3)?,
                        repositories: u32::try_from(row.get::<_, i64>(4)?).unwrap_or(u32::MAX),
                    })
                })?
                .collect()
        })();
        listed.map_err(self.failed())
    }

    /// Adds `login`, whose repositories are listed later; one there
    /// already, in any case, is left as it is.
    pub fn add_repository_owner(&self, login: &str) -> Result<(), StoreError> {
        self.lock()
            .execute(
                "INSERT INTO repository_owners (login, added_unix_ms) VALUES (?1, ?2)
                 ON CONFLICT (login) DO NOTHING",
                params![login, now_unix_ms()],
            )
            .map(drop)
            .map_err(self.failed())
    }

    /// `given` as the owner is spelled where it was added.
    pub fn repository_owner(&self, given: &str) -> Result<String, StoreError> {
        let found = self
            .lock()
            .query_row(
                "SELECT login FROM repository_owners WHERE login = ?1",
                [given],
                |row| row.get(0),
            )
            .optional()
            .map_err(self.failed())?;
        found.ok_or_else(|| StoreError::UnknownOwner {
            login: given.to_owned(),
        })
    }

    /// Removes `login` and the repositories listed for it; those a project
    /// used stay.
    pub fn remove_repository_owner(&self, login: &str) -> Result<(), StoreError> {
        let mut connection = self.lock();
        let removed = (|| -> rusqlite::Result<usize> {
            let transaction = connection.transaction()?;
            let removed =
                transaction.execute("DELETE FROM repository_owners WHERE login = ?1", [login])?;
            transaction.execute(
                "DELETE FROM repositories
                 WHERE source = 'github' AND owner = ?1 COLLATE NOCASE",
                [login],
            )?;
            transaction.commit()?;
            Ok(removed)
        })();
        if removed.map_err(self.failed())? == 0 {
            return Err(StoreError::UnknownOwner {
                login: login.to_owned(),
            });
        }
        Ok(())
    }

    /// The owners not listed since `before`, or never.
    pub fn repository_owners_due(&self, before: i64) -> Result<Vec<String>, StoreError> {
        let connection = self.lock();
        let listed = (|| -> rusqlite::Result<Vec<String>> {
            let mut statement = connection.prepare_cached(
                "SELECT login FROM repository_owners
                 WHERE refreshed_unix_ms IS NULL OR refreshed_unix_ms < ?1
                 ORDER BY login COLLATE NOCASE",
            )?;
            statement.query_map([before], |row| row.get(0))?.collect()
        })();
        listed.map_err(self.failed())
    }

    /// Keeps `repositories` as `login`'s on GitHub now: new ones are saved,
    /// those gone from GitHub are dropped, and those removed or renamed
    /// stay so. One a project used is saved as used, and not again. Does
    /// nothing if `login` was removed meanwhile.
    pub fn repository_owner_listed(
        &self,
        login: &str,
        repositories: &[OwnedRepository],
    ) -> Result<(), StoreError> {
        let now = now_unix_ms();
        let mut connection = self.lock();
        let kept = (|| -> rusqlite::Result<()> {
            let transaction = connection.transaction()?;
            let owner: Option<String> = transaction
                .query_row(
                    "SELECT login FROM repository_owners WHERE login = ?1",
                    [login],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(owner) = owner else {
                return Ok(());
            };
            let used: HashSet<String> = rows(
                &transaction,
                "SELECT id, location FROM repositories WHERE source = 'used'",
                &[],
            )?
            .into_iter()
            .map(|(_, location)| same_place(&location))
            .collect();
            let mut listed = HashSet::new();
            for repository in repositories {
                listed.insert(repository.location.clone());
                if used.contains(&same_place(&repository.location)) {
                    continue;
                }
                transaction.execute(
                    "INSERT INTO repositories (name, location, source, owner, added_unix_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT (location) DO UPDATE SET owner = excluded.owner
                         WHERE repositories.source = 'github'",
                    params![
                        repository.name,
                        repository.location,
                        Source::GitHub.column(),
                        owner,
                        now
                    ],
                )?;
            }
            for (id, location) in rows(
                &transaction,
                "SELECT id, location FROM repositories
                 WHERE source = 'github' AND owner = ?1 COLLATE NOCASE",
                &[&owner as &dyn rusqlite::ToSql],
            )? {
                if !listed.contains(&location) {
                    transaction.execute("DELETE FROM repositories WHERE id = ?1", [id])?;
                }
            }
            transaction.execute(
                "UPDATE repository_owners SET refreshed_unix_ms = ?2, error = NULL
                 WHERE login = ?1",
                params![owner, now],
            )?;
            transaction.commit()
        })();
        kept.map_err(self.failed())
    }

    /// Keeps why listing `login`'s repositories failed; those listed before
    /// stay.
    pub fn repository_owner_failed(&self, login: &str, error: &str) -> Result<(), StoreError> {
        self.lock()
            .execute(
                "UPDATE repository_owners SET error = ?2 WHERE login = ?1",
                params![login, error],
            )
            .map(drop)
            .map_err(self.failed())
    }
}

/// Saves each of `repositories` as used `now`: by the URL it was added by,
/// else its folder. A name the person gave it before is kept, and one from
/// GitHub that is the same repository becomes this.
pub(super) fn remember_used(
    transaction: &Transaction<'_>,
    repositories: &[Repository],
    urls: &BTreeMap<String, String>,
    now: i64,
) -> rusqlite::Result<()> {
    if repositories.is_empty() {
        return Ok(());
    }
    let mut github: HashMap<String, Vec<(i64, String)>> = HashMap::new();
    for (id, location) in rows(
        transaction,
        "SELECT id, location FROM repositories WHERE source = 'github'",
        &[],
    )? {
        github
            .entry(same_place(&location))
            .or_default()
            .push((id, location));
    }
    for repository in repositories {
        let location = urls
            .get(&repository.name)
            .map(|url| url.trim())
            .filter(|url| !url.is_empty())
            .map_or_else(
                || repository.path.to_string_lossy().into_owned(),
                str::to_owned,
            );
        // The same repository spelled another way, as with `.git` or over
        // SSH, gives way to this one.
        for (id, listed) in github.get(&same_place(&location)).into_iter().flatten() {
            if *listed != location {
                transaction.execute("DELETE FROM repositories WHERE id = ?1", [id])?;
            }
        }
        transaction.execute(
            "INSERT INTO repositories (name, location, source, added_unix_ms, last_used_unix_ms)
             VALUES (?1, ?2, ?3, ?4, ?4)
             ON CONFLICT (location) DO UPDATE SET source = excluded.source, owner = NULL,
                 last_used_unix_ms = excluded.last_used_unix_ms, hidden = 0",
            params![repository.name, location, Source::Used.column(), now],
        )?;
    }
    Ok(())
}

/// `location` as compared with others: a GitHub repository however its
/// URL is spelled, else the folder or URL without a trailing `/` or `.git`,
/// ignoring case.
fn same_place(location: &str) -> String {
    let location = location.trim();
    if let Some(repo) = Repo::of(location, false) {
        return format!("https://github.com/{repo}").to_lowercase();
    }
    let location = location.trim_end_matches('/');
    location
        .strip_suffix(".git")
        .unwrap_or(location)
        .to_lowercase()
}

/// The `(id, location)` rows `query` selects.
fn rows(
    transaction: &Transaction<'_>,
    query: &str,
    arguments: &[&dyn rusqlite::ToSql],
) -> rusqlite::Result<Vec<(i64, String)>> {
    let mut statement = transaction.prepare(query)?;
    let rows = statement.query_map(arguments, |row| Ok((row.get(0)?, row.get(1)?)))?;
    rows.collect()
}

fn saved_row(row: &Row<'_>) -> rusqlite::Result<SavedRepository> {
    let source: String = row.get(3)?;
    Ok(SavedRepository {
        id: row.get(0)?,
        name: row.get(1)?,
        location: row.get(2)?,
        source: if source == Source::GitHub.column() {
            Source::GitHub
        } else {
            Source::Used
        },
        owner: row.get(4)?,
        added_unix_ms: row.get(5)?,
        last_used_unix_ms: row.get(6)?,
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use rusqlite::Connection;

    use super::super::MIGRATIONS;
    use super::*;

    fn store(dir: &Path) -> Store {
        Store::open(&dir.join("humata.sqlite3")).expect("the store opens")
    }

    fn folder(name: &str, path: &str) -> Repository {
        Repository {
            name: name.to_owned(),
            path: path.into(),
        }
    }

    fn project(store: &Store, repositories: Vec<Repository>) -> String {
        store
            .add_project(
                "one".into(),
                repositories,
                BTreeMap::new(),
                String::new(),
                Vec::new(),
            )
            .expect("added")
            .id
    }

    fn locations(store: &Store) -> Vec<String> {
        store
            .saved_repositories()
            .expect("listed")
            .into_iter()
            .map(|saved| saved.location)
            .collect()
    }

    fn listed(name: &str) -> OwnedRepository {
        OwnedRepository {
            name: name.to_owned(),
            location: format!("https://github.com/acme/{name}"),
        }
    }

    #[test]
    fn the_projects_repositories_are_saved_when_the_store_is_brought_up_to_date() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let path = dir.path().join("humata.sqlite3");
        let before = MIGRATIONS
            .iter()
            .position(|step| step.contains("CREATE TABLE repositories ("))
            .expect("the step that saves repositories");
        let mut connection = Connection::open(&path).expect("opens");
        for step in &MIGRATIONS[..before] {
            connection.execute_batch(step).expect("an earlier schema");
        }
        let version = i64::try_from(before).expect("few steps");
        connection
            .pragma_update(None, "user_version", version)
            .expect("set");
        let transaction = connection.transaction().expect("a transaction");
        transaction
            .execute_batch(
                "INSERT INTO projects VALUES ('p', 'one', '/code/app', '', 10);
                 INSERT INTO projects VALUES ('q', 'two', '/code/app', '', 20);
                 INSERT INTO project_repositories VALUES ('p', 0, 'app', '/code/app', '');
                 INSERT INTO project_repositories VALUES ('q', 0, 'app', '/code/app', '');
                 INSERT INTO project_repositories
                     VALUES ('q', 1, 'web', '/cache/web', 'https://github.com/acme/web');",
            )
            .expect("projects");
        transaction.commit().expect("committed");
        drop(connection);

        let saved = store(dir.path()).saved_repositories().expect("listed");
        let app = saved
            .iter()
            .find(|saved| saved.location == "/code/app")
            .expect("the folder both projects have, once");
        assert_eq!(app.source, Source::Used);
        assert_eq!(app.added_unix_ms, 10);
        assert_eq!(app.last_used_unix_ms, Some(20));
        // One added by its URL is saved by it, not by the Mac's clone.
        assert!(
            saved
                .iter()
                .any(|saved| saved.location == "https://github.com/acme/web")
        );
        assert_eq!(saved.len(), 2);
    }

    #[test]
    fn making_or_changing_a_project_saves_its_repositories_as_last_used() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let store = store(dir.path());
        let first = project(&store, vec![folder("app", "/code/app")]);
        // Each is used at its own millisecond.
        std::thread::sleep(std::time::Duration::from_millis(2));
        project(&store, vec![folder("lib", "/code/lib")]);
        assert_eq!(locations(&store), ["/code/lib", "/code/app"]);

        let app = store.saved_repositories().expect("listed")[1].clone();
        store
            .rename_saved_repository(app.id, "the app")
            .expect("renamed");
        // Changing the first project uses its folder again, keeping the
        // name the person gave it.
        std::thread::sleep(std::time::Duration::from_millis(2));
        store
            .set_repositories(
                &first,
                &[folder("app", "/code/app"), folder("web", "/code/web")],
                &BTreeMap::new(),
            )
            .expect("set");
        let saved = store.saved_repositories().expect("listed");
        let app = saved
            .iter()
            .find(|saved| saved.location == "/code/app")
            .expect("saved");
        assert_eq!(app.name, "the app");
        assert!(app.last_used_unix_ms > saved[2].last_used_unix_ms);
        assert_eq!(saved[2].location, "/code/lib");
    }

    #[test]
    fn removing_or_renaming_a_saved_repository_leaves_the_projects_alone() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let store = store(dir.path());
        let id = project(&store, vec![folder("app", "/code/app")]);
        let saved = store.saved_repositories().expect("listed")[0].clone();
        store
            .rename_saved_repository(saved.id, "renamed")
            .expect("renamed");
        store.remove_saved_repository(saved.id).expect("removed");
        assert!(locations(&store).is_empty());
        assert_eq!(
            store.project(&id).expect("found").repositories,
            [folder("app", "/code/app")]
        );
        assert!(matches!(
            store.remove_saved_repository(saved.id),
            Err(StoreError::UnknownRepository { .. })
        ));
    }

    #[test]
    fn a_repository_removed_from_an_owners_stays_out_when_listed_again() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let store = store(dir.path());
        store.add_repository_owner("acme").expect("added");
        store
            .repository_owner_listed("acme", &[listed("app"), listed("lib")])
            .expect("kept");
        let lib = store
            .saved_repositories()
            .expect("listed")
            .into_iter()
            .find(|saved| saved.name == "lib")
            .expect("listed");
        assert_eq!(lib.source, Source::GitHub);
        assert_eq!(lib.owner.as_deref(), Some("acme"));
        store.remove_saved_repository(lib.id).expect("removed");
        store
            .repository_owner_listed("acme", &[listed("app"), listed("lib")])
            .expect("kept");
        assert_eq!(locations(&store), ["https://github.com/acme/app"]);
    }

    #[test]
    fn a_repository_gone_from_github_is_dropped_when_its_owner_is_listed_again() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let store = store(dir.path());
        store.add_repository_owner("acme").expect("added");
        store
            .repository_owner_listed("acme", &[listed("app"), listed("old")])
            .expect("kept");
        store
            .repository_owner_listed("acme", &[listed("app")])
            .expect("kept");
        assert_eq!(locations(&store), ["https://github.com/acme/app"]);
        let owners = store.repository_owners().expect("listed");
        assert_eq!(owners[0].repositories, 1);
        assert!(owners[0].refreshed_unix_ms.is_some());
        assert_eq!(owners[0].error, None);
    }

    #[test]
    fn removing_an_owner_drops_only_its_listed_repositories() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let store = store(dir.path());
        store.add_repository_owner("acme").expect("added");
        store.add_repository_owner("other").expect("added");
        store
            .repository_owner_listed("acme", &[listed("app"), listed("web")])
            .expect("kept");
        store
            .repository_owner_listed(
                "other",
                &[OwnedRepository {
                    name: "tool".to_owned(),
                    location: "https://github.com/other/tool".to_owned(),
                }],
            )
            .expect("kept");
        // A project uses one of acme's, by its URL as cloned.
        store
            .add_project(
                "one".into(),
                vec![folder("web", "/cache/web")],
                BTreeMap::from([(
                    "web".to_owned(),
                    "https://github.com/acme/web.git".to_owned(),
                )]),
                String::new(),
                Vec::new(),
            )
            .expect("added");

        store.remove_repository_owner("ACME").expect("removed");
        let mut left = locations(&store);
        left.sort();
        assert_eq!(
            left,
            [
                "https://github.com/acme/web.git",
                "https://github.com/other/tool"
            ]
        );
        assert!(matches!(
            store.remove_repository_owner("acme"),
            Err(StoreError::UnknownOwner { .. })
        ));
    }

    #[test]
    fn a_repository_a_project_used_isnt_listed_again_for_its_owner() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let store = store(dir.path());
        store
            .add_project(
                "one".into(),
                vec![folder("app", "/cache/app")],
                BTreeMap::from([("app".to_owned(), "git@github.com:Acme/app.git".to_owned())]),
                String::new(),
                Vec::new(),
            )
            .expect("added");
        store.add_repository_owner("acme").expect("added");
        store
            .repository_owner_listed("acme", &[listed("app"), listed("lib")])
            .expect("kept");
        let saved = store.saved_repositories().expect("listed");
        assert_eq!(saved.len(), 2);
        assert_eq!(saved[0].location, "git@github.com:Acme/app.git");
        assert_eq!(saved[0].source, Source::Used);
        assert_eq!(saved[1].source, Source::GitHub);
    }

    #[test]
    fn a_listing_for_an_owner_removed_meanwhile_saves_nothing() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let store = store(dir.path());
        store
            .repository_owner_listed("acme", &[listed("app")])
            .expect("kept");
        assert!(locations(&store).is_empty());
    }
}
