//! Writes kept on a machine until the host has them (SL-1).

use std::sync::Arc;

use rusqlite::{OptionalExtension, params};

use crate::store::{Inner, unix_ms};
use crate::{StoreError, blob};

/// The blob type journal payloads are kept as: the caller's own bytes,
/// whatever `kind` says they are.

/// One write waiting for the host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalEntry {
    /// Counts up from 1 and is never reused, even after entries are acked.
    pub seq: u64,
    pub kind: String,
    pub written_unix_ms: i64,
    pub payload: Vec<u8>,
}

/// An append-only list of entries in the `journal` table, and the highest
/// sequence the host has taken. Entries up to it are deleted; the number
/// stays so sequences never repeat.
pub struct Journal {
    inner: Arc<Inner>,
}

impl Journal {
    pub(crate) fn new(inner: Arc<Inner>) -> Self {
        Self { inner }
    }

    /// Adds an entry and returns its sequence once it's on disk.
    pub fn append(&self, kind: &str, payload: &[u8]) -> Result<u64, StoreError> {
        let doing = "add to the journal";
        let failed = self.inner.failed(doing);
        self.inner.write_with("journal.append", doing, |transaction| {
            let seq: i64 = transaction
                .query_row("SELECT next_seq FROM journal_state WHERE id = 1", [], |row| {
                    row.get(0)
                })
                .map_err(&failed)?;
            let placed = blob::place(&self.inner.blobs, blob::encode(blob::Type::JournalPayload, 1, payload)?)?;
            transaction
                .execute(
                    "INSERT INTO journal (seq, kind, written_unix_ms, payload, payload_file)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![seq, kind, unix_ms(), placed.inline(), placed.file()],
                )
                .and_then(|_| {
                    transaction.execute(
                        "UPDATE journal_state SET next_seq = ?1 WHERE id = 1",
                        [seq + 1],
                    )
                })
                .map_err(&failed)?;
            Ok(seq as u64)
        })
    }

    /// The entries the host hasn't acked, in the order they were written.
    pub fn entries(&self) -> Result<Vec<JournalEntry>, StoreError> {
        let rows = self.inner.read("read the journal", |connection| {
            connection
                .prepare(
                    "SELECT seq, kind, written_unix_ms, payload, payload_file FROM journal
                     WHERE seq > (SELECT acked FROM journal_state WHERE id = 1)
                     ORDER BY seq",
                )?
                .query_map([], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, Option<Vec<u8>>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()
        })?;
        rows.into_iter()
            .map(|(seq, kind, written_unix_ms, inline, file)| {
                let what = || format!("journal entry {seq}");
                let (_, payload) = blob::read(
                    &self.inner.blobs,
                    &self.inner.database,
                    inline,
                    file,
                    blob::Type::JournalPayload,
                    what,
                )?;
                Ok(JournalEntry {
                    seq: seq as u64,
                    kind,
                    written_unix_ms,
                    payload,
                })
            })
            .collect()
    }

    /// Records that the host has every entry through `seq`, and deletes
    /// them, in one commit.
    pub fn ack_through(&self, seq: u64) -> Result<(), StoreError> {
        let seq = crate::store::to_sql(seq);
        let had_files = self.inner.write("journal.ack", "record what the host has", |transaction| {
            transaction.execute(
                "UPDATE journal_state SET acked = max(acked, ?1),
                     next_seq = max(next_seq, max(acked, ?1) + 1)
                 WHERE id = 1",
                [seq],
            )?;
            let had_files: Option<i64> = transaction
                .query_row(
                    "SELECT 1 FROM journal WHERE seq <= ?1 AND payload_file IS NOT NULL LIMIT 1",
                    [seq],
                    |row| row.get(0),
                )
                .optional()?;
            transaction.execute("DELETE FROM journal WHERE seq <= ?1", [seq])?;
            Ok(had_files.is_some())
        })?;
        if had_files {
            self.inner.sweep()?;
        }
        Ok(())
    }

    /// The highest sequence the host has acked, 0 before any.
    pub fn acked(&self) -> Result<u64, StoreError> {
        self.inner
            .read("read what the host has", |connection| {
                connection.query_row("SELECT acked FROM journal_state WHERE id = 1", [], |row| {
                    row.get::<_, i64>(0)
                })
            })
            .map(crate::store::from_sql)
    }
}
