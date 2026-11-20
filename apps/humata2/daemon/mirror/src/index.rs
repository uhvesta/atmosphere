//! The full-text index over the mirrored entries (LD-23): derived data in
//! one SQLite file, rebuilt from the mirror files whenever it can't be used.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use rusqlite::types::Value;
use rusqlite::{Connection, OpenFlags, params, params_from_iter};

use crate::session::{self, SavedMeta, State};
use crate::{FORMAT_VERSION, Hit, MirrorEntry, MirrorError, Role};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS sessions (
    session TEXT PRIMARY KEY,
    channel TEXT NOT NULL,
    channel_name TEXT NOT NULL,
    title TEXT NOT NULL,
    machine TEXT NOT NULL,
    machine_name TEXT NOT NULL,
    agent TEXT NOT NULL,
    archived INTEGER NOT NULL,
    as_of TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS entries (
    id INTEGER PRIMARY KEY,
    session TEXT NOT NULL,
    seq INTEGER NOT NULL,
    turn INTEGER NOT NULL,
    role INTEGER NOT NULL,
    written TEXT,
    text TEXT NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS entries_by_session ON entries (session, seq);
CREATE VIRTUAL TABLE IF NOT EXISTS fts USING fts5(
    text, content='entries', content_rowid='id', tokenize='unicode61 remove_diacritics 2'
);
";

/// What a search asks the index, already made plain.
pub(crate) struct Search<'a> {
    /// The FTS5 expression made from the query's words.
    pub expression: &'a str,
    pub channels: &'a [String],
    pub machine: Option<&'a str>,
    pub agent: Option<&'a str>,
    pub since: Option<&'a str>,
    pub until: Option<&'a str>,
    pub limit: u32,
    pub offset: u64,
}

pub(crate) struct Index {
    conn: Connection,
    path: PathBuf,
}

fn role_number(role: Role) -> i64 {
    match role {
        Role::User => 0,
        Role::Agent => 1,
        Role::Tool => 2,
    }
}

fn role_of(number: i64) -> Role {
    match number {
        0 => Role::User,
        1 => Role::Agent,
        _ => Role::Tool,
    }
}

impl Index {
    /// Opens the index at `path`, or makes an empty one when the file is
    /// missing, damaged or of another [`FORMAT_VERSION`]. The version is
    /// stamped by [`Index::finish`], after the index is filled.
    pub(crate) fn open(path: &Path) -> Result<Index, MirrorError> {
        // Any trouble reading the old file means it is replaced: it only
        // holds copies.
        if let Ok(Some(conn)) = open_existing(path) {
            return Ok(Index {
                conn,
                path: path.to_path_buf(),
            });
        }
        for suffix in ["", "-wal", "-shm", "-journal"] {
            let mut name = path.as_os_str().to_owned();
            name.push(suffix);
            match fs::remove_file(PathBuf::from(&name)) {
                Err(source) if source.kind() != ErrorKind::NotFound => {
                    return Err(MirrorError::Io {
                        path: PathBuf::from(name),
                        source,
                    });
                }
                _ => {}
            }
        }
        let index = Index {
            conn: Connection::open(path).map_err(db(path))?,
            path: path.to_path_buf(),
        };
        index.configure()?;
        index.conn.execute_batch(SCHEMA).map_err(db(path))?;
        Ok(index)
    }

    fn configure(&self) -> Result<(), MirrorError> {
        configure(&self.conn).map_err(db(&self.path))
    }

    /// Marks the index as complete, in the current format.
    pub(crate) fn finish(&self) -> Result<(), MirrorError> {
        self.conn
            .pragma_update(None, "user_version", FORMAT_VERSION)
            .map_err(db(&self.path))
    }

    /// Names of the sessions the index knows.
    pub(crate) fn sessions(&self) -> Result<Vec<String>, MirrorError> {
        let mut statement = self
            .conn
            .prepare("SELECT session FROM sessions")
            .map_err(db(&self.path))?;
        statement
            .query_map([], |row| row.get(0))
            .and_then(Iterator::collect)
            .map_err(db(&self.path))
    }

    /// Saves a session's names and sync time, and makes its indexed entries
    /// the ones in its log when they differ in number.
    pub(crate) fn sync_session(
        &mut self,
        session: &str,
        dir: &Path,
        state: &State,
        meta: &SavedMeta,
    ) -> Result<(), MirrorError> {
        let indexed: i64 = self
            .conn
            .query_row(
                "SELECT count(*) FROM entries WHERE session = ?1",
                [session],
                |row| row.get(0),
            )
            .map_err(db(&self.path))?;
        let tx = self.conn.transaction().map_err(db(&self.path))?;
        if u64::try_from(indexed).ok() != Some(state.next_seq) {
            remove_entries(&tx, session).map_err(db(&self.path))?;
            let entries = session::read_log(dir, state.committed_len)?;
            insert_entries(&tx, session, &entries).map_err(db(&self.path))?;
        }
        upsert_session(&tx, session, meta, &state.as_of).map_err(db(&self.path))?;
        tx.commit().map_err(db(&self.path))
    }

    /// Indexes `entries` and saves the session's names and sync time.
    pub(crate) fn add(
        &mut self,
        session: &str,
        meta: &SavedMeta,
        as_of: &str,
        entries: &[MirrorEntry],
    ) -> Result<(), MirrorError> {
        let tx = self.conn.transaction().map_err(db(&self.path))?;
        insert_entries(&tx, session, entries).map_err(db(&self.path))?;
        upsert_session(&tx, session, meta, as_of).map_err(db(&self.path))?;
        tx.commit().map_err(db(&self.path))
    }

    /// Saves a session's names, keeping its sync time.
    pub(crate) fn describe(
        &self,
        session: &str,
        meta: &SavedMeta,
        as_of: &str,
    ) -> Result<(), MirrorError> {
        upsert_session(&self.conn, session, meta, as_of).map_err(db(&self.path))
    }

    pub(crate) fn remove(&mut self, session: &str) -> Result<(), MirrorError> {
        let tx = self.conn.transaction().map_err(db(&self.path))?;
        remove_entries(&tx, session).map_err(db(&self.path))?;
        tx.execute("DELETE FROM sessions WHERE session = ?1", [session])
            .map_err(db(&self.path))?;
        tx.commit().map_err(db(&self.path))
    }

    /// The best entry of each turn that matches, best first. Each hit comes
    /// with the entry's text, for its snippet.
    pub(crate) fn search(&self, search: &Search<'_>) -> Result<Vec<(Hit, String)>, MirrorError> {
        let mut sql = String::from(
            "WITH m AS (
                SELECT e.session AS session, e.seq AS seq, e.turn AS turn,
                       e.written AS written, e.text AS text, bm25(fts) AS score
                FROM fts
                JOIN entries e ON e.id = fts.rowid
                JOIN sessions s ON s.session = e.session
                WHERE fts MATCH ?",
        );
        let mut args = vec![Value::Text(search.expression.to_owned())];
        if !search.channels.is_empty() {
            sql.push_str(" AND s.channel IN (");
            sql.push_str(&vec!["?"; search.channels.len()].join(","));
            sql.push(')');
            args.extend(search.channels.iter().cloned().map(Value::Text));
        }
        for (clause, value) in [
            (" AND s.machine = ?", search.machine),
            (" AND s.agent = ?", search.agent),
            (" AND e.written >= ?", search.since),
            (" AND e.written < ?", search.until),
        ] {
            if let Some(value) = value {
                sql.push_str(clause);
                args.push(Value::Text(value.to_owned()));
            }
        }
        sql.push_str(
            "), r AS (
                SELECT m.*, ROW_NUMBER() OVER (
                    PARTITION BY session, turn ORDER BY score, written DESC, seq
                ) AS rn FROM m)
            SELECT r.session, r.seq, r.turn, r.written, r.text,
                   s.channel, s.channel_name, s.title, s.machine, s.machine_name,
                   s.agent, s.as_of
            FROM r JOIN sessions s ON s.session = r.session
            WHERE r.rn = 1
            ORDER BY r.score, r.written DESC, r.session, r.turn
            LIMIT ? OFFSET ?",
        );
        args.push(Value::Integer(i64::from(search.limit)));
        args.push(Value::Integer(i64::try_from(search.offset).unwrap_or(i64::MAX)));
        let mut statement = self.conn.prepare(&sql).map_err(db(&self.path))?;
        statement
            .query_map(params_from_iter(args), |row| {
                let hit = Hit {
                    session: row.get(0)?,
                    seq: row.get::<_, i64>(1).map(|seq| u64::try_from(seq).unwrap_or(0))?,
                    turn: row.get::<_, i64>(2).map(|turn| u32::try_from(turn).unwrap_or(0))?,
                    at: row.get(3)?,
                    snippet: String::new(),
                    channel: row.get(5)?,
                    channel_name: row.get(6)?,
                    title: row.get(7)?,
                    machine: row.get(8)?,
                    machine_name: row.get(9)?,
                    agent: row.get(10)?,
                    as_of: row.get(11)?,
                };
                Ok((hit, row.get(4)?))
            })
            .and_then(Iterator::collect)
            .map_err(db(&self.path))
    }

    /// Entries of a session after `after` (or from the start) in turns
    /// `first..=last`, at most `limit`.
    pub(crate) fn read(
        &self,
        session: &str,
        after: Option<u64>,
        first: u32,
        last: u32,
        limit: u32,
    ) -> Result<Vec<MirrorEntry>, MirrorError> {
        let after = after.map_or(-1, |seq| i64::try_from(seq).unwrap_or(i64::MAX));
        let mut statement = self
            .conn
            .prepare_cached(
                "SELECT seq, turn, role, text, written FROM entries
                 WHERE session = ?1 AND seq > ?2 AND turn >= ?3 AND turn <= ?4
                 ORDER BY seq LIMIT ?5",
            )
            .map_err(db(&self.path))?;
        statement
            .query_map(
                params![session, after, first, last, limit],
                |row| {
                    Ok(MirrorEntry {
                        seq: row.get::<_, i64>(0).map(|seq| u64::try_from(seq).unwrap_or(0))?,
                        turn: row.get::<_, i64>(1).map(|turn| u32::try_from(turn).unwrap_or(0))?,
                        role: role_of(row.get(2)?),
                        text: row.get(3)?,
                        at: row.get(4)?,
                    })
                },
            )
            .and_then(Iterator::collect)
            .map_err(db(&self.path))
    }
}

