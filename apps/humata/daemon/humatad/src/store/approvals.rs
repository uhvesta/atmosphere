//! Admin actions asked for by clients other than the app, until the person
//! decides them, and the grants that let them run without asking.

use humata_wire::pb;
use prost::Message as _;
use rusqlite::{OptionalExtension, params};

use super::{Store, StoreError};

/// How many decided approvals are kept for the app to list.
const KEPT: i64 = 200;

impl Store {
    /// Keeps `approval`, asked for with `request`.
    pub fn add_approval(
        &self,
        approval: &pb::Approval,
        request: &pb::Request,
    ) -> Result<(), StoreError> {
        let connection = self.lock();
        connection
            .execute(
                "INSERT INTO approvals (id, state, grant_id, approval, request)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    approval.id,
                    approval.state,
                    approval.grant_id,
                    approval.encode_to_vec(),
                    request.encode_to_vec()
                ],
            )
            .map_err(self.failed())?;
        // Old decided ones go; pending ones stay until decided.
        connection
            .execute(
                "DELETE FROM approvals WHERE state != ?1 AND rowid NOT IN
                     (SELECT rowid FROM approvals WHERE state != ?1 ORDER BY rowid DESC LIMIT ?2)",
                params![pb::ApprovalState::Pending as i32, KEPT],
            )
            .map_err(self.failed())?;
        Ok(())
    }

    /// Records how `approval` was decided, if it's still pending: false if
    /// it was decided already.
    pub fn decide_approval(&self, approval: &pb::Approval) -> Result<bool, StoreError> {
        let changed = self
            .lock()
            .execute(
                "UPDATE approvals SET state = ?2, grant_id = ?3, approval = ?4
                 WHERE id = ?1 AND state = ?5",
                params![
                    approval.id,
                    approval.state,
                    approval.grant_id,
                    approval.encode_to_vec(),
                    pb::ApprovalState::Pending as i32
                ],
            )
            .map_err(self.failed())?;
        Ok(changed > 0)
    }

    /// Records what approved `approval` answered once run.
    pub fn record_result(&self, approval: &pb::Approval) -> Result<(), StoreError> {
        self.lock()
            .execute(
                "UPDATE approvals SET approval = ?2 WHERE id = ?1",
                params![approval.id, approval.encode_to_vec()],
            )
            .map_err(self.failed())?;
        Ok(())
    }

    /// Approval `id` and the request it asks to run.
    pub fn approval(&self, id: &str) -> Result<Option<(pb::Approval, pb::Request)>, StoreError> {
        let row: Option<(Vec<u8>, Vec<u8>)> = self
            .lock()
            .query_row(
                "SELECT approval, request FROM approvals WHERE id = ?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(self.failed())?;
        // One this build can't read is as good as gone.
        Ok(row.and_then(|(approval, request)| {
            Some((
                pb::Approval::decode(approval.as_slice()).ok()?,
                pb::Request::decode(request.as_slice()).ok()?,
            ))
        }))
    }

    /// The approvals kept, newest first; only the pending ones, oldest
    /// first, if `pending`.
    pub fn approvals(&self, pending: bool) -> Result<Vec<pb::Approval>, StoreError> {
        let connection = self.lock();
        let listed = (|| -> rusqlite::Result<Vec<Vec<u8>>> {
            let sql = if pending {
                "SELECT approval FROM approvals WHERE state = ?1 ORDER BY rowid"
            } else {
                "SELECT approval FROM approvals WHERE ?1 = ?1 ORDER BY rowid DESC"
            };
            let mut statement = connection.prepare_cached(sql)?;
            statement
                .query_map([pb::ApprovalState::Pending as i32], |row| row.get(0))?
                .collect()
        })();
        Ok(listed
            .map_err(self.failed())?
            .iter()
            .filter_map(|bytes| pb::Approval::decode(bytes.as_slice()).ok())
            .collect())
    }

    pub fn add_grant(&self, grant: &pb::Grant) -> Result<(), StoreError> {
        self.lock()
            .execute(
                "INSERT INTO grants (id, grant_message) VALUES (?1, ?2)",
                params![grant.id, grant.encode_to_vec()],
            )
            .map_err(self.failed())?;
        Ok(())
    }

    /// Every grant, newest first, with how many actions ran under each.
    pub fn grants(&self) -> Result<Vec<pb::Grant>, StoreError> {
        let connection = self.lock();
        let listed = (|| -> rusqlite::Result<Vec<(Vec<u8>, u32)>> {
            let mut statement = connection.prepare_cached(
                "SELECT grant_message,
                     (SELECT count(*) FROM approvals WHERE approvals.grant_id = grants.id)
                 FROM grants ORDER BY rowid DESC",
            )?;
            statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect()
        })();
        Ok(listed
            .map_err(self.failed())?
            .iter()
            .filter_map(|(bytes, used)| {
                let mut grant = pb::Grant::decode(bytes.as_slice()).ok()?;
                grant.used = *used;
                Some(grant)
            })
            .collect())
    }

    /// Revokes grant `id`: false if there's none.
    pub fn revoke_grant(&self, id: &str) -> Result<bool, StoreError> {
        let Some(mut grant) = self.grants()?.into_iter().find(|grant| grant.id == id) else {
            return Ok(false);
        };
        grant.revoked = true;
        grant.used = 0;
        self.lock()
            .execute(
                "UPDATE grants SET grant_message = ?2 WHERE id = ?1",
                params![id, grant.encode_to_vec()],
            )
            .map_err(self.failed())?;
        Ok(true)
    }
}
