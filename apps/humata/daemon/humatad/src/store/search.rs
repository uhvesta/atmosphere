//! The search index: what was said and done in every session, and the
//! review comments and specs, in an FTS5 table. The daemon fills it from
//! each session's history before a search; this keeps how far it got.

use rusqlite::{Transaction, params};

use super::Store;
use crate::error::StoreError;

/// What an entry is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    Message,
    Reply,
    ToolCall,
    ToolOutput,
    Comment,
    Spec,
}

impl EntryKind {
    fn name(self) -> &'static str {
        match self {
            Self::Message => "message",
            Self::Reply => "reply",
            Self::ToolCall => "tool_call",
            Self::ToolOutput => "tool_output",
            Self::Comment => "comment",
            Self::Spec => "spec",
        }
    }

    fn named(name: &str) -> Option<Self> {
        [
            Self::Message,
            Self::Reply,
            Self::ToolCall,
            Self::ToolOutput,
            Self::Comment,
            Self::Spec,
        ]
        .into_iter()
        .find(|kind| kind.name() == name)
    }
}

/// An event of a session's, as it's indexed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub kind: EntryKind,
    pub event: String,
    /// The turn it's in, from 1.
    pub turn: u32,
    pub time_unix_ms: i64,
    pub text: String,
}

/// A session as the index last saw it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Indexed {
    /// How many of its events are indexed.
    pub events: usize,
    pub turns: u32,
}

/// A session as the index last saw it, and where it was then.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexedAt {
    pub indexed: Indexed,
    pub whereabouts: Whereabouts,
}

/// Where a session is, kept with its entries so a search can be narrowed
/// to a project or task.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Whereabouts {
    pub host: String,
    pub project: String,
    pub item: String,
}

/// What to find.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    /// An FTS5 query.
    pub matching: String,
    pub since_unix_ms: Option<i64>,
    pub until_unix_ms: Option<i64>,
    /// Only these projects, when given; empty for any.
    pub projects: Vec<String>,
    pub item: Option<String>,
    pub session: Option<String>,
    pub limit: usize,
}

/// An entry found, with the text around the words found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub kind: EntryKind,
    /// Empty for a spec.
    pub session: String,
    /// Empty for a comment or a spec.
    pub event: String,
    pub turn: u32,
    pub time_unix_ms: i64,
    pub project: String,
    pub item: String,
    pub host: String,
    /// The review thread, for a comment.
    pub thread: i64,
    /// Each word found is between [`MARK_START`] and [`MARK_END`].
    pub snippet: String,
}

/// Around each word found in a snippet: characters from Unicode's private
/// use area, which text from an agent or a person doesn't have.
pub const MARK_START: char = '\u{E000}';
pub const MARK_END: char = '\u{E001}';

impl Store {
    /// How far `session` is indexed, and where it was then; None if it
    /// isn't yet.
    pub fn indexed(&self, session: &str) -> Result<Option<IndexedAt>, StoreError> {
        use rusqlite::OptionalExtension as _;
        let connection = self.lock();
        connection
            .prepare_cached(
                "SELECT events, turns, host, project, item FROM search_sessions WHERE id = ?1",
            )
            .and_then(|mut statement| {
                statement
                    .query_row(params![session], |row| {
                        Ok(IndexedAt {
                            indexed: Indexed {
                                events: row.get::<_, i64>(0)?.try_into().unwrap_or(0),
                                turns: row.get(1)?,
                            },
                            whereabouts: Whereabouts {
                                host: row.get(2)?,
                                project: row.get(3)?,
                                item: row.get(4)?,
                            },
                        })
                    })
                    .optional()
            })
            .map_err(self.failed())
    }

