//! The database's tables, and opening it.

use std::fs::OpenOptions;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use rusqlite::{Connection, TransactionBehavior};

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
    "
    -- The user's review comments for a session's agent and the batches
    -- typed into its terminal (SM-7-SM-21, D138): a row per comment, so a
    -- change writes only the rows it changed. The comment and the batch are
    -- the bytes the daemon encodes, kept as given; the number is the one in
    -- the id (`c-<n>`, `d-<n>`), for order. delivery_state and state:
    -- 0 in no batch, 1 sending, 2 unconfirmed. A session's go with it.
    CREATE TABLE comments (
        session TEXT NOT NULL REFERENCES sessions (id) ON DELETE CASCADE,
        id TEXT NOT NULL,
        n INTEGER NOT NULL,
        archived INTEGER NOT NULL DEFAULT 0,
        delivery_state INTEGER NOT NULL DEFAULT 0,
        updated_unix_ms INTEGER NOT NULL,
        comment BLOB NOT NULL,
        PRIMARY KEY (session, id)
    ) STRICT;
    CREATE INDEX comments_session_archived ON comments (session, archived);
    CREATE TABLE comment_deliveries (
        session TEXT NOT NULL REFERENCES sessions (id) ON DELETE CASCADE,
        key TEXT NOT NULL,
        n INTEGER NOT NULL,
        state INTEGER NOT NULL,
        delivery BLOB NOT NULL,
        PRIMARY KEY (session, key)
    ) STRICT;
    CREATE INDEX comment_deliveries_by_state ON comment_deliveries (state);
    CREATE TABLE comment_numbers (
        session TEXT PRIMARY KEY REFERENCES sessions (id) ON DELETE CASCADE,
        last_comment INTEGER NOT NULL,
        last_delivery INTEGER NOT NULL
    ) STRICT;
",
    "
    -- Conversations a session left behind when it started fresh (PER-15, D143):
    -- never deleted, newest last.
    CREATE TABLE previous_conversations (
        session_id TEXT NOT NULL,
        conversation_id TEXT NOT NULL,
        path BLOB NOT NULL,
        ended_unix_ms INTEGER NOT NULL,
        PRIMARY KEY (session_id, conversation_id)
    ) STRICT;
",
    "
    -- A start fresh's first message until it is typed into the conversation
    -- it was made for (PER-15, D153): one per session, the newest.
    CREATE TABLE fresh_seeds (
        session_id TEXT PRIMARY KEY,
        conversation_id TEXT NOT NULL,
        seed TEXT NOT NULL,
        created_unix_ms INTEGER NOT NULL
    ) STRICT;
",
    "
    -- Board items (BD-1..BD-16, SL-6..SL-8). No foreign keys: items outlive
    -- the sessions and channels they name.
    CREATE TABLE board_prefixes (
        owner TEXT PRIMARY KEY,
        prefix TEXT NOT NULL UNIQUE
    ) STRICT;
    CREATE TABLE board_counters (
        prefix TEXT PRIMARY KEY,
        last INTEGER NOT NULL
    ) STRICT;
    CREATE TABLE board_items (
        key TEXT PRIMARY KEY,
        board_kind INTEGER NOT NULL,
        board_id TEXT NOT NULL,
        channel TEXT NOT NULL,
        title TEXT NOT NULL,
        description TEXT NOT NULL,
        col INTEGER NOT NULL,
        rank TEXT NOT NULL,
        created_by TEXT NOT NULL,
        created_unix_ms INTEGER NOT NULL,
        updated_unix_ms INTEGER NOT NULL,
        revision INTEGER NOT NULL,
        done_unix_ms INTEGER,
        deleted INTEGER NOT NULL DEFAULT 0
    ) STRICT;
    CREATE INDEX board_items_by_board ON board_items (board_kind, board_id, deleted);
    CREATE INDEX board_items_by_channel ON board_items (channel, deleted);
    CREATE TABLE board_item_sessions (
        item TEXT NOT NULL REFERENCES board_items (key),
        session TEXT NOT NULL,
        n INTEGER NOT NULL,
        PRIMARY KEY (item, session)
    ) STRICT;
    CREATE TABLE board_item_deps (
        item TEXT NOT NULL REFERENCES board_items (key),
        depends_on TEXT NOT NULL,
        n INTEGER NOT NULL,
        PRIMARY KEY (item, depends_on)
    ) STRICT;
    CREATE TABLE board_history (
        item TEXT NOT NULL REFERENCES board_items (key),
        n INTEGER NOT NULL,
        at_unix_ms INTEGER NOT NULL,
        actor TEXT NOT NULL,
        kind INTEGER NOT NULL,
        field TEXT NOT NULL,
        old_value TEXT NOT NULL,
        new_value TEXT NOT NULL,
        lost INTEGER NOT NULL,
        revision INTEGER NOT NULL,
        cause TEXT NOT NULL,
        PRIMARY KEY (item, n)
    ) STRICT;
