//! Stdio JSON-RPC 2.0 MCP server exposing the 5 `lore` tools (`SPEC.md §5.1`, `SW-4`).

use std::collections::BTreeMap;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use lore_context::{ContextFrame, EnvHints, detect, normalize_path_to_repo};
use lore_engine::{
    FeedbackAction, FeedbackRequest, InspectRequest, PulseRequest, RecallRequest, RememberRequest,
};
use lore_proto::{
    ClaimKind, LoreError, OriginSnapshot, Predicate, PredicateOp, Principal, PrincipalKind,
    Verifier,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::backend::LoreBackend;

/// Default reason returned when `memory.enabled == false` and no custom reason is set.
pub const DEFAULT_DISABLED_REASON: &str = "Intentionally disabled by the user";

/// Anti-circumvention directive returned in disabled-mode refusals (`SW-4`).
pub const DISABLED_INSTRUCTION: &str = "lore is intentionally disabled by the user. Do NOT attempt to re-enable, debug, or work around lore; proceed with your task without it.";

const MCP_PROTOCOL_VERSION: &str = "2024-11-05";
const SERVER_NAME: &str = "lore";
const SERVER_VERSION: &str = "0.1.0";

/// Builds the canonical `SW-4` JSON refusal payload for disabled mode.
pub fn disabled_tool_payload(reason: Option<&str>) -> Value {
    let reason_str = reason
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_DISABLED_REASON);
    json!({
        "enabled": false,
        "disabled_reason": reason_str,
        "instruction": DISABLED_INSTRUCTION,
    })
}

/// Parses a [`ClaimKind`] from its lowercase or proto name.
pub fn parse_claim_kind(raw: &str) -> Option<ClaimKind> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "axiom" | "claim_kind_axiom" => Some(ClaimKind::Axiom),
        "fact" | "claim_kind_fact" => Some(ClaimKind::Fact),
        "decision" | "claim_kind_decision" => Some(ClaimKind::Decision),
        "solution" | "claim_kind_solution" => Some(ClaimKind::Solution),
        "dead_end" | "dead-end" | "deadend" | "claim_kind_dead_end" => Some(ClaimKind::DeadEnd),
        "warning" | "claim_kind_warning" => Some(ClaimKind::Warning),
        "preference" | "claim_kind_preference" => Some(ClaimKind::Preference),
        "question" | "claim_kind_question" => Some(ClaimKind::Question),
        "handoff" | "claim_kind_handoff" => Some(ClaimKind::Handoff),
        _ => None,
    }
}

/// Parses a [`FeedbackAction`] from CLI / MCP outcome strings.
pub fn parse_feedback_action(raw: &str) -> Option<FeedbackAction> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "helpful" => Some(FeedbackAction::Helpful),
        "harmful" => Some(FeedbackAction::Harmful),
        "not_applicable" | "not-applicable" | "na" => Some(FeedbackAction::NotApplicable),
        "corroborated" | "corroborate" => Some(FeedbackAction::Corroborated),
        "disputed" | "dispute" => Some(FeedbackAction::Disputed),
        "invalidate" | "invalidated" => Some(FeedbackAction::Invalidate),
        "retract" | "retracted" => Some(FeedbackAction::Retract),
        _ => None,
    }
}

fn parse_predicate_op(raw: &str) -> Option<PredicateOp> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "eq" | "=" | "==" => Some(PredicateOp::Eq),
        "in" => Some(PredicateOp::In),
        "glob" => Some(PredicateOp::Glob),
        "semver_req" | "semver-req" | "semver" => Some(PredicateOp::SemverReq),
        "ancestor_of" | "ancestor-of" => Some(PredicateOp::AncestorOf),
        "not_ancestor_of" | "not-ancestor-of" => Some(PredicateOp::NotAncestorOf),
        _ => None,
    }
}

/// Stdio JSON-RPC 2.0 MCP server for `lore`.
pub struct McpServer<B: LoreBackend> {
    backend: B,
    cwd: PathBuf,
    env: EnvHints,
}

impl<B: LoreBackend> McpServer<B> {
    pub fn new(backend: B, cwd: PathBuf, env: EnvHints) -> Self {
        Self { backend, cwd, env }
    }

    /// Consumes the server and returns the inner backend.
    pub fn into_backend(self) -> B {
        self.backend
    }

