//! What became of each keyed Type call (D337). A key is noted pending
//! before its text goes to the shell and typed once all of it is there, so
//! a retry, even one after a restart, neither types the text twice nor
//! says it was typed when it may not have been. Keys are kept for the
//! idempotency window and dropped after it when the daemon starts.

use rusqlite::{OptionalExtension, params};

use crate::{Store, StoreError};

/// Where a keyed Type call stands when a call with its key comes in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypedKey {
    /// No call had the key. It is pending now, and the caller types.
    Claimed,
    /// A call with the key started typing and never finished, so some or
    /// all of its text may have reached the shell. `holder` is the holder
    /// process it typed through, which can say (D426), or 0 when none can.
    Pending { holder: u64 },
    /// A call with the key typed all of its text.
    Typed,
}

const PENDING: &str = "pending";
const TYPED: &str = "typed";

impl Store {
    /// Notes `key` pending at `now_unix_ms` unless a call already had it,
    /// and says where that call stands.
    pub fn claim_typed_key(&self, key: &str, now_unix_ms: i64) -> Result<TypedKey, StoreError> {
        self.inner()
            .write("typed_keys.claim", "note a Type call's key", |transaction| {
                let state: Option<(String, i64)> = transaction
                    .query_row(
                        "SELECT state, holder FROM typed_keys WHERE key = ?1",
                        [key],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()?;
                let holder = state.as_ref().map_or(0, |(_, holder)| holder.cast_unsigned());
                Ok(match state.as_ref().map(|(state, _)| state.as_str()) {
                    None => {
                        transaction.execute(
                            "INSERT INTO typed_keys (key, state, at_unix_ms) VALUES (?1, ?2, ?3)",
                            params![key, PENDING, now_unix_ms],
                        )?;
                        TypedKey::Claimed
                    }
                    Some(TYPED) => TypedKey::Typed,
                    Some(_) => TypedKey::Pending { holder },
                })
            })
    }

    /// Notes the holder process the call holding `key` types through, so a
    /// retry can ask it whether the text arrived (D426).
    pub fn note_typed_key_holder(&self, key: &str, holder: u64) -> Result<(), StoreError> {
        self.inner().write(
            "typed_keys.holder",
            "note which holder a Type call types through",
            |transaction| {
                transaction
                    .execute(
                        "UPDATE typed_keys SET holder = ?2 WHERE key = ?1",
                        params![key, holder.cast_signed()],
                    )
                    .map(drop)
            },
        )
    }

    /// Notes that the call holding `key` typed all of its text.
    pub fn finish_typed_key(&self, key: &str) -> Result<(), StoreError> {
        self.inner()
            .write("typed_keys.finish", "note a Type call typed its text", |transaction| {
                transaction
                    .execute("UPDATE typed_keys SET state = ?2 WHERE key = ?1", params![key, TYPED])
                    .map(drop)
            })
    }

    /// Forgets `key`, for a call that typed none of its text, so a retry
    /// types it.
    pub fn release_typed_key(&self, key: &str) -> Result<(), StoreError> {
        self.inner()
            .write("typed_keys.release", "forget a Type call's key", |transaction| {
                transaction
                    .execute("DELETE FROM typed_keys WHERE key = ?1", [key])
                    .map(drop)
            })
    }

    /// Drops the keys claimed before `before_unix_ms`, and says how many.
    pub fn prune_typed_keys(&self, before_unix_ms: i64) -> Result<usize, StoreError> {
        self.inner()
            .write("typed_keys.prune", "drop old Type call keys", |transaction| {
                transaction.execute("DELETE FROM typed_keys WHERE at_unix_ms < ?1", [before_unix_ms])
            })
    }
}
