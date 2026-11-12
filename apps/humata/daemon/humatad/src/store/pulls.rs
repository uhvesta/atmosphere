//! Sessions' pull requests as last read from GitHub, the issues imported
//! as tasks, and the GitHub review comments brought into threads.

use rusqlite::{OptionalExtension, Row, params};

use super::{Store, now_unix_ms};
use crate::error::StoreError;

/// A pull request of a session's, one per repository it landed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullRequest {
    pub session: String,
    /// The session's name for the repository.
    pub repository: String,
    /// On GitHub, as `owner/name`.
    pub github: String,
    pub number: u64,
    pub url: String,
    pub title: String,
    pub branch: String,
    pub base: String,
    pub state: PullState,
    pub review: Standing,
    pub checks: Checks,
    pub read_unix_ms: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PullState {
    Open,
    Merged,
    Closed,
}

/// Where the reviewers stand, the one most in the way first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Standing {
    None,
    ChangesRequested,
    Commented,
    Approved,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Checks {
    None,
    Pending,
    Passing,
    Failing,
}

/// The names columns keep each in.
macro_rules! named {
    ($kind:ident { $($variant:ident => $name:literal),* $(,)? }) => {
        impl $kind {
            pub fn name(self) -> &'static str {
                match self { $(Self::$variant => $name),* }
            }

            fn named(name: &str) -> Self {
                match name {
                    $($name => Self::$variant,)*
                    _ => Self::default_variant(),
                }
            }
        }
    };
}

impl PullState {
    const fn default_variant() -> Self {
        Self::Open
    }
}
impl Standing {
    const fn default_variant() -> Self {
        Self::None
    }
}
impl Checks {
    const fn default_variant() -> Self {
        Self::None
    }
}

named!(PullState { Open => "open", Merged => "merged", Closed => "closed" });
named!(Standing {
    None => "none",
    ChangesRequested => "changes_requested",
    Commented => "commented",
    Approved => "approved",
});
named!(Checks { None => "none", Pending => "pending", Passing => "passing", Failing => "failing" });

const PULL_COLUMNS: &str = "SELECT session_id, repository, github, number, url, title, branch,
    base, state, review, checks, read_unix_ms FROM pull_requests";

fn pull(row: &Row<'_>) -> rusqlite::Result<PullRequest> {
    Ok(PullRequest {
        session: row.get(0)?,
        repository: row.get(1)?,
        github: row.get(2)?,
        number: row.get::<_, i64>(3)?.try_into().unwrap_or_default(),
        url: row.get(4)?,
        title: row.get(5)?,
        branch: row.get(6)?,
        base: row.get(7)?,
        state: PullState::named(&row.get::<_, String>(8)?),
        review: Standing::named(&row.get::<_, String>(9)?),
        checks: Checks::named(&row.get::<_, String>(10)?),
        read_unix_ms: row.get(11)?,
    })
}

impl Store {
    /// Keeps `pull`, replacing what was kept of the session's pull request
    /// in that repository.
    pub fn keep_pull(&self, pull: &PullRequest) -> Result<(), StoreError> {
        self.lock()
            .execute(
                "INSERT INTO pull_requests (session_id, repository, github, number, url, title,
                     branch, base, state, review, checks, read_unix_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                 ON CONFLICT (session_id, repository) DO UPDATE SET
                     github = excluded.github, number = excluded.number, url = excluded.url,
                     title = excluded.title, branch = excluded.branch, base = excluded.base,
                     state = excluded.state, review = excluded.review, checks = excluded.checks,
                     read_unix_ms = excluded.read_unix_ms",
                params![
                    pull.session,
                    pull.repository,
                    pull.github,
                    i64::try_from(pull.number).unwrap_or(i64::MAX),
                    pull.url,
                    pull.title,
                    pull.branch,
                    pull.base,
                    pull.state.name(),
                    pull.review.name(),
                    pull.checks.name(),
                    pull.read_unix_ms,
                ],
            )
            .map(drop)
            .map_err(self.failed())
    }

    /// The session's pull requests, by repository.
    pub fn pulls(&self, session: &str) -> Result<Vec<PullRequest>, StoreError> {
        let connection = self.lock();
        let mut statement = connection
            .prepare(&format!(
                "{PULL_COLUMNS} WHERE session_id = ?1 ORDER BY repository"
            ))
            .map_err(self.failed())?;
        statement
            .query_map([session], pull)
            .and_then(Iterator::collect)
            .map_err(self.failed())
    }

    /// Every pull request still open, to read again.
    pub fn open_pulls(&self) -> Result<Vec<PullRequest>, StoreError> {
        let connection = self.lock();
        let mut statement = connection
            .prepare(&format!(
                "{PULL_COLUMNS} WHERE state = 'open' ORDER BY session_id, repository"
            ))
            .map_err(self.failed())?;
        statement
            .query_map([], pull)
            .and_then(Iterator::collect)
            .map_err(self.failed())
    }

    /// Records that every pull request of the session merged; true the first
    /// time. Its clean-up on the host is then pending until
    /// [`Store::mark_cleaned`].
    pub fn mark_merged(&self, session: &str) -> Result<bool, StoreError> {
        self.lock()
            .execute(
                "INSERT INTO cleaned_sessions (session_id, cleaned_unix_ms, host_pending)
                 VALUES (?1, ?2, 1) ON CONFLICT (session_id) DO NOTHING",
                params![session, now_unix_ms()],
            )
            .map(|changed| changed > 0)
            .map_err(self.failed())
    }

