//! Each session's review comments (the comment ledger): one row per
//! comment, so a change writes only the rows it changed, not the whole
//! review. The store doesn't read the comments or the batches: they are
//! the bytes the daemon encodes, kept as given.

use rusqlite::params;

use crate::store::{check_id, from_sql, to_sql};
use crate::{Store, StoreError};

/// Whether a comment is in a batch being typed into the agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommentDeliveryState {
    /// In no batch.
    None,
    /// In a batch being typed; a crash now leaves it unknown whether it was.
    Sending,
    /// In a batch typed but not confirmed.
    Unconfirmed,
}

impl CommentDeliveryState {
    fn to_sql(self) -> i64 {
        match self {
            Self::None => 0,
            Self::Sending => 1,
            Self::Unconfirmed => 2,
        }
    }

    fn from_sql(value: i64) -> Self {
        match value {
            1 => Self::Sending,
            2 => Self::Unconfirmed,
            _ => Self::None,
        }
    }
}

/// One saved comment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommentRecord {
    /// `c-<n>`, or `note-<n>` for a review chat note made a draft in place
    /// (D284).
    pub id: String,
    pub archived: bool,
    pub delivery_state: CommentDeliveryState,
    /// The comment as the daemon encodes it.
    pub comment: Vec<u8>,
    pub updated_unix_ms: i64,
}

/// One saved batch of comments sent to the agent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommentDeliveryRecord {
    /// `d-<n>`.
    pub key: String,
    /// Sending or Unconfirmed.
    pub state: CommentDeliveryState,
    /// The batch as the daemon encodes it.
    pub delivery: Vec<u8>,
}

/// The last numbers handed out, so ids are never reused.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CommentNumbers {
    pub last_comment: u64,
    pub last_delivery: u64,
}

/// What one change to a session's comments writes, all in one transaction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommentChange {
    pub session: String,
    /// Comments added or replaced.
    pub put: Vec<CommentRecord>,
    /// Ids of comments to forget.
    pub removed: Vec<String>,
    pub put_deliveries: Vec<CommentDeliveryRecord>,
    /// Keys of batches to forget.
    pub removed_deliveries: Vec<String>,
    pub numbers: CommentNumbers,
}

/// A session's comments as saved.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionComments {
    /// Notes made drafts first, then the user's comments, each by number,
    /// as ListComments orders them.
    pub comments: Vec<CommentRecord>,
    /// By their number.
    pub deliveries: Vec<CommentDeliveryRecord>,
    pub numbers: CommentNumbers,
}

/// The number in `id`, one of `prefixes` followed by digits.
fn number(id: &str, prefixes: &[&str]) -> Result<i64, StoreError> {
    check_id(id)?;
    prefixes
        .iter()
        .find_map(|prefix| id.strip_prefix(prefix))
        .and_then(|digits| digits.parse::<u64>().ok())
        .map(to_sql)
        .ok_or_else(|| StoreError::InvalidId { id: id.to_owned() })
}

