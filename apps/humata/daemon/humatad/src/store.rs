//! The daemon's own records, in SQLite: projects, and which project and host
//! each session belongs to. A session's history stays with the agent on its
//! host; this only says where to find it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use humata_agent::Repository;
use humata_wire::pb;
use prost::Message as _;
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::board::{Author, Change, Item, ItemFields, ItemPatch, Priority, Status};
use crate::error::StoreError;

mod approvals;
mod planning;
mod pulls;
mod recoveries;
mod requests;
mod review;
mod search;

pub use planning::{Decision, Skill};
pub use pulls::{Checks, PullRequest, PullState, Standing};
pub use review::{Comment, NewThread, Review, Thread};
pub use search::{Entry, EntryKind, Found, Indexed, MARK_END, MARK_START, Query, Whereabouts};

/// Each step takes the schema from its index to the next version; the
/// version is kept in `PRAGMA user_version`.
const MIGRATIONS: &[&str] = &["
    CREATE TABLE projects (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        repository TEXT NOT NULL,
        instructions TEXT NOT NULL,
        created_unix_ms INTEGER NOT NULL
    );
    CREATE TABLE sessions (
        id TEXT PRIMARY KEY,
        project_id TEXT REFERENCES projects (id),
        host TEXT NOT NULL
    );
    CREATE INDEX sessions_by_project ON sessions (project_id);
", "
    CREATE TABLE project_repositories (
        project_id TEXT NOT NULL REFERENCES projects (id),
        position INTEGER NOT NULL,
        name TEXT NOT NULL,
        path TEXT NOT NULL,
        PRIMARY KEY (project_id, position)
    );
    -- Each project had one repository, named after its folder.
    INSERT INTO project_repositories (project_id, position, name, path)
        SELECT id, 0,
            coalesce(
                nullif(replace(repository, rtrim(repository, replace(repository, '/', '')), ''), ''),
                'repository'
            ),
            repository
        FROM projects;
", "
    CREATE TABLE items (
        id TEXT PRIMARY KEY,
        project_id TEXT NOT NULL REFERENCES projects (id),
        parent_id TEXT REFERENCES items (id),
        title TEXT NOT NULL,
        spec TEXT NOT NULL,
        status TEXT NOT NULL,
        priority TEXT NOT NULL,
        created_unix_ms INTEGER NOT NULL,
        updated_unix_ms INTEGER NOT NULL
    );
    CREATE INDEX items_by_project ON items (project_id);
    CREATE INDEX items_by_parent ON items (parent_id);
    ALTER TABLE sessions ADD COLUMN item_id TEXT REFERENCES items (id);
    CREATE INDEX sessions_by_item ON sessions (item_id);
    -- Each item's fields before and after, as JSON; null before for an
    -- added item and after for a removed one.
    CREATE TABLE board_changes (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        project_id TEXT NOT NULL REFERENCES projects (id),
        item_id TEXT NOT NULL,
        time_unix_ms INTEGER NOT NULL,
        before TEXT,
        after TEXT,
        reverts INTEGER REFERENCES board_changes (id)
    );
    CREATE INDEX board_changes_by_project ON board_changes (project_id, id);
", "
    -- What the daemon last heard of each session on another machine, to
    -- show while that machine can't be reached: its info, and its history
    -- as last read, compressed. Both are protobuf.
    CREATE TABLE remote_sessions (
        id TEXT PRIMARY KEY,
        host TEXT NOT NULL,
        info BLOB NOT NULL,
        history BLOB
    );
    CREATE INDEX remote_sessions_by_host ON remote_sessions (host);
", "
    -- Review threads on lines of a session's files. `code` is the lines it
    -- was left on and `before` the lines above them, each a JSON array, to
    -- find them again as the file changes.
    CREATE TABLE review_threads (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        session_id TEXT NOT NULL,
        repository TEXT NOT NULL,
        path TEXT NOT NULL,
        start_line INTEGER NOT NULL,
        end_line INTEGER NOT NULL,
        outdated INTEGER NOT NULL DEFAULT 0,
        code TEXT NOT NULL,
        before TEXT NOT NULL,
        quote TEXT NOT NULL,
        resolved INTEGER NOT NULL DEFAULT 0,
        created_unix_ms INTEGER NOT NULL
    );
    CREATE INDEX review_threads_by_session ON review_threads (session_id, id);
    -- Each review as sent to the agent, to find its replies in the
    -- session's history.
    CREATE TABLE reviews (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        session_id TEXT NOT NULL,
        message TEXT NOT NULL,
        sent_unix_ms INTEGER NOT NULL
    );
    CREATE INDEX reviews_by_session ON reviews (session_id, id);
    -- Your comments; the agent's replies are read from the history.
    CREATE TABLE review_comments (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        thread_id INTEGER NOT NULL REFERENCES review_threads (id) ON DELETE CASCADE,
        text TEXT NOT NULL,
        review_id INTEGER REFERENCES reviews (id),
        created_unix_ms INTEGER NOT NULL
    );
    CREATE INDEX review_comments_by_thread ON review_comments (thread_id, id);
    -- Files marked viewed, with the contents they had then.
    CREATE TABLE viewed_files (
        session_id TEXT NOT NULL,
        repository TEXT NOT NULL,
        path TEXT NOT NULL,
        blob TEXT NOT NULL,
        PRIMARY KEY (session_id, repository, path)
    );
", "
    -- How many of its finished turns the person has seen.
    ALTER TABLE sessions ADD COLUMN seen_turns INTEGER NOT NULL DEFAULT 0;
", "
    -- What agents ask of the person: a question, or to move their task.
    CREATE TABLE agent_requests (
        id TEXT PRIMARY KEY,
        session_id TEXT NOT NULL,
        kind INTEGER NOT NULL,
        text TEXT NOT NULL,
        item_id TEXT NOT NULL,
        status INTEGER NOT NULL,
        created_unix_ms INTEGER NOT NULL,
        -- Null while it waits in the inbox.
        answered_unix_ms INTEGER
    );
    CREATE INDEX open_agent_requests ON agent_requests (answered_unix_ms, created_unix_ms);
", "
    -- The recovery copy of each session on another machine, kept after its
    -- host stops listing it: a compressed SessionBackup without bundles,
    -- whose snapshots are in the mirrors under the state folder.
    CREATE TABLE recoveries (
        session_id TEXT PRIMARY KEY,
        host TEXT NOT NULL,
        name TEXT NOT NULL,
        turns INTEGER NOT NULL,
        backup BLOB NOT NULL,
        backed_up_unix_ms INTEGER NOT NULL
    );
    -- Hosts a session was moved away from, whose copy of it is left out.
    CREATE TABLE moved_sessions (
        session_id TEXT NOT NULL,
        host TEXT NOT NULL,
        PRIMARY KEY (session_id, host)
    );
", "
    -- What the person did about each proposal an agent made; proposals
    -- themselves are in the sessions' histories.
    CREATE TABLE proposal_decisions (
        session_id TEXT NOT NULL,
        proposal_id TEXT NOT NULL,
        applied INTEGER NOT NULL,
        -- The item applying it made or changed; empty if declined.
        item_id TEXT NOT NULL,
        decided_unix_ms INTEGER NOT NULL,
        PRIMARY KEY (session_id, proposal_id)
    );
    -- Sessions that plan, in no copy of the code.
    ALTER TABLE sessions ADD COLUMN planning INTEGER NOT NULL DEFAULT 0;
    -- Know-how every session in a project reads, in order.
    CREATE TABLE project_skills (
        project_id TEXT NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
        position INTEGER NOT NULL,
        name TEXT NOT NULL,
        text TEXT NOT NULL,
        PRIMARY KEY (project_id, position)
    );
", "
    -- Where a repository added by its URL was cloned from; its path is then
    -- the Mac's clone of it.
    ALTER TABLE project_repositories ADD COLUMN url TEXT NOT NULL DEFAULT '';
", "
    -- What every session said and did, and the review comments and specs,
    -- to search. A session's entries are added as its history grows;
    -- comments and specs are indexed again before each search.
    CREATE VIRTUAL TABLE search_entries USING fts5(
        text, kind UNINDEXED, session UNINDEXED, event UNINDEXED, turn UNINDEXED,
        time UNINDEXED, project UNINDEXED, item UNINDEXED, thread UNINDEXED,
        tokenize = 'porter unicode61'
    );
    -- How far each session is indexed, and where it is.
    CREATE TABLE search_sessions (
        id TEXT PRIMARY KEY,
        host TEXT NOT NULL,
        project TEXT NOT NULL,
        item TEXT NOT NULL,
        events INTEGER NOT NULL,
        turns INTEGER NOT NULL
    );
", "
    -- Each session's pull requests, one per repository, as last read.
    CREATE TABLE pull_requests (
        session_id TEXT NOT NULL,
        repository TEXT NOT NULL,
        github TEXT NOT NULL,
        number INTEGER NOT NULL,
        url TEXT NOT NULL,
        title TEXT NOT NULL,
        branch TEXT NOT NULL,
        base TEXT NOT NULL,
        state TEXT NOT NULL,
        review TEXT NOT NULL,
        checks TEXT NOT NULL,
        read_unix_ms INTEGER NOT NULL,
        PRIMARY KEY (session_id, repository)
    );
    -- Merged sessions whose worktrees and branches were removed.
    CREATE TABLE cleaned_sessions (
        session_id TEXT PRIMARY KEY,
        cleaned_unix_ms INTEGER NOT NULL
    );
    -- GitHub issues made tasks, as owner/name#12.
    CREATE TABLE imported_issues (
        project_id TEXT NOT NULL,
        issue TEXT NOT NULL,
        item_id TEXT NOT NULL,
        PRIMARY KEY (project_id, issue)
    );
    -- Review comments brought in from GitHub, by their id there.
    ALTER TABLE review_comments ADD COLUMN github_id INTEGER;
    ALTER TABLE review_comments ADD COLUMN github_login TEXT NOT NULL DEFAULT '';
    CREATE UNIQUE INDEX review_comments_by_github ON review_comments (github_id);
", "
    -- Spec changes an idle session is told of with its next message.
    CREATE TABLE spec_notes (
        session_id TEXT NOT NULL,
        item_id TEXT NOT NULL,
        note TEXT NOT NULL,
        PRIMARY KEY (session_id, item_id)
    );
", "
    -- Who made each board change: where from, as a ChangeSurface, and the
    -- session whose proposal or request it applied.
    ALTER TABLE board_changes ADD COLUMN surface INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE board_changes ADD COLUMN session_id TEXT NOT NULL DEFAULT '';
", "
    -- A merged session whose host still has to archive it and delete its
    -- branches; tried again until it does.
    ALTER TABLE cleaned_sessions ADD COLUMN host_pending INTEGER NOT NULL DEFAULT 0;
", "
    -- The archived sessions a deleted item had, one id a line, put back on
    -- it when the deletion is reverted.
    ALTER TABLE board_changes ADD COLUMN sessions TEXT NOT NULL DEFAULT '';
", "
    -- What agents outside Humata proposed through `humata mcp`, as a
    -- Proposal, until the person decides it; decisions are kept with the
    -- sessions' in proposal_decisions, with no session.
    CREATE TABLE outside_proposals (
        id TEXT PRIMARY KEY,
        project_id TEXT NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
        proposal BLOB NOT NULL
    );
", "
    -- Admin actions asked for by clients other than the app: the Approval,
    -- and the request to run once it's approved.
    CREATE TABLE approvals (
        id TEXT PRIMARY KEY,
        state INTEGER NOT NULL,
        grant_id TEXT NOT NULL,
        approval BLOB NOT NULL,
        request BLOB NOT NULL
    );
    -- What the person let run without asking, as a Grant.
    CREATE TABLE grants (
        id TEXT PRIMARY KEY,
        grant_message BLOB NOT NULL
    );
    -- The approval or grant a change by another client ran under.
    ALTER TABLE board_changes ADD COLUMN approval_id TEXT NOT NULL DEFAULT '';
    ALTER TABLE board_changes ADD COLUMN grant_id TEXT NOT NULL DEFAULT '';
"];

/// What reverting a board change did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reverted {
    /// The item as it is now; None if it was removed.
    pub item: Option<Item>,
    /// The fields the change set that were kept, as they changed again
    /// since.
    pub kept: Vec<pb::ItemField>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Project {
    pub id: String,
    pub name: String,
    /// The repositories on this Mac its sessions work in; at least one.
    pub repositories: Vec<Repository>,
    /// Where each repository added by its URL was cloned from, by its name.
    pub urls: BTreeMap<String, String>,
    pub instructions: String,
    pub created_unix_ms: i64,
    /// Read by every session in it, after its instructions.
    pub skills: Vec<Skill>,
}

/// Where a session is, as the daemon records it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placement {
    /// Empty for this Mac.
    pub host: String,
    pub project: Option<String>,
    /// The task or subtask it works on; None for its project itself.
    pub item: Option<String>,
    /// How many of its finished turns the person has seen.
    pub seen_turns: u32,
}

