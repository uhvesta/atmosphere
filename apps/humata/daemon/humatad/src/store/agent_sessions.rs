//! What agents may do to other sessions, a project at a time, and which
//! session's agent started each session.

use humata_wire::pb;
use rusqlite::{OptionalExtension, params};

use super::Store;
use crate::error::StoreError;

/// The most sessions one session's agent may have started, unless the
/// project says otherwise.
const MAX_CHILDREN: u32 = 8;

/// How deep sessions started by agents may nest, unless the project says
/// otherwise.
const MAX_DEPTH: u32 = 3;

/// What a project has until the person chooses: each start and message
/// asks, only an agent's own children are written to.
pub(super) fn defaults() -> pb::AgentSessions {
    pb::AgentSessions {
        mode: pb::AgentSessionsMode::Ask.into(),
        message_others: false,
        max_children: MAX_CHILDREN,
        max_depth: MAX_DEPTH,
    }
}

/// The mode a row holds; asking for none, or one this humatad doesn't know.
pub(super) fn mode(kept: Option<i32>) -> pb::AgentSessionsMode {
    match kept.map(pb::AgentSessionsMode::try_from) {
        Some(Ok(pb::AgentSessionsMode::Off)) => pb::AgentSessionsMode::Off,
        Some(Ok(pb::AgentSessionsMode::Allow)) => pb::AgentSessionsMode::Allow,
        _ => pb::AgentSessionsMode::Ask,
    }
}

/// `given` with the defaults where it leaves a limit at 0 or names no mode.
fn filled(given: &pb::AgentSessions) -> pb::AgentSessions {
    let or = |given: u32, default: u32| if given == 0 { default } else { given };
    pb::AgentSessions {
        mode: mode(Some(given.mode)).into(),
        message_others: given.message_others,
        max_children: or(given.max_children, MAX_CHILDREN),
        max_depth: or(given.max_depth, MAX_DEPTH),
    }
}

/// What `project`'s agents may do to other sessions.
pub(super) fn of_project(
    connection: &rusqlite::Connection,
    project: &str,
) -> rusqlite::Result<pb::AgentSessions> {
    let kept = connection
        .prepare_cached(
            "SELECT mode, message_others, max_children, max_depth FROM agent_sessions
             WHERE project_id = ?1",
        )?
        .query_row([project], |row| {
            Ok(pb::AgentSessions {
                mode: row.get(0)?,
                message_others: row.get(1)?,
                max_children: row.get(2)?,
                max_depth: row.get(3)?,
            })
        })
        .optional()?;
    Ok(kept.as_ref().map_or_else(defaults, filled))
}

impl Store {
    /// What the agents of `project`'s sessions may do to other sessions;
    /// for a session in no project, with None, the defaults.
    pub fn agent_sessions(&self, project: Option<&str>) -> Result<pb::AgentSessions, StoreError> {
        match project {
            Some(project) => of_project(&self.lock(), project).map_err(self.failed()),
            None => Ok(defaults()),
        }
    }

    /// Sets what `project`'s agents may do to other sessions; a limit left
    /// at 0 is the default.
    pub fn set_agent_sessions(
        &self,
        project: &str,
        set: &pb::AgentSessions,
    ) -> Result<(), StoreError> {
        self.project_exists(project)?;
        let set = filled(set);
        self.lock()
            .execute(
                "INSERT INTO agent_sessions
                     (project_id, mode, message_others, max_children, max_depth)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (project_id) DO UPDATE SET mode = excluded.mode,
                     message_others = excluded.message_others,
                     max_children = excluded.max_children, max_depth = excluded.max_depth",
                params![
                    project,
                    set.mode,
                    set.message_others,
                    set.max_children,
                    set.max_depth
                ],
            )
            .map(drop)
            .map_err(self.failed())
    }

    /// Records that `session`, placed already, was started by `starter`'s
    /// agent.
    pub fn set_started_by(&self, session: &str, starter: &str) -> Result<(), StoreError> {
        self.lock()
            .execute(
                "UPDATE sessions SET started_by = ?2 WHERE id = ?1",
                params![session, starter],
            )
            .map(drop)
            .map_err(self.failed())
    }

