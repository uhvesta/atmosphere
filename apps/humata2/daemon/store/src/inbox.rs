//! The Inbox's requests and the grants their answers made (D96), so a
//! restart keeps what an agent asked and what the user let it do. A Migrate
//! item (UP-1, D281) belongs to no session, so its row has none, and only
//! one pending item may name each thing and machine.

use rusqlite::{Row, Transaction, params};

use crate::store::{Inner, check_id};
use crate::{InboxRecord, MigrateRecord, Store, StoreError, blob};

/// The blob version an entry is kept as.
const ENTRY_VERSION: u16 = 1;

const DOING: &str = "save an Inbox request";
const DOING_MIGRATE: &str = "save a Migrate item";

/// Writes `request`'s row inside `transaction`, replacing one with its id.
/// The caller has checked the ids. A session's request holds no Migrate
/// columns, so it replaces a Migrate item's row with its own.
pub(crate) fn put_row(
    inner: &Inner,
    transaction: &Transaction<'_>,
    request: &InboxRecord,
) -> Result<(), StoreError> {
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
                 entry = excluded.entry, entry_file = excluded.entry_file,
                 migrate_thing = NULL, migrate_machine = NULL, migrate_pending = 0",
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

/// Writes Migrate item `request`'s row inside `transaction`, replacing one
/// with its id. Its session is NULL. The caller has checked the ids.
fn put_migrate_row(
    inner: &Inner,
    transaction: &Transaction<'_>,
    request: &MigrateRecord,
) -> Result<(), StoreError> {
    let encoded = blob::encode(blob::Type::InboxEntry, ENTRY_VERSION, &request.entry)?;
    let placed = blob::place(&inner.blobs, encoded)?;
    transaction
        .execute(
            "INSERT INTO inbox_requests
                 (id, session, created_unix_ms, updated_unix_ms, entry, entry_file,
                  migrate_thing, migrate_machine, migrate_pending)
             VALUES (?1, NULL, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT (id) DO UPDATE SET
                 session = NULL,
                 created_unix_ms = excluded.created_unix_ms,
                 updated_unix_ms = excluded.updated_unix_ms,
                 entry = excluded.entry, entry_file = excluded.entry_file,
                 migrate_thing = excluded.migrate_thing,
                 migrate_machine = excluded.migrate_machine,
                 migrate_pending = excluded.migrate_pending",
            params![
                request.id,
                request.created_unix_ms,
                request.updated_unix_ms,
                placed.inline(),
                placed.file(),
                request.thing,
                request.machine,
                request.pending
            ],
        )
        .map(drop)
        .map_err(|source| {
            if names_pending_taken(&source) {
                StoreError::MigrateTaken {
                    thing: request.thing,
                    machine: request.machine.clone(),
                }
            } else {
                inner.failed(DOING_MIGRATE)(source)
            }
        })
}

/// Whether `error` is the partial index refusing a second pending Migrate
/// item for a thing and machine. The id's own key is upserted, so no other
/// unique constraint is hit here.
fn names_pending_taken(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(failure, _)
            if failure.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
    )
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

/// A pending Migrate item's row before its blob is read.
struct MigrateStored {
    id: String,
    thing: i32,
    machine: String,
    created_unix_ms: i64,
    updated_unix_ms: i64,
    entry: Option<Vec<u8>>,
    entry_file: Option<String>,
}

fn migrate_stored(row: &Row<'_>) -> rusqlite::Result<MigrateStored> {
    Ok(MigrateStored {
        id: row.get(0)?,
        thing: row.get(1)?,
        machine: row.get(2)?,
        created_unix_ms: row.get(3)?,
        updated_unix_ms: row.get(4)?,
        entry: row.get(5)?,
        entry_file: row.get(6)?,
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

    /// Every saved Inbox request of a session, oldest first. Migrate items
    /// belong to no session: [`Store::pending_migrate_requests`] lists them.
    pub fn inbox_requests(&self) -> Result<Vec<InboxRecord>, StoreError> {
        let rows = self.inner().read("list the Inbox requests", |connection| {
            let mut statement = connection.prepare(
                "SELECT id, session, created_unix_ms, updated_unix_ms, entry, entry_file
                 FROM inbox_requests WHERE session IS NOT NULL ORDER BY created_unix_ms, id",
            )?;
            statement
                .query_map([], stored)?
                .collect::<rusqlite::Result<Vec<_>>>()
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

    /// Saves a Migrate item, replacing one with its id: a new item, a change
    /// to it, or its withdrawal, which is the same item with `pending` false.
    /// Refused as [`StoreError::MigrateTaken`] when `request` is pending and
    /// another pending item names its thing and machine.
    pub fn put_migrate_request(&self, request: &MigrateRecord) -> Result<(), StoreError> {
        check_id(&request.id)?;
        check_id(&request.machine)?;
        let inner = self.inner();
        inner.write_with("inbox.put", DOING_MIGRATE, |transaction| {
            put_migrate_row(inner, transaction, request)
        })
    }

    /// The pending Migrate items, oldest first.
    pub fn pending_migrate_requests(&self) -> Result<Vec<MigrateRecord>, StoreError> {
        let rows = self
            .inner()
            .read("list the pending Migrate items", |connection| {
                let mut statement = connection.prepare(
                    "SELECT id, migrate_thing, migrate_machine, created_unix_ms, updated_unix_ms,
                        entry, entry_file
                 FROM inbox_requests WHERE migrate_pending = 1 ORDER BY created_unix_ms, id",
                )?;
                statement
                    .query_map([], migrate_stored)?
                    .collect::<rusqlite::Result<Vec<_>>>()
            })?;
        let inner = self.inner();
        rows.into_iter()
            .map(|row| {
                let what = || format!("Migrate item {}", row.id);
                let (_, entry) = blob::read(
                    &inner.blobs,
                    &inner.database,
                    row.entry,
                    row.entry_file,
                    blob::Type::InboxEntry,
                    what,
                )?;
                Ok(MigrateRecord {
                    id: row.id,
                    thing: row.thing,
                    machine: row.machine,
                    pending: true,
                    entry,
                    created_unix_ms: row.created_unix_ms,
                    updated_unix_ms: row.updated_unix_ms,
                })
            })
            .collect()
    }
}
