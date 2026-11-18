//! Settings the user set (ST-13, D196): a text value per key with the
//! revision it was saved at. A key with no row has its default.

use rusqlite::params;

use crate::store::{from_sql, to_sql};
use crate::{Store, StoreError};

/// One saved setting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedSetting {
    pub value: String,
    pub revision: u64,
}

impl Store {
    /// The saved value of `key`, if the user ever set it.
    pub fn setting(&self, key: &str) -> Result<Option<SavedSetting>, StoreError> {
        self.inner().read("read a setting", |connection| {
            let mut statement = connection.prepare("SELECT value, revision FROM settings WHERE key = ?1")?;
            let mut rows = statement.query_map([key], |row| {
                Ok(SavedSetting {
                    value: row.get(0)?,
                    revision: from_sql(row.get(1)?),
                })
            })?;
            rows.next().transpose()
        })
    }

    /// Saves `value` for `key` at `revision`, replacing the one saved before.
    pub fn put_setting(&self, key: &str, value: &str, revision: u64) -> Result<(), StoreError> {
        self.inner().write("setting.put", "save a setting", |transaction| {
            transaction
                .execute(
                    "INSERT INTO settings (key, value, revision) VALUES (?1, ?2, ?3)
                     ON CONFLICT (key) DO UPDATE SET
                         value = excluded.value, revision = excluded.revision",
                    params![key, value, to_sql(revision)],
                )
                .map(drop)
        })
    }

    /// Forgets `key`, so it has its default again. Forgetting a key with no
    /// row does nothing.
    pub fn delete_setting(&self, key: &str) -> Result<(), StoreError> {
        self.inner().write("setting.delete", "delete a setting", |transaction| {
            transaction
                .execute("DELETE FROM settings WHERE key = ?1", [key])
                .map(drop)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_setting_is_absent_until_saved_and_then_replaced() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path()).unwrap();
        assert_eq!(store.setting("branch_prefix").unwrap(), None);
        store.put_setting("branch_prefix", "me/", 1).unwrap();
        store.put_setting("branch_prefix", "", 2).unwrap();
        assert_eq!(
            store.setting("branch_prefix").unwrap(),
            Some(SavedSetting { value: String::new(), revision: 2 })
        );
    }

    #[test]
    fn a_deleted_setting_is_absent_and_the_others_stay() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path()).unwrap();
        store.put_setting("old", "true", 1).unwrap();
        store.put_setting("kept", "x", 1).unwrap();
        store.delete_setting("old").unwrap();
        store.delete_setting("never-saved").unwrap();
        assert_eq!(store.setting("old").unwrap(), None);
        assert_eq!(store.setting("kept").unwrap().map(|saved| saved.value).as_deref(), Some("x"));
    }
}
