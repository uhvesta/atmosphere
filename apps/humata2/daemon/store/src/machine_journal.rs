//! How far the host has applied each machine's journal (AWAY T7, D226): one
//! mark per machine, written in the same transaction as the Inbox request
//! the entry made, so a restart never applies an entry twice or loses one.

use rusqlite::{OptionalExtension, params};

use crate::store::{check_id, from_sql, to_sql};
use crate::{InboxRecord, Store, StoreError};

impl Store {
    /// The highest journal sequence of `machine` the host has applied or
    /// rejected, 0 before any.
    pub fn machine_journal_mark(&self, machine: &str) -> Result<u64, StoreError> {
        check_id(machine)?;
        self.inner()
            .read("read how far a machine's journal is applied", |connection| {
                connection
                    .query_row(
                        "SELECT applied_through FROM machine_journal WHERE machine = ?1",
                        [machine],
                        |row| row.get::<_, i64>(0),
                    )
                    .optional()
            })
            .map(|found| found.map_or(0, from_sql))
    }

    /// Records that `machine`'s journal is applied through `through`, and
    /// saves `request`, if any, in the same transaction: both happen or
    /// neither. The mark only moves up.
    pub fn apply_machine_journal(
        &self,
        machine: &str,
        through: u64,
        request: Option<&InboxRecord>,
    ) -> Result<(), StoreError> {
        check_id(machine)?;
        let session = match request {
            Some(request) => {
                check_id(&request.id)?;
                check_id(&request.session)?;
                Some(request.session.as_str())
            }
            None => None,
        };
        let inner = self.inner();
        let doing = "record a machine's journal entry";
        inner.write_for_session("machine_journal.apply", doing, session, |transaction| {
            if let Some(request) = request {
                crate::inbox::put_row(inner, transaction, request)?;
            }
            transaction
                .execute(
                    "INSERT INTO machine_journal (machine, applied_through) VALUES (?1, ?2)
                     ON CONFLICT (machine) DO UPDATE SET
                         applied_through = max(applied_through, excluded.applied_through)",
                    params![machine, to_sql(through)],
                )
                .map(drop)
                .map_err(inner.failed(doing))
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::SessionRecord;

    fn session(id: &str) -> SessionRecord {
        SessionRecord {
            id: id.to_owned(),
            channel: "ch_1".to_owned(),
            machine: "ma_1".to_owned(),
            title: "Fix the build".to_owned(),
            agent: "claude".to_owned(),
            folder: PathBuf::from("/work/web"),
            created_unix_ms: 1,
        }
    }

    fn request(id: &str, session: &str) -> InboxRecord {
        InboxRecord {
            id: id.to_owned(),
            session: session.to_owned(),
            entry: vec![1, 2, 3],
            created_unix_ms: 1,
            updated_unix_ms: 1,
        }
    }

    #[test]
    fn a_machine_has_no_mark_until_one_is_set_and_the_mark_survives_reopening() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path()).unwrap();
        assert_eq!(store.machine_journal_mark("ma_a").unwrap(), 0);
        store.apply_machine_journal("ma_a", 7, None).unwrap();
        drop(store);
        let store = Store::open(directory.path()).unwrap();
        assert_eq!(store.machine_journal_mark("ma_a").unwrap(), 7);
        assert_eq!(store.machine_journal_mark("ma_b").unwrap(), 0);
    }

    #[test]
    fn the_mark_only_moves_up() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path()).unwrap();
        store.apply_machine_journal("ma_a", 9, None).unwrap();
        store.apply_machine_journal("ma_a", 4, None).unwrap();
        assert_eq!(store.machine_journal_mark("ma_a").unwrap(), 9);
    }

    #[test]
    fn the_request_and_the_mark_are_saved_together() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path()).unwrap();
        store.put_session(&session("se_a")).unwrap();
        store
            .apply_machine_journal("ma_a", 3, Some(&request("rq_1", "se_a")))
            .unwrap();
        assert_eq!(store.machine_journal_mark("ma_a").unwrap(), 3);
        let kept = store.inbox_requests().unwrap();
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].id, "rq_1");
    }

    #[test]
    fn a_request_that_cannot_be_saved_leaves_the_mark_where_it_was() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path()).unwrap();
        store.apply_machine_journal("ma_a", 2, None).unwrap();
        // Its session isn't saved, so the request can't be.
        let failed = store.apply_machine_journal("ma_a", 5, Some(&request("rq_1", "se_missing")));
        assert!(failed.is_err());
        assert_eq!(store.machine_journal_mark("ma_a").unwrap(), 2);
        assert!(store.inbox_requests().unwrap().is_empty());
    }
}
