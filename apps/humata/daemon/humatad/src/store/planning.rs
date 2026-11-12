//! What the daemon keeps for planning: the person's decisions on what
//! agents proposed, which sessions only plan, and each project's skills.
//! The proposals themselves are in the sessions' histories.

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

    /// Marks `session` as one that plans, in no copy of the code.
    pub fn set_planning(&self, session: &str) -> Result<(), StoreError> {
        self.lock()
            .execute("UPDATE sessions SET planning = 1 WHERE id = ?1", [session])
            .map_err(self.failed())?;
        Ok(())
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
