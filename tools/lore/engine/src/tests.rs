//! Behavioral test suite for `lore_engine` (`SW-*`, `MOD-*`, `ST-*`, `GC-*`,
//! `RET-*`, `AW-*`, `SEC-*`) using `ManualClock` and `tempfile::tempdir`,
//! with zero sleeps and zero polling loops.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use lore_context::{ContextFrame, RepoMount, compute_file_digest};
use lore_proto::{
    BlobRef, ClaimKind, ClaimStatus, ClaimTier, Clock, EdgeRel, FileDigestCheck, LoreError,
    ManualClock, Predicate, PredicateOp, Principal, PrincipalKind, Verifier, VerifierStatus,
    verifier,
};
use tempfile::tempdir;

use crate::{
    FeedbackAction, FeedbackRequest, InspectRequest, LoreEngine, MS_PER_DAY, PulseRequest,
    RecallRequest, RememberRequest, code_precedence_notice, format_evidence_envelope,
};

#[derive(Clone)]
struct SharedClock(Arc<ManualClock>);

impl SharedClock {
    fn new(initial_ms: i64) -> Self {
        Self(Arc::new(ManualClock::new(initial_ms)))
    }

    fn advance_days(&self, days: i64) {
        self.0.advance_ms(days * MS_PER_DAY);
    }

    fn advance_ms(&self, ms: i64) {
        self.0.advance_ms(ms);
    }
}

impl Clock for SharedClock {
    fn now_ms(&self) -> i64 {
        self.0.now_ms()
    }
}

fn test_frame(cwd: PathBuf, session_id: &str, repo_root: Option<(&str, PathBuf)>) -> ContextFrame {
    let mounted_repos = match repo_root {
        Some((repo_id, root_path)) => vec![RepoMount {
            repo_id: repo_id.to_string(),
            root_path,
            mount_subpath: PathBuf::new(),
            branch: Some("main".to_string()),
            head_sha: Some("deadbeef".to_string()),
            sandboxed_worktree: false,
            languages: Vec::new(),
            lockfiles: Vec::new(),
        }],
        None => Vec::new(),
    };

    ContextFrame {
        cwd,
        machine_id: "mac-host".to_string(),
        os: "macos".to_string(),
        arch: "aarch64".to_string(),
        channel_id: Some("ch-1".to_string()),
        session_id: Some(session_id.to_string()),
        agent_kind: Some("agy".to_string()),
        mounted_repos,
        cwd_dir_scope: "dir:mac-host:0123456789abcdef".to_string(),
    }
}

fn agent_principal(session_id: &str, machine_id: &str) -> Principal {
    Principal {
        kind: PrincipalKind::Agent as i32,
        id: format!("agent-{session_id}"),
        session_id: session_id.to_string(),
        agent_kind: "agy".to_string(),
        model: "test-model".to_string(),
        machine_id: machine_id.to_string(),
    }
}

fn owner_principal() -> Principal {
    Principal {
        kind: PrincipalKind::Owner as i32,
        id: "owner".to_string(),
        session_id: "owner-sess".to_string(),
        agent_kind: "human".to_string(),
        model: String::new(),
        machine_id: "mac-host".to_string(),
    }
}

// ---------------------------------------------------------------------------
// SW-1, SW-2, SW-4, SW-5: Global Kill-Switch
// ---------------------------------------------------------------------------

#[test]
fn sw_kill_switch_pauses_non_destructively_and_allows_admin_ops() {
    let dir = tempdir().unwrap();
    let clock = SharedClock::new(1_700_000_000_000);
    let mut engine = LoreEngine::open(dir.path(), clock.clone()).unwrap();
    let frame = test_frame(dir.path().to_path_buf(), "s1", None);

    let rem = engine
        .remember(RememberRequest {
            kind: ClaimKind::Decision,
            statement: "Always build via tools/bq/bq instead of bare bazel".to_string(),
            scope: "global".to_string(),
            author: owner_principal(),
            ..Default::default()
        })
        .unwrap();

    // Disable with explicit reason.
    engine
        .set_enabled(false, Some("benchmarking baseline".to_string()))
        .unwrap();

    // Agent operations must fail immediately with LoreError::Disabled carrying reason.
    let err = engine.recall(RecallRequest::new(frame.clone())).unwrap_err();
    match err {
        LoreError::Disabled { reason } => {
            assert_eq!(reason.as_deref(), Some("benchmarking baseline"));
        }
        other => panic!("expected Disabled, got {other:?}"),
    }
    assert!(matches!(
        engine.remember(RememberRequest {
            statement: "blocked".to_string(),
            ..Default::default()
        }),
        Err(LoreError::Disabled { .. })
    ));
    assert!(matches!(
        engine.pulse(PulseRequest::new(frame.clone())),
        Err(LoreError::Disabled { .. })
    ));

    // Admin operations (status, export, gc) remain operable when disabled (SW-5).
    let status = engine.status().unwrap();
    assert!(!status.enabled);
    assert_eq!(status.disabled_reason.as_deref(), Some("benchmarking baseline"));
    assert_eq!(status.hot_claims, 1);
    assert_eq!(engine.export_events(0).unwrap().len(), 1);
    let _gc = engine.gc(true).unwrap();

    // Re-enable: data is completely intact (SW-2).
    engine.set_enabled(true, None).unwrap();
    let recalled = engine.recall(RecallRequest::new(frame)).unwrap();
    assert_eq!(recalled.cards.len(), 1);
    assert_eq!(recalled.cards[0].id, rem.id);
}

// ---------------------------------------------------------------------------
// SEC-1, SEC-2: Secret Redaction & Taint Quarantine
// ---------------------------------------------------------------------------