    /// The sessions `starter`'s agent started that the daemon still has.
    pub fn started_by(&self, starter: &str) -> Result<Vec<String>, StoreError> {
        let connection = self.lock();
        let read = (|| {
            let mut statement = connection
                .prepare_cached("SELECT id FROM sessions WHERE started_by = ?1 ORDER BY rowid")?;
            let rows = statement.query_map([starter], |row| row.get::<_, String>(0))?;
            rows.collect::<rusqlite::Result<Vec<String>>>()
        })();
        read.map_err(self.failed())
    }

    /// How many sessions' agents stand between `session` and one the
    /// person started: 0 for one the person started, 1 for one its agent
    /// started, and so on.
    pub fn started_depth(&self, session: &str) -> Result<u32, StoreError> {
        // A chain the store couldn't hold ends the walk, so a loop written
        // by a bug can't hang it.
        const DEEPEST: u32 = 64;
        let connection = self.lock();
        let read = (|| -> rusqlite::Result<u32> {
            let mut statement =
                connection.prepare_cached("SELECT started_by FROM sessions WHERE id = ?1")?;
            let mut depth = 0;
            let mut at = session.to_owned();
            while depth < DEEPEST {
                let starter = statement
                    .query_row([&at], |row| row.get::<_, Option<String>>(0))
                    .optional()?
                    .flatten();
                match starter {
                    Some(starter) => {
                        depth += 1;
                        at = starter;
                    }
                    None => break,
                }
            }
            Ok(depth)
        })();
        read.map_err(self.failed())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use humata_agent::Repository;

    use super::*;

    fn store_with_project(dir: &std::path::Path) -> (Store, String) {
        let store =
            Store::open(&dir.join("humata.sqlite3")).unwrap_or_else(|error| panic!("{error}"));
        let project = store
            .add_project(
                "one".into(),
                vec![Repository {
                    name: "one".into(),
                    path: PathBuf::from("/code/one"),
                }],
                BTreeMap::new(),
                String::new(),
                Vec::new(),
            )
            .unwrap_or_else(|error| panic!("{error}"));
        (store, project.id)
    }

    #[test]
    fn a_project_asks_within_limits_until_the_person_chooses_otherwise() {
        let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let (store, project) = store_with_project(dir.path());
        let fresh = store
            .project(&project)
            .unwrap_or_else(|error| panic!("{error}"))
            .agent_sessions;
        assert_eq!(fresh.mode(), pb::AgentSessionsMode::Ask);
        assert!(!fresh.message_others);
        assert!(fresh.max_children > 0 && fresh.max_depth > 0);

        store
            .set_agent_sessions(
                &project,
                &pb::AgentSessions {
                    mode: pb::AgentSessionsMode::Allow.into(),
                    message_others: true,
                    max_children: 2,
                    // Left to the default.
                    max_depth: 0,
                },
            )
            .unwrap_or_else(|error| panic!("{error}"));
        let set = store
            .agent_sessions(Some(&project))
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(set.mode(), pb::AgentSessionsMode::Allow);
        assert!(set.message_others);
        assert_eq!(set.max_children, 2);
        assert_eq!(set.max_depth, fresh.max_depth);

        // A session in no project goes by the defaults.
        assert_eq!(
            store
                .agent_sessions(None)
                .unwrap_or_else(|error| panic!("{error}")),
            fresh
        );
        assert!(matches!(
            store.set_agent_sessions("no-such-project", &fresh),
            Err(StoreError::UnknownProject { .. })
        ));
    }

    #[test]
    fn a_session_knows_who_started_it_and_how_deep_it_is() {
        let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let (store, project) = store_with_project(dir.path());
        for session in ["root", "child", "grandchild", "sibling"] {
            store
                .place(session, "", Some(&project), None)
                .unwrap_or_else(|error| panic!("{error}"));
        }
        for (session, starter) in [
            ("child", "root"),
            ("sibling", "root"),
            ("grandchild", "child"),
        ] {
            store
                .set_started_by(session, starter)
                .unwrap_or_else(|error| panic!("{error}"));
        }
        let depth = |session: &str| {
            store
                .started_depth(session)
                .unwrap_or_else(|error| panic!("{error}"))
        };
        assert_eq!(
            [depth("root"), depth("child"), depth("grandchild")],
            [0, 1, 2]
        );
        assert_eq!(
            store
                .started_by("root")
                .unwrap_or_else(|error| panic!("{error}")),
            ["child", "sibling"]
        );
        let known = store
            .known("grandchild")
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            known.and_then(|known| known.started_by).as_deref(),
            Some("child")
        );
    }
}
