//! Review threads, the reviews sent and the files marked viewed.

use rusqlite::{OptionalExtension, Row, params};

use super::{Store, now_unix_ms};
use crate::error::StoreError;

/// Where a new thread goes, as the file is when it is left.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewThread {
    pub session: String,
    pub repository: String,
    pub path: String,
    pub start_line: u32,
    pub end_line: u32,
    /// The lines it is left on, and those just above them.
    pub code: Vec<String>,
    pub before: Vec<String>,
    pub quote: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Thread {
    pub id: i64,
    pub session: String,
    pub repository: String,
    pub path: String,
    /// Where it was last found in the file, from 1.
    pub start_line: u32,
    pub end_line: u32,
    pub outdated: bool,
    pub code: Vec<String>,
    pub before: Vec<String>,
    pub quote: String,
    pub resolved: bool,
    /// Yours, oldest first.
    pub comments: Vec<Comment>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Comment {
    pub id: i64,
    pub text: String,
    /// The review it was sent with; none until it is.
    pub review: Option<i64>,
    pub created_unix_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Review {
    pub id: i64,
    pub message: String,
    /// The threads it had comments on.
    pub threads: Vec<i64>,
}

const THREAD_COLUMNS: &str = "SELECT id, session_id, repository, path, start_line, end_line,
    outdated, code, before, quote, resolved FROM review_threads";

/// Lines kept as a JSON array; a line may be empty.
fn lines(json: &str) -> Vec<String> {
    serde_json::from_str(json).unwrap_or_default()
}

fn json(lines: &[String]) -> String {
    serde_json::to_string(lines).unwrap_or_else(|_| "[]".to_owned())
}

fn thread(row: &Row<'_>) -> rusqlite::Result<Thread> {
    Ok(Thread {
        id: row.get(0)?,
        session: row.get(1)?,
        repository: row.get(2)?,
        path: row.get(3)?,
        start_line: row.get(4)?,
        end_line: row.get(5)?,
        outdated: row.get(6)?,
        code: lines(&row.get::<_, String>(7)?),
        before: lines(&row.get::<_, String>(8)?),
        quote: row.get(9)?,
        resolved: row.get(10)?,
        comments: Vec::new(),
    })
}

fn comment(row: &Row<'_>) -> rusqlite::Result<Comment> {
    Ok(Comment {
        id: row.get(0)?,
        text: row.get(1)?,
        review: row.get(2)?,
        created_unix_ms: row.get(3)?,
    })
}

fn unknown_thread(id: i64) -> StoreError {
    StoreError::UnknownThread { id: id.to_string() }
}

impl Store {
    /// Starts a thread with your first comment on it.
    pub fn add_thread(&self, new: &NewThread, text: &str) -> Result<Thread, StoreError> {
        let id = {
            let mut connection = self.lock();
            let added = (|| {
                let transaction = connection.transaction()?;
                let now = now_unix_ms();
                transaction.execute(
                    "INSERT INTO review_threads (session_id, repository, path, start_line,
                        end_line, code, before, quote, created_unix_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        new.session,
                        new.repository,
                        new.path,
                        new.start_line,
                        new.end_line,
                        json(&new.code),
                        json(&new.before),
                        new.quote,
                        now,
                    ],
                )?;
                let id = transaction.last_insert_rowid();
                transaction.execute(
                    "INSERT INTO review_comments (thread_id, text, created_unix_ms)
                     VALUES (?1, ?2, ?3)",
                    params![id, text, now],
                )?;
                transaction.commit()?;
                Ok(id)
            })();
            added.map_err(self.failed())?
        };
        self.thread(id)
    }

    /// Adds your comment to thread `id`, to send with the next review.
    pub fn add_comment(&self, id: i64, text: &str) -> Result<Thread, StoreError> {
        self.thread(id)?;
        self.lock()
            .execute(
                "INSERT INTO review_comments (thread_id, text, created_unix_ms)
                 VALUES (?1, ?2, ?3)",
                params![id, text, now_unix_ms()],
            )
            .map_err(self.failed())?;
        self.thread(id)
    }

    pub fn resolve_thread(&self, id: i64, resolved: bool) -> Result<Thread, StoreError> {
        let changed = self
            .lock()
            .execute(
                "UPDATE review_threads SET resolved = ?2 WHERE id = ?1",
                params![id, resolved],
            )
            .map_err(self.failed())?;
        if changed == 0 {
            return Err(unknown_thread(id));
        }
        self.thread(id)
    }

    /// Records where thread `id` was found in the file now.
    pub fn move_thread(
        &self,
        id: i64,
        start_line: u32,
        end_line: u32,
        outdated: bool,
    ) -> Result<(), StoreError> {
        self.lock()
            .execute(
                "UPDATE review_threads SET start_line = ?2, end_line = ?3, outdated = ?4
                 WHERE id = ?1",
                params![id, start_line, end_line, outdated],
            )
            .map(drop)
            .map_err(self.failed())
    }

    /// Deletes your comment `id` if it wasn't sent yet, and its thread with
    /// it if that was the thread's last comment.
    pub fn delete_comment(&self, id: i64) -> Result<(), StoreError> {
        let mut connection = self.lock();
        let deleted = (|| {
            let transaction = connection.transaction()?;
            let found: Option<(i64, Option<i64>)> = transaction
                .query_row(
                    "SELECT thread_id, review_id FROM review_comments WHERE id = ?1",
                    [id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let Some((thread, review)) = found else {
                return Ok(Err(StoreError::UnknownComment { id: id.to_string() }));
            };
            if review.is_some() {
                return Ok(Err(StoreError::CommentSent { id: id.to_string() }));
            }
            transaction.execute("DELETE FROM review_comments WHERE id = ?1", [id])?;
            transaction.execute(
                "DELETE FROM review_threads WHERE id = ?1
                 AND NOT EXISTS (SELECT 1 FROM review_comments WHERE thread_id = ?1)",
                [thread],
            )?;
            transaction.commit()?;
            Ok(Ok(()))
        })();
        deleted.map_err(self.failed())?
    }

    pub fn thread(&self, id: i64) -> Result<Thread, StoreError> {
        let connection = self.lock();
        let found = (|| {
            let Some(mut thread) = connection
                .query_row(&format!("{THREAD_COLUMNS} WHERE id = ?1"), [id], thread)
                .optional()?
            else {
                return Ok(None);
            };
            thread.comments = comments(&connection, id)?;
            Ok(Some(thread))
        })();
        found
            .map_err(self.failed())?
            .ok_or_else(|| unknown_thread(id))
    }

    /// `session`'s threads, oldest first.
    pub fn threads(&self, session: &str) -> Result<Vec<Thread>, StoreError> {
        let connection = self.lock();
        let listed = (|| {
            let mut statement = connection.prepare(&format!(
                "{THREAD_COLUMNS} WHERE session_id = ?1 ORDER BY id"
            ))?;
            let mut threads = statement
                .query_map([session], thread)?
                .collect::<rusqlite::Result<Vec<Thread>>>()?;
            for thread in &mut threads {
                thread.comments = comments(&connection, thread.id)?;
            }
            Ok(threads)
        })();
        listed.map_err(self.failed())
    }

    /// Records `message` as a review of `session` sent with `comments`.
    pub fn record_review(
        &self,
        session: &str,
        message: &str,
        comments: &[i64],
    ) -> Result<(), StoreError> {
        let mut connection = self.lock();
        let recorded = (|| {
            let transaction = connection.transaction()?;
            transaction.execute(
                "INSERT INTO reviews (session_id, message, sent_unix_ms) VALUES (?1, ?2, ?3)",
                params![session, message, now_unix_ms()],
            )?;
            let review = transaction.last_insert_rowid();
            for comment in comments {
                transaction.execute(
                    "UPDATE review_comments SET review_id = ?2 WHERE id = ?1",
                    params![comment, review],
                )?;
            }
            transaction.commit()
        })();
        recorded.map_err(self.failed())
    }

    /// The reviews sent to `session`, oldest first.
    pub fn reviews(&self, session: &str) -> Result<Vec<Review>, StoreError> {
        let connection = self.lock();
        let listed = (|| {
            let mut statement = connection
                .prepare("SELECT id, message FROM reviews WHERE session_id = ?1 ORDER BY id")?;
            let mut reviews = statement
                .query_map([session], |row| {
                    Ok(Review {
                        id: row.get(0)?,
                        message: row.get(1)?,
                        threads: Vec::new(),
                    })
                })?
                .collect::<rusqlite::Result<Vec<Review>>>()?;
            let mut statement = connection.prepare(
                "SELECT DISTINCT thread_id FROM review_comments WHERE review_id = ?1
                 ORDER BY thread_id",
            )?;
            for review in &mut reviews {
                review.threads = statement
                    .query_map([review.id], |row| row.get(0))?
                    .collect::<rusqlite::Result<Vec<i64>>>()?;
            }
            Ok(reviews)
        })();
        listed.map_err(self.failed())
    }

    /// Marks a file of `session` viewed with the contents `blob` names, or
    /// not viewed if none.
    pub fn set_viewed(
        &self,
        session: &str,
        repository: &str,
        path: &str,
        blob: Option<&str>,
    ) -> Result<(), StoreError> {
        let connection = self.lock();
        let set = match blob {
            Some(blob) => connection.execute(
                "INSERT INTO viewed_files (session_id, repository, path, blob)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (session_id, repository, path) DO UPDATE SET blob = ?4",
                params![session, repository, path, blob],
            ),
            None => connection.execute(
                "DELETE FROM viewed_files
                 WHERE session_id = ?1 AND repository = ?2 AND path = ?3",
                params![session, repository, path],
            ),
        };
        set.map(drop).map_err(self.failed())
    }

    /// The files of `session` marked viewed: repository, path and blob.
    pub fn viewed(&self, session: &str) -> Result<Vec<(String, String, String)>, StoreError> {
        let connection = self.lock();
        let listed = (|| {
            let mut statement = connection
                .prepare("SELECT repository, path, blob FROM viewed_files WHERE session_id = ?1")?;
            statement
                .query_map([session], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()
        })();
        listed.map_err(self.failed())
    }
}

fn comments(connection: &rusqlite::Connection, thread: i64) -> rusqlite::Result<Vec<Comment>> {
    let mut statement = connection.prepare(
        "SELECT id, text, review_id, created_unix_ms FROM review_comments
         WHERE thread_id = ?1 ORDER BY id",
    )?;
    statement
        .query_map([thread], comment)?
        .collect::<rusqlite::Result<Vec<Comment>>>()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let store = Store::open(&dir.path().join("humata.sqlite")).expect("opened");
        (dir, store)
    }

    fn new_thread() -> NewThread {
        NewThread {
            session: "s".into(),
            repository: "app".into(),
            path: "main.rs".into(),
            start_line: 2,
            end_line: 3,
            code: vec![String::new(), "let a = 1;".into()],
            before: vec!["fn main() {".into()],
            quote: String::new(),
        }
    }

    #[test]
    fn a_thread_keeps_its_lines_and_comments_in_order() {
        let (_dir, store) = store();
        let thread = store.add_thread(&new_thread(), "why one?").expect("added");
        assert_eq!(thread.code, new_thread().code);
        assert_eq!(thread.before, new_thread().before);
        let thread = store.add_comment(thread.id, "and not two").expect("added");
        let texts: Vec<&str> = thread.comments.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(texts, ["why one?", "and not two"]);
        assert_eq!(store.threads("s").expect("listed"), [thread]);
        assert!(store.threads("other").expect("listed").is_empty());
    }

    #[test]
    fn a_sent_comment_stays_and_the_review_knows_its_threads() {
        let (_dir, store) = store();
        let thread = store.add_thread(&new_thread(), "first").expect("added");
        let sent = thread.comments[0].id;
        store
            .record_review("s", "the review", &[sent])
            .expect("recorded");
        let reviews = store.reviews("s").expect("listed");
        assert_eq!(reviews.len(), 1);
        assert_eq!(reviews[0].threads, [thread.id]);
        assert!(matches!(
            store.delete_comment(sent),
            Err(StoreError::CommentSent { .. })
        ));
    }

    #[test]
    fn deleting_a_threads_last_comment_deletes_the_thread() {
        let (_dir, store) = store();
        let thread = store.add_thread(&new_thread(), "oops").expect("added");
        store
            .delete_comment(thread.comments[0].id)
            .expect("deleted");
        assert!(matches!(
            store.thread(thread.id),
            Err(StoreError::UnknownThread { .. })
        ));
        assert!(matches!(
            store.delete_comment(thread.comments[0].id),
            Err(StoreError::UnknownComment { .. })
        ));
    }

    #[test]
    fn a_file_is_viewed_with_its_contents_until_unmarked() {
        let (_dir, store) = store();
        store
            .set_viewed("s", "app", "a.rs", Some("one"))
            .expect("set");
        store
            .set_viewed("s", "app", "a.rs", Some("two"))
            .expect("set");
        assert_eq!(
            store.viewed("s").expect("listed"),
            [("app".to_owned(), "a.rs".to_owned(), "two".to_owned())]
        );
        store.set_viewed("s", "app", "a.rs", None).expect("unset");
        assert!(store.viewed("s").expect("listed").is_empty());
    }

    #[test]
    fn resolving_an_unknown_thread_fails() {
        let (_dir, store) = store();
        assert!(matches!(
            store.resolve_thread(7, true),
            Err(StoreError::UnknownThread { .. })
        ));
    }
}
