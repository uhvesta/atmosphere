//! A session's records, carried beside it when it moves to another machine
//! (MV-13, MV-14, D195).
//!
//! The old machine copies the session's rows into a side SQLite file of the
//! same schema, as a deleted channel's are (`channel_snapshot.rs`); the file
//! travels with the session's folder and its hash is in the move manifest.
//! The new machine brings the rows in, never the database. Tabs stay behind:
//! their terminals are on the old machine, and the agent starts a new one
//! when it resumes.

use std::ffi::OsStr;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use prost::Message;
use rusqlite::Connection;
use rusqlite::types::Value;

use crate::channel_snapshot::{insert_rows, remove_side, rows_of, side_error};
use crate::store::check_id;
use crate::{Store, StoreError, blob, proto, schema};

/// A table whose rows belong to a session.
struct Carried {
    name: &'static str,
    /// Picks the session's rows; `?1` is its id.
    scope: &'static str,
    /// Whether the table is the machine's record of the session itself
    /// (the host keeps its own, so a move onto the host leaves these).
    record: bool,
    /// `(inline column, file column)` of a blob that may live in a file of
    /// the state folder; the side file keeps it inline.
    blob: Option<(&'static str, &'static str)>,
    /// Columns that hold a path as bytes, which a moved session's import
    /// rebases (MV-16, D336).
    paths: &'static [&'static str],
}

/// Parents come before the tables that reference them.
///
/// Every table with a row for the session is here (D336) except the ones that
/// aren't the session's to move: `last_copies` (the host's answers to a
/// machine's reads, by request hash) and the Migrate items of
/// `inbox_requests`, which are about a machine, not a session. A review
/// session's binding (`review_sessions`, `review_session_prs`) moves with it
/// (PR-21), so the machine it lands on knows its clones as a review's; the
/// marks and history of the pull requests' reviewed commits are keyed by
/// the pull request, not the session, and stay with the host. The rest of `inbox_requests` and the AW-9 summary in
/// `away_left_out` move with it, so what the agent asked and what the host
/// left out of its replayed writes are still there where it runs.
///
/// A machine that already holds the session's record (`Take::AgentOnly`, the
/// host) takes only what is the agent's own: its launch, interruptions, the
/// conversations it left and its fresh seed. Everything the host keeps about
/// the session itself (Desk rows, fork origin, archive, comments, Inbox,
/// AW-9 summary) is the host's, and stays.
const CARRIED: &[Carried] = &[
    Carried { name: "sessions", scope: "id = ?1", record: true, blob: None, paths: &["folder"] },
    Carried { name: "session_details", scope: "session = ?1", record: true, blob: None, paths: &[] },
    Carried { name: "session_repos", scope: "session = ?1", record: true, blob: None, paths: &["path"] },
    Carried {
        name: "agents",
        scope: "session = ?1",
        record: false,
        blob: Some(("launch", "launch_file")),
        paths: &[],
    },
    Carried { name: "interruptions", scope: "session = ?1", record: false, blob: None, paths: &[] },
    Carried { name: "review_sessions", scope: "session = ?1", record: true, blob: None, paths: &[] },
    Carried { name: "review_session_prs", scope: "session = ?1", record: true, blob: None, paths: &[] },
    Carried { name: "reviews", scope: "session = ?1", record: true, blob: Some(("state", "state_file")), paths: &[] },
    Carried { name: "views", scope: "session = ?1", record: true, blob: Some(("view", "view_file")), paths: &[] },
    Carried { name: "desk_sessions", scope: "session_id = ?1", record: true, blob: None, paths: &[] },
    Carried { name: "channel_desk_sessions", scope: "session_id = ?1", record: true, blob: None, paths: &[] },
    Carried { name: "previous_conversations", scope: "session_id = ?1", record: false, blob: None, paths: &["path"] },
    Carried { name: "fresh_seeds", scope: "session_id = ?1", record: false, blob: None, paths: &[] },
    Carried { name: "fork_origins", scope: "session_id = ?1", record: true, blob: None, paths: &[] },
    Carried { name: "archives", scope: "session_id = ?1", record: true, blob: None, paths: &[] },
    Carried { name: "comments", scope: "session = ?1", record: true, blob: None, paths: &[] },
    Carried { name: "comment_deliveries", scope: "session = ?1", record: true, blob: None, paths: &[] },
    Carried { name: "comment_numbers", scope: "session = ?1", record: true, blob: None, paths: &[] },
    Carried { name: "doc_comment_records", scope: "session = ?1", record: true, blob: None, paths: &[] },
    Carried {
        name: "inbox_requests",
        scope: "session = ?1 AND migrate_thing IS NULL",
        record: true,
        blob: Some(("entry", "entry_file")),
        paths: &[],
    },
    Carried { name: "away_left_out", scope: "session = ?1", record: true, blob: None, paths: &[] },
];

