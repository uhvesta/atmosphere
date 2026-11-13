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
