//! The views sessions' agents prepare (VW-1, VW-14), so a restart keeps
//! drafts and offers alike.

use prost::Message;
use rusqlite::{Row, params};

use crate::proto::View as ViewProto;
use crate::store::check_id;
use crate::{Store, StoreError, ViewRecord, blob};

/// The blob version a view is kept as.
const VIEW_VERSION: u16 = 1;

/// A view's row before its blob is read.
struct Stored {
    id: String,
    session: String,
    prepared_unix_ms: i64,
    updated_unix_ms: i64,
    view: Option<Vec<u8>>,
    view_file: Option<String>,
}

fn stored(row: &Row<'_>) -> rusqlite::Result<Stored> {
    Ok(Stored {
        id: row.get(0)?,
        session: row.get(1)?,
        prepared_unix_ms: row.get(2)?,
        updated_unix_ms: row.get(3)?,
        view: row.get(4)?,
        view_file: row.get(5)?,
    })
}

const COLUMNS: &str = "id, session, prepared_unix_ms, updated_unix_ms, view, view_file";

impl Store {
    /// Saves a view, replacing one with its id. Its session must be saved.
    pub fn put_view(&self, view: &ViewRecord) -> Result<(), StoreError> {
        check_id(&view.id)?;
        check_id(&view.session)?;
        let inner = self.inner();
        let doing = "save a prepared view";
        let body = ViewProto {
            view: view.view.clone(),
            keys: view.keys.clone(),
        };
        let encoded = blob::encode(blob::Type::View, VIEW_VERSION, &body.encode_to_vec())?;
        inner.write_for_session("view.put", doing, Some(&view.session), |transaction| {
            let placed = blob::place(&inner.blobs, encoded)?;
            transaction
                .execute(
                    "INSERT INTO views (id, session, prepared_unix_ms, updated_unix_ms, view, view_file)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT (id) DO UPDATE SET
                         session = excluded.session,
                         prepared_unix_ms = excluded.prepared_unix_ms,
                         updated_unix_ms = excluded.updated_unix_ms,
                         view = excluded.view, view_file = excluded.view_file",
                    params![
                        view.id,
                        view.session,
                        view.prepared_unix_ms,
                        view.updated_unix_ms,
                        placed.inline(),
                        placed.file()
                    ],
                )
                .map(drop)
                .map_err(inner.failed(doing))
        })
    }

    /// The view `id`, if one was saved.
    pub fn view(&self, id: &str) -> Result<Option<ViewRecord>, StoreError> {
        check_id(id)?;
        let row = self.inner().read("read a prepared view", |connection| {
            let mut statement = connection.prepare(&format!("SELECT {COLUMNS} FROM views WHERE id = ?1"))?;
            let mut rows = statement.query_map([id], stored)?;
            rows.next().transpose()
        })?;
        row.map(|row| self.read_view(row)).transpose()
    }

    /// The views of `session`, newest first.
    pub fn views(&self, session: &str) -> Result<Vec<ViewRecord>, StoreError> {
        check_id(session)?;
        let rows = self.inner().read("list a session's views", |connection| {
            let mut statement = connection.prepare(&format!(
                "SELECT {COLUMNS} FROM views WHERE session = ?1
                 ORDER BY prepared_unix_ms DESC, id DESC"
            ))?;
            statement.query_map([session], stored)?.collect::<rusqlite::Result<Vec<_>>>()
        })?;
        rows.into_iter().map(|row| self.read_view(row)).collect()
    }

    fn read_view(&self, row: Stored) -> Result<ViewRecord, StoreError> {
        let inner = self.inner();
        let what = || format!("view {}", row.id);
        let (_, body) = blob::read(&inner.blobs, &inner.database, row.view, row.view_file, blob::Type::View, what)?;
        let body = ViewProto::decode(body.as_slice()).map_err(|_| StoreError::Damaged {
            path: inner.database.clone(),
            what: what(),
            reason: "its view is unreadable",
        })?;
        Ok(ViewRecord {
            id: row.id,
            session: row.session,
            view: body.view,
            keys: body.keys,
            prepared_unix_ms: row.prepared_unix_ms,
            updated_unix_ms: row.updated_unix_ms,
        })
    }
}