    /// The merged sessions whose host hasn't archived them and deleted their
    /// branches yet.
    pub fn cleanups_pending(&self) -> Result<Vec<String>, StoreError> {
        let connection = self.lock();
        let mut statement = connection
            .prepare("SELECT session_id FROM cleaned_sessions WHERE host_pending = 1 ORDER BY session_id")
            .map_err(self.failed())?;
        statement
            .query_map([], |row| row.get(0))
            .and_then(Iterator::collect)
            .map_err(self.failed())
    }

    /// Records that the merged session's worktrees and branches are gone.
    pub fn mark_cleaned(&self, session: &str) -> Result<(), StoreError> {
        self.lock()
            .execute(
                "UPDATE cleaned_sessions SET host_pending = 0, cleaned_unix_ms = ?2
                 WHERE session_id = ?1",
                params![session, now_unix_ms()],
            )
            .map(drop)
            .map_err(self.failed())
    }

    /// The task made of the issue `key`, as `owner/name#12`, if it was
    /// imported into `project` before.
    pub fn imported_issue(&self, project: &str, key: &str) -> Result<Option<String>, StoreError> {
        self.lock()
            .query_row(
                "SELECT item_id FROM imported_issues WHERE project_id = ?1 AND issue = ?2",
                params![project, key],
                |row| row.get(0),
            )
            .optional()
            .map_err(self.failed())
    }

    pub fn record_issue(&self, project: &str, key: &str, item: &str) -> Result<(), StoreError> {
        self.lock()
            .execute(
                "INSERT INTO imported_issues (project_id, issue, item_id) VALUES (?1, ?2, ?3)
                 ON CONFLICT (project_id, issue) DO UPDATE SET item_id = excluded.item_id",
                params![project, key, item],
            )
            .map(drop)
            .map_err(self.failed())
    }

    /// The issue `item` was imported from, as `owner/name#12`.
    pub fn issue_of(&self, item: &str) -> Result<Option<String>, StoreError> {
        self.lock()
            .query_row(
                "SELECT issue FROM imported_issues WHERE item_id = ?1",
                [item],
                |row| row.get(0),
            )
            .optional()
            .map_err(self.failed())
    }

    /// The thread a GitHub review comment was brought into, if it was.
    pub fn github_thread(&self, comment: u64) -> Result<Option<i64>, StoreError> {
        self.lock()
            .query_row(
                "SELECT thread_id FROM review_comments WHERE github_id = ?1",
                [i64::try_from(comment).unwrap_or(i64::MAX)],
                |row| row.get(0),
            )
            .optional()
            .map_err(self.failed())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let store = Store::open(&dir.path().join("humata.sqlite")).expect("opened");
        (dir, store)
    }

    fn pull(session: &str, repository: &str, state: PullState) -> PullRequest {
        PullRequest {
            session: session.into(),
            repository: repository.into(),
            github: "acme/widgets".into(),
            number: 7,
            url: "https://github.com/acme/widgets/pull/7".into(),
            title: "Keep notes".into(),
            branch: "keep-notes".into(),
            base: "main".into(),
            state,
            review: Standing::None,
            checks: Checks::None,
            read_unix_ms: 1,
        }
    }

    #[test]
    fn a_pull_request_read_again_replaces_the_one_kept() {
        let (_dir, store) = store();
        store
            .keep_pull(&pull("s", "app", PullState::Open))
            .expect("kept");
        store
            .keep_pull(&pull("s", "lib", PullState::Open))
            .expect("kept");
        let mut approved = pull("s", "app", PullState::Merged);
        approved.review = Standing::Approved;
        approved.checks = Checks::Failing;
        store.keep_pull(&approved).expect("kept");
        assert_eq!(
            store.pulls("s").expect("read"),
            [approved, pull("s", "lib", PullState::Open)]
        );
        assert_eq!(store.open_pulls().expect("read").len(), 1);
        assert!(store.pulls("other").expect("read").is_empty());
    }

    #[test]
    fn a_merged_session_waits_for_its_clean_up_until_it_is_done() {
        let (_dir, store) = store();
        assert!(store.mark_merged("s").expect("marked"));
        assert!(!store.mark_merged("s").expect("marked"));
        assert_eq!(store.cleanups_pending().expect("read"), ["s"]);
        store.mark_cleaned("s").expect("cleaned");
        assert!(store.cleanups_pending().expect("read").is_empty());
        assert!(!store.mark_merged("s").expect("marked"));
    }

    #[test]
    fn an_issue_is_imported_into_a_project_once() {
        let (_dir, store) = store();
        assert_eq!(
            store.imported_issue("p", "acme/widgets#3").expect("read"),
            None
        );
        store
            .record_issue("p", "acme/widgets#3", "item")
            .expect("recorded");
        assert_eq!(
            store
                .imported_issue("p", "acme/widgets#3")
                .expect("read")
                .as_deref(),
            Some("item")
        );
        assert_eq!(
            store.issue_of("item").expect("read").as_deref(),
            Some("acme/widgets#3")
        );
        assert_eq!(
            store.imported_issue("q", "acme/widgets#3").expect("read"),
            None
        );
    }
}
