# `lore` — Intent Specification (`SPEC.md`)

**Status:** Draft v1.0 (Intent Spec — no implementation code yet)  
**Date:** 2026-11-21  
**Workspace Package Root:** `//tools/lore/...`  
**Reference Docs:** [01-RESEARCH-AND-LANDSCAPE.md](docs/01-RESEARCH-AND-LANDSCAPE.md), [02-ARCHITECTURE-AND-SCHEMA.md](docs/02-ARCHITECTURE-AND-SCHEMA.md)

---

## 1. Purpose, Scope & Non-Goals

### 1.1 Problem & Intent
`lore` is a **collective long-term epistemic memory and live cross-agent awareness system** for AI coding agents (`claude`, `agy`, `codex`, `copilot`, `opencode`). It solves two problems simultaneously:
1. **Diachronic Amnesia:** Eliminates repeating standing rules, architectural decisions, environment quirks, root-cause fixes, and dead ends across sessions, branches, harnesses, and machines.
2. **Synchronic Blindness:** Gives concurrent sessions ambient awareness of active peers, mounted repos/branches, touched paths, working hypotheses, and recent build/test failures.

### 1.2 Architectural Invariants
1. **Strict Crate & Build Isolation:** All core crates live under `//tools/lore/...` with **zero dependencies on `apps/humata2/*`**. `tools/bq/bq --fast //tools/lore/...` must iterate in ~1–2 seconds without triggering Humata UI, desktop, client, or daemon rebuilds.
2. **Host-as-Single-Brain (Zero Cross-Machine File Sync):** Inside Humata, one authoritative `lore` store lives on the Mac Host. Remote machines forward writes and fleet-wide queries to the Host over Humata's existing authenticated SSH/gRPC `ForwardedCall` bridge, while verifying local file/git citations on the calling machine. Outside Humata, the same `lore` binary operates standalone against `~/.lore/`.
3. **Single Global Kill-Switch (`memory.enabled`):** One boolean toggle non-destructively pauses or resumes the entire system across all machines and agent harnesses. When `false`, prompt/schema overhead is **0 tokens** and all stored data remains intact on disk.
4. **Zero-LLM, Zero-Compression Hot Path:** Active writes append uncompressed framed Protobuf records to `.pblog` and update a disposable SQLite WAL + FTS5 projection. `zstd` compression and GC run **only** on old or non-active items matched by `(status, age)` rules.
5. **Code Precedence Over Stale Memory:** Non-axiom memories are always surfaced as provenance-tagged, locally verified evidence—never as raw system instructions—so agents trust verified current code over drifted claims.
6. **Zero Idle Timers & Fail-Open Hooks:** Idle `lore` / `svc_lore` instances run zero polling loops or periodic timers. Shell/prompt hooks are strictly fail-open (exit `0`) under hard latency caps.

### 1.3 Non-Goals (v1)
- Cross-machine peer-to-peer file sync or CRDT log merging.
- Write-time LLM summarization, automatic entity extraction via LLM, or mandatory vector/embedding databases (a `ScoringPolicy` hook point is reserved for optional future embeddings).
- Custom agent query DSLs (e.g., `KQL`/`KML`).
- Replacing `AGENTS.md` for tiny, always-on core repository invariants.

---

## 2. Crate Topology & Build Boundaries

```text
tools/lore/
  proto/        # lore_proto   — prost + serde types, IDs, enums, wire frames (zero I/O)
  context/      # lore_context — <2ms 0/1/N-repo discovery, worktree parsing, OS/lockfiles, local verifiers
  engine/       # lore_engine  — .pblog WAL, SQLite WAL/FTS5 projection, ScoringPolicy, (status, age) zstd GC
  mcp/          # lore_mcp     — Backend trait, 5 stdio MCP tools, <3ms prompt/cue hook handlers
  src/          # lore CLI     — clap binary (`lore`, `:lore_linux` static musl)
  docs/         # research & architecture docs
  SPEC.md       # this specification
```

### Dependency & Visibility Rules
- **Reused Root `Cargo.toml` Crates (Zero New Mandatory Dependencies):** `prost`, `rusqlite` (bundled w/ FTS5), `zstd`, `crc32fast`, `sha2`, `clap`, `serde`, `serde_json`, `thiserror`, `versions`, `regex`, `rustix`, `uuid` (note: generate time-ordered `clm_<ts_hex>_<rand_hex>` or enable `uuid/v7`; zero extra crates needed).
- **Minimal Visibility:** All Rust items default to private / `pub(crate)`; Bazel `visibility` on `//tools/lore/{proto,context,engine,mcp}` is restricted to `//tools/lore/...` and (at Milestone `M4`) `//apps/humata2/daemon/svc_lore:__pkg__`.
- **Bazel-Only Builds:** All builds, tests, and lints run via `tools/bq/bq [--build] [--fast] //tools/lore/...` (never bare `bazel` or `cargo`).

---

## 3. Numbered Testable Requirements

