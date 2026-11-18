//! What a tab's screen has answered of the program's terminal queries, kept
//! so a restarted daemon answers none twice and drops none (D271). It sits
//! in the tab's own row, so closing the tab removes it, and each save is one
//! transaction: a power cut leaves the record from before or after it.

use rusqlite::{OptionalExtension, Transaction, params};

use crate::store::{check_id, from_sql, put_tab, to_sql};
use crate::{Store, StoreError, TabRecord};

/// A tab screen's reply record, for one holder process.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TabReplies {
    /// The tab's holder generation it was made for. A holder started
    /// again has offsets of its own, so a record of another one is stale.
    pub generation: u64,
    /// The end of the last output whose queries have had their answers.
    pub answered_through: u64,
    /// Answers handed out and not yet confirmed written, by sequence.
    pub pending: Vec<u64>,
    /// An answer was in flight across a restart, so what the program has
    /// read is unknown.
    pub uncertain: bool,
}

fn joined(pending: &[u64]) -> String {
    pending.iter().map(u64::to_string).collect::<Vec<_>>().join(",")
}

fn split(text: &str) -> Vec<u64> {
    text.split(',').filter_map(|part| part.parse().ok()).collect()
}

impl Store {
    /// Saves `tab`'s reply record, replacing the last. A tab that is gone
    /// keeps nothing.
    pub fn put_tab_replies(&self, tab: &str, replies: &TabReplies) -> Result<(), StoreError> {
        check_id(tab)?;
        self.inner().write("tab.replies", "save a tab's reply record", |transaction| {
            put_replies(transaction, tab, replies)
        })
    }

    /// Saves `tab` and its reply record in one commit. A holder new to its
    /// tab is saved this way, so the tab never names it without a record of
    /// its own.
    pub fn put_tab_with_replies(&self, tab: &TabRecord, replies: &TabReplies) -> Result<(), StoreError> {
        check_id(&tab.id)?;
        let doing = "save a tab";
        let inner = self.inner();
        inner.write_for_session("tab.put", doing, tab.session.as_deref(), |transaction| {
            put_tab(transaction, tab).map_err(inner.failed(doing))?;
            put_replies(transaction, &tab.id, replies).map_err(inner.failed(doing))
        })
    }

    /// `tab`'s reply record, or none when it never had one, as a tab saved
    /// before records existed.
    pub fn tab_replies(&self, tab: &str) -> Result<Option<TabReplies>, StoreError> {
        check_id(tab)?;
        self.inner().read("read a tab's reply record", |connection| {
            connection
                .query_row(
                    "SELECT reply_generation, reply_answered, reply_pending, reply_uncertain
                     FROM tabs WHERE id = ?1",
                    [tab],
                    |row| {
                        let generation: Option<i64> = row.get(0)?;
                        Ok(generation.map(|generation| TabReplies {
                            generation: from_sql(generation),
                            answered_through: from_sql(row.get::<_, i64>(1).unwrap_or(0)),
                            pending: split(&row.get::<_, String>(2).unwrap_or_default()),
                            uncertain: row.get::<_, i64>(3).unwrap_or(0) != 0,
                        }))
                    },
                )
                .optional()
                .map(Option::flatten)
        })
    }
}

/// Writes `tab`'s reply record inside `transaction`.
pub(crate) fn put_replies(
    transaction: &Transaction<'_>,
    tab: &str,
    replies: &TabReplies,
) -> rusqlite::Result<()> {
    transaction
        .execute(
            "UPDATE tabs SET reply_generation = ?2, reply_answered = ?3,
                             reply_pending = ?4, reply_uncertain = ?5
             WHERE id = ?1",
            params![
                tab,
                to_sql(replies.generation),
                to_sql(replies.answered_through),
                joined(&replies.pending),
                i64::from(replies.uncertain),
            ],
        )
        .map(drop)
}
