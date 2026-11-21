# 03: Master Prompt to Author the Intent Spec (`lore`)

Copy and paste the prompt block below into a new planning / spec session (e.g. with `/plan` or `/grill-me`) to produce the formal, testable Intent Spec (`SPEC.md` + `TRACE.md`) before implementation begins.

---

````markdown
Write a rigorous, testable Intent Specification (`tools/lore/SPEC.md`) for **`lore`** — our collective long-term epistemic memory and live cross-agent awareness system for AI coding agents, designed to live in isolated new Rust crates under `//tools/lore/...` and bundle once with Humata 2.

Before drafting, read these two reference documents in full:
1. `tools/lore/docs/01-RESEARCH-AND-LANDSCAPE.md` (teardown of `mingley/hivemind` and `ldclabs/KIP` + `Cognitive Nexus`, empirical findings from `VibeMemBench` / `MemDev-Bench` / `STALE`, and why naive RAG and static `AGENTS.md` files fail).
2. `tools/lore/docs/02-ARCHITECTURE-AND-SCHEMA.md` (crate isolation, Host-bridged topology, global switch, `0`/`1`/`N`-repo workspace model, framed Protobuf + SQLite WAL/FTS5 storage, `(status, age)` `zstd` compression & GC, canonical 6-facet metadata schema, and read-time scoring).
Also respect all repository rules in `AGENTS.md` (Bazel-only via `tools/bq/bq`, explicit `thiserror` error types with actionable user-facing fixes, minimal visibility, behavior-driven tests with zero sleeps/timers, and strict crate cache boundaries).

### Do NOT write implementation code yet. Produce the Intent Spec only.

---

## Core Intent & Architectural Decisions Already Locked In

1. **Goal:**
   - Stop repeating claims, preferences, architectural decisions, environment quirks, debugging root causes, and dead ends from session to session and environment to environment.
   - Make concurrent and sequential agents (Claude Code, Antigravity `agy`, Codex, Copilot, OpenCode) both **learn from each other** (durable epistemic memory) and **aware of each other** (live peer presence, branches, touched paths, working hypotheses, recent build/test failures).

2. **Isolated Crates & Fast Iteration (`//tools/lore/...`):**
   - Structure as narrow, fast-compiling Rust crates under `tools/lore/` (`lore_proto`, `lore_context`, `lore_engine`, `lore_mcp`, `lore` CLI binary + `:lore_linux` static musl cross-builds, modeled on `//tools/stack`).
   - **Zero dependencies** on `apps/humata2/ui`, `desktop`, `clients`, or `humatad_core` inside `//tools/lore/...` so `tools/bq/bq --fast //tools/lore/...` compiles and tests in ~1–2 seconds without conflicting with or rebuilding the rest of the monorepo.
   - Reuse existing workspace dependencies from root `Cargo.toml` (`prost`, `rusqlite` bundled w/ FTS5, `zstd`, `crc32fast`, `sha2`, `clap`, `serde`, `serde_json`, `thiserror`, `versions`, `regex`, `rustix`, `uuid`) before proposing any new crate.

3. **Delivery Shape & "Bundle Once with Humata" Topology:**
   - Surface is **just an MCP server + CLI + bundled agent skill (+ two lightweight `<3 ms` fail-open hooks)**.
   - **Host-as-Single-Brain (No Cross-Machine File Sync):** Inside Humata, the authoritative store lives on the Mac Host (which runs the desktop app and bridges all machines). Agents on any linked machine (Mac, Linux VMs, remote SSH hosts) write to the Host and query across all machines over Humata's existing authenticated SSH/gRPC `ForwardedCall` bridge (with away-journal / fail-open behavior when the Host is unreachable). Outside Humata, `lore` falls back cleanly to a standalone local `~/.lore/` store.
   - **Split Read Verification:** The Host evaluates scopes, `applies_when`, and BM25/entity ranking; the thin client on the calling machine verifies file-hash citations, path/symbol existence, commit ancestry, and lockfile versions against *its own* local checkout in `<1 ms` before returning results.
   - **Bundle Once Touchpoints (spec as a narrow integration boundary):** `InstalledFile::Lore` in `daemon/machines`, a thin `svc_lore` Host adapter, MCP/hook injection in `agent_hooks` + `sessions_core`, and one `apps/humata2/skills/lore/` skill folder unpacked by `own_skills.rs`.

4. **Single Global On/Off Kill-Switch:**
   - One global switch (`memory.enabled`, togglable via CLI and Host settings) turns the entire system `ON` or `OFF` fleet-wide.
   - **When `OFF`:** Stored logs/claims/blobs on disk are preserved untouched; in-flight MCP/CLI/hook calls immediately short-circuit (`enabled: false`, empty/no-op); newly launched sessions omit `lore` MCP, hooks, and skills (`0` prompt/schema token overhead).

5. **Workspace Topology (`0`, `1`, or `N` Git Repos) & Branch Rules:**
   - Must work identically:
     1. **Inside a single git repo or git worktree** (canonicalizing `repo_id` from normalized `remote.origin.url`, falling back to `git:<root-commit-sha>` for unpushed local repos — **and** handling sandboxed git worktrees where `.git` is a `gitdir:` file pointing outside the sandbox so `git` CLI exits 128/`EPERM`, by parsing the `.git` pointer / Humata session metadata directly without failing),
     2. **Inside a parent folder containing multiple child git repos** (e.g. Humata multi-repo session folders `session/{api, web}` or `~/oss` — unioning mounted child `repo:<id>` scopes on read and inferring repo-relative citations on write),
     3. **Completely outside any git repo** (Humata Desk sessions, `~`, ops folders — falling back cleanly to `global`, `machine:<id>`, `channel:<id>`, and `dir:<machine>:<cwd>` scopes).
   - **Branches:** Durable claims are **branch-agnostic by default** (scoped to `repo:<id>`, recording `branch` and `head_commit` strictly as informational provenance). Code divergence across branches is detected via local file-hash citations and optional `since_commit`/`until_commit` ancestry checks—never by trapping learnings on ephemeral feature branch names. Live peer `pulse` *is* branch-aware, and `applies_when` keeps an opt-in `branch_glob` only for explicit release-branch policies.

