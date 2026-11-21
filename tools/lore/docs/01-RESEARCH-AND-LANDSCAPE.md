# 01: Collective Agent Memory & Awareness — Frontier Research & Landscape

Status: Research baseline (2026-11-21). Companion architecture: [02-ARCHITECTURE-AND-SCHEMA.md](02-ARCHITECTURE-AND-SCHEMA.md). Spec prompt: [03-INTENT-SPEC-PROMPT.md](03-INTENT-SPEC-PROMPT.md).

---

## 1. Problem Statement

AI coding agents (Claude Code, Antigravity/`agy`, Codex, Copilot, OpenCode) suffer from two compounding forms of amnesia:

1. **Diachronic Amnesia (Session-to-Session & Environment-to-Environment):** Every new session starts blank, forcing the owner to repeat standing rules (*"run builds via `tools/bq/bq`, never bare `bazel` or `cargo`"*), architectural decisions, environment quirks (*"Lima VM `h2-terminal-c1` runs real OpenSSH"*), and hard-won debugging solutions/dead ends whenever switching sessions, tools, branches, or machines.
2. **Synchronic Blindness (Agent-to-Agent):** Concurrent sessions running across terminals, worktrees, or remote VMs have no ambient awareness of who else is active, which files/targets peers are touching, what hypotheses peers are testing, or which build failures peers hit two minutes ago.

---

## 2. Empirical Findings from 2025–2026 Coding-Agent Benchmarks

Independent evaluations overturned several common assumptions about agent memory:

* **Naive RAG / LLM-Extracted Vector Stores Frequently Hurt (`VibeMemBench`, `MemDev-Bench`):**
  * Across 29+ memory configurations tested on real multi-commit repository histories, **11 of 12 solver-memory pairings failed to beat a no-memory baseline**, and plain full-text lexical search (`50.0%`) outperformed most vector + LLM-summary pipelines (`46–53%`).
  * **Root cause:** Unfiltered historical chunks inject plausible-looking facts from older commits or different environments, anchoring agents on wrong files or obsolete APIs.
* **The Stale-Instruction Override Trap (`90.1%` Failure Rate):**
  * When an always-loaded instruction file (`AGENTS.md`, `CLAUDE.md`, `MEMORY.md`) contradicts the current code, coding agents follow the stale text **over the actual code 90.1% of the time**.
  * Delivering non-axiom memories as **provenance-tagged, code-cited evidence** with explicit code-precedence framing flips code adherence from **4.2% to 91.4%**.
* **Implicit State Transitions Break Append-Only Vector Stores (`STALE`, `StateMemBench`):**
  * Standard RAG systems return the current truth **<20% of the time** after a decision changes without repeating the exact negation of the old statement. Explicit lifecycle edges (`supersedes`, `invalidates`, `retracts`) solve ~80%+ of conflict cases deterministically.
* **Always-Loaded Prompt Tax:**
  * Accumulating every project note inside always-loaded markdown files adds **>20% token overhead** to every turn—including tasks that never touch those subsystems—while diluting attention on core invariants.

---

## 3. Deep Teardowns of Reference Systems

### 3.1 `mingley/hivemind` (Rust, MIT)
* **What it is:** A single-node Rust shared memory engine (`hivemind` CLI/MCP shim + `hivemindd` Unix-socket daemon + custom segmented `.hlog` + BLAKE3 blob store + `hivebench` harness) framed as a "shared computational hippocampus."
* **What it gets right:**
  * Models atomic **Claims** (`fact`, `decision`, `preference`, `procedure`, `solution`, `warning`, `question`, `handoff`) instead of chat chunks.
  * **Version-space applicability (`applies_when`):** Predicates such as `os=macos` or `dep:cargo/openssl-sys=>=0.9.100, <0.10` evaluated at read time alongside bitemporal (`valid_time`, `record_time`) metadata.
  * **Five distinct lifecycle endings:** `supersede`, `invalidate`, `retract`, `expire`, `forget`.
  * **Citation checks & taint rules:** Verifies `{path, lines, sha256}` against code on read; enforces session-level taint so reading `untrusted` external text prevents promoting subsequent claims without corroboration or human review.
  * **Zero-LLM hot path:** Code-aware BM25 (`~1.1 ms` p95 scoped recall) returning ~60-token `L0` cards first.
* **Six structural gaps for our workflow:**
  1. **`AF_UNIX` daemon friction & RSS:** `hivemindd` holds projections in RAM (~117 MiB RSS at 100k claims) behind a Unix socket that Seatbelt/container sandboxes (Codex, Copilot `/sandbox`, contained sessions) block unless manually allowlisted.
  2. **Voluntary cold-write friction:** Relies on agents choosing to call `remember`; mid-turn user corrections routinely go unrecorded.
  3. **Turn-0-only push (`hm brief`):** Only injects memories on the initial prompt; misses errors and subsystems encountered mid-session on Turn 14.
  4. **Manual `--ctx` strings:** Agents must invent and pass matching `key=value` context flags by hand (`os=macos` vs `os=darwin`).
  5. **No live peer awareness:** Shipped MCP only exposes 8 past-tense claim-CRUD tools; concurrent agents remain blind to each other's live sessions, touched files, and working hypotheses.
  6. **Single-node storage:** Cross-machine operation requires standing up and managing a separate OAuth 2.1 HTTP server; local-first multi-machine bridging is out of scope.