    /// Runs the newline-delimited JSON-RPC 2.0 stdio loop until EOF.
    pub fn run_stdio<R: BufRead, W: Write>(&mut self, mut reader: R, mut writer: W) -> io::Result<()> {
        let mut line = String::new();
        loop {
            line.clear();
            let n = reader.read_line(&mut line)?;
            if n == 0 {
                break;
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Some(reply) = self.handle_line(trimmed) {
                writer.write_all(reply.as_bytes())?;
                writer.write_all(b"\n")?;
                writer.flush()?;
            }
        }
        Ok(())
    }

    /// Handles one JSON-RPC 2.0 request line. Returns `None` for notifications.
    pub fn handle_line(&mut self, line: &str) -> Option<String> {
        let req: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(err) => {
                let resp = jsonrpc_error(Value::Null, -32700, format!("Parse error: {err}"));
                return Some(resp.to_string());
            }
        };

        let id = req.get("id").cloned();
        let method = match req.get("method").and_then(Value::as_str) {
            Some(m) => m,
            None => {
                return id.map(|req_id| {
                    jsonrpc_error(req_id, -32600, "Invalid Request: missing method".into())
                        .to_string()
                });
            }
        };

        // Notifications (no `id` field) never receive a JSON-RPC response.
        if method == "notifications/initialized" || id.is_none() {
            return None;
        }
        let req_id = id.unwrap_or(Value::Null);
        let params = req.get("params").cloned().unwrap_or_else(|| json!({}));

        let resp = match method {
            "initialize" => jsonrpc_ok(
                req_id,
                json!({
                    "protocolVersion": MCP_PROTOCOL_VERSION,
                    "capabilities": {
                        "tools": {}
                    },
                    "serverInfo": {
                        "name": SERVER_NAME,
                        "version": SERVER_VERSION
                    }
                }),
            ),
            "ping" => jsonrpc_ok(req_id, json!({})),
            "tools/list" => jsonrpc_ok(req_id, json!({ "tools": tool_definitions() })),
            "tools/call" => self.handle_tools_call(req_id, params),
            other => jsonrpc_error(req_id, -32601, format!("Method not found: {other}")),
        };

        Some(resp.to_string())
    }