    /// Adds `entries`, the events of `session` after the `from` it had
    /// indexed, and records it as indexed to `indexed`. Entries that start
    /// before what's indexed now, as when two searches catch up at once,
    /// are dropped.
    pub fn index(
        &self,
        session: &str,
        whereabouts: &Whereabouts,
        from: usize,
        indexed: &Indexed,
        entries: &[Entry],
    ) -> Result<(), StoreError> {
        let mut connection = self.lock();
        let indexing = (|| {
            let transaction = connection.transaction()?;
            let had: i64 = transaction.query_row(
                "SELECT coalesce((SELECT events FROM search_sessions WHERE id = ?1), 0)",
                params![session],
                |row| row.get(0),
            )?;
            if usize::try_from(had).unwrap_or(0) == from {
                for entry in entries {
                    insert(&transaction, session, entry)?;
                }
            }
            transaction.execute(
                "INSERT INTO search_sessions (id, host, project, item, events, turns)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT (id) DO UPDATE SET host = ?2, project = ?3, item = ?4,
                    events = max(events, ?5), turns = ?6",
                params![
                    session,
                    whereabouts.host,
                    whereabouts.project,
                    whereabouts.item,
                    i64::try_from(indexed.events).unwrap_or(i64::MAX),
                    indexed.turns,
                ],
            )?;
            transaction.commit()
        })();
        indexing.map_err(self.failed())
    }

    /// Indexes the review comments again if any was added, changed or
    /// removed since they were last indexed, and the specs likewise, each
    /// apart: a search after neither changed writes nothing. Returns how
    /// many of the two were indexed again.
    pub fn index_comments_and_specs(&self) -> Result<usize, StoreError> {
        use rusqlite::OptionalExtension as _;
        let mut connection = self.lock();
        let indexing = (|| {
            let mut again = 0;
            for part in [&COMMENTS, &SPECS] {
                let now: String = connection.query_row(part.signature, [], |row| row.get(0))?;
                let had: Option<String> = connection
                    .query_row(
                        "SELECT value FROM settings WHERE key = ?1",
                        [part.key],
                        |row| row.get(0),
                    )
                    .optional()?;
                if had.as_deref() == Some(now.as_str()) {
                    continue;
                }
                let transaction = connection.transaction()?;
                transaction.execute("DELETE FROM search_entries WHERE kind = ?1", [part.kind])?;
                transaction.execute(part.insert, [])?;
                transaction.execute(
                    "INSERT INTO settings (key, value) VALUES (?1, ?2)
                     ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                    params![part.key, now],
                )?;
                transaction.commit()?;
                again += 1;
            }
            Ok(again)
        })();
        indexing.map_err(self.failed())
    }

    /// The entries `query` finds, best match first.
    pub fn search(&self, query: &Query) -> Result<Vec<Found>, StoreError> {
        let connection = self.lock();
        let found = (|| -> rusqlite::Result<Vec<Found>> {
            let projects = serde_json::to_string(&query.projects).unwrap_or_default();
            let mut statement = connection.prepare_cached(
                "SELECT entries.kind, entries.session, entries.event, entries.turn, entries.time,
                    coalesce(nullif(entries.project, ''), sessions.project, ''),
                    coalesce(nullif(entries.item, ''), sessions.item, ''),
                    coalesce(sessions.host, ''), entries.thread,
                    snippet(search_entries, 0, char(57344), char(57345), '…', 16)
                 FROM search_entries AS entries
                 LEFT JOIN search_sessions AS sessions ON sessions.id = entries.session
                 WHERE search_entries MATCH ?1
                    AND (?2 IS NULL OR entries.time >= ?2)
                    AND (?3 IS NULL OR entries.time < ?3)
                    AND (?4 = '[]' OR coalesce(nullif(entries.project, ''), sessions.project, '')
                        IN (SELECT value FROM json_each(?4)))
                    AND (?5 IS NULL OR coalesce(nullif(entries.item, ''), sessions.item, '') = ?5)
                    AND (?6 IS NULL OR entries.session = ?6)
                 ORDER BY rank
                 LIMIT ?7",
            )?;
            let rows = statement.query_map(
                params![
                    query.matching,
                    query.since_unix_ms,
                    query.until_unix_ms,
                    projects,
                    query.item,
                    query.session,
                    i64::try_from(query.limit).unwrap_or(i64::MAX),
                ],
                |row| {
                    Ok(Found {
                        kind: EntryKind::named(&row.get::<_, String>(0)?)
                            .unwrap_or(EntryKind::Message),
                        session: row.get(1)?,
                        event: row.get(2)?,
                        turn: row.get(3)?,
                        time_unix_ms: row.get(4)?,
                        project: row.get(5)?,
                        item: row.get(6)?,
                        host: row.get(7)?,
                        thread: row.get(8)?,
                        snippet: row.get(9)?,
                    })
                },
            )?;
            rows.collect()
        })();
        found.map_err(self.failed())
    }