",
    "
    -- The last status and pointers each machine reported for each session that
    -- lives on it (SL-3, PER-27), kept on the host so a restart still shows
    -- them. No foreign key: the host's session row may be gone while a report
    -- for it is still on its way. `reported` is the API's SessionStatus;
    -- `problem` and `pointers` are the messages the daemon encodes.
    CREATE TABLE remote_session_reports (
        machine TEXT NOT NULL,
        session TEXT NOT NULL,
        reported INTEGER NOT NULL,
        exit_code INTEGER NOT NULL,
        problem BLOB,
        pointers BLOB NOT NULL,
        pointers_at_ms INTEGER,
        reported_unix_ms INTEGER NOT NULL,
        last_seen_unix_ms INTEGER,
        PRIMARY KEY (machine, session)
    ) STRICT;
",
    "
    -- Where each fork came from (FK-1..FK-3). Never rewritten: a session shows
    -- under its nearest ancestor that isn't archived (FK-13). No foreign key: a
    -- parent may be deleted while its forks live on. `kind` is the API's
    -- ForkKind; `before_turn` 0 means from now.
    CREATE TABLE fork_origins (
        session_id TEXT PRIMARY KEY,
        parent TEXT NOT NULL,
        kind INTEGER NOT NULL,
        before_turn INTEGER NOT NULL,
        created_unix_ms INTEGER NOT NULL
    ) STRICT;
    CREATE INDEX fork_origins_by_parent ON fork_origins (parent);
    -- Archived sessions (MV-7, LD-2, MV-15, D110, D170). Written before the
    -- agent stops, so a restart finishes an archive cut short. `state` 0
    -- archiving, 1 archived (clones wait for the undo window), 2 freed.
    -- `summary` is the API's ArchiveSummary as the daemon encodes it.
    CREATE TABLE archives (
        session_id TEXT PRIMARY KEY,
        archived_unix_ms INTEGER NOT NULL,
        state INTEGER NOT NULL,
        summary BLOB NOT NULL,
        free_after_unix_ms INTEGER NOT NULL
    ) STRICT;
",
    "
    -- What the agent's own hooks last said about its turn (D169): the API's
    -- TurnState and when the hook's report was made (unix ns), or NULL when
    -- none was heard since the agent launched or resumed (the upsert clears
    -- both then).
    ALTER TABLE agents ADD COLUMN hook_turn INTEGER;
    ALTER TABLE agents ADD COLUMN hook_turn_at INTEGER;
",
    "
    -- Step 16. Desk sessions (DK-2, DK-4, DK-11, D182): what only the
    -- Desk knows about a session whose channel is \"desk\". kind 1 is a
    -- scratch folder under <root>/desk, 2 a folder the user picked. A row
    -- with deleted_unix_ms is a deleted session, hidden everywhere.
    CREATE TABLE desk_sessions (
        session_id TEXT PRIMARY KEY REFERENCES sessions (id) ON DELETE CASCADE,
        kind INTEGER NOT NULL,
        kept INTEGER NOT NULL,
        boards INTEGER NOT NULL,
        channels_and_machines INTEGER NOT NULL,
        last_used_unix_ms INTEGER NOT NULL,
        closed_unix_ms INTEGER,
        deleted_unix_ms INTEGER,
        held_work TEXT NOT NULL
    ) STRICT;
