//! The user's pins and VIP marks on the host (NV-20, NV-24 to NV-26).
//!
//! A thing (a session, a channel, a machine, a PR review or a board item)
//! can be pinned, marked VIP, both or neither. Pinned things keep the order
//! the user gave them, and a new pin goes at the end (NV-25). The marks
//! live in one SQLite file of their own, in WAL mode, every write a
//! transaction, so they are whole after a crash. They are user data:
//! nothing else can rebuild them.
//!
//! One number, the revision, counts the changes ever made to the file. It
//! starts at 0 in a new file, goes up by at least one with every call that
//! changes a mark, the order or a pinned thing's event (NV-21), stays as
//! it is for a call that changes
//! nothing, and survives closing and reopening the file. A client that
//! reorders says which revision it saw, so a list that changed under it is
//! never reordered blind.
//!
//! Archiving a thing keeps its marks (NV-26): there is nothing to call.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

/// Something the user can pin or mark VIP, by its kind and its id. The
/// five kinds are separate: a session and a channel with the same id are
/// two things.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Thing {
    Session(String),
    Channel(String),
    Machine(String),
    PrReview(String),
    BoardItem(String),
}

/// One thing's marks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mark {
    pub thing: Thing,
    pub pinned: bool,
    pub vip: bool,
    /// The file's revision when this was read, or just after the write
    /// that returned it: the same number [`Marks::pinned`] gives.
    pub revision: u64,
}

/// What a pinned thing shows in the Pinned section (NV-21): "needs
/// input", "finished", "failed" and the like, and when it happened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub text: String,
    pub at_unix_ms: i64,
}

/// What went wrong reading or changing the marks.
#[derive(Debug, thiserror::Error)]
pub enum MarksError {
    /// The file couldn't be opened, read or written, or isn't a marks
    /// file. It is left as it was.
    #[error(
        "couldn't read or write Humata's pins, VIP marks and read marks in {}; check that the file belongs to you and that the disk is mounted and has free space, then try again. Don't delete the file: it holds every pin and VIP mark",
        path.display()
    )]
    Database {
        path: PathBuf,
        #[source]
        source: rusqlite::Error,
    },

    /// A new order didn't list every pinned thing exactly once. Nothing
    /// was changed.
    #[error(
        "the new order must list every pinned item once: {} are missing, {} are not pinned and {} are listed more than once. Read the pinned list again and send each of its items once",
        missing.len(),
        not_pinned.len(),
        repeated.len()
    )]
    OrderMismatch {
        /// Pinned things the order left out, in their current order.
        missing: Vec<Thing>,
        /// Things in the order that aren't pinned, each once, as given.
        not_pinned: Vec<Thing>,
        /// Things the order lists more than once, each once, as given.
        repeated: Vec<Thing>,
    },

    /// The marks changed after the caller read them. Nothing was changed.
    #[error(
        "the pinned list changed after it was read (it is now at revision {current}); read it again and reorder what it shows now"
    )]
    Stale {
        /// The file's revision now.
        current: u64,
    },
}

/// The marks file, open. Clones share the one file; all of them can be
/// used from any thread.
#[derive(Clone)]
pub struct Marks {
    path: Arc<PathBuf>,
    db: Arc<Mutex<Connection>>,
}

const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS marks (
        kind TEXT NOT NULL,
        id TEXT NOT NULL,
        position INTEGER,
        vip INTEGER NOT NULL DEFAULT 0,
        event TEXT NOT NULL DEFAULT '',
        event_at INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (kind, id)
    ) WITHOUT ROWID;
    CREATE TABLE IF NOT EXISTS meta (
        key TEXT PRIMARY KEY,
        value INTEGER NOT NULL
    ) WITHOUT ROWID;
    INSERT OR IGNORE INTO meta (key, value) VALUES ('revision', 0);
    CREATE TABLE IF NOT EXISTS unread (
        session TEXT PRIMARY KEY
    ) WITHOUT ROWID;
";

fn split(thing: &Thing) -> (&'static str, &str) {
    match thing {
        Thing::Session(id) => ("session", id),
        Thing::Channel(id) => ("channel", id),
        Thing::Machine(id) => ("machine", id),
        Thing::PrReview(id) => ("pr_review", id),
        Thing::BoardItem(id) => ("board_item", id),
    }
}

fn join(kind: &str, id: String) -> Result<Thing, rusqlite::Error> {
    Ok(match kind {
        "session" => Thing::Session(id),
        "channel" => Thing::Channel(id),
        "machine" => Thing::Machine(id),
        "pr_review" => Thing::PrReview(id),
        "board_item" => Thing::BoardItem(id),
        other => {
            return Err(rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                format!("unknown kind {other:?}").into(),
            ));
        }
    })
}

