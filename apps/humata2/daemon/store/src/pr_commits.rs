//! Which commits of a pull request the user reviewed (A9, D299): a mark per
//! commit sha, and the history of marks, unmarks, carries and whole-PR
//! reviews, which only ever grows. Both belong to the pull request (owner,
//! name, number), not to a session, so every review of it shares them, and
//! deleting a session or a channel leaves them.

use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};

use crate::{Store, StoreError};

/// Who made a mark or an entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkedBy {
    /// The `humata.v1.Actor`, encoded.
    pub actor: Vec<u8>,
    /// The `gh` login at the time; empty if it couldn't be read.
    pub gh_login: String,
}

/// One commit's mark.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitMark {
    pub sha: String,
    /// `git patch-id --stable` of the commit; empty when it has none, as a
    /// merge.
    pub patch_id: String,
    /// False once unmarked. The row stays, so a commit the user unmarked is
    /// never carried to.
    pub reviewed: bool,
    /// When a force push carried the mark here, the sha that was marked;
    /// else empty.
    pub reviewed_as_sha: String,
    pub reviewed_unix_ms: i64,
    /// The head when the mark was made.
    pub reviewed_head_sha: String,
    pub by: Option<MarkedBy>,
}

/// What one history entry says happened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewEvent {
    pub at_unix_ms: i64,
    /// `humata.v1.CommitReviewEntry.Kind` as its number.
    pub kind: i32,
    pub commits: Vec<String>,
    pub head_sha: String,
    /// The RV-6 snapshot of a whole-PR review.
    pub snapshot_n: Option<u32>,
    /// None for what nobody did, as a carry.
    pub by: Option<MarkedBy>,
}

/// A history entry, numbered 1, 2, ... in the order they were made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewEntry {
    pub seq: u32,
    pub event: ReviewEvent,
}

/// What a MarkCommitsReviewed call with an idempotency key got.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeptAnswer {
    /// The encoded answer.
    Kept(Vec<u8>),
    /// The call marked, but its answer isn't kept: it stopped before
    /// keeping it, or newer answers took its place.
    Lost,
}

/// How many answers a pull request keeps for repeats.
const KEPT_ANSWERS: i64 = 64;

/// A pull request's marks, by sha, and its history, oldest first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PrCommitMarks {
    pub marks: Vec<CommitMark>,
    pub history: Vec<ReviewEntry>,
}

fn by_of(actor: Option<Vec<u8>>, gh_login: String) -> Option<MarkedBy> {
    actor.map(|actor| MarkedBy { actor, gh_login })
}

fn number_of(row: &Row<'_>, index: usize) -> rusqlite::Result<u32> {
    let value: i64 = row.get(index)?;
    u32::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
}

fn row_mark(row: &Row<'_>) -> rusqlite::Result<CommitMark> {
    Ok(CommitMark {
        sha: row.get(0)?,
        patch_id: row.get(1)?,
        reviewed: row.get(2)?,
        reviewed_as_sha: row.get(3)?,
        reviewed_unix_ms: row.get(4)?,
        reviewed_head_sha: row.get(5)?,
        by: by_of(row.get(6)?, row.get(7)?),
    })
}

fn row_entry(row: &Row<'_>) -> rusqlite::Result<ReviewEntry> {
    let commits: String = row.get(3)?;
    let snapshot_n = row
        .get::<_, Option<i64>>(5)?
        .map(|n| u32::try_from(n).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(5, n)))
        .transpose()?;
    Ok(ReviewEntry {
        seq: number_of(row, 0)?,
        event: ReviewEvent {
            at_unix_ms: row.get(1)?,
            kind: row.get(2)?,
            commits: commits.lines().map(str::to_owned).collect(),
            head_sha: row.get(4)?,
            snapshot_n,
            by: by_of(row.get(6)?, row.get(7)?),
        },
    })
}

fn marks_of(
    connection: &Connection,
    owner: &str,
    name: &str,
    number: u32,
) -> rusqlite::Result<PrCommitMarks> {
    let number = i64::from(number);
    let marks = connection
        .prepare(
            "SELECT sha, patch_id, reviewed, reviewed_as_sha, reviewed_unix_ms,
                    reviewed_head_sha, actor, gh_login
             FROM pr_commit_marks WHERE owner = ?1 AND name = ?2 AND number = ?3
             ORDER BY sha",
        )?
        .query_map(params![owner, name, number], row_mark)?
        .collect::<rusqlite::Result<_>>()?;
    let history = connection
        .prepare(
            "SELECT seq, at_unix_ms, kind, commits, head_sha, snapshot_n, actor, gh_login
             FROM pr_review_history WHERE owner = ?1 AND name = ?2 AND number = ?3
             ORDER BY seq",
        )?
        .query_map(params![owner, name, number], row_entry)?
        .collect::<rusqlite::Result<_>>()?;
    Ok(PrCommitMarks { marks, history })
}

fn put_mark(
    transaction: &Transaction<'_>,
    owner: &str,
    name: &str,
    number: u32,
    mark: &CommitMark,
) -> rusqlite::Result<()> {
    let (actor, login) = match &mark.by {
        Some(by) => (Some(by.actor.as_slice()), by.gh_login.as_str()),
        None => (None, ""),
    };
    transaction.execute(
        "INSERT INTO pr_commit_marks (owner, name, number, sha, patch_id, reviewed,
                                      reviewed_as_sha, reviewed_unix_ms, reviewed_head_sha,
                                      actor, gh_login)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT (owner, name, number, sha) DO UPDATE SET
             patch_id = excluded.patch_id, reviewed = excluded.reviewed,
             reviewed_as_sha = excluded.reviewed_as_sha,
             reviewed_unix_ms = excluded.reviewed_unix_ms,
             reviewed_head_sha = excluded.reviewed_head_sha, actor = excluded.actor,
             gh_login = excluded.gh_login",
        params![
            owner,
            name,
            i64::from(number),
            mark.sha,
            mark.patch_id,
            mark.reviewed,
            mark.reviewed_as_sha,
            mark.reviewed_unix_ms,
            mark.reviewed_head_sha,
            actor,
            login,
        ],
    )?;
    Ok(())
}

