//! How far the host has applied each machine's journal (AWAY T7, D226): one
//! mark per machine, written in the same transaction as the Inbox request
//! the entry made, so a restart never applies an entry twice or loses one.

use rusqlite::{OptionalExtension, params};

use crate::store::{check_id, from_sql, to_sql};
use crate::{BoardReplay, BoardReplayed, InboxRecord, LeftOut, Store, StoreError, ViewRecord};

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

    /// Forgets how far `machine`'s journal is applied. The host does this when
    /// the machine is forgotten or enrolled again: its journal starts over,
    /// so the old mark would skip its first entries.
    pub fn forget_machine_journal(&self, machine: &str) -> Result<(), StoreError> {
        check_id(machine)?;
        let inner = self.inner();
        let doing = "forget how far a machine's journal is applied";
        inner.write_for_session("machine_journal.forget", doing, None, |transaction| {
            transaction
                .execute("DELETE FROM machine_journal WHERE machine = ?1", params![machine])
                .map(drop)
                .map_err(inner.failed(doing))
        })
    }

    /// Replays one board write `machine` journaled, and records that its
    /// journal is applied through `through`, in the same transaction: both
    /// happen or neither. An entry at or below the mark was applied before,
    /// and nothing changes. An error (the entry names an item or a board
    /// that isn't there) leaves the mark where it was; the caller then
    /// moves it alone with [`Store::apply_machine_journal`].
    pub fn replay_board_entry(
        &self,
        machine: &str,
        through: u64,
        replay: &BoardReplay,
    ) -> Result<BoardReplayed, StoreError> {
        self.replay_board_entry_noting(machine, through, replay, None)
    }

    /// As [`Store::replay_board_entry`], adding `left_out`, if any, to its
    /// session's summary of what the host left out (AW-9, D267) in the same
    /// transaction as the mark: an entry seen twice counts once, and an error
    /// keeps neither.
    pub fn replay_board_entry_noting(
        &self,
        machine: &str,
        through: u64,
        replay: &BoardReplay,
        left_out: Option<&LeftOut>,
    ) -> Result<BoardReplayed, StoreError> {
        check_id(machine)?;
        let inner = self.inner();
        let doing = "replay a machine's journaled board write";
        inner.write_with("machine_journal.replay_board", doing, |transaction| {
            let mark: Option<i64> = transaction
                .query_row(
                    "SELECT applied_through FROM machine_journal WHERE machine = ?1",
                    [machine],
                    |row| row.get(0),
                )
                .optional()
                .map_err(inner.failed(doing))?;
            if through <= mark.map_or(0, from_sql) {
                return Ok(BoardReplayed::AlreadyApplied);
            }
            let replayed = crate::boards::replay_in(inner, transaction, replay)?;
            transaction
                .execute(
                    "INSERT INTO machine_journal (machine, applied_through) VALUES (?1, ?2)
                     ON CONFLICT (machine) DO UPDATE SET
                         applied_through = max(applied_through, excluded.applied_through)",
                    params![machine, to_sql(through)],
                )
                .map_err(inner.failed(doing))?;
            if let Some(left_out) = left_out {
                crate::away_left_out::note(inner, transaction, left_out)?;
            }
            Ok(replayed)
        })
    }

    /// Moves `machine`'s mark to `through` for an entry the host left out
    /// because its rules now refuse or ask about it, adding `left_out` to its
    /// session's summary in the same transaction. An entry at or below the
    /// mark was counted before and changes nothing.
    pub fn apply_machine_journal_left_out(
        &self,
        machine: &str,
        through: u64,
        left_out: &LeftOut,
    ) -> Result<(), StoreError> {
        check_id(machine)?;
        let inner = self.inner();
        let doing = "record a machine's journal entry";
        inner.write_with("machine_journal.apply_left_out", doing, |transaction| {
            let mark: Option<i64> = transaction
                .query_row(
                    "SELECT applied_through FROM machine_journal WHERE machine = ?1",
                    [machine],
                    |row| row.get(0),
                )
                .optional()
                .map_err(inner.failed(doing))?;
            if through <= mark.map_or(0, from_sql) {
                return Ok(());
            }
            crate::away_left_out::note(inner, transaction, left_out)?;
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

    /// Keeps a view `machine` journaled as offered, and records that its
    /// journal is applied through `through`, in the same transaction: both
    /// happen or neither. It says whether a row was inserted. An entry at or
    /// below the mark was applied before, and a view whose id the host
    /// already has is left as it is (the host owns its state); either way
    /// the mark moves up when `through` is past it. The session must be
    /// saved, else this is `NoSession` and nothing changes.
    pub fn apply_machine_journal_view(
        &self,
        machine: &str,
        through: u64,
        view: &ViewRecord,
    ) -> Result<bool, StoreError> {
        check_id(machine)?;
        check_id(&view.id)?;
        check_id(&view.session)?;
        let inner = self.inner();
        let doing = "record a machine's journaled view";
        inner.write_for_session("machine_journal.apply_view", doing, Some(&view.session), |transaction| {
            let mark: Option<i64> = transaction
                .query_row(
                    "SELECT applied_through FROM machine_journal WHERE machine = ?1",
                    [machine],
                    |row| row.get(0),
                )
                .optional()
                .map_err(inner.failed(doing))?;
            if through <= mark.map_or(0, from_sql) {
                return Ok(false);
            }
            let inserted = crate::views::insert_if_absent(inner, transaction, view)?;
            transaction
                .execute(
                    "INSERT INTO machine_journal (machine, applied_through) VALUES (?1, ?2)
                     ON CONFLICT (machine) DO UPDATE SET
                         applied_through = max(applied_through, excluded.applied_through)",
                    params![machine, to_sql(through)],
                )
                .map_err(inner.failed(doing))?;
            Ok(inserted)
        })
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

    #[test]
    fn a_forgotten_machine_has_no_mark_and_others_keep_theirs() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path()).unwrap();
        store.apply_machine_journal("ma_a", 7, None).unwrap();
        store.apply_machine_journal("ma_b", 3, None).unwrap();
        store.forget_machine_journal("ma_a").unwrap();
        assert_eq!(store.machine_journal_mark("ma_a").unwrap(), 0);
        assert_eq!(store.machine_journal_mark("ma_b").unwrap(), 3);
        // Forgetting one that has no mark is not an error, and a new mark
        // starts over from the entry that sets it.
        store.forget_machine_journal("ma_a").unwrap();
        store.apply_machine_journal("ma_a", 1, None).unwrap();
        assert_eq!(store.machine_journal_mark("ma_a").unwrap(), 1);
    }

    fn offered_view(id: &str, session: &str, state: u8) -> ViewRecord {
        ViewRecord {
            id: id.to_owned(),
            session: session.to_owned(),
            view: vec![state],
            keys: Vec::new(),
            prepared_unix_ms: 5,
            updated_unix_ms: 5,
        }
    }

    #[test]
    fn an_offered_view_and_the_mark_are_saved_together() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path()).unwrap();
        store.put_session(&session("se_a")).unwrap();
        let inserted = store
            .apply_machine_journal_view("ma_a", 3, &offered_view("vw_1", "se_a", 1))
            .unwrap();
        assert!(inserted);
        assert_eq!(store.machine_journal_mark("ma_a").unwrap(), 3);
        let kept = store.view("vw_1").unwrap().unwrap();
        assert_eq!((kept.session.as_str(), kept.view, kept.keys), ("se_a", vec![1], Vec::<String>::new()));
    }

    #[test]
    fn a_view_applied_again_or_already_known_keeps_its_row_and_state() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path()).unwrap();
        store.put_session(&session("se_a")).unwrap();
        store.apply_machine_journal_view("ma_a", 3, &offered_view("vw_1", "se_a", 1)).unwrap();
        // The user answers on the host.
        let mut answered = offered_view("vw_1", "se_a", 9);
        answered.keys = vec!["k".to_owned()];
        store.put_view(&answered).unwrap();

        let same_seq = store
            .apply_machine_journal_view("ma_a", 3, &offered_view("vw_1", "se_a", 1))
            .unwrap();
        let known_id = store
            .apply_machine_journal_view("ma_a", 4, &offered_view("vw_1", "se_a", 1))
            .unwrap();
        assert!(!same_seq && !known_id);
        assert_eq!(store.view("vw_1").unwrap().unwrap(), answered);
        assert_eq!(store.machine_journal_mark("ma_a").unwrap(), 4);
    }

    #[test]
    fn a_view_of_a_session_that_isnt_saved_is_no_session_and_leaves_the_mark() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path()).unwrap();
        store.apply_machine_journal("ma_a", 2, None).unwrap();
        let failed = store.apply_machine_journal_view("ma_a", 5, &offered_view("vw_1", "se_missing", 1));
        assert!(matches!(failed, Err(StoreError::NoSession { .. })), "{failed:?}");
        assert_eq!(store.machine_journal_mark("ma_a").unwrap(), 2);
        assert!(store.view("vw_1").unwrap().is_none());
    }

    use crate::{BoardItemChange, BoardKey, BoardWrite, NewBoardItem};

    fn board_item(title: &str) -> NewBoardItem {
        NewBoardItem {
            board: BoardKey::Channel("ch_1".to_owned()),
            channel: "ch_1".to_owned(),
            title: title.to_owned(),
            description: String::new(),
            column: 1,
            rank: "m".to_owned(),
            created_by: String::new(),
            at_unix_ms: 10,
            sessions: Vec::new(),
            depends_on: Vec::new(),
        }
    }

    fn board_change(key: &str) -> BoardItemChange {
        BoardItemChange {
            key: key.to_owned(),
            expected_revision: None,
            by: "se_a".to_owned(),
            at_unix_ms: 20,
            title: None,
            description: None,
            column: None,
            rank: None,
            sessions: None,
            depends_on: None,
            delete: false,
            cause: String::new(),
        }
    }

    /// A store with channel board item PAY-1 titled "a".
    fn with_item() -> (tempfile::TempDir, Store) {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path()).unwrap();
        store.claim_board_prefix("ch_1", "PAY").unwrap();
        store.create_board_item(&board_item("a")).unwrap();
        (directory, store)
    }

    fn host_changes(store: &Store, change: BoardItemChange) {
        let host = BoardItemChange {
            by: String::new(),
            ..change
        };
        assert!(matches!(store.change_board_item(&host).unwrap(), BoardWrite::Applied(_)));
    }

    fn lost(store: &Store, key: &str) -> Vec<crate::BoardHistoryEntry> {
        store
            .board_item_history(key, 0, 50)
            .unwrap()
            .into_iter()
            .filter(|entry| entry.lost)
            .collect()
    }

    #[test]
    fn a_replayed_edit_over_a_field_the_host_changed_wins_and_keeps_the_hosts_value_lost() {
        let (_directory, store) = with_item();
        host_changes(&store, BoardItemChange { title: Some("host".to_owned()), ..board_change("PAY-1") });
        let mut replayed = board_change("PAY-1");
        replayed.title = Some("agent".to_owned());
        replayed.expected_revision = Some(1);
        let done = store
            .replay_board_entry("ma_a", 1, &BoardReplay::Change(replayed))
            .unwrap();
        let BoardReplayed::Written(BoardWrite::Applied(item)) = done else {
            panic!("the replayed edit applies: {done:?}");
        };
        assert_eq!((item.title.as_str(), item.revision), ("agent", 3));
        let lost = lost(&store, "PAY-1");
        assert_eq!(lost.len(), 1);
        assert_eq!(
            (lost[0].field.as_str(), lost[0].before.as_str(), lost[0].after.as_str()),
            ("title", "agent", "host")
        );
        assert_eq!(lost[0].by, "", "the host's value is credited to the host's user");
        assert_eq!(store.machine_journal_mark("ma_a").unwrap(), 1);
    }

    #[test]
    fn a_replayed_edit_of_a_field_the_host_left_alone_has_no_lost_entry() {
        let (_directory, store) = with_item();
        host_changes(&store, BoardItemChange { description: Some("host".to_owned()), ..board_change("PAY-1") });
        let mut replayed = board_change("PAY-1");
        replayed.title = Some("agent".to_owned());
        replayed.expected_revision = Some(1);
        store
            .replay_board_entry("ma_a", 1, &BoardReplay::Change(replayed))
            .unwrap();
        let item = store.board_item("PAY-1").unwrap().unwrap();
        assert_eq!((item.title.as_str(), item.description.as_str()), ("agent", "host"));
        assert!(lost(&store, "PAY-1").is_empty());
    }

    #[test]
    fn a_replayed_edit_with_no_base_revision_is_the_later_write_and_loses_nothing_lost() {
        let (_directory, store) = with_item();
        host_changes(&store, BoardItemChange { title: Some("host".to_owned()), ..board_change("PAY-1") });
        let mut replayed = board_change("PAY-1");
        replayed.title = Some("agent".to_owned());
        store
            .replay_board_entry("ma_a", 1, &BoardReplay::Change(replayed))
            .unwrap();
        assert_eq!(store.board_item("PAY-1").unwrap().unwrap().title, "agent");
        assert!(lost(&store, "PAY-1").is_empty());
    }

    #[test]
    fn a_replayed_delete_made_against_an_older_revision_loses_and_is_kept_lost() {
        let (_directory, store) = with_item();
        host_changes(&store, BoardItemChange { title: Some("host".to_owned()), ..board_change("PAY-1") });
        let mut replayed = board_change("PAY-1");
        replayed.delete = true;
        replayed.expected_revision = Some(1);
        let done = store
            .replay_board_entry("ma_a", 1, &BoardReplay::Change(replayed))
            .unwrap();
        assert!(matches!(done, BoardReplayed::Written(BoardWrite::Stale { .. })));
        let item = store.board_item("PAY-1").unwrap().unwrap();
        assert!(!item.deleted);
        assert_eq!(lost(&store, "PAY-1").len(), 1);
        assert_eq!(store.machine_journal_mark("ma_a").unwrap(), 1);
    }

    #[test]
    fn a_replayed_edit_of_a_deleted_item_is_gone_kept_lost_and_does_not_revive_it() {
        let (_directory, store) = with_item();
        host_changes(&store, BoardItemChange { delete: true, ..board_change("PAY-1") });
        let mut replayed = board_change("PAY-1");
        replayed.title = Some("agent".to_owned());
        let done = store
            .replay_board_entry("ma_a", 1, &BoardReplay::Change(replayed))
            .unwrap();
        assert!(matches!(done, BoardReplayed::Written(BoardWrite::Gone { .. })));
        let item = store.board_item("PAY-1").unwrap().unwrap();
        assert!(item.deleted);
        assert_eq!(item.title, "a");
        assert_eq!(lost(&store, "PAY-1").len(), 1);
    }

    #[test]
    fn the_same_machine_and_sequence_twice_changes_nothing() {
        let (_directory, store) = with_item();
        let mut first = board_change("PAY-1");
        first.title = Some("one".to_owned());
        let mut second = board_change("PAY-1");
        second.title = Some("two".to_owned());
        store.replay_board_entry("ma_a", 4, &BoardReplay::Change(first)).unwrap();
        let again = store
            .replay_board_entry("ma_a", 4, &BoardReplay::Change(second.clone()))
            .unwrap();
        assert_eq!(again, BoardReplayed::AlreadyApplied);
        let earlier = store.replay_board_entry("ma_a", 3, &BoardReplay::Change(second)).unwrap();
        assert_eq!(earlier, BoardReplayed::AlreadyApplied);
        let item = store.board_item("PAY-1").unwrap().unwrap();
        assert_eq!((item.title.as_str(), item.revision), ("one", 2));
        assert_eq!(store.machine_journal_mark("ma_a").unwrap(), 4);
    }

    #[test]
    fn another_machines_sequence_is_counted_on_its_own() {
        let (_directory, store) = with_item();
        let mut change = board_change("PAY-1");
        change.title = Some("one".to_owned());
        store.replay_board_entry("ma_a", 4, &BoardReplay::Change(change.clone())).unwrap();
        change.title = Some("two".to_owned());
        let done = store.replay_board_entry("ma_b", 1, &BoardReplay::Change(change)).unwrap();
        assert!(matches!(done, BoardReplayed::Written(BoardWrite::Applied(_))));
        assert_eq!(store.board_item("PAY-1").unwrap().unwrap().title, "two");
    }

    #[test]
    fn a_refused_change_leaves_the_item_keeps_the_change_lost_and_moves_the_mark() {
        let (_directory, store) = with_item();
        let mut refused = board_change("PAY-1");
        refused.title = Some("agent".to_owned());
        let done = store
            .replay_board_entry("ma_a", 1, &BoardReplay::Refused(refused))
            .unwrap();
        assert_eq!(done, BoardReplayed::Refused);
        let item = store.board_item("PAY-1").unwrap().unwrap();
        assert_eq!((item.title.as_str(), item.revision), ("a", 1));
        let lost = lost(&store, "PAY-1");
        assert_eq!(lost.len(), 1);
        assert_eq!(
            (lost[0].by.as_str(), lost[0].before.as_str(), lost[0].after.as_str()),
            ("se_a", "a", "agent")
        );
        assert_eq!(store.machine_journal_mark("ma_a").unwrap(), 1);
    }

    #[test]
    fn a_replayed_create_makes_the_next_item_and_moves_the_mark() {
        let (_directory, store) = with_item();
        let done = store
            .replay_board_entry("ma_a", 1, &BoardReplay::Create(board_item("b")))
            .unwrap();
        let BoardReplayed::Created(item) = done else {
            panic!("the create makes an item: {done:?}");
        };
        assert_eq!((item.key.as_str(), item.title.as_str()), ("PAY-2", "b"));
        assert_eq!(store.machine_journal_mark("ma_a").unwrap(), 1);
    }

    #[test]
    fn an_entry_for_an_item_that_is_not_there_fails_and_leaves_the_mark() {
        let (_directory, store) = with_item();
        let mut change = board_change("PAY-9");
        change.title = Some("x".to_owned());
        let failed = store.replay_board_entry("ma_a", 5, &BoardReplay::Change(change));
        assert!(matches!(failed, Err(crate::StoreError::NoBoardItem { .. })));
        assert_eq!(store.machine_journal_mark("ma_a").unwrap(), 0);
    }

    fn left_out(item: &str) -> LeftOut {
        LeftOut {
            session: "se_a".to_owned(),
            link_up_unix_ms: 100,
            item: Some(item.to_owned()),
            not_made: None,
            refused: true,
            at_unix_ms: 105,
        }
    }

    fn refused_edit() -> BoardReplay {
        BoardReplay::Refused(BoardItemChange {
            title: Some("agent".to_owned()),
            ..board_change("PAY-1")
        })
    }

    #[test]
    fn the_summary_and_the_mark_land_together_and_survive_reopening() {
        let (directory, store) = with_item();
        store.put_session(&session("se_a")).unwrap();
        store
            .replay_board_entry_noting("ma_a", 1, &refused_edit(), Some(&left_out("PAY-1")))
            .unwrap();
        drop(store);
        let store = Store::open(directory.path()).unwrap();
        assert_eq!(store.machine_journal_mark("ma_a").unwrap(), 1);
        let kept = store.away_left_out("se_a").unwrap().unwrap();
        assert_eq!((kept.count, kept.items, kept.refused), (1, vec!["PAY-1".to_owned()], true));
    }

    #[test]
    fn an_entry_delivered_twice_is_counted_once() {
        let (_directory, store) = with_item();
        store.put_session(&session("se_a")).unwrap();
        for _ in 0..2 {
            store
                .replay_board_entry_noting("ma_a", 1, &refused_edit(), Some(&left_out("PAY-1")))
                .unwrap();
        }
        assert_eq!(store.away_left_out("se_a").unwrap().unwrap().count, 1);
    }

    #[test]
    fn an_entry_that_fails_keeps_neither_the_mark_nor_a_summary() {
        let (_directory, store) = with_item();
        store.put_session(&session("se_a")).unwrap();
        let mut change = board_change("PAY-9");
        change.title = Some("x".to_owned());
        let failed = store.replay_board_entry_noting(
            "ma_a",
            3,
            &BoardReplay::Refused(change),
            Some(&left_out("PAY-9")),
        );
        assert!(failed.is_err());
        assert_eq!(store.machine_journal_mark("ma_a").unwrap(), 0);
        assert_eq!(store.away_left_out("se_a").unwrap(), None);
    }

    #[test]
    fn a_left_out_create_moves_the_mark_and_notes_its_title() {
        let (_directory, store) = with_item();
        store.put_session(&session("se_a")).unwrap();
        let create = LeftOut {
            item: None,
            not_made: Some("Fix login".to_owned()),
            ..left_out("")
        };
        store.apply_machine_journal_left_out("ma_a", 2, &create).unwrap();
        store.apply_machine_journal_left_out("ma_a", 2, &create).unwrap();
        assert_eq!(store.machine_journal_mark("ma_a").unwrap(), 2);
        let kept = store.away_left_out("se_a").unwrap().unwrap();
        assert_eq!((kept.count, kept.not_made), (1, vec!["Fix login".to_owned()]));
    }
}
