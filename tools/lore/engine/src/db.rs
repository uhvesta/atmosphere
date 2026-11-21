//! Disposable SQLite WAL + FTS5 projection (`index.db`, `SPEC.md §4.3`).
//!
//! All state in `index.db` is deterministically rebuildable from hot `.pblog`
//! segments and cold `.pblog.zst` archives (`ST-1`). `live_presence` is
//! ephemeral and never written to `.pblog` (`AW-4`).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use lore_context::tokenize_code_terms;
use lore_proto::{
    Claim, ClaimKind, ClaimStatus, ClaimTier, EdgeRel, LogEvent, LoreError, OriginSnapshot,
    OutcomeKind, PathOrErrorOverlap, PeerPresence, PeerRepoMount, Predicate, Principal, Result,
    Verifier, log_event,
};
use rusqlite::{Connection, OpenFlags, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};

use crate::log::ArchiveRecordLoc;

const PROJECTION_VERSION: &str = "1";

/// Returns `<root>/index.db`.
pub fn index_db_path(root: &Path) -> PathBuf {
    root.join("index.db")
}

/// Raw signal counters stored on a claim (`MOD-4`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimCounters {
    pub corroborator_count: i64,
    pub dispute_count: i64,
    pub surfaced_count: i64,
    pub helpful_count: i64,
    pub harmful_count: i64,
    pub not_applicable_count: i64,
    pub last_surfaced_at_ms: i64,
    pub last_validated_at_ms: i64,
}

/// Materialized row from `claims` + `claim_entities`.
#[derive(Clone, Debug, PartialEq)]
pub struct ClaimRow {
    pub id: String,
    pub kind: ClaimKind,
    pub status: ClaimStatus,
    pub tier: ClaimTier,
    pub scope: String,
    pub statement: String,
    pub body: Option<String>,
    pub dead_ends: Vec<String>,
    pub keywords: Vec<String>,
    pub attrs: BTreeMap<String, String>,
    pub about: Vec<String>,
    pub applies_when: Vec<Predicate>,
    pub author: Option<Principal>,
    pub origin: Option<OriginSnapshot>,
    pub verifiers: Vec<Verifier>,
    pub tainted: bool,
    pub pinned: bool,
    pub importance: f32,
    pub counters: ClaimCounters,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub valid_from_ms: i64,
    pub valid_until_ms: i64,
    pub archive_seg: Option<String>,
    pub archive_frame_off: Option<u64>,
    pub archive_rec_off: Option<u64>,
    pub archive_rec_len: Option<u64>,
}

impl ClaimRow {
    /// Converts a hot (or hydrated) `ClaimRow` into a full `lore_proto::Claim`.
    pub fn to_proto_claim(&self) -> Claim {
        Claim {
            id: self.id.clone(),
            kind: self.kind as i32,
            status: self.status as i32,
            created_at_ms: self.created_at_ms,
            updated_at_ms: self.updated_at_ms,
            valid_from_ms: self.valid_from_ms,
            valid_until_ms: self.valid_until_ms,
            review_after_ms: 0,
            statement: self.statement.clone(),
            body: self.body.clone().unwrap_or_default(),
            dead_ends: self.dead_ends.clone(),
            keywords: self.keywords.clone(),
            attrs: self.attrs.clone(),
            scope: self.scope.clone(),
            about: self.about.clone(),
            applies_when: self.applies_when.clone(),
            author: self.author.clone(),
            tainted: self.tainted,
            origin: self.origin.clone(),
            verifiers: self.verifiers.clone(),
            pinned: self.pinned,
            importance: self.importance,
        }
    }
}

/// Directed edge between two claims stored in `claim_edges`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EdgeRecord {
    pub src_id: String,
    pub rel: EdgeRel,
    pub dst_id: String,
    pub created_at_ms: i64,
    pub author_id: String,
    pub note: String,
}

/// Aggregate counts from `index.db` for `lore status`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DbStats {
    pub hot_claims: u64,
    pub cold_claims: u64,
    pub active_claims: u64,
    pub live_peers: u64,
    pub applied_seq: u64,
    pub last_gc_ms: i64,
}

/// SQLite WAL + FTS5 projection database.
pub struct ProjectionDb {
    conn: Connection,
}

fn sqlite_err(op: &str, err: rusqlite::Error) -> LoreError {
    LoreError::Sqlite {
        op: op.to_string(),
        reason: err.to_string(),
    }
}

impl ProjectionDb {
    /// Opens (or initializes) `<root>/index.db` with WAL mode and `auto_vacuum = INCREMENTAL`.
    pub fn open(root: &Path) -> Result<Self> {
        fs::create_dir_all(root).map_err(|source| LoreError::Io {
            path: root.to_path_buf(),
            source,
        })?;
        let path = index_db_path(root);
        let conn = Connection::open(&path).map_err(|e| sqlite_err("open", e))?;
        let db = Self { conn };
        db.init_schema()?;
        Ok(db)
    }

    /// Opens `<root>/index.db` strictly read-only with a short `busy_timeout_ms`
    /// (for hooks and read-only CLI commands, never creating or mutating files).
    ///
    /// If `index.db` does not exist yet, opens an empty in-memory schema so
    /// read-only queries return empty results cleanly.
    pub fn open_read_only(root: &Path, busy_timeout_ms: u64) -> Result<Self> {
        let path = index_db_path(root);
        if !path.exists() {
            let conn = Connection::open_in_memory().map_err(|e| sqlite_err("open_in_memory", e))?;
            let db = Self { conn };
            db.init_schema()?;
            return Ok(db);
        }
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let conn = Connection::open_with_flags(&path, flags)
            .map_err(|e| sqlite_err("open_read_only", e))?;
        conn.execute_batch(&format!(
            "PRAGMA query_only = ON; PRAGMA busy_timeout = {busy_timeout_ms};"
        ))
        .map_err(|e| sqlite_err("open_read_only_pragma", e))?;
        Ok(Self { conn })
    }

    /// Removes `index.db` (plus `-wal` and `-shm`) and initializes a clean schema (`ST-1`).
    pub fn drop_and_recreate(root: &Path) -> Result<Self> {
        let path = index_db_path(root);
        for suffix in ["", "-wal", "-shm"] {
            let p = PathBuf::from(format!("{}{suffix}", path.display()));
            if p.exists() {
                fs::remove_file(&p).map_err(|source| LoreError::Io { path: p, source })?;
            }
        }
        Self::open(root)
    }