    fn handle_tools_call(&mut self, req_id: Value, params: Value) -> Value {
        let Some(name) = params.get("name").and_then(Value::as_str) else {
            return jsonrpc_error(req_id, -32602, "Invalid params: missing tool name".into());
        };
        let args = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));

        // Verify tool name first so unknown tools still produce a clear error.
        if !matches!(name, "recall" | "inspect" | "remember" | "feedback" | "pulse") {
            return jsonrpc_error(req_id, -32602, format!("Unknown tool: {name}"));
        }

        // SW-4: When `memory.enabled == false`, immediately refuse without touching storage.
        let cfg = self.backend.config();
        if !cfg.memory.enabled {
            let payload = disabled_tool_payload(cfg.memory.disabled_reason.as_deref());
            return jsonrpc_ok(req_id, tool_text_result(&payload, false));
        }

        let outcome = match name {
            "recall" => self.call_recall(args),
            "inspect" => self.call_inspect(args),
            "remember" => self.call_remember(args),
            "feedback" => self.call_feedback(args),
            "pulse" => self.call_pulse(args),
            _ => unreachable!(),
        };

        match outcome {
            Ok(payload) => jsonrpc_ok(req_id, tool_text_result(&payload, false)),
            Err(LoreError::Disabled { reason }) => {
                let payload = disabled_tool_payload(reason.as_deref());
                jsonrpc_ok(req_id, tool_text_result(&payload, false))
            }
            Err(err) => jsonrpc_ok(
                req_id,
                json!({
                    "content": [{ "type": "text", "text": err.to_string() }],
                    "isError": true
                }),
            ),
        }
    }

    fn detect_frame(&self) -> ContextFrame {
        detect(&self.cwd, &self.env)
    }

    fn call_recall(&mut self, args: Value) -> lore_proto::Result<Value> {
        let input: RecallArgs = serde_json::from_value(args).map_err(|e| LoreError::InvalidScope {
            scope: format!("invalid recall arguments: {e}"),
        })?;

        let frame = self.detect_frame();
        let mut req = RecallRequest::new(frame.clone());
        req.query = input.query.unwrap_or_default();
        req.about = input.about;
        req.scope_override = input.scope;
        req.limit = input.limit;
        req.include_history = input.include_history;
        req.include_pulse = input.include_pulse.unwrap_or(true);
        req.touched_paths = input.touched_paths;
        req.recent_failures = input.recent_failures;

        if !input.kinds.is_empty() {
            let mut parsed = Vec::with_capacity(input.kinds.len());
            for k in &input.kinds {
                let kind = parse_claim_kind(k).ok_or_else(|| LoreError::InvalidScope {
                    scope: format!("unknown claim kind `{k}`"),
                })?;
                parsed.push(kind);
            }
            req.kinds = Some(parsed);
        }

        let resp = self.backend.recall(req)?;
        Ok(json!({
            "enabled": resp.enabled,
            "context_frame": frame,
            "cards": resp.cards,
            "peers": resp.peers,
            "overlaps": resp.overlaps,
            "notice": resp.notice,
        }))
    }

    fn call_inspect(&mut self, args: Value) -> lore_proto::Result<Value> {
        let input: InspectArgs =
            serde_json::from_value(args).map_err(|e| LoreError::ClaimNotFound {
                id: format!("invalid inspect arguments: {e}"),
            })?;

        let frame = self.detect_frame();
        let resp = self.backend.inspect(InspectRequest {
            id: input.id,
            frame: Some(frame),
            include_lineage: input.include_lineage,
        })?;

        Ok(json!({
            "enabled": resp.enabled,
            "claim": resp.claim,
            "counters": resp.counters,
            "edges": resp.edges,
            "verifier_status": resp.verifier_status,
            "verifier_results": resp.verifier_results,
            "hydrated_from_cold": resp.hydrated_from_cold,
        }))
    }

    fn call_remember(&mut self, args: Value) -> lore_proto::Result<Value> {
        let input: RememberArgs =
            serde_json::from_value(args).map_err(|e| LoreError::InvalidScope {
                scope: format!("invalid remember arguments: {e}"),
            })?;

        let kind = parse_claim_kind(&input.kind).ok_or_else(|| LoreError::InvalidScope {
            scope: format!("unknown claim kind `{}`", input.kind),
        })?;

        let frame = self.detect_frame();
        let mut about = input.about;
        let scope = resolve_write_scope(input.scope.as_deref(), &frame, &mut about);

        let mut applies_when = Vec::with_capacity(input.applies_when.len());
        for p in input.applies_when {
            let op = parse_predicate_op(&p.op).ok_or_else(|| LoreError::InvalidPredicate {
                axis: p.axis.clone(),
                op: p.op.clone(),
                value: p.value.clone(),
            })?;
            applies_when.push(Predicate {
                axis: p.axis,
                op: op as i32,
                value: p.value,
            });
        }

        let origin = build_origin_snapshot(&frame);
        let author = build_principal(&frame);

        let req = RememberRequest {
            kind,
            statement: input.statement,
            body: input.body.unwrap_or_default(),
            dead_ends: input.dead_ends,
            keywords: input.keywords,
            attrs: input.attrs,
            scope,
            about,
            applies_when,
            author,
            tainted: input.tainted,
            origin: Some(origin),
            verifiers: input.verifiers,
            supersedes: input.supersedes,
            pinned: input.pinned,
            importance: input.importance.unwrap_or(1.0),
            valid_from_ms: 0,
            valid_until_ms: 0,
            redact_secrets: input.redact_secrets,
            event_id: None,
        };

        let resp = self.backend.remember(req)?;
        Ok(json!({
            "enabled": resp.enabled,
            "id": resp.id,
            "status": resp.status,
            "superseded_id": resp.superseded_id,
            "redacted_fields": resp.redacted_fields,
        }))
    }

    fn call_feedback(&mut self, args: Value) -> lore_proto::Result<Value> {
        let input: FeedbackArgs =
            serde_json::from_value(args).map_err(|e| LoreError::ClaimNotFound {
                id: format!("invalid feedback arguments: {e}"),
            })?;

        let action = parse_feedback_action(&input.outcome).ok_or_else(|| LoreError::InvalidScope {
            scope: format!("unknown feedback outcome `{}`", input.outcome),
        })?;

        let frame = self.detect_frame();
        let resp = self.backend.feedback(FeedbackRequest {
            id: input.id,
            action,
            note: input.note.unwrap_or_default(),
            actor: build_principal(&frame),
            tainted: input.tainted,
            event_id: None,
        })?;

        Ok(json!({
            "enabled": resp.enabled,
            "id": resp.id,
            "new_status": resp.new_status,
            "counters": resp.counters,
            "promoted_from_cold": resp.promoted_from_cold,
        }))
    }

    fn call_pulse(&mut self, args: Value) -> lore_proto::Result<Value> {
        let input: PulseArgs = serde_json::from_value(args).map_err(|e| LoreError::InvalidScope {
            scope: format!("invalid pulse arguments: {e}"),
        })?;

        let frame = self.detect_frame();
        let mut req = PulseRequest::new(frame);
        req.intent = input.intent;
        req.hypothesis = input.hypothesis;
        req.touched_paths = input.touched_paths;
        req.recent_failures = input.recent_failures;
        req.ttl_ms = input.ttl_ms;
        req.query_only = input.query_only;
        req.dirty_count = input.dirty_count;

        let resp = self.backend.pulse(req)?;
        Ok(json!({
            "enabled": resp.enabled,
            "peers": resp.peers,
            "overlaps": resp.overlaps,
        }))
    }
}

