//! The database's tables, and opening it.

use std::fs::OpenOptions;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use rusqlite::Connection;

use crate::StoreError;

/// Each step takes the schema from its index to the next version, kept in
/// `PRAGMA user_version`. Steps are only ever added, never changed.
const MIGRATIONS: &[&str] = &[
    "
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
",
    "
    -- When the resumed agent was told it had been interrupted, and how long
    -- its conversation was then, saved before it's told, so a resume a
    -- crash redoes doesn't tell it twice.
    ALTER TABLE interruptions ADD COLUMN told_unix_ms INTEGER;
    ALTER TABLE interruptions ADD COLUMN told_conversation_len INTEGER;
",
    "
    -- Each session's review (PR-26): the review chat's view, notes and
    -- switches, and what the last diff showed, so a restart keeps them.
    CREATE TABLE reviews (
        session TEXT PRIMARY KEY REFERENCES sessions (id) ON DELETE CASCADE,
        updated_unix_ms INTEGER NOT NULL,
        state BLOB,
        state_file TEXT,
        CHECK ((state IS NULL) <> (state_file IS NULL))
    ) STRICT;
",
    "
    -- The views a session's agent prepared (VW-1, VW-14), drafts and
    -- offered ones, with the idempotency keys their calls carried.
    CREATE TABLE views (
        id TEXT PRIMARY KEY,
        session TEXT NOT NULL REFERENCES sessions (id) ON DELETE CASCADE,
        prepared_unix_ms INTEGER NOT NULL,
        updated_unix_ms INTEGER NOT NULL,
        view BLOB,
        view_file TEXT,
        CHECK ((view IS NULL) <> (view_file IS NULL))
    ) STRICT;
    CREATE INDEX views_by_session ON views (session, prepared_unix_ms, id);
",
    "
    -- What a session's agent asked the user (IN-26) and how it was
    -- answered, with the grant the answer made, so a restart keeps the
    -- Inbox and the grants (D96). A session's go with it.
    CREATE TABLE inbox_requests (
        id TEXT PRIMARY KEY,
        session TEXT NOT NULL REFERENCES sessions (id) ON DELETE CASCADE,
        created_unix_ms INTEGER NOT NULL,
        updated_unix_ms INTEGER NOT NULL,
        entry BLOB,
        entry_file TEXT,
        CHECK ((entry IS NULL) <> (entry_file IS NULL))
    ) STRICT;
    CREATE INDEX inbox_requests_by_session ON inbox_requests (session);
",
    "
    -- Channels (CH-1, CH-2, CH-5): their repositories with the state of
    -- each main clone (FC-11-FC-14), and the machines they use. Positions
    -- keep the order the user gave.
    CREATE TABLE channels (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        folder TEXT NOT NULL UNIQUE,
        branch_prefix TEXT NOT NULL,
        default_agent TEXT NOT NULL,
        revision INTEGER NOT NULL,
        created_unix_ms INTEGER NOT NULL
    ) STRICT;
    CREATE INDEX channels_by_age ON channels (created_unix_ms, id);
    CREATE TABLE channel_repos (
        channel TEXT NOT NULL REFERENCES channels (id) ON DELETE CASCADE,
        position INTEGER NOT NULL,
        owner TEXT NOT NULL,
        name TEXT NOT NULL,
        main_state TEXT NOT NULL CHECK (main_state IN ('cloning', 'ready', 'failed')),
        main_error TEXT,
        PRIMARY KEY (channel, owner, name)
    ) STRICT;
    CREATE TABLE channel_machines (
        channel TEXT NOT NULL REFERENCES channels (id) ON DELETE CASCADE,
        machine TEXT NOT NULL,
        position INTEGER NOT NULL,
        PRIMARY KEY (channel, machine)
    ) STRICT;

    -- Where a session's start stands (FC-1, FC-3, SE-4, PER-11, PER-27):
    -- its clones and the step it reached, so a crash part way resumes it.
    CREATE TABLE session_details (
        session TEXT PRIMARY KEY REFERENCES sessions (id) ON DELETE CASCADE,
        model TEXT,
        first_message TEXT,
        start_step TEXT NOT NULL CHECK (start_step IN ('recorded', 'cloned', 'launching', 'done')),
        operation TEXT,
        revision INTEGER NOT NULL
    ) STRICT;
    CREATE TABLE session_repos (
        session TEXT NOT NULL REFERENCES sessions (id) ON DELETE CASCADE,
        position INTEGER NOT NULL,
        owner TEXT NOT NULL,
        name TEXT NOT NULL,
        branch TEXT NOT NULL,
        path BLOB NOT NULL,
        PRIMARY KEY (session, owner, name)
    ) STRICT;
",
    "
    -- The Agent access rules the user set (IN-17, IN-21, ST-4): each
    -- channel's and the defaults new channels start with, as the daemon
    -- encodes them, so a restart keeps them (D96).
    CREATE TABLE access_channel_rules (
        channel TEXT PRIMARY KEY,
        revision INTEGER NOT NULL,
        rules BLOB NOT NULL
    ) STRICT;
    CREATE TABLE access_default_rules (
        id INTEGER PRIMARY KEY CHECK (id = 1),
        revision INTEGER NOT NULL,
        rules BLOB NOT NULL
    ) STRICT;
",
    "
    -- Script tokens (SEC-14): a long-lived credential for a script, limited
    -- to the capabilities and channels it was made with. Only the hash of the
    -- token is kept, never the token; the entry is the daemon's encoding of
    -- what the token may do and who it is for.
    CREATE TABLE script_tokens (
        id TEXT PRIMARY KEY,
        hash TEXT NOT NULL UNIQUE,
        entry BLOB NOT NULL,
        expires_unix_ms INTEGER NOT NULL,
        last_used_unix_ms INTEGER NOT NULL
    ) STRICT;
",
];

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
            Some(folder) if !existed && !folder.as_os_str().is_empty() => {
                humata2_durable::sync_dir(folder)
            }
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