    fn init_schema(&self) -> Result<()> {
        self.conn
            .execute_batch(
                "
                PRAGMA auto_vacuum = INCREMENTAL;
                PRAGMA journal_mode = WAL;
                PRAGMA synchronous = NORMAL;
                PRAGMA secure_delete = ON;
                PRAGMA busy_timeout = 5000;

                CREATE TABLE IF NOT EXISTS meta (
                    key TEXT PRIMARY KEY,
                    value TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS seen_events (
                    event_id TEXT PRIMARY KEY
                );

                CREATE TABLE IF NOT EXISTS tainted_sessions (
                    session_id TEXT PRIMARY KEY,
                    tainted_at_ms INT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS claims (
                    id TEXT PRIMARY KEY,
                    kind INT NOT NULL,
                    status INT NOT NULL,
                    tier INT NOT NULL,
                    scope TEXT NOT NULL,
                    statement TEXT NOT NULL,
                    body TEXT,
                    dead_ends_json TEXT,
                    keywords_text TEXT,
                    attrs_json TEXT,
                    applies_when_json TEXT,
                    author_json TEXT,
                    origin_json TEXT,
                    verifiers_json TEXT,
                    tainted INT NOT NULL,
                    pinned INT NOT NULL,
                    importance REAL NOT NULL,
                    corroborator_count INT NOT NULL,
                    dispute_count INT NOT NULL,
                    surfaced_count INT NOT NULL,
                    helpful_count INT NOT NULL,
                    harmful_count INT NOT NULL,
                    not_applicable_count INT NOT NULL,
                    created_at_ms INT NOT NULL,
                    updated_at_ms INT NOT NULL,
                    valid_from_ms INT NOT NULL,
                    valid_until_ms INT NOT NULL,
                    last_surfaced_at_ms INT NOT NULL,
                    last_validated_at_ms INT NOT NULL,
                    archive_seg TEXT,
                    archive_frame_off INT,
                    archive_rec_off INT,
                    archive_rec_len INT
                );

                CREATE INDEX IF NOT EXISTS idx_claims_status_tier ON claims(status, tier);
                CREATE INDEX IF NOT EXISTS idx_claims_scope ON claims(scope);

                CREATE VIRTUAL TABLE IF NOT EXISTS claims_fts USING fts5(
                    claim_id UNINDEXED,
                    statement,
                    keywords_about,
                    dead_ends,
                    body,
                    tokenize='unicode61'
                );

                CREATE TABLE IF NOT EXISTS claim_entities (
                    entity_uri TEXT NOT NULL,
                    claim_id TEXT NOT NULL,
                    PRIMARY KEY (entity_uri, claim_id)
                ) WITHOUT ROWID;

                CREATE INDEX IF NOT EXISTS idx_claim_entities_claim ON claim_entities(claim_id);

                CREATE TABLE IF NOT EXISTS claim_edges (
                    src_id TEXT NOT NULL,
                    rel INT NOT NULL,
                    dst_id TEXT NOT NULL,
                    created_at_ms INT NOT NULL,
                    author_id TEXT NOT NULL,
                    note TEXT,
                    PRIMARY KEY (src_id, rel, dst_id)
                ) WITHOUT ROWID;

                CREATE INDEX IF NOT EXISTS idx_claim_edges_dst ON claim_edges(dst_id);

                CREATE TABLE IF NOT EXISTS claim_corroborators (
                    claim_id TEXT NOT NULL,
                    session_id TEXT NOT NULL,
                    machine_id TEXT NOT NULL,
                    PRIMARY KEY (claim_id, session_id, machine_id)
                ) WITHOUT ROWID;

                CREATE TABLE IF NOT EXISTS claim_exposures (
                    session_id TEXT NOT NULL,
                    claim_id TEXT NOT NULL,
                    surfaced_at_ms INT NOT NULL,
                    outcome INT,
                    PRIMARY KEY (session_id, claim_id)
                ) WITHOUT ROWID;

                CREATE TABLE IF NOT EXISTS live_presence (
                    session_id TEXT PRIMARY KEY,
                    channel_id TEXT,
                    machine_id TEXT NOT NULL,
                    agent_kind TEXT NOT NULL,
                    mounted_repos_json TEXT NOT NULL,
                    intent TEXT,
                    hypothesis TEXT,
                    touched_paths_json TEXT NOT NULL,
                    recent_failures_json TEXT NOT NULL,
                    updated_at_ms INT NOT NULL,
                    ttl_ms INT NOT NULL
                );
                ",
            )
            .map_err(|e| sqlite_err("init_schema", e))?;

        self.set_meta_if_absent("projection_version", PROJECTION_VERSION)?;
        self.set_meta_if_absent("applied_seq", "0")?;
        self.set_meta_if_absent("last_gc_ms", "0")?;
        self.set_meta_if_absent("writes_since_gc", "0")?;
        Ok(())
    }

    fn set_meta_if_absent(&self, key: &str, value: &str) -> Result<()> {
        self.conn
            .execute(
                "INSERT OR IGNORE INTO meta(key, value) VALUES (?1, ?2)",
                params![key, value],
            )
            .map_err(|e| sqlite_err("set_meta_if_absent", e))?;
        Ok(())
    }

    pub fn get_meta(&self, key: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT value FROM meta WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| sqlite_err("get_meta", e))
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO meta(key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
            .map_err(|e| sqlite_err("set_meta", e))?;
        Ok(())
    }

    pub fn applied_seq(&self) -> Result<u64> {
        Ok(self
            .get_meta("applied_seq")?
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0))
    }

    pub fn writes_since_gc(&self) -> Result<u64> {
        Ok(self
            .get_meta("writes_since_gc")?
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0))
    }

    pub fn increment_writes_since_gc(&self, delta: u64) -> Result<u64> {
        let next = self.writes_since_gc()?.saturating_add(delta);
        self.set_meta("writes_since_gc", &next.to_string())?;
        Ok(next)
    }