/// Formats a `repo_id` into a valid claim scope, avoiding double-prefixing
/// fallback `dir:<machine>:<hash>` scopes (`TOP-3`).
pub fn format_repo_scope(repo_id: &str) -> String {
    if repo_id.starts_with("dir:") || repo_id.starts_with("repo:") {
        repo_id.to_string()
    } else {
        format!("repo:{repo_id}")
    }
}

/// Resolves a default write scope from `ContextFrame` and normalizes `file:` `about` entries (`TOP-3`, `TOP-4`, `TOP-5`).
pub fn resolve_write_scope(
    explicit_scope: Option<&str>,
    frame: &ContextFrame,
    about: &mut Vec<String>,
) -> String {
    let mut matched_repos: Vec<String> = Vec::new();
    for entry in about.iter_mut() {
        if let Some(raw_path) = entry.strip_prefix("file:") {
            if let Some((repo_id, rel)) =
                normalize_path_to_repo(frame, PathBuf::from(raw_path).as_path())
            {
                *entry = format!("file:{rel}");
                if !matched_repos.contains(&repo_id) {
                    matched_repos.push(repo_id);
                }
            }
        }
    }

    if let Some(s) = explicit_scope.map(str::trim).filter(|s| !s.is_empty()) {
        return s.to_string();
    }

    if matched_repos.len() == 1 {
        return format_repo_scope(&matched_repos[0]);
    }
    if matched_repos.len() >= 2 {
        for repo_id in &matched_repos {
            let tag = format_repo_scope(repo_id);
            if !about.contains(&tag) {
                about.push(tag);
            }
        }
        return frame
            .channel_id
            .as_deref()
            .map(|c| format!("channel:{c}"))
            .unwrap_or_else(|| "global".to_string());
    }

    match frame.mounted_repos.len() {
        1 => format_repo_scope(&frame.mounted_repos[0].repo_id),
        0 => frame
            .channel_id
            .as_deref()
            .map(|c| format!("channel:{c}"))
            .unwrap_or_else(|| frame.cwd_dir_scope.clone()),
        _ => frame
            .channel_id
            .as_deref()
            .map(|c| format!("channel:{c}"))
            .unwrap_or_else(|| "global".to_string()),
    }
}

pub(crate) fn build_origin_snapshot(frame: &ContextFrame) -> OriginSnapshot {
    let first_repo = frame.mounted_repos.first();
    OriginSnapshot {
        machine_id: frame.machine_id.clone(),
        repo_id: first_repo.map(|r| r.repo_id.clone()).unwrap_or_default(),
        branch: first_repo
            .and_then(|r| r.branch.clone())
            .unwrap_or_default(),
        head_commit: first_repo
            .and_then(|r| r.head_sha.clone())
            .unwrap_or_default(),
        os: frame.os.clone(),
        arch: frame.arch.clone(),
    }
}

pub(crate) fn build_principal(frame: &ContextFrame) -> Principal {
    let agent_kind = frame
        .agent_kind
        .clone()
        .unwrap_or_else(|| "agent".to_string());
    let kind = if agent_kind.eq_ignore_ascii_case("human") {
        PrincipalKind::Owner
    } else {
        PrincipalKind::Agent
    };
    Principal {
        kind: kind as i32,
        id: agent_kind.clone(),
        session_id: frame.session_id.clone().unwrap_or_default(),
        agent_kind,
        model: String::new(),
        machine_id: frame.machine_id.clone(),
    }
}

fn jsonrpc_ok(id: Value, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    })
}

fn jsonrpc_error(id: Value, code: i32, message: String) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message,
        }
    })
}