    /// The time `modifiers` give from now, in SQLite's date functions,
    /// read in this Mac's time zone: such as the start of today for
    /// `["start of day"]`.
    pub fn local_time(&self, modifiers: &[String]) -> Result<i64, StoreError> {
        let mut sql = "SELECT CAST(unixepoch('now', 'localtime'".to_owned();
        for index in 1..=modifiers.len() {
            sql.push_str(&format!(", ?{index}"));
        }
        sql.push_str(", 'utc') AS INTEGER) * 1000");
        let connection = self.lock();
        connection
            .query_row(&sql, rusqlite::params_from_iter(modifiers), |row| {
                row.get(0)
            })
            .map_err(self.failed())
    }
}

/// Comments or specs, as the index holds them: what tells whether any
/// changed since they were indexed, where that is kept, and how they're
/// indexed.
struct Indexes {
    kind: &'static str,
    /// Its key in the settings.
    key: &'static str,
    /// Changes when one is added, removed or edited.
    signature: &'static str,
    insert: &'static str,
}

const COMMENTS: Indexes = Indexes {
    kind: "comment",
    key: "search.comments",
    signature: "SELECT count(*) || ':' || coalesce(max(id), 0) || ':' || total(length(text))
        FROM review_comments",
    insert:
        "INSERT INTO search_entries (text, kind, session, event, turn, time, project, item, thread)
        SELECT comments.text, 'comment', threads.session_id, '', 0,
            comments.created_unix_ms, '', '', threads.id
        FROM review_comments AS comments
        JOIN review_threads AS threads ON threads.id = comments.thread_id",
};

const SPECS: Indexes = Indexes {
    kind: "spec",
    key: "search.specs",
    signature: "SELECT count(*) || ':' || coalesce(max(updated_unix_ms), 0) || ':'
            || total(length(title) + length(spec))
        FROM items",
    insert:
        "INSERT INTO search_entries (text, kind, session, event, turn, time, project, item, thread)
        SELECT title || char(10) || spec, 'spec', '', '', 0, updated_unix_ms, project_id, id, 0
        FROM items",
};

