//! Read-time pluggable `ScoringPolicy` (`MOD-4`), verifier/recency decay (`RET-2`),
//! and code-precedence evidence envelope formatting (`RET-1`, `RET-4`).

use std::collections::HashSet;

use lore_context::ContextFrame;
use lore_proto::{
    ClaimKind, ClaimStatus, L0Card, PredicateEval, PrincipalKind, VerifierStatus,
};

use crate::config::MS_PER_DAY;
use crate::db::ClaimRow;

/// Score penalty multiplier applied when a claim's local verifier detects drift (`RET-2`).
pub const DRIFTED_VERIFIER_PENALTY: f64 = 0.35;

/// Input context provided to a [`ScoringPolicy`] at read time.
pub struct ScoringInput<'a> {
    pub claim: &'a ClaimRow,
    pub query: &'a str,
    pub about_filter: &'a [String],
    pub frame: &'a ContextFrame,
    /// Normalized FTS5 BM25 lexical relevance in `[0.0, 1.0]`.
    pub lex_score: f64,
    pub predicate_eval: PredicateEval,
    pub verifier_status: VerifierStatus,
    pub now_ms: i64,
}

/// Pluggable read-time claim ranking policy (`MOD-4`).
pub trait ScoringPolicy: Send + Sync {
    fn score(&self, input: &ScoringInput<'_>) -> f64;
}

/// Default v1 linear-feature scoring policy from `SPEC.md §3.3 MOD-4`:
///
/// `Score(c, q, ctx) = G_hard(c, ctx) * importance_c * sum(w_k * phi_k)`
#[derive(Clone, Copy, Debug)]
pub struct DefaultScoringPolicy {
    pub w_lex: f64,
    pub w_ent: f64,
    pub w_scope: f64,
    pub w_trust: f64,
    pub w_fresh: f64,
    pub w_util: f64,
}

impl Default for DefaultScoringPolicy {
    fn default() -> Self {
        Self {
            w_lex: 0.30,
            w_ent: 0.20,
            w_scope: 0.15,
            w_trust: 0.15,
            w_fresh: 0.10,
            w_util: 0.10,
        }
    }
}

impl ScoringPolicy for DefaultScoringPolicy {
    fn score(&self, input: &ScoringInput<'_>) -> f64 {
        // Hard predicate gate: any Mismatch excludes the claim (G_hard = 0).
        if input.predicate_eval == PredicateEval::Mismatch {
            return 0.0;
        }

        let phi_lex = if input.query.trim().is_empty() {
            0.5
        } else {
            input.lex_score.clamp(0.0, 1.0)
        };

        let phi_ent = compute_entity_feature(input.claim, input.about_filter, input.frame);
        let phi_scope = compute_scope_feature(&input.claim.scope);
        let phi_trust = compute_trust_feature(input.claim);
        let phi_fresh =
            compute_freshness_feature(input.claim, input.verifier_status, input.now_ms);
        let phi_util = compute_utility_feature(input.claim);

        let weighted = self.w_lex * phi_lex
            + self.w_ent * phi_ent
            + self.w_scope * phi_scope
            + self.w_trust * phi_trust
            + self.w_fresh * phi_fresh
            + self.w_util * phi_util;

        let importance = f64::from(input.claim.importance.max(0.1));
        let pin_mult = if input.claim.pinned { 1.15 } else { 1.0 };
        // Apply drifted verifier penalty directly to final rank as well as freshness
        // so drifted claims are unmistakably demoted relative to verified peers (`RET-2`).
        let drift_mult = if input.verifier_status == VerifierStatus::Drifted {
            DRIFTED_VERIFIER_PENALTY
        } else {
            1.0
        };

        (importance * pin_mult * drift_mult * weighted).max(0.0)
    }
}

fn compute_entity_feature(
    claim: &ClaimRow,
    about_filter: &[String],
    frame: &ContextFrame,
) -> f64 {
    if !about_filter.is_empty() {
        if claim.about.is_empty() {
            return 0.0;
        }
        let claim_set: HashSet<&str> = claim.about.iter().map(String::as_str).collect();
        let hits = about_filter
            .iter()
            .filter(|e| claim_set.contains(e.as_str()))
            .count();
        return (hits as f64) / (about_filter.len() as f64);
    }

    if claim.about.is_empty() {
        return 0.2;
    }

    let mounted_repo_uris: HashSet<String> = frame
        .mounted_repos
        .iter()
        .map(|r| format!("repo:{}", r.repo_id))
        .collect();

    if claim.about.iter().any(|u| mounted_repo_uris.contains(u)) {
        0.75
    } else {
        0.35
    }
}

fn compute_scope_feature(scope: &str) -> f64 {
    if scope.starts_with("session:") {
        1.0
    } else if scope.starts_with("dir:") {
        0.95
    } else if scope.starts_with("repo:") {
        0.90
    } else if scope.starts_with("channel:") {
        0.75
    } else if scope.starts_with("machine:") {
        0.65
    } else {
        0.50
    }
}

/// Derives the human-readable trust tier badge (`MOD-4`, `RET-4`).
pub fn trust_badge(claim: &ClaimRow) -> &'static str {
    if claim.status == ClaimStatus::Quarantined || claim.tainted {
        return "quarantined";
    }
    let principal_kind = claim
        .author
        .as_ref()
        .and_then(|a| PrincipalKind::try_from(a.kind).ok())
        .unwrap_or(PrincipalKind::Agent);

