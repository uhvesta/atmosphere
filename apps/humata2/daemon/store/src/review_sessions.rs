//! What a pull request review session is bound to (PR-4..PR-8, PR-15,
//! D283): the pull requests it reviews, in the order they were asked for,
//! with each one's base and head branch, the head sha the clone was made at
//! (the reviewed head), the last head a check saw, its state, and when the
//! attempt to post its review began (D287); and whether the agent may push.
//! The agent's switches (Focus, Notes, Order) live with the review chat's own
//! state, not here.
//!
//! A session with a row here is a review session: identity derives
//! `SessionKind::Review { push_allowed }` from it on every call.

use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::store::check_id;
use crate::{Store, StoreError};

/// One pull request of a review session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewPr {
    pub owner: String,
    pub name: String,
    pub number: u32,
    pub base_branch: String,
    /// The head branch's name (`patch-1` for a fork's), which the clone is
    /// checked out on.
    pub head_branch: String,
    /// The head the clone was made at or last updated to: the commit the
    /// review is of, and the `commit_id` it posts with.
    pub head_sha: String,
    /// The newest head a check saw; differs from `head_sha` while there are
    /// commits the clone doesn't have.
    pub checked_sha: String,
    /// `humata.v1.PullRequestState` as its number.
    pub state: i32,
    /// When the attempt to post this pull request's review began, kept
    /// before the post so a restart can look for it on GitHub; none before
    /// the first attempt.
    pub submit_started_unix_ms: Option<i64>,
}

/// A pull request review session's binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewSessionRecord {
    pub session: String,
    /// PR-8: only the user sets it.
    pub push_allowed: bool,
    /// In the order the review was started with.
    pub prs: Vec<ReviewPr>,
}

const PR_COLUMNS: &str = "owner, name, number, base_branch, head_branch, head_sha, checked_sha, state, \
                          submit_started_unix_ms";

fn row_pr(row: &Row<'_>) -> rusqlite::Result<ReviewPr> {
    let number: i64 = row.get(2)?;
    Ok(ReviewPr {
        owner: row.get(0)?,
        name: row.get(1)?,
        number: u32::try_from(number).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(2, number))?,
        base_branch: row.get(3)?,
        head_branch: row.get(4)?,
        head_sha: row.get(5)?,
        checked_sha: row.get(6)?,
        state: row.get(7)?,
        submit_started_unix_ms: row.get(8)?,
    })
}

fn prs_of(connection: &Connection, session: &str) -> rusqlite::Result<Vec<ReviewPr>> {
    let mut statement = connection.prepare(&format!(
        "SELECT {PR_COLUMNS} FROM review_session_prs WHERE session = ?1 ORDER BY ordinal"
    ))?;
    statement.query_map([session], row_pr)?.collect()
}

fn record_of(connection: &Connection, session: &str, push_allowed: i64) -> rusqlite::Result<ReviewSessionRecord> {
    Ok(ReviewSessionRecord {
        session: session.to_owned(),
        push_allowed: push_allowed != 0,
        prs: prs_of(connection, session)?,
    })
}

impl Store {
    /// Saves `record` for its session, in place of any binding it had, in
    /// one transaction.
    pub fn put_review_session(&self, record: &ReviewSessionRecord) -> Result<(), StoreError> {
        check_id(&record.session)?;
        let inner = self.inner();
        inner.write_for_session(
            "review_session.put",
            "save a review session's pull requests",
            Some(&record.session),
            |transaction| {
                let failed = inner.failed("save a review session's pull requests");
                transaction
                    .execute(
                        "INSERT INTO review_sessions (session, push_allowed) VALUES (?1, ?2)
                         ON CONFLICT (session) DO UPDATE SET push_allowed = excluded.push_allowed",
                        params![record.session, record.push_allowed],
                    )
                    .map_err(&failed)?;
                transaction
                    .execute("DELETE FROM review_session_prs WHERE session = ?1", [&record.session])
                    .map_err(&failed)?;
                for (ordinal, pr) in record.prs.iter().enumerate() {
                    transaction
                        .execute(
                            &format!(
                                "INSERT INTO review_session_prs (session, ordinal, {PR_COLUMNS})
                                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"
                            ),
                            params![
                                record.session,
                                i64::try_from(ordinal).unwrap_or(i64::MAX),
                                pr.owner,
                                pr.name,
                                i64::from(pr.number),
                                pr.base_branch,
                                pr.head_branch,
                                pr.head_sha,
                                pr.checked_sha,
                                pr.state,
                                pr.submit_started_unix_ms,
                            ],
                        )
                        .map_err(&failed)?;
                }
                Ok(())
            },
        )
    }