/// How much of a snapshot a machine takes in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Take {
    /// Every row: the machine had no record of the session. A box that a
    /// session moves onto.
    Everything,
    /// The agent and its interruptions only: the machine already holds the
    /// session's record and its place in the tree. The host.
    AgentOnly,
}

impl Store {
    /// Copies the rows of `session` into a new side file at `side`,
    /// replacing one already there, and leaves the state as it was. A blob
    /// kept in a file of the state folder goes inline.
    pub fn export_session(&self, session: &str, side: &Path) -> Result<(), StoreError> {
        check_id(session)?;
        remove_side(side)?;
        let target = schema::open(side)?;
        let (database, blobs) = (self.inner().database.clone(), self.inner().blobs.clone());
        let copied = self.inner().read_with(|source| {
            let write = side_error(side, "copy the session's records");
            let transaction = target.unchecked_transaction().map_err(write)?;
            for table in CARRIED {
                let (names, mut rows) = rows_of(source, table.name, Some((table.scope, session)))
                    .map_err(|source| StoreError::Database {
                        path: database.clone(),
                        doing: "read the session's records",
                        source,
                    })?;
                if let Some((inline, file)) = table.blob {
                    let at = |wanted: &str| names.iter().position(|name| name == wanted);
                    if let (Some(inline), Some(file)) = (at(inline), at(file)) {
                        for row in &mut rows {
                            if let Value::Text(hash) = &row[file] {
                                let path = blobs.join(hash);
                                let bytes = std::fs::read(&path).map_err(|source| StoreError::Read { path, source })?;
                                row[inline] = Value::Blob(bytes);
                                row[file] = Value::Null;
                            }
                        }
                    }
                }
                insert_rows(&transaction, table.name, &names, rows)
                    .map_err(side_error(side, "copy the session's records"))?;
            }
            transaction.commit().map_err(side_error(side, "save the session's records"))
        });
        let finished = copied.and_then(|()| {
            // One file, so the folder holds everything it needs.
            target
                .execute_batch("PRAGMA journal_mode = DELETE")
                .map_err(side_error(side, "close the side file"))
        });
        drop(target);
        if finished.is_err() {
            let _ = remove_side(side);
        }
        finished
    }

    /// Brings the rows of `session` in the side file at `side` into the
    /// state, in one transaction, as `take` says. Rows of the session the
    /// state already holds in those tables are replaced. The side file is
    /// left where it is.
    pub fn import_session(&self, session: &str, side: &Path, take: Take) -> Result<(), StoreError> {
        self.import_with(session, side, take, &[], false)
    }

    fn import_with(
        &self,
        session: &str,
        side: &Path,
        take: Take,
        moves: &[(PathBuf, PathBuf)],
        moved: bool,
    ) -> Result<(), StoreError> {
        check_id(session)?;
        let source = schema::open(side)?;
        let mut tables = Vec::new();
        for table in CARRIED.iter().filter(|table| take == Take::Everything || !table.record) {
            let (names, mut rows) = rows_of(&source, table.name, None)
                .map_err(side_error(side, "read the session's records"))?;
            rebase_rows(table, &names, &mut rows, moves);
            if table.name == "agents" {
                self.rebase_launches(&names, &mut rows, moves)?;
                if moved {
                    stop_agents(&names, &mut rows);
                }
            }
            tables.push((table, names, rows));
        }
        drop(source);
        let doing = "bring a moved session's records in";
        self.inner().write_with("session.import", doing, |transaction| {
            let failed = self.inner().failed(doing);
            // A session's own row goes first, and takes its children with it.
            for (table, _, _) in tables.iter().rev() {
                delete_rows(transaction, table, session).map_err(&failed)?;
            }
            for (table, names, rows) in tables {
                insert_rows(transaction, table.name, &names, rows).map_err(&failed)?;
            }
            Ok(())
        })
        .and_then(|()| self.inner().sweep().map(drop))
    }

    /// Removes what a session's agent left on this machine: the agent, its
    /// interruptions and the session's tabs. The session's own record stays.
    pub fn remove_agent_and_tabs(&self, session: &str) -> Result<(), StoreError> {
        check_id(session)?;
        self.inner().write("agent.remove", "remove a session's agent", |transaction| {
            transaction.execute("DELETE FROM tabs WHERE session = ?1", [session])?;
            transaction.execute("DELETE FROM agents WHERE session = ?1", [session])?;
            Ok(())
        })?;
        self.inner().sweep().map(drop)
    }