    pub fn has_seen_event_id(&self, event_id: &str) -> Result<bool> {
        if event_id.is_empty() {
            return Ok(false);
        }
        let exists: Option<i64> = self
            .conn
            .query_row(
                "SELECT 1 FROM seen_events WHERE event_id = ?1",
                params![event_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| sqlite_err("has_seen_event_id", e))?;
        Ok(exists.is_some())
    }

    /// Marks `session_id` as tainted so subsequent claims from that session
    /// inherit `tainted = true` (`SEC-2`).
    pub fn mark_session_tainted(&self, session_id: &str, now_ms: i64) -> Result<()> {
        if session_id.is_empty() {
            return Ok(());
        }
        self.conn
            .execute(
                "INSERT INTO tainted_sessions(session_id, tainted_at_ms) VALUES (?1, ?2)
                 ON CONFLICT(session_id) DO NOTHING",
                params![session_id, now_ms],
            )
            .map_err(|e| sqlite_err("mark_session_tainted", e))?;
        Ok(())
    }

    /// Returns `true` if `session_id` has been marked tainted (`SEC-2`).
    pub fn is_session_tainted(&self, session_id: &str) -> Result<bool> {
        if session_id.is_empty() {
            return Ok(false);
        }
        let exists: Option<i64> = self
            .conn
            .query_row(
                "SELECT 1 FROM tainted_sessions WHERE session_id = ?1",
                params![session_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| sqlite_err("is_session_tainted", e))?;
        Ok(exists.is_some())
    }

    /// Applies a single [`LogEvent`] to `index.db` inside one SQLite transaction (`ST-2`).
    pub fn apply_event(
        &mut self,
        event: &LogEvent,
        archive_loc: Option<&ArchiveRecordLoc>,
    ) -> Result<()> {
        let tx = self
            .conn
            .transaction()
            .map_err(|e| sqlite_err("begin_tx", e))?;
        apply_event_in_tx(&tx, event, archive_loc)?;
        tx.commit().map_err(|e| sqlite_err("commit_tx", e))?;
        Ok(())
    }

    /// Applies a batch of `(LogEvent, Option<ArchiveRecordLoc>)` inside one SQLite transaction.
    pub fn apply_events_batch(
        &mut self,
        events: &[(LogEvent, Option<ArchiveRecordLoc>)],
    ) -> Result<()> {
        if events.is_empty() {
            return Ok(());
        }
        let tx = self
            .conn
            .transaction()
            .map_err(|e| sqlite_err("begin_batch_tx", e))?;
        for (ev, loc) in events {
            apply_event_in_tx(&tx, ev, loc.as_ref())?;
        }
        tx.commit().map_err(|e| sqlite_err("commit_batch_tx", e))?;
        Ok(())
    }

    /// Fetches a single claim by ID.
    pub fn get_claim(&self, id: &str) -> Result<Option<ClaimRow>> {
        let row = self
            .conn
            .query_row(
                "SELECT
                    id, kind, status, tier, scope, statement, body, dead_ends_json,
                    keywords_text, attrs_json, applies_when_json, author_json, origin_json,
                    verifiers_json, tainted, pinned, importance,
                    corroborator_count, dispute_count, surfaced_count, helpful_count,
                    harmful_count, not_applicable_count, created_at_ms, updated_at_ms,
                    valid_from_ms, valid_until_ms, last_surfaced_at_ms, last_validated_at_ms,
                    archive_seg, archive_frame_off, archive_rec_off, archive_rec_len
                 FROM claims WHERE id = ?1",
                params![id],
                row_to_claim_without_about,
            )
            .optional()
            .map_err(|e| sqlite_err("get_claim", e))?;

        match row {
            None => Ok(None),
            Some(mut claim) => {
                claim.about = self.get_entities_for_claim(&claim.id)?;
                Ok(Some(claim))
            }
        }
    }

    /// Lists all claims in `index.db` (used by GC and admin inspection).
    pub fn list_all_claims(&self) -> Result<Vec<ClaimRow>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT
                    id, kind, status, tier, scope, statement, body, dead_ends_json,
                    keywords_text, attrs_json, applies_when_json, author_json, origin_json,
                    verifiers_json, tainted, pinned, importance,
                    corroborator_count, dispute_count, surfaced_count, helpful_count,
                    harmful_count, not_applicable_count, created_at_ms, updated_at_ms,
                    valid_from_ms, valid_until_ms, last_surfaced_at_ms, last_validated_at_ms,
                    archive_seg, archive_frame_off, archive_rec_off, archive_rec_len
                 FROM claims",
            )
            .map_err(|e| sqlite_err("list_all_claims_prepare", e))?;

        let rows = stmt
            .query_map([], row_to_claim_without_about)
            .map_err(|e| sqlite_err("list_all_claims_query", e))?;

        let mut out = Vec::new();
        for r in rows {
            let mut claim = r.map_err(|e| sqlite_err("list_all_claims_row", e))?;
            claim.about = self.get_entities_for_claim(&claim.id)?;
            out.push(claim);
        }
        Ok(out)
    }

    /// Lists candidate claims for `recall` filtered by permitted scopes, kinds,
    /// lifecycle status, and quarantine visibility (`MOD-2`).
    pub fn list_candidate_claims(
        &self,
        scopes: &[String],
        kinds: Option<&[ClaimKind]>,
        include_history: bool,
        caller_session_id: Option<&str>,
        now_ms: i64,
    ) -> Result<Vec<ClaimRow>> {
        let all = self.list_all_claims()?;
        let scope_set: HashSet<&str> = scopes.iter().map(String::as_str).collect();
        let kind_set: Option<HashSet<ClaimKind>> =
            kinds.map(|ks| ks.iter().copied().collect());

        let mut out = Vec::new();
        for claim in all {
            if !scope_set.is_empty() && !scope_set.contains(claim.scope.as_str()) {
                continue;
            }
            if let Some(ks) = &kind_set {
                if !ks.contains(&claim.kind) {
                    continue;
                }
            }
            // SEC-2 / MOD-2: Quarantined claims are visible ONLY to their originating session,
            // even when `include_history` is true.
            if claim.status == ClaimStatus::Quarantined {
                let same_session = match (
                    caller_session_id,
                    claim.author.as_ref().map(|a| a.session_id.as_str()),
                ) {
                    (Some(caller), Some(author_sess)) if !caller.is_empty() => {
                        caller == author_sess
                    }
                    _ => false,
                };
                if !same_session {
                    continue;
                }
            } else if !include_history {
                match claim.status {
                    ClaimStatus::Active => {
                        if claim.valid_until_ms > 0 && now_ms >= claim.valid_until_ms {
                            continue;
                        }
                        if claim.valid_from_ms > 0 && now_ms < claim.valid_from_ms {
                            continue;
                        }
                    }
                    _ => continue,
                }
            }
            out.push(claim);
        }

        Ok(out)
    }

    /// Runs code-aware FTS5 BM25 search over Tier-0 Hot claims (`ST-6`).
    ///
    /// Returns a map `claim_id -> normalized_lex_score` in `(0.0, 1.0]`.
    pub fn search_fts(&self, query: &str, limit: usize) -> Result<HashMap<String, f64>> {
        let terms = tokenize_code_terms(query);
        if terms.is_empty() || limit == 0 {
            return Ok(HashMap::new());
        }

        let fts_query = terms
            .iter()
            .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" OR ");

        let mut stmt = self
            .conn
            .prepare(
                "SELECT claim_id, bm25(claims_fts, 0.0, 5.0, 4.0, 2.5, 1.0) AS rank
                 FROM claims_fts
                 WHERE claims_fts MATCH ?1
                 ORDER BY rank ASC
                 LIMIT ?2",
            )
            .map_err(|e| sqlite_err("search_fts_prepare", e))?;

