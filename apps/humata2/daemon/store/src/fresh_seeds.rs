//! The first message a start fresh types into its new conversation, kept
//! until it is typed (PER-15, D153), so a daemon stopped in between types
//! it after the restart, and a second start fresh reuses it rather than
//! summarizing a conversation that never began.

use rusqlite::{OptionalExtension, params};

use crate::store::check_id;
use crate::{Store, StoreError};

/// A first message not yet typed into `conversation_id`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FreshSeed {
    pub session_id: String,
    pub conversation_id: String,
    pub seed: String,
    pub created_unix_ms: i64,
}

impl Store {
    /// Keeps `seed` as the session's untyped first message, in place of any
    /// earlier one.
    pub fn put_fresh_seed(&self, seed: &FreshSeed) -> Result<(), StoreError> {
        check_id(&seed.session_id)?;
        let inner = self.inner();
        inner.write_for_session(
            "fresh_seed.put",
            "save a start fresh's first message",
            Some(&seed.session_id),
            |transaction| {
                transaction
                    .execute(
                        "INSERT INTO fresh_seeds (session_id, conversation_id, seed, created_unix_ms)
                         VALUES (?1, ?2, ?3, ?4)
                         ON CONFLICT (session_id) DO UPDATE SET
                             conversation_id = excluded.conversation_id,
                             seed = excluded.seed,
                             created_unix_ms = excluded.created_unix_ms",
                        params![
                            seed.session_id,
                            seed.conversation_id,
                            seed.seed,
                            seed.created_unix_ms
                        ],
                    )
                    .map(drop)
                    .map_err(inner.failed("save a start fresh's first message"))
            },
        )
    }

    /// The session's untyped first message, if it has one.
    pub fn fresh_seed(&self, session: &str) -> Result<Option<FreshSeed>, StoreError> {
        check_id(session)?;
        self.inner().read("read a start fresh's first message", |connection| {
            connection
                .query_row(
                    "SELECT session_id, conversation_id, seed, created_unix_ms
                     FROM fresh_seeds WHERE session_id = ?1",
                    [session],
                    row_seed,
                )
                .optional()
        })
    }

    /// Every untyped first message, oldest first.
    pub fn fresh_seeds(&self) -> Result<Vec<FreshSeed>, StoreError> {
        self.inner().read("read the start fresh first messages", |connection| {
            let mut statement = connection.prepare(
                "SELECT session_id, conversation_id, seed, created_unix_ms
                 FROM fresh_seeds ORDER BY created_unix_ms, session_id",
            )?;
            statement.query_map([], row_seed)?.collect()
        })
    }

    /// Forgets the session's first message if it is still the one for
    /// `conversation`: a newer start fresh's message stays.
    pub fn typed_fresh_seed(&self, session: &str, conversation: &str) -> Result<(), StoreError> {
        check_id(session)?;
        self.inner().write("fresh_seed.typed", "forget a typed first message", |transaction| {
            transaction
                .execute(
                    "DELETE FROM fresh_seeds WHERE session_id = ?1 AND conversation_id = ?2",
                    params![session, conversation],
                )
                .map(drop)
        })
    }
}

fn row_seed(row: &rusqlite::Row<'_>) -> rusqlite::Result<FreshSeed> {
    Ok(FreshSeed {
        session_id: row.get(0)?,
        conversation_id: row.get(1)?,
        seed: row.get(2)?,
        created_unix_ms: row.get(3)?,
    })
}
