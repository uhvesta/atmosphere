//! Board items (BD-1, BD-3, BD-14, BD-16, SL-6..SL-8): one row per item,
//! its sessions and dependencies, a numbered history per item, and the
//! per-prefix counters that make keys like `PAY-7` that are never reused.
//! Plain columns, not blobs: the daemon maps them to the proto.

use std::collections::HashSet;

use rusqlite::{params, Connection, OptionalExtension};

use crate::store::{check_id, from_sql, to_sql};
use crate::{Store, StoreError};

/// The column number of Done (`BoardColumn`).
const DONE: i32 = 5;
const FIRST_COLUMN: i32 = 1;

/// The board an item sits on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BoardKey {
    Global,
    /// A channel's id.
    Channel(String),
    /// A session's id.
    Session(String),
}

impl BoardKey {
    fn parts(&self) -> (i64, &str) {
        match self {
            Self::Global => (0, ""),
            Self::Channel(id) => (1, id),
            Self::Session(id) => (2, id),
        }
    }

    fn from_parts(kind: i64, id: String) -> Self {
        match kind {
            1 => Self::Channel(id),
            2 => Self::Session(id),
            _ => Self::Global,
        }
    }
}

/// How `claim_board_prefix` went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrefixClaim {
    /// The owner had none; it now has the prefix asked for.
    Claimed,
    /// The owner already had this prefix (kept, whatever was asked for).
    AlreadyYours(String),
    /// Another owner has the prefix; ask for the next candidate.
    TakenByAnother,
}

/// One board item as saved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoardItemRecord {
    /// `PAY-7`.
    pub key: String,
    pub board: BoardKey,
    /// The channel the item belongs to: the channel board's, a session
    /// item's session's, empty on the Global board.
    pub channel: String,
    pub title: String,
    pub description: String,
    /// `BoardColumn` number, 1 to 5.
    pub column: i32,
    /// Order within the column, smaller first, compared as text.
    pub rank: String,
    /// Empty for the user, else a session id.
    pub created_by: String,
    pub created_unix_ms: i64,
    pub updated_unix_ms: i64,
    /// 1 when made, plus one for every applied change.
    pub revision: u64,
    /// When it entered Done; none while it isn't there.
    pub done_unix_ms: Option<i64>,
    /// A tombstone: kept, never listed, never revived (SL-8).
    pub deleted: bool,
    /// Sessions working on it, in the order added (BD-16).
    pub sessions: Vec<String>,
    /// Items it depends on, in the order added (BD-14).
    pub depends_on: Vec<String>,
}

/// An item to make.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewBoardItem {
    pub board: BoardKey,
    /// The owning channel; empty only on the Global board. Names the
    /// prefix the key is numbered under (`global` for the Global board).
    pub channel: String,
    pub title: String,
    pub description: String,
    pub column: i32,
    pub rank: String,
    /// Empty for the user, else a session id.
    pub created_by: String,
    pub at_unix_ms: i64,
    pub sessions: Vec<String>,
    pub depends_on: Vec<String>,
}

/// One change to an item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoardItemChange {
    pub key: String,
    /// The revision the change was made against; none writes regardless,
    /// the later write winning (SL-7).
    pub expected_revision: Option<u64>,
    /// Empty for the user, else a session id.
    pub by: String,
    pub at_unix_ms: i64,
    pub title: Option<String>,
    pub description: Option<String>,
    pub column: Option<i32>,
    pub rank: Option<String>,
    /// Replaces the whole list.
    pub sessions: Option<Vec<String>>,
    /// Replaces the whole list.
    pub depends_on: Option<Vec<String>>,
    /// Deletes the item (other fields are then ignored).
    pub delete: bool,
    /// The session whose start made the change (SE-3); empty otherwise.
    pub cause: String,
}

