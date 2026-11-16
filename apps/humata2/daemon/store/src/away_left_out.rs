//! What the host left out of a machine agent's replayed board writes (AW-9,
//! D267): per session, how many of its writes a tightened rule stopped, which
//! items keep them in their history as lost, the titles of the new items that
//! weren't made, and whether any is now refused (Never) rather than only
//! asked about (Ask me). The summary is written in the same transaction as
//! the machine's applied-through mark, so a delivery the host sees twice
//! counts once, and it lives until a later link-up that leaves out a write
//! replaces it.

use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::store::{check_id, from_sql, to_sql, Inner};
use crate::{Store, StoreError};

/// The most item keys, and the most titles, a summary names.
const KEPT: usize = 8;

/// A title is kept to this many characters.
const TITLE_CHARS: usize = 80;

/// One write the host's rules stopped at replay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeftOut {
    /// The session whose agent made it.
    pub session: String,
    /// When the link that carried it came up, in milliseconds: writes of one
    /// link-up add to each other, a later link-up's replace them.
    pub link_up_unix_ms: i64,
    /// The item whose history keeps the write as lost; `None` for a new item.
    pub item: Option<String>,
    /// The title of a new item that wasn't made.
    pub not_made: Option<String>,
    /// Whether the rule now says Never; `false` when it says Ask me.
    pub refused: bool,
    pub at_unix_ms: i64,
}

/// A session's summary of what the host left out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AwayLeftOut {
    pub link_up_unix_ms: i64,
    pub count: u32,
    /// The items whose history keeps a write as lost, the first few.
    pub items: Vec<String>,
    /// The titles of the new items that weren't made, the first few.
    pub not_made: Vec<String>,
    /// Whether any write is now refused; `false` when all now say Ask me.
    pub refused: bool,
    pub at_unix_ms: i64,
}

fn joined(names: &[String]) -> String {
    names.join("\n")
}

fn split(text: &str) -> Vec<String> {
    text.split('\n').filter(|part| !part.is_empty()).map(str::to_owned).collect()
}

/// A title as one short line.
fn tidy(title: &str) -> String {
    let one_line: String = title
        .trim()
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(TITLE_CHARS)
        .collect();
    one_line.trim().to_owned()
}

fn row_of(row: &rusqlite::Row<'_>) -> rusqlite::Result<AwayLeftOut> {
    Ok(AwayLeftOut {
        link_up_unix_ms: row.get(0)?,
        count: u32::try_from(from_sql(row.get(1)?)).unwrap_or(u32::MAX),
        items: split(&row.get::<_, String>(2)?),
        not_made: split(&row.get::<_, String>(3)?),
        refused: row.get::<_, i64>(4)? != 0,
        at_unix_ms: row.get(5)?,
    })
}

const COLUMNS: &str = "link_up_unix_ms, count, items, not_made, refused, at_unix_ms";

fn read(connection: &Connection, session: &str) -> rusqlite::Result<Option<AwayLeftOut>> {
    connection
        .query_row(
            &format!("SELECT {COLUMNS} FROM away_left_out WHERE session = ?1"),
            [session],
            row_of,
        )
        .optional()
}

/// Adds `left_out` to its session's summary inside `transaction`: onto the
/// summary of the same link-up, in place of one of an earlier link-up. A
/// session that is gone keeps nothing.
pub(crate) fn note(inner: &Inner, transaction: &Transaction<'_>, left_out: &LeftOut) -> Result<(), StoreError> {
    let failed = inner.failed("keep what the host left out of an agent's replayed writes");
    let known = transaction
        .query_row("SELECT 1 FROM sessions WHERE id = ?1", [&left_out.session], |_| Ok(()))
        .optional()
        .map_err(&failed)?;
    if known.is_none() {
        return Ok(());
    }
    let mut now = match read(transaction, &left_out.session).map_err(&failed)? {
        Some(before) if before.link_up_unix_ms == left_out.link_up_unix_ms => before,
        _ => AwayLeftOut {
            link_up_unix_ms: left_out.link_up_unix_ms,
            count: 0,
            items: Vec::new(),
            not_made: Vec::new(),
            refused: false,
            at_unix_ms: left_out.at_unix_ms,
        },
    };
    now.count = now.count.saturating_add(1);
    if let Some(item) = &left_out.item
        && now.items.len() < KEPT
        && !now.items.contains(item)
    {
        now.items.push(item.clone());
    }
    if let Some(title) = left_out.not_made.as_deref().map(tidy)
        && now.not_made.len() < KEPT
    {
        now.not_made.push(title);
    }
    now.refused |= left_out.refused;
    now.at_unix_ms = left_out.at_unix_ms;
    transaction
        .execute(
            "INSERT INTO away_left_out (session, link_up_unix_ms, count, items, not_made, refused, at_unix_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (session) DO UPDATE SET
                 link_up_unix_ms = excluded.link_up_unix_ms,
                 count = excluded.count,
                 items = excluded.items,
                 not_made = excluded.not_made,
                 refused = excluded.refused,
                 at_unix_ms = excluded.at_unix_ms",
            params![
                left_out.session,
                now.link_up_unix_ms,
                to_sql(u64::from(now.count)),
                joined(&now.items),
                joined(&now.not_made),
                i64::from(now.refused),
                now.at_unix_ms
            ],
        )
        .map(drop)
        .map_err(&failed)
}