        let rows = stmt
            .query_map(params![fts_query, limit as i64], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
            })
            .map_err(|e| sqlite_err("search_fts_query", e))?;

        let mut raw_scores: Vec<(String, f64)> = Vec::new();
        for r in rows {
            let (id, bm25_val) = r.map_err(|e| sqlite_err("search_fts_row", e))?;
            // SQLite FTS5 bm25() returns negative values where more negative = better match.
            let pos = (-bm25_val).max(0.0);
            raw_scores.push((id, pos));
        }

        let max_score = raw_scores
            .iter()
            .map(|(_, s)| *s)
            .fold(0.0_f64, f64::max);

        let mut out = HashMap::with_capacity(raw_scores.len());
        for (id, s) in raw_scores {
            let norm = if max_score > 0.0 {
                (s / max_score).clamp(0.05, 1.0)
            } else {
                0.5
            };
            out.insert(id, norm);
        }
        Ok(out)
    }

    /// Returns all edges where `claim_id` is either `src_id` or `dst_id`.
    pub fn get_edges_for_claim(&self, claim_id: &str) -> Result<Vec<EdgeRecord>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT src_id, rel, dst_id, created_at_ms, author_id, note
                 FROM claim_edges
                 WHERE src_id = ?1 OR dst_id = ?1
                 ORDER BY created_at_ms ASC",
            )
            .map_err(|e| sqlite_err("get_edges_prepare", e))?;

        let rows = stmt
            .query_map(params![claim_id], |row| {
                let rel_i32: i32 = row.get(1)?;
                Ok(EdgeRecord {
                    src_id: row.get(0)?,
                    rel: EdgeRel::try_from(rel_i32).unwrap_or(EdgeRel::Unspecified),
                    dst_id: row.get(2)?,
                    created_at_ms: row.get(3)?,
                    author_id: row.get(4)?,
                    note: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
                })
            })
            .map_err(|e| sqlite_err("get_edges_query", e))?;

        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| sqlite_err("get_edges_row", e))?);
        }
        Ok(out)
    }

    fn get_entities_for_claim(&self, claim_id: &str) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT entity_uri FROM claim_entities WHERE claim_id = ?1 ORDER BY entity_uri ASC",
            )
            .map_err(|e| sqlite_err("get_entities_prepare", e))?;
        let rows = stmt
            .query_map(params![claim_id], |row| row.get::<_, String>(0))
            .map_err(|e| sqlite_err("get_entities_query", e))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| sqlite_err("get_entities_row", e))?);
        }
        Ok(out)
    }

    /// Records session exposures in `claim_exposures` and increments `surfaced_count` (`RET-3`).
    pub fn record_exposures(
        &mut self,
        session_id: &str,
        claim_ids: &[String],
        now_ms: i64,
    ) -> Result<()> {
        if session_id.is_empty() || claim_ids.is_empty() {
            return Ok(());
        }
        let tx = self
            .conn
            .transaction()
            .map_err(|e| sqlite_err("record_exposures_tx", e))?;
        for cid in claim_ids {
            tx.execute(
                "INSERT INTO claim_exposures(session_id, claim_id, surfaced_at_ms, outcome)
                 VALUES (?1, ?2, ?3, NULL)
                 ON CONFLICT(session_id, claim_id) DO UPDATE SET surfaced_at_ms = excluded.surfaced_at_ms",
                params![session_id, cid, now_ms],
            )
            .map_err(|e| sqlite_err("record_exposures_upsert", e))?;

            tx.execute(
                "UPDATE claims
                 SET surfaced_count = surfaced_count + 1,
                     last_surfaced_at_ms = CASE
                         WHEN last_surfaced_at_ms < ?2 THEN ?2
                         ELSE last_surfaced_at_ms
                     END
                 WHERE id = ?1",
                params![cid, now_ms],
            )
            .map_err(|e| sqlite_err("record_exposures_counter", e))?;
        }
        tx.commit().map_err(|e| sqlite_err("record_exposures_commit", e))?;
        Ok(())
    }

    /// Returns `claim_id -> surfaced_at_ms` for claims already surfaced to `session_id` (`RET-3`).
    pub fn exposed_claims_for_session(&self, session_id: &str) -> Result<HashMap<String, i64>> {
        if session_id.is_empty() {
            return Ok(HashMap::new());
        }
        let mut stmt = self
            .conn
            .prepare(
                "SELECT claim_id, surfaced_at_ms FROM claim_exposures WHERE session_id = ?1",
            )
            .map_err(|e| sqlite_err("exposed_claims_prepare", e))?;
        let rows = stmt
            .query_map(params![session_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|e| sqlite_err("exposed_claims_query", e))?;
        let mut out = HashMap::new();
        for r in rows {
            let (cid, ts) = r.map_err(|e| sqlite_err("exposed_claims_row", e))?;
            out.insert(cid, ts);
        }
        Ok(out)
    }

    /// Converts a claim row into a lightweight Tier-1 Cold stub (`GC-2`),
    /// stripping heavy payload columns and removing it from `claims_fts`.
    pub fn mark_claim_cold(&mut self, claim_id: &str, loc: &ArchiveRecordLoc) -> Result<()> {
        let tx = self
            .conn
            .transaction()
            .map_err(|e| sqlite_err("mark_claim_cold_tx", e))?;
        tx.execute(
            "UPDATE claims
             SET tier = ?2,
                 body = NULL,
                 dead_ends_json = NULL,
                 attrs_json = NULL,
                 verifiers_json = NULL,
                 origin_json = NULL,
                 archive_seg = ?3,
                 archive_frame_off = ?4,
                 archive_rec_off = ?5,
                 archive_rec_len = ?6
             WHERE id = ?1",
            params![
                claim_id,
                ClaimTier::ColdArchived.as_i32(),
                loc.archive_seg,
                loc.frame_offset as i64,
                loc.record_offset as i64,
                loc.record_len as i64,
            ],
        )
        .map_err(|e| sqlite_err("mark_claim_cold_update", e))?;

        tx.execute(
            "DELETE FROM claims_fts WHERE claim_id = ?1",
            params![claim_id],
        )
        .map_err(|e| sqlite_err("mark_claim_cold_fts", e))?;

        tx.commit().map_err(|e| sqlite_err("mark_claim_cold_commit", e))?;
        Ok(())
    }

    /// Updates archive coordinates for a cold claim after an archive segment rewrite.
    pub fn update_archive_loc(&mut self, claim_id: &str, loc: &ArchiveRecordLoc) -> Result<()> {
        self.conn
            .execute(
                "UPDATE claims
                 SET archive_seg = ?2,
                     archive_frame_off = ?3,
                     archive_rec_off = ?4,
                     archive_rec_len = ?5
                 WHERE id = ?1 AND tier = ?6",
                params![
                    claim_id,
                    loc.archive_seg,
                    loc.frame_offset as i64,
                    loc.record_offset as i64,
                    loc.record_len as i64,
                    ClaimTier::ColdArchived.as_i32(),
                ],
            )
            .map_err(|e| sqlite_err("update_archive_loc", e))?;
        Ok(())
    }

    /// Promotes a hydrated cold claim back to Tier-0 Hot and re-indexes it in `claims_fts` (`GC-3`).
    pub fn promote_claim_hot(&mut self, claim: &Claim, now_ms: i64) -> Result<()> {
        let tx = self
            .conn
            .transaction()
            .map_err(|e| sqlite_err("promote_claim_hot_tx", e))?;

        let dead_ends_json = serde_json::to_string(&claim.dead_ends).unwrap_or_else(|_| "[]".into());
        let keywords_text = claim.keywords.join(" ");
        let attrs_json = serde_json::to_string(&claim.attrs).unwrap_or_else(|_| "{}".into());
        let origin_json = claim
            .origin
            .as_ref()
            .and_then(|o| serde_json::to_string(o).ok());
        let verifiers_json =
            serde_json::to_string(&claim.verifiers).unwrap_or_else(|_| "[]".into());

        tx.execute(
            "UPDATE claims
             SET tier = ?2,
                 body = ?3,
                 dead_ends_json = ?4,
                 keywords_text = ?5,
                 attrs_json = ?6,
                 origin_json = ?7,
                 verifiers_json = ?8,
                 last_validated_at_ms = ?9,
                 updated_at_ms = CASE WHEN updated_at_ms < ?9 THEN ?9 ELSE updated_at_ms END,
                 archive_seg = NULL,
                 archive_frame_off = NULL,
                 archive_rec_off = NULL,
                 archive_rec_len = NULL
             WHERE id = ?1",
            params![
                claim.id,
                ClaimTier::Hot.as_i32(),
                claim.body,
                dead_ends_json,
                keywords_text,
                attrs_json,
                origin_json,
                verifiers_json,
                now_ms,
            ],
        )
        .map_err(|e| sqlite_err("promote_claim_hot_update", e))?;

        reindex_claim_fts_in_tx(&tx, claim)?;
        tx.commit().map_err(|e| sqlite_err("promote_claim_hot_commit", e))?;
        Ok(())
    }

    /// Permanently removes a claim and its auxiliary rows from `index.db` (`GC-4`, `GC-5`).
    pub fn purge_claim_row(&mut self, claim_id: &str) -> Result<()> {
        let tx = self
            .conn
            .transaction()
            .map_err(|e| sqlite_err("purge_claim_row_tx", e))?;
        purge_claim_in_tx(&tx, claim_id)?;
        tx.commit().map_err(|e| sqlite_err("purge_claim_row_commit", e))?;
        Ok(())
    }

    /// Folds `claim_exposures` older than `cutoff_ms` into `claims.surfaced_count`
    /// (already incremented on exposure) and deletes the old rows (`GC-4 (d)`).
    pub fn fold_and_prune_exposures(&mut self, cutoff_ms: i64, dry_run: bool) -> Result<u64> {
        if dry_run {
            let count: i64 = self
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM claim_exposures WHERE surfaced_at_ms < ?1",
                    params![cutoff_ms],
                    |row| row.get(0),
                )
                .map_err(|e| sqlite_err("count_old_exposures", e))?;
            return Ok(count.max(0) as u64);
        }

        let deleted = self
            .conn
            .execute(
                "DELETE FROM claim_exposures WHERE surfaced_at_ms < ?1",
                params![cutoff_ms],
            )
            .map_err(|e| sqlite_err("prune_old_exposures", e))?;
        Ok(deleted as u64)
    }

    /// Runs `PRAGMA incremental_vacuum` to reclaim freed SQLite pages (`GC-4`).
    pub fn incremental_vacuum(&self) -> Result<()> {
        self.conn
            .execute_batch("PRAGMA incremental_vacuum;")
            .map_err(|e| sqlite_err("incremental_vacuum", e))
    }

    /// Rebuilds the SQLite file and truncates the WAL so erased payload bytes
    /// cannot linger in free pages or `index.db-wal` (`GC-5`).
    pub fn secure_purge_vacuum(&self) -> Result<()> {
        self.conn
            .execute_batch("VACUUM; PRAGMA wal_checkpoint(TRUNCATE);")
            .map_err(|e| sqlite_err("secure_purge_vacuum", e))
    }

    /// Upserts a live peer presence row and opportunistically prunes expired rows (`AW-1`, `AW-2`).
    pub fn upsert_presence(&mut self, presence: &PeerPresence, now_ms: i64) -> Result<()> {
        let tx = self
            .conn
            .transaction()
            .map_err(|e| sqlite_err("upsert_presence_tx", e))?;

        tx.execute(
            "DELETE FROM live_presence WHERE (updated_at_ms + ttl_ms) < ?1",
            params![now_ms],
        )
        .map_err(|e| sqlite_err("prune_presence", e))?;

        let mounted_repos_json =
            serde_json::to_string(&presence.mounted_repos).unwrap_or_else(|_| "[]".into());
        let touched_paths_json =
            serde_json::to_string(&presence.touched_paths).unwrap_or_else(|_| "[]".into());
        let recent_failures_json =
            serde_json::to_string(&presence.recent_failures).unwrap_or_else(|_| "[]".into());

        tx.execute(
            "INSERT INTO live_presence(
                session_id, channel_id, machine_id, agent_kind,
                mounted_repos_json, intent, hypothesis,
                touched_paths_json, recent_failures_json,
                updated_at_ms, ttl_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(session_id) DO UPDATE SET
                channel_id = excluded.channel_id,
                machine_id = excluded.machine_id,
                agent_kind = excluded.agent_kind,
                mounted_repos_json = excluded.mounted_repos_json,
                intent = excluded.intent,
                hypothesis = excluded.hypothesis,
                touched_paths_json = excluded.touched_paths_json,
                recent_failures_json = excluded.recent_failures_json,
                updated_at_ms = excluded.updated_at_ms,
                ttl_ms = excluded.ttl_ms",
            params![
                presence.session_id,
                presence.channel_id,
                presence.machine_id,
                presence.agent_kind,
                mounted_repos_json,
                presence.intent,
                presence.hypothesis,
                touched_paths_json,
                recent_failures_json,
                presence.updated_at_ms,
                presence.ttl_ms,
            ],
        )
        .map_err(|e| sqlite_err("upsert_presence_insert", e))?;

        tx.commit().map_err(|e| sqlite_err("upsert_presence_commit", e))?;
        Ok(())
    }

    /// Queries all non-expired live peer sessions (`updated_at_ms + ttl_ms >= now_ms`) (`AW-2`).
    pub fn query_live_presence(
        &self,
        now_ms: i64,
        exclude_session: Option<&str>,
    ) -> Result<Vec<PeerPresence>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT session_id, channel_id, machine_id, agent_kind,
                        mounted_repos_json, intent, hypothesis,
                        touched_paths_json, recent_failures_json,
                        updated_at_ms, ttl_ms
                 FROM live_presence
                 WHERE (updated_at_ms + ttl_ms) >= ?1
                 ORDER BY updated_at_ms DESC",
            )
            .map_err(|e| sqlite_err("query_live_presence_prepare", e))?;

        let rows = stmt
            .query_map(params![now_ms], |row| {
                let repos_json: String = row.get(4)?;
                let paths_json: String = row.get(7)?;
                let fails_json: String = row.get(8)?;
                Ok(PeerPresence {
                    session_id: row.get(0)?,
                    channel_id: row.get(1)?,
                    machine_id: row.get(2)?,
                    agent_kind: row.get(3)?,
                    mounted_repos: serde_json::from_str::<Vec<PeerRepoMount>>(&repos_json)
                        .unwrap_or_default(),
                    intent: row.get(5)?,
                    hypothesis: row.get(6)?,
                    touched_paths: serde_json::from_str::<Vec<String>>(&paths_json)
                        .unwrap_or_default(),
                    recent_failures: serde_json::from_str::<Vec<String>>(&fails_json)
                        .unwrap_or_default(),
                    updated_at_ms: row.get(9)?,
                    ttl_ms: row.get(10)?,
                })
            })
            .map_err(|e| sqlite_err("query_live_presence_query", e))?;

        let mut out = Vec::new();
        for r in rows {
            let peer = r.map_err(|e| sqlite_err("query_live_presence_row", e))?;
            if let Some(ex) = exclude_session {
                if !ex.is_empty() && peer.session_id == ex {
                    continue;
                }
            }
            out.push(peer);
        }
        Ok(out)
    }

    /// Returns aggregate counts for `lore status`.
    pub fn stats(&self, now_ms: i64) -> Result<DbStats> {
        let hot_claims: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM claims WHERE tier = ?1",
                params![ClaimTier::Hot.as_i32()],
                |r| r.get(0),
            )
            .map_err(|e| sqlite_err("stats_hot", e))?;
        let cold_claims: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM claims WHERE tier = ?1",
                params![ClaimTier::ColdArchived.as_i32()],
                |r| r.get(0),
            )
            .map_err(|e| sqlite_err("stats_cold", e))?;
        let active_claims: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM claims WHERE status = ?1",
                params![ClaimStatus::Active as i32],
                |r| r.get(0),
            )
            .map_err(|e| sqlite_err("stats_active", e))?;
        let live_peers: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM live_presence WHERE (updated_at_ms + ttl_ms) >= ?1",
                params![now_ms],
                |r| r.get(0),
            )
            .map_err(|e| sqlite_err("stats_peers", e))?;
        let applied_seq = self.applied_seq()?;
        let last_gc_ms = self
            .get_meta("last_gc_ms")?
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0);

        Ok(DbStats {
            hot_claims: hot_claims.max(0) as u64,
            cold_claims: cold_claims.max(0) as u64,
            active_claims: active_claims.max(0) as u64,
            live_peers: live_peers.max(0) as u64,
            applied_seq,
            last_gc_ms,
        })
    }
}