",
    "
    -- Step 17. Settings the user set (ST-13, D196): one row per key; no
    -- row means the default. A channel's own branch prefix may be empty,
    -- so branch_prefix_set says whether the channel has one at all.
    CREATE TABLE settings (
        key TEXT PRIMARY KEY,
        value TEXT NOT NULL,
        revision INTEGER NOT NULL
    ) STRICT;
    ALTER TABLE channels ADD COLUMN branch_prefix_set INTEGER NOT NULL DEFAULT 0;
    UPDATE channels SET branch_prefix_set = 1 WHERE branch_prefix <> '';
",
    "
    -- How far the host has applied each machine's Inbox journal (D226): one
    -- mark per machine, written with the Inbox row the entry made.
    CREATE TABLE machine_journal (
        machine TEXT PRIMARY KEY,
        applied_through INTEGER NOT NULL
    ) STRICT;
",
    "
    -- The host's last answers to a machine's agents' reads (AWAY T10d, D227):
    -- by session, method and the sha256 of the request's bytes, with when the
    -- host answered. A machine whose host is away answers the same read from
    -- here. A session keeps its newest 64; they go with the session.
    CREATE TABLE last_copies (
        session TEXT NOT NULL REFERENCES sessions (id) ON DELETE CASCADE,
        method TEXT NOT NULL,
        request_hash BLOB NOT NULL,
        response BLOB NOT NULL,
        answered_unix_ms INTEGER NOT NULL,
        PRIMARY KEY (session, method, request_hash)
    ) STRICT;
",
    "
    -- Step 20. What the host left out of a machine agent's replayed board
    -- writes (AW-9, D267): one summary per session, replaced by a later
    -- link-up's. Names are newline-joined (item keys, one-line titles).
    CREATE TABLE away_left_out (
        session TEXT PRIMARY KEY REFERENCES sessions (id) ON DELETE CASCADE,
        link_up_unix_ms INTEGER NOT NULL,
        count INTEGER NOT NULL,
        items TEXT NOT NULL,
        not_made TEXT NOT NULL,
        refused INTEGER NOT NULL,
        at_unix_ms INTEGER NOT NULL
    ) STRICT;
",
    "
    -- Step 21. Which holder process a tab has had (D271): bumped each time
    -- a tab gets a new one. A tab saved before this reads as generation 0.
    ALTER TABLE tabs ADD COLUMN holder_generation INTEGER NOT NULL DEFAULT 0;
",
    "
    -- Step 22. What a tab's screen has answered of the program's queries
    -- (D271): reply_generation is null until a record is first saved, and
    -- names the holder generation the record is for. Pending answers are
    -- comma-joined sequence numbers.
    ALTER TABLE tabs ADD COLUMN reply_generation INTEGER;
    ALTER TABLE tabs ADD COLUMN reply_answered INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE tabs ADD COLUMN reply_pending TEXT NOT NULL DEFAULT '';
    ALTER TABLE tabs ADD COLUMN reply_uncertain INTEGER NOT NULL DEFAULT 0;
",
    "
    -- Step 23 (PR, D283). A pull request review session (PR-4..PR-8): which
    -- pull requests it reviews, in the order asked for, the head each clone
    -- was made at, the newest head a check saw, and whether its agent may
    -- push. submit_started_unix_ms is kept before a review is posted, so a
    -- restart can look for it on GitHub (D287). Must land after TERM's 21, 22.
    CREATE TABLE review_sessions (
        session TEXT PRIMARY KEY REFERENCES sessions (id) ON DELETE CASCADE,
        push_allowed INTEGER NOT NULL
    ) STRICT;
    CREATE TABLE review_session_prs (
        session TEXT NOT NULL REFERENCES review_sessions (session) ON DELETE CASCADE,
        ordinal INTEGER NOT NULL,
        owner TEXT NOT NULL,
        name TEXT NOT NULL,
        number INTEGER NOT NULL,
        base_branch TEXT NOT NULL,
        head_branch TEXT NOT NULL,
        head_sha TEXT NOT NULL,
        checked_sha TEXT NOT NULL,
        state INTEGER NOT NULL,
        submit_started_unix_ms INTEGER,
        PRIMARY KEY (session, ordinal)
    ) STRICT;
