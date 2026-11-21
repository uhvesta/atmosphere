//! `(status, age)` Tier-1 `zstd` cold archival, Tier-2 GC purge, blob
//! mark-and-sweep, and immediate `forget(id)` erasure (`GC-1`..`GC-6`).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use lore_proto::{
    ClaimForgotten, ClaimKind, ClaimStatus, ClaimTier, ClaimTransitioned, LogEvent, Principal,
    PrincipalKind, Result, log_event, new_event_id,
};
use serde::{Deserialize, Serialize};

use crate::blobs::sweep_unreferenced_blobs;
use crate::config::{MS_PER_DAY, RetentionConfig};
use crate::db::{ClaimRow, ProjectionDb};
use crate::log::{
    ArchiveRecordLoc, DEFAULT_HOT_SEGMENT_MAX_BYTES, append_hot_event, collect_referenced_blobs,
    list_archive_segments, list_hot_segments, rewrite_archive_segment_filtered,
    rewrite_hot_segment_filtered, stream_hot_events, write_archive_segment,
};

/// Tier-2 purge category for a claim (`GC-4`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PurgeCategory {
    Handoff,
    Retracted,
    Quarantined,
}

/// Summary returned by `LoreEngine::gc(dry_run)` (`GC-6`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GcReport {
    pub dry_run: bool,
    pub archived_claims: u64,
    pub archived_events: u64,
    pub purged_handoff: u64,
    pub purged_retracted: u64,
    pub purged_quarantined: u64,
    pub folded_exposures: u64,
    pub swept_blobs: u64,
    pub freed_blob_bytes: u64,
    pub vacuumed: bool,
}

/// Returns `true` if `claim` is protected in Tier-0 Hot (`GC-1`).
pub fn is_tier0_exempt(claim: &ClaimRow, now_ms: i64, cfg: &RetentionConfig) -> bool {
    let inactive_window_ms = (cfg.hot_inactive_days as i64).saturating_mul(MS_PER_DAY);
    let dormant_window_ms = (cfg.hot_dormant_days as i64).saturating_mul(MS_PER_DAY);

    if claim.status == ClaimStatus::Active {
        if claim.pinned || matches!(claim.kind, ClaimKind::Axiom | ClaimKind::Decision) {
            return true;
        }
        let validated = claim.counters.last_validated_at_ms.max(claim.created_at_ms);
        return now_ms.saturating_sub(validated) <= dormant_window_ms;
    }

    now_ms.saturating_sub(claim.updated_at_ms) <= inactive_window_ms
}

/// Returns `true` if a Tier-0 Hot claim is eligible for Tier-1 Cold `zstd` archival (`GC-2`).
pub fn is_tier1_cold_eligible(claim: &ClaimRow, now_ms: i64, cfg: &RetentionConfig) -> bool {
    if claim.tier != ClaimTier::Hot {
        return false;
    }
    if is_tier0_exempt(claim, now_ms, cfg) {
        return false;
    }

    let inactive_window_ms = (cfg.hot_inactive_days as i64).saturating_mul(MS_PER_DAY);
    let dormant_window_ms = (cfg.hot_dormant_days as i64).saturating_mul(MS_PER_DAY);

    match claim.status {
        ClaimStatus::Superseded | ClaimStatus::Invalidated => {
            now_ms.saturating_sub(claim.updated_at_ms) > inactive_window_ms
        }
        ClaimStatus::Active | ClaimStatus::Expired => {
            !claim.pinned
                && !matches!(claim.kind, ClaimKind::Axiom | ClaimKind::Decision)
                && now_ms.saturating_sub(claim.counters.last_validated_at_ms.max(claim.created_at_ms))
                    > dormant_window_ms
        }
        _ => false,
    }
}