/// Detects path and normalized `err:<sig>` overlaps between the caller and active peers (`AW-3`).
pub fn detect_overlaps(
    caller_repos: &[String],
    caller_paths: &[String],
    caller_errors: &[String],
    peers: &[PeerPresence],
) -> Vec<PathOrErrorOverlap> {
    let caller_repo_set: HashSet<&str> = caller_repos.iter().map(String::as_str).collect();
    let caller_path_set: HashSet<&str> = caller_paths.iter().map(String::as_str).collect();
    let caller_err_set: HashSet<&str> = caller_errors.iter().map(String::as_str).collect();

    let mut overlaps = Vec::new();
    for peer in peers {
        let shared_repo = peer
            .mounted_repos
            .iter()
            .find(|r| caller_repo_set.contains(r.repo_id.as_str()))
            .map(|r| r.repo_id.clone());

        let overlapping_paths: Vec<String> = if shared_repo.is_some() || caller_repo_set.is_empty() {
            peer.touched_paths
                .iter()
                .filter(|p| caller_path_set.contains(p.as_str()))
                .cloned()
                .collect()
        } else {
            Vec::new()
        };

        let overlapping_errors: Vec<String> = peer
            .recent_failures
            .iter()
            .filter(|e| caller_err_set.contains(e.as_str()))
            .cloned()
            .collect();

        if !overlapping_paths.is_empty() || !overlapping_errors.is_empty() {
            overlaps.push(PathOrErrorOverlap {
                peer_session_id: peer.session_id.clone(),
                peer_machine_id: peer.machine_id.clone(),
                peer_agent_kind: peer.agent_kind.clone(),
                repo_id: shared_repo,
                overlapping_paths,
                overlapping_errors,
            });
        }
    }

    overlaps
}

