//! The Agent access rules the user set (IN-17, IN-21, ST-4), so a restart
//! keeps them. Each channel's rules and the defaults new channels start
//! with are saved as the bytes the daemon encodes, with the revision they
//! were saved at; the store doesn't read them.

use rusqlite::params;

use crate::store::{check_id, to_sql};
use crate::{Store, StoreError};

/// One channel's saved rules, or the saved defaults.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedRules {
    pub revision: u64,
    /// The rules as the daemon encoded them.
    pub rules: Vec<u8>,
}

impl Store {
    /// Saves a channel's rules, replacing the ones saved before.
    pub fn put_channel_rules(&self, channel: &str, saved: &SavedRules) -> Result<(), StoreError> {
        check_id(channel)?;
        let inner = self.inner();
        let doing = "save a channel's access rules";
        inner.write("rules.put_channel", doing, |transaction| {
            transaction
                .execute(
                    "INSERT INTO access_channel_rules (channel, revision, rules)
                     VALUES (?1, ?2, ?3)
                     ON CONFLICT (channel) DO UPDATE SET
                         revision = excluded.revision, rules = excluded.rules",
                    params![channel, to_sql(saved.revision), saved.rules],
                )
                .map(drop)
        })
    }

    /// Every channel's saved rules, by channel id.
    pub fn channel_rules(&self) -> Result<Vec<(String, SavedRules)>, StoreError> {
        let rows = self
            .inner()
            .read("list the channels' access rules", |connection| {
                let mut statement = connection.prepare(
                    "SELECT channel, revision, rules FROM access_channel_rules ORDER BY channel",
                )?;
                statement
                    .query_map([], |row| {
                        let revision: i64 = row.get(1)?;
                        Ok((
                            row.get::<_, String>(0)?,
                            SavedRules {
                                revision: u64::try_from(revision).unwrap_or(0),
                                rules: row.get(2)?,
                            },
                        ))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()
            })?;
        Ok(rows)
    }

    /// Forgets a deleted channel's rules.
    pub fn remove_channel_rules(&self, channel: &str) -> Result<(), StoreError> {
        check_id(channel)?;
        let doing = "forget a channel's access rules";
        self.inner()
            .write("rules.remove_channel", doing, |transaction| {
                transaction
                    .execute(
                        "DELETE FROM access_channel_rules WHERE channel = ?1",
                        params![channel],
                    )
                    .map(drop)
            })
    }

    /// Saves the rules new channels start with.
    pub fn put_default_rules(&self, saved: &SavedRules) -> Result<(), StoreError> {
        let doing = "save the default access rules";
        self.inner()
            .write("rules.put_default", doing, |transaction| {
                transaction
                    .execute(
                        "INSERT INTO access_default_rules (id, revision, rules) VALUES (1, ?1, ?2)
                     ON CONFLICT (id) DO UPDATE SET
                         revision = excluded.revision, rules = excluded.rules",
                        params![to_sql(saved.revision), saved.rules],
                    )
                    .map(drop)
            })
    }

    /// The saved default rules, if the user ever set them.
    pub fn default_rules(&self) -> Result<Option<SavedRules>, StoreError> {
        self.inner()
            .read("read the default access rules", |connection| {
                let mut statement = connection
                    .prepare("SELECT revision, rules FROM access_default_rules WHERE id = 1")?;
                let mut rows = statement.query_map([], |row| {
                    let revision: i64 = row.get(0)?;
                    Ok(SavedRules {
                        revision: u64::try_from(revision).unwrap_or(0),
                        rules: row.get(1)?,
                    })
                })?;
                rows.next().transpose()
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved(revision: u64, rules: &[u8]) -> SavedRules {
        SavedRules {
            revision,
            rules: rules.to_vec(),
        }
    }

    #[test]
    fn channel_rules_survive_reopening_and_a_rewrite_replaces_them() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        assert_eq!(store.channel_rules().unwrap(), Vec::new());
        store.put_channel_rules("ch_b", &saved(1, b"one")).unwrap();
        store
            .put_channel_rules("ch_a", &saved(3, b"three"))
            .unwrap();
        store.put_channel_rules("ch_b", &saved(2, b"two")).unwrap();
        drop(store);

        let store = Store::open(dir.path()).unwrap();
        assert_eq!(
            store.channel_rules().unwrap(),
            vec![
                ("ch_a".to_owned(), saved(3, b"three")),
                ("ch_b".to_owned(), saved(2, b"two"))
            ]
        );
        store.remove_channel_rules("ch_a").unwrap();
        assert_eq!(
            store.channel_rules().unwrap(),
            vec![("ch_b".to_owned(), saved(2, b"two"))]
        );
    }

    #[test]
    fn default_rules_are_none_until_saved_then_survive_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        assert_eq!(store.default_rules().unwrap(), None);
        store.put_default_rules(&saved(1, b"a")).unwrap();
        store.put_default_rules(&saved(2, b"b")).unwrap();
        drop(store);

        let store = Store::open(dir.path()).unwrap();
        assert_eq!(store.default_rules().unwrap(), Some(saved(2, b"b")));
    }

    #[test]
    fn a_channel_id_with_a_slash_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        assert!(matches!(
            store.put_channel_rules("a/b", &saved(1, b"x")),
            Err(StoreError::InvalidId { .. })
        ));
    }
}
