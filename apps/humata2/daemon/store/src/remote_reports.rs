//! The last status and pointers each machine reported for each session that lives on
//! it (SL-3, PER-27, D150), so the host's restart still shows them.

use rusqlite::params;

use crate::{Store, StoreError};

/// What one machine last reported about one of its sessions. Plain columns: the two
/// messages are the bytes the daemon encodes, kept as given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteReportRow {
    pub machine: String,
    pub session: String,
    /// The API's `SessionStatus`.
    pub reported: i32,
    pub exit_code: i32,
    pub problem: Option<Vec<u8>>,
    pub pointers: Vec<u8>,
    pub pointers_at_ms: Option<i64>,
    pub reported_unix_ms: i64,
    pub last_seen_unix_ms: Option<i64>,
    /// The machine's read mark for the session as last reported (CH-16, D396).
    pub unread: bool,
}

impl Store {
    /// Keeps `row` in place of any earlier report of the same session.
    pub fn put_remote_report(&self, row: &RemoteReportRow) -> Result<(), StoreError> {
        let inner = self.inner();
        inner.write(
            "remote_report.put",
            "save a remote session's last report",
            |transaction| {
                transaction
                    .execute(
                        "INSERT INTO remote_session_reports
                             (machine, session, reported, exit_code, problem, pointers,
                              pointers_at_ms, reported_unix_ms, last_seen_unix_ms, unread)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                         ON CONFLICT (machine, session) DO UPDATE SET
                             reported = excluded.reported,
                             exit_code = excluded.exit_code,
                             problem = excluded.problem,
                             pointers = excluded.pointers,
                             pointers_at_ms = excluded.pointers_at_ms,
                             reported_unix_ms = excluded.reported_unix_ms,
                             last_seen_unix_ms = excluded.last_seen_unix_ms,
                             unread = excluded.unread",
                        params![
                            row.machine,
                            row.session,
                            row.reported,
                            row.exit_code,
                            row.problem,
                            row.pointers,
                            row.pointers_at_ms,
                            row.reported_unix_ms,
                            row.last_seen_unix_ms,
                            row.unread
                        ],
                    )
                    .map(drop)
            },
        )
    }

    /// Every report kept, by machine then session.
    pub fn remote_reports(&self) -> Result<Vec<RemoteReportRow>, StoreError> {
        self.inner()
            .read("read the remote sessions' last reports", |connection| {
                let mut statement = connection.prepare(
                    "SELECT machine, session, reported, exit_code, problem, pointers,
                            pointers_at_ms, reported_unix_ms, last_seen_unix_ms, unread
                     FROM remote_session_reports ORDER BY machine, session",
                )?;
                statement
                    .query_map([], |row| {
                        Ok(RemoteReportRow {
                            machine: row.get(0)?,
                            session: row.get(1)?,
                            reported: row.get(2)?,
                            exit_code: row.get(3)?,
                            problem: row.get(4)?,
                            pointers: row.get(5)?,
                            pointers_at_ms: row.get(6)?,
                            reported_unix_ms: row.get(7)?,
                            last_seen_unix_ms: row.get(8)?,
                            unread: row.get(9)?,
                        })
                    })?
                    .collect()
            })
    }

    /// Forgets the report of one session; one that isn't kept is left alone.
    pub fn delete_remote_report(&self, machine: &str, session: &str) -> Result<(), StoreError> {
        self.inner().write(
            "remote_report.delete",
            "forget a remote session's last report",
            |transaction| {
                transaction
                    .execute(
                        "DELETE FROM remote_session_reports WHERE machine = ?1 AND session = ?2",
                        params![machine, session],
                    )
                    .map(drop)
            },
        )
    }
}
