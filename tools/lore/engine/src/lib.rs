//! Core storage, projection, scoring, live-presence, and `(status, age)` `zstd`
//! GC engine for `lore` (`SPEC.md` M1–M2).

pub mod blobs;
pub mod config;
pub mod db;
pub mod gc;
pub mod log;
pub mod redact;
pub mod scoring;

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use lore_context::{
    ContextFrame, VerifierResult, evaluate_all, normalize_error_signature, normalize_path_to_repo,
    permitted_scopes, verify_all,
};
use lore_proto::{
    Claim, ClaimKind, ClaimStatus, ClaimTier, ClaimTransitioned, ClaimUpserted, Clock, EdgeAdded,
    EdgeRel, FeedbackRecorded, L0Card, LogEvent, LoreError, MAX_STATEMENT_CHARS, OriginSnapshot,
    OutcomeKind, PathOrErrorOverlap, PeerPresence, PeerRepoMount, Predicate, PredicateEval,
    Principal, PrincipalKind, Result, Verifier, VerifierStatus, log_event, new_claim_id,
    new_event_id,
};
use serde::{Deserialize, Serialize};

pub use blobs::{blob_exists, get_blob, list_blob_hashes, put_blob};
pub use config::{LoreConfig, MS_PER_DAY, MemoryConfig, RetentionConfig, ensure_enabled, load_config, save_config};
pub use db::{ClaimCounters, ClaimRow, DbStats, EdgeRecord, ProjectionDb};
pub use gc::{GcReport, PurgeCategory, is_tier0_exempt, is_tier1_cold_eligible, tier2_purge_category};
pub use log::{RecoveryReport, VerifyReport, stream_log_events, verify_all_logs};
pub use redact::{scan_or_redact_claim, scan_or_redact_field};
pub use scoring::{
    DRIFTED_VERIFIER_PENALTY, DefaultScoringPolicy, ScoringInput, ScoringPolicy, build_l0_card,
    claim_kind_str, code_precedence_notice, format_evidence_envelope, trust_badge,
    verifier_status_str,
};

/// Default `recall` card limit (`RET-1`).
pub const DEFAULT_RECALL_LIMIT: usize = 8;

/// Hard maximum `recall` card limit (`RET-1`).
pub const MAX_RECALL_LIMIT: usize = 20;

/// Default live presence TTL (15 minutes, `AW-2`).
pub const DEFAULT_PRESENCE_TTL_MS: i64 = 15 * 60 * 1000;

/// Default live presence TTL when reporting recent failures (30 minutes, `AW-2`).
pub const DEFAULT_FAILURE_PRESENCE_TTL_MS: i64 = 30 * 60 * 1000;

/// Status snapshot returned by [`LoreEngine::status`] (`SW-5`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineStatus {
    pub enabled: bool,
    pub disabled_reason: Option<String>,
    pub hot_claims: u64,
    pub cold_claims: u64,
    pub active_claims: u64,
    pub live_peers: u64,
    pub applied_seq: u64,
    pub hot_bytes: u64,
    pub archive_bytes: u64,
    pub blob_bytes: u64,
    pub index_bytes: u64,
    pub last_gc_ms: i64,
}

/// Request for [`LoreEngine::recall`] (`RET-1`..`RET-4`).
#[derive(Clone, Debug)]
pub struct RecallRequest {
    pub frame: ContextFrame,
    pub query: String,
    pub about: Vec<String>,
    pub kinds: Option<Vec<ClaimKind>>,
    pub scope_override: Option<String>,
    pub limit: Option<usize>,
    pub include_history: bool,
    pub include_pulse: bool,
    /// When `true` (proactive hook mode), suppresses claims already surfaced to
    /// `frame.session_id` unless updated since last exposure (`RET-3`).
    pub suppress_exposed: bool,
    /// When `true` (default), records returned cards in `claim_exposures`.
    pub record_exposures: bool,
    /// Optional caller touched paths for peer overlap detection.
    pub touched_paths: Vec<String>,
    /// Optional caller recent error signatures for peer overlap detection.
    pub recent_failures: Vec<String>,
}

impl RecallRequest {
    pub fn new(frame: ContextFrame) -> Self {
        Self {
            frame,
            query: String::new(),
            about: Vec::new(),
            kinds: None,
            scope_override: None,
            limit: None,
            include_history: false,
            include_pulse: true,
            suppress_exposed: false,
            record_exposures: true,
            touched_paths: Vec::new(),
            recent_failures: Vec::new(),
        }
    }
}

/// Response from [`LoreEngine::recall`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecallResponse {
    pub enabled: bool,
    pub cards: Vec<L0Card>,
    pub peers: Vec<PeerPresence>,
    pub overlaps: Vec<PathOrErrorOverlap>,
    pub notice: Option<String>,
}

/// Request for [`LoreEngine::inspect`] (`RET-1`, `RET-2`, `GC-3`).
#[derive(Clone, Debug)]
pub struct InspectRequest {
    pub id: String,
    pub frame: Option<ContextFrame>,
    pub include_lineage: bool,
}

/// Response from [`LoreEngine::inspect`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InspectResponse {
    pub enabled: bool,
    pub claim: Claim,
    pub counters: ClaimCounters,
    pub edges: Vec<EdgeRecord>,
    pub verifier_status: VerifierStatus,
    pub verifier_results: Vec<VerifierResult>,
    pub hydrated_from_cold: bool,
}

