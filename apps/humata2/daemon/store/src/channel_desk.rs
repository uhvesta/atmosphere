//! What only a channel's Desk knows about one of its sessions (CD-1..CD-5,
//! D306): the channel it belongs to, the session it looks after, its six
//! management switches, when it was last used and closed, whether its
//! target is gone, and the trash item its closed folder is in. The session
//! itself is an ordinary record filed under the Desk's channel id, so it is
//! in no channel's Sessions tree; this row says which channel owns it.

use rusqlite::{OptionalExtension, params};

use crate::store::check_id;
use crate::{Store, StoreError};

const COLUMNS: &str = "session_id, channel, target_session, read_files, send_notes,
     type_into_terminal, start_and_fork, stop_and_archive, change_board_items,
     last_used_unix_ms, closed_unix_ms, target_gone, trash_item";

/// What only a channel's Desk knows about one of its sessions (CD-1..CD-5, D306).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelDeskRow {
    pub session_id: String,
    /// The channel whose Desk it is on.
    pub channel: String,
    /// The session it looks after; `None` for the whole channel.
    pub target_session: Option<String>,
    pub read_files: bool,
    pub send_notes: bool,
    pub type_into_terminal: bool,
    pub start_and_fork: bool,
    pub stop_and_archive: bool,
    pub change_board_items: bool,
    pub last_used_unix_ms: i64,
    /// Unset while open.
    pub closed_unix_ms: Option<i64>,
    /// CD-5: its target was deleted, or archived past the undo window, while
    /// its folder held Markdown files, so it stopped instead of closing.
    pub target_gone: bool,
    /// CD-4: the trash item its folder went to when it closed.
    pub trash_item: Option<String>,
}

impl Store {
    /// Saves `row` for its session, in place of any row it had before.
    pub fn put_channel_desk(&self, row: &ChannelDeskRow) -> Result<(), StoreError> {
        check_id(&row.session_id)?;
        let inner = self.inner();
        inner.write_for_session(
            "channel_desk.put",
            "save a channel Desk session's row",
            Some(&row.session_id),
            |transaction| {
                transaction
                    .execute(
                        "INSERT INTO channel_desk_sessions (
                             session_id, channel, target_session, read_files, send_notes,
                             type_into_terminal, start_and_fork, stop_and_archive,
                             change_board_items, last_used_unix_ms, closed_unix_ms,
                             target_gone, trash_item)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
                         ON CONFLICT (session_id) DO UPDATE SET
                             channel = excluded.channel,
                             target_session = excluded.target_session,
                             read_files = excluded.read_files,
                             send_notes = excluded.send_notes,
                             type_into_terminal = excluded.type_into_terminal,
                             start_and_fork = excluded.start_and_fork,
                             stop_and_archive = excluded.stop_and_archive,
                             change_board_items = excluded.change_board_items,
                             last_used_unix_ms = excluded.last_used_unix_ms,
                             closed_unix_ms = excluded.closed_unix_ms,
                             target_gone = excluded.target_gone,
                             trash_item = excluded.trash_item",
                        params![
                            row.session_id,
                            row.channel,
                            row.target_session,
                            row.read_files,
                            row.send_notes,
                            row.type_into_terminal,
                            row.start_and_fork,
                            row.stop_and_archive,
                            row.change_board_items,
                            row.last_used_unix_ms,
                            row.closed_unix_ms,
                            row.target_gone,
                            row.trash_item
                        ],
                    )
                    .map(drop)
                    .map_err(inner.failed("save a channel Desk session's row"))
            },
        )
    }

    /// The channel Desk row of `session`, if it is one.
    pub fn channel_desk(&self, session: &str) -> Result<Option<ChannelDeskRow>, StoreError> {
        check_id(session)?;
        self.inner().read("read a channel Desk session's row", |connection| {
            connection
                .query_row(
                    &format!("SELECT {COLUMNS} FROM channel_desk_sessions WHERE session_id = ?1"),
                    [session],
                    row_channel_desk,
                )
                .optional()
        })
    }

    /// Every channel Desk row, closed ones included, ordered by session id.
    pub fn channel_desks(&self) -> Result<Vec<ChannelDeskRow>, StoreError> {
        self.inner().read("read the channel Desk session rows", |connection| {
            let mut statement = connection.prepare(&format!(
                "SELECT {COLUMNS} FROM channel_desk_sessions ORDER BY session_id"
            ))?;
            statement.query_map([], row_channel_desk)?.collect()
        })
    }
}

fn row_channel_desk(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChannelDeskRow> {
    Ok(ChannelDeskRow {
        session_id: row.get(0)?,
        channel: row.get(1)?,
        target_session: row.get(2)?,
        read_files: row.get(3)?,
        send_notes: row.get(4)?,
        type_into_terminal: row.get(5)?,
        start_and_fork: row.get(6)?,
        stop_and_archive: row.get(7)?,
        change_board_items: row.get(8)?,
        last_used_unix_ms: row.get(9)?,
        closed_unix_ms: row.get(10)?,
        target_gone: row.get(11)?,
        trash_item: row.get(12)?,
    })
}

#[cfg(test)]
mod tests {
    use super::ChannelDeskRow;
    use crate::{SessionRecord, Store};

    fn row(id: &str) -> ChannelDeskRow {
        ChannelDeskRow {
            session_id: id.to_owned(),
            channel: "ch_payments".to_owned(),
            target_session: Some("se_target".to_owned()),
            read_files: true,
            send_notes: true,
            type_into_terminal: false,
            start_and_fork: false,
            stop_and_archive: true,
            change_board_items: false,
            last_used_unix_ms: 5,
            closed_unix_ms: None,
            target_gone: false,
            trash_item: None,
        }
    }

    fn record(id: &str) -> SessionRecord {
        SessionRecord {
            id: id.to_owned(),
            channel: "desk".to_owned(),
            machine: String::new(),
            title: id.to_owned(),
            agent: "zsh".to_owned(),
            folder: std::path::PathBuf::from("/h/ch/payments/desk").join(id),
            created_unix_ms: 0,
        }
    }

    #[test]
    fn a_saved_row_reads_back_and_a_second_save_replaces_it() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("store")).unwrap();
        store.put_session(&record("se_cd")).unwrap();
        store.put_channel_desk(&row("se_cd")).unwrap();
        assert_eq!(store.channel_desk("se_cd").unwrap(), Some(row("se_cd")));
        let closed = ChannelDeskRow {
            closed_unix_ms: Some(9),
            trash_item: Some("tr_1".to_owned()),
            ..row("se_cd")
        };
        store.put_channel_desk(&closed).unwrap();
        assert_eq!(store.channel_desks().unwrap(), vec![closed]);
        assert_eq!(store.channel_desk("se_other").unwrap(), None);
    }

    #[test]
    fn removing_the_session_removes_its_row() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("store")).unwrap();
        store.put_session(&record("se_cd")).unwrap();
        store.put_channel_desk(&row("se_cd")).unwrap();
        store.remove_session("se_cd").unwrap();
        assert_eq!(store.channel_desk("se_cd").unwrap(), None);
    }
}
