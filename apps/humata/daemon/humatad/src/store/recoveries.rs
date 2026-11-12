//! The Mac's recovery copies of sessions on other machines, and the hosts
//! sessions were moved away from.

use humata_wire::pb;
use prost::Message;
use rusqlite::{OptionalExtension, params};

use super::{Store, now_unix_ms};
use crate::error::StoreError;

/// A recovery copy as listed, without the copy itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recovery {
    pub session: String,
    pub host: String,
    pub name: String,
    pub turns: u32,
    pub backed_up_unix_ms: i64,
}

impl Store {
    /// Keeps `backup` as the recovery copy of `session` on `host`, in place
    /// of the last.
    pub fn keep_recovery(
        &self,
        session: &str,
        host: &str,
        name: &str,
        backup: &pb::SessionBackup,
    ) -> Result<(), StoreError> {
        let compressed = ruzstd::encoding::compress_to_vec(
            backup.encode_to_vec().as_slice(),
            ruzstd::encoding::CompressionLevel::Fastest,
        );
        self.lock()
            .execute(
                "INSERT INTO recoveries (session_id, host, name, turns, backup, backed_up_unix_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT (session_id) DO UPDATE SET host = excluded.host,
                     name = excluded.name, turns = excluded.turns, backup = excluded.backup,
                     backed_up_unix_ms = excluded.backed_up_unix_ms",
                params![session, host, name, backup.turns, compressed, now_unix_ms()],
            )
            .map(drop)
            .map_err(self.failed())
    }

    /// Every recovery copy, by host and then name.
    pub fn recoveries(&self) -> Result<Vec<Recovery>, StoreError> {
        let connection = self.lock();
        let mut statement = connection
            .prepare(
                "SELECT session_id, host, name, turns, backed_up_unix_ms FROM recoveries
                 ORDER BY host, name, session_id",
            )
            .map_err(self.failed())?;
        statement
            .query_map([], |row| {
                Ok(Recovery {
                    session: row.get(0)?,
                    host: row.get(1)?,
                    name: row.get(2)?,
                    turns: row.get(3)?,
                    backed_up_unix_ms: row.get(4)?,
                })
            })
            .map_err(self.failed())?
            .collect::<Result<_, _>>()
            .map_err(self.failed())
    }

    /// How many turns `session`'s recovery copy has; None without one.
    pub fn recovery_turns(&self, session: &str) -> Result<Option<u32>, StoreError> {
        self.lock()
            .query_row(
                "SELECT turns FROM recoveries WHERE session_id = ?1",
                [session],
                |row| row.get(0),
            )
            .optional()
            .map_err(self.failed())
    }

    /// `session`'s recovery copy and the host it is of; None without one,
    /// or if this build can't read it.
    pub fn recovery(
        &self,
        session: &str,
    ) -> Result<Option<(String, pb::SessionBackup)>, StoreError> {
        let found: Option<(String, Vec<u8>)> = self
            .lock()
            .query_row(
                "SELECT host, backup FROM recoveries WHERE session_id = ?1",
                [session],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(self.failed())?;
        Ok(found.and_then(|(host, compressed)| {
            let mut bytes = Vec::new();
            let mut decoder =
                ruzstd::decoding::StreamingDecoder::new(compressed.as_slice()).ok()?;
            std::io::Read::read_to_end(&mut decoder, &mut bytes).ok()?;
            let backup = pb::SessionBackup::decode(bytes.as_slice()).ok()?;
            Some((host, backup))
        }))
    }

    /// Records that `session` now runs on `to`, moved from `from`, whose
    /// copy of it is left out from now on.
    pub fn move_host(&self, session: &str, from: &str, to: &str) -> Result<(), StoreError> {
        let mut connection = self.lock();
        let transaction = connection.transaction().map_err(self.failed())?;
        let moved = (|| {
            transaction.execute(
                "DELETE FROM moved_sessions WHERE session_id = ?1 AND host = ?2",
                params![session, to],
            )?;
            if from != to {
                transaction.execute(
                    "INSERT OR IGNORE INTO moved_sessions (session_id, host) VALUES (?1, ?2)",
                    params![session, from],
                )?;
            }
            transaction.execute(
                "UPDATE recoveries SET host = ?2 WHERE session_id = ?1",
                params![session, to],
            )?;
            transaction.execute(
                "UPDATE sessions SET host = ?2 WHERE id = ?1",
                params![session, to],
            )?;
            transaction.execute("DELETE FROM remote_sessions WHERE id = ?1", [session])?;
            Ok(())
        })();
        moved.map_err(self.failed())?;
        transaction.commit().map_err(self.failed())
    }

    /// Whether `session` was moved away from `host`.
    pub fn moved_away(&self, session: &str, host: &str) -> Result<bool, StoreError> {
        let connection = self.lock();
        connection
            .prepare_cached("SELECT 1 FROM moved_sessions WHERE session_id = ?1 AND host = ?2")
            .and_then(|mut statement| {
                statement
                    .query_row(params![session, host], |_| Ok(()))
                    .optional()
            })
            .map(|found| found.is_some())
            .map_err(self.failed())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn backup(turns: u32) -> pb::SessionBackup {
        pb::SessionBackup {
            record: b"{}".to_vec(),
            turns,
            ..Default::default()
        }
    }

    #[test]
    fn the_last_recovery_copy_is_kept_and_follows_a_move() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let store = Store::open(&dir.path().join("humata.sqlite3")).expect("the store opens");
        store
            .keep_recovery("s", "old", "chat", &backup(1))
            .expect("kept");
        store
            .keep_recovery("s", "old", "chat", &backup(2))
            .expect("kept");
        assert_eq!(store.recovery_turns("s").expect("read"), Some(2));
        assert_eq!(
            store.recovery("s").expect("read"),
            Some(("old".into(), backup(2)))
        );

        store.move_host("s", "old", "new").expect("moved");
        assert!(store.moved_away("s", "old").expect("read"));
        assert!(!store.moved_away("s", "new").expect("read"));
        let listed = store.recoveries().expect("listed");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].host, "new");

        store.move_host("s", "new", "old").expect("moved back");
        assert!(!store.moved_away("s", "old").expect("read"));
        assert!(store.moved_away("s", "new").expect("read"));
    }
}
