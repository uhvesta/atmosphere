// New file daemon/store/src/previous.rs (or paste into agents.rs), plus
// `mod previous;` in lib.rs and `pub use previous::PreviousConversation;`.
// Needs the `previous_conversations` table of the seam (schema migration).
// Also: `remove_session` should delete the rows for the session (add
// `DELETE FROM previous_conversations WHERE session_id = ?1` next to the others).

use std::path::{Path, PathBuf};

use rusqlite::params;

use crate::store::{bytes_path, check_id, path_bytes};
use crate::{Store, StoreError};

/// A conversation a session left behind when it started fresh (PER-15).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviousConversation {
    pub conversation_id: String,
    pub path: PathBuf,
    pub ended_unix_ms: i64,
}

impl Store {
    /// Records that `session` left `conversation` (kept at `path`) behind.
    /// Never deleted; recording it again changes nothing.
    pub fn push_previous_conversation(
        &self,
        session: &str,
        conversation: &str,
        path: &Path,
        ended_unix_ms: i64,
    ) -> Result<(), StoreError> {
        check_id(session)?;
        let inner = self.inner();
        inner.write_for_session(
            "previous_conversation.push",
            "save a session's previous conversation",
            Some(session),
            |transaction| {
                transaction
                    .execute(
                        "INSERT INTO previous_conversations
                             (session_id, conversation_id, path, ended_unix_ms)
                         VALUES (?1, ?2, ?3, ?4)
                         ON CONFLICT (session_id, conversation_id) DO NOTHING",
                        params![session, conversation, path_bytes(path), ended_unix_ms],
                    )
                    .map(|_| ())
                    .map_err(inner.failed("save a session's previous conversation"))
            },
        )
    }

    /// The conversations `session` left behind, oldest first.
    pub fn previous_conversations(
        &self,
        session: &str,
    ) -> Result<Vec<PreviousConversation>, StoreError> {
        check_id(session)?;
        let inner = self.inner();
        inner.read("read a session's previous conversations", |connection| {
            let mut statement = connection.prepare(
                "SELECT conversation_id, path, ended_unix_ms FROM previous_conversations
                 WHERE session_id = ?1 ORDER BY ended_unix_ms, rowid",
            )?;
            statement
                .query_map([session], |row| {
                    Ok(PreviousConversation {
                        conversation_id: row.get(0)?,
                        path: bytes_path(row.get(1)?),
                        ended_unix_ms: row.get(2)?,
                    })
                })?
                .collect()
        })
    }
}
// Schema note: `path` is declared TEXT in the seam but written as bytes
// (path_bytes), as agents.launch is; declare it BLOB, or write
// `path.to_string_lossy()` and read with PathBuf::from(String).