fn insert(transaction: &Transaction<'_>, session: &str, entry: &Entry) -> rusqlite::Result<()> {
    transaction.execute(
        "INSERT INTO search_entries (text, kind, session, event, turn, time, project, item, thread)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, '', '', 0)",
        params![
            entry.text,
            entry.kind.name(),
            session,
            entry.event,
            entry.turn,
            entry.time_unix_ms,
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let store = Store::open(&dir.path().join("humata.sqlite3")).expect("the store opens");
        (dir, store)
    }

    fn entry(event: &str, kind: EntryKind, time_unix_ms: i64, text: &str) -> Entry {
        Entry {
            kind,
            event: event.to_owned(),
            turn: 1,
            time_unix_ms,
            text: text.to_owned(),
        }
    }

    fn whereabouts(project: &str) -> Whereabouts {
        Whereabouts {
            host: "local".to_owned(),
            project: project.to_owned(),
            item: String::new(),
        }
    }

    fn query(matching: &str) -> Query {
        Query {
            matching: matching.to_owned(),
            limit: 10,
            ..Query::default()
        }
    }

    fn indexed(events: usize) -> Indexed {
        Indexed { events, turns: 1 }
    }

    #[test]
    fn entries_are_found_by_word_stem_with_the_words_marked() {
        let (_dir, store) = store();
        let entries = [
            entry(
                "e1",
                EntryKind::Message,
                1_000,
                "the migrations keep failing",
            ),
            entry("e2", EntryKind::ToolCall, 2_000, "cargo test --workspace"),
        ];
        store
            .index("s1", &whereabouts("p1"), 0, &indexed(2), &entries)
            .expect("indexed");

        let found = store.search(&query("\"migration\"")).expect("searched");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].event, "e1");
        assert_eq!(found[0].project, "p1");
        assert!(found[0].snippet.contains(MARK_START));
        assert!(found[0].snippet.contains(MARK_END));
        assert_eq!(
            store
                .indexed("s1")
                .expect("read")
                .map(|indexed| indexed.indexed.events),
            Some(2)
        );
    }

    #[test]
    fn a_search_is_narrowed_to_its_projects_and_time() {
        let (_dir, store) = store();
        store
            .index(
                "s1",
                &whereabouts("p1"),
                0,
                &indexed(1),
                &[entry("e1", EntryKind::Reply, 1_000, "deploy the app")],
            )
            .expect("indexed");
        store
            .index(
                "s2",
                &whereabouts("p2"),
                0,
                &indexed(1),
                &[entry("e2", EntryKind::Reply, 5_000, "deploy the server")],
            )
            .expect("indexed");

        let mut narrowed = query("\"deploy\"");
        narrowed.projects = vec!["p2".to_owned()];
        let found = store.search(&narrowed).expect("searched");
        assert_eq!(
            found
                .iter()
                .map(|found| found.session.as_str())
                .collect::<Vec<_>>(),
            ["s2"]
        );

        let mut since = query("\"deploy\"");
        since.since_unix_ms = Some(2_000);
        let found = store.search(&since).expect("searched");
        assert_eq!(
            found
                .iter()
                .map(|found| found.event.as_str())
                .collect::<Vec<_>>(),
            ["e2"]
        );
    }

    #[test]
    fn entries_from_an_index_already_moved_on_are_dropped() {
        let (_dir, store) = store();
        let first = [entry("e1", EntryKind::Message, 1_000, "rename the crate")];
        store
            .index("s1", &whereabouts("p1"), 0, &indexed(1), &first)
            .expect("indexed");
        // Another search caught up from 0 meanwhile; its entries are already in.
        store
            .index("s1", &whereabouts("p1"), 0, &indexed(1), &first)
            .expect("indexed");
        assert_eq!(
            store.search(&query("\"rename\"")).expect("searched").len(),
            1
        );
    }

    #[test]
    fn specs_are_indexed_again_only_when_one_changed() {
        use crate::board::{Author, ItemFields, ItemPatch, Priority, Status};
        let (_dir, store) = store();
        let project = store
            .add_project(
                "one".into(),
                vec![humata_agent::Repository {
                    name: "one".into(),
                    path: "/code/one".into(),
                }],
                std::collections::BTreeMap::new(),
                String::new(),
                Vec::new(),
            )
            .expect("added");
        let author = Author::on(humata_wire::pb::ChangeSurface::Mcp);
        let item = store
            .add_item(
                &project.id,
                ItemFields {
                    parent_id: None,
                    status: Status::Todo,
                    priority: Priority::None,
                    title: "Tidy".into(),
                    spec: "rename the schema".into(),
                },
                &author,
            )
            .expect("added");
        assert_eq!(store.index_comments_and_specs().expect("indexed"), 2);
        assert_eq!(store.index_comments_and_specs().expect("indexed"), 0);
        assert_eq!(
            store.search(&query("\"schema\"")).expect("searched").len(),
            1
        );

        store
            .update_item(
                &item.id,
                ItemPatch {
                    spec: Some("rename the migrations".into()),
                    ..ItemPatch::default()
                },
                &author,
            )
            .expect("updated");
        assert_eq!(store.index_comments_and_specs().expect("indexed"), 1);
        assert!(
            store
                .search(&query("\"schema\""))
                .expect("searched")
                .is_empty()
        );
        assert_eq!(
            store
                .search(&query("\"migrations\""))
                .expect("searched")
                .len(),
            1
        );
    }

    #[test]
    fn a_named_time_is_read_from_now() {
        let (_dir, store) = store();
        let today = store
            .local_time(&["start of day".to_owned()])
            .expect("read");
        let yesterday = store
            .local_time(&["start of day".to_owned(), "-1 day".to_owned()])
            .expect("read");
        assert!(yesterday < today);
        assert!(today <= super::super::now_unix_ms());
    }
}