/// Classifies whether a claim is eligible for permanent Tier-2 GC purge (`GC-4`).
pub fn tier2_purge_category(
    claim: &ClaimRow,
    now_ms: i64,
    cfg: &RetentionConfig,
) -> Option<PurgeCategory> {
    let age_updated = now_ms.saturating_sub(claim.updated_at_ms);
    let handoff_ms = (cfg.handoff_purge_days as i64).saturating_mul(MS_PER_DAY);
    let retracted_ms = (cfg.retracted_purge_days as i64).saturating_mul(MS_PER_DAY);
    let quarantined_ms = (cfg.quarantined_purge_days as i64).saturating_mul(MS_PER_DAY);

    if claim.kind == ClaimKind::Handoff
        && matches!(
            claim.status,
            ClaimStatus::Superseded | ClaimStatus::Invalidated | ClaimStatus::Expired
        )
        && age_updated > handoff_ms
    {
        return Some(PurgeCategory::Handoff);
    }

    if claim.status == ClaimStatus::Retracted && age_updated > retracted_ms {
        return Some(PurgeCategory::Retracted);
    }

    if claim.status == ClaimStatus::Quarantined
        && claim.counters.corroborator_count == 0
        && age_updated > quarantined_ms
    {
        return Some(PurgeCategory::Quarantined);
    }

    None
}

/// Returns the claim ID associated with a [`LogEvent`], if any.
pub fn event_claim_id(event: &LogEvent) -> Option<&str> {
    match &event.payload {
        Some(log_event::Payload::ClaimUpserted(up)) => up.claim.as_ref().map(|c| c.id.as_str()),
        Some(log_event::Payload::ClaimTransitioned(tr)) => Some(tr.claim_id.as_str()),
        Some(log_event::Payload::FeedbackRecorded(fb)) => Some(fb.claim_id.as_str()),
        Some(log_event::Payload::ClaimForgotten(fg)) => Some(fg.claim_id.as_str()),
        Some(log_event::Payload::EdgeAdded(edge)) => Some(edge.src_id.as_str()),
        None => None,
    }
}

/// Returns `true` if `event` references `claim_id` in any payload field.
pub fn event_references_claim(event: &LogEvent, claim_id: &str) -> bool {
    match &event.payload {
        Some(log_event::Payload::ClaimUpserted(up)) => {
            up.claim.as_ref().is_some_and(|c| c.id == claim_id)
        }
        Some(log_event::Payload::ClaimTransitioned(tr)) => tr.claim_id == claim_id,
        Some(log_event::Payload::FeedbackRecorded(fb)) => fb.claim_id == claim_id,
        Some(log_event::Payload::ClaimForgotten(fg)) => fg.claim_id == claim_id,
        Some(log_event::Payload::EdgeAdded(edge)) => {
            edge.src_id == claim_id || edge.dst_id == claim_id
        }
        None => false,
    }
}

