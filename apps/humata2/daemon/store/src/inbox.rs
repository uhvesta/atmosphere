//! The Inbox's requests and the grants their answers made (D96), so a
//! restart keeps what an agent asked and what the user let it do.

use rusqlite::{Row, Transaction, params};

use crate::store::{Inner, check_id};
use crate::{InboxRecord, Store, StoreError, blob};

/// The blob version an entry is kept as.
const ENTRY_VERSION: u16 = 1;

const DOING: &str = "save an Inbox request";

/// Writes `request`'s row inside `transaction`, replacing one with its id.
/// The caller has checked the ids.
pub(crate) fn put_row(inner: &Inner, transaction: &Transaction<'_>, request: &InboxRecord) -> Result<(), StoreError> {
    let encoded = blob::encode(blob::Type::InboxEntry, ENTRY_VERSION, &request.entry)?;
    let placed = blob::place(&inner.blobs, encoded)?;
    transaction
        .execute(
            "INSERT INTO inbox_requests
                 (id, session, created_unix_ms, updated_unix_ms, entry, entry_file)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (id) DO UPDATE SET
                 session = excluded.session,
                 created_unix_ms = excluded.created_unix_ms,
                 updated_unix_ms = excluded.updated_unix_ms,
                 entry = excluded.entry, entry_file = excluded.entry_file",
            params![
                request.id,
                request.session,
                request.created_unix_ms,
                request.updated_unix_ms,
                placed.inline(),
                placed.file()
            ],
        )
        .map(drop)
        .map_err(inner.failed(DOING))
}

/// A request's row before its blob is read.
struct Stored {
    id: String,
    session: String,
    created_unix_ms: i64,
    updated_unix_ms: i64,
    entry: Option<Vec<u8>>,
    entry_file: Option<String>,
}

fn stored(row: &Row<'_>) -> rusqlite::Result<Stored> {
    Ok(Stored {
        id: row.get(0)?,
        session: row.get(1)?,
        created_unix_ms: row.get(2)?,
        updated_unix_ms: row.get(3)?,
        entry: row.get(4)?,
        entry_file: row.get(5)?,
    })
}

impl Store {
    /// Saves an Inbox request, replacing one with its id. Its session must
    /// be saved.
    pub fn put_inbox_request(&self, request: &InboxRecord) -> Result<(), StoreError> {
        check_id(&request.id)?;
        check_id(&request.session)?;
        let inner = self.inner();
        inner.write_for_session("inbox.put", DOING, Some(&request.session), |transaction| {
            put_row(inner, transaction, request)
        })
    }

    /// Every saved Inbox request, oldest first.
    pub fn inbox_requests(&self) -> Result<Vec<InboxRecord>, StoreError> {
        let rows = self.inner().read("list the Inbox requests", |connection| {
            let mut statement = connection.prepare(
                "SELECT id, session, created_unix_ms, updated_unix_ms, entry, entry_file
                 FROM inbox_requests ORDER BY created_unix_ms, id",
            )?;
            statement.query_map([], stored)?.collect::<rusqlite::Result<Vec<_>>>()
        })?;
        let inner = self.inner();
        rows.into_iter()
            .map(|row| {
                let what = || format!("Inbox request {}", row.id);
                let (_, entry) = blob::read(
                    &inner.blobs,
                    &inner.database,
                    row.entry,
                    row.entry_file,
                    blob::Type::InboxEntry,
                    what,
                )?;
                Ok(InboxRecord {
                    id: row.id,
                    session: row.session,
                    entry,
                    created_unix_ms: row.created_unix_ms,
                    updated_unix_ms: row.updated_unix_ms,
                })
            })
            .collect()
    }
}