fn add_entry(
    transaction: &Transaction<'_>,
    owner: &str,
    name: &str,
    number: u32,
    event: &ReviewEvent,
    key: Option<&str>,
) -> rusqlite::Result<()> {
    let (actor, login) = match &event.by {
        Some(by) => (Some(by.actor.as_slice()), by.gh_login.as_str()),
        None => (None, ""),
    };
    transaction.execute(
        "INSERT INTO pr_review_history (owner, name, number, seq, at_unix_ms, kind, commits,
                                        head_sha, snapshot_n, actor, gh_login,
                                        idempotency_key)
         VALUES (?1, ?2, ?3,
                 (SELECT COALESCE(MAX(seq), 0) + 1 FROM pr_review_history
                  WHERE owner = ?1 AND name = ?2 AND number = ?3),
                 ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            owner,
            name,
            i64::from(number),
            event.at_unix_ms,
            event.kind,
            event.commits.join("\n"),
            event.head_sha,
            event.snapshot_n.map(i64::from),
            actor,
            login,
            key,
        ],
    )?;
    Ok(())
}

impl Store {
    /// The marks and history of pull request `owner/name#number`; none
    /// before its first mark.
    pub fn pr_commit_marks(
        &self,
        owner: &str,
        name: &str,
        number: u32,
    ) -> Result<PrCommitMarks, StoreError> {
        self.inner()
            .read("read a pull request's reviewed commits", |connection| {
                marks_of(connection, owner, name, number)
            })
    }

    /// Saves `marks` of pull request `owner/name#number`, in place of the
    /// marks of the same shas, and appends `event` to its history, in one
    /// transaction. Returns the marks and history as they are now.
    pub fn add_pr_commit_marks(
        &self,
        owner: &str,
        name: &str,
        number: u32,
        marks: &[CommitMark],
        event: &ReviewEvent,
    ) -> Result<PrCommitMarks, StoreError> {
        self.add_marks(owner, name, number, marks, event, None)
    }

    /// As [`Store::add_pr_commit_marks`], for the call with idempotency key
    /// `key`: the entry records it, so [`Store::pr_marks_answer`] knows the
    /// call was made. A key is used once per pull request.
    pub fn add_keyed_pr_commit_marks(
        &self,
        owner: &str,
        name: &str,
        number: u32,
        marks: &[CommitMark],
        event: &ReviewEvent,
        key: &str,
    ) -> Result<PrCommitMarks, StoreError> {
        self.add_marks(owner, name, number, marks, event, Some(key))
    }

    fn add_marks(
        &self,
        owner: &str,
        name: &str,
        number: u32,
        marks: &[CommitMark],
        event: &ReviewEvent,
        key: Option<&str>,
    ) -> Result<PrCommitMarks, StoreError> {
        let doing = "keep a pull request's reviewed commits";
        self.inner().write("pr_commits.mark", doing, |transaction| {
            for mark in marks {
                put_mark(transaction, owner, name, number, mark)?;
            }
            add_entry(transaction, owner, name, number, event, key)?;
            marks_of(transaction, owner, name, number)
        })
    }

    /// What the call with idempotency key `key` on pull request
    /// `owner/name#number` got; `None` if no such call marked.
    pub fn pr_marks_answer(
        &self,
        owner: &str,
        name: &str,
        number: u32,
        key: &str,
    ) -> Result<Option<KeptAnswer>, StoreError> {
        self.inner()
            .read("read a repeated mark's answer", |connection| {
                connection
                    .query_row(
                        "SELECT answer FROM pr_review_history
                         WHERE owner = ?1 AND name = ?2 AND number = ?3
                           AND idempotency_key = ?4",
                        params![owner, name, i64::from(number), key],
                        |row| row.get::<_, Option<Vec<u8>>>(0),
                    )
                    .optional()
                    .map(|found| {
                        found.map(|answer| answer.map_or(KeptAnswer::Lost, KeptAnswer::Kept))
                    })
            })
    }

    /// Keeps `answer` for a repeat of the call with idempotency key `key`
    /// on pull request `owner/name#number`, dropping the answers older than
    /// its latest few; their entries keep their keys.
    pub fn keep_pr_marks_answer(
        &self,
        owner: &str,
        name: &str,
        number: u32,
        key: &str,
        answer: &[u8],
    ) -> Result<(), StoreError> {
        let doing = "keep a mark's answer for a repeat";
        self.inner()
            .write("pr_commits.answer", doing, |transaction| {
                let number = i64::from(number);
                transaction.execute(
                    "UPDATE pr_review_history SET answer = ?5
                 WHERE owner = ?1 AND name = ?2 AND number = ?3 AND idempotency_key = ?4",
                    params![owner, name, number, key, answer],
                )?;
                transaction.execute(
                    "UPDATE pr_review_history SET answer = NULL
                 WHERE owner = ?1 AND name = ?2 AND number = ?3 AND answer IS NOT NULL
                   AND seq NOT IN (
                       SELECT seq FROM pr_review_history
                       WHERE owner = ?1 AND name = ?2 AND number = ?3 AND answer IS NOT NULL
                       ORDER BY seq DESC LIMIT ?4)",
                    params![owner, name, number, KEPT_ANSWERS],
                )?;
                Ok(())
            })
    }
}