/// Request for [`LoreEngine::remember`] (`MOD-1`, `MOD-2`, `SEC-1`, `SEC-2`).
#[derive(Clone, Debug)]
pub struct RememberRequest {
    pub kind: ClaimKind,
    pub statement: String,
    pub body: String,
    pub dead_ends: Vec<String>,
    pub keywords: Vec<String>,
    pub attrs: BTreeMap<String, String>,
    pub scope: String,
    pub about: Vec<String>,
    pub applies_when: Vec<Predicate>,
    pub author: Principal,
    pub tainted: bool,
    pub origin: Option<OriginSnapshot>,
    pub verifiers: Vec<Verifier>,
    pub supersedes: Option<String>,
    pub pinned: bool,
    pub importance: f32,
    pub valid_from_ms: i64,
    pub valid_until_ms: i64,
    pub redact_secrets: bool,
    /// Optional client-supplied idempotency key (`BR-2`).
    pub event_id: Option<String>,
}

impl Default for RememberRequest {
    fn default() -> Self {
        Self {
            kind: ClaimKind::Fact,
            statement: String::new(),
            body: String::new(),
            dead_ends: Vec::new(),
            keywords: Vec::new(),
            attrs: BTreeMap::new(),
            scope: "global".to_string(),
            about: Vec::new(),
            applies_when: Vec::new(),
            author: Principal {
                kind: PrincipalKind::Agent as i32,
                id: "agent".to_string(),
                session_id: String::new(),
                agent_kind: "agy".to_string(),
                model: String::new(),
                machine_id: "local".to_string(),
            },
            tainted: false,
            origin: None,
            verifiers: Vec::new(),
            supersedes: None,
            pinned: false,
            importance: 1.0,
            valid_from_ms: 0,
            valid_until_ms: 0,
            redact_secrets: false,
            event_id: None,
        }
    }
}

/// Response from [`LoreEngine::remember`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RememberResponse {
    pub enabled: bool,
    pub id: String,
    pub status: ClaimStatus,
    pub superseded_id: Option<String>,
    pub redacted_fields: Vec<String>,
}

/// Action supported by [`LoreEngine::feedback`] (`SPEC.md §5.1`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FeedbackAction {
    Helpful,
    Harmful,
    NotApplicable,
    Corroborated,
    Disputed,
    Invalidate,
    Retract,
}

/// Request for [`LoreEngine::feedback`].
#[derive(Clone, Debug)]
pub struct FeedbackRequest {
    pub id: String,
    pub action: FeedbackAction,
    pub note: String,
    pub actor: Principal,
    pub tainted: bool,
    pub event_id: Option<String>,
}

/// Response from [`LoreEngine::feedback`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedbackResponse {
    pub enabled: bool,
    pub id: String,
    pub new_status: ClaimStatus,
    pub counters: ClaimCounters,
    pub promoted_from_cold: bool,
}

/// Request for [`LoreEngine::pulse`] (`AW-1`..`AW-4`).
#[derive(Clone, Debug)]
pub struct PulseRequest {
    pub frame: ContextFrame,
    pub intent: Option<String>,
    pub hypothesis: Option<String>,
    pub touched_paths: Vec<String>,
    pub recent_failures: Vec<String>,
    pub ttl_ms: Option<i64>,
    pub query_only: bool,
    pub dirty_count: u32,
}

impl PulseRequest {
    pub fn new(frame: ContextFrame) -> Self {
        Self {
            frame,
            intent: None,
            hypothesis: None,
            touched_paths: Vec::new(),
            recent_failures: Vec::new(),
            ttl_ms: None,
            query_only: false,
            dirty_count: 0,
        }
    }
}

/// Response from [`LoreEngine::pulse`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PulseResponse {
    pub enabled: bool,
    pub peers: Vec<PeerPresence>,
    pub overlaps: Vec<PathOrErrorOverlap>,
}

/// Verifies all hot `.pblog` segments and cold `.pblog.zst` archives strictly
/// read-only without acquiring `write.lock` or truncating torn tails (`ST-5`, `#7a`).
pub fn verify_store(root: impl AsRef<Path>) -> Result<VerifyReport> {
    log::verify_all_logs(root.as_ref())
}

/// Streams all [`LogEvent`]s with `seq > since_seq` strictly read-only without
/// acquiring `write.lock` or truncating torn tails (`ST-7`, `#7a`).
pub fn export_store_events(root: impl AsRef<Path>, since_seq: u64) -> Result<Vec<LogEvent>> {
    log::stream_log_events(root.as_ref(), since_seq)
}

/// Returns store status strictly read-only without acquiring `write.lock`,
/// creating `index.db`, or truncating torn tails (`SW-5`, `#7a`).
pub fn read_only_status(root: impl AsRef<Path>, clock: &dyn Clock) -> Result<EngineStatus> {
    let root = root.as_ref();
    let config = load_config(root)?;
    let db = ProjectionDb::open_read_only(root, 50)?;
    let stats = db.stats(clock.now_ms())?;
    let hot_bytes = log::total_hot_bytes(root)?;
    let archive_bytes = log::total_archive_bytes(root)?;
    let blob_bytes = blobs::total_blob_bytes(root)?;
    let index_bytes = fs::metadata(db::index_db_path(root))
        .map(|m| m.len())
        .unwrap_or(0);

    Ok(EngineStatus {
        enabled: config.memory.enabled,
        disabled_reason: config.memory.disabled_reason,
        hot_claims: stats.hot_claims,
        cold_claims: stats.cold_claims,
        active_claims: stats.active_claims,
        live_peers: stats.live_peers,
        applied_seq: stats.applied_seq,
        hot_bytes,
        archive_bytes,
        blob_bytes,
        index_bytes,
        last_gc_ms: stats.last_gc_ms,
    })
}

/// Primary `lore` storage, indexing, retrieval, and GC engine.
pub struct LoreEngine<C: Clock> {
    root: PathBuf,
    clock: C,
    config: LoreConfig,
    db: ProjectionDb,
    active_seg: Option<PathBuf>,
    next_seq: u64,
    recovery: RecoveryReport,
    scoring_policy: Box<dyn ScoringPolicy>,
    read_only: bool,
}