",
    "
    -- Step 24. A Migrate item (UP-1, D281) belongs to no
    -- session, so its row's session is NULL, and the table is rebuilt to
    -- allow it, every row and index kept. migrate_thing and migrate_machine
    -- are set on a Migrate item only, and migrate_pending while it is still
    -- pending, so the partial index holds at most one pending item for each
    -- thing and machine. A withdrawn item keeps its row and blocks nothing.
    CREATE TABLE inbox_requests_new (
        id TEXT PRIMARY KEY,
        session TEXT REFERENCES sessions (id) ON DELETE CASCADE,
        created_unix_ms INTEGER NOT NULL,
        updated_unix_ms INTEGER NOT NULL,
        entry BLOB,
        entry_file TEXT,
        migrate_thing INTEGER,
        migrate_machine TEXT,
        migrate_pending INTEGER NOT NULL DEFAULT 0 CHECK (migrate_pending IN (0, 1)),
        CHECK ((entry IS NULL) <> (entry_file IS NULL)),
        CHECK ((migrate_thing IS NULL) = (migrate_machine IS NULL)),
        CHECK (session IS NOT NULL OR migrate_thing IS NOT NULL),
        CHECK (migrate_pending = 0 OR migrate_thing IS NOT NULL)
    ) STRICT;
    INSERT INTO inbox_requests_new (id, session, created_unix_ms, updated_unix_ms, entry, entry_file)
        SELECT id, session, created_unix_ms, updated_unix_ms, entry, entry_file FROM inbox_requests;
    DROP TABLE inbox_requests;
    ALTER TABLE inbox_requests_new RENAME TO inbox_requests;
    CREATE INDEX inbox_requests_by_session ON inbox_requests (session);
    CREATE UNIQUE INDEX inbox_requests_migrate_pending
        ON inbox_requests (migrate_thing, migrate_machine) WHERE migrate_pending = 1;
",
    "
    -- Step 25. A channel's own Desk sessions (CD-1..CD-5, D306): the
    -- channel each belongs to, the session it looks after, its six
    -- management switches, and where its closed folder went. The session
    -- itself is filed under the Desk's channel id.
    CREATE TABLE channel_desk_sessions (
        session_id TEXT PRIMARY KEY REFERENCES sessions (id) ON DELETE CASCADE,
        channel TEXT NOT NULL,
        target_session TEXT,
        read_files INTEGER NOT NULL,
        send_notes INTEGER NOT NULL,
        type_into_terminal INTEGER NOT NULL,
        start_and_fork INTEGER NOT NULL,
        stop_and_archive INTEGER NOT NULL,
        change_board_items INTEGER NOT NULL,
        last_used_unix_ms INTEGER NOT NULL,
        closed_unix_ms INTEGER,
        target_gone INTEGER NOT NULL,
        trash_item TEXT
    ) STRICT;
",
    "
    -- Step 26 (PR, D299). Which commits of a pull request the user
    -- reviewed, keyed by the pull request, so every review of it shares
    -- them: a mark per commit sha (reviewed 0 once unmarked, kept so the
    -- commit is never carried to), and the history of marks, unmarks,
    -- carries and whole-PR reviews, numbered per pull request and only
    -- ever added to. actor is the encoded humata.v1.Actor, NULL for what
    -- nobody did; commits are shas, one per line.
    CREATE TABLE pr_commit_marks (
        owner TEXT NOT NULL,
        name TEXT NOT NULL,
        number INTEGER NOT NULL,
        sha TEXT NOT NULL,
        patch_id TEXT NOT NULL,
        reviewed INTEGER NOT NULL CHECK (reviewed IN (0, 1)),
        reviewed_as_sha TEXT NOT NULL,
        reviewed_unix_ms INTEGER NOT NULL,
        reviewed_head_sha TEXT NOT NULL,
        actor BLOB,
        gh_login TEXT NOT NULL,
        PRIMARY KEY (owner, name, number, sha)
    ) STRICT;
    CREATE TABLE pr_review_history (
        owner TEXT NOT NULL,
        name TEXT NOT NULL,
        number INTEGER NOT NULL,
        seq INTEGER NOT NULL,
        at_unix_ms INTEGER NOT NULL,
        kind INTEGER NOT NULL,
        commits TEXT NOT NULL,
        head_sha TEXT NOT NULL,
        snapshot_n INTEGER,
        actor BLOB,
        gh_login TEXT NOT NULL,
        PRIMARY KEY (owner, name, number, seq)
    ) STRICT;