### 3.2 `ldclabs/KIP` (KIP 2.0) & `Cognitive Nexus` (`AndaDB`, Rust, MIT)
* **What it is:** An open specification (**Knowledge Interaction Protocol 2.0**, `ldclabs/KIP`) plus a Rust embedded graph/document engine (`ldclabs/anda-db`, combining B-Tree, BM25, and HNSW indexes) and executor crate (`anda_cognitive_nexus`), wrapped by `ldclabs/anda-brain`.
* **What to borrow:**
  * Clean separation between a truth-neutral **Proposition** and an actor's **Assertion/Stance**.
  * Computing belief/trust via a **versioned read-time policy** over raw evidence and confirmations rather than freezing a single score on write.
  * Keeping **governance/permissions strictly unreachable** from memory content.
* **Why not depend on `anda_cognitive_nexus` directly:**
  * Forces agents to read and write a bespoke SQL-like graph DSL (`KQL` `FIND`, `KML` `UPSERT`/`MERGE` over Concept Nodes and Proposition Links), adding token overhead and syntax error retries.
  * Single-writer in-process embedded store with no multi-machine host bridge.
  * Lacks code-native primitives: no git/file-hash citations, no `0`/`1`/`N`-repo workspace discovery, no lockfile semver predicates, and no live peer session presence.

### 3.3 Broader Frontier Survey

| System | Core Mechanism | What We Take | What We Avoid |
| :--- | :--- | :--- | :--- |
| **GitHub Copilot Memory** | Repo-scoped facts with pre-execution `{path, lines}` verification against current branch; 90-day TTL refreshed only on validated use. | Citation verification on local checkout + reinforcement *only* on validated helpful use. | Vendor lock-in; inability to verify non-repo / machine / multi-repo claims. |
| **Letta Code (`MemFS`)** | Git-backed markdown Context Repositories + sleep-time reflection subagents. | Deterministic human-readable export (`--jsonl` / markdown) for auditability. | Using flat markdown files as the primary query/applicability engine. |
| **`activeloopai/hivemind`** | Session trace capture $\rightarrow$ background synthesis into `.claude/skills/` + AST symbol graph. | Optional downstream promotion of heavily corroborated procedures into skills. | Heavy external Deeplake/cloud dependency; lack of atomic contradiction/taint tracking. |
| **Graphiti (`Zep`) & Hindsight** | Bi-temporal entity graphs + hybrid BM25/vector/graph reranking. | Explicit validity intervals `[valid_from, valid_until)` + Reciprocal Rank Fusion (RRF) hook point. | 8–19 LLM calls per write (`Graphiti`); Python + external DB operational weight. |
| **`huggingface/funes`** | Zero-LLM verbatim session indexing (`session_id` + `turn_uuid`). | Exact episodic provenance pointers without lossy LLM write-time summarization. | Episodic-only search without claim supersession or normative axioms. |
| **`AdviceNXT/sbp`, `PheroPath`, `MCP Agent Mail`, `Beads`** | Stigmergic blackboards, decaying file/error "pheromones", advisory file leases, task DAGs. | Decaying file/target/error footprints + live session presence in `pulse`. | Ephemeral-only state that evaporates without distilling durable claims. |

---

## 4. Head-to-Head Summary

| Capability | `mingley/hivemind` | `KIP / Cognitive Nexus` | **Our Design (`lore`)** |
| :--- | :--- | :--- | :--- |
| **Multi-Machine Model** | Single-box daemon or separate OAuth cloud server | Single-process embedded library | **Host-Authoritative over Humata's SSH/gRPC Bridge** (plus standalone local mode) |
| **Workspace Topology** | Single project scope or manual `--scope` | Repo-unaware concept triples | **Auto-discovered `0`, `1`, or `N` child git repos, worktrees & non-git dirs** |
| **Live Peer Awareness** | None (past-only CRUD) | None (past-only graph) | **Fleet-wide live `pulse` (active peers, branches, touched paths, hypotheses, build failures)** |
| **Storage & Hot/Cold Cost** | Uncompressed/mmap custom `.hlog`, ~117 MiB RSS @ 100k | In-process B-Tree + BM25 + HNSW | **Uncompressed framed Protobuf hot log + SQLite WAL/FTS5; `(status, age)` `zstd` archive & GC** |
| **Clean-Room Kill Switch** | None | None | **Single global `ON`/`OFF` switch** (instant fleet silence, `0` prompt tokens when `OFF`) |
| **Crate & Build Isolation** | Standalone Cargo workspace | Multi-crate Cargo workspace | **Isolated `//tools/lore/...` leaf crates (`~1–2s` incremental Bazel loops, zero UI/daemon rebuilds)** |