/// What a change came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BoardWrite {
    /// Applied (or a no-op, which doesn't bump the revision).
    Applied(BoardItemRecord),
    /// The item is at another revision than the change was made against;
    /// nothing was written, except that a stale delete is kept in history
    /// as lost.
    Stale { current: BoardItemRecord },
    /// The item is deleted; the change was kept in history as lost.
    Gone { tombstone: BoardItemRecord },
    /// The new dependencies would close a loop: the keys around it, from
    /// this item and back to it.
    Loop { cycle: Vec<String> },
    /// A dependency names no item.
    MissingDependency { key: String },
    /// A dependency names a deleted item.
    DeletedDependency { key: String },
}

/// What a history entry records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoardHistoryKind {
    Created,
    /// Title or description (see `field`).
    FieldChanged,
    /// Column or rank (see `field`).
    Moved,
    Deleted,
    SessionsChanged,
    DependenciesChanged,
}

impl BoardHistoryKind {
    fn to_sql(self) -> i64 {
        match self {
            Self::Created => 1,
            Self::FieldChanged => 2,
            Self::Moved => 3,
            Self::Deleted => 4,
            Self::SessionsChanged => 5,
            Self::DependenciesChanged => 6,
        }
    }

    fn from_sql(value: i64) -> Self {
        match value {
            2 => Self::FieldChanged,
            3 => Self::Moved,
            4 => Self::Deleted,
            5 => Self::SessionsChanged,
            6 => Self::DependenciesChanged,
            _ => Self::Created,
        }
    }
}

/// One entry in an item's history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoardHistoryEntry {
    /// Numbered from 1 per item.
    pub n: u64,
    pub at_unix_ms: i64,
    /// Empty for the user, else a session id.
    pub by: String,
    pub kind: BoardHistoryKind,
    /// `title`, `description`, `column`, `rank`, `sessions`, `depends_on`;
    /// empty for created and deleted.
    pub field: String,
    /// The value before (lists joined by `,`). For a lost write, the value
    /// that stayed.
    pub before: String,
    /// The value after. For a lost write, the value that lost.
    pub after: String,
    /// A write that lost to another (SL-6, SL-8): it changed nothing.
    pub lost: bool,
    /// The item's revision when the entry was written.
    pub revision: u64,
    /// The session whose start made the change (SE-3); empty otherwise.
    pub cause: String,
}

/// Which prefix an item's key is numbered under.
fn owner_of(board: &BoardKey, channel: &str) -> String {
    match board {
        BoardKey::Global => "global".to_owned(),
        _ => channel.to_owned(),
    }
}

fn check_column(column: i32) -> Result<(), StoreError> {
    if (FIRST_COLUMN..=DONE).contains(&column) {
        Ok(())
    } else {
        Err(StoreError::InvalidBoardColumn { column })
    }
}

const ITEM_COLUMNS: &str = "key, board_kind, board_id, channel, title, description, col, rank,
     created_by, created_unix_ms, updated_unix_ms, revision, done_unix_ms, deleted";

fn item_row(connection: &Connection, row: &rusqlite::Row<'_>) -> rusqlite::Result<BoardItemRecord> {
    let key: String = row.get(0)?;
    let sessions = strings(
        connection,
        "SELECT session FROM board_item_sessions WHERE item = ?1 ORDER BY n",
        &key,
    )?;
    let depends_on = strings(
        connection,
        "SELECT depends_on FROM board_item_deps WHERE item = ?1 ORDER BY n",
        &key,
    )?;
    Ok(BoardItemRecord {
        board: BoardKey::from_parts(row.get(1)?, row.get(2)?),
        channel: row.get(3)?,
        title: row.get(4)?,
        description: row.get(5)?,
        column: row.get(6)?,
        rank: row.get(7)?,
        created_by: row.get(8)?,
        created_unix_ms: row.get(9)?,
        updated_unix_ms: row.get(10)?,
        revision: from_sql(row.get(11)?),
        done_unix_ms: row.get(12)?,
        deleted: row.get(13)?,
        sessions,
        depends_on,
        key,
    })
}

fn strings(connection: &Connection, sql: &str, key: &str) -> rusqlite::Result<Vec<String>> {
    connection
        .prepare_cached(sql)?
        .query_map([key], |row| row.get(0))?
        .collect()
}

