//! Moving a Desk session into a channel (DK-15, DK-18, D305): the one write
//! that turns it from a Desk session into a channel session.

use rusqlite::params;

use crate::channels::write_session_detail;
use crate::store::path_bytes;
use crate::{SessionDetail, SessionRecord, Store, StoreError};

impl Store {
    /// Files the Desk session `session.id` under `session.channel` in one
    /// commit. It saves the record's channel and folder (its id, title,
    /// agent, machine and creation time stay as they are). It drops the
    /// session's Desk row and saves `detail` (its repos, each on its new
    /// branch and path) in place of any detail it had. A crash leaves the
    /// session wholly a Desk session or wholly a channel session, never
    /// between (DK-17).
    ///
    /// [`StoreError::NoSession`] when no session has that id, and then
    /// nothing changes.
    pub fn desk_to_channel(
        &self,
        session: &SessionRecord,
        detail: &SessionDetail,
    ) -> Result<(), StoreError> {
        let doing = "move a Desk session into a channel";
        let inner = self.inner();
        inner.write_for_session("desk.to_channel", doing, Some(&session.id), |transaction| {
            let failed = inner.failed(doing);
            let updated = transaction
                .execute(
                    "UPDATE sessions SET channel = ?2, folder = ?3 WHERE id = ?1",
                    params![session.id, session.channel, path_bytes(&session.folder)],
                )
                .map_err(&failed)?;
            if updated == 0 {
                return Err(StoreError::NoSession {
                    id: session.id.clone(),
                });
            }
            transaction
                .execute("DELETE FROM desk_sessions WHERE session_id = ?1", [&session.id])
                .map_err(&failed)?;
            write_session_detail(transaction, detail, &failed)
        })
    }
}
