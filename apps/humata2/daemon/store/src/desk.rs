//! What only the Desk knows about a session whose channel is "desk": whether
//! its folder is a scratch one (and kept) or a picked one, the board and
//! channel toggles, when it was last used, closed or deleted, and the work
//! it still holds (DK-2, DK-4, DK-11, D182).

use rusqlite::{OptionalExtension, params};

use crate::store::check_id;
use crate::{Store, StoreError};

/// What only the Desk knows about a session whose channel is "desk" (DK-2, DK-4, DK-11, D182).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeskRow {
    pub session_id: String,
    pub kind: DeskFolderKind,
    pub boards: bool,
    pub channels_and_machines: bool,
    pub last_used_unix_ms: i64,
    pub closed_unix_ms: Option<i64>,
    pub deleted_unix_ms: Option<i64>,
    pub held_work: String,
}

/// Whether a Desk session's folder is one Humata made, or one the person picked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeskFolderKind {
    /// A folder Humata made; `kept` once the person has asked to keep it.
    Scratch { kept: bool },
    /// A folder the person picked.
    Picked,
}

impl Store {
    /// Saves `row` for its session, in place of any row it had before.
    pub fn put_desk(&self, row: &DeskRow) -> Result<(), StoreError> {
        check_id(&row.session_id)?;
        let (kind, kept) = kind_columns(row.kind);
        let inner = self.inner();
        inner.write_for_session(
            "desk.put",
            "save a Desk session's row",
            Some(&row.session_id),
            |transaction| {
                transaction
                    .execute(
                        "INSERT INTO desk_sessions (
                             session_id, kind, kept, boards, channels_and_machines,
                             last_used_unix_ms, closed_unix_ms, deleted_unix_ms, held_work)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                         ON CONFLICT (session_id) DO UPDATE SET
                             kind = excluded.kind,
                             kept = excluded.kept,
                             boards = excluded.boards,
                             channels_and_machines = excluded.channels_and_machines,
                             last_used_unix_ms = excluded.last_used_unix_ms,
                             closed_unix_ms = excluded.closed_unix_ms,
                             deleted_unix_ms = excluded.deleted_unix_ms,
                             held_work = excluded.held_work",
                        params![
                            row.session_id,
                            kind,
                            kept,
                            row.boards,
                            row.channels_and_machines,
                            row.last_used_unix_ms,
                            row.closed_unix_ms,
                            row.deleted_unix_ms,
                            row.held_work
                        ],
                    )
                    .map(drop)
                    .map_err(inner.failed("save a Desk session's row"))
            },
        )
    }

    /// The Desk row of `session`, if it has one.
    pub fn desk(&self, session: &str) -> Result<Option<DeskRow>, StoreError> {
        check_id(session)?;
        self.inner().read("read a Desk session's row", |connection| {
            connection
                .query_row(
                    "SELECT session_id, kind, kept, boards, channels_and_machines,
                            last_used_unix_ms, closed_unix_ms, deleted_unix_ms, held_work
                     FROM desk_sessions WHERE session_id = ?1",
                    [session],
                    row_desk,
                )
                .optional()
        })
    }

    /// Every Desk row, deleted ones included, ordered by session id.
    pub fn desks(&self) -> Result<Vec<DeskRow>, StoreError> {
        self.inner().read("read the Desk session rows", |connection| {
            let mut statement = connection.prepare(
                "SELECT session_id, kind, kept, boards, channels_and_machines,
                        last_used_unix_ms, closed_unix_ms, deleted_unix_ms, held_work
                 FROM desk_sessions ORDER BY session_id",
            )?;
            statement.query_map([], row_desk)?.collect()
        })
    }
}

/// The `kind` and `kept` columns for `kind`: 1 scratch, 2 picked, and
/// `kept` is only ever set for a scratch folder.
fn kind_columns(kind: DeskFolderKind) -> (i64, bool) {
    match kind {
        DeskFolderKind::Scratch { kept } => (1, kept),
        DeskFolderKind::Picked => (2, false),
    }
}

fn row_desk(row: &rusqlite::Row<'_>) -> rusqlite::Result<DeskRow> {
    let kind = match row.get::<_, i64>(1)? {
        1 => DeskFolderKind::Scratch { kept: row.get(2)? },
        2 => DeskFolderKind::Picked,
        other => return Err(rusqlite::Error::IntegralValueOutOfRange(1, other)),
    };
    Ok(DeskRow {
        session_id: row.get(0)?,
        kind,
        boards: row.get(3)?,
        channels_and_machines: row.get(4)?,
        last_used_unix_ms: row.get(5)?,
        closed_unix_ms: row.get(6)?,
        deleted_unix_ms: row.get(7)?,
        held_work: row.get(8)?,
    })
}