fn apply_event_in_tx(
    tx: &Transaction<'_>,
    event: &LogEvent,
    archive_loc: Option<&ArchiveRecordLoc>,
) -> Result<()> {
    if !event.event_id.is_empty() {
        let already: Option<i64> = tx
            .query_row(
                "SELECT 1 FROM seen_events WHERE event_id = ?1",
                params![event.event_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| sqlite_err("check_seen_event", e))?;
        if already.is_some() {
            update_applied_seq_in_tx(tx, event.seq)?;
            return Ok(());
        }
        tx.execute(
            "INSERT INTO seen_events(event_id) VALUES (?1)",
            params![event.event_id],
        )
        .map_err(|e| sqlite_err("insert_seen_event", e))?;
    }

    match &event.payload {
        Some(log_event::Payload::ClaimUpserted(up)) => {
            if let Some(claim) = &up.claim {
                upsert_claim_in_tx(tx, claim, event.recorded_at_ms, archive_loc)?;
            }
        }
        Some(log_event::Payload::ClaimTransitioned(tr)) => {
            tx.execute(
                "UPDATE claims
                 SET status = ?2,
                     updated_at_ms = CASE WHEN updated_at_ms < ?3 THEN ?3 ELSE updated_at_ms END
                 WHERE id = ?1",
                params![tr.claim_id, tr.new_status, event.recorded_at_ms],
            )
            .map_err(|e| sqlite_err("apply_claim_transitioned", e))?;
        }
        Some(log_event::Payload::EdgeAdded(edge)) => {
            let author_id = event
                .actor
                .as_ref()
                .map(|a| a.id.as_str())
                .unwrap_or("");
            tx.execute(
                "INSERT INTO claim_edges(src_id, rel, dst_id, created_at_ms, author_id, note)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(src_id, rel, dst_id) DO UPDATE SET
                    created_at_ms = excluded.created_at_ms,
                    author_id = excluded.author_id,
                    note = excluded.note",
                params![
                    edge.src_id,
                    edge.rel,
                    edge.dst_id,
                    event.recorded_at_ms,
                    author_id,
                    edge.note,
                ],
            )
            .map_err(|e| sqlite_err("apply_edge_added", e))?;
        }
        Some(log_event::Payload::FeedbackRecorded(fb)) => {
            let outcome = OutcomeKind::try_from(fb.outcome).unwrap_or(OutcomeKind::Unspecified);
            match outcome {
                OutcomeKind::Helpful => {
                    tx.execute(
                        "UPDATE claims
                         SET helpful_count = helpful_count + 1,
                             last_validated_at_ms = CASE
                                 WHEN last_validated_at_ms < ?2 THEN ?2
                                 ELSE last_validated_at_ms
                             END
                         WHERE id = ?1",
                        params![fb.claim_id, event.recorded_at_ms],
                    )
                    .map_err(|e| sqlite_err("apply_feedback_helpful", e))?;
                }
                OutcomeKind::Harmful => {
                    tx.execute(
                        "UPDATE claims SET harmful_count = harmful_count + 1 WHERE id = ?1",
                        params![fb.claim_id],
                    )
                    .map_err(|e| sqlite_err("apply_feedback_harmful", e))?;
                }
                OutcomeKind::NotApplicable => {
                    tx.execute(
                        "UPDATE claims SET not_applicable_count = not_applicable_count + 1 WHERE id = ?1",
                        params![fb.claim_id],
                    )
                    .map_err(|e| sqlite_err("apply_feedback_na", e))?;
                }
                OutcomeKind::Corroborated => {
                    // MOD-5: deduplicate by (claim_id, session_id, machine_id)
                    let (sess_id, mach_id) = match &event.actor {
                        Some(a) => (a.session_id.as_str(), a.machine_id.as_str()),
                        None => ("", ""),
                    };
                    let inserted = tx
                        .execute(
                            "INSERT OR IGNORE INTO claim_corroborators(claim_id, session_id, machine_id)
                             VALUES (?1, ?2, ?3)",
                            params![fb.claim_id, sess_id, mach_id],
                        )
                        .map_err(|e| sqlite_err("apply_feedback_corroborator_insert", e))?;

                    if inserted > 0 {
                        tx.execute(
                            "UPDATE claims
                             SET corroborator_count = corroborator_count + 1,
                                 last_validated_at_ms = CASE
                                     WHEN last_validated_at_ms < ?2 THEN ?2
                                     ELSE last_validated_at_ms
                                 END,
                                 tainted = CASE
                                     WHEN status = ?3 THEN 0
                                     ELSE tainted
                                 END,
                                 status = CASE
                                     WHEN status = ?3 THEN ?4
                                     ELSE status
                                 END
                             WHERE id = ?1",
                            params![
                                fb.claim_id,
                                event.recorded_at_ms,
                                ClaimStatus::Quarantined as i32,
                                ClaimStatus::Active as i32,
                            ],
                        )
                        .map_err(|e| sqlite_err("apply_feedback_corroborated", e))?;
                    }
                }
                OutcomeKind::Disputed => {
                    tx.execute(
                        "UPDATE claims SET dispute_count = dispute_count + 1 WHERE id = ?1",
                        params![fb.claim_id],
                    )
                    .map_err(|e| sqlite_err("apply_feedback_disputed", e))?;
                }
                OutcomeKind::Unspecified => {}
            }
        }
        Some(log_event::Payload::ClaimForgotten(fg)) => {
            purge_claim_in_tx(tx, &fg.claim_id)?;
        }
        None => {}
    }

    update_applied_seq_in_tx(tx, event.seq)?;
    Ok(())
}

fn update_applied_seq_in_tx(tx: &Transaction<'_>, seq: u64) -> Result<()> {
    let cur: u64 = tx
        .query_row(
            "SELECT value FROM meta WHERE key = 'applied_seq'",
            [],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| sqlite_err("read_applied_seq", e))?
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);

    if seq > cur {
        tx.execute(
            "INSERT INTO meta(key, value) VALUES ('applied_seq', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![seq.to_string()],
        )
        .map_err(|e| sqlite_err("update_applied_seq", e))?;
    }
    Ok(())
}

