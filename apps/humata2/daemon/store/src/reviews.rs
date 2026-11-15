//! Each session's review: the review chat and what its diff last showed,
//! so a restart keeps them (PR-26).

use prost::Message;
use rusqlite::{OptionalExtension, params};

use crate::proto::ReviewState as StateProto;
use crate::store::check_id;
use crate::{ReviewRecord, Store, StoreError, blob};

/// The blob version a review is kept as.
const STATE_VERSION: u16 = 1;

impl Store {
    /// Saves a session's review. Its session must be saved.
    pub fn put_review(&self, review: &ReviewRecord) -> Result<(), StoreError> {
        check_id(&review.session)?;
        let inner = self.inner();
        let doing = "save a session's review";
        let state = StateProto {
            chat: review.chat.clone(),
            shown: review.shown.clone(),
            last_seq: review.last_seq,
            last_note: review.last_note,
            view_reported: review.view_reported,
            history: review.history.clone(),
        };
        let encoded = blob::encode(blob::Type::ReviewState, STATE_VERSION, &state.encode_to_vec())?;
        inner.write_for_session("review.put", doing, Some(&review.session), |transaction| {
            let placed = blob::place(&inner.blobs, encoded)?;
            transaction
                .execute(
                    "INSERT INTO reviews (session, updated_unix_ms, state, state_file)
                     VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT (session) DO UPDATE SET
                         updated_unix_ms = excluded.updated_unix_ms,
                         state = excluded.state, state_file = excluded.state_file",
                    params![review.session, review.updated_unix_ms, placed.inline(), placed.file()],
                )
                .map(drop)
                .map_err(inner.failed(doing))
        })
    }

    /// The review of `session`, if one was saved.
    pub fn review(&self, session: &str) -> Result<Option<ReviewRecord>, StoreError> {
        check_id(session)?;
        let inner = self.inner();
        inner.read_with(|connection| {
            let row = connection
                .query_row(
                    "SELECT updated_unix_ms, state, state_file FROM reviews WHERE session = ?1",
                    [session],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, Option<Vec<u8>>>(1)?,
                            row.get::<_, Option<String>>(2)?,
                        ))
                    },
                )
                .optional()
                .map_err(inner.failed("read a session's review"))?;
            let Some((updated_unix_ms, state, state_file)) = row else {
                return Ok(None);
            };
            let what = || format!("the review of session {session}");
            let (_, body) = blob::read(
                &inner.blobs,
                &inner.database,
                state,
                state_file,
                blob::Type::ReviewState,
                what,
            )?;
            let state = StateProto::decode(body.as_slice()).map_err(|_| StoreError::Damaged {
                path: inner.database.clone(),
                what: what(),
                reason: "its review is unreadable",
            })?;
            Ok(Some(ReviewRecord {
                session: session.to_owned(),
                chat: state.chat,
                shown: state.shown,
                last_seq: state.last_seq,
                last_note: state.last_note,
                view_reported: state.view_reported,
                history: state.history,
                updated_unix_ms,
            }))
        })
    }
}