impl<C: Clock> LoreEngine<C> {
    /// Opens a `lore` store at `root`, recovering any torn hot-segment tail (`ST-4`)
    /// and replaying any log events with `seq > index.applied_seq` (`ST-1`).
    pub fn open(root: impl AsRef<Path>, clock: C) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root).map_err(|source| LoreError::Io {
            path: root.clone(),
            source,
        })?;

        let config = load_config(&root)?;
        let _lock = log::WriteLock::acquire(&root)?;
        let (recovery, active_seg) = log::recover_hot_segments(&root)?;
        let mut db = ProjectionDb::open(&root)?;

        // Replay any unapplied events from archives + hot segments.
        let applied_seq = db.applied_seq()?;
        let all_with_locs = log::stream_events_with_archive_locs(&root)?;
        let mut max_seq = recovery.max_seq.max(applied_seq);
        let pending: Vec<(LogEvent, Option<log::ArchiveRecordLoc>)> = all_with_locs
            .into_iter()
            .inspect(|(ev, _)| {
                if ev.seq > max_seq {
                    max_seq = ev.seq;
                }
            })
            .filter(|(ev, _)| ev.seq > applied_seq)
            .collect();

        if !pending.is_empty() {
            db.apply_events_batch(&pending)?;
        }

        Ok(Self {
            root,
            clock,
            config,
            db,
            active_seg,
            next_seq: max_seq + 1,
            recovery,
            scoring_policy: Box::new(DefaultScoringPolicy::default()),
            read_only: false,
        })
    }

    /// Opens a `lore` store strictly read-only with a short SQLite `busy_timeout_ms`
    /// (for latency-capped hooks and read-only CLI queries). Does NOT acquire
    /// `write.lock`, does NOT truncate torn tails, and skips exposure writes (`#7a`, `#7c`).
    pub fn open_read_only(root: impl AsRef<Path>, clock: C, busy_timeout_ms: u64) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        let config = load_config(&root)?;
        let db = ProjectionDb::open_read_only(&root, busy_timeout_ms)?;
        let applied_seq = db.applied_seq().unwrap_or(0);
        Ok(Self {
            root,
            clock,
            config,
            db,
            active_seg: None,
            next_seq: applied_seq + 1,
            recovery: RecoveryReport::default(),
            scoring_policy: Box::new(DefaultScoringPolicy::default()),
            read_only: true,
        })
    }

    /// Installs a custom [`ScoringPolicy`] (`MOD-4`).
    pub fn with_scoring_policy(mut self, policy: Box<dyn ScoringPolicy>) -> Self {
        self.scoring_policy = policy;
        self
    }

    /// Returns the store root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the loaded [`LoreConfig`].
    pub fn config(&self) -> &LoreConfig {
        &self.config
    }

    /// Returns the startup torn-tail recovery report (`ST-4`).
    pub fn recovery_report(&self) -> &RecoveryReport {
        &self.recovery
    }

    /// Updates `[memory] enabled` and optional `disabled_reason` atomically (`SW-1`, `SW-2`, `SW-5`).
    pub fn set_enabled(&mut self, enabled: bool, reason: Option<String>) -> Result<LoreConfig> {
        self.config.memory.enabled = enabled;
        self.config.memory.disabled_reason = if enabled { None } else { reason };
        save_config(&self.root, &self.config)?;
        Ok(self.config.clone())
    }

    /// Updates the full [`LoreConfig`] atomically (`SW-5`).
    pub fn update_config(&mut self, config: LoreConfig) -> Result<()> {
        save_config(&self.root, &config)?;
        self.config = config;
        Ok(())
    }

    /// Returns store status, remaining operable when `memory.enabled == false` (`SW-5`).
    pub fn status(&self) -> Result<EngineStatus> {
        let live_cfg = load_config(&self.root)?;
        let now_ms = self.clock.now_ms();
        let stats = self.db.stats(now_ms)?;
        let hot_bytes = log::total_hot_bytes(&self.root)?;
        let archive_bytes = log::total_archive_bytes(&self.root)?;
        let blob_bytes = blobs::total_blob_bytes(&self.root)?;
        let index_bytes = fs::metadata(db::index_db_path(&self.root))
            .map(|m| m.len())
            .unwrap_or(0);

        Ok(EngineStatus {
            enabled: live_cfg.memory.enabled,
            disabled_reason: live_cfg.memory.disabled_reason,
            hot_claims: stats.hot_claims,
            cold_claims: stats.cold_claims,
            active_claims: stats.active_claims,
            live_peers: stats.live_peers,
            applied_seq: stats.applied_seq,
            hot_bytes,
            archive_bytes,
            blob_bytes,
            index_bytes,
            last_gc_ms: stats.last_gc_ms,
        })
    }

    /// Stores a content-addressed evidence blob (`ST-8`).
    pub fn put_blob(&self, raw: &[u8]) -> Result<String> {
        let live_cfg = load_config(&self.root)?;
        ensure_enabled(&live_cfg)?;
        blobs::put_blob(&self.root, raw)
    }

    /// Reads a content-addressed evidence blob (`ST-8`).
    pub fn get_blob(&self, sha256: &str) -> Result<Vec<u8>> {
        blobs::get_blob(&self.root, sha256)
    }

    /// Reloads `config.toml` from disk and enforces `memory.enabled == true` (`#7a`).
    fn ensure_live_enabled(&mut self) -> Result<()> {
        self.config = load_config(&self.root)?;
        ensure_enabled(&self.config)
    }

    /// Refreshes `active_seg`, replays any events appended by concurrent processes
    /// (`seq > applied_seq`), and advances `next_seq` past the on-disk/DB head (`#1`).
    ///
    /// Caller must hold `WriteLock`.
    fn sync_under_lock(&mut self) -> Result<()> {
        self.active_seg = log::list_hot_segments(&self.root)?.last().cloned();
        let applied_seq = self.db.applied_seq()?;
        let all_with_locs = log::stream_events_with_archive_locs(&self.root)?;
        let mut max_seq = applied_seq;
        let pending: Vec<(LogEvent, Option<log::ArchiveRecordLoc>)> = all_with_locs
            .into_iter()
            .inspect(|(ev, _)| {
                if ev.seq > max_seq {
                    max_seq = ev.seq;
                }
            })
            .filter(|(ev, _)| ev.seq > applied_seq)
            .collect();
        if !pending.is_empty() {
            self.db.apply_events_batch(&pending)?;
        }
        let latest = self.db.applied_seq()?.max(max_seq);
        self.next_seq = self.next_seq.max(latest + 1);
        Ok(())
    }

    /// Recalls ranked L0 cards for the caller's [`ContextFrame`] (`RET-1`..`RET-4`, `MOD-3`, `MOD-4`).
    pub fn recall(&mut self, req: RecallRequest) -> Result<RecallResponse> {
        self.ensure_live_enabled()?;
        let now_ms = self.clock.now_ms();
        let limit = req
            .limit
            .unwrap_or(DEFAULT_RECALL_LIMIT)
            .clamp(1, MAX_RECALL_LIMIT);

        // Normalize `file:<mount_subpath>/<rel>` in `req.about` and detect targeted mounted repos (`#8b-recall`).
        let (normalized_about, targeted_repos) = normalize_about_filter(&req.frame, &req.about);

        let scopes = match &req.scope_override {
            Some(s) if !s.trim().is_empty() => vec![s.trim().to_string()],
            _ => {
                let mut base = permitted_scopes(&req.frame);
                if targeted_repos.len() == 1 {
                    let target_repo = targeted_repos.iter().next().unwrap();
                    let target_scope = if target_repo.starts_with("dir:") {
                        target_repo.clone()
                    } else {
                        format!("repo:{target_repo}")
                    };
                    let mounted_repo_scopes: HashSet<String> = req
                        .frame
                        .mounted_repos
                        .iter()
                        .map(|r| {
                            if r.repo_id.starts_with("dir:") {
                                r.repo_id.clone()
                            } else {
                                format!("repo:{}", r.repo_id)
                            }
                        })
                        .collect();
                    base.retain(|s| !mounted_repo_scopes.contains(s) || s == &target_scope);
                }
                base
            }
        };

        let candidates = self.db.list_candidate_claims(
            &scopes,
            req.kinds.as_deref(),
            req.include_history,
            req.frame.session_id.as_deref(),
            now_ms,
        )?;

        let fts_scores: HashMap<String, f64> = if req.query.trim().is_empty() {
            HashMap::new()
        } else {
            self.db.search_fts(&req.query, limit.saturating_mul(8).max(64))?
        };

        let exposed: HashMap<String, i64> = if req.suppress_exposed {
            req.frame
                .session_id
                .as_deref()
                .map(|sid| self.db.exposed_claims_for_session(sid))
                .transpose()?
                .unwrap_or_default()
        } else {
            HashMap::new()
        };

        let query_terms = lore_context::tokenize_code_terms(&req.query);
        let mut scored: Vec<(L0Card, f64)> = Vec::new();

        for mut claim in candidates {
            if req.suppress_exposed {
                if let Some(&surfaced_at) = exposed.get(&claim.id) {
                    if claim.updated_at_ms <= surfaced_at {
                        continue;
                    }
                }
            }

            // Transparently hydrate cold claims when `include_history` is requested (`GC-3`).
            if claim.tier == ClaimTier::ColdArchived {
                if let Ok(hydrated) = self.hydrate_row(&claim) {
                    claim.body = Some(hydrated.body);
                    claim.dead_ends = hydrated.dead_ends;
                    claim.verifiers = hydrated.verifiers;
                    claim.origin = hydrated.origin;
                }
            }

            let pred_eval = evaluate_all(&req.frame, &claim.applies_when);
            if pred_eval == PredicateEval::Mismatch {
                continue;
            }

            let lex_score = if req.query.trim().is_empty() {
                0.5
            } else if let Some(&s) = fts_scores.get(&claim.id) {
                s
            } else if claim.tier == ClaimTier::ColdArchived {
                // Fallback lexical match for cold claims (which are not in claims_fts).
                cold_lexical_overlap(&query_terms, &claim)
            } else {
                0.0
            };

            // When a query is supplied and there is zero lexical match and no explicit entity hit,
            // skip completely unrelated noise unless `about` matched.
            if !req.query.trim().is_empty() && lex_score == 0.0 && normalized_about.is_empty() {
                continue;
            }

            let (ver_status, _ver_results) = verify_all(&req.frame, &claim.verifiers);
            let score = self.scoring_policy.score(&ScoringInput {
                claim: &claim,
                query: &req.query,
                about_filter: &normalized_about,
                frame: &req.frame,
                lex_score,
                predicate_eval: pred_eval,
                verifier_status: ver_status,
                now_ms,
            });

            if score <= 0.0 {
                continue;
            }

            let card = build_l0_card(&claim, ver_status, score);
            scored.push((card, score));
        }

        scored.sort_by(|a, b| {
            b.1.total_cmp(&a.1)
                .then_with(|| b.0.updated_at_ms.cmp(&a.0.updated_at_ms))
                .then_with(|| a.0.id.cmp(&b.0.id))
        });
        scored.truncate(limit);

        let cards: Vec<L0Card> = scored.into_iter().map(|(c, _)| c).collect();

        if req.record_exposures && !self.read_only {
            if let Some(sid) = req.frame.session_id.as_deref() {
                let ids: Vec<String> = cards.iter().map(|c| c.id.clone()).collect();
                self.db.record_exposures(sid, &ids, now_ms)?;
            }
        }

        let (peers, overlaps) = if req.include_pulse {
            let peers = self
                .db
                .query_live_presence(now_ms, req.frame.session_id.as_deref())?;
            let caller_repos: Vec<String> = req
                .frame
                .mounted_repos
                .iter()
                .map(|r| r.repo_id.clone())
                .collect();
            let norm_errors = normalize_failure_list(&req.recent_failures);
            let overlaps = db::detect_overlaps(&caller_repos, &req.touched_paths, &norm_errors, &peers);
            (peers, overlaps)
        } else {
            (Vec::new(), Vec::new())
        };

        let notice = code_precedence_notice(&cards);
        Ok(RecallResponse {
            enabled: true,
            cards,
            peers,
            overlaps,
            notice,
        })
    }

    /// Fetches full L1 claim details, transparently hydrating cold stubs (`RET-1`, `GC-3`).
    pub fn inspect(&self, req: InspectRequest) -> Result<InspectResponse> {
        let live_cfg = load_config(&self.root)?;
        ensure_enabled(&live_cfg)?;
        let row = self
            .db
            .get_claim(&req.id)?
            .ok_or_else(|| LoreError::ClaimNotFound { id: req.id.clone() })?;

        let (claim, hydrated_from_cold) = if row.tier == ClaimTier::ColdArchived {
            let mut hydrated = self.hydrate_row(&row)?;
            // Preserve latest projection status/timestamps in case of later transitions.
            hydrated.status = row.status as i32;
            hydrated.updated_at_ms = row.updated_at_ms;
            (hydrated, true)
        } else {
            (row.to_proto_claim(), false)
        };

        let (verifier_status, verifier_results) = match &req.frame {
            Some(frame) => verify_all(frame, &claim.verifiers),
            None => (VerifierStatus::UnverifiedLocal, Vec::new()),
        };

        let edges = if req.include_lineage {
            self.db.get_edges_for_claim(&req.id)?
        } else {
            Vec::new()
        };

        Ok(InspectResponse {
            enabled: true,
            claim,
            counters: row.counters,
            edges,
            verifier_status,
            verifier_results,
            hydrated_from_cold,
        })
    }

    /// Records a new durable claim, optionally superseding an existing claim (`MOD-1`, `MOD-2`, `SEC-1`, `SEC-2`).
    pub fn remember(&mut self, req: RememberRequest) -> Result<RememberResponse> {
        self.ensure_live_enabled()?;

        let stmt_len = req.statement.chars().count();
        if stmt_len > MAX_STATEMENT_CHARS {
            return Err(LoreError::StatementTooLong {
                len: stmt_len,
                max: MAX_STATEMENT_CHARS,
            });
        }
        validate_scope(&req.scope)?;

        let now_ms = self.clock.now_ms();
        let claim_id = new_claim_id(now_ms);

        let _lock = log::WriteLock::acquire(&self.root)?;
        self.sync_under_lock()?;

        // SEC-2: Propagate session taint across writes from the same `session_id`.
        let effective_tainted =
            req.tainted || self.db.is_session_tainted(&req.author.session_id)?;
        if effective_tainted && !req.author.session_id.is_empty() {
            self.db
                .mark_session_tainted(&req.author.session_id, now_ms)?;
        }

        // SEC-2: Tainted sessions cannot create AXIOM claims or supersede OWNER claims.
        if effective_tainted && req.kind == ClaimKind::Axiom {
            return Err(LoreError::TaintedPromotionDenied {
                claim_id,
                attempted: "create AXIOM claim".to_string(),
            });
        }

        if let Some(old_id) = &req.supersedes {
            let old_row = self
                .db
                .get_claim(old_id)?
                .ok_or_else(|| LoreError::ClaimNotFound { id: old_id.clone() })?;
            if effective_tainted {
                let old_is_owner = old_row
                    .author
                    .as_ref()
                    .and_then(|a| PrincipalKind::try_from(a.kind).ok())
                    == Some(PrincipalKind::Owner);
                if old_is_owner {
                    return Err(LoreError::TaintedPromotionDenied {
                        claim_id: old_id.clone(),
                        attempted: "supersede OWNER claim".to_string(),
                    });
                }
            }
        }

        let status = if effective_tainted {
            ClaimStatus::Quarantined
        } else {
            ClaimStatus::Active
        };

        let mut claim = Claim {
            id: claim_id.clone(),
            kind: req.kind as i32,
            status: status as i32,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
            valid_from_ms: req.valid_from_ms,
            valid_until_ms: req.valid_until_ms,
            review_after_ms: 0,
            statement: req.statement,
            body: req.body,
            dead_ends: req.dead_ends,
            keywords: req.keywords,
            attrs: req.attrs,
            scope: req.scope,
            about: req.about,
            applies_when: req.applies_when,
            author: Some(req.author.clone()),
            tainted: effective_tainted,
            origin: req.origin,
            verifiers: req.verifiers,
            pinned: req.pinned,
            importance: if req.importance > 0.0 {
                req.importance
            } else {
                1.0
            },
        };

        // SEC-1: Scan or redact secrets before appending to `.pblog`.
        let redacted_fields = scan_or_redact_claim(&mut claim, req.redact_secrets)?;

        // Idempotent event_id check (`BR-2`).
        let event_id = req.event_id.unwrap_or_else(|| new_event_id(now_ms));
        if self.db.has_seen_event_id(&event_id)? {
            return Ok(RememberResponse {
                enabled: true,
                id: claim_id,
                status,
                superseded_id: req.supersedes,
                redacted_fields,
            });
        }

        let mut batch_events = Vec::with_capacity(if req.supersedes.is_some() { 3 } else { 1 });
        batch_events.push(LogEvent {
            seq: self.alloc_seq(),
            event_id,
            recorded_at_ms: now_ms,
            actor: Some(req.author.clone()),
            payload: Some(log_event::Payload::ClaimUpserted(ClaimUpserted {
                claim: Some(claim),
            })),
        });

        // MOD-2 / #4: Superseding claim A with B atomically appends B, creates SUPERSEDES
        // edge B -> A, and transitions A.status = SUPERSEDED in one write+sync and one SQLite tx.
        if let Some(old_id) = &req.supersedes {
            batch_events.push(LogEvent {
                seq: self.alloc_seq(),
                event_id: new_event_id(now_ms),
                recorded_at_ms: now_ms,
                actor: Some(req.author.clone()),
                payload: Some(log_event::Payload::EdgeAdded(EdgeAdded {
                    src_id: claim_id.clone(),
                    rel: EdgeRel::Supersedes as i32,
                    dst_id: old_id.clone(),
                    note: String::new(),
                })),
            });

            batch_events.push(LogEvent {
                seq: self.alloc_seq(),
                event_id: new_event_id(now_ms),
                recorded_at_ms: now_ms,
                actor: Some(req.author),
                payload: Some(log_event::Payload::ClaimTransitioned(ClaimTransitioned {
                    claim_id: old_id.clone(),
                    new_status: ClaimStatus::Superseded as i32,
                    reason: format!("superseded by {claim_id}"),
                })),
            });
        }

        let events_written = batch_events.len() as u64;
        log::append_hot_events_batch(
            &self.root,
            &mut self.active_seg,
            &batch_events,
            log::DEFAULT_HOT_SEGMENT_MAX_BYTES,
        )?;
        let db_batch: Vec<(LogEvent, Option<log::ArchiveRecordLoc>)> =
            batch_events.into_iter().map(|ev| (ev, None)).collect();
        self.db.apply_events_batch(&db_batch)?;

        self.maybe_lazy_gc(events_written, now_ms)?;

        Ok(RememberResponse {
            enabled: true,
            id: claim_id,
            status,
            superseded_id: req.supersedes,
            redacted_fields,
        })
    }

    /// Records outcome feedback or lifecycle transition (`MOD-2`, `MOD-5`, `GC-3`).
    pub fn feedback(&mut self, mut req: FeedbackRequest) -> Result<FeedbackResponse> {
        self.ensure_live_enabled()?;
        // SEC-1 / #9: Scan feedback note/reason for secrets before persisting.
        scan_or_redact_field("note", &mut req.note, false)?;

        let now_ms = self.clock.now_ms();
        let _lock = log::WriteLock::acquire(&self.root)?;
        self.sync_under_lock()?;

        let row = self
            .db
            .get_claim(&req.id)?
            .ok_or_else(|| LoreError::ClaimNotFound { id: req.id.clone() })?;

        let effective_tainted =
            req.tainted || self.db.is_session_tainted(&req.actor.session_id)?;
        if effective_tainted && !req.actor.session_id.is_empty() {
            self.db
                .mark_session_tainted(&req.actor.session_id, now_ms)?;
        }

        let actor_is_owner =
            PrincipalKind::try_from(req.actor.kind).ok() == Some(PrincipalKind::Owner);
        let same_session = row.author.as_ref().is_some_and(|a| {
            !a.session_id.is_empty() && a.session_id == req.actor.session_id
        });
        let target_is_owner = row
            .author
            .as_ref()
            .and_then(|a| PrincipalKind::try_from(a.kind).ok())
            == Some(PrincipalKind::Owner);

        if req.action == FeedbackAction::Corroborated
            && row.status == ClaimStatus::Quarantined
            && !actor_is_owner
            && (effective_tainted || same_session)
        {
            return Err(LoreError::TaintedPromotionDenied {
                claim_id: req.id,
                attempted: "corroborate quarantined claim".to_string(),
            });
        }

        if effective_tainted
            && matches!(
                req.action,
                FeedbackAction::Invalidate | FeedbackAction::Retract
            )
            && target_is_owner
        {
            return Err(LoreError::TaintedPromotionDenied {
                claim_id: req.id,
                attempted: format!("{:?}", req.action),
            });
        }

        // GC-3: If a dormant Tier-1 Cold claim is corroborated or marked helpful,
        // hydrate it and promote back to Tier-0 Hot (re-entering `claims_fts`).
        let mut promoted_from_cold = false;
        if row.tier == ClaimTier::ColdArchived
            && matches!(
                req.action,
                FeedbackAction::Corroborated | FeedbackAction::Helpful
            )
        {
            let mut hydrated = self.hydrate_row(&row)?;
            hydrated.status = row.status as i32;
            hydrated.updated_at_ms = now_ms;

            let promo_ev = LogEvent {
                seq: self.alloc_seq(),
                event_id: new_event_id(now_ms),
                recorded_at_ms: now_ms,
                actor: Some(req.actor.clone()),
                payload: Some(log_event::Payload::ClaimUpserted(ClaimUpserted {
                    claim: Some(hydrated.clone()),
                })),
            };
            log::append_hot_event(
                &self.root,
                &mut self.active_seg,
                &promo_ev,
                log::DEFAULT_HOT_SEGMENT_MAX_BYTES,
            )?;
            self.db.apply_event(&promo_ev, None)?;
            self.db.promote_claim_hot(&hydrated, now_ms)?;
            promoted_from_cold = true;
        }

        let event_id = req.event_id.unwrap_or_else(|| new_event_id(now_ms));
        let payload = match req.action {
            FeedbackAction::Helpful => log_event::Payload::FeedbackRecorded(FeedbackRecorded {
                claim_id: req.id.clone(),
                outcome: OutcomeKind::Helpful as i32,
                note: req.note,
            }),
            FeedbackAction::Harmful => log_event::Payload::FeedbackRecorded(FeedbackRecorded {
                claim_id: req.id.clone(),
                outcome: OutcomeKind::Harmful as i32,
                note: req.note,
            }),
            FeedbackAction::NotApplicable => {
                log_event::Payload::FeedbackRecorded(FeedbackRecorded {
                    claim_id: req.id.clone(),
                    outcome: OutcomeKind::NotApplicable as i32,
                    note: req.note,
                })
            }
            FeedbackAction::Corroborated => {
                log_event::Payload::FeedbackRecorded(FeedbackRecorded {
                    claim_id: req.id.clone(),
                    outcome: OutcomeKind::Corroborated as i32,
                    note: req.note,
                })
            }
            FeedbackAction::Disputed => log_event::Payload::FeedbackRecorded(FeedbackRecorded {
                claim_id: req.id.clone(),
                outcome: OutcomeKind::Disputed as i32,
                note: req.note,
            }),
            FeedbackAction::Invalidate => log_event::Payload::ClaimTransitioned(ClaimTransitioned {
                claim_id: req.id.clone(),
                new_status: ClaimStatus::Invalidated as i32,
                reason: req.note,
            }),
            FeedbackAction::Retract => log_event::Payload::ClaimTransitioned(ClaimTransitioned {
                claim_id: req.id.clone(),
                new_status: ClaimStatus::Retracted as i32,
                reason: req.note,
            }),
        };

        let ev = LogEvent {
            seq: self.alloc_seq(),
            event_id,
            recorded_at_ms: now_ms,
            actor: Some(req.actor),
            payload: Some(payload),
        };
        log::append_hot_event(
            &self.root,
            &mut self.active_seg,
            &ev,
            log::DEFAULT_HOT_SEGMENT_MAX_BYTES,
        )?;
        self.db.apply_event(&ev, None)?;
        self.maybe_lazy_gc(1, now_ms)?;

        let updated = self
            .db
            .get_claim(&req.id)?
            .ok_or_else(|| LoreError::ClaimNotFound { id: req.id.clone() })?;

        Ok(FeedbackResponse {
            enabled: true,
            id: req.id,
            new_status: updated.status,
            counters: updated.counters,
            promoted_from_cold,
        })
    }

    /// Immediately erases a claim across `index.db`, `claims_fts`, hot `.pblog`,
    /// cold `.pblog.zst`, and orphan blobs (`GC-5`, `SW-5`).
    pub fn forget(&mut self, claim_id: &str, reason: &str, actor: Principal) -> Result<()> {
        let mut reason_buf = reason.to_string();
        scan_or_redact_field("reason", &mut reason_buf, false)?;
        let now_ms = self.clock.now_ms();
        let _lock = log::WriteLock::acquire(&self.root)?;
        self.sync_under_lock()?;
        let seq = self.alloc_seq();
        gc::forget_claim_immediately(
            &self.root,
            &mut self.db,
            &mut self.active_seg,
            seq,
            actor,
            claim_id,
            &reason_buf,
            now_ms,
        )
    }

    /// Updates ephemeral peer presence (unless `query_only`) and returns active peers
    /// plus detected path/error overlaps (`AW-1`..`AW-4`).
    pub fn pulse(&mut self, req: PulseRequest) -> Result<PulseResponse> {
        self.ensure_live_enabled()?;
        let now_ms = self.clock.now_ms();
        let norm_errors = normalize_failure_list(&req.recent_failures);

        if !req.query_only {
            let session_id = req
                .frame
                .session_id
                .clone()
                .unwrap_or_else(|| format!("anon:{}", req.frame.machine_id));
            let ttl_ms = req.ttl_ms.unwrap_or_else(|| {
                if norm_errors.is_empty() {
                    DEFAULT_PRESENCE_TTL_MS
                } else {
                    DEFAULT_FAILURE_PRESENCE_TTL_MS
                }
            });

            let mounted_repos: Vec<PeerRepoMount> = req
                .frame
                .mounted_repos
                .iter()
                .map(|m| PeerRepoMount {
                    repo_id: m.repo_id.clone(),
                    mount_subpath: m.mount_subpath.to_string_lossy().to_string(),
                    branch: m.branch.clone(),
                    head_sha: m.head_sha.clone(),
                    dirty_count: req.dirty_count,
                })
                .collect();

            let presence = PeerPresence {
                session_id,
                channel_id: req.frame.channel_id.clone(),
                machine_id: req.frame.machine_id.clone(),
                agent_kind: req
                    .frame
                    .agent_kind
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string()),
                mounted_repos,
                intent: req.intent,
                hypothesis: req.hypothesis,
                touched_paths: req.touched_paths.clone(),
                recent_failures: norm_errors.clone(),
                updated_at_ms: now_ms,
                ttl_ms,
            };
            self.db.upsert_presence(&presence, now_ms)?;
        }

        let peers = self
            .db
            .query_live_presence(now_ms, req.frame.session_id.as_deref())?;
        let caller_repos: Vec<String> = req
            .frame
            .mounted_repos
            .iter()
            .map(|r| r.repo_id.clone())
            .collect();
        let overlaps = db::detect_overlaps(&caller_repos, &req.touched_paths, &norm_errors, &peers);

        Ok(PulseResponse {
            enabled: true,
            peers,
            overlaps,
        })
    }

    /// Runs `(status, age)` Tier-1 cold archival, Tier-2 purge, blob mark-and-sweep,
    /// and `PRAGMA incremental_vacuum` (`GC-1`..`GC-6`, `SW-5`).
    pub fn gc(&mut self, dry_run: bool) -> Result<GcReport> {
        let now_ms = self.clock.now_ms();
        let _lock = log::WriteLock::acquire(&self.root)?;
        self.sync_under_lock()?;
        gc::run_gc(
            &self.root,
            &mut self.db,
            &mut self.active_seg,
            &mut self.next_seq,
            &self.config.retention,
            now_ms,
            dry_run,
        )
    }

    /// Drops and deterministically rebuilds `index.db` from `.pblog` + `.pblog.zst` (`ST-1`).
    pub fn reindex(&mut self) -> Result<u64> {
        let _lock = log::WriteLock::acquire(&self.root)?;
        let events = log::stream_events_with_archive_locs(&self.root)?;
        let count = events.len() as u64;
        self.db = ProjectionDb::drop_and_recreate(&self.root)?;
        self.db.apply_events_batch(&events)?;
        let max_seq = events.iter().map(|(e, _)| e.seq).max().unwrap_or(0);
        self.next_seq = self.next_seq.max(max_seq + 1);
        Ok(count)
    }

    /// Verifies all hot segments and cold archives (`ST-3`, `ST-5`).
    pub fn verify(&self) -> Result<VerifyReport> {
        log::verify_all_logs(&self.root)
    }

    /// Streams all [`LogEvent`]s with `seq > since_seq` for `lore log --jsonl`
    /// and `lore export --jsonl` (`ST-7`, `SW-5`).
    pub fn export_events(&self, since_seq: u64) -> Result<Vec<LogEvent>> {
        log::stream_log_events(&self.root, since_seq)
    }

    fn alloc_seq(&mut self) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        seq
    }

    fn hydrate_row(&self, row: &ClaimRow) -> Result<Claim> {
        let (Some(seg), Some(frame_off), Some(rec_off), Some(rec_len)) = (
            row.archive_seg.as_deref(),
            row.archive_frame_off,
            row.archive_rec_off,
            row.archive_rec_len,
        ) else {
            return Ok(row.to_proto_claim());
        };

        let ev = log::hydrate_archive_event(
            &self.root,
            &row.id,
            seg,
            frame_off,
            rec_off,
            rec_len,
        )?;

        match ev.payload {
            Some(log_event::Payload::ClaimUpserted(up)) => up.claim.ok_or_else(|| {
                LoreError::ArchiveHydrationFailed {
                    claim_id: row.id.clone(),
                    segment: log::archive_dir(&self.root).join(seg),
                    source: lore_proto::FrameError::InvalidMagic,
                }
            }),
            _ => Err(LoreError::ArchiveHydrationFailed {
                claim_id: row.id.clone(),
                segment: log::archive_dir(&self.root).join(seg),
                source: lore_proto::FrameError::InvalidMagic,
            }),
        }
    }

    /// Lazy watermark check after writes (zero background timers, `GC-6`).
    fn maybe_lazy_gc(&mut self, events_written: u64, now_ms: i64) -> Result<()> {
        let writes = self.db.increment_writes_since_gc(events_written)?;
        let hot_bytes = log::total_hot_bytes(&self.root)?;
        let hit_write_wm = self.config.retention.gc_write_watermark > 0
            && writes >= self.config.retention.gc_write_watermark;
        let hit_bytes_wm = self.config.retention.gc_hot_bytes_watermark > 0
            && hot_bytes >= self.config.retention.gc_hot_bytes_watermark;

        if hit_write_wm || hit_bytes_wm {
            let _ = gc::run_gc(
                &self.root,
                &mut self.db,
                &mut self.active_seg,
                &mut self.next_seq,
                &self.config.retention,
                now_ms,
                false,
            )?;
        }
        Ok(())
    }
}

