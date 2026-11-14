//! What the daemon keeps for planning: the person's decisions on what
//! agents proposed, which sessions only plan, and each project's skills.
//! The sessions' proposals themselves are in their histories; outside
//! agents' are kept here.

use humata_wire::pb;
use prost::Message as _;
use rusqlite::{Connection, OptionalExtension, params};

use super::{Store, StoreError, now_unix_ms};

/// What the person did about a proposal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Applied, making or changing this item.
    Applied {
        item: String,
    },
    Declined,
}

/// A named piece of know-how every session in a project reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skill {
    pub name: String,
    pub text: String,
}

impl Store {
    /// Records the person's decision on `proposal`, once: a proposal already
    /// decided is [`StoreError::AlreadyDecided`].
    pub fn decide(
        &self,
        session: &str,
        proposal: &str,
        decision: &Decision,
    ) -> Result<(), StoreError> {
        let (applied, item) = match decision {
            Decision::Applied { item } => (true, item.as_str()),
            Decision::Declined => (false, ""),
        };
        let added = self
            .lock()
            .execute(
                "INSERT INTO proposal_decisions
                     (session_id, proposal_id, applied, item_id, decided_unix_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (session_id, proposal_id) DO NOTHING",
                params![session, proposal, applied, item, now_unix_ms()],
            )
            .map_err(self.failed())?;
        if added == 0 {
            return Err(StoreError::AlreadyDecided {
                proposal: proposal.to_owned(),
            });
        }
        Ok(())
    }

    /// Gives the fork `to` the decisions on `from`'s proposals, which its
    /// history carries too.
    pub fn copy_decisions(&self, from: &str, to: &str) -> Result<(), StoreError> {
        self.lock()
            .execute(
                "INSERT OR IGNORE INTO proposal_decisions
                     (session_id, proposal_id, applied, item_id, decided_unix_ms)
                 SELECT ?2, proposal_id, applied, item_id, decided_unix_ms
                 FROM proposal_decisions WHERE session_id = ?1",
                [from, to],
            )
            .map_err(self.failed())?;
        Ok(())
    }

    /// The person's decisions on `session`'s proposals, by proposal.
    pub fn decisions(&self, session: &str) -> Result<Vec<(String, Decision)>, StoreError> {
        let connection = self.lock();
        let listed = (|| -> rusqlite::Result<Vec<(String, Decision)>> {
            let mut statement = connection.prepare_cached(
                "SELECT proposal_id, applied, item_id FROM proposal_decisions
                 WHERE session_id = ?1",
            )?;
            statement
                .query_map([session], |row| {
                    let decision = if row.get::<_, bool>(1)? {
                        Decision::Applied { item: row.get(2)? }
                    } else {
                        Decision::Declined
                    };
                    Ok((row.get(0)?, decision))
                })?
                .collect()
        })();
        listed.map_err(self.failed())
    }

    /// Keeps what an outside agent proposed for its project's board.
    pub fn add_outside_proposal(&self, proposal: &pb::Proposal) -> Result<(), StoreError> {
        self.lock()
            .execute(
                "INSERT INTO outside_proposals (id, project_id, proposal) VALUES (?1, ?2, ?3)",
                params![proposal.id, proposal.project_id, proposal.encode_to_vec()],
            )
            .map_err(self.failed())?;
        Ok(())
    }

    /// What outside agents proposed for `project`, oldest first, as
    /// proposed: their decisions are in [`Store::decisions`] with no session.
    pub fn outside_proposals(&self, project: &str) -> Result<Vec<pb::Proposal>, StoreError> {
        let connection = self.lock();
        let listed = (|| -> rusqlite::Result<Vec<Vec<u8>>> {
            let mut statement = connection.prepare_cached(
                "SELECT proposal FROM outside_proposals WHERE project_id = ?1 ORDER BY rowid",
            )?;
            statement.query_map([project], |row| row.get(0))?.collect()
        })();
        let listed = listed.map_err(self.failed())?;
        // One this build can't read is left out.
        Ok(listed
            .iter()
            .filter_map(|bytes| pb::Proposal::decode(bytes.as_slice()).ok())
            .collect())
    }

    /// The outside agent's proposal `id`, as proposed.
    pub fn outside_proposal(&self, id: &str) -> Result<Option<pb::Proposal>, StoreError> {
        let bytes: Option<Vec<u8>> = self
            .lock()
            .query_row(
                "SELECT proposal FROM outside_proposals WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .optional()
            .map_err(self.failed())?;
        Ok(bytes.and_then(|bytes| pb::Proposal::decode(bytes.as_slice()).ok()))
    }

    /// Marks `session` as one that plans, in no copy of the code.
    pub fn set_planning(&self, session: &str) -> Result<(), StoreError> {
        self.lock()
            .execute("UPDATE sessions SET planning = 1 WHERE id = ?1", [session])
            .map_err(self.failed())?;
        Ok(())
    }

    /// Sets whether Plan With an Agent starts with a copy of the code in
    /// `project`, kept with the daemon's settings.
    pub fn set_plan_with_code(&self, project: &str, with_code: bool) -> Result<(), StoreError> {
        self.project_exists(project)?;
        self.set_setting(
            &plan_with_code_key(project),
            if with_code { "1" } else { "0" },
        )
    }

    pub fn planning(&self, session: &str) -> Result<bool, StoreError> {
        let found = self
            .lock()
            .query_row(
                "SELECT planning FROM sessions WHERE id = ?1",
                [session],
                |row| row.get(0),
            )
            .optional()
            .map_err(self.failed())?;
        Ok(found.unwrap_or(false))
    }

    /// The sessions that read `item`'s spec, with their hosts: those on it
    /// and on its subtasks, leaving out planning sessions.
    pub fn sessions_on(&self, item: &str) -> Result<Vec<(String, String)>, StoreError> {
        let connection = self.lock();
        let listed = (|| -> rusqlite::Result<Vec<(String, String)>> {
            let mut statement = connection.prepare_cached(
                "SELECT sessions.id, sessions.host FROM sessions
                 LEFT JOIN items ON items.id = sessions.item_id
                 WHERE sessions.planning = 0
                     AND (sessions.item_id = ?1 OR items.parent_id = ?1)
                 ORDER BY sessions.id",
            )?;
            statement
                .query_map([item], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect()
        })();
        listed.map_err(self.failed())
    }

    /// Keeps `note` about `item`'s spec for `session`'s next message, in
    /// place of an older one about the same item.
    pub fn note_spec(&self, session: &str, item: &str, note: &str) -> Result<(), StoreError> {
        self.lock()
            .execute(
                "INSERT OR REPLACE INTO spec_notes (session_id, item_id, note) VALUES (?1, ?2, ?3)",
                [session, item, note],
            )
            .map(|_| ())
            .map_err(self.failed())
    }

    /// The notes kept for `session`'s next message, with their items.
    pub fn spec_notes(&self, session: &str) -> Result<Vec<(String, String)>, StoreError> {
        let connection = self.lock();
        let read = (|| -> rusqlite::Result<Vec<(String, String)>> {
            connection
                .prepare_cached(
                    "SELECT item_id, note FROM spec_notes WHERE session_id = ?1 ORDER BY item_id",
                )?
                .query_map([session], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect()
        })();
        read.map_err(self.failed())
    }

    /// Forgets `notes` once a message carried them; a note that changed
    /// since they were read is kept for the next message.
    pub fn clear_spec_notes(
        &self,
        session: &str,
        notes: &[(String, String)],
    ) -> Result<(), StoreError> {
        let connection = self.lock();
        let cleared = (|| -> rusqlite::Result<()> {
            let mut statement = connection.prepare_cached(
                "DELETE FROM spec_notes WHERE session_id = ?1 AND item_id = ?2 AND note = ?3",
            )?;
            for (item, note) in notes {
                statement.execute([session, item, note])?;
            }
            Ok(())
        })();
        cleared.map_err(self.failed())
    }

    /// Replaces `project`'s skills.
    pub fn set_skills(&self, project: &str, skills: &[Skill]) -> Result<(), StoreError> {
        let mut connection = self.lock();
        let set = (|| {
            let transaction = connection.transaction()?;
            transaction.execute(
                "DELETE FROM project_skills WHERE project_id = ?1",
                [project],
            )?;
            insert_skills(&transaction, project, skills)?;
            transaction.commit()
        })();
        set.map_err(self.failed())
    }

    /// Changes `project`'s name or instructions, where given.
    pub fn update_project(
        &self,
        project: &str,
        name: Option<&str>,
        instructions: Option<&str>,
    ) -> Result<(), StoreError> {
        let changed = self
            .lock()
            .execute(
                "UPDATE projects SET name = coalesce(?2, name),
                     instructions = coalesce(?3, instructions)
                 WHERE id = ?1",
                params![project, name, instructions],
            )
            .map_err(self.failed())?;
        if changed == 0 {
            return Err(StoreError::UnknownProject {
                id: project.to_owned(),
            });
        }
        Ok(())
    }
}

/// The setting that keeps whether `project` plans with the code.
fn plan_with_code_key(project: &str) -> String {
    format!("plan_with_code:{project}")
}

/// Whether Plan With an Agent starts with a copy of the code in `project`;
/// it doesn't until the person says so.
pub(super) fn plan_with_code(connection: &Connection, project: &str) -> rusqlite::Result<bool> {
    let set: Option<String> = connection
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            [plan_with_code_key(project)],
            |row| row.get(0),
        )
        .optional()?;
    Ok(set.as_deref() == Some("1"))
}

pub(super) fn skills(connection: &Connection, project: &str) -> rusqlite::Result<Vec<Skill>> {
    let mut statement = connection.prepare_cached(
        "SELECT name, text FROM project_skills WHERE project_id = ?1 ORDER BY position",
    )?;
    statement
        .query_map([project], |row| {
            Ok(Skill {
                name: row.get(0)?,
                text: row.get(1)?,
            })
        })?
        .collect()
}

pub(super) fn insert_skills(
    connection: &Connection,
    project: &str,
    skills: &[Skill],
) -> rusqlite::Result<()> {
    for (position, skill) in skills.iter().enumerate() {
        connection.execute(
            "INSERT INTO project_skills (project_id, position, name, text)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                project,
                i64::try_from(position).unwrap_or(i64::MAX),
                skill.name,
                skill.text,
            ],
        )?;
    }
    Ok(())
}