fn upsert_claim_in_tx(
    tx: &Transaction<'_>,
    claim: &Claim,
    recorded_at_ms: i64,
    archive_loc: Option<&ArchiveRecordLoc>,
) -> Result<()> {
    let tier = if archive_loc.is_some() {
        ClaimTier::ColdArchived
    } else {
        ClaimTier::Hot
    };

    let created = if claim.created_at_ms > 0 {
        claim.created_at_ms
    } else {
        recorded_at_ms
    };
    let updated = if claim.updated_at_ms > 0 {
        claim.updated_at_ms
    } else {
        recorded_at_ms
    };
    let importance = if claim.importance > 0.0 {
        claim.importance
    } else {
        1.0
    };

    let applies_when_json =
        serde_json::to_string(&claim.applies_when).unwrap_or_else(|_| "[]".into());
    let author_json = claim
        .author
        .as_ref()
        .and_then(|a| serde_json::to_string(a).ok());

    let (body, dead_ends_json, keywords_text, attrs_json, origin_json, verifiers_json) =
        if tier == ClaimTier::ColdArchived {
            (None, None, None, None, None, None)
        } else {
            (
                Some(claim.body.clone()),
                Some(serde_json::to_string(&claim.dead_ends).unwrap_or_else(|_| "[]".into())),
                Some(claim.keywords.join(" ")),
                Some(serde_json::to_string(&claim.attrs).unwrap_or_else(|_| "{}".into())),
                claim
                    .origin
                    .as_ref()
                    .and_then(|o| serde_json::to_string(o).ok()),
                Some(serde_json::to_string(&claim.verifiers).unwrap_or_else(|_| "[]".into())),
            )
        };

    let (arch_seg, arch_frame, arch_rec, arch_len) = match archive_loc {
        Some(loc) => (
            Some(loc.archive_seg.clone()),
            Some(loc.frame_offset as i64),
            Some(loc.record_offset as i64),
            Some(loc.record_len as i64),
        ),
        None => (None, None, None, None),
    };

    tx.execute(
        "INSERT INTO claims(
            id, kind, status, tier, scope, statement, body, dead_ends_json,
            keywords_text, attrs_json, applies_when_json, author_json, origin_json,
            verifiers_json, tainted, pinned, importance,
            corroborator_count, dispute_count, surfaced_count, helpful_count,
            harmful_count, not_applicable_count, created_at_ms, updated_at_ms,
            valid_from_ms, valid_until_ms, last_surfaced_at_ms, last_validated_at_ms,
            archive_seg, archive_frame_off, archive_rec_off, archive_rec_len
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
            ?9, ?10, ?11, ?12, ?13,
            ?14, ?15, ?16, ?17,
            0, 0, 0, 0,
            0, 0, ?18, ?19,
            ?20, ?21, 0, ?19,
            ?22, ?23, ?24, ?25
        )
        ON CONFLICT(id) DO UPDATE SET
            kind = excluded.kind,
            status = excluded.status,
            tier = excluded.tier,
            scope = excluded.scope,
            statement = excluded.statement,
            body = excluded.body,
            dead_ends_json = excluded.dead_ends_json,
            keywords_text = excluded.keywords_text,
            attrs_json = excluded.attrs_json,
            applies_when_json = excluded.applies_when_json,
            author_json = excluded.author_json,
            origin_json = excluded.origin_json,
            verifiers_json = excluded.verifiers_json,
            tainted = excluded.tainted,
            pinned = excluded.pinned,
            importance = excluded.importance,
            updated_at_ms = excluded.updated_at_ms,
            valid_from_ms = excluded.valid_from_ms,
            valid_until_ms = excluded.valid_until_ms,
            last_validated_at_ms = CASE
                WHEN claims.last_validated_at_ms < excluded.last_validated_at_ms
                THEN excluded.last_validated_at_ms
                ELSE claims.last_validated_at_ms
            END,
            archive_seg = excluded.archive_seg,
            archive_frame_off = excluded.archive_frame_off,
            archive_rec_off = excluded.archive_rec_off,
            archive_rec_len = excluded.archive_rec_len",
        params![
            claim.id,
            claim.kind,
            claim.status,
            tier.as_i32(),
            claim.scope,
            claim.statement,
            body,
            dead_ends_json,
            keywords_text,
            attrs_json,
            applies_when_json,
            author_json,
            origin_json,
            verifiers_json,
            i32::from(claim.tainted),
            i32::from(claim.pinned),
            f64::from(importance),
            created,
            updated,
            claim.valid_from_ms,
            claim.valid_until_ms,
            arch_seg,
            arch_frame,
            arch_rec,
            arch_len,
        ],
    )
    .map_err(|e| sqlite_err("upsert_claim", e))?;

    tx.execute(
        "DELETE FROM claim_entities WHERE claim_id = ?1",
        params![claim.id],
    )
    .map_err(|e| sqlite_err("delete_claim_entities", e))?;

    for entity in &claim.about {
        if !entity.is_empty() {
            tx.execute(
                "INSERT OR IGNORE INTO claim_entities(entity_uri, claim_id) VALUES (?1, ?2)",
                params![entity, claim.id],
            )
            .map_err(|e| sqlite_err("insert_claim_entity", e))?;
        }
    }

    if tier == ClaimTier::Hot {
        reindex_claim_fts_in_tx(tx, claim)?;
    } else {
        tx.execute(
            "DELETE FROM claims_fts WHERE claim_id = ?1",
            params![claim.id],
        )
        .map_err(|e| sqlite_err("delete_cold_fts", e))?;
    }

    Ok(())
}

