//! `lore` CLI — collective long-term epistemic memory and live cross-agent
//! awareness (`SPEC.md §5.2`, `SW-4`, `SW-5`).

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use clap::{Args, Parser, Subcommand};
use lore_context::{ContextFrame, EnvHints, detect};
use lore_engine::{
    FeedbackRequest, InspectRequest, PulseRequest, RecallRequest, RememberRequest,
    claim_kind_str, ensure_enabled, export_store_events, format_evidence_envelope, load_config,
    read_only_status, save_config, verifier_status_str, verify_store,
};
use lore_mcp::{
    LocalBackend, LoreBackend, McpServer, handle_cue_hook, handle_prompt_hook, parse_claim_kind,
    parse_feedback_action, resolve_write_scope,
};
use lore_proto::{
    ClaimKind, LoreError, OriginSnapshot, Predicate, PredicateOp, Principal, PrincipalKind, Result,
    SystemClock, log_event,
};
use serde_json::json;

/// SQLite busy timeout (ms) for fail-open hooks (`HOOK-3`).
const HOOK_BUSY_TIMEOUT_MS: u64 = 25;

const GUIDE_TEXT: &str = "\
lore — Collective Epistemic Memory & Live Cross-Agent Awareness

Core workflow:
  lore recall [query] [--about <ns:key>] [--kind <kind>] [--include-history] [--json]
  lore show <id> [--lineage] [--json]
  lore remember --kind <kind> <statement> [--body <text>] [--dead-end <text>] [--about <ns:key>] [--supersedes <id>] [--pin]
  lore feedback <id> <helpful|harmful|not-applicable|corroborate|dispute|invalidate|retract> [--note <text>]
  lore pulse [--intent <text>] [--hypothesis <text>] [--touch <path>] [--fail <err>] [--query-only]

Kinds:
  axiom | fact | decision | solution | dead_end | warning | preference | question | handoff

Admin & maintenance (operable even when memory.enabled = false):
  lore status [--json]
  lore config get memory.enabled
  lore config set memory.enabled <true|false> [--reason <text>]
  lore gc [--dry-run] [--json]
  lore forget <id> --reason <text>
  lore reindex | lore verify | lore log [--jsonl] [--since-seq N] | lore export --jsonl

Integrations:
  lore mcp          # stdio JSON-RPC 2.0 MCP server (recall, inspect, remember, feedback, pulse)
  lore hook prompt  # fail-open prompt hook (<=350 tokens, always exits 0)
  lore hook cue     # fail-open error-cue hook (<=250 tokens, always exits 0)
";

#[derive(Debug, Parser)]
#[command(
    name = "lore",
    version,
    about = "Collective long-term epistemic memory and live cross-agent awareness"
)]
struct Cli {
    /// Override the lore data directory (defaults to $LORE_DIR or ~/.lore).
    #[arg(long, global = true)]
    root: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Recall ranked L0 memory cards for the current workspace and query.
    Recall(RecallArgs),
    /// Show full L1 details, dead ends, lineage, and local verifier results for a claim.
    Show(ShowArgs),
    /// Record a new durable claim.
    Remember(RememberArgs),
    /// Record outcome feedback or transition a claim's lifecycle status.
    Feedback(FeedbackArgs),
    /// Immediately erase a claim across hot logs, cold archives, index, and orphan blobs.
    Forget(ForgetArgs),
    /// Publish ephemeral live session presence and inspect active peer overlaps.
    Pulse(PulseArgs),
    /// Show store health, global switch state, and disk usage.
    Status(StatusArgs),
    /// Read or update global lore configuration.
    Config(ConfigCmd),
    /// Run (status, age) cold zstd archival, purge expired items, and vacuum index.db.
    Gc(GcArgs),
    /// Rebuild index.db deterministically from authoritative .pblog and .pblog.zst logs.
    Reindex(JsonFlag),
    /// Verify integrity and checksums of all hot segments and cold archives.
    Verify(JsonFlag),
    /// Stream authoritative log events.
    Log(LogArgs),
    /// Export authoritative log events as JSONL.
    Export(ExportArgs),
    /// Run the stdio JSON-RPC 2.0 MCP server.
    Mcp,
    /// Fail-open agent hook entrypoints.
    Hook(HookCmd),
    /// Print compact agent usage cheat-sheet.
    Guide,
}