6. **Storage Format, Hot/Cold `(status, age)` `zstd` Compression & Garbage Collection:**
   - **Canonical Log Format:** Length-delimited, CRC32-checked **Protobuf records (`.pblog` via `prost`)**, *not* JSONL, for compact size (~3× smaller), fast decoding, exact power-cut torn-tail recovery, and tag-numbered schema evolution—paired with `lore log --jsonl` / `lore export --jsonl` for human CLI inspection.
   - **Disposable Local Projection:** SQLite WAL (`index.db`) with code-aware **FTS5** (`camelCase`, `snake_case`, `::`, `/`, error codes), entity prefix index, typed edges, exposure dedup table, and live fleet presence. 100% rebuildable from `.pblog` + `.pblog.zst`.
   - **Zero Hot-Path Compression Tax:** New and active hot claims are written and queried **uncompressed**.
   - **Cold `zstd` Archival & GC (`lore gc` + watermark trigger, driven by `status`, `age`, or `status + age`):**
     - *Cold Archive (`zstd` bulk frames `.pblog.zst`):* Compress and evict from hot FTS5 items that are `superseded`/`invalidated` older than $T_{\text{closed}}$ (e.g. 7d) or unpinned dormant items unvalidated for $> T_{\text{dormant}}$ (e.g. 90d), keeping lightweight stubs in SQLite for instant on-demand hydration (`lore show`, `--include-history`) and automatic re-promotion to Hot if re-validated.
     - *Garbage Collection (Purge):* Reclaim closed `handoff`s $> 14\text{d}$, `retracted` mistakes $> 30\text{d}$, uncorroborated `quarantined` items $> 14\text{d}$, fold raw `claim_exposures` $> 7\text{d}$ into aggregate claim counters, mark-and-sweep orphaned SHA-256 evidence blobs, and execute immediate segment rewrite + blob purge on `forget(id)`.

7. **Metadata Foundation & Extensible Weighting:**
   - Specify the 6 orthogonal claim facets: **(1)** Identity & Lifecycle (`axiom`, `fact`, `decision`, `solution`, `dead_end`, `warning`, `preference`, `question`, `handoff`; 5 lifecycle transitions `corroborate`, `supersede`, `invalidate`, `retract`, `forget`), **(2)** Progressive Content (`L0` `statement` $\le 280$ chars, `L1` `body`, `dead_ends[]`, `keywords[]`, extensible `attrs` map), **(3)** Visibility `scope` vs. `about[]` `<ns>:<key>` entity URIs, **(4)** Extensible `applies_when[]` `{axis, op, value}` predicates, **(5)** Provenance, session taint propagation & deterministic local `verifiers[]`, **(6)** Raw signal counters (`pinned`, `importance`, `corroborator_count`, `dispute_count`, `surfaced_count`, `helpful_count`, `harmful_count`, `last_validated_at`) feeding a pluggable read-time `ScoringPolicy` (never baking a frozen float into stored claims).

8. **Ambient Capture & Reactive Recall Surfaces:**
   - **5 MCP tools:** `pulse`, `remember`, `recall`, `show`, `resolve` (returning near-duplicates on `remember` so callers corroborate/supersede instead of duplicating).
   - **CLI (`lore`):** Matching commands + `on|off|status`, `peers`, `gc [--dry-run]`, `log --jsonl`, `doctor`, `wire`, `guide`, `--json`, and actionable `Next:` error recovery hints.
   - **2 Fail-Open `<3 ms` Hooks:** `lore hook prompt` (Turn-0 fleet `pulse` + scoped `owner` axioms + 1-line nudge when user prompt states a correction/directive) and `lore hook cue` (post-tool match on non-zero command `stderr` signatures or guarded file paths, deduplicated per session).

---

## What the Intent Spec (`tools/lore/SPEC.md`) Must Contain

1. **Numbered, Testable Requirements** grouped by domain (`SW-*` Global Switch, `TOP-*` Workspace/Repo/Branch Topology, `MOD-*` Epistemic Model & Taint/Trust, `ST-*` `.pblog` Durability & Crash Recovery, `GC-*` `(status, age)` `zstd` Archival & Garbage Collection, `RET-*` Progressive Retrieval, Verification & Scoring, `AW-*` Live Fleet Presence & Stigmergy, `HOOK-*` Ambient Capture & Cue Recall, `BR-*` Humata Host Bridge & Offline Behavior, `SEC-*` Secret Redaction & Boundaries).
2. **Exact Protobuf Schema Draft** (with field numbers and evolution rules) + `.pblog` / `.pblog.zst` binary frame layouts.
3. **Exact CLI & MCP Contracts** (commands, flags, exit codes, JSON envelopes, refusal/error kinds with user-facing `fix` guidance).
4. **Capability & Milestone Breakdown (`M0`..`M4`)** ordered so `M0`–`M3` build and verify 100% inside `//tools/lore/...` with fast in-process deterministic tests (injected `Clock`, power-cut/torn-frame tests, zero `sleep`s, zero idle timers), and `M4` wires the thin Humata Host bridge + bundle + skill.
5. **Verification & Falsification Matrix** mapping every requirement ID to a concrete behavioral test (never testing message wording or private internals) plus retrieval/token-overhead benchmarks.
````