impl Store {
    /// Writes `change` in one transaction: either all of it is saved or
    /// none. Its session must be saved.
    pub fn apply_comments(&self, change: &CommentChange) -> Result<(), StoreError> {
        check_id(&change.session)?;
        let put = change
            .put
            .iter()
            .map(|comment| number(&comment.id, &["c-", "note-"]))
            .collect::<Result<Vec<_>, _>>()?;
        let put_deliveries = change
            .put_deliveries
            .iter()
            .map(|delivery| number(&delivery.key, &["d-"]))
            .collect::<Result<Vec<_>, _>>()?;
        for id in &change.removed {
            check_id(id)?;
        }
        for key in &change.removed_deliveries {
            check_id(key)?;
        }
        let inner = self.inner();
        let doing = "save a session's comments";
        let session = change.session.as_str();
        inner.write_for_session("comments.apply", doing, Some(session), |transaction| {
            let failed = inner.failed(doing);
            // The numbers row comes first: it names the session, so a
            // missing one fails before anything else is written.
            transaction
                .execute(
                    "INSERT INTO comment_numbers (session, last_comment, last_delivery)
                     VALUES (?1, ?2, ?3)
                     ON CONFLICT (session) DO UPDATE SET
                         last_comment = excluded.last_comment,
                         last_delivery = excluded.last_delivery",
                    params![
                        session,
                        to_sql(change.numbers.last_comment),
                        to_sql(change.numbers.last_delivery)
                    ],
                )
                .map_err(&failed)?;
            let mut forget = transaction
                .prepare_cached("DELETE FROM comments WHERE session = ?1 AND id = ?2")
                .map_err(&failed)?;
            for id in &change.removed {
                forget.execute(params![session, id]).map_err(&failed)?;
            }
            let mut forget_delivery = transaction
                .prepare_cached("DELETE FROM comment_deliveries WHERE session = ?1 AND key = ?2")
                .map_err(&failed)?;
            for key in &change.removed_deliveries {
                forget_delivery.execute(params![session, key]).map_err(&failed)?;
            }
            let mut save = transaction
                .prepare_cached(
                    "INSERT INTO comments
                         (session, id, n, archived, delivery_state, updated_unix_ms, comment)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                     ON CONFLICT (session, id) DO UPDATE SET
                         archived = excluded.archived,
                         delivery_state = excluded.delivery_state,
                         updated_unix_ms = excluded.updated_unix_ms,
                         comment = excluded.comment",
                )
                .map_err(&failed)?;
            for (comment, n) in change.put.iter().zip(put.iter().copied()) {
                save.execute(params![
                    session,
                    comment.id,
                    n,
                    comment.archived,
                    comment.delivery_state.to_sql(),
                    comment.updated_unix_ms,
                    comment.comment
                ])
                .map_err(&failed)?;
            }
            let mut save_delivery = transaction
                .prepare_cached(
                    "INSERT INTO comment_deliveries (session, key, n, state, delivery)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT (session, key) DO UPDATE SET
                         state = excluded.state, delivery = excluded.delivery",
                )
                .map_err(&failed)?;
            for (delivery, n) in change.put_deliveries.iter().zip(put_deliveries.iter().copied()) {
                save_delivery
                    .execute(params![session, delivery.key, n, delivery.state.to_sql(), delivery.delivery])
                    .map_err(&failed)?;
            }
            Ok(())
        })
    }

    /// The session's comments, batches and numbers; empty when it has none.
    pub fn comments(&self, session: &str) -> Result<SessionComments, StoreError> {
        check_id(session)?;
        self.inner().read("read a session's comments", |connection| {
            let comments = connection
                .prepare(
                    "SELECT id, archived, delivery_state, comment, updated_unix_ms
                     FROM comments WHERE session = ?1
                     ORDER BY id GLOB 'c-*', n",
                )?
                .query_map([session], |row| {
                    Ok(CommentRecord {
                        id: row.get(0)?,
                        archived: row.get(1)?,
                        delivery_state: CommentDeliveryState::from_sql(row.get(2)?),
                        comment: row.get(3)?,
                        updated_unix_ms: row.get(4)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let deliveries = connection
                .prepare(
                    "SELECT key, state, delivery FROM comment_deliveries
                     WHERE session = ?1 ORDER BY n",
                )?
                .query_map([session], |row| {
                    Ok(CommentDeliveryRecord {
                        key: row.get(0)?,
                        state: CommentDeliveryState::from_sql(row.get(1)?),
                        delivery: row.get(2)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let numbers = connection
                .prepare("SELECT last_comment, last_delivery FROM comment_numbers WHERE session = ?1")?
                .query_map([session], |row| {
                    Ok(CommentNumbers {
                        last_comment: from_sql(row.get(0)?),
                        last_delivery: from_sql(row.get(1)?),
                    })
                })?
                .next()
                .transpose()?
                .unwrap_or_default();
            Ok(SessionComments {
                comments,
                deliveries,
                numbers,
            })
        })
    }

    /// Sessions with a batch saved as Sending: after a restart, the ones
    /// whose batch may or may not have been typed.
    pub fn sessions_with_sending_comments(&self) -> Result<Vec<String>, StoreError> {
        self.inner().read("list sessions with comments being sent", |connection| {
            connection
                .prepare(
                    "SELECT DISTINCT session FROM comment_deliveries WHERE state = 1 ORDER BY session",
                )?
                .query_map([], |row| row.get(0))?
                .collect()
        })
    }
}
