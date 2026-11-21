//! Fail-open prompt and error-cue hook handlers (`HOOK-1`, `HOOK-2`, `HOOK-3`).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use lore_context::{EnvHints, detect, normalize_error_signature};
use lore_engine::{RecallRequest, format_evidence_envelope};
use lore_proto::{ClaimKind, L0Card, PathOrErrorOverlap, PeerPresence};
use regex::Regex;
use serde_json::Value;

use crate::backend::LoreBackend;

/// Token budget for `lore hook prompt` output (`HOOK-1`).
pub const PROMPT_HOOK_TOKEN_BUDGET: usize = 350;

/// Token budget for `lore hook cue` output (`HOOK-2`).
pub const CUE_HOOK_TOKEN_BUDGET: usize = 250;

/// Minimum score threshold for high-confidence `lore hook cue` matches (`HOOK-2`).
const CUE_MIN_SCORE: f64 = 0.25;

/// Approximate token count (~4 UTF-8 bytes per token, rounded up).
fn approx_tokens(text: &str) -> usize {
    text.len().div_ceil(4)
}

/// Fail-open `lore hook prompt` handler (`HOOK-1`, `HOOK-3`).
///
/// Returns `None` on disabled config, malformed stdin, backend errors, or when
/// there are no unsurfaced relevant cards or overlapping peers.
pub fn handle_prompt_hook<B: LoreBackend>(
    backend: &mut B,
    stdin_bytes: &[u8],
    cwd: &Path,
    env: &EnvHints,
) -> Option<String> {
    if !backend.config().memory.enabled {
        return None;
    }

    let payload = parse_stdin_json(stdin_bytes)?;
    let query = extract_string_field(&payload, &["prompt", "user_prompt", "query", "message"])
        .unwrap_or_default();

    let effective_cwd = extract_string_field(&payload, &["cwd"])
        .map(PathBuf::from)
        .unwrap_or_else(|| cwd.to_path_buf());

    let mut effective_env = env.clone();
    if let Some(sid) = extract_string_field(&payload, &["session_id", "sessionId"]) {
        effective_env.session_id = Some(sid);
    }

    let frame = detect(&effective_cwd, &effective_env);
    let mut req = RecallRequest::new(frame);
    req.query = query;
    req.limit = Some(6);
    req.include_pulse = true;
    req.suppress_exposed = true;
    req.record_exposures = true;

    let resp = backend.recall(req).ok()?;
    if resp.cards.is_empty() && resp.overlaps.is_empty() && resp.peers.is_empty() {
        return None;
    }

    format_budgeted_output(
        &resp.cards,
        &resp.peers,
        &resp.overlaps,
        resp.notice.as_deref(),
        PROMPT_HOOK_TOKEN_BUDGET,
    )
}

/// Fail-open `lore hook cue` handler (`HOOK-2`, `HOOK-3`).
///
/// Extracts normalized `err:<sig>` signatures and file paths from failing tool
/// or command output, queries matching `SOLUTION`/`DEAD_END`/`WARNING` claims
/// and peer failures, and emits `<= 250` tokens only when high-confidence
/// matches exist.
pub fn handle_cue_hook<B: LoreBackend>(
    backend: &mut B,
    stdin_bytes: &[u8],
    cwd: &Path,
    env: &EnvHints,
) -> Option<String> {
    if !backend.config().memory.enabled {
        return None;
    }

    let payload = parse_stdin_json(stdin_bytes)?;

    // If an explicit exit_code is 0 and is_error is not true, nothing failed.
    let exit_code = payload
        .get("exit_code")
        .or_else(|| payload.get("exitCode"))
        .and_then(Value::as_i64);
    let is_error = payload
        .get("is_error")
        .or_else(|| payload.get("isError"))
        .and_then(Value::as_bool);
    if exit_code == Some(0) && is_error != Some(true) {
        return None;
    }

    let error_text = collect_error_text(&payload);
    if error_text.trim().is_empty() {
        return None;
    }

    let sigs = normalize_error_signature(&error_text);
    let paths = extract_file_paths(&error_text);
    if sigs.is_empty() && paths.is_empty() {
        return None;
    }

    let effective_cwd = extract_string_field(&payload, &["cwd"])
        .map(PathBuf::from)
        .unwrap_or_else(|| cwd.to_path_buf());

    let mut effective_env = env.clone();
    if let Some(sid) = extract_string_field(&payload, &["session_id", "sessionId"]) {
        effective_env.session_id = Some(sid);
    }

    let frame = detect(&effective_cwd, &effective_env);
    let mut about: Vec<String> = sigs.clone();
    for p in &paths {
        about.push(format!("file:{p}"));
    }

    // Build a concise query string from extracted error signatures, paths, and first error line.
    let mut query_parts: Vec<String> = Vec::new();
    for s in &sigs {
        query_parts.push(s.clone());
    }
    for p in paths.iter().take(3) {
        query_parts.push(p.clone());
    }
    if let Some(first_line) = error_text.lines().find(|l| !l.trim().is_empty()) {
        let clipped: String = first_line.trim().chars().take(160).collect();
        query_parts.push(clipped);
    }

    let mut req = RecallRequest::new(frame);
    req.query = query_parts.join(" ");
    req.about = about;
    req.kinds = Some(vec![
        ClaimKind::Solution,
        ClaimKind::DeadEnd,
        ClaimKind::Warning,
    ]);
    req.limit = Some(4);
    req.include_pulse = true;
    req.suppress_exposed = true;
    req.record_exposures = true;
    req.touched_paths = paths;
    req.recent_failures = sigs;

    let resp = backend.recall(req).ok()?;
    let high_conf_cards: Vec<L0Card> = resp
        .cards
        .into_iter()
        .filter(|c| c.score >= CUE_MIN_SCORE)
        .collect();

    if high_conf_cards.is_empty() && resp.overlaps.is_empty() {
        return None;
    }

    format_budgeted_output(
        &high_conf_cards,
        &[],
        &resp.overlaps,
        resp.notice.as_deref(),
        CUE_HOOK_TOKEN_BUDGET,
    )
}

