//! 3-stage config rewrite pipeline:
//! - Stage 1: Declarative TOML rules (`path_rewrites`, `mcp_rules`).
//! - Stage 2: Capability-free pure-Rust Scheme / S-expression AST transformer
//!   with strict step & recursion budgets (zero I/O, zero network).
//! - Stage 3: Target-local `bash` filter (`~/.humata2/scripts/*.sh`) with
//!   bounded timeout and JSON stdin/stdout.
//! - Post-transform invariant validator: guarantees valid JSON/TOML schema and
//!   ensures the protected `humata` MCP server entry is never deleted or hijacked.

use crate::model::{AgentKind, ConfigFormat, glob_match};
use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value as JsonValue};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use thiserror::Error;

/// Context passed into all rewrite stages for the target machine & agent.
#[derive(Debug, Clone)]
pub struct RewriteContext {
    pub machine_id: String,
    pub machine_name: String,
    pub os: String,
    pub arch: String,
    pub home_dir: PathBuf,
    pub agent: AgentKind,
    /// Optional local environment overrides (used by `pass_env` in tests without
    /// mutating process-global environment variables).
    pub local_env: BTreeMap<String, String>,
}

impl RewriteContext {
    pub fn resolve_env(&self, key: &str) -> Option<String> {
        if let Some(v) = self.local_env.get(key) {
            return Some(v.clone());
        }
        std::env::var(key).ok()
    }
}

/// Protected `humata` MCP server entry that no rewrite stage may remove or alter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProtectedHumataMcp {
    /// Server key name inside `mcpServers` / `mcp_servers` (defaults to `"humata"`).
    pub server_name: String,
    /// Exact canonical JSON object for the local `humata` MCP entry.
    pub entry: JsonValue,
}

impl ProtectedHumataMcp {
    pub fn new(command: impl Into<String>, args: Vec<String>) -> Self {
        let mut obj = JsonMap::new();
        obj.insert("command".to_string(), JsonValue::String(command.into()));
        obj.insert(
            "args".to_string(),
            JsonValue::Array(args.into_iter().map(JsonValue::String).collect()),
        );
        Self {
            server_name: "humata".to_string(),
            entry: JsonValue::Object(obj),
        }
    }
}

/// Errors produced by individual rewrite stages or post-transform validation.
#[derive(Debug, Error)]
pub enum RewriteError {
    #[error("invalid declarative TOML rules: {0}")]
    InvalidTomlRules(#[from] toml::de::Error),

    #[error("scheme syntax error: {0}")]
    SchemeSyntax(String),

    #[error("scheme evaluation error: {0}")]
    SchemeEval(String),

    #[error("scheme step budget exceeded (max {max_steps} steps)")]
    StepBudgetExceeded { max_steps: usize },

    #[error("scheme recursion limit exceeded (max depth {max_depth})")]
    RecursionLimitExceeded { max_depth: usize },

    #[error("invalid bash filter script name '{0}': must be a plain '*.sh' filename")]
    InvalidScriptName(String),

    #[error("bash filter script not found: {0}")]
    ScriptNotFound(PathBuf),

    #[error("failed to spawn bash filter: {0}")]
    BashSpawn(std::io::Error),

    #[error("bash filter timed out after {0:?}")]
    BashTimeout(Duration),

    #[error("bash filter exited with code {code:?}: {stderr}")]
    BashNonZeroExit { code: Option<i32>, stderr: String },

    #[error("bash filter produced malformed JSON output: {0}")]
    BashInvalidOutput(String),

    #[error("post-transform schema validation failed: {0}")]
    InvalidSchema(String),
}

/// Non-fatal or recovered invariant violations recorded during a pipeline run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RewriteViolation {
    /// A transform stage attempted to delete or alter the protected `humata` MCP entry;
    /// the validator restored the protected entry.
    ProtectedHumataMcpTampered { reason: String },
    /// A transform stage failed and the pipeline fell back to the last valid config.
    StageFailedRecovered { stage: String, error: String },
}

/// Result of running the full rewrite pipeline with post-transform enforcement.
#[derive(Debug, Clone)]
pub struct PipelineReport {
    pub output: JsonValue,
    pub violations: Vec<RewriteViolation>,
    pub stage1_applied: bool,
    pub stage2_applied: bool,
    pub stage3_applied: bool,
}

// ============================================================================
// Stage 1: Declarative TOML Rules
// ============================================================================

/// Prefix rewrite applied to string values in config ASTs on matching machines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathRewriteRule {
    #[serde(default)]
    pub machines: Vec<String>,
    pub from_prefix: String,
    pub to_prefix: String,
}

/// Per-MCP-server declarative override rule.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServerRule {
    #[serde(default)]
    pub machines: Vec<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Option<Vec<String>>,
    #[serde(default)]
    pub pass_env: Vec<String>,
}

/// Declarative TOML rules for Stage 1.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeclarativeRules {
    pub path_rewrites: Vec<PathRewriteRule>,
    pub mcp_rules: BTreeMap<String, McpServerRule>,
}