",
    "
    -- Step 27 (PR, D299). The idempotency key of the MarkCommitsReviewed
    -- call that made an entry, and its encoded answer, so a repeat answers
    -- the same and marks nothing. Only the latest answers per pull request
    -- are kept; an older entry keeps its key and loses its answer.
    ALTER TABLE pr_review_history ADD COLUMN idempotency_key TEXT;
    ALTER TABLE pr_review_history ADD COLUMN answer BLOB;
    CREATE UNIQUE INDEX pr_review_history_keys
        ON pr_review_history (owner, name, number, idempotency_key)
        WHERE idempotency_key IS NOT NULL;
",
    "
    -- Step 28 (TERM, D337). What became of each keyed Type call: pending
    -- from before its text goes to the shell, typed once all of it is
    -- there, so a retry after a restart neither types twice nor says typed
    -- when it may not have been. key is the call's method and idempotency
    -- key; at_unix_ms is when it was first seen. Rows older than the
    -- idempotency window are dropped when the daemon starts.
    CREATE TABLE typed_keys (
        key TEXT PRIMARY KEY,
        state TEXT NOT NULL CHECK (state IN ('pending', 'typed')),
        at_unix_ms INTEGER NOT NULL
    ) STRICT;
    CREATE INDEX typed_keys_by_age ON typed_keys (at_unix_ms);
",
    "
    -- Step 29 (TERM, D426). The holder process a pending Type call's text
    -- went to, so a retry asks that holder whether it typed the text; 0
    -- when no holder could say.
    ALTER TABLE typed_keys ADD COLUMN holder INTEGER NOT NULL DEFAULT 0;
",
    "
    -- Step 30 (CH-16, D396). The machine's read mark for a remote session as
    -- last reported, so a host restart shows it before the machine links
    -- again. A report kept before this step reads as not unread.
    ALTER TABLE remote_session_reports ADD COLUMN unread INTEGER NOT NULL DEFAULT 0;
",
    "
    -- Step 31 (DOCV, DV-12). The comments a Desk file had when a version of
    -- it was made: one row per session, commit and file, written once and
    -- kept with the session. comments is the session's comments on the file
    -- as the daemon encodes them (length-delimited humata.v1.Comment, in
    -- order), empty when it had none.
    CREATE TABLE doc_comment_records (
        session TEXT NOT NULL REFERENCES sessions (id) ON DELETE CASCADE,
        commit_id TEXT NOT NULL,
        path TEXT NOT NULL,
        recorded_unix_ms INTEGER NOT NULL,
        comments BLOB NOT NULL,
        PRIMARY KEY (session, commit_id, path)
    ) STRICT;
    CREATE INDEX doc_comment_records_by_path ON doc_comment_records (session, path);
",
    "
    -- Step 32 (TERM, D480). What the program in a terminal tab reports about
    -- itself: the title it sets (OSC 0/2) and its folder (OSC 7, kept as a
    -- path). Apart from tabs.title, the user's own name for the tab. Empty
    -- means the program never said.
    CREATE TABLE tab_program (
        tab TEXT PRIMARY KEY REFERENCES tabs (id) ON DELETE CASCADE,
        program_title TEXT NOT NULL DEFAULT '',
        pwd TEXT NOT NULL DEFAULT ''
    ) STRICT;