/// Normalizes `file:<mount_subpath>/<rel>` entries in `about` to `file:<rel>` and
/// collects any distinct mounted `repo_id`s targeted by `file:` or `repo:` entries (`#8b-recall`).
fn normalize_about_filter(frame: &ContextFrame, about: &[String]) -> (Vec<String>, HashSet<String>) {
    let mounted_ids: HashSet<&str> = frame
        .mounted_repos
        .iter()
        .map(|r| r.repo_id.as_str())
        .collect();
    let mut normalized = Vec::with_capacity(about.len());
    let mut targeted_repos = HashSet::new();

    for entry in about {
        if let Some(raw_path) = entry.strip_prefix("file:") {
            if let Some((repo_id, rel_posix)) = normalize_path_to_repo(frame, Path::new(raw_path)) {
                targeted_repos.insert(repo_id);
                normalized.push(format!("file:{rel_posix}"));
                continue;
            }
        } else if let Some(repo_id) = entry.strip_prefix("repo:") {
            if mounted_ids.contains(repo_id) {
                targeted_repos.insert(repo_id.to_string());
            }
        }
        normalized.push(entry.clone());
    }

    (normalized, targeted_repos)
}

fn validate_scope(scope: &str) -> Result<()> {
    let valid = scope == "global"
        || scope.starts_with("channel:")
        || scope.starts_with("repo:")
        || scope.starts_with("machine:")
        || scope.starts_with("dir:")
        || scope.starts_with("session:");
    if !valid || scope.ends_with(':') {
        return Err(LoreError::InvalidScope {
            scope: scope.to_string(),
        });
    }
    Ok(())
}

fn normalize_failure_list(raw_failures: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for item in raw_failures {
        let trimmed = item.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with("err:") {
            out.push(trimmed.to_string());
        } else {
            let sigs = normalize_error_signature(trimmed);
            if sigs.is_empty() {
                out.push(trimmed.to_string());
            } else {
                out.extend(sigs);
            }
        }
    }
    out
}

fn cold_lexical_overlap(query_terms: &[String], claim: &ClaimRow) -> f64 {
    if query_terms.is_empty() {
        return 0.5;
    }
    let hay = lore_context::tokenize_code(&format!(
        "{} {} {}",
        claim.statement,
        claim.body.as_deref().unwrap_or(""),
        claim.about.join(" ")
    ));
    let hits = query_terms
        .iter()
        .filter(|t| hay.contains(t.as_str()))
        .count();
    if hits == 0 {
        0.0
    } else {
        ((hits as f64) / (query_terms.len() as f64)).clamp(0.1, 1.0)
    }
}