    /// As [`Store::import_session`], for a session moved to another folder
    /// or machine (MV-13, MV-16, D336): every path the store keeps for the
    /// session (`sessions.folder`, `session_repos.path`, the agent launch's
    /// `folder` and `conversation_file`, `previous_conversations.path`) is
    /// moved from under the first `from` of `moves` it lies in, by whole
    /// path components, to under its `to`, in the same transaction. A path
    /// under no `from` is kept. Blobs the daemon encodes (archive
    /// summaries, comments, Inbox entries) are carried as they are; the
    /// launch record is decoded, rebased and encoded again.
    pub fn import_moved_session(
        &self,
        session: &str,
        side: &Path,
        take: Take,
        moves: &[(PathBuf, PathBuf)],
    ) -> Result<(), StoreError> {
        self.import_with(session, side, take, moves, true)
    }

    /// Rebases the folder and conversation file of each agent launch in
    /// `rows` (the `agents` rows of a side file, launch inline).
    fn rebase_launches(
        &self,
        names: &[String],
        rows: &mut [Vec<Value>],
        moves: &[(PathBuf, PathBuf)],
    ) -> Result<(), StoreError> {
        let Some(at) = names.iter().position(|name| name == "launch") else {
            return Ok(());
        };
        let inner = self.inner();
        for row in rows {
            let Value::Blob(bytes) = &row[at] else { continue };
            let what = || "an agent's launch record".to_owned();
            let (version, body) =
                blob::read(&inner.blobs, &inner.database, Some(bytes.clone()), None, blob::Type::AgentLaunch, what)?;
            let mut launch = proto::AgentLaunch::decode(body.as_slice()).map_err(|_| StoreError::Damaged {
                path: inner.database.clone(),
                what: what(),
                reason: "how it was launched is unreadable",
            })?;
            for path in [&mut launch.folder, &mut launch.conversation_file] {
                if let Some(moved) = rebase(path, moves) {
                    *path = moved;
                }
            }
            row[at] = Value::Blob(blob::encode(blob::Type::AgentLaunch, version, &launch.encode_to_vec())?);
        }
        Ok(())
    }
}

/// Saves a moved session's agent rows as stopped. Its holder runs on the
/// old machine, so no pid or holder in the row is this machine's: until the
/// new machine points the agent at a tab of its own, a restart here must not
/// treat the old machine's holder as one to resume or stop, and an undo must
/// not take the copy for the session's live one.
fn stop_agents(names: &[String], rows: &mut [Vec<Value>]) {
    let column = |name: &str| names.iter().position(|named| named == name);
    for row in rows {
        if let Some(cell) = column("state").and_then(|at| row.get_mut(at)) {
            *cell = Value::Text("stopped".to_owned());
        }
        for name in ["holder_pid", "exit_code", "failure"] {
            if let Some(cell) = column(name).and_then(|at| row.get_mut(at)) {
                *cell = Value::Null;
            }
        }
    }
}

/// `path` under the `to` of the first move whose `from` it lies in, by whole
/// path components.
fn rebase(path: &[u8], moves: &[(PathBuf, PathBuf)]) -> Option<Vec<u8>> {
    let path = Path::new(OsStr::from_bytes(path));
    moves.iter().find_map(|(from, to)| {
        let rest = path.strip_prefix(from).ok()?;
        let moved = if rest.as_os_str().is_empty() { to.clone() } else { to.join(rest) };
        Some(moved.into_os_string().into_vec())
    })
}

/// Rebases the path columns of `table`'s rows.
fn rebase_rows(table: &Carried, names: &[String], rows: &mut [Vec<Value>], moves: &[(PathBuf, PathBuf)]) {
    for column in table.paths {
        let Some(at) = names.iter().position(|name| name == column) else { continue };
        for row in rows.iter_mut() {
            if let Value::Blob(bytes) = &mut row[at]
                && let Some(moved) = rebase(bytes, moves)
            {
                *bytes = moved;
            }
        }
    }
}

