//! The Mac's recovery copies of sessions on other machines, and the hosts
//! sessions were moved away from.

use std::collections::HashSet;

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

/// The events a backup brought: how many, and the id of the last.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NewEvents {
    pub count: u32,
    pub last_id: String,
}

fn compress(bytes: &[u8]) -> Vec<u8> {
    ruzstd::encoding::compress_to_vec(bytes, ruzstd::encoding::CompressionLevel::Fastest)
}

fn decompress(compressed: &[u8]) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut decoder = ruzstd::decoding::StreamingDecoder::new(compressed).ok()?;
    std::io::Read::read_to_end(&mut decoder, &mut bytes).ok()?;
    Some(bytes)
}

impl Store {
    /// Keeps `backup` as the recovery copy of `session` on `host`, in place
    /// of the last. Its events are the history from `backup.events_from`
    /// on, `events` of them ending at `last_event_id`: from 0 they replace
    /// the history kept, otherwise they are added to it. Returns false,
    /// keeping nothing, when they don't start where the kept history ends,
    /// so the backup is asked for again whole.
    pub fn keep_recovery(
        &self,
        session: &str,
        host: &str,
        name: &str,
        backup: &pb::SessionBackup,
        new_events: &NewEvents,
    ) -> Result<bool, StoreError> {
        let rest = pb::SessionBackup {
            events: Vec::new(),
            events_from: 0,
            backend_files: Vec::new(),
            backend_unchanged: Vec::new(),
            ..backup.clone()
        };
        let files: Vec<(&str, Vec<u8>, Vec<u8>)> = backup
            .backend_files
            .iter()
            .map(|file| {
                let sha256 = <sha2::Sha256 as sha2::Digest>::digest(&file.contents);
                (
                    file.name.as_str(),
                    sha256.to_vec(),
                    compress(&file.contents),
                )
            })
            .collect();
        let compressed = compress(&rest.encode_to_vec());
        let chunk = compress(&backup.events);
        let mut connection = self.lock();
        let transaction = connection.transaction().map_err(self.failed())?;
        let kept = (|| {
            let (have, parts): (u32, u32) = transaction.query_row(
                "SELECT COALESCE(SUM(events), 0), COUNT(*) FROM recovery_events WHERE session_id = ?1",
                [session],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            let part = if backup.events_from == 0 {
                transaction.execute(
                    "DELETE FROM recovery_events WHERE session_id = ?1",
                    [session],
                )?;
                0
            } else if backup.events_from == have && parts > 0 {
                parts
            } else {
                return Ok(false);
            };
            // Files named unchanged must be in the copy still; the rest
            // that aren't sent are gone from the host.
            let kept_files: HashSet<String> = {
                let mut statement = transaction
                    .prepare_cached("SELECT name FROM recovery_files WHERE session_id = ?1")?;
                statement
                    .query_map([session], |row| row.get(0))?
                    .collect::<rusqlite::Result<_>>()?
            };
            if !backup
                .backend_unchanged
                .iter()
                .all(|name| kept_files.contains(name))
            {
                return Ok(false);
            }
            for name in &kept_files {
                let sent = backup.backend_files.iter().any(|file| &file.name == name);
                if !sent && !backup.backend_unchanged.contains(name) {
                    transaction.execute(
                        "DELETE FROM recovery_files WHERE session_id = ?1 AND name = ?2",
                        params![session, name],
                    )?;
                }
            }
            for (name, sha256, contents) in &files {
                transaction.execute(
                    "INSERT INTO recovery_files (session_id, name, sha256, contents)
                     VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT (session_id, name) DO UPDATE SET sha256 = excluded.sha256,
                         contents = excluded.contents",
                    params![session, name, sha256, contents],
                )?;
            }
            if part == 0 || new_events.count > 0 {
                transaction.execute(
                    "INSERT INTO recovery_events (session_id, part, events, last_event_id, chunk)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![session, part, new_events.count, new_events.last_id, chunk],
                )?;
            }
            transaction.execute(
                "INSERT INTO recoveries (session_id, host, name, turns, backup, backed_up_unix_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT (session_id) DO UPDATE SET host = excluded.host,
                     name = excluded.name, turns = excluded.turns, backup = excluded.backup,
                     backed_up_unix_ms = excluded.backed_up_unix_ms",
                params![session, host, name, backup.turns, compressed, now_unix_ms()],
            )?;
            Ok(true)
        })();
        let kept = kept.map_err(self.failed())?;
        if kept {
            transaction.commit().map_err(self.failed())?;
        }
        Ok(kept)
    }

    /// The backend files `session`'s recovery copy has, by name, with the
    /// SHA-256 of each.
    pub fn recovery_file_hashes(
        &self,
        session: &str,
    ) -> Result<Vec<pb::BackendFileHash>, StoreError> {
        let connection = self.lock();
        let read = (|| {
            let mut statement = connection.prepare_cached(
                "SELECT name, sha256 FROM recovery_files WHERE session_id = ?1 ORDER BY name",
            )?;
            statement
                .query_map([session], |row| {
                    Ok(pb::BackendFileHash {
                        name: row.get(0)?,
                        sha256: row.get(1)?,
                    })
                })?
                .collect::<rusqlite::Result<_>>()
        })();
        read.map_err(self.failed())
    }

    /// How many events of `session`'s history its recovery copy has, and
    /// the id of the last; (0, "") without one, or for a copy kept whole.
    pub fn recovery_events(&self, session: &str) -> Result<(u32, String), StoreError> {
        self.lock()
            .query_row(
                "SELECT COALESCE(SUM(events), 0),
                     COALESCE((SELECT last_event_id FROM recovery_events WHERE session_id = ?1
                               ORDER BY part DESC LIMIT 1), '')
                 FROM recovery_events WHERE session_id = ?1",
                [session],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
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
        let connection = self.lock();
        let read = (|| {
            let found: Option<(String, Vec<u8>)> = connection
                .query_row(
                    "SELECT host, backup FROM recoveries WHERE session_id = ?1",
                    [session],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let mut statement = connection.prepare_cached(
                "SELECT chunk FROM recovery_events WHERE session_id = ?1 ORDER BY part",
            )?;
            let chunks = statement
                .query_map([session], |row| row.get::<_, Vec<u8>>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut statement = connection.prepare_cached(
                "SELECT name, contents FROM recovery_files WHERE session_id = ?1 ORDER BY name",
            )?;
            let files = statement
                .query_map([session], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok((found, chunks, files))
        })();
        let (found, chunks, files) = read.map_err(self.failed())?;
        drop(connection);
        Ok(found.and_then(|(host, compressed)| {
            let mut backup = pb::SessionBackup::decode(decompress(&compressed)?.as_slice()).ok()?;
            if !chunks.is_empty() {
                let mut events: Vec<serde_json::Value> = Vec::new();
                for chunk in &chunks {
                    let part: Vec<serde_json::Value> =
                        serde_json::from_slice(&decompress(chunk)?).ok()?;
                    events.extend(part);
                }
                backup.events = serde_json::to_vec(&events).ok()?;
            }
            // A copy kept before its files were apart has them in it.
            for (name, contents) in &files {
                backup.backend_files.push(pb::BackendFile {
                    name: name.clone(),
                    contents: decompress(contents)?,
                });
            }
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
            transaction.execute("DELETE FROM remote_histories WHERE id = ?1", [session])?;
            Ok(())
        })();
        moved.map_err(self.failed())?;
        transaction.commit().map_err(self.failed())
    }

    /// The sessions moved away from `host`.
    pub fn moved_away_from(&self, host: &str) -> Result<HashSet<String>, StoreError> {
        let connection = self.lock();
        let read = (|| {
            let mut statement = connection
                .prepare_cached("SELECT session_id FROM moved_sessions WHERE host = ?1")?;
            let rows = statement.query_map([host], |row| row.get::<_, String>(0))?;
            rows.collect::<rusqlite::Result<HashSet<String>>>()
        })();
        read.map_err(self.failed())
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
            events: b"[]".to_vec(),
            turns,
            ..Default::default()
        }
    }

    /// A backup bringing events `ids`, which start at `from`.
    fn events(from: u32, ids: &[&str]) -> (pb::SessionBackup, NewEvents) {
        let events: Vec<_> = ids
            .iter()
            .map(|id| serde_json::json!({ "id": id }))
            .collect();
        let backup = pb::SessionBackup {
            record: b"{}".to_vec(),
            events: serde_json::to_vec(&events).expect("encoded"),
            events_from: from,
            ..Default::default()
        };
        let new = NewEvents {
            count: u32::try_from(ids.len()).expect("few"),
            last_id: ids.last().map(|id| (*id).to_owned()).unwrap_or_default(),
        };
        (backup, new)
    }

    fn kept_ids(store: &Store) -> Vec<String> {
        let (_, backup) = store.recovery("s").expect("read").expect("a copy");
        let events: Vec<serde_json::Value> = serde_json::from_slice(&backup.events).expect("JSON");
        events
            .iter()
            .map(|event| event["id"].as_str().expect("an id").to_owned())
            .collect()
    }

    #[test]
    fn a_backup_adds_its_new_events_to_the_copy() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let store = Store::open(&dir.path().join("humata.sqlite3")).expect("the store opens");
        let (whole, new) = events(0, &["a", "b"]);
        assert!(
            store
                .keep_recovery("s", "far", "chat", &whole, &new)
                .expect("kept")
        );
        assert_eq!(
            store.recovery_events("s").expect("read"),
            (2, "b".to_owned())
        );

        let (later, new) = events(2, &["c"]);
        assert!(
            store
                .keep_recovery("s", "far", "chat", &later, &new)
                .expect("kept")
        );
        assert_eq!(kept_ids(&store), ["a", "b", "c"]);
        assert_eq!(
            store.recovery_events("s").expect("read"),
            (3, "c".to_owned())
        );

        // Nothing new adds nothing.
        let (none, new) = events(3, &[]);
        assert!(
            store
                .keep_recovery("s", "far", "chat", &none, &new)
                .expect("kept")
        );
        assert_eq!(kept_ids(&store), ["a", "b", "c"]);
    }

    #[test]
    fn backend_files_named_unchanged_are_kept_and_unnamed_ones_dropped() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let store = Store::open(&dir.path().join("humata.sqlite3")).expect("the store opens");
        let file = |name: &str, contents: &[u8]| pb::BackendFile {
            name: name.to_owned(),
            contents: contents.to_vec(),
        };
        let files = |store: &Store| {
            let (_, backup) = store.recovery("s").expect("read").expect("a copy");
            backup.backend_files
        };
        let mut first = backup(1);
        first.backend_files = vec![file("a", b"one"), file("b", b"two")];
        assert!(
            store
                .keep_recovery("s", "far", "chat", &first, &NewEvents::default())
                .expect("kept")
        );
        assert_eq!(store.recovery_file_hashes("s").expect("read").len(), 2);

        // "a" changed, "b" didn't.
        let mut second = backup(2);
        second.backend_files = vec![file("a", b"three")];
        second.backend_unchanged = vec!["b".to_owned()];
        assert!(
            store
                .keep_recovery("s", "far", "chat", &second, &NewEvents::default())
                .expect("kept")
        );
        assert_eq!(files(&store), [file("a", b"three"), file("b", b"two")]);

        // "b" is gone from the host; "c" was never kept, so it's asked again.
        let mut third = backup(3);
        third.backend_unchanged = vec!["c".to_owned()];
        assert!(
            !store
                .keep_recovery("s", "far", "chat", &third, &NewEvents::default())
                .expect("refused")
        );
        let mut fourth = backup(3);
        fourth.backend_unchanged = vec!["a".to_owned()];
        assert!(
            store
                .keep_recovery("s", "far", "chat", &fourth, &NewEvents::default())
                .expect("kept")
        );
        assert_eq!(files(&store), [file("a", b"three")]);
    }

    #[test]
    fn events_that_dont_follow_on_are_refused_and_a_whole_history_replaces() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let store = Store::open(&dir.path().join("humata.sqlite3")).expect("the store opens");
        let (whole, new) = events(0, &["a", "b"]);
        assert!(
            store
                .keep_recovery("s", "far", "chat", &whole, &new)
                .expect("kept")
        );

        let (gap, new) = events(5, &["f"]);
        assert!(
            !store
                .keep_recovery("s", "far", "chat", &gap, &new)
                .expect("refused")
        );
        assert_eq!(kept_ids(&store), ["a", "b"]);

        // As after a rewind: the whole history again, shorter.
        let (rewound, new) = events(0, &["a"]);
        assert!(
            store
                .keep_recovery("s", "far", "chat", &rewound, &new)
                .expect("kept")
        );
        assert_eq!(kept_ids(&store), ["a"]);
    }

    #[test]
    fn the_last_recovery_copy_is_kept_and_follows_a_move() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let store = Store::open(&dir.path().join("humata.sqlite3")).expect("the store opens");
        store
            .keep_recovery("s", "old", "chat", &backup(1), &NewEvents::default())
            .expect("kept");
        store
            .keep_recovery("s", "old", "chat", &backup(2), &NewEvents::default())
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