impl Store {
    /// What the host left out of `session`'s agent's replayed writes, if
    /// anything.
    pub fn away_left_out(&self, session: &str) -> Result<Option<AwayLeftOut>, StoreError> {
        check_id(session)?;
        self.inner()
            .read("read what the host left out of an agent's writes", |connection| read(connection, session))
    }

    /// Every session's summary, by session id.
    pub fn away_left_outs(&self) -> Result<Vec<(String, AwayLeftOut)>, StoreError> {
        self.inner().read("list what the host left out of agents' writes", |connection| {
            let mut statement =
                connection.prepare(&format!("SELECT session, {COLUMNS} FROM away_left_out ORDER BY session"))?;
            statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        AwayLeftOut {
                            link_up_unix_ms: row.get(1)?,
                            count: u32::try_from(from_sql(row.get(2)?)).unwrap_or(u32::MAX),
                            items: split(&row.get::<_, String>(3)?),
                            not_made: split(&row.get::<_, String>(4)?),
                            refused: row.get::<_, i64>(5)? != 0,
                            at_unix_ms: row.get(6)?,
                        },
                    ))
                })?
                .collect()
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::SessionRecord;

    fn store_with_session() -> (tempfile::TempDir, Store) {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path()).unwrap();
        store
            .put_session(&SessionRecord {
                id: "se_a".to_owned(),
                channel: "ch_1".to_owned(),
                machine: "ma_1".to_owned(),
                title: "agent".to_owned(),
                agent: "claude".to_owned(),
                folder: PathBuf::from("/work"),
                created_unix_ms: 1,
            })
            .unwrap();
        (directory, store)
    }

    fn left(link_up: i64, item: Option<&str>, not_made: Option<&str>, refused: bool) -> LeftOut {
        LeftOut {
            session: "se_a".to_owned(),
            link_up_unix_ms: link_up,
            item: item.map(str::to_owned),
            not_made: not_made.map(str::to_owned),
            refused,
            at_unix_ms: link_up + 5,
        }
    }

    fn note_alone(store: &Store, seq: u64, left_out: &LeftOut) {
        store.apply_machine_journal_left_out("ma_1", seq, left_out).unwrap();
    }

    #[test]
    fn a_session_with_nothing_left_out_has_no_summary() {
        let (_directory, store) = store_with_session();
        assert_eq!(store.away_left_out("se_a").unwrap(), None);
        assert!(store.away_left_outs().unwrap().is_empty());
    }

    #[test]
    fn writes_of_one_link_up_add_up_and_name_each_item_once() {
        let (_directory, store) = store_with_session();
        note_alone(&store, 1, &left(100, Some("PAY-1"), None, false));
        note_alone(&store, 2, &left(100, Some("PAY-1"), None, false));
        note_alone(&store, 3, &left(100, None, Some("Fix login"), true));
        let kept = store.away_left_out("se_a").unwrap().unwrap();
        assert_eq!(kept.count, 3);
        assert_eq!(kept.items, vec!["PAY-1".to_owned()]);
        assert_eq!(kept.not_made, vec!["Fix login".to_owned()]);
        assert!(kept.refused, "one refused write makes the summary refused");
    }

    #[test]
    fn a_later_link_up_replaces_the_summary() {
        let (_directory, store) = store_with_session();
        note_alone(&store, 1, &left(100, Some("PAY-1"), None, true));
        note_alone(&store, 2, &left(200, Some("PAY-2"), None, false));
        let kept = store.away_left_out("se_a").unwrap().unwrap();
        assert_eq!((kept.count, kept.items, kept.refused), (1, vec!["PAY-2".to_owned()], false));
    }

    #[test]
    fn a_delivery_seen_twice_counts_once_and_the_summary_survives_reopening() {
        let directory = tempfile::tempdir().unwrap();
        {
            let store = Store::open(directory.path()).unwrap();
            store
                .put_session(&SessionRecord {
                    id: "se_a".to_owned(),
                    channel: "ch_1".to_owned(),
                    machine: "ma_1".to_owned(),
                    title: "agent".to_owned(),
                    agent: "claude".to_owned(),
                    folder: PathBuf::from("/work"),
                    created_unix_ms: 1,
                })
                .unwrap();
            note_alone(&store, 4, &left(100, Some("PAY-1"), None, false));
            note_alone(&store, 4, &left(100, Some("PAY-1"), None, false));
        }
        let store = Store::open(directory.path()).unwrap();
        assert_eq!(store.away_left_out("se_a").unwrap().unwrap().count, 1);
    }

    #[test]
    fn the_summary_names_only_the_first_few_and_keeps_titles_to_one_short_line() {
        let (_directory, store) = store_with_session();
        for n in 0..12 {
            let title = format!("title {n}\nsecond line {}", "x".repeat(200));
            note_alone(&store, n + 1, &left(100, Some(&format!("PAY-{n}")), Some(&title), true));
        }
        let kept = store.away_left_out("se_a").unwrap().unwrap();
        assert_eq!(kept.count, 12);
        assert!(kept.items.len() < 12 && kept.not_made.len() < 12);
        assert!(kept.not_made.iter().all(|title| !title.contains('\n') && title.chars().count() <= 80));
    }

    #[test]
    fn a_session_that_is_gone_keeps_nothing_and_the_mark_still_moves() {
        let (_directory, store) = store_with_session();
        let mut gone = left(100, Some("PAY-1"), None, false);
        gone.session = "se_gone".to_owned();
        note_alone(&store, 1, &gone);
        assert_eq!(store.away_left_out("se_gone").unwrap(), None);
        assert_eq!(store.machine_journal_mark("ma_1").unwrap(), 1);
    }
}