#[test]
fn sec_secret_detection_rejects_by_default_and_redacts_on_request() {
    let dir = tempdir().unwrap();
    let clock = SharedClock::new(1_700_000_000_000);
    let mut engine = LoreEngine::open(dir.path(), clock).unwrap();

    // Default mode rejects GitHub PAT, AWS key, Bearer, HUMATA_SESSION_TOKEN, PEM.
    let err = engine
        .remember(RememberRequest {
            kind: ClaimKind::Fact,
            statement: "Do not leak ghp_AbCdEfGhIjKlMnOpQrStUvWxYz123456".to_string(),
            ..Default::default()
        })
        .unwrap_err();
    assert!(matches!(err, LoreError::SecretDetected { .. }));

    let err_aws = engine
        .remember(RememberRequest {
            kind: ClaimKind::Fact,
            statement: "Safe headline".to_string(),
            body: "key is AKIAIOSFODNN7EXAMPLE in env".to_string(),
            ..Default::default()
        })
        .unwrap_err();
    assert!(matches!(err_aws, LoreError::SecretDetected { .. }));

    // Redact mode replaces secrets and records redacted field names.
    let res = engine
        .remember(RememberRequest {
            kind: ClaimKind::Fact,
            statement: "Auth uses Bearer supersecret_token_value_123".to_string(),
            body: "HUMATA_SESSION_TOKEN=tok_abcdef1234567890".to_string(),
            redact_secrets: true,
            ..Default::default()
        })
        .unwrap();
    assert!(res.redacted_fields.contains(&"statement".to_string()));
    assert!(res.redacted_fields.contains(&"body".to_string()));

    let inspected = engine
        .inspect(InspectRequest {
            id: res.id,
            frame: None,
            include_lineage: false,
        })
        .unwrap();
    assert!(inspected.claim.statement.contains("[REDACTED:bearer_token]"));
    assert!(inspected.claim.body.contains("[REDACTED:humata_session_token]"));
    assert!(!inspected.claim.statement.contains("supersecret_token_value_123"));
}

