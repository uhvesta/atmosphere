//! A session's records, carried beside it when it moves to another machine
//! (MV-13, MV-14, D195).
//!
//! The old machine copies the session's rows into a side SQLite file of the
//! same schema, as a deleted channel's are (`channel_snapshot.rs`); the file
//! travels with the session's folder and its hash is in the move manifest.
//! The new machine brings the rows in, never the database. Tabs stay behind:
//! their terminals are on the old machine, and the agent starts a new one
//! when it resumes.

use std::path::Path;

use rusqlite::Connection;
use rusqlite::types::Value;

use crate::channel_snapshot::{insert_rows, remove_side, rows_of, side_error};
use crate::store::check_id;
use crate::{Store, StoreError, schema};

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
}

/// Parents come before the tables that reference them.
const CARRIED: &[Carried] = &[
    Carried { name: "sessions", scope: "id = ?1", record: true, blob: None },
    Carried { name: "session_details", scope: "session = ?1", record: true, blob: None },
    Carried { name: "session_repos", scope: "session = ?1", record: true, blob: None },
    Carried { name: "agents", scope: "session = ?1", record: false, blob: Some(("launch", "launch_file")) },
    Carried { name: "interruptions", scope: "session = ?1", record: false, blob: None },
    Carried { name: "reviews", scope: "session = ?1", record: true, blob: Some(("state", "state_file")) },
    Carried { name: "views", scope: "session = ?1", record: true, blob: Some(("view", "view_file")) },
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
        check_id(session)?;
        let source = schema::open(side)?;
        let mut tables = Vec::new();
        for table in CARRIED.iter().filter(|table| take == Take::Everything || !table.record) {
            let (names, rows) = rows_of(&source, table.name, None)
                .map_err(side_error(side, "read the session's records"))?;
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