",
    "
    -- Step 33 (TODO2). Agent runs and cost recorded against a to-do item.
    CREATE TABLE todo_runs (
        id TEXT PRIMARY KEY,
        item_key TEXT NOT NULL,
        tool TEXT NOT NULL DEFAULT '',
        model TEXT NOT NULL DEFAULT '',
        outcome TEXT NOT NULL DEFAULT '',
        cost_micros INTEGER NOT NULL DEFAULT 0,
        started_unix_ms INTEGER NOT NULL DEFAULT 0,
        finished_unix_ms INTEGER NOT NULL DEFAULT 0,
        links_json TEXT NOT NULL DEFAULT '[]',
        session TEXT NOT NULL DEFAULT ''
    ) STRICT;
    CREATE INDEX todo_runs_by_item ON todo_runs (item_key, started_unix_ms, id);
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
            "PRAGMA busy_timeout = 5000;
             PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             PRAGMA fullfsync = ON;
             PRAGMA checkpoint_fullfsync = ON;",
        )
        .map_err(opened)?;
    migrate(&mut connection, path)?;
    Ok(connection)
}

fn migrate(connection: &mut Connection, path: &Path) -> Result<(), StoreError> {
    let opened = |source| StoreError::Open {
        path: path.to_owned(),
        source,
    };
    // All steps in one transaction: a crash part way leaves the schema
    // as it was. It takes the write lock before reading the version, so a
    // second process opening the same new database (the daemon binds its
    // socket before it opens the store) waits for the first one's steps
    // and then finds them done, instead of running them again.
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(opened)?;
    let found: i64 = transaction
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(opened)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::InboxRecord;
    use crate::blob;
    use crate::store::DATABASE;

    /// A database last written just before the step that lets an Inbox
    /// request have no session keeps its requests when the step rebuilds
    /// the table.
    #[test]
    fn rebuilding_the_inbox_table_keeps_its_requests() {
        let root = tempfile::tempdir().unwrap();
        let step = MIGRATIONS
            .iter()
            .position(|sql| sql.contains("inbox_requests_new"))
            .unwrap();
        let connection = Connection::open(root.path().join(DATABASE)).unwrap();
        connection
            .execute_batch("PRAGMA foreign_keys = ON;")
            .unwrap();
        for sql in &MIGRATIONS[..step] {
            connection.execute_batch(sql).unwrap();
        }
        connection
            .execute(
                "INSERT INTO sessions (id, channel, machine, title, agent, folder, created_unix_ms)
                 VALUES ('se_a', 'ch_a', 'ma_a', 'asker', 'claude', x'', 1)",
                [],
            )
            .unwrap();
        let entry = blob::encode(blob::Type::InboxEntry, 1, b"request").unwrap();
        connection
            .execute(
                "INSERT INTO inbox_requests (id, session, created_unix_ms, updated_unix_ms, entry)
                 VALUES ('rq_1', 'se_a', 5, 6, ?1)",
                [entry],
            )
            .unwrap();
        connection
            .pragma_update(None, "user_version", i64::try_from(step).unwrap())
            .unwrap();
        drop(connection);

        let store = crate::Store::open(root.path()).unwrap();
        assert_eq!(
            store.inbox_requests().unwrap(),
            vec![InboxRecord {
                id: "rq_1".to_owned(),
                session: "se_a".to_owned(),
                entry: b"request".to_vec(),
                created_unix_ms: 5,
                updated_unix_ms: 6,
            }]
        );
    }

    /// A report kept before the read mark was saved reads as not unread, and
    /// the rest of it is as it was.
    #[test]
    fn a_report_kept_before_the_read_mark_reads_as_read() {
        let root = tempfile::tempdir().unwrap();
        let step = MIGRATIONS
            .iter()
            .position(|sql| sql.contains("ADD COLUMN unread"))
            .unwrap();
        let connection = Connection::open(root.path().join(DATABASE)).unwrap();
        for sql in &MIGRATIONS[..step] {
            connection.execute_batch(sql).unwrap();
        }
        connection
            .execute(
                "INSERT INTO remote_session_reports
                     (machine, session, reported, exit_code, pointers, reported_unix_ms)
                 VALUES ('box-1', 'se_1', 2, 0, x'', 20)",
                [],
            )
            .unwrap();
        connection
            .pragma_update(None, "user_version", i64::try_from(step).unwrap())
            .unwrap();
        drop(connection);

        let store = crate::Store::open(root.path()).unwrap();
        let reports = store.remote_reports().unwrap();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].session, "se_1");
        assert_eq!(reports[0].reported, 2);
        assert!(!reports[0].unread);
    }
}