#[test]
fn sec_taint_quarantine_visibility_and_promotion_guards() {
    let dir = tempdir().unwrap();
    let clock = SharedClock::new(1_700_000_000_000);
    let mut engine = LoreEngine::open(dir.path(), clock).unwrap();

    // Owner claim that tainted session cannot supersede.
    let owner_claim = engine
        .remember(RememberRequest {
            kind: ClaimKind::Decision,
            statement: "Owner architectural rule".to_string(),
            author: owner_principal(),
            ..Default::default()
        })
        .unwrap();

    // Tainted session cannot create AXIOM.
    assert!(matches!(
        engine.remember(RememberRequest {
            kind: ClaimKind::Axiom,
            statement: "Fake axiom".to_string(),
            author: agent_principal("tainted-s1", "mac-host"),
            tainted: true,
            ..Default::default()
        }),
        Err(LoreError::TaintedPromotionDenied { .. })
    ));

    // Tainted session cannot supersede OWNER claim.
    assert!(matches!(
        engine.remember(RememberRequest {
            kind: ClaimKind::Decision,
            statement: "Attempt override".to_string(),
            author: agent_principal("tainted-s1", "mac-host"),
            tainted: true,
            supersedes: Some(owner_claim.id),
            ..Default::default()
        }),
        Err(LoreError::TaintedPromotionDenied { .. })
    ));

    // Tainted FACT enters QUARANTINED and is visible only to originating session.
    let q = engine
        .remember(RememberRequest {
            kind: ClaimKind::Fact,
            statement: "Unverified external observation".to_string(),
            author: agent_principal("tainted-s1", "mac-host"),
            tainted: true,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(q.status, ClaimStatus::Quarantined);

    let other_frame = test_frame(dir.path().to_path_buf(), "clean-s2", None);
    let other_recall = engine.recall(RecallRequest::new(other_frame.clone())).unwrap();
    assert!(!other_recall.cards.iter().any(|c| c.id == q.id));

    let same_frame = test_frame(dir.path().to_path_buf(), "tainted-s1", None);
    let same_recall = engine.recall(RecallRequest::new(same_frame)).unwrap();
    assert!(same_recall.cards.iter().any(|c| c.id == q.id));

    // Corroboration from a clean session promotes QUARANTINED -> ACTIVE.
    let fb = engine
        .feedback(FeedbackRequest {
            id: q.id.clone(),
            action: FeedbackAction::Corroborated,
            note: "confirmed".to_string(),
            actor: agent_principal("clean-s2", "mac-host"),
            tainted: false,
            event_id: None,
        })
        .unwrap();
    assert_eq!(fb.new_status, ClaimStatus::Active);

    let after_recall = engine.recall(RecallRequest::new(other_frame)).unwrap();
    assert!(after_recall.cards.iter().any(|c| c.id == q.id));
}

// ---------------------------------------------------------------------------
// MOD-2, MOD-3, MOD-4, MOD-5: Supersession, Predicates, Scoring, Corroboration
// ---------------------------------------------------------------------------

#[test]
fn mod_supersession_predicates_and_corroboration_dedup() {
    let dir = tempdir().unwrap();
    let clock = SharedClock::new(1_700_000_000_000);
    let mut engine = LoreEngine::open(dir.path(), clock).unwrap();
    let frame = test_frame(dir.path().to_path_buf(), "s1", None);

    let old = engine
        .remember(RememberRequest {
            kind: ClaimKind::Solution,
            statement: "Use old Bazel flag for rustc".to_string(),
            about: vec!["tool:bazel".to_string()],
            author: agent_principal("s1", "mac-host"),
            ..Default::default()
        })
        .unwrap();

    let new = engine
        .remember(RememberRequest {
            kind: ClaimKind::Solution,
            statement: "Use --config=fast for local edit-test loops".to_string(),
            about: vec!["tool:bazel".to_string()],
            supersedes: Some(old.id.clone()),
            author: agent_principal("s2", "mac-host"),
            ..Default::default()
        })
        .unwrap();

    // Default recall hides superseded claim.
    let rec = engine.recall(RecallRequest::new(frame.clone())).unwrap();
    assert!(rec.cards.iter().any(|c| c.id == new.id));
    assert!(!rec.cards.iter().any(|c| c.id == old.id));

    // Inspect lineage shows SUPERSEDES edge.
    let insp = engine
        .inspect(InspectRequest {
            id: new.id.clone(),
            frame: Some(frame.clone()),
            include_lineage: true,
        })
        .unwrap();
    assert!(insp.edges.iter().any(|e| {
        e.src_id == new.id && e.dst_id == old.id && e.rel == EdgeRel::Supersedes
    }));

    // Corroboration deduplication by (session_id, machine_id) (MOD-5).
    for _ in 0..3 {
        let fb = engine
            .feedback(FeedbackRequest {
                id: new.id.clone(),
                action: FeedbackAction::Corroborated,
                note: "worked".to_string(),
                actor: agent_principal("s-peer", "mac-host"),
                tainted: false,
                event_id: None,
            })
            .unwrap();
        assert_eq!(fb.counters.corroborator_count, 1);
    }

    // Distinct session increments count to 2.
    let fb2 = engine
        .feedback(FeedbackRequest {
            id: new.id.clone(),
            action: FeedbackAction::Corroborated,
            note: "also worked".to_string(),
            actor: agent_principal("s-peer-2", "linux-box"),
            tainted: false,
            event_id: None,
        })
        .unwrap();
    assert_eq!(fb2.counters.corroborator_count, 2);

    // Unknown predicate axis yields UNKNOWN (does not exclude or crash);
    // mismatched os predicate excludes claim (G_hard = 0) (MOD-3).
    let linux_only = engine
        .remember(RememberRequest {
            kind: ClaimKind::Fact,
            statement: "Linux epoll tuning".to_string(),
            applies_when: vec![Predicate {
                axis: "os".to_string(),
                op: PredicateOp::Eq as i32,
                value: "linux".to_string(),
            }],
            ..Default::default()
        })
        .unwrap();
    let unknown_axis = engine
        .remember(RememberRequest {
            kind: ClaimKind::Fact,
            statement: "Future axis claim".to_string(),
            applies_when: vec![Predicate {
                axis: "future_quantum_axis".to_string(),
                op: PredicateOp::Eq as i32,
                value: "yes".to_string(),
            }],
            ..Default::default()
        })
        .unwrap();

    let rec2 = engine.recall(RecallRequest::new(frame)).unwrap();
    assert!(!rec2.cards.iter().any(|c| c.id == linux_only.id));
    assert!(rec2.cards.iter().any(|c| c.id == unknown_axis.id));
}

// ---------------------------------------------------------------------------
// ST-1, ST-4, ST-5, ST-7: Torn-Tail Recovery, Mid-Segment Corruption, Reindex
// ---------------------------------------------------------------------------

#[test]
fn st_torn_tail_recovery_mid_corruption_and_reindex_parity() {
    let dir = tempdir().unwrap();
    let clock = SharedClock::new(1_700_000_000_000);

    {
        let mut engine = LoreEngine::open(dir.path(), clock.clone()).unwrap();
        for i in 0..25 {
            engine
                .remember(RememberRequest {
                    kind: ClaimKind::Fact,
                    statement: format!("Claim number {i} about bazelBuildCache"),
                    body: format!("Detailed body for item {i}"),
                    ..Default::default()
                })
                .unwrap();
        }
    }

    // Append garbage bytes to simulate a torn tail write on the active hot segment (ST-4).
    let segs = crate::log::list_hot_segments(dir.path()).unwrap();
    assert_eq!(segs.len(), 1);
    let seg_path = &segs[0];
    {
        let mut f = OpenOptions::new().append(true).open(seg_path).unwrap();
        f.write_all(&[0xde, 0xad, 0xbe, 0xef, 0x01, 0x02, 0x03]).unwrap();
        f.sync_all().unwrap();
    }

    // Reopen: engine deterministically truncates the torn tail and preserves all 25 claims.
    let mut engine = LoreEngine::open(dir.path(), clock.clone()).unwrap();
    assert_eq!(engine.recovery_report().truncated_tail_bytes, 7);
    assert_eq!(engine.status().unwrap().hot_claims, 25);
    assert_eq!(engine.export_events(0).unwrap().len(), 25);

    // Delete index.db and run reindex -> identical query results (ST-1).
    let frame = test_frame(dir.path().to_path_buf(), "s1", None);
    let mut q = RecallRequest::new(frame.clone());
    q.query = "bazel build cache".to_string();
    let before = engine.recall(q.clone()).unwrap();
    assert!(!before.cards.is_empty());

    let reindexed = engine.reindex().unwrap();
    assert_eq!(reindexed, 25);
    let after = engine.recall(q).unwrap();
    assert_eq!(
        before.cards.iter().map(|c| &c.id).collect::<Vec<_>>(),
        after.cards.iter().map(|c| &c.id).collect::<Vec<_>>()
    );
    drop(engine);

    // Flip 1 bit in the middle of the segment -> must return LoreError::CorruptSegment (ST-5).
    let mut raw = fs::read(seg_path).unwrap();
    let mid = raw.len() / 2;
    raw[mid] ^= 0x01;
    fs::write(seg_path, &raw).unwrap();

    let err = LoreEngine::open(dir.path(), clock).err().unwrap();
    assert!(matches!(err, LoreError::CorruptSegment { .. }));
}

// ---------------------------------------------------------------------------
// GC-1..GC-6: (status, age) Cold zstd Archival, Hydration, Promotion, Purge, Forget
// ---------------------------------------------------------------------------

#[test]
fn gc_tiered_archival_hydration_promotion_purge_and_forget() {
    let dir = tempdir().unwrap();
    let clock = SharedClock::new(1_700_000_000_000);
    let mut engine = LoreEngine::open(dir.path(), clock.clone()).unwrap();

    // Put two blobs: one referenced by a claim, one orphaned.
    let kept_blob_hash = engine.put_blob(b"referenced stack trace payload ".repeat(60).as_slice()).unwrap();
    let orphan_blob_hash = engine.put_blob(b"orphan blob payload").unwrap();
    let forget_blob_hash = engine.put_blob(b"to be forgotten blob").unwrap();

    // 1. Active AXIOM (exempt from cold archival forever, GC-1).
    let axiom = engine
        .remember(RememberRequest {
            kind: ClaimKind::Axiom,
            statement: "Never run cargo directly".to_string(),
            author: owner_principal(),
            ..Default::default()
        })
        .unwrap();

    // 2. Pinned FACT (exempt from cold archival, GC-1).
    let pinned = engine
        .remember(RememberRequest {
            kind: ClaimKind::Fact,
            statement: "Pinned fact stays hot".to_string(),
            pinned: true,
            verifiers: vec![Verifier {
                check: Some(verifier::Check::BlobRef(BlobRef {
                    sha256: kept_blob_hash.clone(),
                    role: "trace".to_string(),
                    summary: "kept".to_string(),
                })),
            }],
            ..Default::default()
        })
        .unwrap();

    // 3. Dormant unpinned SOLUTION (>90d -> cold, then corroborated -> promotes back to hot, GC-2/GC-3).
    let dormant = engine
        .remember(RememberRequest {
            kind: ClaimKind::Solution,
            statement: "Dormant solution with rich body".to_string(),
            body: "Full L1 troubleshooting details for cold hydration test".repeat(10),
            ..Default::default()
        })
        .unwrap();

    // 4. Superseded HANDOFF (>14d -> Tier-2 purge, GC-4(a)).
    let handoff = engine
        .remember(RememberRequest {
            kind: ClaimKind::Handoff,
            statement: "Temporary handoff notes".to_string(),
            ..Default::default()
        })
        .unwrap();
    engine
        .feedback(FeedbackRequest {
            id: handoff.id.clone(),
            action: FeedbackAction::Invalidate,
            note: "done".to_string(),
            actor: owner_principal(),
            tainted: false,
            event_id: None,
        })
        .unwrap();

    // 5. Retracted claim (>30d -> Tier-2 purge, GC-4(b)).
    let retracted = engine
        .remember(RememberRequest {
            kind: ClaimKind::Fact,
            statement: "Wrong assumption retracted".to_string(),
            ..Default::default()
        })
        .unwrap();
    engine
        .feedback(FeedbackRequest {
            id: retracted.id.clone(),
            action: FeedbackAction::Retract,
            note: "incorrect".to_string(),
            actor: owner_principal(),
            tainted: false,
            event_id: None,
        })
        .unwrap();

    // 6. Quarantined claim (>14d -> Tier-2 purge, GC-4(c)).
    let quarantined = engine
        .remember(RememberRequest {
            kind: ClaimKind::Fact,
            statement: "Uncorroborated tainted claim".to_string(),
            tainted: true,
            ..Default::default()
        })
        .unwrap();

    // 7. Claim to be immediately forgotten via `forget` (GC-5).
    let to_forget = engine
        .remember(RememberRequest {
            kind: ClaimKind::Fact,
            statement: "UNIQUE_FORGET_SECRET_MARKER_999".to_string(),
            body: "UNIQUE_FORGET_BODY_MARKER_999".to_string(),
            verifiers: vec![Verifier {
                check: Some(verifier::Check::BlobRef(BlobRef {
                    sha256: forget_blob_hash.clone(),
                    role: "stderr".to_string(),
                    summary: "to forget".to_string(),
                })),
            }],
            ..Default::default()
        })
        .unwrap();

    // Advance clock by 91 days and dry-run GC first.
    clock.advance_days(91);
    let dry = engine.gc(true).unwrap();
    assert!(dry.dry_run);
    assert_eq!(dry.purged_handoff, 1);
    assert_eq!(dry.purged_retracted, 1);
    assert_eq!(dry.purged_quarantined, 1);
    assert!(dry.archived_claims >= 1);
    assert_eq!(dry.swept_blobs, 1); // orphan_blob_hash

    // Real GC run.
    let report = engine.gc(false).unwrap();
    assert!(!report.dry_run);
    assert!(report.vacuumed);
    assert_eq!(report.purged_handoff, 1);
    assert_eq!(report.purged_retracted, 1);
    assert_eq!(report.purged_quarantined, 1);
    assert_eq!(report.swept_blobs, 1);
    assert!(crate::blob_exists(dir.path(), &kept_blob_hash));
    assert!(!crate::blob_exists(dir.path(), &orphan_blob_hash));

    // AXIOM and pinned stayed Hot; dormant moved to Cold.
    assert_eq!(engine.db.get_claim(&axiom.id).unwrap().unwrap().tier, ClaimTier::Hot);
    assert_eq!(engine.db.get_claim(&pinned.id).unwrap().unwrap().tier, ClaimTier::Hot);
    let dormant_row = engine.db.get_claim(&dormant.id).unwrap().unwrap();
    assert_eq!(dormant_row.tier, ClaimTier::ColdArchived);
    assert!(dormant_row.body.is_none()); // stripped from SQLite stub

    // Purged claims are gone.
    assert!(engine.db.get_claim(&handoff.id).unwrap().is_none());
    assert!(engine.db.get_claim(&retracted.id).unwrap().is_none());
    assert!(engine.db.get_claim(&quarantined.id).unwrap().is_none());

    // Transparent cold hydration on `inspect` (GC-3).
    let hydrated = engine
        .inspect(InspectRequest {
            id: dormant.id.clone(),
            frame: None,
            include_lineage: false,
        })
        .unwrap();
    assert!(hydrated.hydrated_from_cold);
    assert!(hydrated.claim.body.contains("Full L1 troubleshooting details"));

    // Corroborating a cold claim promotes it back to Tier-0 Hot (GC-3).
    let promo = engine
        .feedback(FeedbackRequest {
            id: dormant.id.clone(),
            action: FeedbackAction::Corroborated,
            note: "still relevant".to_string(),
            actor: agent_principal("s-new", "mac-host"),
            tainted: false,
            event_id: None,
        })
        .unwrap();
    assert!(promo.promoted_from_cold);
    assert_eq!(engine.db.get_claim(&dormant.id).unwrap().unwrap().tier, ClaimTier::Hot);

    // Immediate `forget(id)` erases bytes from hot, cold, index.db, and sweeps its blob (GC-5).
    engine
        .forget(&to_forget.id, "gdpr erasure", owner_principal())
        .unwrap();
    assert!(engine.db.get_claim(&to_forget.id).unwrap().is_none());
    assert!(!crate::blob_exists(dir.path(), &forget_blob_hash));

    // Verify zero payload bytes of `to_forget` remain in any hot segment or cold archive.
    for seg in crate::log::list_hot_segments(dir.path()).unwrap() {
        let raw = fs::read(&seg).unwrap();
        assert!(
            !raw.windows(b"UNIQUE_FORGET_SECRET_MARKER_999".len())
                .any(|w| w == b"UNIQUE_FORGET_SECRET_MARKER_999")
        );
    }
    for arch in crate::log::list_archive_segments(dir.path()).unwrap() {
        for (_, recs) in crate::log::read_archive_file(&arch).unwrap() {
            for (_, _, ev) in recs {
                assert!(!crate::gc::event_references_claim(&ev, &to_forget.id));
            }
        }
    }

    // Verify archive and hot logs pass full integrity verification.
    let vrep = engine.verify().unwrap();
    assert!(vrep.archive_segments >= 1);
    assert!(vrep.hot_segments >= 1);
}

// ---------------------------------------------------------------------------
// RET-2, RET-3, RET-4: Local Verification Drift Demotion, Exposure Dedup, Envelopes
// ---------------------------------------------------------------------------

#[test]
fn ret_local_verifier_drift_demotes_rank_and_exposure_dedup_suppresses() {
    let dir = tempdir().unwrap();
    let repo_dir = dir.path().join("repo");
    fs::create_dir_all(&repo_dir).unwrap();
    let file_a = repo_dir.join("a.rs");
    let file_b = repo_dir.join("b.rs");
    fs::write(&file_a, "pub fn alpha() -> i32 { 1 }\n").unwrap();
    fs::write(&file_b, "pub fn beta() -> i32 { 2 }\n").unwrap();

    let digest_a = compute_file_digest(&file_a, 0, 0).unwrap();
    let digest_b = compute_file_digest(&file_b, 0, 0).unwrap();

    let clock = SharedClock::new(1_700_000_000_000);
    let mut engine = LoreEngine::open(dir.path().join("store"), clock).unwrap();
    let frame = test_frame(
        repo_dir.clone(),
        "sess-ret",
        Some(("github.com/org/demo", repo_dir.clone())),
    );

    let claim_a = engine
        .remember(RememberRequest {
            kind: ClaimKind::Solution,
            statement: "Widget cache fix in alpha".to_string(),
            scope: "repo:github.com/org/demo".to_string(),
            verifiers: vec![Verifier {
                check: Some(verifier::Check::FileDigest(FileDigestCheck {
                    repo_id: "github.com/org/demo".to_string(),
                    path: "a.rs".to_string(),
                    start_line: 0,
                    end_line: 0,
                    sha256: digest_a,
                })),
            }],
            ..Default::default()
        })
        .unwrap();

    let claim_b = engine
        .remember(RememberRequest {
            kind: ClaimKind::Solution,
            statement: "Widget cache fix in beta".to_string(),
            scope: "repo:github.com/org/demo".to_string(),
            verifiers: vec![Verifier {
                check: Some(verifier::Check::FileDigest(FileDigestCheck {
                    repo_id: "github.com/org/demo".to_string(),
                    path: "b.rs".to_string(),
                    start_line: 0,
                    end_line: 0,
                    sha256: digest_b,
                })),
            }],
            ..Default::default()
        })
        .unwrap();

    // Mutate a.rs on disk so claim_a drifts while claim_b remains verified (RET-2).
    fs::write(&file_a, "pub fn alpha() -> i32 { 999 }\n").unwrap();

    let mut q = RecallRequest::new(frame.clone());
    q.query = "widget cache fix".to_string();
    let res = engine.recall(q).unwrap();
    assert_eq!(res.cards.len(), 2);
    assert_eq!(res.cards[0].id, claim_b.id);
    assert_eq!(res.cards[0].verifier_badge, VerifierStatus::Verified);
    assert_eq!(res.cards[1].id, claim_a.id);
    assert_eq!(res.cards[1].verifier_badge, VerifierStatus::Drifted);
    assert!(res.cards[0].score > res.cards[1].score);
    assert!(res.notice.is_some());
    assert!(code_precedence_notice(&res.cards).is_some());

    // Evidence envelope wraps non-axioms with provenance + citation status (RET-4).
    let env_drifted = format_evidence_envelope(&res.cards[1]);
    assert!(env_drifted.contains("citation: drifted"));
    assert!(env_drifted.contains("kind: solution"));

    // Second recall in same session with `suppress_exposed = true` suppresses both cards (RET-3).
    let mut hook_q = RecallRequest::new(frame);
    hook_q.query = "widget cache fix".to_string();
    hook_q.suppress_exposed = true;
    let res2 = engine.recall(hook_q).unwrap();
    assert!(res2.cards.is_empty());
}

// ---------------------------------------------------------------------------
// AW-1..AW-4: Live Presence Overlap Detection & TTL Expiry Without Timers
// ---------------------------------------------------------------------------

#[test]
fn aw_live_presence_overlap_and_ttl_expiry() {
    let dir = tempdir().unwrap();
    let clock = SharedClock::new(1_700_000_000_000);
    let mut engine = LoreEngine::open(dir.path(), clock.clone()).unwrap();

    let frame_a = test_frame(
        dir.path().to_path_buf(),
        "peer-a",
        Some(("github.com/org/demo", dir.path().to_path_buf())),
    );
    let frame_b = test_frame(
        dir.path().to_path_buf(),
        "peer-b",
        Some(("github.com/org/demo", dir.path().to_path_buf())),
    );

    // Peer A emits pulse touching src/lib.rs with a Rust compiler error.
    let mut pa = PulseRequest::new(frame_a);
    pa.intent = Some("refactoring engine".to_string());
    pa.touched_paths = vec!["src/lib.rs".to_string()];
    pa.recent_failures = vec!["error[E0382]: borrow of moved value at src/lib.rs:42".to_string()];
    pa.ttl_ms = Some(60_000);
    engine.pulse(pa).unwrap();

    // Peer B touches the same path and hits the same normalized E0382 error.
    let mut pb = PulseRequest::new(frame_b.clone());
    pb.touched_paths = vec!["src/lib.rs".to_string()];
    pb.recent_failures = vec!["error[E0382]: borrow of moved value at src/lib.rs:99".to_string()];
    let resp_b = engine.pulse(pb).unwrap();
    assert_eq!(resp_b.peers.len(), 1);
    assert_eq!(resp_b.overlaps.len(), 1);
    assert_eq!(resp_b.overlaps[0].peer_session_id, "peer-a");
    assert_eq!(resp_b.overlaps[0].overlapping_paths, vec!["src/lib.rs"]);
    assert!(!resp_b.overlaps[0].overlapping_errors.is_empty());

    // Advance clock past Peer A's 60s TTL -> Peer A disappears without any background timer (AW-2).
    clock.advance_ms(65_000);
    let mut pq = PulseRequest::new(frame_b);
    pq.query_only = true;
    let resp_after = engine.pulse(pq).unwrap();
    assert!(resp_after.peers.is_empty());

    // AW-4: live_presence is never written to `.pblog`.
    assert!(engine.export_events(0).unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// Regression tests for review-expert #1, #2, #4, #5, #6, #7a, #8b-recall, #9
// ---------------------------------------------------------------------------

#[test]
fn reg_1_two_handle_concurrent_writers_never_collide_on_seq() {
    let dir = tempdir().unwrap();
    let clock = SharedClock::new(1_700_000_000_000);
    let mut eng_a = LoreEngine::open(dir.path(), clock.clone()).unwrap();
    let mut eng_b = LoreEngine::open(dir.path(), clock).unwrap();

    for i in 0..5 {
        eng_a
            .remember(RememberRequest {
                kind: ClaimKind::Fact,
                statement: format!("from handle A #{i}"),
                ..Default::default()
            })
            .unwrap();
        eng_b
            .remember(RememberRequest {
                kind: ClaimKind::Fact,
                statement: format!("from handle B #{i}"),
                ..Default::default()
            })
            .unwrap();
    }

    let events = crate::export_store_events(dir.path(), 0).unwrap();
    assert_eq!(events.len(), 10);
    for (idx, ev) in events.iter().enumerate() {
        assert_eq!(ev.seq, (idx + 1) as u64);
    }
    let vrep = crate::verify_store(dir.path()).unwrap();
    assert_eq!(vrep.hot_events, 10);
    assert_eq!(vrep.max_seq, 10);
}

#[test]
fn reg_2_rewrite_and_stream_reject_corrupt_or_non_final_truncated_segments() {
    let dir = tempdir().unwrap();
    let clock = SharedClock::new(1_700_000_000_000);
    let mut engine = LoreEngine::open(dir.path(), clock).unwrap();
    engine
        .remember(RememberRequest {
            kind: ClaimKind::Fact,
            statement: "first record".to_string(),
            ..Default::default()
        })
        .unwrap();
    engine
        .remember(RememberRequest {
            kind: ClaimKind::Fact,
            statement: "second record".to_string(),
            ..Default::default()
        })
        .unwrap();

    let seg1 = crate::log::list_hot_segments(dir.path()).unwrap()[0].clone();
    // Create a second valid hot segment so seg1 is non-final.
    let seg2 = crate::log::hot_dir(dir.path()).join("seg-00000000000000000010.pblog");
    {
        let mut buf = Vec::new();
        lore_proto::write_segment_header(&mut buf).unwrap();
        fs::write(&seg2, &buf).unwrap();
    }
    // Append torn bytes to non-final seg1.
    {
        let mut f = OpenOptions::new().append(true).open(&seg1).unwrap();
        f.write_all(&[0xaa, 0xbb, 0xcc]).unwrap();
        f.sync_all().unwrap();
    }

    assert!(matches!(
        crate::log::stream_log_events(dir.path(), 0),
        Err(LoreError::CorruptSegment { .. })
    ));
    assert!(matches!(
        crate::log::rewrite_hot_segment_filtered(&seg1, |_| true),
        Err(LoreError::CorruptSegment { .. })
    ));
}

#[test]
fn reg_5_forget_leaves_zero_bytes_in_raw_sqlite_and_wal() {
    let dir = tempdir().unwrap();
    let clock = SharedClock::new(1_700_000_000_000);
    let mut engine = LoreEngine::open(dir.path(), clock).unwrap();

    let marker = "ULTRA_UNIQUE_ERASE_ME_PAYLOAD_XYZ789";
    let res = engine
        .remember(RememberRequest {
            kind: ClaimKind::Fact,
            statement: format!("headline {marker}"),
            body: format!("body {marker}"),
            ..Default::default()
        })
        .unwrap();

    engine
        .forget(&res.id, "erasure requested", owner_principal())
        .unwrap();
    drop(engine);

    for suffix in ["", "-wal", "-shm"] {
        let p = PathBuf::from(format!(
            "{}{suffix}",
            crate::db::index_db_path(dir.path()).display()
        ));
        if p.exists() {
            let bytes = fs::read(&p).unwrap();
            assert!(
                !bytes
                    .windows(marker.len())
                    .any(|w| w == marker.as_bytes()),
                "erased marker bytes found in {}",
                p.display()
            );
        }
    }
}

#[test]
fn reg_6_rearchive_no_duplicate_seqs_and_expired_handoff_purges() {
    let dir = tempdir().unwrap();
    let t0 = 1_700_000_000_000;
    let clock = SharedClock::new(t0);
    let mut engine = LoreEngine::open(dir.path(), clock.clone()).unwrap();

    // 1. Dormant solution -> archive -> corroborate (promote) -> advance 91d -> re-archive.
    let sol = engine
        .remember(RememberRequest {
            kind: ClaimKind::Solution,
            statement: "Re-promoted solution".to_string(),
            body: "Full body text".to_string(),
            ..Default::default()
        })
        .unwrap();

    // 2. HANDOFF with valid_until_ms = t0 + 1 day; after 91 days it has been expired >14d -> must purge!
    let handoff = engine
        .remember(RememberRequest {
            kind: ClaimKind::Handoff,
            statement: "Short-lived handoff".to_string(),
            valid_until_ms: t0 + MS_PER_DAY,
            ..Default::default()
        })
        .unwrap();

    clock.advance_days(91);
    let rep1 = engine.gc(false).unwrap();
    assert_eq!(rep1.purged_handoff, 1);
    assert!(engine.db.get_claim(&handoff.id).unwrap().is_none());
    assert_eq!(
        engine.db.get_claim(&sol.id).unwrap().unwrap().tier,
        ClaimTier::ColdArchived
    );

    // Promote back to hot via corroboration.
    let fb = engine
        .feedback(FeedbackRequest {
            id: sol.id.clone(),
            action: FeedbackAction::Corroborated,
            note: "still good".to_string(),
            actor: agent_principal("s-promo", "mac-host"),
            tainted: false,
            event_id: None,
        })
        .unwrap();
    assert!(fb.promoted_from_cold);

    // Advance another 91 days and re-archive.
    clock.advance_days(91);
    let rep2 = engine.gc(false).unwrap();
    assert_eq!(rep2.archived_claims, 1);

    // Verify no duplicate `seq` across all archive segments.
    let mut seen_seqs = std::collections::HashSet::new();
    for arch in crate::log::list_archive_segments(dir.path()).unwrap() {
        for (_, recs) in crate::log::read_archive_file(&arch).unwrap() {
            for (_, _, ev) in recs {
                assert!(
                    seen_seqs.insert(ev.seq),
                    "duplicate seq {} found across archives",
                    ev.seq
                );
            }
        }
    }
}

#[test]
fn reg_7a_live_config_reload_and_read_only_verify_no_truncation() {
    let dir = tempdir().unwrap();
    let clock = SharedClock::new(1_700_000_000_000);
    let mut server = LoreEngine::open(dir.path(), clock.clone()).unwrap();
    let frame = test_frame(dir.path().to_path_buf(), "s1", None);

    server
        .remember(RememberRequest {
            kind: ClaimKind::Fact,
            statement: "live reload test".to_string(),
            ..Default::default()
        })
        .unwrap();

    // External process toggles config.toml to disabled while `server` is still open.
    let mut cfg = crate::load_config(dir.path()).unwrap();
    cfg.memory.enabled = false;
    cfg.memory.disabled_reason = Some("paused externally".to_string());
    crate::save_config(dir.path(), &cfg).unwrap();

    // Long-running `server` observes the toggle immediately on next recall & status!
    assert!(matches!(
        server.recall(RecallRequest::new(frame.clone())),
        Err(LoreError::Disabled { .. })
    ));
    assert!(!server.status().unwrap().enabled);

    // Re-enable externally -> server resumes immediately.
    cfg.memory.enabled = true;
    cfg.memory.disabled_reason = None;
    crate::save_config(dir.path(), &cfg).unwrap();
    assert_eq!(server.recall(RecallRequest::new(frame.clone())).unwrap().cards.len(), 1);

    // Read-only engine can recall without write lock or mutating exposures.
    let mut ro = LoreEngine::open_read_only(dir.path(), clock.clone(), 25).unwrap();
    assert_eq!(ro.recall(RecallRequest::new(frame)).unwrap().cards.len(), 1);
    assert!(crate::read_only_status(dir.path(), &clock).unwrap().enabled);

    // Append torn bytes to active hot segment; verify_store must detect it WITHOUT truncating!
    let seg = crate::log::list_hot_segments(dir.path()).unwrap()[0].clone();
    let before_len = fs::metadata(&seg).unwrap().len();
    {
        let mut f = OpenOptions::new().append(true).open(&seg).unwrap();
        f.write_all(&[0xde, 0xad, 0xbe]).unwrap();
        f.sync_all().unwrap();
    }
    assert!(matches!(
        crate::verify_store(dir.path()),
        Err(LoreError::CorruptSegment { .. })
    ));
    assert_eq!(fs::metadata(&seg).unwrap().len(), before_len + 3);
}

#[test]
fn reg_8b_recall_normalizes_file_about_and_narrows_multi_repo_scope() {
    let dir = tempdir().unwrap();
    let repo_a = dir.path().join("repo-a");
    let repo_b = dir.path().join("repo-b");
    fs::create_dir_all(&repo_a).unwrap();
    fs::create_dir_all(&repo_b).unwrap();

    let clock = SharedClock::new(1_700_000_000_000);
    let mut engine = LoreEngine::open(dir.path().join("store"), clock).unwrap();

    let claim_a = engine
        .remember(RememberRequest {
            kind: ClaimKind::Fact,
            statement: "Repo A build rule".to_string(),
            scope: "repo:github.com/org/repo-a".to_string(),
            about: vec!["file:src/main.rs".to_string()],
            ..Default::default()
        })
        .unwrap();
    let claim_b = engine
        .remember(RememberRequest {
            kind: ClaimKind::Fact,
            statement: "Repo B build rule".to_string(),
            scope: "repo:github.com/org/repo-b".to_string(),
            about: vec!["file:src/main.rs".to_string()],
            ..Default::default()
        })
        .unwrap();

    let multi_frame = ContextFrame {
        cwd: dir.path().to_path_buf(),
        machine_id: "mac-host".to_string(),
        os: "macos".to_string(),
        arch: "aarch64".to_string(),
        channel_id: None,
        session_id: Some("s-multi".to_string()),
        agent_kind: Some("agy".to_string()),
        mounted_repos: vec![
            RepoMount {
                repo_id: "github.com/org/repo-a".to_string(),
                root_path: repo_a,
                mount_subpath: PathBuf::from("repo-a"),
                branch: Some("main".to_string()),
                head_sha: None,
                sandboxed_worktree: false,
                languages: Vec::new(),
                lockfiles: Vec::new(),
            },
            RepoMount {
                repo_id: "github.com/org/repo-b".to_string(),
                root_path: repo_b,
                mount_subpath: PathBuf::from("repo-b"),
                branch: Some("main".to_string()),
                head_sha: None,
                sandboxed_worktree: false,
                languages: Vec::new(),
                lockfiles: Vec::new(),
            },
        ],
        cwd_dir_scope: "dir:mac-host:multi".to_string(),
    };

    let mut req = RecallRequest::new(multi_frame);
    req.about = vec!["file:repo-a/src/main.rs".to_string()];
    let res = engine.recall(req).unwrap();
    assert!(res.cards.iter().any(|c| c.id == claim_a.id));
    assert!(!res.cards.iter().any(|c| c.id == claim_b.id));
}

#[test]
fn reg_9_feedback_secret_scan_taint_propagation_and_self_corroboration_denied() {
    let dir = tempdir().unwrap();
    let clock = SharedClock::new(1_700_000_000_000);
    let mut engine = LoreEngine::open(dir.path(), clock).unwrap();

    // First tainted write marks session "sess-bad" tainted.
    let q1 = engine
        .remember(RememberRequest {
            kind: ClaimKind::Fact,
            statement: "First tainted claim".to_string(),
            author: agent_principal("sess-bad", "mac-host"),
            tainted: true,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(q1.status, ClaimStatus::Quarantined);

    // Subsequent write from "sess-bad" WITHOUT tainted=true still inherits quarantine!
    let q2 = engine
        .remember(RememberRequest {
            kind: ClaimKind::Fact,
            statement: "Second claim from same tainted session".to_string(),
            author: agent_principal("sess-bad", "mac-host"),
            tainted: false,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(q2.status, ClaimStatus::Quarantined);

    // Other session cannot see quarantined claim even with include_history=true.
    let other_frame = test_frame(dir.path().to_path_buf(), "sess-clean", None);
    let mut hist_req = RecallRequest::new(other_frame);
    hist_req.include_history = true;
    let hist = engine.recall(hist_req).unwrap();
    assert!(!hist.cards.iter().any(|c| c.id == q1.id));

    // Originating session cannot self-corroborate its own quarantined claim.
    assert!(matches!(
        engine.feedback(FeedbackRequest {
            id: q1.id.clone(),
            action: FeedbackAction::Corroborated,
            note: "self confirm".to_string(),
            actor: agent_principal("sess-bad", "mac-host"),
            tainted: false,
            event_id: None,
        }),
        Err(LoreError::TaintedPromotionDenied { .. })
    ));

    // Feedback note containing a secret is rejected with SecretDetected.
    assert!(matches!(
        engine.feedback(FeedbackRequest {
            id: q1.id.clone(),
            action: FeedbackAction::Helpful,
            note: "leaked ghp_AbCdEfGhIjKlMnOpQrStUvWxYz123456".to_string(),
            actor: agent_principal("sess-clean", "mac-host"),
            tainted: false,
            event_id: None,
        }),
        Err(LoreError::SecretDetected { .. })
    ));

    // Clean session corroborates q1 -> status becomes ACTIVE and tainted flag clears to false.
    let promoted = engine
        .feedback(FeedbackRequest {
            id: q1.id.clone(),
            action: FeedbackAction::Corroborated,
            note: "independently verified".to_string(),
            actor: agent_principal("sess-clean", "mac-host"),
            tainted: false,
            event_id: None,
        })
        .unwrap();
    assert_eq!(promoted.new_status, ClaimStatus::Active);
    let row = engine.db.get_claim(&q1.id).unwrap().unwrap();
    assert!(!row.tainted);
    assert_eq!(crate::trust_badge(&row), "corroborated");
}
