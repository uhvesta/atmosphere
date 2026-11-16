//! Script tokens (SEC-14), so a restart keeps them. Only the hash of a
//! token is saved, never the token; the entry is the bytes the daemon
//! encodes (what the token may do, and its name), which the store doesn't
//! read.

use rusqlite::params;

use crate::store::check_id;
use crate::{Store, StoreError};

/// One saved script token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedScriptToken {
    pub id: String,
    /// The hash of the token text.
    pub hash: String,
    /// What the token may do, as the daemon encoded it.
    pub entry: Vec<u8>,
    pub expires_unix_ms: i64,
    pub last_used_unix_ms: i64,
}

impl Store {
    /// Saves a script token, replacing the one with the same id.
    pub fn put_script_token(&self, token: &SavedScriptToken) -> Result<(), StoreError> {
        check_id(&token.id)?;
        let doing = "save a script token";
        self.inner().write("script_tokens.put", doing, |transaction| {
            transaction
                .execute(
                    "INSERT INTO script_tokens
                         (id, hash, entry, expires_unix_ms, last_used_unix_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT (id) DO UPDATE SET
                         hash = excluded.hash,
                         entry = excluded.entry,
                         expires_unix_ms = excluded.expires_unix_ms,
                         last_used_unix_ms = excluded.last_used_unix_ms",
                    params![
                        token.id,
                        token.hash,
                        token.entry,
                        token.expires_unix_ms,
                        token.last_used_unix_ms
                    ],
                )
                .map(drop)
        })
    }

    /// Every saved script token, by id.
    pub fn script_tokens(&self) -> Result<Vec<SavedScriptToken>, StoreError> {
        self.inner().read("list the script tokens", |connection| {
            let mut statement = connection.prepare(
                "SELECT id, hash, entry, expires_unix_ms, last_used_unix_ms
                 FROM script_tokens ORDER BY id",
            )?;
            statement
                .query_map([], |row| {
                    Ok(SavedScriptToken {
                        id: row.get(0)?,
                        hash: row.get(1)?,
                        entry: row.get(2)?,
                        expires_unix_ms: row.get(3)?,
                        last_used_unix_ms: row.get(4)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
    }

    /// Forgets a script token; false when there was none with that id.
    pub fn remove_script_token(&self, id: &str) -> Result<bool, StoreError> {
        check_id(id)?;
        let doing = "revoke a script token";
        self.inner()
            .write("script_tokens.remove", doing, |transaction| {
                transaction
                    .execute("DELETE FROM script_tokens WHERE id = ?1", params![id])
                    .map(|rows| rows > 0)
            })
    }

    /// Saves when a script token was last used.
    pub fn touch_script_token(&self, id: &str, last_used_unix_ms: i64) -> Result<(), StoreError> {
        check_id(id)?;
        let doing = "save when a script token was last used";
        self.inner()
            .write("script_tokens.touch", doing, |transaction| {
                transaction
                    .execute(
                        "UPDATE script_tokens SET last_used_unix_ms = ?2 WHERE id = ?1",
                        params![id, last_used_unix_ms],
                    )
                    .map(drop)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(id: &str, hash: &str) -> SavedScriptToken {
        SavedScriptToken {
            id: id.to_owned(),
            hash: hash.to_owned(),
            entry: vec![1, 2, 3],
            expires_unix_ms: 1_000,
            last_used_unix_ms: 0,
        }
    }

    #[test]
    fn tokens_survive_reopening_and_a_touch_changes_only_the_last_use() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        assert_eq!(store.script_tokens().unwrap(), Vec::new());
        store.put_script_token(&token("st_b", "hb")).unwrap();
        store.put_script_token(&token("st_a", "ha")).unwrap();
        store.touch_script_token("st_a", 77).unwrap();
        drop(store);

        let store = Store::open(dir.path()).unwrap();
        let mut touched = token("st_a", "ha");
        touched.last_used_unix_ms = 77;
        assert_eq!(
            store.script_tokens().unwrap(),
            vec![touched, token("st_b", "hb")]
        );
    }

    #[test]
    fn removing_says_whether_a_token_was_there() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        store.put_script_token(&token("st_a", "ha")).unwrap();

        assert!(store.remove_script_token("st_a").unwrap());
        assert!(!store.remove_script_token("st_a").unwrap());
        assert!(!store.remove_script_token("st_unknown").unwrap());
        assert_eq!(store.script_tokens().unwrap(), Vec::new());
    }
}