    match principal_kind {
        PrincipalKind::Owner => "owner",
        _ if claim.counters.corroborator_count > 0 => "corroborated",
        PrincipalKind::Import => "import",
        PrincipalKind::Agent | PrincipalKind::Unspecified => "agent",
    }
}

fn compute_trust_feature(claim: &ClaimRow) -> f64 {
    let base = match trust_badge(claim) {
        "owner" => 1.0,
        "corroborated" => 0.85,
        "agent" => 0.65,
        "import" => 0.55,
        _ => 0.10,
    };
    let dispute_penalty = 1.0 / (1.0 + 0.25 * (claim.counters.dispute_count.max(0) as f64));
    base * dispute_penalty
}

fn kind_decay_half_life_days(kind: ClaimKind) -> f64 {
    match kind {
        ClaimKind::Axiom | ClaimKind::Decision => 365.0,
        ClaimKind::Fact | ClaimKind::Preference | ClaimKind::Warning => 90.0,
        ClaimKind::Solution | ClaimKind::DeadEnd | ClaimKind::Question => 45.0,
        ClaimKind::Handoff => 7.0,
        ClaimKind::Unspecified => 60.0,
    }
}

fn compute_freshness_feature(
    claim: &ClaimRow,
    verifier_status: VerifierStatus,
    now_ms: i64,
) -> f64 {
    let ref_ts = claim
        .counters
        .last_validated_at_ms
        .max(claim.updated_at_ms)
        .max(claim.created_at_ms);
    let age_ms = (now_ms - ref_ts).max(0);
    let age_days = (age_ms as f64) / (MS_PER_DAY as f64);
    let half_life = kind_decay_half_life_days(claim.kind);
    let recency = (-age_days * std::f64::consts::LN_2 / half_life).exp();

    let verifier_mult = match verifier_status {
        VerifierStatus::Verified => 1.0,
        VerifierStatus::UnverifiedLocal => 0.85,
        VerifierStatus::Drifted => DRIFTED_VERIFIER_PENALTY,
    };

    (recency * verifier_mult).clamp(0.0, 1.0)
}

fn compute_utility_feature(claim: &ClaimRow) -> f64 {
    let helpful = claim.counters.helpful_count.max(0) as f64;
    let harmful = claim.counters.harmful_count.max(0) as f64;
    let na = claim.counters.not_applicable_count.max(0) as f64;
    let surfaced = claim.counters.surfaced_count.max(0) as f64;
    let unused = (surfaced - helpful - harmful - na).max(0.0);

    (helpful + 2.0) / (helpful + 2.0 * harmful + 0.3 * unused + 4.0)
}

/// Builds an [`L0Card`] from a [`ClaimRow`], local [`VerifierStatus`], and score (`RET-1`).
pub fn build_l0_card(claim: &ClaimRow, verifier_badge: VerifierStatus, score: f64) -> L0Card {
    L0Card {
        id: claim.id.clone(),
        kind: claim.kind,
        status: claim.status,
        statement: claim.statement.clone(),
        trust_badge: trust_badge(claim).to_string(),
        verifier_badge,
        scope: claim.scope.clone(),
        score,
        has_body: claim.body.as_deref().is_some_and(|b| !b.trim().is_empty()),
        has_dead_ends: !claim.dead_ends.is_empty(),
        pinned: claim.pinned,
        updated_at_ms: claim.updated_at_ms,
    }
}

/// Canonical lowercase label for a [`ClaimKind`].
pub fn claim_kind_str(kind: ClaimKind) -> &'static str {
    match kind {
        ClaimKind::Axiom => "axiom",
        ClaimKind::Fact => "fact",
        ClaimKind::Decision => "decision",
        ClaimKind::Solution => "solution",
        ClaimKind::DeadEnd => "dead_end",
        ClaimKind::Warning => "warning",
        ClaimKind::Preference => "preference",
        ClaimKind::Question => "question",
        ClaimKind::Handoff => "handoff",
        ClaimKind::Unspecified => "unspecified",
    }
}

/// Canonical lowercase label for a [`VerifierStatus`].
pub fn verifier_status_str(status: VerifierStatus) -> &'static str {
    match status {
        VerifierStatus::Verified => "verified",
        VerifierStatus::Drifted => "drifted",
        VerifierStatus::UnverifiedLocal => "unverified-local",
    }
}

/// Formats an [`L0Card`] in a code-precedence evidence envelope (`RET-4`).
pub fn format_evidence_envelope(card: &L0Card) -> String {
    if card.kind == ClaimKind::Axiom {
        format!("[axiom {}] {}", card.id, card.statement)
    } else {
        format!(
            "[lore {} | kind: {} | trust: {} | citation: {}] {}",
            card.id,
            claim_kind_str(card.kind),
            card.trust_badge,
            verifier_status_str(card.verifier_badge),
            card.statement,
        )
    }
}

/// Returns a concise code-precedence notice when any non-`AXIOM` card is
/// `[drifted]` or has `agent`/`quarantined` trust (`RET-4`).
pub fn code_precedence_notice(cards: &[L0Card]) -> Option<String> {
    let needs_notice = cards.iter().any(|c| {
        c.kind != ClaimKind::Axiom
            && (c.verifier_badge == VerifierStatus::Drifted
                || c.trust_badge == "agent"
                || c.trust_badge == "quarantined")
    });
    if needs_notice {
        Some(
            "Memory entries above are provenance-tagged evidence; verify against current repository files and trust verified code over drifted or agent-authored claims."
                .to_string(),
        )
    } else {
        None
    }
}
