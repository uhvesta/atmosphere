//! Where a fork came from, and what archiving a session left (FK-1..FK-3,
//! FK-13, MV-7, LD-2, MV-15, D110, D170).
//!
//! A fork's origin is never rewritten: the session it shows under is its
//! nearest ancestor that isn't archived, so restoring a parent puts the tree
//! back. An archive row is written before the agent stops (the intent), so a
//! restart finishes an archive cut short and frees the clones still waiting.

use rusqlite::{OptionalExtension, params};

use crate::store::check_id;
use crate::{Store, StoreError};

/// The session a fork came from and how it was made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForkOrigin {
    pub session_id: String,
    pub parent: String,
    /// The API's ForkKind.
    pub kind: i32,
    /// The parent's turn the fork was made before; 0 for "from now".
    pub before_turn: u32,
    pub created_unix_ms: i64,
}

/// How far an archive has got.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArchiveState {
    /// Recorded; the agent may still be running and the summary unwritten.
    Archiving,
    /// Stopped and summarized; the clones wait for the undo window to close.
    Archived,
    /// The clones and folder are gone; only Restore from what was pushed
    /// brings the session back (LD-4).
    Freed,
}

/// An archived session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArchiveRow {
    pub session_id: String,
    pub archived_unix_ms: i64,
    pub state: ArchiveState,
    /// The API's ArchiveSummary as the daemon encodes it; empty until the
    /// summary is written.
    pub summary: Vec<u8>,
    /// When the clones may be deleted: the end of the undo window (MV-15).
    pub free_after_unix_ms: i64,
}

impl Store {
    /// Records where `origin.session_id` was forked from.
    pub fn put_fork_origin(&self, origin: &ForkOrigin) -> Result<(), StoreError> {
        check_id(&origin.session_id)?;
        check_id(&origin.parent)?;
        self.inner().write("fork_origin.put", "save where a fork came from", |transaction| {
            transaction
                .execute(
                    "INSERT INTO fork_origins (session_id, parent, kind, before_turn, created_unix_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT (session_id) DO UPDATE SET
                         parent = excluded.parent,
                         kind = excluded.kind,
                         before_turn = excluded.before_turn,
                         created_unix_ms = excluded.created_unix_ms",
                    params![
                        origin.session_id,
                        origin.parent,
                        origin.kind,
                        origin.before_turn,
                        origin.created_unix_ms
                    ],
                )
                .map(drop)
        })
    }

    /// Where the session was forked from, if it is a fork.
    pub fn fork_origin(&self, session: &str) -> Result<Option<ForkOrigin>, StoreError> {
        check_id(session)?;
        self.inner().read("read where a fork came from", |connection| {
            connection
                .query_row(
                    "SELECT session_id, parent, kind, before_turn, created_unix_ms
                     FROM fork_origins WHERE session_id = ?1",
                    [session],
                    row_origin,
                )
                .optional()
        })
    }

    /// Every fork's origin, oldest first.
    pub fn fork_origins(&self) -> Result<Vec<ForkOrigin>, StoreError> {
        self.inner().read("read where forks came from", |connection| {
            let mut statement = connection.prepare(
                "SELECT session_id, parent, kind, before_turn, created_unix_ms
                 FROM fork_origins ORDER BY created_unix_ms, session_id",
            )?;
            statement.query_map([], row_origin)?.collect()
        })
    }

    /// Saves the session's archive row, in place of any earlier one.
    pub fn put_archive(&self, row: &ArchiveRow) -> Result<(), StoreError> {
        check_id(&row.session_id)?;
        self.inner().write("archive.put", "save that a session is archived", |transaction| {
            transaction
                .execute(
                    "INSERT INTO archives (session_id, archived_unix_ms, state, summary, free_after_unix_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT (session_id) DO UPDATE SET
                         archived_unix_ms = excluded.archived_unix_ms,
                         state = excluded.state,
                         summary = excluded.summary,
                         free_after_unix_ms = excluded.free_after_unix_ms",
                    params![
                        row.session_id,
                        row.archived_unix_ms,
                        state_code(row.state),
                        row.summary,
                        row.free_after_unix_ms
                    ],
                )
                .map(drop)
        })
    }

    /// The session's archive row, if it is archived.
    pub fn archive(&self, session: &str) -> Result<Option<ArchiveRow>, StoreError> {
        check_id(session)?;
        self.inner().read("read a session's archive", |connection| {
            connection
                .query_row(
                    "SELECT session_id, archived_unix_ms, state, summary, free_after_unix_ms
                     FROM archives WHERE session_id = ?1",
                    [session],
                    row_archive,
                )
                .optional()
        })
    }

    /// Every archive row, oldest first.
    pub fn archives(&self) -> Result<Vec<ArchiveRow>, StoreError> {
        self.inner().read("read the archived sessions", |connection| {
            let mut statement = connection.prepare(
                "SELECT session_id, archived_unix_ms, state, summary, free_after_unix_ms
                 FROM archives ORDER BY archived_unix_ms, session_id",
            )?;
            statement.query_map([], row_archive)?.collect()
        })
    }

    /// Forgets the session's archive row: it was restored.
    pub fn remove_archive(&self, session: &str) -> Result<(), StoreError> {
        check_id(session)?;
        self.inner().write("archive.remove", "save that a session was restored", |transaction| {
            transaction
                .execute("DELETE FROM archives WHERE session_id = ?1", [session])
                .map(drop)
        })
    }
}

fn state_code(state: ArchiveState) -> i64 {
    match state {
        ArchiveState::Archiving => 0,
        ArchiveState::Archived => 1,
        ArchiveState::Freed => 2,
    }
}

fn row_origin(row: &rusqlite::Row<'_>) -> rusqlite::Result<ForkOrigin> {
    Ok(ForkOrigin {
        session_id: row.get(0)?,
        parent: row.get(1)?,
        kind: row.get(2)?,
        before_turn: row.get(3)?,
        created_unix_ms: row.get(4)?,
    })
}

fn row_archive(row: &rusqlite::Row<'_>) -> rusqlite::Result<ArchiveRow> {
    let code: i64 = row.get(2)?;
    let state = match code {
        0 => ArchiveState::Archiving,
        1 => ArchiveState::Archived,
        2 => ArchiveState::Freed,
        other => {
            return Err(rusqlite::Error::IntegralValueOutOfRange(2, other));
        }
    };
    Ok(ArchiveRow {
        session_id: row.get(0)?,
        archived_unix_ms: row.get(1)?,
        state,
        summary: row.get(3)?,
        free_after_unix_ms: row.get(4)?,
    })
}