impl DeclarativeRules {
    /// Parses Stage 1 rules from `config-sync.toml`.
    ///
    /// Supports both `[[path_rewrite]]` / `[[path_rewrites]]` and
    /// `[mcp.servers."<name>"]` / `[mcp_rules."<name>"]`.
    pub fn from_toml_str(input: &str) -> Result<Self, RewriteError> {
        #[derive(Deserialize, Default)]
        struct RawMcpSection {
            #[serde(default)]
            servers: BTreeMap<String, McpServerRule>,
        }

        #[derive(Deserialize, Default)]
        struct RawRules {
            #[serde(default, rename = "path_rewrite", alias = "path_rewrites")]
            path_rewrites: Vec<PathRewriteRule>,
            #[serde(default)]
            mcp: RawMcpSection,
            #[serde(default)]
            mcp_rules: BTreeMap<String, McpServerRule>,
        }

        let raw: RawRules = toml::from_str(input)?;
        let mut mcp_rules = raw.mcp.servers;
        for (name, rule) in raw.mcp_rules {
            mcp_rules.insert(name, rule);
        }

        Ok(Self {
            path_rewrites: raw.path_rewrites,
            mcp_rules,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.path_rewrites.is_empty() && self.mcp_rules.is_empty()
    }

    /// Applies Stage 1 rules to `input` for `ctx`.
    pub fn apply(&self, input: &JsonValue, ctx: &RewriteContext) -> JsonValue {
        let mut current = input.clone();

        // 1. Apply matching path prefix rewrites recursively.
        for rule in &self.path_rewrites {
            if machines_match(&rule.machines, &ctx.machine_name) {
                current = rewrite_json_prefix(&current, &rule.from_prefix, &rule.to_prefix);
            }
        }

        // 2. Apply matching MCP server rules.
        if !self.mcp_rules.is_empty() {
            let mcp_key = ctx.agent.spec().mcp_root_key;
            if let Some(obj) = current.as_object_mut() {
                let target_key = if obj.contains_key(mcp_key) {
                    Some(mcp_key.to_string())
                } else if obj.contains_key("mcpServers") {
                    Some("mcpServers".to_string())
                } else if obj.contains_key("mcp_servers") {
                    Some("mcp_servers".to_string())
                } else {
                    None
                };

                if let Some(key) = target_key {
                    if let Some(servers) = obj.get_mut(&key).and_then(JsonValue::as_object_mut) {
                        for (server_name, rule) in &self.mcp_rules {
                            if !machines_match(&rule.machines, &ctx.machine_name) {
                                continue;
                            }
                            if rule.enabled == Some(false) {
                                servers.remove(server_name);
                                continue;
                            }
                            if let Some(entry) = servers
                                .get_mut(server_name)
                                .and_then(JsonValue::as_object_mut)
                            {
                                if let Some(cmd) = &rule.command {
                                    entry.insert(
                                        "command".to_string(),
                                        JsonValue::String(cmd.clone()),
                                    );
                                }
                                if let Some(args) = &rule.args {
                                    entry.insert(
                                        "args".to_string(),
                                        JsonValue::Array(
                                            args.iter().cloned().map(JsonValue::String).collect(),
                                        ),
                                    );
                                }
                                if !rule.pass_env.is_empty() {
                                    let env_obj = entry
                                        .entry("env".to_string())
                                        .or_insert_with(|| JsonValue::Object(JsonMap::new()));
                                    if let Some(env_map) = env_obj.as_object_mut() {
                                        for var_name in &rule.pass_env {
                                            if let Some(val) = ctx.resolve_env(var_name) {
                                                env_map.insert(
                                                    var_name.clone(),
                                                    JsonValue::String(val),
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        current
    }
}

fn machines_match(patterns: &[String], machine_name: &str) -> bool {
    if patterns.is_empty() {
        return true;
    }
    patterns.iter().any(|p| glob_match(p, machine_name))
}

fn rewrite_json_prefix(value: &JsonValue, from: &str, to: &str) -> JsonValue {
    match value {
        JsonValue::String(s) => {
            if let Some(rest) = s.strip_prefix(from) {
                JsonValue::String(format!("{to}{rest}"))
            } else {
                JsonValue::String(s.clone())
            }
        }
        JsonValue::Array(items) => JsonValue::Array(
            items
                .iter()
                .map(|v| rewrite_json_prefix(v, from, to))
                .collect(),
        ),
        JsonValue::Object(map) => {
            let mut out = JsonMap::new();
            for (k, v) in map {
                out.insert(k.clone(), rewrite_json_prefix(v, from, to));
            }
            JsonValue::Object(out)
        }
        other => other.clone(),
    }
}

// ============================================================================
// Stage 2: Capability-Free Pure-Rust Scheme / S-Expr AST Transformer
// ============================================================================

/// Configuration for Stage 2 Scheme AST transformation.
#[derive(Debug, Clone)]
pub struct SchemeScriptConfig {
    pub source: String,
    pub max_steps: usize,
    pub max_depth: usize,
}

impl SchemeScriptConfig {
    pub fn new(source: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            max_steps: 20_000,
            max_depth: 128,
        }
    }

    pub fn with_step_budget(mut self, max_steps: usize) -> Self {
        self.max_steps = max_steps;
        self
    }

    pub fn with_max_depth(mut self, max_depth: usize) -> Self {
        self.max_depth = max_depth;
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
enum SExpr {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Symbol(String),
    List(Vec<SExpr>),
}

#[derive(Debug, Clone, PartialEq)]
enum SchemeVal {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Symbol(String),
    List(Vec<SchemeVal>),
    Map(BTreeMap<String, SchemeVal>),
    Closure {
        params: Vec<String>,
        body: Vec<SExpr>,
        captured: Vec<BTreeMap<String, SchemeVal>>,
    },
}

/// Evaluates a capability-free Scheme script against `(ctx)` and returns the
/// transformed JSON AST.
pub fn run_scheme_transform(
    cfg: &SchemeScriptConfig,
    input: &JsonValue,
    ctx: &RewriteContext,
) -> Result<JsonValue, RewriteError> {
    let exprs = parse_sexprs(&cfg.source)?;
    if exprs.is_empty() {
        return Err(RewriteError::SchemeSyntax(
            "empty scheme script".to_string(),
        ));
    }

    let mut vm = SchemeVm::new(cfg.max_steps, cfg.max_depth);

    // Build immutable context map `ctx`.
    let mut ctx_map = BTreeMap::new();
    ctx_map.insert(
        "machine-id".to_string(),
        SchemeVal::Str(ctx.machine_id.clone()),
    );
    ctx_map.insert(
        "machine-name".to_string(),
        SchemeVal::Str(ctx.machine_name.clone()),
    );
    ctx_map.insert("os".to_string(), SchemeVal::Str(ctx.os.clone()));
    ctx_map.insert("arch".to_string(), SchemeVal::Str(ctx.arch.clone()));
    ctx_map.insert(
        "home-dir".to_string(),
        SchemeVal::Str(ctx.home_dir.to_string_lossy().into_owned()),
    );
    ctx_map.insert(
        "agent".to_string(),
        SchemeVal::Str(ctx.agent.as_str().to_string()),
    );
    ctx_map.insert("config".to_string(), json_to_scheme(input));
    let ctx_val = SchemeVal::Map(ctx_map);

    // Also bind `ctx` globally so one-liner scripts can reference it directly.
    vm.globals.insert("ctx".to_string(), ctx_val.clone());

    let mut last = SchemeVal::Null;
    for expr in &exprs {
        last = vm.eval(expr, &mut Vec::new(), 0)?;
    }

    // If the script defined `(define (transform ctx) ...)`, invoke it.
    let result_val = if let Some(transform_fn) = vm.globals.get("transform").cloned() {
        vm.apply_val(transform_fn, vec![ctx_val], 0)?
    } else {
        last
    };

    scheme_to_json(&result_val)
}

fn json_to_scheme(v: &JsonValue) -> SchemeVal {
    match v {
        JsonValue::Null => SchemeVal::Null,
        JsonValue::Bool(b) => SchemeVal::Bool(*b),
        JsonValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                SchemeVal::Int(i)
            } else if let Some(f) = n.as_f64() {
                SchemeVal::Float(f)
            } else {
                SchemeVal::Null
            }
        }
        JsonValue::String(s) => SchemeVal::Str(s.clone()),
        JsonValue::Array(arr) => SchemeVal::List(arr.iter().map(json_to_scheme).collect()),
        JsonValue::Object(map) => {
            let mut out = BTreeMap::new();
            for (k, val) in map {
                out.insert(k.clone(), json_to_scheme(val));
            }
            SchemeVal::Map(out)
        }
    }
}

fn scheme_to_json(v: &SchemeVal) -> Result<JsonValue, RewriteError> {
    match v {
        SchemeVal::Null => Ok(JsonValue::Null),
        SchemeVal::Bool(b) => Ok(JsonValue::Bool(*b)),
        SchemeVal::Int(i) => Ok(JsonValue::Number((*i).into())),
        SchemeVal::Float(f) => serde_json::Number::from_f64(*f)
            .map(JsonValue::Number)
            .ok_or_else(|| RewriteError::SchemeEval("non-finite float in output".to_string())),
        SchemeVal::Str(s) | SchemeVal::Symbol(s) => Ok(JsonValue::String(s.clone())),
        SchemeVal::List(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(scheme_to_json(item)?);
            }
            Ok(JsonValue::Array(out))
        }
        SchemeVal::Map(map) => {
            let mut out = JsonMap::new();
            for (k, val) in map {
                out.insert(k.clone(), scheme_to_json(val)?);
            }
            Ok(JsonValue::Object(out))
        }
        SchemeVal::Closure { .. } => Err(RewriteError::SchemeEval(
            "cannot serialize closure as JSON config".to_string(),
        )),
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    LParen,
    RParen,
    Quote,
    Str(String),
    Atom(String),
}

fn tokenize(input: &str) -> Result<Vec<Token>, RewriteError> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == ';' {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '(' {
            tokens.push(Token::LParen);
            i += 1;
        } else if c == ')' {
            tokens.push(Token::RParen);
            i += 1;
        } else if c == '\'' {
            tokens.push(Token::Quote);
            i += 1;
        } else if c == '"' {
            i += 1;
            let mut s = String::new();
            let mut closed = false;
            while i < chars.len() {
                let ch = chars[i];
                if ch == '"' {
                    closed = true;
                    i += 1;
                    break;
                }
                if ch == '\\' {
                    i += 1;
                    if i >= chars.len() {
                        return Err(RewriteError::SchemeSyntax(
                            "unterminated escape sequence".to_string(),
                        ));
                    }
                    match chars[i] {
                        'n' => s.push('\n'),
                        't' => s.push('\t'),
                        'r' => s.push('\r'),
                        '"' => s.push('"'),
                        '\\' => s.push('\\'),
                        other => s.push(other),
                    }
                    i += 1;
                } else {
                    s.push(ch);
                    i += 1;
                }
            }
            if !closed {
                return Err(RewriteError::SchemeSyntax(
                    "unterminated string literal".to_string(),
                ));
            }
            tokens.push(Token::Str(s));
        } else {
            let start = i;
            while i < chars.len()
                && !chars[i].is_whitespace()
                && !matches!(chars[i], '(' | ')' | ';' | '"' | '\'')
            {
                i += 1;
            }
            let atom: String = chars[start..i].iter().collect();
            tokens.push(Token::Atom(atom));
        }
    }

    Ok(tokens)
}

fn parse_sexprs(input: &str) -> Result<Vec<SExpr>, RewriteError> {
    let tokens = tokenize(input)?;
    let mut pos = 0;
    let mut out = Vec::new();
    while pos < tokens.len() {
        out.push(parse_one(&tokens, &mut pos, 0)?);
    }
    Ok(out)
}

fn parse_one(tokens: &[Token], pos: &mut usize, depth: usize) -> Result<SExpr, RewriteError> {
    if depth > 256 {
        return Err(RewriteError::SchemeSyntax(
            "nesting depth exceeded during parse".to_string(),
        ));
    }
    let tok = tokens
        .get(*pos)
        .ok_or_else(|| RewriteError::SchemeSyntax("unexpected end of input".to_string()))?;
    match tok {
        Token::LParen => {
            *pos += 1;
            let mut items = Vec::new();
            while *pos < tokens.len() && tokens[*pos] != Token::RParen {
                items.push(parse_one(tokens, pos, depth + 1)?);
            }
            if *pos >= tokens.len() {
                return Err(RewriteError::SchemeSyntax(
                    "unmatched '(' in scheme script".to_string(),
                ));
            }
            *pos += 1; // consume ')'
            Ok(SExpr::List(items))
        }
        Token::RParen => Err(RewriteError::SchemeSyntax(
            "unexpected ')' in scheme script".to_string(),
        )),
        Token::Quote => {
            *pos += 1;
            let quoted = parse_one(tokens, pos, depth + 1)?;
            Ok(SExpr::List(vec![
                SExpr::Symbol("quote".to_string()),
                quoted,
            ]))
        }
        Token::Str(s) => {
            *pos += 1;
            Ok(SExpr::Str(s.clone()))
        }
        Token::Atom(a) => {
            *pos += 1;
            if a == "#t" || a == "true" {
                Ok(SExpr::Bool(true))
            } else if a == "#f" || a == "false" {
                Ok(SExpr::Bool(false))
            } else if a == "null" || a == "nil" {
                Ok(SExpr::Null)
            } else if let Ok(i) = a.parse::<i64>() {
                Ok(SExpr::Int(i))
            } else if let Ok(f) = a.parse::<f64>() {
                Ok(SExpr::Float(f))
            } else {
                Ok(SExpr::Symbol(a.clone()))
            }
        }
    }
}

struct SchemeVm {
    globals: BTreeMap<String, SchemeVal>,
    steps: usize,
    max_steps: usize,
    max_depth: usize,
}

impl SchemeVm {
    fn new(max_steps: usize, max_depth: usize) -> Self {
        Self {
            globals: BTreeMap::new(),
            steps: 0,
            max_steps,
            max_depth,
        }
    }

    fn tick(&mut self, depth: usize) -> Result<(), RewriteError> {
        self.steps += 1;
        if self.steps > self.max_steps {
            return Err(RewriteError::StepBudgetExceeded {
                max_steps: self.max_steps,
            });
        }
        if depth > self.max_depth {
            return Err(RewriteError::RecursionLimitExceeded {
                max_depth: self.max_depth,
            });
        }
        Ok(())
    }

    fn lookup(&self, name: &str, locals: &[BTreeMap<String, SchemeVal>]) -> Option<SchemeVal> {
        for frame in locals.iter().rev() {
            if let Some(v) = frame.get(name) {
                return Some(v.clone());
            }
        }
        self.globals.get(name).cloned()
    }

    fn eval(
        &mut self,
        expr: &SExpr,
        locals: &mut Vec<BTreeMap<String, SchemeVal>>,
        depth: usize,
    ) -> Result<SchemeVal, RewriteError> {
        self.tick(depth)?;
        match expr {
            SExpr::Null => Ok(SchemeVal::Null),
            SExpr::Bool(b) => Ok(SchemeVal::Bool(*b)),
            SExpr::Int(i) => Ok(SchemeVal::Int(*i)),
            SExpr::Float(f) => Ok(SchemeVal::Float(*f)),
            SExpr::Str(s) => Ok(SchemeVal::Str(s.clone())),
            SExpr::Symbol(sym) => self
                .lookup(sym, locals)
                .ok_or_else(|| RewriteError::SchemeEval(format!("unbound symbol '{sym}'"))),
            SExpr::List(items) => {
                if items.is_empty() {
                    return Ok(SchemeVal::List(Vec::new()));
                }
                if let SExpr::Symbol(head) = &items[0] {
                    match head.as_str() {
                        "quote" => {
                            if items.len() != 2 {
                                return Err(RewriteError::SchemeEval(
                                    "quote expects 1 argument".to_string(),
                                ));
                            }
                            return Ok(sexpr_to_quoted_val(&items[1]));
                        }
                        "if" => {
                            if items.len() < 3 || items.len() > 4 {
                                return Err(RewriteError::SchemeEval(
                                    "if expects 2 or 3 arguments".to_string(),
                                ));
                            }
                            let cond = self.eval(&items[1], locals, depth + 1)?;
                            if is_truthy(&cond) {
                                return self.eval(&items[2], locals, depth + 1);
                            } else if items.len() == 4 {
                                return self.eval(&items[3], locals, depth + 1);
                            } else {
                                return Ok(SchemeVal::Null);
                            }
                        }
                        "cond" => {
                            for clause in &items[1..] {
                                let SExpr::List(parts) = clause else {
                                    return Err(RewriteError::SchemeEval(
                                        "cond clause must be a list".to_string(),
                                    ));
                                };
                                if parts.len() < 2 {
                                    return Err(RewriteError::SchemeEval(
                                        "cond clause needs test and body".to_string(),
                                    ));
                                }
                                let matched = matches!(&parts[0], SExpr::Symbol(s) if s == "else")
                                    || is_truthy(&self.eval(&parts[0], locals, depth + 1)?);
                                if matched {
                                    let mut res = SchemeVal::Null;
                                    for b in &parts[1..] {
                                        res = self.eval(b, locals, depth + 1)?;
                                    }
                                    return Ok(res);
                                }
                            }
                            return Ok(SchemeVal::Null);
                        }
                        "and" => {
                            let mut last = SchemeVal::Bool(true);
                            for arg in &items[1..] {
                                last = self.eval(arg, locals, depth + 1)?;
                                if !is_truthy(&last) {
                                    return Ok(SchemeVal::Bool(false));
                                }
                            }
                            return Ok(last);
                        }
                        "or" => {
                            for arg in &items[1..] {
                                let v = self.eval(arg, locals, depth + 1)?;
                                if is_truthy(&v) {
                                    return Ok(v);
                                }
                            }
                            return Ok(SchemeVal::Bool(false));
                        }
                        "begin" => {
                            let mut last = SchemeVal::Null;
                            for arg in &items[1..] {
                                last = self.eval(arg, locals, depth + 1)?;
                            }
                            return Ok(last);
                        }
                        "let" | "let*" => {
                            if items.len() < 3 {
                                return Err(RewriteError::SchemeEval(format!(
                                    "{head} expects bindings and body"
                                )));
                            }
                            let SExpr::List(bindings) = &items[1] else {
                                return Err(RewriteError::SchemeEval(format!(
                                    "{head} bindings must be a list"
                                )));
                            };
                            let sequential = head == "let*";
                            let mut frame = BTreeMap::new();
                            if sequential {
                                locals.push(BTreeMap::new());
                                for b in bindings {
                                    let (k, expr) = parse_binding(b)?;
                                    let val = self.eval(expr, locals, depth + 1)?;
                                    if let Some(top) = locals.last_mut() {
                                        top.insert(k, val);
                                    }
                                }
                            } else {
                                for b in bindings {
                                    let (k, expr) = parse_binding(b)?;
                                    let val = self.eval(expr, locals, depth + 1)?;
                                    frame.insert(k, val);
                                }
                                locals.push(frame);
                            }
                            let mut last = SchemeVal::Null;
                            for body_expr in &items[2..] {
                                last = self.eval(body_expr, locals, depth + 1)?;
                            }
                            locals.pop();
                            return Ok(last);
                        }
                        "lambda" => {
                            if items.len() < 3 {
                                return Err(RewriteError::SchemeEval(
                                    "lambda expects params and body".to_string(),
                                ));
                            }
                            let params = parse_param_list(&items[1])?;
                            return Ok(SchemeVal::Closure {
                                params,
                                body: items[2..].to_vec(),
                                captured: locals.clone(),
                            });
                        }
                        "define" => {
                            if items.len() < 3 {
                                return Err(RewriteError::SchemeEval(
                                    "define expects name and value".to_string(),
                                ));
                            }
                            match &items[1] {
                                SExpr::Symbol(name) => {
                                    let val = self.eval(&items[2], locals, depth + 1)?;
                                    if let Some(top) = locals.last_mut() {
                                        top.insert(name.clone(), val.clone());
                                    } else {
                                        self.globals.insert(name.clone(), val.clone());
                                    }
                                    return Ok(val);
                                }
                                SExpr::List(sig) if !sig.is_empty() => {
                                    let SExpr::Symbol(fn_name) = &sig[0] else {
                                        return Err(RewriteError::SchemeEval(
                                            "define function name must be a symbol".to_string(),
                                        ));
                                    };
                                    let mut params = Vec::with_capacity(sig.len() - 1);
                                    for p in &sig[1..] {
                                        let SExpr::Symbol(pname) = p else {
                                            return Err(RewriteError::SchemeEval(
                                                "parameter name must be a symbol".to_string(),
                                            ));
                                        };
                                        params.push(pname.clone());
                                    }
                                    let closure = SchemeVal::Closure {
                                        params,
                                        body: items[2..].to_vec(),
                                        captured: locals.clone(),
                                    };
                                    if let Some(top) = locals.last_mut() {
                                        top.insert(fn_name.clone(), closure.clone());
                                    } else {
                                        self.globals.insert(fn_name.clone(), closure.clone());
                                    }
                                    return Ok(closure);
                                }
                                _ => {
                                    return Err(RewriteError::SchemeEval(
                                        "invalid define target".to_string(),
                                    ));
                                }
                            }
                        }
                        _ => {
                            // Check if it's a built-in primitive before looking up variable.
                            if is_builtin(head) {
                                let mut args = Vec::with_capacity(items.len() - 1);
                                for arg in &items[1..] {
                                    args.push(self.eval(arg, locals, depth + 1)?);
                                }
                                return self.call_builtin(head, args, depth + 1);
                            }
                        }
                    }
                }

                // General function application.
                let callee = self.eval(&items[0], locals, depth + 1)?;
                let mut args = Vec::with_capacity(items.len() - 1);
                for arg in &items[1..] {
                    args.push(self.eval(arg, locals, depth + 1)?);
                }
                self.apply_val(callee, args, depth + 1)
            }
        }
    }

    fn apply_val(
        &mut self,
        callee: SchemeVal,
        args: Vec<SchemeVal>,
        depth: usize,
    ) -> Result<SchemeVal, RewriteError> {
        self.tick(depth)?;
        match callee {
            SchemeVal::Closure {
                params,
                body,
                captured,
            } => {
                if params.len() != args.len() {
                    return Err(RewriteError::SchemeEval(format!(
                        "arity mismatch: expected {} args, got {}",
                        params.len(),
                        args.len()
                    )));
                }
                let mut call_locals = captured;
                let mut frame = BTreeMap::new();
                for (p, a) in params.into_iter().zip(args.into_iter()) {
                    frame.insert(p, a);
                }
                call_locals.push(frame);
                let mut last = SchemeVal::Null;
                for expr in &body {
                    last = self.eval(expr, &mut call_locals, depth + 1)?;
                }
                Ok(last)
            }
            other => Err(RewriteError::SchemeEval(format!(
                "attempted to call non-function value: {other:?}"
            ))),
        }
    }

    fn call_builtin(
        &mut self,
        name: &str,
        args: Vec<SchemeVal>,
        depth: usize,
    ) -> Result<SchemeVal, RewriteError> {
        self.tick(depth)?;
        match name {
            "equal?" | "=" => {
                expect_arity(name, &args, 2)?;
                Ok(SchemeVal::Bool(args[0] == args[1]))
            }
            "not" => {
                expect_arity(name, &args, 1)?;
                Ok(SchemeVal::Bool(!is_truthy(&args[0])))
            }
            "null?" => {
                expect_arity(name, &args, 1)?;
                Ok(SchemeVal::Bool(matches!(args[0], SchemeVal::Null)))
            }
            "string?" => {
                expect_arity(name, &args, 1)?;
                Ok(SchemeVal::Bool(matches!(args[0], SchemeVal::Str(_))))
            }
            "map?" => {
                expect_arity(name, &args, 1)?;
                Ok(SchemeVal::Bool(matches!(args[0], SchemeVal::Map(_))))
            }
            "list?" => {
                expect_arity(name, &args, 1)?;
                Ok(SchemeVal::Bool(matches!(args[0], SchemeVal::List(_))))
            }
            "bool?" => {
                expect_arity(name, &args, 1)?;
                Ok(SchemeVal::Bool(matches!(args[0], SchemeVal::Bool(_))))
            }
            "number?" => {
                expect_arity(name, &args, 1)?;
                Ok(SchemeVal::Bool(matches!(
                    args[0],
                    SchemeVal::Int(_) | SchemeVal::Float(_)
                )))
            }
            "+" => {
                let mut acc = 0i64;
                for a in args {
                    acc = acc.saturating_add(as_int(&a, name)?);
                }
                Ok(SchemeVal::Int(acc))
            }
            "-" => {
                if args.is_empty() {
                    return Err(RewriteError::SchemeEval("- expects at least 1 arg".into()));
                }
                let first = as_int(&args[0], name)?;
                if args.len() == 1 {
                    return Ok(SchemeVal::Int(-first));
                }
                let mut acc = first;
                for a in &args[1..] {
                    acc = acc.saturating_sub(as_int(a, name)?);
                }
                Ok(SchemeVal::Int(acc))
            }
            "*" => {
                let mut acc = 1i64;
                for a in args {
                    acc = acc.saturating_mul(as_int(&a, name)?);
                }
                Ok(SchemeVal::Int(acc))
            }
            "<" | ">" | "<=" | ">=" => {
                expect_arity(name, &args, 2)?;
                let a = as_int(&args[0], name)?;
                let b = as_int(&args[1], name)?;
                let res = match name {
                    "<" => a < b,
                    ">" => a > b,
                    "<=" => a <= b,
                    ">=" => a >= b,
                    _ => unreachable!(),
                };
                Ok(SchemeVal::Bool(res))
            }
            "string-append" => {
                let mut out = String::new();
                for a in &args {
                    out.push_str(as_str_or_sym(a, name)?);
                }
                Ok(SchemeVal::Str(out))
            }
            "string-prefix?" => {
                expect_arity(name, &args, 2)?;
                let s = as_str_or_sym(&args[0], name)?;
                let prefix = as_str_or_sym(&args[1], name)?;
                Ok(SchemeVal::Bool(s.starts_with(prefix)))
            }
            "string-contains?" => {
                expect_arity(name, &args, 2)?;
                let s = as_str_or_sym(&args[0], name)?;
                let needle = as_str_or_sym(&args[1], name)?;
                Ok(SchemeVal::Bool(s.contains(needle)))
            }
            "string-replace" => {
                expect_arity(name, &args, 3)?;
                let s = as_str_or_sym(&args[0], name)?;
                let from = as_str_or_sym(&args[1], name)?;
                let to = as_str_or_sym(&args[2], name)?;
                Ok(SchemeVal::Str(s.replace(from, to)))
            }
            "string-length" => {
                expect_arity(name, &args, 1)?;
                let s = as_str_or_sym(&args[0], name)?;
                Ok(SchemeVal::Int(s.chars().count() as i64))
            }
            "hash" => {
                if args.len() % 2 != 0 {
                    return Err(RewriteError::SchemeEval(
                        "hash expects an even number of key/value args".to_string(),
                    ));
                }
                let mut map = BTreeMap::new();
                let mut iter = args.into_iter();
                while let (Some(k), Some(v)) = (iter.next(), iter.next()) {
                    let ks = as_str_or_sym(&k, name)?.to_string();
                    map.insert(ks, v);
                }
                Ok(SchemeVal::Map(map))
            }
            "hash-get" => {
                if args.len() < 2 || args.len() > 3 {
                    return Err(RewriteError::SchemeEval(
                        "hash-get expects 2 or 3 args".to_string(),
                    ));
                }
                let SchemeVal::Map(map) = &args[0] else {
                    return Err(RewriteError::SchemeEval(
                        "hash-get first arg must be a map".to_string(),
                    ));
                };
                let key = as_str_or_sym(&args[1], name)?;
                Ok(map
                    .get(key)
                    .cloned()
                    .unwrap_or_else(|| args.get(2).cloned().unwrap_or(SchemeVal::Null)))
            }
            "hash-set" => {
                expect_arity(name, &args, 3)?;
                let SchemeVal::Map(mut map) = args[0].clone() else {
                    return Err(RewriteError::SchemeEval(
                        "hash-set first arg must be a map".to_string(),
                    ));
                };
                let key = as_str_or_sym(&args[1], name)?.to_string();
                map.insert(key, args[2].clone());
                Ok(SchemeVal::Map(map))
            }
            "hash-remove" => {
                expect_arity(name, &args, 2)?;
                let SchemeVal::Map(mut map) = args[0].clone() else {
                    return Err(RewriteError::SchemeEval(
                        "hash-remove first arg must be a map".to_string(),
                    ));
                };
                let key = as_str_or_sym(&args[1], name)?;
                map.remove(key);
                Ok(SchemeVal::Map(map))
            }
            "hash-has?" => {
                expect_arity(name, &args, 2)?;
                let SchemeVal::Map(map) = &args[0] else {
                    return Err(RewriteError::SchemeEval(
                        "hash-has? first arg must be a map".to_string(),
                    ));
                };
                let key = as_str_or_sym(&args[1], name)?;
                Ok(SchemeVal::Bool(map.contains_key(key)))
            }
            "hash-keys" => {
                expect_arity(name, &args, 1)?;
                let SchemeVal::Map(map) = &args[0] else {
                    return Err(RewriteError::SchemeEval(
                        "hash-keys expects a map".to_string(),
                    ));
                };
                Ok(SchemeVal::List(
                    map.keys().cloned().map(SchemeVal::Str).collect(),
                ))
            }
            "hash-merge" => {
                expect_arity(name, &args, 2)?;
                let SchemeVal::Map(mut left) = args[0].clone() else {
                    return Err(RewriteError::SchemeEval(
                        "hash-merge expects two maps".to_string(),
                    ));
                };
                let SchemeVal::Map(right) = args[1].clone() else {
                    return Err(RewriteError::SchemeEval(
                        "hash-merge expects two maps".to_string(),
                    ));
                };
                for (k, v) in right {
                    left.insert(k, v);
                }
                Ok(SchemeVal::Map(left))
            }
            "list" => Ok(SchemeVal::List(args)),
            "empty?" => {
                expect_arity(name, &args, 1)?;
                match &args[0] {
                    SchemeVal::List(l) => Ok(SchemeVal::Bool(l.is_empty())),
                    SchemeVal::Map(m) => Ok(SchemeVal::Bool(m.is_empty())),
                    SchemeVal::Str(s) => Ok(SchemeVal::Bool(s.is_empty())),
                    _ => Ok(SchemeVal::Bool(false)),
                }
            }
            "len" => {
                expect_arity(name, &args, 1)?;
                match &args[0] {
                    SchemeVal::List(l) => Ok(SchemeVal::Int(l.len() as i64)),
                    SchemeVal::Map(m) => Ok(SchemeVal::Int(m.len() as i64)),
                    SchemeVal::Str(s) => Ok(SchemeVal::Int(s.chars().count() as i64)),
                    _ => Err(RewriteError::SchemeEval(
                        "len expects list, map, or string".to_string(),
                    )),
                }
            }
            "car" => {
                expect_arity(name, &args, 1)?;
                let SchemeVal::List(l) = &args[0] else {
                    return Err(RewriteError::SchemeEval("car expects a list".to_string()));
                };
                l.first()
                    .cloned()
                    .ok_or_else(|| RewriteError::SchemeEval("car on empty list".to_string()))
            }
            "cdr" => {
                expect_arity(name, &args, 1)?;
                let SchemeVal::List(l) = &args[0] else {
                    return Err(RewriteError::SchemeEval("cdr expects a list".to_string()));
                };
                if l.is_empty() {
                    Ok(SchemeVal::List(Vec::new()))
                } else {
                    Ok(SchemeVal::List(l[1..].to_vec()))
                }
            }
            "cons" => {
                expect_arity(name, &args, 2)?;
                let SchemeVal::List(mut tail) = args[1].clone() else {
                    return Err(RewriteError::SchemeEval(
                        "cons second arg must be a list".to_string(),
                    ));
                };
                tail.insert(0, args[0].clone());
                Ok(SchemeVal::List(tail))
            }
            "append" => {
                let mut out = Vec::new();
                for a in args {
                    let SchemeVal::List(l) = a else {
                        return Err(RewriteError::SchemeEval(
                            "append expects list arguments".to_string(),
                        ));
                    };
                    out.extend(l);
                }
                Ok(SchemeVal::List(out))
            }
            "map" => {
                expect_arity(name, &args, 2)?;
                let f = args[0].clone();
                let SchemeVal::List(items) = args[1].clone() else {
                    return Err(RewriteError::SchemeEval(
                        "map second arg must be a list".to_string(),
                    ));
                };
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.push(self.apply_val(f.clone(), vec![item], depth + 1)?);
                }
                Ok(SchemeVal::List(out))
            }
            "filter" => {
                expect_arity(name, &args, 2)?;
                let pred = args[0].clone();
                let SchemeVal::List(items) = args[1].clone() else {
                    return Err(RewriteError::SchemeEval(
                        "filter second arg must be a list".to_string(),
                    ));
                };
                let mut out = Vec::new();
                for item in items {
                    let keep = self.apply_val(pred.clone(), vec![item.clone()], depth + 1)?;
                    if is_truthy(&keep) {
                        out.push(item);
                    }
                }
                Ok(SchemeVal::List(out))
            }
            "rewrite-prefix" => {
                expect_arity(name, &args, 3)?;
                let from = as_str_or_sym(&args[1], name)?;
                let to = as_str_or_sym(&args[2], name)?;
                Ok(rewrite_scheme_prefix(&args[0], from, to))
            }
            "drop-mcp-server" => {
                expect_arity(name, &args, 2)?;
                let SchemeVal::Map(mut cfg) = args[0].clone() else {
                    return Err(RewriteError::SchemeEval(
                        "drop-mcp-server first arg must be a map".to_string(),
                    ));
                };
                let target = as_str_or_sym(&args[1], name)?;
                for key in ["mcpServers", "mcp_servers"] {
                    if let Some(SchemeVal::Map(servers)) = cfg.get_mut(key) {
                        servers.remove(target);
                    }
                }
                Ok(SchemeVal::Map(cfg))
            }
            "set-mcp-server" => {
                expect_arity(name, &args, 3)?;
                let SchemeVal::Map(mut cfg) = args[0].clone() else {
                    return Err(RewriteError::SchemeEval(
                        "set-mcp-server first arg must be a map".to_string(),
                    ));
                };
                let server_name = as_str_or_sym(&args[1], name)?.to_string();
                let entry = args[2].clone();
                let key = if cfg.contains_key("mcp_servers") {
                    "mcp_servers"
                } else {
                    "mcpServers"
                };
                let servers = cfg
                    .entry(key.to_string())
                    .or_insert_with(|| SchemeVal::Map(BTreeMap::new()));
                if let SchemeVal::Map(map) = servers {
                    map.insert(server_name, entry);
                }
                Ok(SchemeVal::Map(cfg))
            }
            other => Err(RewriteError::SchemeEval(format!(
                "unknown builtin '{other}'"
            ))),
        }
    }
}

fn is_builtin(name: &str) -> bool {
    matches!(
        name,
        "equal?"
            | "="
            | "not"
            | "null?"
            | "string?"
            | "map?"
            | "list?"
            | "bool?"
            | "number?"
            | "+"
            | "-"
            | "*"
            | "<"
            | ">"
            | "<="
            | ">="
            | "string-append"
            | "string-prefix?"
            | "string-contains?"
            | "string-replace"
            | "string-length"
            | "hash"
            | "hash-get"
            | "hash-set"
            | "hash-remove"
            | "hash-has?"
            | "hash-keys"
            | "hash-merge"
            | "list"
            | "empty?"
            | "len"
            | "car"
            | "cdr"
            | "cons"
            | "append"
            | "map"
            | "filter"
            | "rewrite-prefix"
            | "drop-mcp-server"
            | "set-mcp-server"
    )
}

fn rewrite_scheme_prefix(val: &SchemeVal, from: &str, to: &str) -> SchemeVal {
    match val {
        SchemeVal::Str(s) => {
            if let Some(rest) = s.strip_prefix(from) {
                SchemeVal::Str(format!("{to}{rest}"))
            } else {
                SchemeVal::Str(s.clone())
            }
        }
        SchemeVal::List(items) => SchemeVal::List(
            items
                .iter()
                .map(|v| rewrite_scheme_prefix(v, from, to))
                .collect(),
        ),
        SchemeVal::Map(map) => {
            let mut out = BTreeMap::new();
            for (k, v) in map {
                out.insert(k.clone(), rewrite_scheme_prefix(v, from, to));
            }
            SchemeVal::Map(out)
        }
        other => other.clone(),
    }
}

fn sexpr_to_quoted_val(e: &SExpr) -> SchemeVal {
    match e {
        SExpr::Null => SchemeVal::Null,
        SExpr::Bool(b) => SchemeVal::Bool(*b),
        SExpr::Int(i) => SchemeVal::Int(*i),
        SExpr::Float(f) => SchemeVal::Float(*f),
        SExpr::Str(s) => SchemeVal::Str(s.clone()),
        SExpr::Symbol(s) => SchemeVal::Symbol(s.clone()),
        SExpr::List(items) => SchemeVal::List(items.iter().map(sexpr_to_quoted_val).collect()),
    }
}

fn is_truthy(v: &SchemeVal) -> bool {
    !matches!(v, SchemeVal::Bool(false) | SchemeVal::Null)
}

fn expect_arity(name: &str, args: &[SchemeVal], expected: usize) -> Result<(), RewriteError> {
    if args.len() != expected {
        return Err(RewriteError::SchemeEval(format!(
            "{name} expects {expected} args, got {}",
            args.len()
        )));
    }
    Ok(())
}

fn as_int(v: &SchemeVal, ctx: &str) -> Result<i64, RewriteError> {
    match v {
        SchemeVal::Int(i) => Ok(*i),
        _ => Err(RewriteError::SchemeEval(format!("{ctx} expects integer"))),
    }
}

fn as_str_or_sym<'a>(v: &'a SchemeVal, ctx: &str) -> Result<&'a str, RewriteError> {
    match v {
        SchemeVal::Str(s) | SchemeVal::Symbol(s) => Ok(s.as_str()),
        _ => Err(RewriteError::SchemeEval(format!(
            "{ctx} expects string or symbol"
        ))),
    }
}

fn parse_binding(b: &SExpr) -> Result<(String, &SExpr), RewriteError> {
    let SExpr::List(pair) = b else {
        return Err(RewriteError::SchemeEval(
            "let binding must be (name expr)".to_string(),
        ));
    };
    if pair.len() != 2 {
        return Err(RewriteError::SchemeEval(
            "let binding must have 2 elements".to_string(),
        ));
    }
    let SExpr::Symbol(name) = &pair[0] else {
        return Err(RewriteError::SchemeEval(
            "let binding name must be a symbol".to_string(),
        ));
    };
    Ok((name.clone(), &pair[1]))
}

fn parse_param_list(expr: &SExpr) -> Result<Vec<String>, RewriteError> {
    let SExpr::List(list) = expr else {
        return Err(RewriteError::SchemeEval(
            "lambda parameters must be a list".to_string(),
        ));
    };
    let mut out = Vec::with_capacity(list.len());
    for p in list {
        let SExpr::Symbol(name) = p else {
            return Err(RewriteError::SchemeEval(
                "lambda parameter must be a symbol".to_string(),
            ));
        };
        out.push(name.clone());
    }
    Ok(out)
}

// ============================================================================
// Stage 3: Target-Local Bash Filter
// ============================================================================

/// Configuration for Stage 3 target-local `bash` filter execution.
#[derive(Debug, Clone)]
pub struct BashFilterConfig {
    /// Directory containing trusted target-local scripts (e.g. `~/.humata2/scripts/`,
    /// or a tempdir in tests).
    pub scripts_dir: PathBuf,
    /// Plain script filename ending in `.sh` (no `/`, `\`, or `..`).
    pub script_name: String,
    /// Bounded execution timeout (default 5 seconds).
    pub timeout: Duration,
}

impl BashFilterConfig {
    pub fn new(scripts_dir: impl Into<PathBuf>, script_name: impl Into<String>) -> Self {
        Self {
            scripts_dir: scripts_dir.into(),
            script_name: script_name.into(),
            timeout: Duration::from_secs(5),
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

/// Validates script name against path traversal (`MACHINESCRIPTS` invariant).
pub fn validate_script_name(name: &str) -> Result<(), RewriteError> {
    if name.is_empty()
        || !name.ends_with(".sh")
        || name.contains('/')
        || name.contains('\\')
        || name.contains("..")
        || name.contains('\0')
    {
        return Err(RewriteError::InvalidScriptName(name.to_string()));
    }
    Ok(())
}

/// Executes a target-local `bash` filter script with JSON on `stdin` and
/// transformed JSON on `stdout`, enforcing a strict wall-clock timeout.
pub fn run_bash_filter(
    cfg: &BashFilterConfig,
    input: &JsonValue,
    ctx: &RewriteContext,
) -> Result<JsonValue, RewriteError> {
    validate_script_name(&cfg.script_name)?;
    let script_path = cfg.scripts_dir.join(&cfg.script_name);
    if !script_path.is_file() {
        return Err(RewriteError::ScriptNotFound(script_path));
    }

    let payload = serde_json::to_vec(input)
        .map_err(|e| RewriteError::BashInvalidOutput(format!("failed to encode stdin: {e}")))?;

    let mut child = Command::new("bash")
        .arg(&script_path)
        .env("HUMATA_MACHINE", &ctx.machine_name)
        .env("HUMATA_MACHINE_ID", &ctx.machine_id)
        .env("HUMATA_OS", &ctx.os)
        .env("HUMATA_ARCH", &ctx.arch)
        .env("HUMATA_AGENT", ctx.agent.as_str())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(RewriteError::BashSpawn)?;

    // Drain stdout/stderr concurrently so pipe buffers never deadlock.
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let stdout_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut p) = stdout_pipe {
            let _ = p.read_to_end(&mut buf);
        }
        buf
    });
    let stderr_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut p) = stderr_pipe {
            let _ = p.read_to_end(&mut buf);
        }
        buf
    });

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(&payload);
        // Dropping stdin closes EOF for the child process.
    }

    let deadline = Instant::now() + cfg.timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = stdout_thread.join();
                    let _ = stderr_thread.join();
                    return Err(RewriteError::BashTimeout(cfg.timeout));
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_thread.join();
                let _ = stderr_thread.join();
                return Err(RewriteError::BashSpawn(e));
            }
        }
    };

    let stdout = stdout_thread.join().unwrap_or_default();
    let stderr = stderr_thread.join().unwrap_or_default();

    if !status.success() {
        return Err(RewriteError::BashNonZeroExit {
            code: status.code(),
            stderr: String::from_utf8_lossy(&stderr).trim().to_string(),
        });
    }

    let parsed: JsonValue = serde_json::from_slice(&stdout)
        .map_err(|e| RewriteError::BashInvalidOutput(e.to_string()))?;

    if !parsed.is_object() {
        return Err(RewriteError::BashInvalidOutput(
            "expected JSON object from bash filter".to_string(),
        ));
    }

    Ok(parsed)
}

// ============================================================================
// Full 3-Stage Pipeline + Post-Transform Invariant Validator
// ============================================================================

/// Orchestrates Stages 1, 2, 3 and post-transform schema + protected-`humata`-MCP validation.
#[derive(Debug, Clone, Default)]
pub struct RewritePipeline {
    pub declarative: DeclarativeRules,
    pub scheme: Option<SchemeScriptConfig>,
    pub bash: Option<BashFilterConfig>,
}

impl RewritePipeline {
    /// Runs all configured stages in fail-safe mode: if any stage errors or
    /// produces invalid schema, the pipeline recovers the last valid config,
    /// records a `RewriteViolation::StageFailedRecovered`, and enforces the
    /// protected `humata` MCP entry.
    pub fn run_safe(
        &self,
        input: &JsonValue,
        ctx: &RewriteContext,
        format: ConfigFormat,
        protected: Option<&ProtectedHumataMcp>,
    ) -> PipelineReport {
        let mut current = input.clone();
        let mut violations = Vec::new();
        let mut stage1_applied = false;
        let mut stage2_applied = false;
        let mut stage3_applied = false;

        // Stage 1: Declarative TOML rules.
        if !self.declarative.is_empty() {
            let candidate = self.declarative.apply(&current, ctx);
            match validate_schema(&candidate, format) {
                Ok(()) => {
                    current = candidate;
                    stage1_applied = true;
                }
                Err(err) => {
                    violations.push(RewriteViolation::StageFailedRecovered {
                        stage: "declarative_toml".to_string(),
                        error: err.to_string(),
                    });
                }
            }
        }

        // Stage 2: Capability-free Scheme AST transform.
        if let Some(scheme_cfg) = &self.scheme {
            match run_scheme_transform(scheme_cfg, &current, ctx)
                .and_then(|cand| validate_schema(&cand, format).map(|()| cand))
            {
                Ok(candidate) => {
                    current = candidate;
                    stage2_applied = true;
                }
                Err(err) => {
                    violations.push(RewriteViolation::StageFailedRecovered {
                        stage: "scheme_ast".to_string(),
                        error: err.to_string(),
                    });
                }
            }
        }

        // Stage 3: Target-local bash filter.
        if let Some(bash_cfg) = &self.bash {
            match run_bash_filter(bash_cfg, &current, ctx)
                .and_then(|cand| validate_schema(&cand, format).map(|()| cand))
            {
                Ok(candidate) => {
                    current = candidate;
                    stage3_applied = true;
                }
                Err(err) => {
                    violations.push(RewriteViolation::StageFailedRecovered {
                        stage: "bash_filter".to_string(),
                        error: err.to_string(),
                    });
                }
            }
        }

        // Post-transform invariant validator: enforce protected `humata` MCP entry.
        if let Some(prot) = protected {
            if let Some(violation) = enforce_protected_humata_mcp(&mut current, ctx.agent, prot) {
                violations.push(violation);
            }
        }

        PipelineReport {
            output: current,
            violations,
            stage1_applied,
            stage2_applied,
            stage3_applied,
        }
    }