fn db(path: &Path) -> impl Fn(rusqlite::Error) -> MirrorError + '_ {
    |source| MirrorError::Database {
        path: path.to_path_buf(),
        source,
    }
}

fn configure(conn: &Connection) -> rusqlite::Result<()> {
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.query_row("PRAGMA journal_mode = WAL", [], |row| row.get::<_, String>(0))?;
    conn.execute_batch(
        "PRAGMA synchronous = NORMAL; PRAGMA cache_size = -8000; PRAGMA temp_store = MEMORY;",
    )
}

/// The existing index, when it is usable as it is.
fn open_existing(path: &Path) -> rusqlite::Result<Option<Connection>> {
    if !path.exists() {
        return Ok(None);
    }
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version != FORMAT_VERSION {
        return Ok(None);
    }
    configure(&conn)?;
    conn.query_row(
        "SELECT (SELECT count(*) FROM sessions) + (SELECT count(*) FROM entries)",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(Some(conn))
}

fn upsert_session(
    conn: &Connection,
    session: &str,
    meta: &SavedMeta,
    as_of: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO sessions (session, channel, channel_name, title, machine, machine_name,
                               agent, archived, as_of)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT (session) DO UPDATE SET
            channel = excluded.channel, channel_name = excluded.channel_name,
            title = excluded.title, machine = excluded.machine,
            machine_name = excluded.machine_name, agent = excluded.agent,
            archived = excluded.archived, as_of = excluded.as_of",
        params![
            session,
            meta.channel,
            meta.channel_name,
            meta.title,
            meta.machine,
            meta.machine_name,
            meta.agent,
            meta.archived,
            as_of
        ],
    )
    .map(drop)
}

fn insert_entries(conn: &Connection, session: &str, entries: &[MirrorEntry]) -> rusqlite::Result<()> {
    let mut insert = conn.prepare_cached(
        "INSERT INTO entries (session, seq, turn, role, written, text)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    let mut index = conn.prepare_cached("INSERT INTO fts (rowid, text) VALUES (?1, ?2)")?;
    for entry in entries {
        insert.execute(params![
            session,
            i64::try_from(entry.seq).unwrap_or(i64::MAX),
            entry.turn,
            role_number(entry.role),
            entry.at,
            entry.text
        ])?;
        index.execute(params![conn.last_insert_rowid(), entry.text])?;
    }
    Ok(())
}

fn remove_entries(conn: &Connection, session: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO fts (fts, rowid, text)
         SELECT 'delete', id, text FROM entries WHERE session = ?1",
        [session],
    )?;
    conn.execute("DELETE FROM entries WHERE session = ?1", [session])
        .map(drop)
}