fn tool_text_result(payload: &Value, is_error: bool) -> Value {
    json!({
        "content": [{
            "type": "text",
            "text": payload.to_string(),
        }],
        "isError": is_error,
    })
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RecallArgs {
    query: Option<String>,
    about: Vec<String>,
    kinds: Vec<String>,
    scope: Option<String>,
    limit: Option<usize>,
    include_history: bool,
    include_pulse: Option<bool>,
    touched_paths: Vec<String>,
    recent_failures: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct InspectArgs {
    id: String,
    #[serde(default)]
    include_lineage: bool,
}

#[derive(Debug, Deserialize)]
struct PredicateArg {
    axis: String,
    op: String,
    value: String,
}

#[derive(Debug, Deserialize)]
struct RememberArgs {
    kind: String,
    statement: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    dead_ends: Vec<String>,
    #[serde(default)]
    keywords: Vec<String>,
    #[serde(default)]
    attrs: BTreeMap<String, String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    about: Vec<String>,
    #[serde(default)]
    applies_when: Vec<PredicateArg>,
    #[serde(default)]
    verifiers: Vec<Verifier>,
    #[serde(default)]
    supersedes: Option<String>,
    #[serde(default)]
    pinned: bool,
    #[serde(default)]
    importance: Option<f32>,
    #[serde(default)]
    redact_secrets: bool,
    #[serde(default)]
    tainted: bool,
}

#[derive(Debug, Deserialize)]
struct FeedbackArgs {
    id: String,
    outcome: String,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    tainted: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct PulseArgs {
    intent: Option<String>,
    hypothesis: Option<String>,
    touched_paths: Vec<String>,
    recent_failures: Vec<String>,
    ttl_ms: Option<i64>,
    query_only: bool,
    dirty_count: u32,
}

fn tool_definitions() -> Value {
    json!([
        {
            "name": "recall",
            "description": "Recall ranked L0 memory cards relevant to the current workspace context and query, plus active peer presence.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "about": { "type": "array", "items": { "type": "string" } },
                    "kinds": { "type": "array", "items": { "type": "string" } },
                    "scope": { "type": "string" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 20 },
                    "include_history": { "type": "boolean" },
                    "include_pulse": { "type": "boolean" },
                    "touched_paths": { "type": "array", "items": { "type": "string" } },
                    "recent_failures": { "type": "array", "items": { "type": "string" } }
                }
            }
        },
        {
            "name": "inspect",
            "description": "Fetch full L1 details, dead ends, lineage edges, and local citation verifier results for a claim ID.",
            "inputSchema": {
                "type": "object",
                "required": ["id"],
                "properties": {
                    "id": { "type": "string" },
                    "include_lineage": { "type": "boolean" }
                }
            }
        },
        {
            "name": "remember",
            "description": "Record a new durable claim (axiom, fact, decision, solution, dead_end, warning, preference, question, or handoff).",
            "inputSchema": {
                "type": "object",
                "required": ["kind", "statement"],
                "properties": {
                    "kind": { "type": "string" },
                    "statement": { "type": "string", "maxLength": 280 },
                    "body": { "type": "string" },
                    "dead_ends": { "type": "array", "items": { "type": "string" } },
                    "keywords": { "type": "array", "items": { "type": "string" } },
                    "scope": { "type": "string" },
                    "about": { "type": "array", "items": { "type": "string" } },
                    "applies_when": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "required": ["axis", "op", "value"],
                            "properties": {
                                "axis": { "type": "string" },
                                "op": { "type": "string" },
                                "value": { "type": "string" }
                            }
                        }
                    },
                    "supersedes": { "type": "string" },
                    "pinned": { "type": "boolean" },
                    "importance": { "type": "number" },
                    "redact_secrets": { "type": "boolean" }
                }
            }
        },
        {
            "name": "feedback",
            "description": "Record outcome feedback or lifecycle transition (helpful, harmful, not_applicable, corroborated, disputed, invalidate, retract).",
            "inputSchema": {
                "type": "object",
                "required": ["id", "outcome"],
                "properties": {
                    "id": { "type": "string" },
                    "outcome": { "type": "string" },
                    "note": { "type": "string" }
                }
            }
        },
        {
            "name": "pulse",
            "description": "Publish ephemeral live session presence and detect overlapping active peers or shared failures.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "intent": { "type": "string" },
                    "hypothesis": { "type": "string" },
                    "touched_paths": { "type": "array", "items": { "type": "string" } },
                    "recent_failures": { "type": "array", "items": { "type": "string" } },
                    "ttl_ms": { "type": "integer" },
                    "query_only": { "type": "boolean" },
                    "dirty_count": { "type": "integer" }
                }
            }
        }
    ])
}