### 3.1 Global Switch (`SW-*`)
- **SW-1 (Single Source of Truth, Disabled Reason & Settings UI Toggle):** Global enablement is controlled by `[memory]` in a single global TOML config (`~/.lore/config.toml` standalone, or Humata's Host config propagated to all linked machines):
  ```toml
  [memory]
  enabled = true
  disabled_reason = "" # optional human-supplied reason shown when disabled
  ```
  Toggleable via `lore config set memory.enabled <true|false> [--reason "..."]` or the dedicated **Collective Memory (`lore`) toggle switch** in the Humata Desktop Settings page (`apps/humata2/features/settings`, showing live fleet status, store size, and one-click Pause/Resume).
- **SW-2 (Non-Destructive Pause):** Flipping `memory.enabled` from `true` to `false` MUST NOT delete, truncate, or modify any `.pblog`, `.pblog.zst`, `blobs/`, or `index.db` files. Flipping back to `true` immediately resumes full recall over existing data.
- **SW-3 (Zero-Token Session Omission):** When `memory.enabled == false`, newly launched Humata agent sessions MUST NOT inject the `lore` MCP server, MUST NOT register `lore hook prompt` / `lore hook cue`, and MUST NOT load the `lore` skill (0 prompt/schema tokens).
- **SW-4 (Immediate Fleet-Wide Refusal with `disabled_reason` — CLI & MCP):** When `memory.enabled == false`, **both** CLI agent commands (`lore recall`, `lore show`, `lore remember`, `lore feedback`, `lore pulse`, `lore guide`) AND in-flight MCP tool calls MUST stop working immediately without touching storage, returning an unmistakable refusal containing `disabled_reason` (or a default *"Intentionally disabled by the user"*) plus an explicit anti-circumvention directive (*"Do NOT attempt to re-enable, debug, or work around `lore`; proceed with your task without it."*). Hooks (`lore hook prompt` / `lore hook cue`) silently exit `0` with empty stdout (0 tokens).
- **SW-5 (Human Admin Commands Only):** Only `lore status`, `lore config get|set`, `lore gc`, `lore forget`, and `lore export` remain operable when `memory.enabled == false`, and `lore status` clearly prints `memory.enabled = false` along with `disabled_reason`.

### 3.2 Workspace Topology & Branch Semantics (`TOP-*`)
- **TOP-1 (`ContextFrame` Latency & Pure-Rust Discovery):** `lore_context::detect(cwd, env_hints)` MUST complete in `<2 ms` p95 on local SSD without spawning `git` subprocesses for basic `.git` pointer detection.
- **TOP-2 (Single Git Repo & Standard Worktree):** Walking upward from `cwd`, if a `.git` directory or `.git` file (`gitdir: <path>`) is found, `lore_context` resolves one `RepoMount` with canonical `repo_id`, detected `languages: Vec<Language>` (from `Language` enum, supporting `Polyglot`/`Other`/`Unknown`), and discovered `lockfiles: Vec<LockfileRef>` (`Cargo.lock`, `go.mod`, `uv.lock`, `poetry.lock`, `requirements.txt`, `pyproject.toml`, `pom.xml`, `gradle.lockfile`, `MODULE.bazel.lock`, `MODULE.bazel`, `Package.resolved`):
  1. Normalized `remote.origin.url` (lowercase host, stripped credentials, SSH/HTTPS unified, `.git` suffix removed, e.g., `github.com/owner/repo`).
  2. If no `origin` remote exists, deterministic `git:<root_commit_sha>` (hash of the repository's initial commit).
- **TOP-3 (Sandboxed Worktree Resilience):** If `.git` is a `gitdir:` pointer whose target lies outside a sandbox (e.g., Seatbelt/container returning `EPERM` or `git` exiting `128`), `lore_context` MUST NOT fail or panic. It MUST derive a fallback `repo_id` from (in order): (a) Humata's injected `HUMATA_SESSION_REPOS` env hint, (b) the parent repo directory name parsed directly from the `gitdir:` string (`<parent>/.git/worktrees/<name>` -> `worktree:<parent_basename>`), or (c) `dir:<machine_id>:<sha256(canonical_cwd)[0..16]>`.
- **TOP-4 (Multi-Repo Parent Directory — `N` Repos):** If upward walk finds no `.git`, `lore_context` inspects `HUMATA_SESSION_REPOS` (if set) plus a bounded depth-1..2 directory scan (skipping hidden dirs, `node_modules`, `target`, `bazel-*`, symlinks, capped at 64 entries) to populate `mounted_repos: Vec<RepoMount { repo_id, mount_subpath, branch?, head_sha? }>`.
  - Queries span `global` $\cup$ `machine:<m>` $\cup$ `channel:<c>` $\cup$ `{repo:<r> | r in mounted_repos}`. If `about` specifies paths inside one `mount_subpath`, repo filtering narrows to that `repo_id`.
  - Writes citing a file under `<mount_subpath>/<rel>` auto-normalize to `(scope: "repo:<id>", path: "<rel>")`. Cross-repo writes bind to `channel:<c>` (if in a Humata channel) or `global`, with `about: ["repo:<a>", "repo:<b>"]`.
- **TOP-5 (Non-Git Directory — `0` Repos):** Outside any git repo (e.g., `~`, Humata Desk, ops dirs), `mounted_repos` is empty. Queries and writes succeed cleanly using `global`, `machine:<m>`, `channel:<c>`, and `dir:<m>:<cwd_hash>`.
- **TOP-6 (Branch-Agnostic Durable Claims):** Durable claims MUST NOT include branch name in `scope`. `origin.branch` and `origin.head_commit` are recorded strictly as audit provenance. Branch-specific divergence is detected at read time on the calling machine via local `Verifier` checks (`file_digest`, `symbol_present`, `since_commit`, `until_commit`).
- **TOP-7 (Opt-In Branch Predicates & Branch-Aware Live Pulse):** Claims that genuinely apply only to specific branches (e.g., release freezes) MAY set an explicit `applies_when` predicate `{ axis: "branch_glob", op: GLOB, value: "release/*" }`. Live `pulse` presence MUST always report each peer's current `branch`, `head_sha`, and `dirty_count`.

### 3.3 Canonical Data Model, Lifecycle & Scoring (`MOD-*`)
- **MOD-1 (Six Orthogonal Facets):** Every `Claim` record MUST carry the 6 facets defined in §4.1: (1) Identity & Lifecycle, (2) Progressive Content (`L0` `statement <= 280` chars, `L1` `body`, `dead_ends`, `keywords`, extensible `attrs`), (3) `scope` + typed `<ns>:<key>` `about[]` entity URIs, (4) `applies_when[]` predicates, (5) Provenance/Taint/Verifiers, and (6) Raw Signal Counters.
- **MOD-2 (Explicit Lifecycle State Machine):** Claims have `status in {ACTIVE, SUPERSEDED, INVALIDATED, RETRACTED, EXPIRED, QUARANTINED}`.
  - Superseding claim `A` with `B` atomically appends `B`, creates a `SUPERSEDES` edge `B -> A`, and transitions `A.status = SUPERSEDED`.
  - `invalidate(id, reason)` transitions to `INVALIDATED`; `retract(id, reason)` transitions to `RETRACTED`; `expire` transitions when `now >= valid_until_ms`.
  - Untrusted/tainted writes enter `QUARANTINED` (visible only to the originating session until corroborated by a clean session or promoted by the owner).
- **MOD-3 (Extensible Predicate Evaluation):** `applies_when` predicates evaluate tri-state `{MATCH, MISMATCH, UNKNOWN}`. Any `MISMATCH` excludes the claim from default recall ($G_{\text{hard}} = 0$). Any unrecognized `axis` evaluates to `UNKNOWN` (soft badge, never error or crash).
- **MOD-4 (No Frozen Score — Read-Time `ScoringPolicy`):** Storage records only raw counters (`pinned`, `importance`, `corroborator_count`, `dispute_count`, `surfaced_count`, `helpful_count`, `harmful_count`, `not_applicable_count`, `last_surfaced_at_ms`, `last_validated_at_ms`). Ranking is computed at read time via a pluggable `ScoringPolicy` trait:
  $$\text{Score}(c, q, \text{ctx}) = G_{\text{hard}}(c, \text{ctx}) \cdot \text{importance}_c \cdot \sum_{k} w_k \, \phi_k(c, q, \text{ctx})$$
  with default v1 features: $\phi_{\text{lex}}$ (code-aware FTS5 BM25), $\phi_{\text{ent}}$ (`about` entity specificity), $\phi_{\text{scope}}$ (scope specificity), $\phi_{\text{trust}}$ (derived trust tier: `owner > corroborated > agent > quarantined`), $\phi_{\text{fresh}}$ (verifier status $\times$ kind-specific exponential recency decay), and $\phi_{\text{util}} = \frac{\text{helpful} + 2}{\text{helpful} + 2\cdot\text{harmful} + 0.3\cdot\text{unused} + 4}$.
- **MOD-5 (Corroboration Deduplication):** `corroborator_count` counts distinct `(session_id, machine_id)` pairs that confirmed the claim; repeated confirmations from the same session are idempotent.

### 3.4 Storage, Framing & Crash Recovery (`ST-*`)
- **ST-1 (Source of Truth vs. Disposable Index):** Append-only framed Protobuf logs (`hot/seg-*.pblog` + `archive/arch-*.pblog.zst`) and content-addressed blobs (`blobs/<sha256>.blob`) are the sole authoritative source of truth. `index.db` (SQLite WAL + FTS5) is a fully rebuildable projection (`lore reindex` reproduces `index.db` byte-equivalent in semantics from logs alone).
- **ST-2 (Zero-Compression Hot Writes):** Appending a new event to `hot/seg-<base>.pblog` MUST NOT run `zstd` compression. Writes acquire `write.lock`, append one binary frame (§4.2), `sync_all()` (`F_FULLFSYNC` on macOS), apply to `index.db` in a single SQLite transaction, and release `write.lock` in `<3 ms` p95 on local SSD.
- **ST-3 (Segment Header & Checksum Agility):** Every `.pblog` segment starts with a 16-byte header specifying `magic`, `format_version`, `checksum_alg` (`1 = crc32fast` SIMD IEEE default with zero new deps; `2 = crc32c`, `3 = xxh3_64` reserved), and `header_crc32`. Per-record checksums verify `(seq || payload)` in ~20 ns/record to deterministically catch torn writes and bit flips.
- **ST-4 (Deterministic Torn-Tail Recovery):** On startup, `lore_engine` replays only records with `seq > index.applied_seq`. If the final frame in the active hot segment is truncated or fails its checksum, `lore_engine` truncates the segment to the last valid frame boundary, `sync_all()`s, logs a structured recovery event, and continues without data loss of committed records.
- **ST-5 (Corrupt Mid-Segment Rejection):** A checksum mismatch *not* at the active segment tail MUST NOT silently truncate; it MUST return `LoreError::CorruptSegment { path, offset, seq }` with an actionable recovery message (`lore verify` / `lore reindex`).
- **ST-6 (Code-Aware FTS5 Indexing):** `claims_fts` indexes **Tier-0 Hot** claims only, applying deterministic code-aware pre-tokenization (splitting `camelCase`, `snake_case`, `::`, `/`, `.`, `-` while preserving original tokens) with BM25 column weights `(statement=5.0, keywords_about=4.0, dead_ends=2.5, body=1.0)`.
- **ST-7 (JSONL Human Auditability):** `lore log --jsonl` and `lore export --jsonl` MUST stream deterministic JSONL directly from the same `prost` + `serde` event structs without maintaining a separate text log.
- **ST-8 (Content-Addressed Blobs):** Evidence blobs (`blobs/<sha256>.blob`) are deduplicated by SHA-256. Blobs `< 1 KiB` are stored raw; blobs `>= 1 KiB` are stored as `zstd` frames (with built-in XXH64 checksum) *only* if compressed size saves `>= 6.25%` (`1/16`), else raw.

### 3.5 `(status, age)` `zstd` Cold Archival & Garbage Collection (`GC-*`)
- **GC-1 (Hot-Path Exemption):** Items matching Tier 0 (`status == ACTIVE` AND (`pinned == true` OR `kind in {AXIOM, DECISION}` OR `age_since_validated <= 90d`), OR any non-active item with `updated_at <= 7d` ago) MUST NEVER be compressed or evicted from hot storage.
- **GC-2 (Tier-1 Cold `zstd` Archival):** Items matching either:
  - `(a)` `status in {SUPERSEDED, INVALIDATED}` AND `now - updated_at > 7d`, OR
  - `(b)` `status in {ACTIVE, EXPIRED}` AND `!pinned` AND `kind not in {AXIOM, DECISION}` AND `now - max(created_at, last_validated_at) > 90d`
  are compacted into bulk `archive/arch-<min_seq>-<max_seq>.pblog.zst` files (256 KiB–4 MiB uncompressed blocks, `zstd` level 6, `ZSTD_c_checksumFlag = 1` XXH64 integrity), removed from `claims_fts`, and replaced in `claims` by a lightweight stub (`id, kind, status, statement, tier=COLD, archive_seg, archive_offset, archive_len`).
- **GC-3 (Transparent Hydration & Re-Promotion):** `lore show <id>`, supersession lineage walks, and `recall(include_history=true)` MUST transparently decompress cold stubs in `<5 ms`. If a dormant Tier-1 claim is explicitly corroborated or marked `helpful`, it MUST emit a re-validation event and promote back to Tier 0 (re-entering `claims_fts`).
- **GC-4 (Tier-2 GC Purge Rules):** Maintenance permanently reclaims payloads for:
  - `(a)` `kind == HANDOFF` in `{SUPERSEDED, INVALIDATED, EXPIRED}` with `age > 14d`
  - `(b)` `status == RETRACTED` with `age > 30d`
  - `(c)` `status == QUARANTINED` (uncorroborated) with `age > 14d`
  - `(d)` `claim_exposures` rows with `surfaced_at > 7d` (folded into integer counters on `claims` before deletion)
  - `(e)` Unreferenced files in `blobs/` via mark-and-sweep across live hot + cold archive references
  followed by `PRAGMA incremental_vacuum` on `index.db`.
- **GC-5 (Immediate `forget(id)` Erasure):** `lore forget <id>` (age `>= 0`) MUST immediately remove the claim's content from `index.db`, `claims_fts`, hot `.pblog` segments, cold `.pblog.zst` archives (rewriting only the affected segment atomically via temp file + rename), and orphaned blobs, leaving only a minimal `{ id, purged_at_ms, reason }` tombstone.
- **GC-6 (Zero-Timer Lazy Triggering & Dry Run):** Maintenance runs only via explicit `lore gc [--dry-run]` or lazily piggybacked after write watermark crossings (default: `>= 512` cold-eligible events or `>= 16 MiB` hot segments), never via a background polling timer. All thresholds are configurable and tested via an injected `Clock` trait.

### 3.6 Retrieval, Progressive Disclosure & Local Verification (`RET-*`)
- **RET-1 (Two-Stage Progressive Disclosure):** Default `recall` returns compact `L0` cards (`id`, `kind`, `statement`, `trust_badge`, `verifier_badge`, `scope`, `score`, `has_body`, `has_dead_ends`) budgeted to `<= 60` tokens/card (default limit `8` cards, hard cap `20`). Full `L1` `body`, `dead_ends`, `verifiers`, and lineage are fetched on demand via `inspect(id)` / `lore show <id>`.
- **RET-2 (Local-Side Citation Verification):** Even when claims are fetched from the Mac Host bridge, `Verifier` checks (`FileDigest`, `PathExists`, `SymbolPresent`, `LockfileSemver`) MUST execute locally on the calling machine against its current working tree before formatting the response.
  - Matching digest -> `[verified]`
  - Changed file/line digest or missing symbol -> `[drifted]` (score penalized by `0.35×` and explicitly flagged)
  - Uncheckable on this machine (e.g., remote repo not mounted here) -> `[unverified-local]`
- **RET-3 (Session Exposure Deduplication):** Within a single `session_id`, `recall` records exposures in `claim_exposures`. Claims already surfaced to that session are suppressed from proactive hooks unless the query explicitly requests them or the claim was updated since last surfaced.
- **RET-4 (Code-Precedence Envelope):** All retrieved non-`AXIOM` claims MUST be wrapped in structured evidence framing (e.g., `[lore clm_... | kind: solution | trust: agent | citation: verified]`) accompanied by a concise code-precedence reminder when any `[drifted]` or `agent`-trust claim is present.

### 3.7 Live Cross-Agent Awareness (`AW-*`)
- **AW-1 (Fleet-Wide Live `pulse`):** `pulse` records and queries ephemeral presence rows in `live_presence`: `{ session_id, channel_id?, machine_id, agent_kind, mounted_repos[], intent?, hypothesis?, touched_paths[], recent_failures[], updated_at_ms, ttl_ms }`.
- **AW-2 (TTL Expiry Without Timers):** Presence rows expire automatically when `now > updated_at_ms + ttl_ms` (default TTL: `15 min` for session presence, `30 min` for failure footprints). Expiry is enforced on read and opportunistically pruned on write—zero background timers.
- **AW-3 (Overlap & Conflict Detection):** When `pulse` or `recall` runs, `lore` highlights active peers whose `mounted_repos` + `touched_paths` overlap the caller's current repo/paths, or whose `recent_failures` share a normalized `err:<sig>` signature.
- **AW-4 (Ephemeral vs. Durable Boundary):** `live_presence` is never written to `.pblog` or `.pblog.zst`. When a peer resolves a failure or finishes a handoff, it explicitly promotes the takeaway via `remember` (creating a durable `SOLUTION`, `DEAD_END`, or `HANDOFF` claim).

### 3.8 Fail-Open Hooks (`HOOK-*`)
- **HOOK-1 (`lore hook prompt`):** Triggered on user prompt submission (Turn 0 and subsequent turns). Reads stdin JSON, runs `ContextFrame` + top-K `recall` + overlapping `pulse` peers, suppresses already-surfaced claims for this `session_id`, and emits `<= 350` tokens of `L0` cards + peer overlap summary.
- **HOOK-2 (`lore hook cue`):** Triggered after failing tool/command execution (e.g., non-zero build/test exit). Extracts normalized error signatures (`err:<sig>`) and touched file paths from tool stderr/stdout, queries matching `SOLUTION` / `DEAD_END` / `WARNING` claims and peer `recent_failures`, and emits `<= 250` tokens only when high-confidence matches exist (else empty stdout).
- **HOOK-3 (Hard Latency Cap & Fail-Open Guarantee):** Both hooks enforce a strict timeout (`50 ms` local, `150 ms` over Humata bridge) and MUST exit `0` on timeout, unreachable Host, malformed stdin, or `memory.enabled == false`—never blocking agent execution.

### 3.9 Host-Bridged Multi-Machine & Humata "Bundle Once" Seams (`BR-*`)
- **BR-1 (Backend Abstraction):** `lore_mcp` and `lore` CLI talk to a narrow `LoreBackend` trait with two implementations:
  1. `LocalBackend` — direct `lore_engine` on `~/.lore/` (standalone mode or when running directly on the Mac Host without `humatad`).
  2. `HumataBridgeBackend` — calls local `humatad.sock` (`LoreService` gRPC), which serves locally on the Mac Host and transparently forwards from remote machines to the Mac Host via Humata's existing `ForwardedCall` Uplink.
- **BR-2 (Away / Offline Degradation):** When a remote machine cannot reach the Mac Host (Mac asleep or disconnected), `HumataBridgeBackend`:
  - Returns an explicit `HostUnreachable` status on `recall` / `pulse` (hooks silently degrade and exit `0`).
  - Buffers writes (`remember`, `feedback`) into Humata's existing away-journal (or `~/.lore/away_spool.pblog`) and drains them idempotently (keyed by client-generated `event_id`) upon reconnect.
- **BR-3 (Five Narrow Humata Integration Seams — Milestone `M4` Only):**
  1. `apps/humata2/daemon/machines/src/bundle.rs`: add `InstalledFile::Lore` alongside `InstalledFile::Stack` for SHA-256-verified installation into `~/.humata2/bin/lore` (macOS + `:lore_linux` musl).
  2. `apps/humata2/daemon/svc_lore/`: new narrow leaf crate wrapping `lore_engine` behind Host `memory.enabled`, exposed via `//apps/humata2/api/grpc:lore` (never pulling `daemon/humatad:grpc` umbrella).
  3. **Multi-Machine Bridge & Auto-Provisioning Magic (`daemon/machines` + `ForwardedCall` Uplink):** When any remote machine connects or enrolls via Humata's SSH/gRPC Uplink, Humata automatically pushes `lore` + propagates the current `memory.enabled` state, injects `HUMATA_LORE_BRIDGE=1` / `HUMATA_MACHINE_ID` / `HUMATA_SESSION_REPOS` into remote agent sessions, and routes all `LoreService` RPCs from every remote `humatad` directly to the Mac Host's `svc_lore` over `ForwardedCall` (so machines collaborate zero-config with no per-machine setup).
  4. `apps/humata2/daemon/agent_hooks/src/mcp.rs` & `sessions_core/src/agents.rs`: conditionally wire `lore mcp`, pre-allow `mcp__lore` + `Bash(lore:*)`, and register `lore hook prompt || true` / `lore hook cue || true` iff `memory.enabled == true`.
  5. `apps/humata2/skills/lore/{SKILL.md,lore.md}` & `apps/humata2/features/settings/`: pack the `lore` skill via `//apps/humata2/skills:bundle` when enabled (retired cleanly when disabled), and expose a **Collective Memory (`lore`)** integration toggle button + status readout in the Humata Settings page.

### 3.10 Security, Secret Redaction & Taint Containment (`SEC-*`)
- **SEC-1 (Pre-Write Secret Redaction):** Before appending any `LogEvent` to `.pblog`, `lore_engine` MUST scan `statement`, `body`, `dead_ends`, `attrs`, and inline evidence for high-entropy credential patterns (`ghp_*`, `gho_*`, `sk-*`, `AKIA[0-9A-Z]{16}`, PEM private key headers, `Bearer <token>`, `HUMATA_SESSION_TOKEN` values). Matches either reject with `LoreError::SecretDetected` (default) or redact to `[REDACTED:<kind>]` if `--redact` is passed.
- **SEC-2 (Taint Propagation & Quarantine):** If a session marks itself tainted (or ingests `UNTRUSTED` external content), all subsequent claims written by that session inherit `tainted = true` and `status = QUARANTINED`. Tainted sessions MUST NOT create `AXIOM` claims or supersede `OWNER` claims.
- **SEC-3 (Governance Isolation):** Stored claim payloads MUST NEVER alter `memory.enabled`, `ScoringPolicy` weights, redaction rules, or principal trust tiers.

---

## 4. Schemas & Binary Layouts

### 4.1 Protobuf Schema (`tools/lore/proto/lore.proto`, `package lore.v1`)

```protobuf
syntax = "proto3";
package lore.v1;

enum ClaimKind {
  CLAIM_KIND_UNSPECIFIED = 0;
  CLAIM_KIND_AXIOM = 1;
  CLAIM_KIND_FACT = 2;
  CLAIM_KIND_DECISION = 3;
  CLAIM_KIND_SOLUTION = 4;
  CLAIM_KIND_DEAD_END = 5;
  CLAIM_KIND_WARNING = 6;
  CLAIM_KIND_PREFERENCE = 7;
  CLAIM_KIND_QUESTION = 8;
  CLAIM_KIND_HANDOFF = 9;
}

enum ClaimStatus {
  CLAIM_STATUS_UNSPECIFIED = 0;
  CLAIM_STATUS_ACTIVE = 1;
  CLAIM_STATUS_SUPERSEDED = 2;
  CLAIM_STATUS_INVALIDATED = 3;
  CLAIM_STATUS_RETRACTED = 4;
  CLAIM_STATUS_EXPIRED = 5;
  CLAIM_STATUS_QUARANTINED = 6;
}

enum PredicateOp {
  PREDICATE_OP_UNSPECIFIED = 0;
  PREDICATE_OP_EQ = 1;
  PREDICATE_OP_IN = 2;
  PREDICATE_OP_GLOB = 3;
  PREDICATE_OP_SEMVER_REQ = 4;
  PREDICATE_OP_ANCESTOR_OF = 5;
  PREDICATE_OP_NOT_ANCESTOR_OF = 6;
}

enum EdgeRel {
  EDGE_REL_UNSPECIFIED = 0;
  EDGE_REL_SUPERSEDES = 1;
  EDGE_REL_CONTRADICTS = 2;
  EDGE_REL_SUPPORTS = 3;
  EDGE_REL_DERIVED_FROM = 4;
  EDGE_REL_ANSWERS = 5;
  EDGE_REL_CONTRASTS_WITH = 6;
}

enum PrincipalKind {
  PRINCIPAL_KIND_UNSPECIFIED = 0;
  PRINCIPAL_KIND_OWNER = 1;
  PRINCIPAL_KIND_AGENT = 2;
  PRINCIPAL_KIND_IMPORT = 3;
}

enum OutcomeKind {
  OUTCOME_KIND_UNSPECIFIED = 0;
  OUTCOME_KIND_HELPFUL = 1;
  OUTCOME_KIND_HARMFUL = 2;
  OUTCOME_KIND_NOT_APPLICABLE = 3;
  OUTCOME_KIND_CORROBORATED = 4;
  OUTCOME_KIND_DISPUTED = 5;
}

// Tight, closed language & ecosystem enums (no unbounded free-form strings).
// Polyglot and unrecognized ecosystems map explicitly to POLYGLOT / OTHER / UNSPECIFIED.
enum Language {
  LANGUAGE_UNSPECIFIED = 0; // unknown
  LANGUAGE_RUST = 1;
  LANGUAGE_GO = 2;
  LANGUAGE_PYTHON = 3;
  LANGUAGE_JAVA = 4;
  LANGUAGE_BAZEL = 5;
  LANGUAGE_STARLARK = 6;
  LANGUAGE_SWIFT = 7;
  LANGUAGE_POLYGLOT = 8;
  LANGUAGE_OTHER = 9;
}

enum Ecosystem {
  ECOSYSTEM_UNSPECIFIED = 0; // unknown
  ECOSYSTEM_CARGO = 1;       // Rust: Cargo.lock / Cargo.toml (`prost`)
  ECOSYSTEM_GO = 2;          // Go: go.mod / go.sum (`github.com/spf13/cobra`)
  ECOSYSTEM_PYTHON = 3;      // Python: uv.lock / poetry.lock / requirements.txt / pyproject.toml (PEP 503 name)
  ECOSYSTEM_MAVEN = 4;       // Java: pom.xml / gradle.lockfile / libs.versions.toml (`groupId:artifactId`)
  ECOSYSTEM_BAZEL = 5;       // Bazel/Bzlmod: MODULE.bazel.lock / MODULE.bazel (`rules_rust`)
  ECOSYSTEM_SWIFT = 6;       // SwiftPM: Package.resolved / Package.swift (`swift-nio`)
  ECOSYSTEM_POLYGLOT = 7;
  ECOSYSTEM_OTHER = 8;
}

enum LockfileKind {
  LOCKFILE_KIND_UNSPECIFIED = 0;
  LOCKFILE_KIND_CARGO_LOCK = 1;
  LOCKFILE_KIND_GO_MOD = 2;
  LOCKFILE_KIND_PY_UV_LOCK = 3;
  LOCKFILE_KIND_PY_POETRY_LOCK = 4;
  LOCKFILE_KIND_PY_REQUIREMENTS = 5;
  LOCKFILE_KIND_JAVA_POM = 6;
  LOCKFILE_KIND_JAVA_GRADLE_LOCK = 7;
  LOCKFILE_KIND_BAZEL_MODULE_LOCK = 8;
  LOCKFILE_KIND_SWIFT_PACKAGE_RESOLVED = 9;
  LOCKFILE_KIND_OTHER = 10;
}

message Predicate {
  string axis = 1;       // e.g. "os", "arch", "machine", "agent", "branch_glob", "dep:cargo/prost"
  PredicateOp op = 2;
  string value = 3;      // comma-separated for IN
}

message Principal {
  PrincipalKind kind = 1;
  string id = 2;
  string session_id = 3;
  string agent_kind = 4; // claude | agy | codex | copilot | opencode | human
  string model = 5;
  string machine_id = 6;
}

message OriginSnapshot {
  string machine_id = 1;
  string repo_id = 2;
  string branch = 3;
  string head_commit = 4;
  string os = 5;
  string arch = 6;
}

message Verifier {
  oneof check {
    FileDigestCheck file_digest = 1;
    PathExistsCheck path_exists = 2;
    SymbolPresentCheck symbol_present = 3;
    LockfileSemverCheck lockfile_semver = 4;
    BlobRef blob_ref = 5;
  }
}

message FileDigestCheck {
  string repo_id = 1;
  string path = 2;        // repo-relative path
  uint32 start_line = 3;  // 0 = whole file
  uint32 end_line = 4;
  string sha256 = 5;
}

message PathExistsCheck {
  string repo_id = 1;
  string path = 2;
  bool expect_exists = 3;
}

message SymbolPresentCheck {
  string repo_id = 1;
  string path = 2;
  string symbol = 3;
  Language language = 4;
}

message LockfileSemverCheck {
  Ecosystem ecosystem = 1;
  LockfileKind lockfile_kind = 2; // optional; UNSPECIFIED = auto-detect ecosystem lockfile
  string package_name = 3;        // canonical notation per ecosystem (e.g. `groupId:artifactId` for Maven)
  string semver_req = 4;
}

message BlobRef {
  string sha256 = 1;
  string role = 2;        // stderr | diff | trace
  string summary = 3;
}

message Claim {
  // Facet 1: Identity & Lifecycle
  string id = 1;                 // clm_<ts_hex>_<rand_hex>
  ClaimKind kind = 2;
  ClaimStatus status = 3;
  int64 created_at_ms = 4;
  int64 updated_at_ms = 5;
  int64 valid_from_ms = 6;       // 0 = unset
  int64 valid_until_ms = 7;      // 0 = unset
  int64 review_after_ms = 8;     // 0 = unset

  // Facet 2: Progressive Content
  string statement = 10;         // <= 280 UTF-8 chars (L0 card)
  string body = 11;              // optional L1 markdown
  repeated string dead_ends = 12;
  repeated string keywords = 13;
  map<string, string> attrs = 14;

  // Facet 3: Scope vs. Entities
  string scope = 20;             // global | channel:<id> | repo:<id> | machine:<id> | dir:<m>:<h> | session:<id>
  repeated string about = 21;    // typed <ns>:<key> URIs

  // Facet 4: Applicability
  repeated Predicate applies_when = 30;

  // Facet 5: Provenance, Taint & Verifiers
  Principal author = 40;
  bool tainted = 41;
  OriginSnapshot origin = 42;
  repeated Verifier verifiers = 43;

  // Facet 6: Raw Signals
  bool pinned = 50;
  float importance = 51;         // default 1.0
}

message LogEvent {
  uint64 seq = 1;
  string event_id = 2;           // client idempotency key for away-spool replay
  int64 recorded_at_ms = 3;
  Principal actor = 4;
  oneof payload {
    ClaimUpserted claim_upserted = 10;
    ClaimTransitioned claim_transitioned = 11;
    EdgeAdded edge_added = 12;
    FeedbackRecorded feedback_recorded = 13;
    ClaimForgotten claim_forgotten = 14;
  }
}

message ClaimUpserted { Claim claim = 1; }
message ClaimTransitioned { string claim_id = 1; ClaimStatus new_status = 2; string reason = 3; }
message EdgeAdded { string src_id = 1; EdgeRel rel = 2; string dst_id = 3; string note = 4; }
message FeedbackRecorded { string claim_id = 1; OutcomeKind outcome = 2; string note = 3; }
message ClaimForgotten { string claim_id = 1; string reason = 2; }
```

### 4.2 Binary On-Disk Frame Layouts

#### Hot Segment (`hot/seg-<base_seq_20d>.pblog`)
```text
File Header (16 bytes):
  [0..8]   magic         = b"\x89LORE\r\n\x1a"
  [8..10]  format_ver    = u16 LE (1)
  [10]     checksum_alg  = u8 (1 = crc32fast IEEE, 2 = crc32c reserved, 3 = xxh3_64 reserved)
  [11]     flags         = u8 (0 = raw uncompressed)
  [12..16] header_crc32  = u32 LE over bytes [0..12]

Record Frame (16 + payload_len bytes):
  [0..4]   payload_len   = u32 LE (max 4 MiB)
  [4..8]   checksum      = u32 LE (crc32fast over seq_le_bytes || payload)
  [8..16]  seq           = u64 LE (strictly monotonic)
  [16..]   payload       = prost::Message::encode(LogEvent)
```

#### Cold Archive (`archive/arch-<min_seq>-<max_seq>.pblog.zst`)
```text
Archive File Header (16 bytes):
  [0..8]   magic         = b"\x89LORZ\r\n\x1a"
  [8..10]  format_ver    = u16 LE (1)
  [10]     codec         = u8 (1 = zstd w/ native XXH64 frame checksum)
  [11]     flags         = u8 (0)
  [12..16] header_crc32  = u32 LE over bytes [0..12]

Archive Body:
  1..N independent seekable zstd frames (256 KiB..4 MiB uncompressed each, level 6, ZSTD_c_checksumFlag=1).
  Each decompressed frame contains standard Record Frames (above), indexed by (archive_seg, frame_offset, record_offset_in_frame) in SQLite cold stubs.
```

### 4.3 SQLite Projection (`index.db`, WAL Mode + `auto_vacuum = INCREMENTAL`)
- `meta(key TEXT PRIMARY KEY, value TEXT NOT NULL)` — `applied_seq`, `projection_version`, `last_gc_ms`.
- `claims(id TEXT PRIMARY KEY, kind INT, status INT, tier INT, scope TEXT, statement TEXT, body TEXT, dead_ends_json TEXT, keywords_text TEXT, attrs_json TEXT, applies_when_json TEXT, author_json TEXT, origin_json TEXT, verifiers_json TEXT, tainted INT, pinned INT, importance REAL, corroborator_count INT, dispute_count INT, surfaced_count INT, helpful_count INT, harmful_count INT, not_applicable_count INT, created_at_ms INT, updated_at_ms INT, valid_from_ms INT, valid_until_ms INT, last_surfaced_at_ms INT, last_validated_at_ms INT, archive_seg TEXT, archive_frame_off INT, archive_rec_off INT, archive_rec_len INT)`.
  - For `tier = COLD (1)`: `body`, `dead_ends_json`, `attrs_json`, `verifiers_json`, `origin_json` are `NULL` to minimize DB size.
- `claims_fts` — `VIRTUAL TABLE claims_fts USING fts5(claim_id UNINDEXED, statement, keywords_about, dead_ends, body, tokenize='unicode61')` (Tier-0 Hot rows only).
- `claim_entities(entity_uri TEXT NOT NULL, claim_id TEXT NOT NULL, PRIMARY KEY (entity_uri, claim_id)) WITHOUT ROWID`.
- `claim_edges(src_id TEXT NOT NULL, rel INT NOT NULL, dst_id TEXT NOT NULL, created_at_ms INT NOT NULL, author_id TEXT NOT NULL, note TEXT, PRIMARY KEY (src_id, rel, dst_id)) WITHOUT ROWID`.
- `claim_corroborators(claim_id TEXT NOT NULL, session_id TEXT NOT NULL, machine_id TEXT NOT NULL, PRIMARY KEY (claim_id, session_id, machine_id)) WITHOUT ROWID`.
- `claim_exposures(session_id TEXT NOT NULL, claim_id TEXT NOT NULL, surfaced_at_ms INT NOT NULL, outcome INT, PRIMARY KEY (session_id, claim_id)) WITHOUT ROWID`.
- `live_presence(session_id TEXT PRIMARY KEY, channel_id TEXT, machine_id TEXT NOT NULL, agent_kind TEXT NOT NULL, mounted_repos_json TEXT NOT NULL, intent TEXT, hypothesis TEXT, touched_paths_json TEXT NOT NULL, recent_failures_json TEXT NOT NULL, updated_at_ms INT NOT NULL, ttl_ms INT NOT NULL)`.

---

## 5. Interfaces: MCP, CLI, Hooks & Error Types

### 5.1 MCP Surface (5 Tools, `lore mcp` over stdio JSON-RPC)
1. **`recall`** — `{ query?, about?, kinds?, scope?, limit? (default 8, max 20), include_history? (bool), include_pulse? (bool, default true) }` $\rightarrow$ `{ enabled, context_frame, cards: [L0Card], peers: [PeerSummary], notice? }`.
2. **`inspect`** — `{ id, include_lineage? (bool) }` $\rightarrow$ `{ enabled, claim: FullL1Claim, edges: [Edge], verifier_results: [VerifierResult], hydrated_from_cold: bool }`.
3. **`remember`** — `{ kind, statement, body?, dead_ends?, scope?, about?, applies_when?, verifiers?, supersedes?, pinned?, importance?, redact_secrets? }` $\rightarrow$ `{ enabled, id, status, superseded_id?, redacted_fields? }`.
4. **`feedback`** — `{ id, outcome (helpful|harmful|not_applicable|corroborated|disputed|invalidate|retract), note? }` $\rightarrow$ `{ enabled, id, new_status, counters }`.
5. **`pulse`** — `{ intent?, hypothesis?, touched_paths?, recent_failures?, ttl_ms?, query_only? }` $\rightarrow$ `{ enabled, peers: [PeerSummary], overlaps: [PathOrErrorOverlap] }`.

### 5.2 CLI Surface (`lore`)
- `lore recall [query] [--about ...] [--kind ...] [--include-history] [--json]`
- `lore show <id> [--lineage] [--json]`
- `lore remember --kind <k> <statement> [--body ...] [--dead-end ...] [--about ...] [--supersedes <id>] [--pin]`
- `lore feedback <id> <helpful|harmful|corroborate|dispute|invalidate|retract> [--note ...]`
- `lore forget <id> --reason <text>`
- `lore pulse [--intent ...] [--touch ...] [--fail ...]`
- `lore status [--json]` (shows `memory.enabled`, mode `local|bridge`, hot/cold counts, disk bytes, last GC)
- `lore config get|set memory.enabled [true|false]`
- `lore gc [--dry-run] [--json]`
- `lore reindex` / `lore verify`
- `lore log [--jsonl] [--since-seq N]` / `lore export --jsonl`
- `lore mcp`
- `lore hook prompt` / `lore hook cue`
- `lore guide` (prints compact agent usage cheat-sheet)

### 5.3 Explicit Error Types (`thiserror`, Actionable Messages)
No `anyhow::Error` or `Box<dyn Error>` in public/crate APIs. `LoreError` variants carry structured fields, `#[source]`, and tell the user *what failed*, *how to fix it*, and *when an obvious fix is unsafe*:
- `Disabled { reason: Option<String> }` — *"lore is intentionally disabled (`memory.enabled = false`){reason_suffix}. Do NOT attempt to re-enable, debug, or work around lore; proceed with your task without it. (Human owner: re-enable via `lore config set memory.enabled true` or Humata Settings; existing data is untouched.)"*
- `StatementTooLong { len, max }` — *"Claim statement is {len} chars (max {max}). Move details into `body` and keep `statement` to a single headline."*
- `SecretDetected { pattern_kind, field }` — *"Refused to store potential {pattern_kind} secret in `{field}`. Remove the secret before recording; only pass `--redact` if you have verified the surrounding text contains no other credentials."*
- `TaintedPromotionDenied { claim_id, attempted }` — *"Tainted session cannot perform {attempted}. Confirm the claim from a clean session or as the owner before promoting."*
- `CorruptSegment { path, offset, seq, source }` — *"Checksum mismatch in {path} at byte {offset} (seq {seq}). Do NOT delete the log; run `lore verify` to inspect damaged frames or restore from backup before running `lore reindex`."*
- `ArchiveHydrationFailed { claim_id, segment, source }` — *"Failed to decompress cold archive {segment} for {claim_id}. Run `lore verify --archives` to check integrity."*
- `HostUnreachable { endpoint, buffered, source }` — *"Mac Host bridge ({endpoint}) is unreachable; write buffered={buffered}. Ensure Humata is running on the Host; buffered writes will drain automatically on reconnect."*
- `ClaimNotFound { id }` / `InvalidScope { scope }` / `InvalidPredicate { axis, op, value }` / `LockContention { path, timeout_ms }` / `Io { path, source }` / `Sqlite { op, source }`.

---

## 6. Milestones (Isolated First, Humata Last)

| Milestone | Scope | Bazel Targets | Exit Gate |
| :--- | :--- | :--- | :--- |
| **M0: Proto & Context** | `lore_proto` (`prost` + `serde`, `.pblog` frame codec) + `lore_context` (`0`/`1`/`N`-repo discovery, sandboxed `gitdir:` fallback, local verifiers). | `//tools/lore/proto/...`, `//tools/lore/context/...` | `<2 ms` context detection; unit tests pass for `0`/`1`/`N` repos, sandboxed worktrees, and local `FileDigest`/`SymbolPresent` checks. |
| **M1: Engine & Storage** | `lore_engine`: `.pblog` append + torn-tail recovery, SQLite WAL/FTS5 projection, `lore reindex`, `ScoringPolicy`, secret redaction, taint quarantine, `live_presence`. | `//tools/lore/engine/...` | `<3 ms` hot writes; deterministic torn-tail recovery; `reindex` parity; secret/taint enforcement; `Clock`-injected decay tests. |
| **M2: Cold `zstd` & GC** | `(status, age)` Tier-1 `zstd` archival + on-demand hydration + re-promotion, Tier-2 GC purge, blob mark-and-sweep, immediate `forget(id)` across hot + cold. | `//tools/lore/engine/...` | Space reduction verified on synthetic corpus; cold hydration `<5 ms`; `forget` leaves zero payload bytes in `.pblog`, `.pblog.zst`, `blobs/`, and `index.db`. |
| **M3: MCP, Hooks & CLI** | `lore_mcp` (5 tools + `prompt`/`cue` hooks + `LocalBackend`), `lore` CLI + `:lore_linux` static musl build, `memory.enabled` global switch. | `//tools/lore/...` | End-to-end stdio MCP + CLI + fail-open hook tests pass with full `tools/bq/bq //tools/lore/...` (clippy/lint clean). |
| **M4: Humata Bundle, Settings UI & Multi-Machine Bridge** | 5 narrow seams: `InstalledFile::Lore`, `svc_lore` + `//apps/humata2/api/grpc:lore`, zero-config multi-machine `ForwardedCall` + env auto-provisioning, `agent_hooks`/`sessions_core` wiring, `skills/lore`, and Settings page `lore` toggle button. | `//apps/humata2/daemon/svc_lore/...` + `//apps/humata2/features/settings/...` + narrow Humata tests | Remote machine auto-provisions `lore`, writes to Mac Host via `ForwardedCall`, verifies citations locally, buffers on sleep, and toggling the Settings button (`memory.enabled = false`) drops session prompt overhead to 0 tokens. |

---

## 7. Behavioral Verification Matrix

All tests are in-process behavioral tests (never asserting on internal call order or exact prose wording), using an injected deterministic `Clock` trait, **zero `sleep`s**, and **zero polling loops**.

| Req Group | Key Behavioral Test Cases |
| :--- | :--- |
| **`SW-*`** | Toggle `memory.enabled` `true -> false -> true`: data persists intact; MCP returns `{ enabled: false }`; hooks print nothing and exit `0`; Humata session builder emits 0 MCP/hook/skill entries when `false`. |
| **`TOP-*`** | Fixture dirs for (a) single repo with `origin`, (b) repo without `origin` (`git:<root_sha>`), (c) sandboxed `.git` `gitdir:` file with unreadable target (`chmod 000` / invalid path), (d) parent dir with 2 child repos, (e) non-git dir. Verify scope resolution, path normalization, and branch-agnostic recall vs. branch-aware `pulse`. |
| **`MOD-*`** | Supersession atomically hides old claim from default recall and creates `SUPERSEDES` edge; unknown predicate axis yields `UNKNOWN` without failing; `ScoringPolicy` ranks verified/corroborated/entity-matched claims above stale/harmful ones; duplicate corroboration from same `(session, machine)` does not inflate count. |
| **`ST-*`** | Append 1,000 events, truncate final frame mid-payload or flip 1 bit in tail -> startup truncates cleanly to `seq 999`; flip 1 bit in middle frame -> returns `LoreError::CorruptSegment`; delete `index.db` and run `reindex` -> identical query results; `lore log --jsonl` round-trips all events. |
| **`GC-*`** | Advance injected `Clock` by 8d / 15d / 31d / 91d: active `AXIOM`/`DECISION` and `pinned` claims stay Tier-0 Hot; `superseded > 7d` and unpinned dormant `> 90d` move to `.pblog.zst` with 64-byte stubs; `inspect` hydrates from cold; corroborating a cold claim promotes it back to Tier-0; `handoff > 14d`, `retracted > 30d`, `quarantined > 14d` purge; `forget(id)` removes bytes from both `.pblog` and `.pblog.zst` and sweeps orphan blobs. |
| **`RET-*`** | Mutate a cited file on disk -> local verifier flips badge `[verified] -> [drifted]` and demotes rank; second `recall` in same `session_id` suppresses already-surfaced claim in hook mode; `L0` cards stay within token budget and wrap non-axioms in evidence envelopes. |
| **`AW-*`** | Two sessions emit `pulse` with overlapping `touched_paths` and matching `err:<sig>` -> both see overlap warning; advancing `Clock` past `ttl_ms` hides expired peer without any background task. |
| **`HOOK-*`** | Feed malformed JSON, unreachable socket, and simulated slow backend -> `lore hook prompt` and `lore hook cue` both exit `0` within timeout budget with valid or empty stdout. |
| **`BR-*`** | Fake Host bridge routes remote writes to Host store while verifying citations against remote's temp workspace; disconnecting bridge spools writes to away-spool and drains idempotently on reconnect with zero duplicate claims. |
| **`SEC-*`** | Writing `ghp_...` / `sk-...` / `AKIA...` / PEM key returns `LoreError::SecretDetected` (or redacts with `--redact`); tainted session writes land in `QUARANTINED` and cannot create `AXIOM` or supersede `OWNER` claims. |