/// Executes `(status, age)` Tier-1 cold archival, Tier-2 purge, exposure folding,
/// blob mark-and-sweep, and `PRAGMA incremental_vacuum` (`GC-1`..`GC-6`).
///
/// Caller must hold `WriteLock`.
pub fn run_gc(
    root: &Path,
    db: &mut ProjectionDb,
    active_seg: &mut Option<PathBuf>,
    next_seq: &mut u64,
    cfg: &RetentionConfig,
    now_ms: i64,
    dry_run: bool,
) -> Result<GcReport> {
    let mut report = GcReport {
        dry_run,
        ..Default::default()
    };

    let mut all_claims = db.list_all_claims()?;

    // 0. Transition expired ACTIVE claims (`valid_until_ms > 0 && valid_until_ms <= now_ms`)
    //    to `ClaimStatus::Expired` so expired HANDOFFs can purge under GC-4(a).
    let gc_actor = Principal {
        kind: PrincipalKind::Agent as i32,
        id: "lore-gc".to_string(),
        session_id: String::new(),
        agent_kind: "gc".to_string(),
        model: String::new(),
        machine_id: "local".to_string(),
    };
    for claim in &mut all_claims {
        if claim.status == ClaimStatus::Active
            && claim.valid_until_ms > 0
            && now_ms >= claim.valid_until_ms
        {
            claim.status = ClaimStatus::Expired;
            // Anchor expiry timestamp at valid_until_ms for age calculations so a claim
            // that expired >14d ago is immediately eligible for GC-4(a) purge.
            let expired_ts = claim.valid_until_ms;
            claim.updated_at_ms = claim.updated_at_ms.max(expired_ts);
            if !dry_run {
                let seq = *next_seq;
                *next_seq += 1;
                let ev = LogEvent {
                    seq,
                    event_id: new_event_id(now_ms),
                    recorded_at_ms: expired_ts,
                    actor: Some(gc_actor.clone()),
                    payload: Some(log_event::Payload::ClaimTransitioned(ClaimTransitioned {
                        claim_id: claim.id.clone(),
                        new_status: ClaimStatus::Expired as i32,
                        reason: "valid_until_ms elapsed".to_string(),
                    })),
                };
                append_hot_event(root, active_seg, &ev, DEFAULT_HOT_SEGMENT_MAX_BYTES)?;
                db.apply_event(&ev, None)?;
            }
        }
    }

    let mut purge_ids: HashSet<String> = HashSet::new();
    let mut cold_ids: HashSet<String> = HashSet::new();

    for claim in &all_claims {
        if let Some(cat) = tier2_purge_category(claim, now_ms, cfg) {
            purge_ids.insert(claim.id.clone());
            match cat {
                PurgeCategory::Handoff => report.purged_handoff += 1,
                PurgeCategory::Retracted => report.purged_retracted += 1,
                PurgeCategory::Quarantined => report.purged_quarantined += 1,
            }
        } else if is_tier1_cold_eligible(claim, now_ms, cfg) {
            cold_ids.insert(claim.id.clone());
        }
    }

    report.archived_claims = cold_ids.len() as u64;

    // 1. Tier-1 Cold Archival (`GC-2`) — read ONLY hot segments so previously archived
    //    cold records of re-promoted claims are never duplicated.
    if !cold_ids.is_empty() {
        let hot_events = stream_hot_events(root)?;
        let cold_events: Vec<LogEvent> = hot_events
            .into_iter()
            .filter(|ev| {
                event_claim_id(ev).is_some_and(|cid| cold_ids.contains(cid))
            })
            .collect();
        report.archived_events = cold_events.len() as u64;

        if !dry_run && !cold_events.is_empty() {
            let (_arch_name, locs) = write_archive_segment(root, &cold_events)?;
            let loc_by_seq: HashMap<u64, ArchiveRecordLoc> = locs.into_iter().collect();

            // Find the latest ClaimUpserted event location per archived claim.
            let mut latest_upsert_loc: HashMap<String, ArchiveRecordLoc> = HashMap::new();
            for ev in &cold_events {
                if let Some(log_event::Payload::ClaimUpserted(up)) = &ev.payload {
                    if let Some(claim) = &up.claim {
                        if let Some(loc) = loc_by_seq.get(&ev.seq) {
                            latest_upsert_loc.insert(claim.id.clone(), loc.clone());
                        }
                    }
                }
            }

            for (cid, loc) in &latest_upsert_loc {
                db.mark_claim_cold(cid, loc)?;
            }

            // Rewrite hot segments to remove archived events so hot storage shrinks.
            let archived_seqs: HashSet<u64> = cold_events.iter().map(|e| e.seq).collect();
            for seg_path in list_hot_segments(root)? {
                let _ = rewrite_hot_segment_filtered(&seg_path, |ev| {
                    !archived_seqs.contains(&ev.seq)
                })?;
            }
            refresh_active_seg(root, active_seg)?;
        }
    }

    // 2. Tier-2 Purge (`GC-4`)
    if !dry_run && !purge_ids.is_empty() {
        for seg_path in list_hot_segments(root)? {
            let _ = rewrite_hot_segment_filtered(&seg_path, |ev| {
                !purge_ids.iter().any(|pid| event_references_claim(ev, pid))
            })?;
        }
        refresh_active_seg(root, active_seg)?;

        for arch_path in list_archive_segments(root)? {
            let (_kept, removed, new_locs) =
                rewrite_archive_segment_filtered(&arch_path, |ev| {
                    !purge_ids.iter().any(|pid| event_references_claim(ev, pid))
                })?;
            if removed > 0 && !new_locs.is_empty() {
                update_shifted_archive_locs(root, db, &arch_path, &new_locs)?;
            }
        }

        for pid in &purge_ids {
            db.purge_claim_row(pid)?;
        }
    }

    // 3. Fold & prune old `claim_exposures` (`GC-4 (d)`)
    let exp_cutoff = now_ms.saturating_sub((cfg.exposure_fold_days as i64).saturating_mul(MS_PER_DAY));
    report.folded_exposures = db.fold_and_prune_exposures(exp_cutoff, dry_run)?;

    // 4. Mark-and-sweep unreferenced blobs (`GC-4 (e)`)
    let referenced = collect_referenced_blobs(root)?;
    let (swept, freed) = sweep_unreferenced_blobs(root, &referenced, dry_run)?;
    report.swept_blobs = swept;
    report.freed_blob_bytes = freed;

    // 5. Incremental vacuum & metadata update
    if !dry_run {
        db.incremental_vacuum()?;
        db.set_meta("last_gc_ms", &now_ms.to_string())?;
        db.set_meta("writes_since_gc", "0")?;
        report.vacuumed = true;
    }

    Ok(report)
}