fn delete_rows(connection: &Connection, table: &Carried, session: &str) -> rusqlite::Result<()> {
    connection
        .execute(&format!("DELETE FROM {} WHERE {}", table.name, table.scope), [session])
        .map(drop)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::{
        AgentLaunch, AgentRecord, AgentState, ChannelRecord, ChannelRepo, MainClone, SessionDetail, SessionRecord,
        SessionRepo, StartStep, TabKind, TabRecord,
    };

    fn filled(root: &Path, session: &str) -> Store {
        let store = Store::open(root.join("state")).unwrap();
        store
            .put_channel(&ChannelRecord {
                id: "ch_1".to_owned(),
                name: "Payments".to_owned(),
                folder: "payments".to_owned(),
                branch_prefix: None,
                default_agent: "claude".to_owned(),
                repos: vec![ChannelRepo { owner: "acme".to_owned(), name: "api".to_owned(), main: MainClone::Ready }],
                machines: vec!["ma_1".to_owned()],
                revision: 1,
                created_unix_ms: 1,
            })
            .unwrap();
        store
            .put_session(&SessionRecord {
                id: session.to_owned(),
                channel: "ch_1".to_owned(),
                machine: String::new(),
                title: "Fix".to_owned(),
                agent: "claude".to_owned(),
                folder: PathBuf::from("/old/payments/sessions/fix"),
                created_unix_ms: 1,
            })
            .unwrap();
        store
            .put_session_detail(&SessionDetail {
                session: session.to_owned(),
                repos: vec![SessionRepo {
                    owner: "acme".to_owned(),
                    name: "api".to_owned(),
                    branch: "fix".to_owned(),
                    path: PathBuf::from("/old/payments/sessions/fix/api"),
                }],
                model: None,
                first_message: None,
                start: StartStep::Done,
                operation: None,
                revision: 1,
            })
            .unwrap();
        let tab = TabRecord {
            id: "tb_1".to_owned(),
            session: Some(session.to_owned()),
            kind: TabKind::Agent,
            title: "claude".to_owned(),
            holder_socket: PathBuf::from("/old/h.sock"),
            holder_pid: 7,
            created_unix_ms: 1,
            holder_generation: 0,
        };
        let agent = AgentRecord {
            session: session.to_owned(),
            tab: tab.id.clone(),
            launch: AgentLaunch {
                agent: "claude".to_owned(),
                folder: PathBuf::from("/old/payments/sessions/fix"),
                conversation_id: "conv-1".to_owned(),
                ..AgentLaunch::default()
            },
            state: AgentState::Running { holder_pid: 7 },
            generation: 1,
            interruptions: Vec::new(),
            updated_unix_ms: 1,
        };
        store.put_agent_and_tab(&agent, &tab).unwrap();
        store
    }

    #[test]
    fn a_snapshot_brings_a_session_with_its_agent_to_a_store_that_never_had_it() {
        let dir = tempfile::tempdir().unwrap();
        let old = filled(&dir.path().join("old"), "se_1");
        let side = dir.path().join("records.sqlite");
        old.export_session("se_1", &side).unwrap();

        let new = Store::open(dir.path().join("new")).unwrap();
        new.put_channel(&old.channel("ch_1").unwrap().unwrap()).unwrap();
        new.import_session("se_1", &side, Take::Everything).unwrap();

        assert_eq!(new.session("se_1").unwrap().unwrap().title, "Fix");
        assert_eq!(new.session_detail("se_1").unwrap().unwrap().repos.len(), 1);
        assert_eq!(new.agent("se_1").unwrap().unwrap().launch.conversation_id, "conv-1");
        assert!(new.tabs().unwrap().is_empty(), "terminals stay behind");
        assert_eq!(old.agent("se_1").unwrap().unwrap().launch.conversation_id, "conv-1", "the old store is as it was");
    }

    #[test]
    fn a_snapshot_taken_agent_only_leaves_the_hosts_own_record_alone() {
        let dir = tempfile::tempdir().unwrap();
        let old = filled(&dir.path().join("old"), "se_1");
        let side = dir.path().join("records.sqlite");
        old.export_session("se_1", &side).unwrap();

        let host = Store::open(dir.path().join("host")).unwrap();
        host.put_channel(&old.channel("ch_1").unwrap().unwrap()).unwrap();
        let mut mine = old.session("se_1").unwrap().unwrap();
        mine.title = "The host's title".to_owned();
        host.put_session(&mine).unwrap();
        host.import_session("se_1", &side, Take::AgentOnly).unwrap();

        assert_eq!(host.session("se_1").unwrap().unwrap().title, "The host's title");
        assert!(host.agent("se_1").unwrap().is_some());
    }

    #[test]
    fn removing_the_agent_and_tabs_keeps_the_session() {
        let dir = tempfile::tempdir().unwrap();
        let store = filled(dir.path(), "se_1");

        store.remove_agent_and_tabs("se_1").unwrap();

        assert!(store.session("se_1").unwrap().is_some());
        assert!(store.agent("se_1").unwrap().is_none());
        assert!(store.tabs().unwrap().is_empty());
    }
}