    /// Runs all configured stages in strict mode, returning `Err` immediately on
    /// any stage failure or schema error. Still enforces protected `humata` MCP
    /// and records any tampering violation in the returned `PipelineReport`.
    pub fn run_strict(
        &self,
        input: &JsonValue,
        ctx: &RewriteContext,
        format: ConfigFormat,
        protected: Option<&ProtectedHumataMcp>,
    ) -> Result<PipelineReport, RewriteError> {
        let mut current = input.clone();
        let mut violations = Vec::new();
        let mut stage1_applied = false;
        let mut stage2_applied = false;
        let mut stage3_applied = false;

        if !self.declarative.is_empty() {
            current = self.declarative.apply(&current, ctx);
            validate_schema(&current, format)?;
            stage1_applied = true;
        }

        if let Some(scheme_cfg) = &self.scheme {
            current = run_scheme_transform(scheme_cfg, &current, ctx)?;
            validate_schema(&current, format)?;
            stage2_applied = true;
        }

        if let Some(bash_cfg) = &self.bash {
            current = run_bash_filter(bash_cfg, &current, ctx)?;
            validate_schema(&current, format)?;
            stage3_applied = true;
        }

        if let Some(prot) = protected {
            if let Some(violation) = enforce_protected_humata_mcp(&mut current, ctx.agent, prot) {
                violations.push(violation);
            }
        }

        Ok(PipelineReport {
            output: current,
            violations,
            stage1_applied,
            stage2_applied,
            stage3_applied,
        })
    }
}

/// Validates that `value` is a JSON object and, when `format == ConfigFormat::Toml`,
/// can be cleanly represented and serialized as valid TOML.
pub fn validate_schema(value: &JsonValue, format: ConfigFormat) -> Result<(), RewriteError> {
    if !value.is_object() {
        return Err(RewriteError::InvalidSchema(
            "top-level config must be a JSON/TOML object".to_string(),
        ));
    }
    if format == ConfigFormat::Toml {
        let toml_val = json_to_toml(value)?;
        toml::to_string_pretty(&toml_val)
            .map_err(|e| RewriteError::InvalidSchema(format!("invalid TOML schema: {e}")))?;
    }
    Ok(())
}

/// Guarantees that the protected `humata` MCP server entry exists and matches
/// `protected.entry` verbatim. If any transform deleted or hijacked it, restores
/// it immediately and returns a `RewriteViolation::ProtectedHumataMcpTampered`.
pub fn enforce_protected_humata_mcp(
    config: &mut JsonValue,
    agent: AgentKind,
    protected: &ProtectedHumataMcp,
) -> Option<RewriteViolation> {
    let root_key = agent.spec().mcp_root_key;
    let obj = match config.as_object_mut() {
        Some(o) => o,
        None => {
            *config = JsonValue::Object(JsonMap::new());
            config.as_object_mut().expect("just created object")
        }
    };

    let servers_val = obj
        .entry(root_key.to_string())
        .or_insert_with(|| JsonValue::Object(JsonMap::new()));

    if !servers_val.is_object() {
        *servers_val = JsonValue::Object(JsonMap::new());
        let servers = servers_val.as_object_mut().expect("just created object");
        servers.insert(protected.server_name.clone(), protected.entry.clone());
        return Some(RewriteViolation::ProtectedHumataMcpTampered {
            reason: format!("'{root_key}' was replaced with a non-object value"),
        });
    }

    let servers = servers_val.as_object_mut().expect("checked is_object");
    match servers.get(&protected.server_name) {
        Some(existing) if existing == &protected.entry => None,
        Some(_) => {
            servers.insert(protected.server_name.clone(), protected.entry.clone());
            Some(RewriteViolation::ProtectedHumataMcpTampered {
                reason: format!(
                    "protected MCP server '{}' was modified/hijacked; restored canonical entry",
                    protected.server_name
                ),
            })
        }
        None => {
            servers.insert(protected.server_name.clone(), protected.entry.clone());
            Some(RewriteViolation::ProtectedHumataMcpTampered {
                reason: format!(
                    "protected MCP server '{}' was deleted; restored canonical entry",
                    protected.server_name
                ),
            })
        }
    }
}

/// Converts a `serde_json::Value` to `toml::Value` (rejecting nulls, which TOML forbids).
pub fn json_to_toml(value: &JsonValue) -> Result<toml::Value, RewriteError> {
    match value {
        JsonValue::Null => Err(RewriteError::InvalidSchema(
            "TOML does not support null values".to_string(),
        )),
        JsonValue::Bool(b) => Ok(toml::Value::Boolean(*b)),
        JsonValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(toml::Value::Integer(i))
            } else if let Some(f) = n.as_f64() {
                Ok(toml::Value::Float(f))
            } else {
                Err(RewriteError::InvalidSchema(
                    "numeric value out of range for TOML".to_string(),
                ))
            }
        }
        JsonValue::String(s) => Ok(toml::Value::String(s.clone())),
        JsonValue::Array(arr) => {
            let mut out = Vec::with_capacity(arr.len());
            for item in arr {
                out.push(json_to_toml(item)?);
            }
            Ok(toml::Value::Array(out))
        }
        JsonValue::Object(map) => {
            let mut out = toml::map::Map::new();
            for (k, v) in map {
                if !v.is_null() {
                    out.insert(k.clone(), json_to_toml(v)?);
                }
            }
            Ok(toml::Value::Table(out))
        }
    }
}

/// Converts a `toml::Value` to `serde_json::Value`.
pub fn toml_to_json(value: &toml::Value) -> JsonValue {
    match value {
        toml::Value::String(s) => JsonValue::String(s.clone()),
        toml::Value::Integer(i) => JsonValue::Number((*i).into()),
        toml::Value::Float(f) => serde_json::Number::from_f64(*f)
            .map(JsonValue::Number)
            .unwrap_or(JsonValue::Null),
        toml::Value::Boolean(b) => JsonValue::Bool(*b),
        toml::Value::Datetime(dt) => JsonValue::String(dt.to_string()),
        toml::Value::Array(arr) => JsonValue::Array(arr.iter().map(toml_to_json).collect()),
        toml::Value::Table(table) => {
            let mut out = JsonMap::new();
            for (k, v) in table {
                out.insert(k.clone(), toml_to_json(v));
            }
            JsonValue::Object(out)
        }
    }
}
