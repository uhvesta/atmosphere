//! The host's last answers to the reads a machine's agents made (AWAY T10d,
//! D227). A machine whose host is linked sends an agent's board reads up and
//! keeps each answer here, by session, method and the request's bytes. While
//! the host is away the same read is answered from it, marked with the time
//! the host answered. A session keeps its newest answers only, none of them
//! large, and they go with the session.

use rusqlite::{OptionalExtension, params};
use sha2::{Digest, Sha256};

use crate::store::check_id;
use crate::{Store, StoreError};

/// The answers a session keeps; past it the oldest go.
const ROWS_PER_SESSION: i64 = 64;

/// An answer longer than this many bytes isn't kept.
const LARGEST_ANSWER: usize = 256 * 1024;

/// What the host answered a read with, and when.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LastCopy {
    pub response: Vec<u8>,
    /// When the host answered, in milliseconds since the epoch.
    pub answered_unix_ms: i64,
}

fn hash(request: &[u8]) -> Vec<u8> {
    Sha256::digest(request).to_vec()
}

impl Store {
    /// Keeps `response`, the host's answer at `answered_unix_ms` to
    /// `session`'s call of `method` with these `request` bytes, in place of
    /// an earlier answer to the same call. Returns false, keeping nothing,
    /// when the answer is too large to keep. The session's oldest answers go
    /// when it holds more than it may.
    pub fn put_last_copy(
        &self,
        session: &str,
        method: &str,
        request: &[u8],
        response: &[u8],
        answered_unix_ms: i64,
    ) -> Result<bool, StoreError> {
        check_id(session)?;
        if response.len() > LARGEST_ANSWER {
            return Ok(false);
        }
        let inner = self.inner();
        let doing = "keep the host's answer to a read";
        inner.write_for_session("last_copy.put", doing, Some(session), |transaction| {
            transaction
                .execute(
                    "INSERT INTO last_copies (session, method, request_hash, response, answered_unix_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT (session, method, request_hash) DO UPDATE SET
                         response = excluded.response,
                         answered_unix_ms = excluded.answered_unix_ms",
                    params![session, method, hash(request), response, answered_unix_ms],
                )
                .and_then(|_| {
                    transaction.execute(
                        "DELETE FROM last_copies WHERE session = ?1 AND rowid NOT IN (
                             SELECT rowid FROM last_copies WHERE session = ?1
                             ORDER BY answered_unix_ms DESC, rowid DESC LIMIT ?2)",
                        params![session, ROWS_PER_SESSION],
                    )
                })
                .map_err(inner.failed(doing))?;
            Ok(true)
        })
    }

    /// The host's last answer to `session`'s call of `method` with these
    /// `request` bytes, if one is kept.
    pub fn last_copy(
        &self,
        session: &str,
        method: &str,
        request: &[u8],
    ) -> Result<Option<LastCopy>, StoreError> {
        check_id(session)?;
        self.inner()
            .read("read the host's last answer to a read", |connection| {
                connection
                    .query_row(
                        "SELECT response, answered_unix_ms FROM last_copies
                     WHERE session = ?1 AND method = ?2 AND request_hash = ?3",
                        params![session, method, hash(request)],
                        |row| {
                            Ok(LastCopy {
                                response: row.get(0)?,
                                answered_unix_ms: row.get(1)?,
                            })
                        },
                    )
                    .optional()
            })
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::SessionRecord;

    fn store_with(sessions: &[&str]) -> (tempfile::TempDir, Store) {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path()).unwrap();
        for id in sessions {
            store
                .put_session(&SessionRecord {
                    id: (*id).to_owned(),
                    channel: "ch_1".to_owned(),
                    machine: String::new(),
                    title: "t".to_owned(),
                    agent: "claude".to_owned(),
                    folder: PathBuf::from("/work"),
                    created_unix_ms: 1,
                })
                .unwrap();
        }
        (directory, store)
    }

    const METHOD: &str = "/humata.v1.BoardService/ListItems";

    #[test]
    fn an_answer_is_read_back_only_for_the_same_session_method_and_request() {
        let (_directory, store) = store_with(&["se_a", "se_b"]);
        assert!(
            store
                .put_last_copy("se_a", METHOD, b"ask", b"answer", 5)
                .unwrap()
        );
        let found = store.last_copy("se_a", METHOD, b"ask").unwrap();
        assert_eq!(
            found,
            Some(LastCopy {
                response: b"answer".to_vec(),
                answered_unix_ms: 5
            })
        );
        assert_eq!(store.last_copy("se_a", METHOD, b"other ask").unwrap(), None);
        assert_eq!(
            store
                .last_copy("se_a", "/humata.v1.BoardService/GetItem", b"ask")
                .unwrap(),
            None
        );
        assert_eq!(store.last_copy("se_b", METHOD, b"ask").unwrap(), None);
    }

    #[test]
    fn a_newer_answer_to_the_same_call_replaces_the_older() {
        let (_directory, store) = store_with(&["se_a"]);
        store
            .put_last_copy("se_a", METHOD, b"ask", b"first", 5)
            .unwrap();
        store
            .put_last_copy("se_a", METHOD, b"ask", b"second", 9)
            .unwrap();
        let found = store.last_copy("se_a", METHOD, b"ask").unwrap().unwrap();
        assert_eq!(found.response, b"second");
        assert_eq!(found.answered_unix_ms, 9);
    }

    #[test]
    fn a_session_keeps_its_newest_answers_and_the_oldest_go() {
        let (_directory, store) = store_with(&["se_a", "se_b"]);
        store
            .put_last_copy("se_b", METHOD, b"theirs", b"x", 1)
            .unwrap();
        let total = ROWS_PER_SESSION + 1;
        for number in 0..total {
            let request = format!("ask {number}");
            store
                .put_last_copy("se_a", METHOD, request.as_bytes(), b"x", 10 + number)
                .unwrap();
        }
        assert_eq!(
            store.last_copy("se_a", METHOD, b"ask 0").unwrap(),
            None,
            "the oldest went"
        );
        for number in 1..total {
            let request = format!("ask {number}");
            assert!(
                store
                    .last_copy("se_a", METHOD, request.as_bytes())
                    .unwrap()
                    .is_some(),
                "answer {number} is kept"
            );
        }
        assert!(
            store
                .last_copy("se_b", METHOD, b"theirs")
                .unwrap()
                .is_some(),
            "another session's answers don't count against this one"
        );
    }

    #[test]
    fn an_answer_over_the_limit_is_not_kept_and_one_at_it_is() {
        let (_directory, store) = store_with(&["se_a"]);
        let at_limit = vec![7_u8; LARGEST_ANSWER];
        assert!(
            store
                .put_last_copy("se_a", METHOD, b"fits", &at_limit, 1)
                .unwrap()
        );
        assert!(store.last_copy("se_a", METHOD, b"fits").unwrap().is_some());
        let over = vec![7_u8; LARGEST_ANSWER + 1];
        assert!(
            !store
                .put_last_copy("se_a", METHOD, b"big", &over, 2)
                .unwrap()
        );
        assert_eq!(store.last_copy("se_a", METHOD, b"big").unwrap(), None);
    }

    #[test]
    fn a_session_removed_takes_its_answers_with_it() {
        let (_directory, store) = store_with(&["se_a"]);
        store
            .put_last_copy("se_a", METHOD, b"ask", b"answer", 1)
            .unwrap();
        store.remove_session("se_a").unwrap();
        assert_eq!(store.last_copy("se_a", METHOD, b"ask").unwrap(), None);
    }

    #[test]
    fn an_answer_for_a_session_the_store_doesnt_hold_is_refused() {
        let (_directory, store) = store_with(&[]);
        let refused = store.put_last_copy("se_gone", METHOD, b"ask", b"answer", 1);
        assert!(
            matches!(refused, Err(StoreError::NoSession { .. })),
            "{refused:?}"
        );
    }

    #[test]
    fn the_answers_survive_the_store_being_opened_again() {
        let (directory, store) = store_with(&["se_a"]);
        store
            .put_last_copy("se_a", METHOD, b"ask", b"answer", 3)
            .unwrap();
        drop(store);
        let reopened = Store::open(directory.path()).unwrap();
        let found = reopened.last_copy("se_a", METHOD, b"ask").unwrap().unwrap();
        assert_eq!(found.response, b"answer");
    }
}
