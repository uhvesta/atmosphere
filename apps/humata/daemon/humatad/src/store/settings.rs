//! The daemon's own settings, and keeping the store small.

use rusqlite::{OptionalExtension, params};

use super::Store;
use crate::error::StoreError;

impl Store {
    pub fn setting(&self, key: &str) -> Result<Option<String>, StoreError> {
        self.lock()
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()
            .map_err(self.failed())
    }

    /// The disk space the search index takes in the store.
    pub fn search_bytes(&self) -> Result<u64, StoreError> {
        self.lock()
            .query_row(
                "SELECT COALESCE(SUM(pgsize), 0) FROM dbstat WHERE name LIKE 'search_entries%'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|bytes| u64::try_from(bytes).unwrap_or(0))
            .map_err(self.failed())
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), StoreError> {
        self.lock()
            .execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
            .map(drop)
            .map_err(self.failed())
    }

    /// Every pick of which skill wins: (name, candidates) to the winner's
    /// hash, the candidates' hashes sorted and joined by commas.
    pub fn skill_picks(
        &self,
    ) -> Result<std::collections::HashMap<(String, String), String>, StoreError> {
        let connection = self.lock();
        let mut statement = connection
            .prepare("SELECT name, candidates, winner FROM skill_picks")
            .map_err(self.failed())?;
        let rows = statement
            .query_map([], |row| Ok(((row.get(0)?, row.get(1)?), row.get(2)?)))
            .map_err(self.failed())?;
        rows.collect::<Result<_, _>>().map_err(self.failed())
    }

    pub fn pick_skill(&self, name: &str, candidates: &str, winner: &str) -> Result<(), StoreError> {
        self.lock()
            .execute(
                "INSERT INTO skill_picks (name, candidates, winner) VALUES (?1, ?2, ?3)
                 ON CONFLICT (name, candidates) DO UPDATE SET winner = excluded.winner",
                params![name, candidates, winner],
            )
            .map(drop)
            .map_err(self.failed())
    }

    /// Turns `plugin` of `agent` on or off for `project`'s sessions, or
    /// with None back to how its CLI has it.
    pub fn set_project_plugin(
        &self,
        project: &str,
        agent: i32,
        plugin: &str,
        enabled: Option<bool>,
    ) -> Result<(), StoreError> {
        self.project(project)?;
        let connection = self.lock();
        let set = match enabled {
            Some(enabled) => connection.execute(
                "INSERT INTO project_plugins (project_id, agent, plugin, enabled) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (project_id, agent, plugin) DO UPDATE SET enabled = excluded.enabled",
                params![project, agent, plugin, enabled],
            ),
            None => connection.execute(
                "DELETE FROM project_plugins WHERE project_id = ?1 AND agent = ?2 AND plugin = ?3",
                params![project, agent, plugin],
            ),
        };
        set.map(drop).map_err(self.failed())
    }

    /// What New Session starts from, for every project with `project`
    /// empty, else for it; None for none set.
    pub fn session_defaults(
        &self,
        project: &str,
    ) -> Result<Option<humata_wire::pb::SessionDefaults>, StoreError> {
        session_defaults(&self.lock(), project).map_err(self.failed())
    }

    /// Sets what New Session starts from, for every project with `project`
    /// empty, else for it; None takes them out.
    pub fn set_session_defaults(
        &self,
        project: &str,
        defaults: Option<&humata_wire::pb::SessionDefaults>,
    ) -> Result<(), StoreError> {
        if !project.is_empty() {
            self.project(project)?;
        }
        let connection = self.lock();
        let set = match defaults {
            Some(defaults) => connection.execute(
                "INSERT INTO session_defaults (project_id, backend, model, effort, experimental)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (project_id) DO UPDATE SET backend = excluded.backend,
                     model = excluded.model, effort = excluded.effort,
                     experimental = excluded.experimental",
                params![
                    project,
                    defaults.backend,
                    defaults.model,
                    defaults.effort,
                    defaults.experimental
                ],
            ),
            None => connection.execute(
                "DELETE FROM session_defaults WHERE project_id = ?1",
                [project],
            ),
        };
        set.map(drop).map_err(self.failed())
    }

    /// Keeps the newest `kept` board changes of each project, so older
    /// ones can't be undone, then gives the space freed back to the disk.
    pub fn compact(&self, kept: u32) -> Result<(), StoreError> {
        let mut connection = self.lock();
        let trimmed = (|| {
            let transaction = connection.transaction()?;
            transaction.execute(
                "CREATE TEMP TABLE IF NOT EXISTS dropped_changes (id INTEGER PRIMARY KEY)",
                [],
            )?;
            transaction.execute("DELETE FROM dropped_changes", [])?;
            transaction.execute(
                "INSERT INTO dropped_changes (id)
                     SELECT id FROM (
                         SELECT id, row_number() OVER (
                             PARTITION BY project_id ORDER BY id DESC) AS newer
                         FROM board_changes)
                     WHERE newer > ?1",
                [kept],
            )?;
            // A kept change that undid a dropped one just can't say which.
            transaction.execute(
                "UPDATE board_changes SET reverts = NULL
                 WHERE reverts IN (SELECT id FROM dropped_changes)",
                [],
            )?;
            transaction.execute(
                "DELETE FROM board_changes WHERE id IN (SELECT id FROM dropped_changes)",
                [],
            )?;
            transaction.execute("DELETE FROM dropped_changes", [])?;
            transaction.commit()
        })();
        trimmed.map_err(self.failed())?;
        connection
            .execute_batch("VACUUM; PRAGMA wal_checkpoint(TRUNCATE);")
            .map_err(self.failed())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_setting_reads_back_as_last_set() {
        let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let store = Store::open(&dir.path().join("store.sqlite3"))
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(store.setting("days").ok().flatten(), None);
        for value in ["30", "90"] {
            store
                .set_setting("days", value)
                .unwrap_or_else(|error| panic!("{error}"));
        }
        assert_eq!(store.setting("days").ok().flatten().as_deref(), Some("90"));
        store.compact(10).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(store.setting("days").ok().flatten().as_deref(), Some("90"));
    }
}

/// The plugins `project`'s sessions have on or off.
pub(super) fn project_plugins(
    connection: &rusqlite::Connection,
    project: &str,
) -> rusqlite::Result<Vec<humata_wire::pb::PluginChoice>> {
    let mut statement = connection.prepare_cached(
        "SELECT agent, plugin, enabled FROM project_plugins WHERE project_id = ?1
         ORDER BY agent, plugin",
    )?;
    let rows = statement.query_map([project], |row| {
        Ok(humata_wire::pb::PluginChoice {
            agent: row.get(0)?,
            id: row.get(1)?,
            enabled: row.get(2)?,
        })
    })?;
    rows.collect()
}

/// What New Session starts from in `project`, or in every project when
/// it's empty.
pub(super) fn session_defaults(
    connection: &rusqlite::Connection,
    project: &str,
) -> rusqlite::Result<Option<humata_wire::pb::SessionDefaults>> {
    connection
        .prepare_cached(
            "SELECT backend, model, effort, experimental FROM session_defaults
             WHERE project_id = ?1",
        )?
        .query_row([project], |row| {
            Ok(humata_wire::pb::SessionDefaults {
                backend: row.get(0)?,
                model: row.get(1)?,
                effort: row.get(2)?,
                experimental: row.get(3)?,
            })
        })
        .optional()
}
