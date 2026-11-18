//! The comments a Desk session's file had when a version was made (DV-12):
//! one row per session, commit and file, kept with the session (PER-25)
//! and never changed once written. The store doesn't read the comments:
//! they are the bytes the daemon encodes, kept as given.

use rusqlite::{OptionalExtension, params};

use crate::store::check_id;
use crate::{Store, StoreError};

impl Store {
    /// Records `comments` as the comments of `path` at `commit`, unless the
    /// session already has a record for them: the first one stays. Returns
    /// whether this call wrote it. The session must be saved.
    pub fn put_doc_comments(
        &self,
        session: &str,
        commit: &str,
        path: &str,
        comments: &[u8],
        recorded_unix_ms: i64,
    ) -> Result<bool, StoreError> {
        check_id(session)?;
        check_id(commit)?;
        let inner = self.inner();
        let doing = "record the comments a document version had";
        inner.write_for_session("doc_comments.put", doing, Some(session), |transaction| {
            transaction
                .execute(
                    "INSERT OR IGNORE INTO doc_comment_records
                         (session, commit_id, path, recorded_unix_ms, comments)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![session, commit, path, recorded_unix_ms, comments],
                )
                .map(|changed| changed > 0)
                .map_err(inner.failed(doing))
        })
    }

    /// The comments recorded for `path` at `commit`, or `None` when no
    /// record was made (an empty record is `Some` of nothing).
    pub fn doc_comments(&self, session: &str, commit: &str, path: &str) -> Result<Option<Vec<u8>>, StoreError> {
        check_id(session)?;
        check_id(commit)?;
        self.inner().read("read the comments a document version had", |connection| {
            connection
                .query_row(
                    "SELECT comments FROM doc_comment_records
                     WHERE session = ?1 AND commit_id = ?2 AND path = ?3",
                    params![session, commit, path],
                    |row| row.get(0),
                )
                .optional()
        })
    }

    /// The commits that have a record for any file, in no particular order.
    pub fn recorded_doc_commits(&self, session: &str) -> Result<Vec<String>, StoreError> {
        check_id(session)?;
        self.inner().read("list the document versions that have recorded comments", |connection| {
            connection
                .prepare("SELECT DISTINCT commit_id FROM doc_comment_records WHERE session = ?1")?
                .query_map(params![session], |row| row.get(0))?
                .collect()
        })
    }

    /// The commits that have a record for `path`, in no particular order.
    pub fn doc_comment_commits(&self, session: &str, path: &str) -> Result<Vec<String>, StoreError> {
        check_id(session)?;
        self.inner().read("list the document versions that have recorded comments", |connection| {
            connection
                .prepare("SELECT commit_id FROM doc_comment_records WHERE session = ?1 AND path = ?2")?
                .query_map(params![session, path], |row| row.get(0))?
                .collect()
        })
    }
}