fn parse_stdin_json(stdin_bytes: &[u8]) -> Option<Value> {
    if stdin_bytes.is_empty() {
        return None;
    }
    let text = std::str::from_utf8(stdin_bytes).ok()?.trim();
    if text.is_empty() {
        return None;
    }
    serde_json::from_str::<Value>(text).ok()
}

fn extract_string_field(val: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(s) = val.get(*key).and_then(Value::as_str) {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn collect_error_text(val: &Value) -> String {
    let mut parts = Vec::new();
    for key in ["stderr", "error", "tool_response", "output", "stdout"] {
        if let Some(field) = val.get(key) {
            match field {
                Value::String(s) if !s.trim().is_empty() => parts.push(s.as_str().to_string()),
                Value::Object(map) => {
                    for subkey in ["stderr", "error", "message", "output"] {
                        if let Some(s) = map.get(subkey).and_then(Value::as_str) {
                            if !s.trim().is_empty() {
                                parts.push(s.to_string());
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    parts.join("\n")
}

fn file_path_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?:^|[\s:'\(\[,])([A-Za-z0-9_.\-/]+\.(?:rs|go|py|java|bzl|bazel|swift|toml|json|md|ts|tsx|js|c|cc|cpp|h|hpp|sh))",
        )
        .expect("valid file path regex")
    })
}

fn extract_file_paths(text: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for cap in file_path_re().captures_iter(text) {
        if let Some(m) = cap.get(1) {
            let p = m.as_str().trim_start_matches("./").to_string();
            if seen.insert(p.clone()) {
                out.push(p);
                if out.len() >= 8 {
                    break;
                }
            }
        }
    }
    out
}

fn format_budgeted_output(
    cards: &[L0Card],
    peers: &[PeerPresence],
    overlaps: &[PathOrErrorOverlap],
    notice: Option<&str>,
    token_budget: usize,
) -> Option<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut used_tokens = 0usize;

    let mut try_push = |line: String| -> bool {
        let cost = approx_tokens(&line) + 1;
        if used_tokens + cost <= token_budget {
            used_tokens += cost;
            lines.push(line);
            true
        } else {
            false
        }
    };

    for card in cards {
        let env_line = format_evidence_envelope(card);
        if !try_push(env_line) {
            break;
        }
    }

    for ov in overlaps {
        let mut details = Vec::new();
        if !ov.overlapping_paths.is_empty() {
            details.push(format!("paths: {}", ov.overlapping_paths.join(", ")));
        }
        if !ov.overlapping_errors.is_empty() {
            details.push(format!("errors: {}", ov.overlapping_errors.join(", ")));
        }
        let line = format!(
            "[peer overlap {} ({}@{}) | {}]",
            ov.peer_session_id,
            ov.peer_agent_kind,
            ov.peer_machine_id,
            details.join("; ")
        );
        if !try_push(line) {
            break;
        }
    }

    if overlaps.is_empty() {
        for peer in peers.iter().take(2) {
            let intent = peer.intent.as_deref().unwrap_or("active");
            let line = format!(
                "[peer {} ({}@{}) | {}]",
                peer.session_id, peer.agent_kind, peer.machine_id, intent
            );
            if !try_push(line) {
                break;
            }
        }
    }

    if let Some(n) = notice {
        let _ = try_push(format!("Note: {n}"));
    }

    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}