fn reindex_claim_fts_in_tx(tx: &Transaction<'_>, claim: &Claim) -> Result<()> {
    tx.execute(
        "DELETE FROM claims_fts WHERE claim_id = ?1",
        params![claim.id],
    )
    .map_err(|e| sqlite_err("fts_delete", e))?;

    let stmt_tok = lore_context::tokenize_code(&claim.statement);
    let mut kw_about_raw = claim.keywords.join(" ");
    if !claim.about.is_empty() {
        if !kw_about_raw.is_empty() {
            kw_about_raw.push(' ');
        }
        kw_about_raw.push_str(&claim.about.join(" "));
    }
    let kw_about_tok = lore_context::tokenize_code(&kw_about_raw);
    let dead_ends_tok = lore_context::tokenize_code(&claim.dead_ends.join(" "));
    let body_tok = lore_context::tokenize_code(&claim.body);

    tx.execute(
        "INSERT INTO claims_fts(claim_id, statement, keywords_about, dead_ends, body)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![claim.id, stmt_tok, kw_about_tok, dead_ends_tok, body_tok],
    )
    .map_err(|e| sqlite_err("fts_insert", e))?;
    Ok(())
}

fn purge_claim_in_tx(tx: &Transaction<'_>, claim_id: &str) -> Result<()> {
    tx.execute("DELETE FROM claims_fts WHERE claim_id = ?1", params![claim_id])
        .map_err(|e| sqlite_err("purge_fts", e))?;
    tx.execute(
        "DELETE FROM claim_entities WHERE claim_id = ?1",
        params![claim_id],
    )
    .map_err(|e| sqlite_err("purge_entities", e))?;
    tx.execute(
        "DELETE FROM claim_edges WHERE src_id = ?1 OR dst_id = ?1",
        params![claim_id],
    )
    .map_err(|e| sqlite_err("purge_edges", e))?;
    tx.execute(
        "DELETE FROM claim_corroborators WHERE claim_id = ?1",
        params![claim_id],
    )
    .map_err(|e| sqlite_err("purge_corroborators", e))?;
    tx.execute(
        "DELETE FROM claim_exposures WHERE claim_id = ?1",
        params![claim_id],
    )
    .map_err(|e| sqlite_err("purge_exposures", e))?;
    tx.execute("DELETE FROM claims WHERE id = ?1", params![claim_id])
        .map_err(|e| sqlite_err("purge_claims", e))?;
    Ok(())
}

fn row_to_claim_without_about(row: &rusqlite::Row<'_>) -> rusqlite::Result<ClaimRow> {
    let kind_i32: i32 = row.get(1)?;
    let status_i32: i32 = row.get(2)?;
    let tier_i32: i32 = row.get(3)?;
    let body: Option<String> = row.get(6)?;
    let dead_ends_json: Option<String> = row.get(7)?;
    let keywords_text: Option<String> = row.get(8)?;
    let attrs_json: Option<String> = row.get(9)?;
    let applies_when_json: Option<String> = row.get(10)?;
    let author_json: Option<String> = row.get(11)?;
    let origin_json: Option<String> = row.get(12)?;
    let verifiers_json: Option<String> = row.get(13)?;
    let tainted_i32: i32 = row.get(14)?;
    let pinned_i32: i32 = row.get(15)?;
    let importance_f64: f64 = row.get(16)?;

    let archive_seg: Option<String> = row.get(29)?;
    let archive_frame_off: Option<i64> = row.get(30)?;
    let archive_rec_off: Option<i64> = row.get(31)?;
    let archive_rec_len: Option<i64> = row.get(32)?;

    Ok(ClaimRow {
        id: row.get(0)?,
        kind: ClaimKind::try_from(kind_i32).unwrap_or(ClaimKind::Unspecified),
        status: ClaimStatus::try_from(status_i32).unwrap_or(ClaimStatus::Unspecified),
        tier: ClaimTier::from_i32(tier_i32).unwrap_or(ClaimTier::Hot),
        scope: row.get(4)?,
        statement: row.get(5)?,
        body,
        dead_ends: dead_ends_json
            .as_deref()
            .and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
            .unwrap_or_default(),
        keywords: keywords_text
            .as_deref()
            .map(|s| s.split_whitespace().map(str::to_string).collect())
            .unwrap_or_default(),
        attrs: attrs_json
            .as_deref()
            .and_then(|s| serde_json::from_str::<BTreeMap<String, String>>(s).ok())
            .unwrap_or_default(),
        about: Vec::new(),
        applies_when: applies_when_json
            .as_deref()
            .and_then(|s| serde_json::from_str::<Vec<Predicate>>(s).ok())
            .unwrap_or_default(),
        author: author_json
            .as_deref()
            .and_then(|s| serde_json::from_str::<Principal>(s).ok()),
        origin: origin_json
            .as_deref()
            .and_then(|s| serde_json::from_str::<OriginSnapshot>(s).ok()),
        verifiers: verifiers_json
            .as_deref()
            .and_then(|s| serde_json::from_str::<Vec<Verifier>>(s).ok())
            .unwrap_or_default(),
        tainted: tainted_i32 != 0,
        pinned: pinned_i32 != 0,
        importance: importance_f64 as f32,
        counters: ClaimCounters {
            corroborator_count: row.get(17)?,
            dispute_count: row.get(18)?,
            surfaced_count: row.get(19)?,
            helpful_count: row.get(20)?,
            harmful_count: row.get(21)?,
            not_applicable_count: row.get(22)?,
            last_surfaced_at_ms: row.get(27)?,
            last_validated_at_ms: row.get(28)?,
        },
        created_at_ms: row.get(23)?,
        updated_at_ms: row.get(24)?,
        valid_from_ms: row.get(25)?,
        valid_until_ms: row.get(26)?,
        archive_seg,
        archive_frame_off: archive_frame_off.and_then(|v| u64::try_from(v).ok()),
        archive_rec_off: archive_rec_off.and_then(|v| u64::try_from(v).ok()),
        archive_rec_len: archive_rec_len.and_then(|v| u64::try_from(v).ok()),
    })
}