pub struct Store {
    path: PathBuf,
    connection: Mutex<Connection>,
}

impl Store {
    /// Opens the store at `path`, creating it and bringing its schema up to
    /// date.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|source| StoreError::CreateDirectory {
                path: dir.to_owned(),
                source,
            })?;
        }
        let opened = |source| StoreError::Open {
            path: path.to_owned(),
            source,
        };
        let mut connection = Connection::open(path).map_err(opened)?;
        connection
            .execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")
            .map_err(opened)?;
        migrate(&mut connection, path)?;
        Ok(Self {
            path: path.to_owned(),
            connection: Mutex::new(connection),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Connection> {
        self.connection
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn failed(&self) -> impl Fn(rusqlite::Error) -> StoreError + '_ {
        |source| StoreError::Query {
            path: self.path.clone(),
            source,
        }
    }

    pub fn add_project(
        &self,
        name: String,
        repositories: Vec<Repository>,
        urls: BTreeMap<String, String>,
        instructions: String,
        skills: Vec<Skill>,
    ) -> Result<Project, StoreError> {
        let project = Project {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            repositories,
            urls,
            instructions,
            created_unix_ms: now_unix_ms(),
            skills,
        };
        let mut connection = self.lock();
        let added = (|| {
            let transaction = connection.transaction()?;
            // The first repository also goes in the column the first schema
            // had, which is still there.
            let first = project
                .repositories
                .first()
                .map(|repository| repository.path.to_string_lossy().into_owned())
                .unwrap_or_default();
            transaction.execute(
                "INSERT INTO projects (id, name, repository, instructions, created_unix_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    project.id,
                    project.name,
                    first,
                    project.instructions,
                    project.created_unix_ms,
                ],
            )?;
            insert_repositories(
                &transaction,
                &project.id,
                &project.repositories,
                &project.urls,
            )?;
            planning::insert_skills(&transaction, &project.id, &project.skills)?;
            transaction.commit()
        })();
        added.map_err(self.failed())?;
        Ok(project)
    }

    /// Every project, oldest first.
    pub fn projects(&self) -> Result<Vec<Project>, StoreError> {
        let connection = self.lock();
        let listed = (|| -> rusqlite::Result<Vec<Project>> {
            let mut statement = connection.prepare(
                "SELECT id, name, instructions, created_unix_ms FROM projects
                 ORDER BY created_unix_ms, id",
            )?;
            let projects: Vec<Project> = statement
                .query_map([], project)?
                .collect::<Result<_, _>>()?;
            projects
                .into_iter()
                .map(|project| with_repositories(&connection, project))
                .collect()
        })();
        listed.map_err(self.failed())
    }

    pub fn project(&self, id: &str) -> Result<Project, StoreError> {
        let connection = self.lock();
        let found = connection
            .query_row(
                "SELECT id, name, instructions, created_unix_ms FROM projects WHERE id = ?1",
                [id],
                project,
            )
            .optional()
            .and_then(|found| {
                found
                    .map(|project| with_repositories(&connection, project))
                    .transpose()
            })
            .map_err(self.failed())?;
        found.ok_or_else(|| StoreError::UnknownProject { id: id.to_owned() })
    }

    /// Removes `id`, its repositories and its board. Its sessions are kept,
    /// in no project and on no item.
    pub fn delete_project(&self, id: &str) -> Result<(), StoreError> {
        self.project(id)?;
        let mut connection = self.lock();
        let transaction = connection.transaction().map_err(self.failed())?;
        let deleted = (|| {
            transaction.execute(
                "UPDATE sessions SET project_id = NULL, item_id = NULL WHERE project_id = ?1",
                [id],
            )?;
            transaction.execute("DELETE FROM board_changes WHERE project_id = ?1", [id])?;
            transaction.execute("DELETE FROM items WHERE project_id = ?1", [id])?;
            transaction.execute(
                "DELETE FROM project_repositories WHERE project_id = ?1",
                [id],
            )?;
            transaction.execute("DELETE FROM projects WHERE id = ?1", [id])?;
            Ok(())
        })();
        deleted
            .and_then(|()| transaction.commit())
            .map_err(self.failed())
    }

    /// Replaces `project`'s repositories, for sessions started after.
    pub fn set_repositories(
        &self,
        project: &str,
        repositories: &[Repository],
        urls: &BTreeMap<String, String>,
    ) -> Result<(), StoreError> {
        self.project(project)?;
        let mut connection = self.lock();
        let set = (|| {
            let transaction = connection.transaction()?;
            transaction.execute(
                "DELETE FROM project_repositories WHERE project_id = ?1",
                [project],
            )?;
            insert_repositories(&transaction, project, repositories, urls)?;
            // The first also goes in the column the first schema had.
            let first = repositories
                .first()
                .map(|repository| repository.path.to_string_lossy().into_owned())
                .unwrap_or_default();
            transaction.execute(
                "UPDATE projects SET repository = ?2 WHERE id = ?1",
                params![project, first],
            )?;
            transaction.commit()
        })();
        set.map_err(self.failed())
    }

    /// Records that `session` runs on `host`, and, if given, belongs to
    /// `project` and works on `item`. A session's project and item, once
    /// set, are kept; [`Store::move_session`] changes its item.
    pub fn place(
        &self,
        session: &str,
        host: &str,
        project: Option<&str>,
        item: Option<&str>,
    ) -> Result<(), StoreError> {
        self.lock()
            .execute(
                "INSERT INTO sessions (id, project_id, host, item_id) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (id) DO UPDATE SET
                     host = excluded.host,
                     project_id = coalesce(sessions.project_id, excluded.project_id),
                     item_id = coalesce(sessions.item_id, excluded.item_id)
                 WHERE sessions.host IS NOT excluded.host
                     OR (sessions.project_id IS NULL AND excluded.project_id IS NOT NULL)
                     OR (sessions.item_id IS NULL AND excluded.item_id IS NOT NULL)",
                params![session, project, host, item],
            )
            .map_err(|source| match source {
                rusqlite::Error::SqliteFailure(failure, _)
                    if failure.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_FOREIGNKEY =>
                {
                    StoreError::UnknownProject {
                        id: project.unwrap_or_default().to_owned(),
                    }
                }
                source => self.failed()(source),
            })?;
        Ok(())
    }

    /// Where `session` is; None if the daemon has never seen it.
    pub fn placement(&self, session: &str) -> Result<Option<Placement>, StoreError> {
        self.lock()
            .query_row(
                "SELECT host, project_id, item_id, seen_turns FROM sessions WHERE id = ?1",
                [session],
                |row| {
                    Ok(Placement {
                        host: row.get(0)?,
                        project: row.get(1)?,
                        item: row.get(2)?,
                        seen_turns: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(self.failed())
    }

    /// Records that the person has seen `turns` of the session's finished
    /// turns, unless more were seen already.
    pub fn mark_seen(&self, session: &str, turns: u32) -> Result<(), StoreError> {
        self.lock()
            .execute(
                "UPDATE sessions SET seen_turns = max(seen_turns, ?2) WHERE id = ?1",
                params![session, turns],
            )
            .map(drop)
            .map_err(self.failed())
    }

    /// Forgets everything kept of `session`, which was deleted.
    pub fn forget_session(&self, session: &str) -> Result<(), StoreError> {
        const FORGET: &[&str] = &[
            "DELETE FROM review_comments WHERE thread_id IN
                (SELECT id FROM review_threads WHERE session_id = ?1)",
            "DELETE FROM review_threads WHERE session_id = ?1",
            "DELETE FROM reviews WHERE session_id = ?1",
            "DELETE FROM viewed_files WHERE session_id = ?1",
            "DELETE FROM agent_requests WHERE session_id = ?1",
            "DELETE FROM recoveries WHERE session_id = ?1",
            "DELETE FROM moved_sessions WHERE session_id = ?1",
            "DELETE FROM proposal_decisions WHERE session_id = ?1",
            "DELETE FROM search_entries WHERE session = ?1",
            "DELETE FROM search_sessions WHERE id = ?1",
            "DELETE FROM pull_requests WHERE session_id = ?1",
            "DELETE FROM cleaned_sessions WHERE session_id = ?1",
            "DELETE FROM spec_notes WHERE session_id = ?1",
            "DELETE FROM remote_sessions WHERE id = ?1",
            "DELETE FROM sessions WHERE id = ?1",
        ];
        let mut connection = self.lock();
        let transaction = connection.transaction().map_err(self.failed())?;
        let forgotten = FORGET
            .iter()
            .try_for_each(|sql| transaction.execute(sql, [session]).map(drop));
        forgotten
            .and_then(|()| transaction.commit())
            .map_err(self.failed())
    }

    /// Keeps `sessions` as what `host` has now, forgetting any it no longer
    /// lists.
    pub fn cache_sessions(
        &self,
        host: &str,
        sessions: &[pb::SessionInfo],
    ) -> Result<(), StoreError> {
        // Hosts are listed every two seconds and seldom change: write only
        // when they did.
        let mut listed: Vec<&pb::SessionInfo> = sessions.iter().collect();
        listed.sort_by(|a, b| a.id.cmp(&b.id));
        let cached = self.cached_sessions(host)?;
        if cached.len() == listed.len() && cached.iter().zip(&listed).all(|(a, b)| a == *b) {
            return Ok(());
        }
        let mut connection = self.lock();
        let transaction = connection.transaction().map_err(self.failed())?;
        let kept = (|| {
            transaction.execute(
                "CREATE TEMP TABLE IF NOT EXISTS listed (id TEXT PRIMARY KEY)",
                [],
            )?;
            transaction.execute("DELETE FROM temp.listed", [])?;
            for session in sessions {
                transaction.execute(
                    "INSERT INTO remote_sessions (id, host, info) VALUES (?1, ?2, ?3)
                     ON CONFLICT (id) DO UPDATE SET host = excluded.host, info = excluded.info",
                    params![session.id, host, session.encode_to_vec()],
                )?;
                transaction.execute(
                    "INSERT OR IGNORE INTO temp.listed (id) VALUES (?1)",
                    [&session.id],
                )?;
            }
            transaction.execute(
                "DELETE FROM remote_sessions WHERE host = ?1 AND id NOT IN (SELECT id FROM temp.listed)",
                [host],
            )?;
            Ok(())
        })();
        kept.and_then(|()| transaction.commit())
            .map_err(self.failed())
    }

    /// What `host` last listed.
    pub fn cached_sessions(&self, host: &str) -> Result<Vec<pb::SessionInfo>, StoreError> {
        let connection = self.lock();
        let mut statement = connection
            .prepare("SELECT info FROM remote_sessions WHERE host = ?1 ORDER BY id")
            .map_err(self.failed())?;
        let rows = statement
            .query_map([host], |row| row.get::<_, Vec<u8>>(0))
            .map_err(self.failed())?;
        let mut sessions = Vec::new();
        for row in rows {
            // A row this build can't read is left out, as if never cached.
            if let Ok(session) = pb::SessionInfo::decode(row.map_err(self.failed())?.as_slice()) {
                sessions.push(session);
            }
        }
        Ok(sessions)
    }

    /// Keeps `history` as `session`'s, if the session is cached.
    pub fn cache_history(&self, session: &str, history: &pb::History) -> Result<(), StoreError> {
        let compressed = ruzstd::encoding::compress_to_vec(
            history.encode_to_vec().as_slice(),
            ruzstd::encoding::CompressionLevel::Fastest,
        );
        self.lock()
            .execute(
                "UPDATE remote_sessions SET history = ?2 WHERE id = ?1",
                params![session, compressed],
            )
            .map(drop)
            .map_err(self.failed())
    }

    /// `session`'s history as last read, if it was.
    pub fn cached_history(&self, session: &str) -> Result<Option<pb::History>, StoreError> {
        let compressed: Option<Vec<u8>> = self
            .lock()
            .query_row(
                "SELECT history FROM remote_sessions WHERE id = ?1",
                [session],
                |row| row.get(0),
            )
            .optional()
            .map_err(self.failed())?
            .flatten();
        Ok(compressed.and_then(|compressed| {
            let mut bytes = Vec::new();
            let mut decoder =
                ruzstd::decoding::StreamingDecoder::new(compressed.as_slice()).ok()?;
            std::io::Read::read_to_end(&mut decoder, &mut bytes).ok()?;
            pb::History::decode(bytes.as_slice()).ok()
        }))
    }

    /// Forgets what was cached of `host`'s sessions.
    pub fn forget_host(&self, host: &str) -> Result<(), StoreError> {
        self.lock()
            .execute("DELETE FROM remote_sessions WHERE host = ?1", [host])
            .map(drop)
            .map_err(self.failed())
    }

    /// Moves `session` to `item` of its project, or to the project itself.
    pub fn move_session(&self, session: &str, item: Option<&str>) -> Result<(), StoreError> {
        let placement = self
            .placement(session)?
            .ok_or_else(|| StoreError::UnknownSession {
                id: session.to_owned(),
            })?;
        if let Some(item) = item {
            let found = self.item(item)?;
            if placement.project.as_deref() != Some(found.project_id.as_str()) {
                return Err(StoreError::ItemInAnotherProject {
                    title: found.fields.title,
                });
            }
        }
        self.lock()
            .execute(
                "UPDATE sessions SET item_id = ?2 WHERE id = ?1",
                params![session, item],
            )
            .map_err(self.failed())?;
        Ok(())
    }

    /// Adds a task, or a subtask under `fields.parent_id`, to `project`.
    pub fn add_item(
        &self,
        project: &str,
        fields: ItemFields,
        author: &Author,
    ) -> Result<Item, StoreError> {
        if fields.title.trim().is_empty() {
            return Err(StoreError::NoTitle);
        }
        self.project(project)?;
        if let Some(parent) = &fields.parent_id {
            let parent = self.item(parent)?;
            if parent.project_id != project {
                return Err(StoreError::ParentInAnotherProject {
                    title: parent.fields.title,
                });
            }
            if parent.fields.parent_id.is_some() {
                return Err(StoreError::NestedSubtask {
                    title: parent.fields.title,
                });
            }
        }
        let id = uuid::Uuid::new_v4().to_string();
        let now = now_unix_ms();
        let mut connection = self.lock();
        let added = (|| {
            let transaction = connection.transaction()?;
            insert_item(&transaction, &id, project, &fields, now)?;
            log(
                &transaction,
                project,
                &id,
                (None, Some(&fields)),
                None,
                author,
            )?;
            transaction.commit()
        })();
        added.map_err(self.failed())?;
        drop(connection);
        self.item(&id)
    }

    /// Changes the fields `patch` sets, recording the change if there is
    /// one.
    pub fn update_item(
        &self,
        id: &str,
        patch: ItemPatch,
        author: &Author,
    ) -> Result<Item, StoreError> {
        let item = self.item(id)?;
        let mut fields = item.fields.clone();
        if let Some(title) = patch.title {
            if title.trim().is_empty() {
                return Err(StoreError::NoTitle);
            }
            fields.title = title;
        }
        if let Some(spec) = patch.spec {
            fields.spec = spec;
        }
        if let Some(status) = patch.status {
            fields.status = status;
        }
        if let Some(priority) = patch.priority {
            fields.priority = priority;
        }
        if fields == item.fields {
            return Ok(item);
        }
        let mut connection = self.lock();
        let updated = (|| {
            let transaction = connection.transaction()?;
            update_fields(&transaction, id, &fields)?;
            log(
                &transaction,
                &item.project_id,
                id,
                (Some(&item.fields), Some(&fields)),
                None,
                author,
            )?;
            transaction.commit()
        })();
        updated.map_err(self.failed())?;
        drop(connection);
        self.item(id)
    }

    /// Removes item `id`, logged so reverting the change brings it back,
    /// with the `detach`ed sessions back on it. Refused while subtasks or
    /// other sessions are on it.
    pub fn delete_item(
        &self,
        id: &str,
        author: &Author,
        detach: &[String],
    ) -> Result<(), StoreError> {
        let item = self.item(id)?;
        let mut connection = self.lock();
        let transaction = connection.transaction().map_err(self.failed())?;
        for session in detach {
            transaction
                .execute(
                    "UPDATE sessions SET item_id = NULL WHERE id = ?1 AND item_id = ?2",
                    [session, id],
                )
                .map_err(self.failed())?;
        }
        let (subtasks, sessions): (i64, i64) = transaction
            .query_row(
                "SELECT
                    (SELECT count(*) FROM items WHERE parent_id = ?1),
                    (SELECT count(*) FROM sessions WHERE item_id = ?1)",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(self.failed())?;
        if subtasks > 0 || sessions > 0 {
            return Err(StoreError::ItemInUse {
                title: item.fields.title,
                subtasks,
                sessions,
            });
        }
        let deleted = (|| {
            transaction.execute("DELETE FROM items WHERE id = ?1", [id])?;
            log(
                &transaction,
                &item.project_id,
                id,
                (Some(&item.fields), None),
                None,
                author,
            )?;
            transaction.execute(
                "UPDATE board_changes SET sessions = ?1 WHERE id = last_insert_rowid()",
                [detach.join("\n")],
            )?;
            transaction.commit()
        })();
        deleted.map_err(self.failed())
    }

    pub fn item(&self, id: &str) -> Result<Item, StoreError> {
        let connection = self.lock();
        let found = connection
            .query_row(&format!("{ITEM_COLUMNS} WHERE id = ?1"), [id], item_row)
            .optional()
            .and_then(|found| {
                found
                    .map(|item| with_sessions(&connection, item))
                    .transpose()
            })
            .map_err(self.failed())?;
        found.ok_or_else(|| StoreError::UnknownItem { id: id.to_owned() })
    }

    /// `project`'s items: tasks by priority, most urgent first, then by when
    /// they were added, each followed by its subtasks in the same order.
    pub fn items(&self, project: &str) -> Result<Vec<Item>, StoreError> {
        self.project(project)?;
        let connection = self.lock();
        let listed = (|| -> rusqlite::Result<Vec<Item>> {
            let mut statement = connection.prepare(&format!(
                "{ITEM_COLUMNS} WHERE project_id = ?1 ORDER BY created_unix_ms, rowid"
            ))?;
            let items: Vec<Item> = statement
                .query_map([project], item_row)?
                .collect::<Result<_, _>>()?;
            items
                .into_iter()
                .map(|item| with_sessions(&connection, item))
                .collect()
        })();
        let mut items = listed.map_err(self.failed())?;
        // Stable, so equal priorities keep the order they were added in.
        items.sort_by(|a, b| b.fields.priority.cmp(&a.fields.priority));
        let (tasks, subtasks): (Vec<Item>, Vec<Item>) = items
            .into_iter()
            .partition(|item| item.fields.parent_id.is_none());
        let mut ordered = Vec::with_capacity(tasks.len() + subtasks.len());
        for task in tasks {
            let id = task.id.clone();
            ordered.push(task);
            ordered.extend(
                subtasks
                    .iter()
                    .filter(|subtask| subtask.fields.parent_id.as_deref() == Some(id.as_str()))
                    .cloned(),
            );
        }
        Ok(ordered)
    }

    /// `project`'s board changes, newest first.
    pub fn board_log(&self, project: &str) -> Result<Vec<Change>, StoreError> {
        self.project(project)?;
        let connection = self.lock();
        let listed = (|| -> rusqlite::Result<Vec<Change>> {
            let mut statement = connection.prepare(
                "SELECT id, item_id, time_unix_ms, before, after, reverts, surface, session_id,
                     approval_id, grant_id
                 FROM board_changes
                 WHERE project_id = ?1 ORDER BY id DESC",
            )?;
            statement
                .query_map([project], change_row)?
                .collect::<Result<_, _>>()
        })();
        listed.map_err(self.failed())
    }

    /// The item change `id` was made to, as it is now; None if it is gone.
    pub fn changed_item(&self, id: u64) -> Result<Option<Item>, StoreError> {
        self.lock()
            .query_row(
                &format!(
                    "{ITEM_COLUMNS} WHERE id = (SELECT item_id FROM board_changes WHERE id = ?1)"
                ),
                [i64::try_from(id).unwrap_or(i64::MAX)],
                item_row,
            )
            .optional()
            .map_err(self.failed())
    }

    /// Puts an item back as it was before change `id`, and records that as
    /// a change of its own. A field changed again since is left as it is,
    /// rather than overwrite that later change.
    pub fn revert(&self, id: u64, author: &Author) -> Result<Reverted, StoreError> {
        let mut connection = self.lock();
        let change = connection
            .query_row(
                "SELECT id, item_id, time_unix_ms, before, after, reverts, surface, session_id,
                     approval_id, grant_id
                 FROM board_changes
                 WHERE id = ?1",
                [i64::try_from(id).unwrap_or(i64::MAX)],
                change_row,
            )
            .optional()
            .map_err(self.failed())?
            .ok_or(StoreError::UnknownChange { id })?;
        let project: String = connection
            .query_row(
                "SELECT project_id FROM board_changes WHERE id = ?1",
                [i64::try_from(id).unwrap_or(i64::MAX)],
                |row| row.get(0),
            )
            .map_err(self.failed())?;
        let current = connection
            .query_row(
                &format!("{ITEM_COLUMNS} WHERE id = ?1"),
                [&change.item_id],
                item_row,
            )
            .optional()
            .map_err(self.failed())?;
        let now = now_unix_ms();
        let mut kept = Vec::new();
        let transaction = connection.transaction().map_err(self.failed())?;
        match (change.before, current) {
            // It added the item: remove it, unless something hangs off it.
            (None, Some(item)) => {
                let (subtasks, sessions): (i64, i64) = transaction
                    .query_row(
                        "SELECT
                            (SELECT count(*) FROM items WHERE parent_id = ?1),
                            (SELECT count(*) FROM sessions WHERE item_id = ?1)",
                        [&item.id],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .map_err(self.failed())?;
                if subtasks > 0 || sessions > 0 {
                    return Err(StoreError::ItemInUse {
                        title: item.fields.title,
                        subtasks,
                        sessions,
                    });
                }
                transaction
                    .execute("DELETE FROM items WHERE id = ?1", [&item.id])
                    .map_err(self.failed())?;
                log(
                    &transaction,
                    &project,
                    &item.id,
                    (Some(&item.fields), None),
                    Some(id),
                    author,
                )
                .map_err(self.failed())?;
            }
            // Already gone; nothing to undo.
            (None, None) => {}
            (Some(before), Some(item)) => {
                let after = change.after.as_ref().unwrap_or(&item.fields);
                let (back, skipped) = put_back(&before, after, &item.fields);
                kept = skipped;
                if back != item.fields {
                    update_fields(&transaction, &item.id, &back).map_err(self.failed())?;
                    log(
                        &transaction,
                        &project,
                        &item.id,
                        (Some(&item.fields), Some(&back)),
                        Some(id),
                        author,
                    )
                    .map_err(self.failed())?;
                }
            }
            // It removed the item: add it back, under its task if that is
            // still there.
            (Some(before), None) => {
                if let Some(parent) = &before.parent_id {
                    let there: bool = transaction
                        .query_row(
                            "SELECT EXISTS (SELECT 1 FROM items WHERE id = ?1)",
                            [parent],
                            |row| row.get(0),
                        )
                        .map_err(self.failed())?;
                    if !there {
                        return Err(StoreError::ParentRemoved {
                            title: before.title,
                        });
                    }
                }
                insert_item(&transaction, &change.item_id, &project, &before, now)
                    .map_err(self.failed())?;
                // Its sessions come back too, unless they went elsewhere.
                let sessions: String = transaction
                    .query_row(
                        "SELECT sessions FROM board_changes WHERE id = ?1",
                        [i64::try_from(id).unwrap_or(i64::MAX)],
                        |row| row.get(0),
                    )
                    .map_err(self.failed())?;
                for session in sessions.lines() {
                    transaction
                        .execute(
                            "UPDATE sessions SET item_id = ?2
                             WHERE id = ?1 AND item_id IS NULL AND project_id = ?3",
                            [session, &change.item_id, &project],
                        )
                        .map_err(self.failed())?;
                }
                log(
                    &transaction,
                    &project,
                    &change.item_id,
                    (None, Some(&before)),
                    Some(id),
                    author,
                )
                .map_err(self.failed())?;
            }
        }
        transaction.commit().map_err(self.failed())?;
        drop(connection);
        let item = match self.item(&change.item_id) {
            Ok(item) => Some(item),
            Err(StoreError::UnknownItem { .. }) => None,
            Err(error) => return Err(error),
        };
        Ok(Reverted { item, kept })
    }
}

const ITEM_COLUMNS: &str = "SELECT id, project_id, parent_id, title, spec, status, priority,
    created_unix_ms, updated_unix_ms FROM items";

/// An item's row, without its sessions.
fn item_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Item> {
    Ok(Item {
        id: row.get(0)?,
        project_id: row.get(1)?,
        fields: ItemFields {
            parent_id: row.get(2)?,
            title: row.get(3)?,
            spec: row.get(4)?,
            status: from_text(row, 5)?,
            priority: from_text(row, 6)?,
        },
        created_unix_ms: row.get(7)?,
        updated_unix_ms: row.get(8)?,
        session_ids: Vec::new(),
    })
}

fn with_sessions(connection: &Connection, item: Item) -> rusqlite::Result<Item> {
    let mut statement =
        connection.prepare_cached("SELECT id FROM sessions WHERE item_id = ?1 ORDER BY rowid")?;
    let session_ids = statement
        .query_map([&item.id], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    Ok(Item {
        session_ids,
        ..item
    })
}

/// What reverting an edit from `before` to `after` makes of `now`: each
/// field the edit changed goes back to `before` if it is still as the edit
/// left it, and is kept, and named, if it has changed again since.
fn put_back(
    before: &ItemFields,
    after: &ItemFields,
    now: &ItemFields,
) -> (ItemFields, Vec<pb::ItemField>) {
    let mut back = now.clone();
    let mut kept = Vec::new();
    macro_rules! field {
        ($name:ident, $field:expr) => {
            if before.$name != after.$name {
                if now.$name == after.$name {
                    back.$name.clone_from(&before.$name);
                } else {
                    kept.push($field);
                }
            }
        };
    }
    field!(title, pb::ItemField::Title);
    field!(spec, pb::ItemField::Spec);
    field!(status, pb::ItemField::Status);
    field!(priority, pb::ItemField::Priority);
    (back, kept)
}

fn change_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Change> {
    let fields = |index: usize| -> rusqlite::Result<Option<ItemFields>> {
        let text: Option<String> = row.get(index)?;
        text.map(|text| {
            serde_json::from_str(&text).map_err(|source| {
                rusqlite::Error::FromSqlConversionFailure(
                    index,
                    rusqlite::types::Type::Text,
                    Box::new(source),
                )
            })
        })
        .transpose()
    };
    let id: i64 = row.get(0)?;
    let reverts: Option<i64> = row.get(5)?;
    let surface: i32 = row.get(6)?;
    let session: String = row.get(7)?;
    let via = crate::board::Via {
        approval_id: row.get(8)?,
        grant_id: row.get(9)?,
    };
    Ok(Change {
        id: u64::try_from(id).unwrap_or_default(),
        item_id: row.get(1)?,
        time_unix_ms: row.get(2)?,
        before: fields(3)?,
        after: fields(4)?,
        reverts: reverts.and_then(|reverts| u64::try_from(reverts).ok()),
        author: Author {
            surface: pb::ChangeSurface::try_from(surface).unwrap_or_default(),
            session_id: Some(session).filter(|id| !id.is_empty()),
            via,
        },
    })
}

/// A status or priority, kept as its name.
fn from_text<T: serde::de::DeserializeOwned>(
    row: &rusqlite::Row<'_>,
    index: usize,
) -> rusqlite::Result<T> {
    let text: String = row.get(index)?;
    serde_json::from_value(serde_json::Value::String(text)).map_err(|source| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(source),
        )
    })
}

fn text<T: serde::Serialize>(value: T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(text)) => text,
        _ => String::new(),
    }
}

fn insert_item(
    transaction: &Transaction<'_>,
    id: &str,
    project: &str,
    fields: &ItemFields,
    now: i64,
) -> rusqlite::Result<()> {
    transaction.execute(
        "INSERT INTO items (id, project_id, parent_id, title, spec, status, priority,
             created_unix_ms, updated_unix_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
        params![
            id,
            project,
            fields.parent_id,
            fields.title,
            fields.spec,
            text::<Status>(fields.status),
            text::<Priority>(fields.priority),
            now,
        ],
    )?;
    Ok(())
}

fn update_fields(
    transaction: &Transaction<'_>,
    id: &str,
    fields: &ItemFields,
) -> rusqlite::Result<()> {
    transaction.execute(
        "UPDATE items SET title = ?2, spec = ?3, status = ?4, priority = ?5,
             updated_unix_ms = ?6
         WHERE id = ?1",
        params![
            id,
            fields.title,
            fields.spec,
            text::<Status>(fields.status),
            text::<Priority>(fields.priority),
            now_unix_ms(),
        ],
    )?;
    Ok(())
}

fn log(
    transaction: &Transaction<'_>,
    project: &str,
    item: &str,
    (before, after): (Option<&ItemFields>, Option<&ItemFields>),
    reverts: Option<u64>,
    author: &Author,
) -> rusqlite::Result<()> {
    let json = |fields: Option<&ItemFields>| {
        fields
            .map(serde_json::to_string)
            .transpose()
            .map_err(|source| rusqlite::Error::ToSqlConversionFailure(Box::new(source)))
    };
    transaction.execute(
        "INSERT INTO board_changes (project_id, item_id, time_unix_ms, before, after, reverts,
             surface, session_id, approval_id, grant_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            project,
            item,
            now_unix_ms(),
            json(before)?,
            json(after)?,
            reverts.and_then(|reverts| i64::try_from(reverts).ok()),
            i32::from(author.surface),
            author.session_id.as_deref().unwrap_or_default(),
            author.via.approval_id,
            author.via.grant_id,
        ],
    )?;
    Ok(())
}

fn insert_repositories(
    transaction: &Transaction<'_>,
    project: &str,
    repositories: &[Repository],
    urls: &BTreeMap<String, String>,
) -> rusqlite::Result<()> {
    for (position, repository) in repositories.iter().enumerate() {
        transaction.execute(
            "INSERT INTO project_repositories (project_id, position, name, path, url)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                project,
                i64::try_from(position).unwrap_or(i64::MAX),
                repository.name,
                repository.path.to_string_lossy(),
                urls.get(&repository.name)
                    .map(String::as_str)
                    .unwrap_or_default(),
            ],
        )?;
    }
    Ok(())
}

/// A project's row, without its repositories.
fn project(row: &rusqlite::Row<'_>) -> rusqlite::Result<Project> {
    Ok(Project {
        id: row.get(0)?,
        name: row.get(1)?,
        repositories: Vec::new(),
        urls: BTreeMap::new(),
        instructions: row.get(2)?,
        created_unix_ms: row.get(3)?,
        skills: Vec::new(),
    })
}

fn with_repositories(connection: &Connection, project: Project) -> rusqlite::Result<Project> {
    let mut statement = connection.prepare_cached(
        "SELECT name, path, url FROM project_repositories WHERE project_id = ?1 ORDER BY position",
    )?;
    let rows: Vec<(Repository, String)> = statement
        .query_map([&project.id], |row| {
            let repository = Repository {
                name: row.get(0)?,
                path: PathBuf::from(row.get::<_, String>(1)?),
            };
            Ok((repository, row.get(2)?))
        })?
        .collect::<Result<_, _>>()?;
    let urls = rows
        .iter()
        .filter(|(_, url)| !url.is_empty())
        .map(|(repository, url)| (repository.name.clone(), url.clone()))
        .collect();
    let repositories = rows.into_iter().map(|(repository, _)| repository).collect();
    let skills = planning::skills(connection, &project.id)?;
    Ok(Project {
        repositories,
        urls,
        skills,
        ..project
    })
}

fn migrate(connection: &mut Connection, path: &Path) -> Result<(), StoreError> {
    let failed = |source| StoreError::Migrate {
        path: path.to_owned(),
        source,
    };
    let found: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(failed)?;
    let mut version = 0_i64;
    for step in MIGRATIONS {
        version += 1;
        if version <= found {
            continue;
        }
        let transaction = connection.transaction().map_err(failed)?;
        transaction.execute_batch(step).map_err(failed)?;
        transaction
            .pragma_update(None, "user_version", version)
            .map_err(failed)?;
        transaction.commit().map_err(failed)?;
    }
    if found > version {
        return Err(StoreError::NewerSchema {
            path: path.to_owned(),
            found,
            known: version,
        });
    }
    Ok(())
}

pub(crate) fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repositories(path: &str) -> Vec<Repository> {
        vec![Repository {
            name: "code".into(),
            path: path.into(),
        }]
    }

    fn store(dir: &Path) -> Store {
        Store::open(&dir.join("humata.sqlite3")).expect("the store opens")
    }

    #[test]
    fn projects_survive_reopening_in_the_order_they_were_added() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let first = store(dir.path())
            .add_project(
                "one".into(),
                repositories("/code/one"),
                BTreeMap::new(),
                "be brief".into(),
                Vec::new(),
            )
            .expect("added");
        let second = store(dir.path())
            .add_project(
                "two".into(),
                repositories("/code/two"),
                BTreeMap::new(),
                String::new(),
                Vec::new(),
            )
            .expect("added");
        let reopened = store(dir.path());
        assert_eq!(
            reopened.projects().expect("listed"),
            vec![first.clone(), second]
        );
        assert_eq!(reopened.project(&first.id).expect("found"), first);
    }

    #[test]
    fn an_unknown_project_is_an_error() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let store = store(dir.path());
        assert!(matches!(
            store.project("nope"),
            Err(StoreError::UnknownProject { .. })
        ));
        assert!(matches!(
            store.place("s", "", Some("nope"), None),
            Err(StoreError::UnknownProject { .. })
        ));
    }

    #[test]
    fn a_session_keeps_its_project_when_placed_again() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let store = store(dir.path());
        let project = store
            .add_project(
                "one".into(),
                repositories("/code/one"),
                BTreeMap::new(),
                String::new(),
                Vec::new(),
            )
            .expect("added");
        assert_eq!(store.placement("s").expect("read"), None);
        store
            .place("s", "", Some(&project.id), None)
            .expect("placed");
        store.place("s", "build", None, None).expect("placed again");
        assert_eq!(
            store.placement("s").expect("read"),
            Some(Placement {
                host: "build".into(),
                project: Some(project.id),
                item: None,
                seen_turns: 0,
            })
        );
        store.mark_seen("s", 3).expect("marked");
        store.mark_seen("s", 2).expect("marked");
        assert_eq!(
            store
                .placement("s")
                .expect("read")
                .map(|placed| placed.seen_turns),
            Some(3)
        );
    }

    #[test]
    fn a_project_from_the_first_schema_keeps_its_repository() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let path = dir.path().join("humata.sqlite3");
        let connection = Connection::open(&path).expect("opens");
        connection
            .execute_batch(MIGRATIONS[0])
            .expect("the first schema");
        connection
            .pragma_update(None, "user_version", 1)
            .expect("set");
        connection
            .execute(
                "INSERT INTO projects VALUES ('p', 'one', '/code/app', '', 0)",
                [],
            )
            .expect("inserted");
        drop(connection);
        let project = Store::open(&path)
            .expect("opens")
            .project("p")
            .expect("found");
        assert_eq!(
            project.repositories,
            vec![Repository {
                name: "app".into(),
                path: "/code/app".into(),
            }]
        );
    }

    #[test]
    fn a_hosts_sessions_are_kept_with_their_history_until_it_stops_listing_them() {
        let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let store = store(dir.path());
        let session = |id: &str| pb::SessionInfo {
            id: id.to_owned(),
            name: id.to_owned(),
            ..pb::SessionInfo::default()
        };
        let history = pb::History {
            events: vec![pb::SessionEvent {
                id: "e1".to_owned(),
                ..pb::SessionEvent::default()
            }],
        };
        let cached = |store: &Store| -> Vec<String> {
            store
                .cached_sessions("build")
                .unwrap_or_else(|error| panic!("{error}"))
                .into_iter()
                .map(|session| session.id)
                .collect()
        };

        store
            .cache_sessions("build", &[session("a"), session("b")])
            .unwrap_or_else(|error| panic!("{error}"));
        store
            .cache_history("a", &history)
            .unwrap_or_else(|error| panic!("{error}"));
        let reopened = Store::open(&dir.path().join("humata.sqlite3"))
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(cached(&reopened), ["a", "b"]);
        assert_eq!(
            reopened.cached_history("a").ok().flatten(),
            Some(history.clone())
        );

        reopened
            .cache_sessions("build", &[session("a")])
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(cached(&reopened), ["a"]);
        assert_eq!(reopened.cached_history("a").ok().flatten(), Some(history));
        assert_eq!(reopened.cached_history("b").ok().flatten(), None);
    }

    #[test]
    fn a_host_listing_the_same_sessions_again_keeps_them_and_a_change_is_kept() {
        let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let store = store(dir.path());
        let session = |id: &str, name: &str| pb::SessionInfo {
            id: id.to_owned(),
            name: name.to_owned(),
            ..pb::SessionInfo::default()
        };
        let names = |store: &Store| -> Vec<String> {
            store
                .cached_sessions("build")
                .unwrap_or_else(|error| panic!("{error}"))
                .into_iter()
                .map(|session| session.name)
                .collect()
        };

        store
            .cache_sessions("build", &[session("a", "A"), session("b", "B")])
            .unwrap_or_else(|error| panic!("{error}"));
        store
            .cache_sessions("build", &[session("b", "B"), session("a", "A")])
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(names(&store), ["A", "B"]);

        store
            .cache_sessions("build", &[session("a", "A"), session("b", "Renamed")])
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(names(&store), ["A", "Renamed"]);
    }

    #[test]
    fn a_store_from_a_newer_humatad_is_refused() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let path = dir.path().join("humata.sqlite3");
        drop(Store::open(&path).expect("the store opens"));
        let connection = Connection::open(&path).expect("opens");
        connection
            .pragma_update(None, "user_version", 1_000)
            .expect("set");
        drop(connection);
        assert!(matches!(
            Store::open(&path),
            Err(StoreError::NewerSchema { .. })
        ));
    }
}