/// Adds the event columns to a marks table made before they were kept.
fn add_event_columns(conn: &Connection) -> Result<(), rusqlite::Error> {
    let mut stmt = conn.prepare("SELECT name FROM pragma_table_info('marks')")?;
    let columns: HashSet<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<Result<_, _>>()?;
    if !columns.contains("event") {
        conn.execute_batch("ALTER TABLE marks ADD COLUMN event TEXT NOT NULL DEFAULT ''")?;
    }
    if !columns.contains("event_at") {
        conn.execute_batch("ALTER TABLE marks ADD COLUMN event_at INTEGER NOT NULL DEFAULT 0")?;
    }
    Ok(())
}

fn revision_of(conn: &Connection) -> Result<u64, rusqlite::Error> {
    let value: i64 = conn.query_row("SELECT value FROM meta WHERE key = 'revision'", [], |r| r.get(0))?;
    Ok(u64::try_from(value).unwrap_or(0))
}

fn bump(tx: &Transaction<'_>) -> Result<u64, rusqlite::Error> {
    tx.execute("UPDATE meta SET value = value + 1 WHERE key = 'revision'", [])?;
    revision_of(tx)
}

/// The pinned things in order, with their VIP flags.
fn pinned_in(conn: &Connection) -> Result<Vec<(Thing, bool)>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT kind, id, vip FROM marks WHERE position IS NOT NULL ORDER BY position",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)? != 0)))?;
    let mut out = Vec::new();
    for row in rows {
        let (kind, id, vip) = row?;
        out.push((join(&kind, id)?, vip));
    }
    Ok(out)
}

