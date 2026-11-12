//! What agents ask of the person: questions, and moves of their tasks
//! waiting for approval. Each waits in the inbox until answered.

use humata_wire::pb;
use rusqlite::{OptionalExtension, Row, params};

use super::{Store, now_unix_ms};
use crate::error::StoreError;

const COLUMNS: &str = "SELECT id, session_id, kind, text, item_id, status, created_unix_ms
    FROM agent_requests";

fn request(row: &Row<'_>) -> rusqlite::Result<pb::AgentRequest> {
    Ok(pb::AgentRequest {
        id: row.get(0)?,
        session_id: row.get(1)?,
        kind: row.get(2)?,
        text: row.get(3)?,
        item_id: row.get(4)?,
        status: row.get(5)?,
        created_unix_ms: row.get(6)?,
    })
}

impl Store {
    /// Keeps `request` waiting, under a new id, and returns it.
    pub fn add_request(
        &self,
        mut request: pb::AgentRequest,
    ) -> Result<pb::AgentRequest, StoreError> {
        request.id = uuid::Uuid::new_v4().to_string();
        request.created_unix_ms = now_unix_ms();
        self.lock()
            .execute(
                "INSERT INTO agent_requests (id, session_id, kind, text, item_id, status,
                    created_unix_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    request.id,
                    request.session_id,
                    request.kind,
                    request.text,
                    request.item_id,
                    request.status,
                    request.created_unix_ms,
                ],
            )
            .map_err(self.failed())?;
        Ok(request)
    }

    /// Every request still waiting, oldest first.
    pub fn open_requests(&self) -> Result<Vec<pb::AgentRequest>, StoreError> {
        let connection = self.lock();
        let mut statement = connection
            .prepare(&format!(
                "{COLUMNS} WHERE answered_unix_ms IS NULL ORDER BY created_unix_ms, rowid"
            ))
            .map_err(self.failed())?;
        statement
            .query_map([], request)
            .and_then(Iterator::collect)
            .map_err(self.failed())
    }

    /// Whether `session` has a request that waits on the person.
    pub fn asking(&self, session: &str) -> Result<bool, StoreError> {
        self.lock()
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM agent_requests
                    WHERE session_id = ?1 AND answered_unix_ms IS NULL)",
                [session],
                |row| row.get(0),
            )
            .map_err(self.failed())
    }

    /// The request `id`, if it still waits.
    pub fn open_request(&self, id: &str) -> Result<pb::AgentRequest, StoreError> {
        self.lock()
            .query_row(
                &format!("{COLUMNS} WHERE id = ?1 AND answered_unix_ms IS NULL"),
                params![id],
                request,
            )
            .optional()
            .map_err(self.failed())?
            .ok_or_else(|| StoreError::UnknownRequest { id: id.to_owned() })
    }

    /// Takes the request out of the inbox.
    pub fn answer_request(&self, id: &str) -> Result<(), StoreError> {
        let answered = self
            .lock()
            .execute(
                "UPDATE agent_requests SET answered_unix_ms = ?2
                 WHERE id = ?1 AND answered_unix_ms IS NULL",
                params![id, now_unix_ms()],
            )
            .map_err(self.failed())?;
        if answered == 0 {
            return Err(StoreError::UnknownRequest { id: id.to_owned() });
        }
        Ok(())
    }

    /// Answers `session`'s open questions: the person wrote to it.
    pub fn answer_questions(&self, session: &str) -> Result<(), StoreError> {
        self.answer_where("session_id", session, pb::AgentRequestKind::Question)
    }

    /// Answers the open requests to move `item`: the person moved it.
    pub fn answer_moves(&self, item: &str) -> Result<(), StoreError> {
        self.answer_where("item_id", item, pb::AgentRequestKind::Move)
    }

    fn answer_where(
        &self,
        column: &str,
        value: &str,
        kind: pb::AgentRequestKind,
    ) -> Result<(), StoreError> {
        self.lock()
            .execute(
                &format!(
                    "UPDATE agent_requests SET answered_unix_ms = ?3
                     WHERE {column} = ?1 AND kind = ?2 AND answered_unix_ms IS NULL"
                ),
                params![value, i32::from(kind), now_unix_ms()],
            )
            .map_err(self.failed())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn question(session: &str, text: &str) -> pb::AgentRequest {
        pb::AgentRequest {
            session_id: session.to_owned(),
            kind: pb::AgentRequestKind::Question.into(),
            text: text.to_owned(),
            ..Default::default()
        }
    }

    #[test]
    fn a_request_waits_until_answered_once_and_survives_reopening() {
        let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let path = dir.path().join("humata.sqlite3");
        let (first, second) = {
            let store = Store::open(&path).unwrap_or_else(|error| panic!("{error}"));
            let first = store
                .add_request(question("s", "Which database?"))
                .unwrap_or_else(|error| panic!("{error}"));
            let second = store
                .add_request(question("s", "Tabs or spaces?"))
                .unwrap_or_else(|error| panic!("{error}"));
            (first, second)
        };
        let store = Store::open(&path).unwrap_or_else(|error| panic!("{error}"));
        let open = store
            .open_requests()
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(open, vec![first.clone(), second.clone()]);

        store
            .answer_request(&first.id)
            .unwrap_or_else(|error| panic!("{error}"));
        let open = store
            .open_requests()
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(open, vec![second]);
        assert!(matches!(
            store.answer_request(&first.id),
            Err(StoreError::UnknownRequest { .. })
        ));
        assert!(matches!(
            store.open_request(&first.id),
            Err(StoreError::UnknownRequest { .. })
        ));
    }
}