#[derive(Debug, Args)]
struct RecallArgs {
    /// Optional search query.
    query: Option<String>,
    /// Filter or boost by typed `<ns>:<key>` entity URIs.
    #[arg(long = "about")]
    about: Vec<String>,
    /// Filter by claim kind (repeatable).
    #[arg(long = "kind")]
    kinds: Vec<String>,
    /// Override permitted scope resolution.
    #[arg(long)]
    scope: Option<String>,
    /// Maximum number of cards to return (1..=20, default 8).
    #[arg(long)]
    limit: Option<usize>,
    /// Include non-active / cold-archived historical claims.
    #[arg(long)]
    include_history: bool,
    /// Skip querying live peer presence.
    #[arg(long)]
    no_pulse: bool,
    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct ShowArgs {
    /// Claim ID (e.g. `clm_...`).
    id: String,
    /// Include supersession/contradiction/support lineage edges.
    #[arg(long)]
    lineage: bool,
    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct RememberArgs {
    /// Claim kind (axiom|fact|decision|solution|dead_end|warning|preference|question|handoff).
    #[arg(long)]
    kind: String,
    /// Concise L0 headline statement (<= 280 UTF-8 chars).
    statement: String,
    /// Optional L1 markdown body with details.
    #[arg(long)]
    body: Option<String>,
    /// Dead end tried that did not work (repeatable).
    #[arg(long = "dead-end")]
    dead_ends: Vec<String>,
    /// Extra search keywords (repeatable).
    #[arg(long = "keyword")]
    keywords: Vec<String>,
    /// Explicit scope override (`global`, `repo:<id>`, `channel:<id>`, `machine:<id>`, etc.).
    #[arg(long)]
    scope: Option<String>,
    /// Typed `<ns>:<key>` entity URI (repeatable).
    #[arg(long = "about")]
    about: Vec<String>,
    /// Applicability predicate in `axis:op:value` format (repeatable).
    #[arg(long = "when")]
    when: Vec<String>,
    /// Claim ID superseded by this new claim.
    #[arg(long)]
    supersedes: Option<String>,
    /// Pin this claim in Tier-0 hot storage.
    #[arg(long)]
    pin: bool,
    /// Relative importance multiplier (default 1.0).
    #[arg(long, default_value_t = 1.0)]
    importance: f32,
    /// Redact detected secrets instead of rejecting the write.
    #[arg(long)]
    redact: bool,
    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct FeedbackArgs {
    /// Claim ID.
    id: String,
    /// Outcome or lifecycle transition (`helpful|harmful|not-applicable|corroborate|dispute|invalidate|retract`).
    outcome: String,
    /// Optional note explaining the feedback.
    #[arg(long)]
    note: Option<String>,
    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct ForgetArgs {
    /// Claim ID to permanently erase.
    id: String,
    /// Reason recorded on the minimal tombstone.
    #[arg(long)]
    reason: String,
    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct PulseArgs {
    /// Current session intent.
    #[arg(long)]
    intent: Option<String>,
    /// Current working hypothesis.
    #[arg(long)]
    hypothesis: Option<String>,
    /// File path currently being edited (repeatable).
    #[arg(long = "touch")]
    touched_paths: Vec<String>,
    /// Recent error message or `err:<sig>` (repeatable).
    #[arg(long = "fail")]
    recent_failures: Vec<String>,
    /// Custom presence TTL in milliseconds.
    #[arg(long)]
    ttl_ms: Option<i64>,
    /// Query active peers without updating this session's presence.
    #[arg(long)]
    query_only: bool,
    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct StatusArgs {
    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct ConfigCmd {
    #[command(subcommand)]
    action: ConfigAction,
}

#[derive(Debug, Subcommand)]
enum ConfigAction {
    /// Print a configuration value (e.g. `memory.enabled`).
    Get { key: String },
    /// Set a configuration value (e.g. `memory.enabled false --reason "..."`).
    Set {
        key: String,
        value: String,
        /// Optional human-supplied reason when disabling `memory.enabled`.
        #[arg(long)]
        reason: Option<String>,
    },
}

#[derive(Debug, Args)]
struct GcArgs {
    /// Report what would be archived/purged without modifying disk.
    #[arg(long)]
    dry_run: bool,
    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct JsonFlag {
    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct LogArgs {
    /// Emit deterministic JSONL (one LogEvent JSON object per line).
    #[arg(long)]
    jsonl: bool,
    /// Only emit events with `seq > since_seq`.
    #[arg(long, default_value_t = 0)]
    since_seq: u64,
}

#[derive(Debug, Args)]
struct ExportArgs {
    /// Emit deterministic JSONL (one LogEvent JSON object per line).
    #[arg(long)]
    jsonl: bool,
    /// Only export events with `seq > since_seq`.
    #[arg(long, default_value_t = 0)]
    since_seq: u64,
}

#[derive(Debug, Args)]
struct HookCmd {
    #[command(subcommand)]
    kind: HookKind,
}

#[derive(Debug, Subcommand)]
enum HookKind {
    /// Fail-open user prompt submission hook (`HOOK-1`).
    Prompt,
    /// Fail-open failing tool/command cue hook (`HOOK-2`).
    Cue,
}

/// Production CLI entrypoint reading `std::env::args_os()`, stdin, cwd, and env.
pub fn run_from_env() -> i32 {
    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let env_hints = EnvHints::from_env();
    let args: Vec<OsString> = env::args_os().collect();

    // Special-case `mcp` so it streams stdio directly instead of buffering all stdin first.
    // Uses `open_lazy` so starting `lore mcp` while `memory.enabled = false` never touches storage (`SW-4`).
    if let Ok(cli) = Cli::try_parse_from(&args) {
        if matches!(cli.command, Command::Mcp) {
            let root = resolve_root(cli.root);
            let backend = LocalBackend::open_lazy(&root);
            let mut server = McpServer::new(backend, cwd, env_hints);
            let stdin = io::stdin();
            let stdout = io::stdout();
            return match server.run_stdio(stdin.lock(), stdout.lock()) {
                Ok(()) => 0,
                Err(err) => {
                    let _ = writeln!(io::stderr(), "lore mcp I/O error: {err}");
                    1
                }
            };
        }
    }

    let mut stdin_buf = Vec::new();
    // Only read stdin for hook subcommands so interactive CLI commands don't block on a TTY.
    if args.iter().any(|a| a == "hook") {
        let _ = io::stdin().read_to_end(&mut stdin_buf);
    }

    let mut stdout = io::stdout();
    let mut stderr = io::stderr();
    run_with_io(args, &stdin_buf, &cwd, &env_hints, &mut stdout, &mut stderr)
}

/// Testable in-process CLI entrypoint.
pub fn run_with_io<I, T>(
    args: I,
    stdin: &[u8],
    cwd: &Path,
    env_hints: &EnvHints,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = match Cli::try_parse_from(args) {
        Ok(c) => c,
        Err(err) => {
            let code = err.exit_code();
            let _ = write!(stderr, "{err}");
            return code;
        }
    };

    let root = resolve_root(cli.root);

    // HOOK-3: Hooks are strictly fail-open and MUST ALWAYS exit 0.
    if let Command::Hook(hook) = cli.command {
        run_hook_fail_open(&root, hook.kind, stdin, cwd, env_hints, stdout);
        return 0;
    }

    match dispatch(&root, cli.command, stdin, cwd, env_hints, stdout) {
        Ok(()) => 0,
        Err(err) => {
            let _ = writeln!(stderr, "{err}");
            1
        }
    }
}

fn resolve_root(explicit: Option<PathBuf>) -> PathBuf {
    if let Some(p) = explicit {
        return p;
    }
    if let Ok(raw) = env::var("LORE_DIR") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    if let Ok(home) = env::var("HOME") {
        if !home.trim().is_empty() {
            return PathBuf::from(home).join(".lore");
        }
    }
    PathBuf::from(".lore")
}

fn run_hook_fail_open(
    root: &Path,
    kind: HookKind,
    stdin: &[u8],
    cwd: &Path,
    env_hints: &EnvHints,
    stdout: &mut dyn Write,
) {
    // SW-4 / HOOK-3: check config first without touching storage.
    let Ok(cfg) = load_config(root) else {
        return;
    };
    if !cfg.memory.enabled {
        return;
    }

    let Ok(mut backend) = LocalBackend::open_read_only(root, HOOK_BUSY_TIMEOUT_MS) else {
        return;
    };
    let output = match kind {
        HookKind::Prompt => handle_prompt_hook(&mut backend, stdin, cwd, env_hints),
        HookKind::Cue => handle_cue_hook(&mut backend, stdin, cwd, env_hints),
    };
    if let Some(text) = output {
        let _ = writeln!(stdout, "{text}");
    }
}

fn dispatch(
    root: &Path,
    command: Command,
    stdin: &[u8],
    cwd: &Path,
    env_hints: &EnvHints,
    stdout: &mut dyn Write,
) -> Result<()> {
    match command {
        // SW-4: Agent-facing commands check `memory.enabled` BEFORE touching storage.
        Command::Guide => {
            let cfg = load_config(root)?;
            ensure_enabled(&cfg)?;
            write_str(stdout, GUIDE_TEXT)?;
            Ok(())
        }
        Command::Recall(args) => {
            ensure_enabled(&load_config(root)?)?;
            let mut backend = LocalBackend::open(root)?;
            cmd_recall(&mut backend, args, cwd, env_hints, stdout)
        }
        Command::Show(args) => {
            ensure_enabled(&load_config(root)?)?;
            let backend = LocalBackend::open(root)?;
            cmd_show(&backend, args, cwd, env_hints, stdout)
        }
        Command::Remember(args) => {
            ensure_enabled(&load_config(root)?)?;
            let mut backend = LocalBackend::open(root)?;
            cmd_remember(&mut backend, args, cwd, env_hints, stdout)
        }
        Command::Feedback(args) => {
            ensure_enabled(&load_config(root)?)?;
            let mut backend = LocalBackend::open(root)?;
            cmd_feedback(&mut backend, args, cwd, env_hints, stdout)
        }
        Command::Pulse(args) => {
            ensure_enabled(&load_config(root)?)?;
            let mut backend = LocalBackend::open(root)?;
            cmd_pulse(&mut backend, args, cwd, env_hints, stdout)
        }

        // SW-5: Human admin commands remain operable when `memory.enabled == false`.
        // Read-only commands (`status`, `verify`, `log`, `export`) never acquire `write.lock`
        // or truncate torn tails.
        Command::Config(cfg_cmd) => cmd_config(root, cfg_cmd, stdout),
        Command::Status(args) => cmd_status(root, args, stdout),
        Command::Gc(args) => {
            let mut backend = LocalBackend::open(root)?;
            let report = backend.gc(args.dry_run)?;
            if args.json {
                writeln_json(stdout, &report)?;
            } else {
                writeln_str(
                    stdout,
                    &format!(
                        "gc (dry_run={}): archived_claims={} archived_events={} purged_handoff={} purged_retracted={} purged_quarantined={} folded_exposures={} swept_blobs={} freed_blob_bytes={} vacuumed={}",
                        report.dry_run,
                        report.archived_claims,
                        report.archived_events,
                        report.purged_handoff,
                        report.purged_retracted,
                        report.purged_quarantined,
                        report.folded_exposures,
                        report.swept_blobs,
                        report.freed_blob_bytes,
                        report.vacuumed,
                    ),
                )?;
            }
            Ok(())
        }
        Command::Forget(args) => {
            let mut backend = LocalBackend::open(root)?;
            let frame = detect(cwd, env_hints);
            let actor = Principal {
                kind: PrincipalKind::Owner as i32,
                id: "human".to_string(),
                session_id: frame.session_id.unwrap_or_default(),
                agent_kind: "human".to_string(),
                model: String::new(),
                machine_id: frame.machine_id,
            };
            backend.forget(&args.id, &args.reason, actor)?;
            if args.json {
                writeln_json(
                    stdout,
                    &json!({ "forgotten": true, "id": args.id, "reason": args.reason }),
                )?;
            } else {
                writeln_str(stdout, &format!("forgotten {}", args.id))?;
            }
            Ok(())
        }
        Command::Reindex(flag) => {
            let mut backend = LocalBackend::open(root)?;
            let replayed = backend.reindex()?;
            if flag.json {
                writeln_json(stdout, &json!({ "replayed_events": replayed }))?;
            } else {
                writeln_str(stdout, &format!("reindexed {replayed} events"))?;
            }
            Ok(())
        }
        Command::Verify(flag) => {
            let report = verify_store(root)?;
            if flag.json {
                writeln_json(stdout, &report)?;
            } else {
                writeln_str(
                    stdout,
                    &format!(
                        "verified ok: hot_segments={} hot_events={} archive_segments={} archive_frames={} archive_events={} max_seq={}",
                        report.hot_segments,
                        report.hot_events,
                        report.archive_segments,
                        report.archive_frames,
                        report.archive_events,
                        report.max_seq,
                    ),
                )?;
            }
            Ok(())
        }
        Command::Log(args) => {
            let events = export_store_events(root, args.since_seq)?;
            for ev in &events {
                if args.jsonl {
                    writeln_json(stdout, ev)?;
                } else {
                    writeln_str(stdout, &format_log_event_line(ev))?;
                }
            }
            Ok(())
        }
        Command::Export(args) => {
            let events = export_store_events(root, args.since_seq)?;
            let _ = args.jsonl; // export is always JSONL (`ST-7`)
            for ev in &events {
                writeln_json(stdout, ev)?;
            }
            Ok(())
        }
        Command::Mcp => {
            let backend = LocalBackend::open_lazy(root);
            let mut server = McpServer::new(backend, cwd.to_path_buf(), env_hints.clone());
            server
                .run_stdio(io::Cursor::new(stdin), stdout)
                .map_err(|source| LoreError::Io {
                    path: root.to_path_buf(),
                    source,
                })
        }
        Command::Hook(_) => unreachable!("handled fail-open above"),
    }
}

fn cmd_config(root: &Path, cmd: ConfigCmd, stdout: &mut dyn Write) -> Result<()> {
    match cmd.action {
        ConfigAction::Get { key } => {
            let cfg = load_config(root)?;
            match key.trim() {
                "memory.enabled" => writeln_str(stdout, &cfg.memory.enabled.to_string())?,
                "memory.disabled_reason" => {
                    writeln_str(stdout, cfg.memory.disabled_reason.as_deref().unwrap_or(""))?
                }
                other => {
                    return Err(LoreError::InvalidScope {
                        scope: format!("unknown config key `{other}` (expected `memory.enabled` or `memory.disabled_reason`)"),
                    });
                }
            }
            Ok(())
        }
        ConfigAction::Set { key, value, reason } => {
            let mut cfg = load_config(root)?;
            match key.trim() {
                "memory.enabled" => {
                    let enabled = parse_bool(&value).ok_or_else(|| LoreError::InvalidScope {
                        scope: format!(
                            "invalid boolean `{value}` for `memory.enabled` (expected `true` or `false`)"
                        ),
                    })?;
                    cfg.memory.enabled = enabled;
                    cfg.memory.disabled_reason = if enabled {
                        None
                    } else {
                        reason.filter(|r| !r.trim().is_empty())
                    };
                    save_config(root, &cfg)?;
                    if enabled {
                        writeln_str(stdout, "memory.enabled = true")?;
                    } else if let Some(r) = &cfg.memory.disabled_reason {
                        writeln_str(stdout, &format!("memory.enabled = false (reason: {r})"))?;
                    } else {
                        writeln_str(stdout, "memory.enabled = false")?;
                    }
                    Ok(())
                }
                "memory.disabled_reason" => {
                    let trimmed = value.trim();
                    cfg.memory.disabled_reason = if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.to_string())
                    };
                    save_config(root, &cfg)?;
                    writeln_str(
                        stdout,
                        &format!(
                            "memory.disabled_reason = {}",
                            cfg.memory.disabled_reason.as_deref().unwrap_or("")
                        ),
                    )?;
                    Ok(())
                }
                other => Err(LoreError::InvalidScope {
                    scope: format!("unknown config key `{other}` (expected `memory.enabled` or `memory.disabled_reason`)"),
                }),
            }
        }
    }
}

fn cmd_status(root: &Path, args: StatusArgs, stdout: &mut dyn Write) -> Result<()> {
    let status = read_only_status(root, &SystemClock)?;
    if args.json {
        writeln_json(stdout, &status)?;
        return Ok(());
    }

    writeln_str(stdout, &format!("memory.enabled = {}", status.enabled))?;
    if !status.enabled {
        let reason = status
            .disabled_reason
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("Intentionally disabled by the user");
        writeln_str(stdout, &format!("disabled_reason = {reason}"))?;
    }
    writeln_str(stdout, "mode = local")?;
    writeln_str(
        stdout,
        &format!(
            "claims: hot={} cold={} active={} applied_seq={} live_peers={}",
            status.hot_claims,
            status.cold_claims,
            status.active_claims,
            status.applied_seq,
            status.live_peers,
        ),
    )?;
    writeln_str(
        stdout,
        &format!(
            "disk_bytes: hot={} archive={} blobs={} index={} last_gc_ms={}",
            status.hot_bytes,
            status.archive_bytes,
            status.blob_bytes,
            status.index_bytes,
            status.last_gc_ms,
        ),
    )?;
    Ok(())
}

fn cmd_recall<B: LoreBackend>(
    backend: &mut B,
    args: RecallArgs,
    cwd: &Path,
    env_hints: &EnvHints,
    stdout: &mut dyn Write,
) -> Result<()> {
    let frame = detect(cwd, env_hints);
    let mut req = RecallRequest::new(frame);
    req.query = args.query.unwrap_or_default();
    req.about = args.about;
    req.scope_override = args.scope;
    req.limit = args.limit;
    req.include_history = args.include_history;
    req.include_pulse = !args.no_pulse;

    if !args.kinds.is_empty() {
        let mut parsed = Vec::with_capacity(args.kinds.len());
        for k in &args.kinds {
            let kind = parse_claim_kind(k).ok_or_else(|| LoreError::InvalidScope {
                scope: format!("unknown claim kind `{k}`"),
            })?;
            parsed.push(kind);
        }
        req.kinds = Some(parsed);
    }

    let resp = backend.recall(req)?;
    if args.json {
        writeln_json(stdout, &resp)?;
        return Ok(());
    }

    if resp.cards.is_empty() {
        writeln_str(stdout, "No matching claims.")?;
    } else {
        for card in &resp.cards {
            writeln_str(stdout, &format_evidence_envelope(card))?;
        }
        if let Some(notice) = &resp.notice {
            writeln_str(stdout, &format!("Note: {notice}"))?;
        }
    }

    for peer in &resp.peers {
        writeln_str(
            stdout,
            &format!(
                "[peer {} ({}@{}) | intent: {}]",
                peer.session_id,
                peer.agent_kind,
                peer.machine_id,
                peer.intent.as_deref().unwrap_or("active"),
            ),
        )?;
    }
    Ok(())
}

fn cmd_show<B: LoreBackend>(
    backend: &B,
    args: ShowArgs,
    cwd: &Path,
    env_hints: &EnvHints,
    stdout: &mut dyn Write,
) -> Result<()> {
    let frame = detect(cwd, env_hints);
    let resp = backend.inspect(InspectRequest {
        id: args.id,
        frame: Some(frame),
        include_lineage: args.lineage,
    })?;

    if args.json {
        writeln_json(stdout, &resp)?;
        return Ok(());
    }

    let kind = ClaimKind::try_from(resp.claim.kind).unwrap_or(ClaimKind::Unspecified);
    writeln_str(
        stdout,
        &format!(
            "{} [{}] ({}, verifier: {}, hydrated_from_cold={})",
            resp.claim.id,
            claim_kind_str(kind),
            resp.claim.scope,
            verifier_status_str(resp.verifier_status),
            resp.hydrated_from_cold,
        ),
    )?;
    writeln_str(stdout, &resp.claim.statement)?;
    if !resp.claim.body.trim().is_empty() {
        writeln_str(stdout, "")?;
        writeln_str(stdout, &resp.claim.body)?;
    }
    if !resp.claim.dead_ends.is_empty() {
        writeln_str(stdout, "Dead ends:")?;
        for de in &resp.claim.dead_ends {
            writeln_str(stdout, &format!("  - {de}"))?;
        }
    }
    if !resp.edges.is_empty() {
        writeln_str(stdout, "Lineage edges:")?;
        for edge in &resp.edges {
            writeln_str(
                stdout,
                &format!("  {} --{:?}--> {}", edge.src_id, edge.rel, edge.dst_id),
            )?;
        }
    }
    Ok(())
}

fn cmd_remember<B: LoreBackend>(
    backend: &mut B,
    args: RememberArgs,
    cwd: &Path,
    env_hints: &EnvHints,
    stdout: &mut dyn Write,
) -> Result<()> {
    let kind = parse_claim_kind(&args.kind).ok_or_else(|| LoreError::InvalidScope {
        scope: format!("unknown claim kind `{}`", args.kind),
    })?;

    let frame = detect(cwd, env_hints);
    let mut about = args.about;
    let scope = resolve_write_scope(args.scope.as_deref(), &frame, &mut about);

    let mut applies_when = Vec::with_capacity(args.when.len());
    for raw in &args.when {
        applies_when.push(parse_when_predicate(raw)?);
    }

    let req = RememberRequest {
        kind,
        statement: args.statement,
        body: args.body.unwrap_or_default(),
        dead_ends: args.dead_ends,
        keywords: args.keywords,
        attrs: BTreeMap::new(),
        scope,
        about,
        applies_when,
        author: build_cli_principal(&frame),
        tainted: false,
        origin: Some(build_cli_origin(&frame)),
        verifiers: Vec::new(),
        supersedes: args.supersedes,
        pinned: args.pin,
        importance: args.importance,
        valid_from_ms: 0,
        valid_until_ms: 0,
        redact_secrets: args.redact,
        event_id: None,
    };

    let resp = backend.remember(req)?;
    if args.json {
        writeln_json(stdout, &resp)?;
    } else {
        writeln_str(stdout, &resp.id)?;
    }
    Ok(())
}

fn cmd_feedback<B: LoreBackend>(
    backend: &mut B,
    args: FeedbackArgs,
    cwd: &Path,
    env_hints: &EnvHints,
    stdout: &mut dyn Write,
) -> Result<()> {
    let action = parse_feedback_action(&args.outcome).ok_or_else(|| LoreError::InvalidScope {
        scope: format!("unknown feedback outcome `{}`", args.outcome),
    })?;

    let frame = detect(cwd, env_hints);
    let resp = backend.feedback(FeedbackRequest {
        id: args.id,
        action,
        note: args.note.unwrap_or_default(),
        actor: build_cli_principal(&frame),
        tainted: false,
        event_id: None,
    })?;

    if args.json {
        writeln_json(stdout, &resp)?;
    } else {
        writeln_str(
            stdout,
            &format!(
                "{} status={:?} helpful={} harmful={} corroborated={} disputed={}",
                resp.id,
                resp.new_status,
                resp.counters.helpful_count,
                resp.counters.harmful_count,
                resp.counters.corroborator_count,
                resp.counters.dispute_count,
            ),
        )?;
    }
    Ok(())
}

fn cmd_pulse<B: LoreBackend>(
    backend: &mut B,
    args: PulseArgs,
    cwd: &Path,
    env_hints: &EnvHints,
    stdout: &mut dyn Write,
) -> Result<()> {
    let frame = detect(cwd, env_hints);
    let mut req = PulseRequest::new(frame);
    req.intent = args.intent;
    req.hypothesis = args.hypothesis;
    req.touched_paths = args.touched_paths;
    req.recent_failures = args.recent_failures;
    req.ttl_ms = args.ttl_ms;
    req.query_only = args.query_only;

    let resp = backend.pulse(req)?;
    if args.json {
        writeln_json(stdout, &resp)?;
        return Ok(());
    }

    if resp.peers.is_empty() {
        writeln_str(stdout, "No other active peers.")?;
    } else {
        for peer in &resp.peers {
            writeln_str(
                stdout,
                &format!(
                    "{} ({}@{}) intent={}",
                    peer.session_id,
                    peer.agent_kind,
                    peer.machine_id,
                    peer.intent.as_deref().unwrap_or("active"),
                ),
            )?;
        }
    }
    for ov in &resp.overlaps {
        writeln_str(
            stdout,
            &format!(
                "OVERLAP with {} ({}@{}): paths={:?} errors={:?}",
                ov.peer_session_id,
                ov.peer_agent_kind,
                ov.peer_machine_id,
                ov.overlapping_paths,
                ov.overlapping_errors,
            ),
        )?;
    }
    Ok(())
}

fn build_cli_origin(frame: &ContextFrame) -> OriginSnapshot {
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

fn build_cli_principal(frame: &ContextFrame) -> Principal {
    let agent_kind = frame
        .agent_kind
        .clone()
        .unwrap_or_else(|| "human".to_string());
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

fn parse_when_predicate(raw: &str) -> Result<Predicate> {
    // Scan colons from the right so axes containing colons (e.g. `dep:cargo/serde`
    // or `dep:maven/com.google.guava:guava`) parse cleanly.
    let mut parts: Vec<&str> = raw.split(':').collect();
    if parts.len() >= 3 {
        for idx in (1..parts.len() - 1).rev() {
            let op_candidate = parts[idx].trim();
            let op = match op_candidate.to_ascii_lowercase().as_str() {
                "eq" | "=" | "==" => Some(PredicateOp::Eq),
                "in" => Some(PredicateOp::In),
                "glob" => Some(PredicateOp::Glob),
                "semver_req" | "semver-req" | "semver" => Some(PredicateOp::SemverReq),
                "ancestor_of" | "ancestor-of" => Some(PredicateOp::AncestorOf),
                "not_ancestor_of" | "not-ancestor-of" => Some(PredicateOp::NotAncestorOf),
                _ => None,
            };
            if let Some(op) = op {
                let value = parts.split_off(idx + 1).join(":");
                let value = value.trim();
                let axis = parts[..idx].join(":");
                let axis = axis.trim();
                if axis.is_empty() || value.is_empty() {
                    return Err(LoreError::InvalidPredicate {
                        axis: axis.to_string(),
                        op: op_candidate.to_string(),
                        value: value.to_string(),
                    });
                }
                return Ok(Predicate {
                    axis: axis.to_string(),
                    op: op as i32,
                    value: value.to_string(),
                });
            }
        }
    }
    Err(LoreError::InvalidPredicate {
        axis: raw.trim().to_string(),
        op: String::new(),
        value: String::new(),
    })
}

fn parse_bool(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn format_log_event_line(ev: &lore_proto::LogEvent) -> String {
    let kind = match &ev.payload {
        Some(log_event::Payload::ClaimUpserted(u)) => format!(
            "claim_upserted({})",
            u.claim.as_ref().map(|c| c.id.as_str()).unwrap_or("?")
        ),
        Some(log_event::Payload::ClaimTransitioned(t)) => {
            format!("claim_transitioned({} -> {})", t.claim_id, t.new_status)
        }
        Some(log_event::Payload::EdgeAdded(e)) => {
            format!("edge_added({} -> {})", e.src_id, e.dst_id)
        }
        Some(log_event::Payload::FeedbackRecorded(f)) => {
            format!("feedback_recorded({} outcome={})", f.claim_id, f.outcome)
        }
        Some(log_event::Payload::ClaimForgotten(f)) => {
            format!("claim_forgotten({})", f.claim_id)
        }
        None => "empty".to_string(),
    };
    format!("seq={} event={} {}", ev.seq, ev.event_id, kind)
}

fn write_str(w: &mut dyn Write, text: &str) -> Result<()> {
    w.write_all(text.as_bytes()).map_err(|source| LoreError::Io {
        path: PathBuf::from("<stdout>"),
        source,
    })
}

fn writeln_str(w: &mut dyn Write, line: &str) -> Result<()> {
    writeln!(w, "{line}").map_err(|source| LoreError::Io {
        path: PathBuf::from("<stdout>"),
        source,
    })
}

fn writeln_json<T: serde::Serialize>(w: &mut dyn Write, val: &T) -> Result<()> {
    let rendered = serde_json::to_string(val).map_err(|err| LoreError::Io {
        path: PathBuf::from("<stdout>"),
        source: io::Error::new(io::ErrorKind::InvalidData, err),
    })?;
    writeln_str(w, &rendered)
}