fn flags(conn: &Connection, thing: &Thing) -> Result<(bool, bool), rusqlite::Error> {
    let (kind, id) = split(thing);
    let row: Option<(Option<i64>, i64)> = conn
        .query_row(
            "SELECT position, vip FROM marks WHERE kind = ?1 AND id = ?2",
            params![kind, id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    Ok(row.map_or((false, false), |(position, vip)| (position.is_some(), vip != 0)))
}

impl Marks {
    /// Opens the marks file at `path`, creating it with nothing marked and
    /// revision 0 if it isn't there. A file that is there but isn't a
    /// marks database is an error and is left untouched.
    pub fn open(path: &Path) -> Result<Marks, MarksError> {
        let fail = |source| MarksError::Database { path: path.to_owned(), source };
        let conn = Connection::open(path).map_err(fail)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             PRAGMA fullfsync = ON;
             PRAGMA checkpoint_fullfsync = ON;
             PRAGMA busy_timeout = 5000;
             PRAGMA cache_size = -2000;",
        )
        .map_err(fail)?;
        conn.execute_batch(SCHEMA).map_err(fail)?;
        add_event_columns(&conn).map_err(fail)?;
        revision_of(&conn).map_err(fail)?;
        Ok(Marks { path: Arc::new(path.to_owned()), db: Arc::new(Mutex::new(conn)) })
    }

    fn lock(&self) -> MutexGuard<'_, Connection> {
        self.db.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn fail(&self, source: rusqlite::Error) -> MarksError {
        MarksError::Database { path: self.path.as_ref().clone(), source }
    }

    /// Runs `body` in one immediate transaction and commits it.
    fn write<T>(
        &self,
        body: impl FnOnce(&Transaction<'_>) -> Result<Result<T, MarksError>, rusqlite::Error>,
    ) -> Result<T, MarksError> {
        let mut conn = self.lock();
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| self.fail(e))?;
        match body(&tx).map_err(|e| self.fail(e))? {
            Ok(value) => {
                tx.commit().map_err(|e| self.fail(e))?;
                Ok(value)
            }
            // Dropping the transaction rolls it back.
            Err(error) => Err(error),
        }
    }

    /// Sets a thing's marks and returns them as they now are. `None`
    /// leaves that flag as it is.
    ///
    /// A thing that becomes pinned goes at the end of the order (NV-25); a
    /// thing already pinned keeps its place. Unpinning takes it out of the
    /// order, so pinning it again later puts it at the end. A thing
    /// neither pinned nor VIP is not kept at all. The revision goes up if
    /// a flag changed and stays if none did.
    pub fn set(
        &self,
        thing: &Thing,
        pinned: Option<bool>,
        vip: Option<bool>,
    ) -> Result<Mark, MarksError> {
        let (kind, id) = split(thing);
        self.write(|tx| {
            let (was_pinned, was_vip) = flags(tx, thing)?;
            let now_pinned = pinned.unwrap_or(was_pinned);
            let now_vip = vip.unwrap_or(was_vip);
            let mut revision = revision_of(tx)?;
            if (now_pinned, now_vip) != (was_pinned, was_vip) {
                if !now_pinned && !now_vip {
                    tx.execute("DELETE FROM marks WHERE kind = ?1 AND id = ?2", params![kind, id])?;
                } else {
                    let position: Option<i64> = if !now_pinned {
                        None
                    } else if was_pinned {
                        tx.query_row(
                            "SELECT position FROM marks WHERE kind = ?1 AND id = ?2",
                            params![kind, id],
                            |r| r.get(0),
                        )?
                    } else {
                        Some(tx.query_row(
                            "SELECT COALESCE(MAX(position), -1) + 1 FROM marks",
                            [],
                            |r| r.get(0),
                        )?)
                    };
                    tx.execute(
                        "INSERT INTO marks (kind, id, position, vip) VALUES (?1, ?2, ?3, ?4)
                         ON CONFLICT (kind, id) DO UPDATE SET position = ?3, vip = ?4",
                        params![kind, id, position, i64::from(now_vip)],
                    )?;
                    if now_pinned && !was_pinned {
                        // A new pin shows no event until the next one comes.
                        tx.execute(
                            "UPDATE marks SET event = '', event_at = 0 WHERE kind = ?1 AND id = ?2",
                            params![kind, id],
                        )?;
                    }
                }
                revision = bump(tx)?;
            }
            Ok(Ok(Mark { thing: thing.clone(), pinned: now_pinned, vip: now_vip, revision }))
        })
    }

    /// A thing's marks. A thing never marked, or forgotten, has both flags
    /// false.
    pub fn get(&self, thing: &Thing) -> Result<Mark, MarksError> {
        let conn = self.lock();
        let read = || -> Result<Mark, rusqlite::Error> {
            let (pinned, vip) = flags(&conn, thing)?;
            Ok(Mark { thing: thing.clone(), pinned, vip, revision: revision_of(&conn)? })
        };
        read().map_err(|e| self.fail(e))
    }

    /// The pinned things in the user's order, each with its VIP flag, and
    /// the revision they were read at.
    pub fn pinned(&self) -> Result<(Vec<Mark>, u64), MarksError> {
        let mut conn = self.lock();
        let read = |conn: &mut Connection| -> Result<(Vec<Mark>, u64), rusqlite::Error> {
            let tx = conn.transaction()?;
            let revision = revision_of(&tx)?;
            let marks = pinned_in(&tx)?
                .into_iter()
                .map(|(thing, vip)| Mark { thing, pinned: true, vip, revision })
                .collect();
            Ok((marks, revision))
        };
        read(&mut conn).map_err(|e| self.fail(e))
    }

    /// Every VIP thing, pinned or not, in no particular order.
    pub fn vips(&self) -> Result<Vec<Thing>, MarksError> {
        let conn = self.lock();
        let read = || -> Result<Vec<Thing>, rusqlite::Error> {
            let mut stmt = conn.prepare("SELECT kind, id FROM marks WHERE vip != 0")?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            let mut out = Vec::new();
            for row in rows {
                let (kind, id) = row?;
                out.push(join(&kind, id)?);
            }
            Ok(out)
        };
        read().map_err(|e| self.fail(e))
    }

    /// Puts the pinned things in a new order and returns the list as
    /// [`Marks::pinned`] would.
    ///
    /// `expected` is the revision the caller read the list at. If the file
    /// is at another revision the call fails with [`MarksError::Stale`],
    /// whatever `order` holds: the caller's list is out of date, and it
    /// should read again. `None` skips that check. Otherwise `order` must
    /// hold every pinned thing exactly once and nothing else, or the call
    /// fails with [`MarksError::OrderMismatch`]. A failed call changes
    /// nothing.
    pub fn reorder(
        &self,
        order: &[Thing],
        expected: Option<u64>,
    ) -> Result<(Vec<Mark>, u64), MarksError> {
        self.write(|tx| {
            let current = revision_of(tx)?;
            if let Some(expected) = expected
                && expected != current
            {
                return Ok(Err(MarksError::Stale { current }));
            }
            let pinned = pinned_in(tx)?;
            let pinned_set: HashSet<&Thing> = pinned.iter().map(|(t, _)| t).collect();
            let mut seen = HashSet::new();
            let mut repeated: Vec<Thing> = Vec::new();
            let mut not_pinned: Vec<Thing> = Vec::new();
            for thing in order {
                if !seen.insert(thing) {
                    if !repeated.contains(thing) {
                        repeated.push(thing.clone());
                    }
                } else if !pinned_set.contains(thing) {
                    not_pinned.push(thing.clone());
                }
            }
            let missing: Vec<Thing> = pinned
                .iter()
                .map(|(t, _)| t)
                .filter(|t| !seen.contains(*t))
                .cloned()
                .collect();
            if !(missing.is_empty() && not_pinned.is_empty() && repeated.is_empty()) {
                return Ok(Err(MarksError::OrderMismatch { missing, not_pinned, repeated }));
            }
            let unchanged = pinned.iter().map(|(t, _)| t).eq(order.iter());
            let mut revision = current;
            if !unchanged {
                for (position, thing) in order.iter().enumerate() {
                    let (kind, id) = split(thing);
                    tx.execute(
                        "UPDATE marks SET position = ?3 WHERE kind = ?1 AND id = ?2",
                        params![kind, id, i64::try_from(position).unwrap_or(i64::MAX)],
                    )?;
                }
                revision = bump(tx)?;
            }
            let vips: std::collections::HashMap<&Thing, bool> = pinned.iter().map(|(t, v)| (t, *v)).collect();
            let marks = order
                .iter()
                .map(|thing| Mark {
                    thing: thing.clone(),
                    pinned: true,
                    vip: vips.get(thing).copied().unwrap_or(false),
                    revision,
                })
                .collect();
            Ok(Ok((marks, revision)))
        })
    }

    /// Sets the event a pinned thing shows (NV-21), or clears it with
    /// `None`. The event is part of the list, so a new one raises the
    /// revision: the new revision is returned. The same event again, or
    /// any event for a thing that isn't pinned, changes nothing and
    /// returns `None`; a thing pinned later starts with no event.
    pub fn set_event(&self, thing: &Thing, event: Option<&Event>) -> Result<Option<u64>, MarksError> {
        let (kind, id) = split(thing);
        let (text, at) = event.map_or(("", 0), |event| (event.text.as_str(), event.at_unix_ms));
        self.write(|tx| {
            let updated = tx.execute(
                "UPDATE marks SET event = ?3, event_at = ?4
                 WHERE kind = ?1 AND id = ?2 AND position IS NOT NULL
                   AND (event != ?3 OR event_at != ?4)",
                params![kind, id, text, at],
            )?;
            Ok(Ok(if updated > 0 { Some(bump(tx)?) } else { None }))
        })
    }

    /// The events the pinned things show, by thing. A pinned thing with no
    /// event isn't there.
    pub fn events(&self) -> Result<HashMap<Thing, Event>, MarksError> {
        let conn = self.lock();
        let read = || -> Result<HashMap<Thing, Event>, rusqlite::Error> {
            let mut stmt = conn.prepare(
                "SELECT kind, id, event, event_at FROM marks WHERE position IS NOT NULL AND event != ''",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                ))
            })?;
            let mut out = HashMap::new();
            for row in rows {
                let (kind, id, text, at_unix_ms) = row?;
                out.insert(join(&kind, id)?, Event { text, at_unix_ms });
            }
            Ok(out)
        };
        read().map_err(|e| self.fail(e))
    }

    /// Drops a thing's pin and VIP mark, for a thing that was deleted or
    /// forgotten (NV-26). The other pins keep their order. Forgetting a
    /// thing with no marks does nothing and is not an error.
    pub fn forget(&self, thing: &Thing) -> Result<(), MarksError> {
        let (kind, id) = split(thing);
        self.write(|tx| {
            let removed = tx.execute("DELETE FROM marks WHERE kind = ?1 AND id = ?2", params![kind, id])?;
            if removed > 0 {
                bump(tx)?;
            }
            if kind == "session" {
                tx.execute("DELETE FROM unread WHERE session = ?1", params![id])?;
            }
            Ok(Ok(()))
        })
    }

    /// Marks `session` unread (CH-16) or read (CH-17), and says whether
    /// that changed it. Read marks aren't part of the pinned list, so the
    /// revision stays.
    pub fn set_unread(&self, session: &str, unread: bool) -> Result<bool, MarksError> {
        self.write(|tx| {
            let changed = if unread {
                tx.execute("INSERT OR IGNORE INTO unread (session) VALUES (?1)", params![session])?
            } else {
                tx.execute("DELETE FROM unread WHERE session = ?1", params![session])?
            };
            Ok(Ok(changed > 0))
        })
    }

    /// The sessions marked unread, by id.
    pub fn unread(&self) -> Result<Vec<String>, MarksError> {
        let conn = self.lock();
        let read = || -> Result<Vec<String>, rusqlite::Error> {
            let mut stmt = conn.prepare("SELECT session FROM unread ORDER BY session")?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            rows.collect()
        };
        read().map_err(|e| self.fail(e))
    }
}
