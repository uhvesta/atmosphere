//! The database's tables, and opening it.

use std::fs::OpenOptions;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use rusqlite::Connection;

use crate::StoreError;

/// Each step takes the schema from its index to the next version, kept in
/// `PRAGMA user_version`. Steps are only ever added, never changed.
const MIGRATIONS: &[&str] = &["
    CREATE TABLE sessions (
        id TEXT PRIMARY KEY,
        channel TEXT NOT NULL,
        machine TEXT NOT NULL,
        title TEXT NOT NULL,
        agent TEXT NOT NULL,
        folder BLOB NOT NULL,
        created_unix_ms INTEGER NOT NULL
    ) STRICT;
    CREATE INDEX sessions_by_age ON sessions (created_unix_ms, id);

    CREATE TABLE tabs (
        id TEXT PRIMARY KEY,
        session TEXT REFERENCES sessions (id) ON DELETE CASCADE,
        kind TEXT NOT NULL CHECK (kind IN ('agent', 'shell')),
        title TEXT NOT NULL,
        holder_socket BLOB NOT NULL,
        holder_pid INTEGER NOT NULL,
        created_unix_ms INTEGER NOT NULL
    ) STRICT;
    CREATE INDEX tabs_by_session ON tabs (session);
    CREATE INDEX tabs_by_age ON tabs (created_unix_ms, id);

    CREATE TABLE pointer_reports (
        session TEXT PRIMARY KEY REFERENCES sessions (id) ON DELETE CASCADE,
        reported_unix_ms INTEGER NOT NULL
    ) STRICT;
    CREATE TABLE repo_pointers (
        session TEXT NOT NULL REFERENCES pointer_reports (session) ON DELETE CASCADE,
        position INTEGER NOT NULL,
        owner TEXT NOT NULL,
        name TEXT NOT NULL,
        branch TEXT NOT NULL,
        sha TEXT NOT NULL,
        pushed INTEGER NOT NULL,
        PRIMARY KEY (session, position)
    ) STRICT;
    CREATE TABLE dirty_files (
        session TEXT NOT NULL,
        position INTEGER NOT NULL,
        n INTEGER NOT NULL,
        path TEXT NOT NULL,
        PRIMARY KEY (session, position, n),
        FOREIGN KEY (session, position)
            REFERENCES repo_pointers (session, position) ON DELETE CASCADE
    ) STRICT;

    CREATE TABLE journal (
        seq INTEGER PRIMARY KEY,
        kind TEXT NOT NULL,
        written_unix_ms INTEGER NOT NULL,
        payload BLOB,
        payload_file TEXT,
        CHECK ((payload IS NULL) <> (payload_file IS NULL))
    ) STRICT;
    CREATE TABLE journal_state (
        id INTEGER PRIMARY KEY CHECK (id = 1),
        next_seq INTEGER NOT NULL,
        acked INTEGER NOT NULL
    ) STRICT;
    INSERT INTO journal_state (id, next_seq, acked) VALUES (1, 1, 0);

    CREATE TABLE agents (
        session TEXT PRIMARY KEY REFERENCES sessions (id) ON DELETE CASCADE,
        tab TEXT NOT NULL,
        state TEXT NOT NULL,
        holder_pid INTEGER,
        exit_code INTEGER,
        failure TEXT,
        generation INTEGER NOT NULL,
        updated_unix_ms INTEGER NOT NULL,
        launch BLOB,
        launch_file TEXT,
        CHECK ((launch IS NULL) <> (launch_file IS NULL))
    ) STRICT;
    CREATE INDEX agents_by_state ON agents (state);
    CREATE TABLE interruptions (
        session TEXT NOT NULL REFERENCES agents (session) ON DELETE CASCADE,
        n INTEGER NOT NULL,
        at_unix_ms INTEGER NOT NULL,
        output_saved_through INTEGER NOT NULL,
        output_resumes_at INTEGER NOT NULL,
        saved_tail_damaged INTEGER NOT NULL,
        PRIMARY KEY (session, n)
    ) STRICT;
", "
    -- When the resumed agent was told it had been interrupted, and how long
    -- its conversation was then, saved before it's told, so a resume a
    -- crash redoes doesn't tell it twice.
    ALTER TABLE interruptions ADD COLUMN told_unix_ms INTEGER;
    ALTER TABLE interruptions ADD COLUMN told_conversation_len INTEGER;
"];

/// The schema this build writes and reads.
pub const SCHEMA_VERSION: i64 = MIGRATIONS.len() as i64;

/// Opens the database at `path`, readable only by this user, and brings
/// its schema up to this build's.
pub(crate) fn open(path: &Path) -> Result<Connection, StoreError> {
    let opened = |source| StoreError::Open {
        path: path.to_owned(),
        source,
    };
    // SQLite gives the -wal and -shm files the database's own mode. A new
    // one's folder is synced, so the file outlasts a power cut.
    let made = (|| {
        let existed = path.exists();
        let mut options = OpenOptions::new();
        options.create(true).append(true).mode(0o600);
        humata2_durable::File::open(&options, path)?;
        match path.parent() {
            Some(folder) if !existed && !folder.as_os_str().is_empty() => humata2_durable::sync_dir(folder),
            _ => Ok(()),
        }
    })();
    made.map_err(|source| StoreError::Write {
        path: path.to_owned(),
        source,
    })?;
    #[cfg(feature = "faults")]
    humata2_durable::install_sqlite();
    let mut connection = Connection::open(path).map_err(opened)?;
    // FULL: a commit returns once it's on disk, so what the host acks and
    // what a resume reads survive a power cut. fullfsync makes that true
    // on macOS too, where fsync alone leaves it in the drive's cache.
    connection
        .execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             PRAGMA fullfsync = ON;
             PRAGMA checkpoint_fullfsync = ON;
             PRAGMA busy_timeout = 5000;",
        )
        .map_err(opened)?;
    migrate(&mut connection, path)?;
    Ok(connection)
}

fn migrate(connection: &mut Connection, path: &Path) -> Result<(), StoreError> {
    let found: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|source| StoreError::Open {
            path: path.to_owned(),
            source,
        })?;
    let supported = SCHEMA_VERSION;
    if found > supported {
        return Err(StoreError::SchemaTooNew {
            path: path.to_owned(),
            found,
            supported,
        });
    }
    let failed = |source| StoreError::Migrate {
        path: path.to_owned(),
        from: found,
        to: supported,
        source,
    };
    // All steps in one transaction: a crash part way leaves the schema
    // as it was.
    let transaction = connection.transaction().map_err(failed)?;
    for (step, sql) in MIGRATIONS.iter().enumerate() {
        if (step as i64) < found {
            continue;
        }
        transaction.execute_batch(sql).map_err(failed)?;
    }
    if found < supported {
        transaction
            .pragma_update(None, "user_version", supported)
            .map_err(failed)?;
    }
    transaction.commit().map_err(failed)
}