    /// The binding of `session`, if it is a review session.
    pub fn review_session(&self, session: &str) -> Result<Option<ReviewSessionRecord>, StoreError> {
        check_id(session)?;
        self.inner().read("read a review session's pull requests", |connection| {
            let push_allowed: Option<i64> = connection
                .query_row(
                    "SELECT push_allowed FROM review_sessions WHERE session = ?1",
                    [session],
                    |row| row.get(0),
                )
                .optional()?;
            push_allowed
                .map(|push_allowed| record_of(connection, session, push_allowed))
                .transpose()
        })
    }

    /// Every review session's binding, ordered by session id.
    pub fn review_sessions(&self) -> Result<Vec<ReviewSessionRecord>, StoreError> {
        self.inner().read("read the review sessions' pull requests", |connection| {
            let mut statement =
                connection.prepare("SELECT session, push_allowed FROM review_sessions ORDER BY session")?;
            let heads: Vec<(String, i64)> = statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;
            heads
                .into_iter()
                .map(|(session, push_allowed)| record_of(connection, &session, push_allowed))
                .collect()
        })
    }

    /// Lets the review session's agent push (or not), PR-8. Returns whether
    /// `session` is a review session; if it isn't, nothing changed.
    pub fn set_review_push_allowed(&self, session: &str, allowed: bool) -> Result<bool, StoreError> {
        check_id(session)?;
        let inner = self.inner();
        inner.write_for_session(
            "review_session.push_allowed",
            "change whether a review session may push",
            Some(session),
            |transaction| {
                transaction
                    .execute(
                        "UPDATE review_sessions SET push_allowed = ?2 WHERE session = ?1",
                        params![session, allowed],
                    )
                    .map(|changed| changed > 0)
                    .map_err(inner.failed("change whether a review session may push"))
            },
        )
    }

    /// Keeps when the attempt to post one pull request's review began, or
    /// clears it (`None`) once the attempt is settled (D287). Returns whether
    /// `session` has that pull request; if it doesn't, nothing changed.
    pub fn set_review_submit_started(
        &self,
        session: &str,
        owner: &str,
        name: &str,
        number: u32,
        started_unix_ms: Option<i64>,
    ) -> Result<bool, StoreError> {
        check_id(session)?;
        let inner = self.inner();
        inner.write_for_session(
            "review_session.submit_started",
            "keep when a review's posting began",
            Some(session),
            |transaction| {
                transaction
                    .execute(
                        "UPDATE review_session_prs SET submit_started_unix_ms = ?5
                         WHERE session = ?1 AND owner = ?2 AND name = ?3 AND number = ?4",
                        params![session, owner, name, i64::from(number), started_unix_ms],
                    )
                    .map(|changed| changed > 0)
                    .map_err(inner.failed("keep when a review's posting began"))
            },
        )
    }

    /// Keeps the newest head a check saw and the pull request's state, and
    /// the head the clone is at when `head_sha` is given, in one UPDATE, so
    /// it never undoes another field's write. Returns whether `session` has
    /// that pull request.
    pub fn set_review_pr_heads(
        &self,
        session: &str,
        owner: &str,
        name: &str,
        number: u32,
        head_sha: Option<&str>,
        checked_sha: &str,
        state: i32,
    ) -> Result<bool, StoreError> {
        check_id(session)?;
        let inner = self.inner();
        inner.write_for_session(
            "review_session.heads",
            "keep a review's pull request heads",
            Some(session),
            |transaction| {
                transaction
                    .execute(
                        "UPDATE review_session_prs
                         SET head_sha = COALESCE(?5, head_sha), checked_sha = ?6, state = ?7
                         WHERE session = ?1 AND owner = ?2 AND name = ?3 AND number = ?4",
                        params![session, owner, name, i64::from(number), head_sha, checked_sha, state],
                    )
                    .map(|changed| changed > 0)
                    .map_err(inner.failed("keep a review's pull request heads"))
            },
        )
    }

    /// Records the branch a review's pull request merges into now, after
    /// GitHub retargeted it (A8, D298). Returns whether `session` has that
    /// pull request.
    pub fn set_review_pr_base(
        &self,
        session: &str,
        owner: &str,
        name: &str,
        number: u32,
        base_branch: &str,
    ) -> Result<bool, StoreError> {
        check_id(session)?;
        let inner = self.inner();
        inner.write_for_session(
            "review_session.base",
            "keep a review's pull request base",
            Some(session),
            |transaction| {
                transaction
                    .execute(
                        "UPDATE review_session_prs SET base_branch = ?5
                         WHERE session = ?1 AND owner = ?2 AND name = ?3 AND number = ?4",
                        params![session, owner, name, i64::from(number), base_branch],
                    )
                    .map(|changed| changed > 0)
                    .map_err(inner.failed("keep a review's pull request base"))
            },
        )
    }
}