/// Immediately erases a claim's payload from `index.db`, `claims_fts`, hot `.pblog`
/// segments, cold `.pblog.zst` archives, and orphan blobs, appending a minimal
/// `ClaimForgotten` tombstone (`GC-5`).
///
/// Caller must hold `WriteLock`.
#[allow(clippy::too_many_arguments)]
pub fn forget_claim_immediately(
    root: &Path,
    db: &mut ProjectionDb,
    active_seg: &mut Option<PathBuf>,
    next_seq: u64,
    actor: Principal,
    claim_id: &str,
    reason: &str,
    now_ms: i64,
) -> Result<()> {
    // 1. Rewrite hot segments removing all events referencing `claim_id`.
    for seg_path in list_hot_segments(root)? {
        let _ = rewrite_hot_segment_filtered(&seg_path, |ev| {
            !event_references_claim(ev, claim_id)
        })?;
    }
    refresh_active_seg(root, active_seg)?;

    // 2. Rewrite cold archives removing all events referencing `claim_id` and
    //    updating shifted archive coordinates in `index.db`.
    for arch_path in list_archive_segments(root)? {
        let (_kept, removed, new_locs) = rewrite_archive_segment_filtered(&arch_path, |ev| {
            !event_references_claim(ev, claim_id)
        })?;
        if removed > 0 && !new_locs.is_empty() {
            update_shifted_archive_locs(root, db, &arch_path, &new_locs)?;
        }
    }

    // 3. Append minimal tombstone event to the active hot segment.
    let tombstone = LogEvent {
        seq: next_seq,
        event_id: new_event_id(now_ms),
        recorded_at_ms: now_ms,
        actor: Some(actor),
        payload: Some(log_event::Payload::ClaimForgotten(ClaimForgotten {
            claim_id: claim_id.to_string(),
            reason: reason.to_string(),
        })),
    };
    append_hot_event(root, active_seg, &tombstone, DEFAULT_HOT_SEGMENT_MAX_BYTES)?;

    // 4. Remove from `index.db` + `claims_fts`, advance `applied_seq`, and
    //    VACUUM + truncate WAL so zero erased payload bytes linger on disk (`GC-5`).
    db.apply_event(&tombstone, None)?;
    db.secure_purge_vacuum()?;

    // 5. Immediately sweep any blobs orphaned by this erasure.
    let referenced = collect_referenced_blobs(root)?;
    let _ = sweep_unreferenced_blobs(root, &referenced, false)?;

    Ok(())
}

fn refresh_active_seg(root: &Path, active_seg: &mut Option<PathBuf>) -> Result<()> {
    let segs = list_hot_segments(root)?;
    *active_seg = segs.last().cloned();
    Ok(())
}

fn update_shifted_archive_locs(
    _root: &Path,
    db: &mut ProjectionDb,
    arch_path: &Path,
    new_locs: &[(u64, ArchiveRecordLoc)],
) -> Result<()> {
    let loc_by_seq: HashMap<u64, &ArchiveRecordLoc> =
        new_locs.iter().map(|(seq, loc)| (*seq, loc)).collect();
    let frames = crate::log::read_archive_file(arch_path)?;
    for (_, recs) in frames {
        for (_, _, ev) in recs {
            if let Some(log_event::Payload::ClaimUpserted(up)) = &ev.payload {
                if let Some(claim) = &up.claim {
                    if let Some(loc) = loc_by_seq.get(&ev.seq) {
                        db.update_archive_loc(&claim.id, loc)?;
                    }
                }
            }
        }
    }
    Ok(())
}