fn load(connection: &Connection, key: &str) -> rusqlite::Result<Option<BoardItemRecord>> {
    connection
        .query_row(
            &format!("SELECT {ITEM_COLUMNS} FROM board_items WHERE key = ?1"),
            [key],
            |row| item_row(connection, row),
        )
        .optional()
}

fn load_where(
    connection: &Connection,
    condition: &str,
    args: &[&dyn rusqlite::ToSql],
) -> rusqlite::Result<Vec<BoardItemRecord>> {
    let mut items = connection
        .prepare(&format!(
            "SELECT {ITEM_COLUMNS} FROM board_items WHERE deleted = 0 AND {condition}"
        ))?
        .query_map(args, |row| item_row(connection, row))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    items.sort_by(|a, b| (a.column, &a.rank, &a.key).cmp(&(b.column, &b.rank, &b.key)));
    Ok(items)
}

fn push_history(
    connection: &Connection,
    key: &str,
    entry: (i64, &str, BoardHistoryKind, &str, &str, &str, bool, u64),
    cause: &str,
) -> rusqlite::Result<()> {
    let (at, by, kind, field, before, after, lost, revision) = entry;
    connection.execute(
        "INSERT INTO board_history
             (item, n, at_unix_ms, actor, kind, field, old_value, new_value, lost, revision, cause)
         VALUES (?1,
                 (SELECT COALESCE(MAX(n), 0) + 1 FROM board_history WHERE item = ?1),
                 ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![key, at, by, kind.to_sql(), field, before, after, lost, to_sql(revision), cause],
    )?;
    Ok(())
}

/// Whether a field the change sets was changed by an applied write after
/// `revision` (SL-7). Session and dependency lists are added to and taken
/// from, so they never conflict.
fn touched_since(
    connection: &Connection,
    change: &BoardItemChange,
    revision: u64,
) -> rusqlite::Result<bool> {
    let fields = [
        ("title", change.title.is_some()),
        ("description", change.description.is_some()),
        ("column", change.column.is_some()),
        ("rank", change.rank.is_some()),
    ];
    let mut query = connection.prepare_cached(
        "SELECT EXISTS (SELECT 1 FROM board_history
                        WHERE item = ?1 AND revision > ?2 AND lost = 0 AND field = ?3)",
    )?;
    for (field, set) in fields {
        if set && query.query_row(params![change.key, to_sql(revision), field], |row| row.get(0))? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn dedup(list: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    list.iter().filter(|item| seen.insert(item.as_str())).cloned().collect()
}

/// The new session list in the stored order: the ones already there stay
/// where they were, new ones follow in the order given.
fn merged_sessions(current: &[String], wanted: &[String]) -> Vec<String> {
    let wanted = dedup(wanted);
    let mut merged: Vec<String> =
        current.iter().filter(|session| wanted.contains(session)).cloned().collect();
    merged.extend(wanted.into_iter().filter(|session| !current.contains(session)));
    merged
}

fn set_list(
    connection: &Connection,
    sql_delete: &str,
    sql_insert: &str,
    key: &str,
    list: &[String],
) -> rusqlite::Result<()> {
    connection.execute(sql_delete, [key])?;
    let mut insert = connection.prepare_cached(sql_insert)?;
    for (n, value) in list.iter().enumerate() {
        insert.execute(params![key, value, to_sql(n as u64)])?;
    }
    Ok(())
}

fn set_sessions(connection: &Connection, key: &str, list: &[String]) -> rusqlite::Result<()> {
    set_list(
        connection,
        "DELETE FROM board_item_sessions WHERE item = ?1",
        "INSERT INTO board_item_sessions (item, session, n) VALUES (?1, ?2, ?3)",
        key,
        list,
    )
}

fn set_deps(connection: &Connection, key: &str, list: &[String]) -> rusqlite::Result<()> {
    set_list(
        connection,
        "DELETE FROM board_item_deps WHERE item = ?1",
        "INSERT INTO board_item_deps (item, depends_on, n) VALUES (?1, ?2, ?3)",
        key,
        list,
    )
}

/// Depth-first along dependency edges from `node` looking for `target`;
/// `path` ends with the way found.
fn walk(
    connection: &Connection,
    node: &str,
    target: &str,
    path: &mut Vec<String>,
    seen: &mut HashSet<String>,
) -> rusqlite::Result<bool> {
    path.push(node.to_owned());
    if node == target {
        return Ok(true);
    }
    if seen.insert(node.to_owned()) {
        let next = strings(
            connection,
            "SELECT depends_on FROM board_item_deps WHERE item = ?1 ORDER BY n",
            node,
        )?;
        for step in next {
            if walk(connection, &step, target, path, seen)? {
                return Ok(true);
            }
        }
    }
    path.pop();
    Ok(false)
}

/// The loop `item` would close by depending on `deps`, if any: the keys
/// from `item` around and back to it.
fn find_cycle(
    connection: &Connection,
    item: &str,
    deps: &[String],
) -> rusqlite::Result<Option<Vec<String>>> {
    let mut seen = HashSet::new();
    for dep in deps {
        let mut path = vec![item.to_owned()];
        if walk(connection, dep, item, &mut path, &mut seen)? {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

/// A dependency list that can't be kept: the outcome naming why.
fn refuse_deps(
    connection: &Connection,
    item: &str,
    already: &[String],
    deps: &[String],
) -> rusqlite::Result<Option<BoardWrite>> {
    for dep in deps {
        match load(connection, dep)? {
            None => return Ok(Some(BoardWrite::MissingDependency { key: dep.clone() })),
            Some(found) if found.deleted && !already.contains(dep) => {
                return Ok(Some(BoardWrite::DeletedDependency { key: dep.clone() }));
            }
            Some(_) => {}
        }
    }
    Ok(find_cycle(connection, item, deps)?.map(|cycle| BoardWrite::Loop { cycle }))
}

impl Store {
    /// The prefix this owner (a channel id, or `global`) numbers its items
    /// under, if it has one.
    pub fn board_prefix(&self, owner: &str) -> Result<Option<String>, StoreError> {
        check_id(owner)?;
        self.inner().read("read a board prefix", |connection| {
            connection
                .query_row("SELECT prefix FROM board_prefixes WHERE owner = ?1", [owner], |row| {
                    row.get(0)
                })
                .optional()
        })
    }

    /// Records `prefix` for `owner`, atomically; refuses one another owner
    /// has.
    pub fn claim_board_prefix(&self, owner: &str, prefix: &str) -> Result<PrefixClaim, StoreError> {
        check_id(owner)?;
        check_id(prefix)?;
        self.inner().write("boards.claim_prefix", "claim a board prefix", |transaction| {
            let mine: Option<String> = transaction
                .query_row("SELECT prefix FROM board_prefixes WHERE owner = ?1", [owner], |row| {
                    row.get(0)
                })
                .optional()?;
            if let Some(mine) = mine {
                return Ok(PrefixClaim::AlreadyYours(mine));
            }
            let taken: Option<String> = transaction
                .query_row("SELECT owner FROM board_prefixes WHERE prefix = ?1", [prefix], |row| {
                    row.get(0)
                })
                .optional()?;
            if taken.is_some() {
                return Ok(PrefixClaim::TakenByAnother);
            }
            transaction.execute(
                "INSERT INTO board_prefixes (owner, prefix) VALUES (?1, ?2)",
                params![owner, prefix],
            )?;
            Ok(PrefixClaim::Claimed)
        })
    }

    /// Makes an item under the next number of its owner's prefix, with its
    /// sessions, dependencies and a created entry, in one transaction.
    pub fn create_board_item(&self, new: &NewBoardItem) -> Result<BoardItemRecord, StoreError> {
        check_column(new.column)?;
        let owner = owner_of(&new.board, &new.channel);
        check_id(&owner)?;
        let sessions = dedup(&new.sessions);
        let deps = dedup(&new.depends_on);
        let inner = self.inner();
        let doing = "create a board item";
        inner.write_with("boards.create", doing, |transaction| {
            let failed = inner.failed(doing);
            let prefix: Option<String> = transaction
                .query_row("SELECT prefix FROM board_prefixes WHERE owner = ?1", [&owner], |row| {
                    row.get(0)
                })
                .optional()
                .map_err(&failed)?;
            let Some(prefix) = prefix else {
                return Err(StoreError::NoBoardPrefix { owner });
            };
            for dep in &deps {
                let found = load(transaction, dep).map_err(&failed)?;
                if found.is_none_or(|item| item.deleted) {
                    return Err(StoreError::NoBoardItem { key: dep.clone() });
                }
            }
            let last: i64 = transaction
                .query_row("SELECT last FROM board_counters WHERE prefix = ?1", [&prefix], |row| {
                    row.get(0)
                })
                .optional()
                .map_err(&failed)?
                .unwrap_or(0);
            let number = last + 1;
            transaction
                .execute(
                    "INSERT INTO board_counters (prefix, last) VALUES (?1, ?2)
                     ON CONFLICT (prefix) DO UPDATE SET last = excluded.last",
                    params![prefix, number],
                )
                .map_err(&failed)?;
            let key = format!("{prefix}-{number}");
            let (kind, id) = new.board.parts();
            let done = (new.column == DONE).then_some(new.at_unix_ms);
            transaction
                .execute(
                    "INSERT INTO board_items
                         (key, board_kind, board_id, channel, title, description, col, rank,
                          created_by, created_unix_ms, updated_unix_ms, revision, done_unix_ms, deleted)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10, 1, ?11, 0)",
                    params![
                        key, kind, id, new.channel, new.title, new.description, new.column,
                        new.rank, new.created_by, new.at_unix_ms, done
                    ],
                )
                .map_err(&failed)?;
            set_sessions(transaction, &key, &sessions).map_err(&failed)?;
            set_deps(transaction, &key, &deps).map_err(&failed)?;
            push_history(
                transaction,
                &key,
                (new.at_unix_ms, &new.created_by, BoardHistoryKind::Created, "", "", "", false, 1),
                "",
            )
            .map_err(&failed)?;
            load(transaction, &key)
                .map_err(&failed)?
                .ok_or(StoreError::NoBoardItem { key })
        })
    }

    /// One item, a tombstone included (`deleted` is true).
    pub fn board_item(&self, key: &str) -> Result<Option<BoardItemRecord>, StoreError> {
        check_id(key)?;
        self.inner().read("read a board item", |connection| load(connection, key))
    }

    /// The live items on any of `boards`, by column, then rank, then key.
    pub fn board_items(&self, boards: &[BoardKey]) -> Result<Vec<BoardItemRecord>, StoreError> {
        for board in boards {
            if !matches!(board, BoardKey::Global) {
                check_id(board.parts().1)?;
            }
        }
        self.inner().read("list board items", |connection| {
            let mut items = Vec::new();
            for board in boards {
                let (kind, id) = board.parts();
                items.extend(load_where(
                    connection,
                    "board_kind = ?1 AND board_id = ?2",
                    &[&kind, &id],
                )?);
            }
            items.sort_by(|a, b| (a.column, &a.rank, &a.key).cmp(&(b.column, &b.rank, &b.key)));
            Ok(items)
        })
    }

    /// A channel's items and every session item of that channel (BD-5).
    pub fn board_items_in_channel(&self, channel: &str) -> Result<Vec<BoardItemRecord>, StoreError> {
        check_id(channel)?;
        self.inner().read("list a channel's board items", |connection| {
            load_where(connection, "channel = ?1", &[&channel])
        })
    }

    /// The Global board's own items and every channel's, not session items
    /// (BD-7).
    pub fn all_board_items_but_sessions(&self) -> Result<Vec<BoardItemRecord>, StoreError> {
        self.inner().read("list board items", |connection| {
            load_where(connection, "board_kind IN (0, 1)", &[])
        })
    }

    /// Applies `change` in one transaction; see [`BoardWrite`].
    pub fn change_board_item(&self, change: &BoardItemChange) -> Result<BoardWrite, StoreError> {
        check_id(&change.key)?;
        if let Some(column) = change.column {
            check_column(column)?;
        }
        let inner = self.inner();
        let doing = "change a board item";
        inner.write_with("boards.change", doing, |transaction| {
            let failed = inner.failed(doing);
            let Some(current) = load(transaction, &change.key).map_err(&failed)? else {
                return Err(StoreError::NoBoardItem {
                    key: change.key.clone(),
                });
            };
            apply(transaction, change, current).map_err(&failed)
        })
    }

    /// An item's history after entry `after_n`, oldest first, at most
    /// `limit` entries.
    pub fn board_item_history(
        &self,
        key: &str,
        after_n: u64,
        limit: usize,
    ) -> Result<Vec<BoardHistoryEntry>, StoreError> {
        check_id(key)?;
        self.inner().read("read a board item's history", |connection| {
            connection
                .prepare(
                    "SELECT n, at_unix_ms, actor, kind, field, old_value, new_value, lost, revision,
                            cause
                     FROM board_history WHERE item = ?1 AND n > ?2 ORDER BY n LIMIT ?3",
                )?
                .query_map(params![key, to_sql(after_n), to_sql(limit as u64)], |row| {
                    Ok(BoardHistoryEntry {
                        n: from_sql(row.get(0)?),
                        at_unix_ms: row.get(1)?,
                        by: row.get(2)?,
                        kind: BoardHistoryKind::from_sql(row.get(3)?),
                        field: row.get(4)?,
                        before: row.get(5)?,
                        after: row.get(6)?,
                        lost: row.get(7)?,
                        revision: from_sql(row.get(8)?),
                        cause: row.get(9)?,
                    })
                })?
                .collect()
        })
    }
}

/// One field the change would set, against what the item holds.
struct FieldEdit {
    kind: BoardHistoryKind,
    field: &'static str,
    before: String,
    after: String,
}

fn edits(change: &BoardItemChange, current: &BoardItemRecord, include_rank: bool) -> Vec<FieldEdit> {
    let mut edits = Vec::new();
    let mut text = |kind, field, before: &str, after: &str| {
        if before != after {
            edits.push(FieldEdit {
                kind,
                field,
                before: before.to_owned(),
                after: after.to_owned(),
            });
        }
    };
    if let Some(title) = &change.title {
        text(BoardHistoryKind::FieldChanged, "title", &current.title, title);
    }
    if let Some(description) = &change.description {
        text(BoardHistoryKind::FieldChanged, "description", &current.description, description);
    }
    if let Some(column) = change.column {
        text(BoardHistoryKind::Moved, "column", &current.column.to_string(), &column.to_string());
    }
    if let (true, Some(rank)) = (include_rank, &change.rank) {
        text(BoardHistoryKind::Moved, "rank", &current.rank, rank);
    }
    if let Some(sessions) = &change.sessions {
        text(
            BoardHistoryKind::SessionsChanged,
            "sessions",
            &current.sessions.join(","),
            &merged_sessions(&current.sessions, sessions).join(","),
        );
    }
    if let Some(deps) = &change.depends_on {
        text(
            BoardHistoryKind::DependenciesChanged,
            "depends_on",
            &current.depends_on.join(","),
            &dedup(deps).join(","),
        );
    }
    edits
}

fn apply(
    connection: &Connection,
    change: &BoardItemChange,
    current: BoardItemRecord,
) -> rusqlite::Result<BoardWrite> {
    let key = change.key.as_str();
    let lost = |kind, field: &str, before: &str, after: &str| {
        push_history(
            connection,
            key,
            (change.at_unix_ms, &change.by, kind, field, before, after, true, current.revision),
            &change.cause,
        )
    };
    if current.deleted {
        // An edit or move for a deleted item is kept, not applied (SL-8).
        for edit in edits(change, &current, true) {
            lost(edit.kind, edit.field, &edit.before, &edit.after)?;
        }
        return Ok(BoardWrite::Gone { tombstone: current });
    }
    // A stale delete always loses; a stale edit or move loses only when a
    // field it sets was changed since (SL-7, D151).
    if let Some(expected) = change.expected_revision.filter(|&expected| expected != current.revision) {
        if change.delete {
            lost(BoardHistoryKind::Deleted, "", "", "")?;
            return Ok(BoardWrite::Stale { current });
        }
        if touched_since(connection, change, expected)? {
            for edit in edits(change, &current, true) {
                lost(edit.kind, edit.field, &edit.before, &edit.after)?;
            }
            return Ok(BoardWrite::Stale { current });
        }
    }
    let revision = current.revision + 1;
    let write = |kind, field: &str, before: &str, after: &str| {
        push_history(
            connection,
            key,
            (change.at_unix_ms, &change.by, kind, field, before, after, false, revision),
            &change.cause,
        )
    };
    if change.delete {
        connection.execute(
            "UPDATE board_items SET deleted = 1, revision = ?2, updated_unix_ms = ?3 WHERE key = ?1",
            params![key, to_sql(revision), change.at_unix_ms],
        )?;
        write(BoardHistoryKind::Deleted, "", "", "")?;
        // Nothing may depend on a deleted item: the edges to it go, and
        // each item that had one says so in its history.
        let dependents = strings(
            connection,
            "SELECT item FROM board_item_deps WHERE depends_on = ?1 ORDER BY item",
            key,
        )?;
        connection.execute("DELETE FROM board_item_deps WHERE depends_on = ?1", [key])?;
        for dependent in dependents {
            let at = load(connection, &dependent)?.map_or(0, |item| item.revision);
            push_history(
                connection,
                &dependent,
                (
                    change.at_unix_ms,
                    &change.by,
                    BoardHistoryKind::DependenciesChanged,
                    "depends_on",
                    key,
                    "",
                    false,
                    at,
                ),
                &change.cause,
            )?;
        }
        return Ok(BoardWrite::Applied(reload(connection, key)?));
    }
    // A move into another column takes its rank with it; a rank on its own
    // is an entry too.
    let applied = edits(change, &current, true);
    if applied.is_empty() {
        return Ok(BoardWrite::Applied(current));
    }
    if let Some(deps) = &change.depends_on {
        let deps = dedup(deps);
        if let Some(refused) = refuse_deps(connection, key, &current.depends_on, &deps)? {
            return Ok(refused);
        }
    }
    for edit in &applied {
        match edit.field {
            "title" => connection.execute(
                "UPDATE board_items SET title = ?2 WHERE key = ?1",
                params![key, edit.after],
            )?,
            "description" => connection.execute(
                "UPDATE board_items SET description = ?2 WHERE key = ?1",
                params![key, edit.after],
            )?,
            "rank" => connection.execute(
                "UPDATE board_items SET rank = ?2 WHERE key = ?1",
                params![key, edit.after],
            )?,
            "column" => {
                let column: i32 = change.column.unwrap_or(current.column);
                let done = if column == DONE {
                    Some(change.at_unix_ms)
                } else {
                    None
                };
                connection.execute(
                    "UPDATE board_items SET col = ?2, done_unix_ms = ?3 WHERE key = ?1",
                    params![key, column, done],
                )?
            }
            "sessions" => {
                let wanted = change.sessions.as_deref().unwrap_or(&[]);
                set_sessions(connection, key, &merged_sessions(&current.sessions, wanted))?;
                0
            }
            _ => {
                set_deps(connection, key, &dedup(change.depends_on.as_deref().unwrap_or(&[])))?;
                0
            }
        };
        write(edit.kind, edit.field, &edit.before, &edit.after)?;
    }
    connection.execute(
        "UPDATE board_items SET revision = ?2, updated_unix_ms = ?3 WHERE key = ?1",
        params![key, to_sql(revision), change.at_unix_ms],
    )?;
    Ok(BoardWrite::Applied(reload(connection, key)?))
}

fn reload(connection: &Connection, key: &str) -> rusqlite::Result<BoardItemRecord> {
    load(connection, key)?.ok_or(rusqlite::Error::QueryReturnedNoRows)
}
