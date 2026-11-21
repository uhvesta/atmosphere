# 02: Architecture, Storage, Metadata Schema & Considerations (`lore`)

Status: Pre-spec architectural design (2026-11-21). Companion research: [01-RESEARCH-AND-LANDSCAPE.md](01-RESEARCH-AND-LANDSCAPE.md). Spec prompt: [03-INTENT-SPEC-PROMPT.md](03-INTENT-SPEC-PROMPT.md).

---

## 1. Design Pillars & Non-Negotiables

1. **Fast-Iteration Crate Isolation (`//tools/lore/...`):** Zero build dependencies on `apps/humata2/ui`, `desktop`, `clients`, or `humatad_core`. Incremental edit-test loops (`tools/bq/bq --fast //tools/lore/...`) run in ~1–2 seconds using crates already in root `Cargo.toml` (`prost`, `rusqlite` bundled w/ FTS5, `zstd`, `crc32fast`, `sha2`, `clap`, `serde`, `serde_json`, `thiserror`, `versions`, `regex`, `rustix`, `uuid`).
2. **Host-as-Single-Brain over Humata's Bridge (Zero Cross-Machine File Sync):** When running with Humata, one authoritative store lives on the Mac Host (where the desktop app runs). Every agent on any linked machine writes to the Host and queries across all machines via Humata's existing authenticated SSH/gRPC `ForwardedCall` bridge, while verifying local file citations on the calling machine. Outside Humata, the exact same binary falls back seamlessly to a local `~/.lore/` store.
3. **Single Global On/Off Switch (`memory.enabled`):** One global toggle pauses/resumes the entire system across all machines and harnesses without deleting stored data, dropping prompt/schema token overhead to **0** when `OFF`.
4. **Hot Path Stays Zero-LLM & Zero-Compression:** Active/new items append raw length-prefixed Protobuf frames and sit hot in SQLite WAL + FTS5 with zero `zstd` CPU tax; **only old or non-active items** (by `status`, `age`, or `status + age`) are compacted, `zstd`-compressed, or garbage-collected.
5. **Dual Horizon — Epistemic Past + Stigmergic Present:** Unifies durable, citation-verified claims with live cross-agent awareness (active peers, mounted repos/branches, touched files, working hypotheses, and recent build failures).

---

## 2. Isolated Crate Topology & Build Boundaries

```text
tools/lore/
  proto/        # lore_proto: pure prost + serde types, IDs, enums, wire envelopes (<0.5s compile, zero I/O)
  context/      # lore_context: <2ms 0/1/N-repo discovery, root-commit ID, OS/lockfiles, local citation verifiers
  engine/       # lore_engine: framed .pblog log, SQLite WAL/FTS5 projection, scoring, (status, age) zstd GC
  mcp/          # lore_mcp: Backend trait + 5 stdio MCP tools + <3ms `prompt` / `cue` hook handlers
  src/          # lore binary: clap CLI (`lore <cmd>`, `lore mcp`, `lore hook`, `lore gc`, `lore guide`) + :lore_linux
  docs/         # research, architecture, and intent spec documents
```

### Thin "Bundle Once" Seams into Humata (Touched Only at Integration)
1. **Binary Bundle:** Add `InstalledFile::Lore` alongside `InstalledFile::Stack` in `apps/humata2/daemon/machines/src/bundle.rs` so Humata installs and SHA-256-verifies `lore` in `~/.humata2/bin` on the Mac Host and every remote machine.
2. **Host Bridge Service:** A tiny leaf crate `apps/humata2/daemon/svc_lore` wrapping `lore_engine` behind the Host's global `memory.enabled` switch, serving local calls on `humatad.sock` and remote machine calls via existing `ForwardedCall` Uplink (with Humata §9 away-journal support when the Mac is asleep).
3. **Session Launch Wiring:** When `memory.enabled == true`, `apps/humata2/daemon/agent_hooks/src/mcp.rs` and `sessions_core/src/agents.rs` include `lore mcp`, pre-allow `mcp__lore` and `Bash(lore:*)`, and attach non-blocking `lore hook prompt || true` and `lore hook cue || true` hooks across Claude, Codex, Copilot, and Antigravity (`agy`). When `false`, nothing is injected (`0` tokens).
4. **Bundled Skill:** `apps/humata2/skills/lore/SKILL.md` + `lore.md`, auto-packed by `//apps/humata2/skills:bundle` into `~/.claude/skills`, `~/.codex/skills`, and `~/.copilot/skills` (and retired cleanly when disabled).

---

## 3. On-Disk Storage: Framed Protobuf over JSONL + `(status, age)` `zstd` Tiering & GC

### 3.1 Why Length-Delimited Framed Protobuf (`.pblog`) Beats JSONL

| Criterion | JSONL (`.jsonl`) | **Length-Delimited Framed Protobuf (`.pblog` via `prost`)** |
| :--- | :--- | :--- |
| **Schema Evolution** | Renaming/refactoring keys breaks parsers or needs string aliases; repeats verbose key names on every line. | Field tag numbers (`1..N`) + `reserved` tags allow additive/optional evolution cleanly forever; `attrs` map covers arbitrary dynamic metadata. |
| **Size & Speed (Uncompressed Hot Path)** | ~900–1,400 B/event; slower UTF-8 string escaping on embedded diffs/stderr/newlines. | **~250–400 B/event (~3× smaller raw)**; ~6× faster encode/decode; raw bytes/newlines never break framing. |
| **Power-Cut Torn-Tail Safety** | Newline scan can misparse truncated UTF-8 or partial lines without a separate wrapper. | Exact byte framing + CRC32C/IEEE header makes torn-tail detection and truncation deterministic on recovery. |
| **Human Inspectability (`jq` / `grep`)** | Native text. | Preserved via `lore log --jsonl` / `lore export --jsonl` (same Rust structs derive both `prost::Message` and `serde::Serialize`). |

#### Hot Segment Binary Record Layout (`hot/seg-<base_seq>.pblog`)
```text
File Header (16 B):  magic b"\x89LORE\r\n\x1a" (8 B) | format_version u16 (1) | flags u16 (0 = raw) | header_crc32 u32
Record Frame:        payload_len u32 LE | crc32_ieee(seq || payload) u32 LE | seq u64 LE | prost_encoded_LogEvent[payload_len]
```
* **Hot Writes:** Acquire `write.lock`, append frame to current active `seg-<base>.pblog`, `File::sync_all()` (`F_FULLFSYNC` on macOS), apply to `index.db` WAL, release lock (`<2–3 ms`, **zero compression CPU**).
* **Crash Recovery:** On startup, scan only the active tail segment from the last SQLite `applied_seq` checkpoint; if the final record is truncated or fails `crc32`, truncate to the last valid frame boundary, `sync_all`, and log a recovery notice.

### 3.2 Hot vs. Cold `(status, age)` Tiering, `zstd` Compression, and GC

New and active items stay **100% uncompressed** so everyday reads and writes pay zero `zstd` latency. Background maintenance (triggered lazily when cold candidates cross a byte/count watermark, or explicitly via `lore gc [--dry-run]`) applies **`(status, age)` rules**:

| Tier | Match Rule (`status`, `kind`, `age`) | Storage Action | Query Visibility |
| :--- | :--- | :--- | :--- |
| **Tier 0: Hot** | `status == active` AND (`pinned` OR `kind in {axiom, decision}` OR `age_since_validated <= 90d`), plus any non-active item updated `< 7d` ago | Kept uncompressed in `hot/*.pblog` + full columns & FTS5 tokens in `index.db`. Evidence blobs `< 512 B` inline; larger blobs content-addressed. | Instant `<1 ms` default `recall` & `pulse`. |
| **Tier 1: Cold Archive (`zstd`)** | **(a)** `status in {superseded, invalidated}` with `updated_at > 7d`, OR **(b)** `status in {active, expired}` (unpinned `fact`/`solution`/`warning`/`dead_end`) with `max(created_at, last_validated_at) > 90d` | Evicted from `hot/*.pblog` and packed into **256 KiB–4 MiB bulk `zstd` archive frames** (`archive/arch-<seq_range>.pblog.zst`, level 6, `\x89H2Z`-style CRC32 header, **~7×–10× space reduction**). Removed from hot `claims_fts`; only a tiny ~64-byte lineage/header stub (`id, kind, status, statement, archive_offset`) stays in `index.db`. | Hidden from hot search noise; transparently decompressed in `<2 ms` on `lore show <id>`, supersession lineage walks, or `recall --include-history`. If a dormant claim is re-validated, it promotes back to Tier 0! |
| **Tier 2: Fold & Reclaim (GC Purge)** | **(a)** `kind == handoff` in `{superseded, invalidated, expired}` and `age > 14d`<br>**(b)** `status == retracted` (`wrong`/`misrecorded`) and `age > 30d`<br>**(c)** `status == quarantined` (uncorroborated untrusted) and `age > 14d`<br>**(d)** `claim_exposures` rows (`age > 7d`)<br>**(e)** Explicit `forget(id)` (`age >= 0`) | Exposure rows fold into integer counters (`surfaced_count`, `helpful_count`) and delete raw rows. Purged claims drop payloads (leaving only a minimal `{id, purged_at, reason}` tombstone for `forget`). Mark-and-sweep deletes unreferenced files in `blobs/`. `PRAGMA incremental_vacuum` shrinks `index.db`. | Permanently reclaimed. |

* **Content-Addressed Blobs (`blobs/<sha256>.blob`):** Raw build error logs and diffs attached as evidence are deduplicated by SHA-256 and compressed on write *only* if `len >= 1 KiB` (saving $\ge 1/16$ via `zstd` frame, matching `humata2_chunk::encode_framed`), otherwise stored raw.

---

## 4. Workspace Topology (`0`, `1`, or `N` Git Repos) & Branch Semantics

### 4.1 `ContextFrame` Discovery (`lore_context`, `<2 ms`)
1. **Single Git Repo / Worktree (Including Sandboxed Worktrees):** Walk upward from `cwd` for `.git` (directory or `gitdir:` worktree pointer file).
   * **Normal checkout/worktree:** Canonicalize `repo_id` from normalized `remote.origin.url` (stripping credentials/`.git` suffix, e.g. `github.com/owner/repo`), falling back to `git:<root-commit-sha>` so unpushed local clones still share one deterministic ID.
   * **Sandboxed git worktree fallback (critical edge case):** In sandboxed worktrees (where `.git` is a file pointing to `gitdir: /path/outside/sandbox/.git/worktrees/<name>` and shelling out to `git` fails with `EPERM` / exit 128), `lore_context` never crashes or shells out blindly: it parses the `gitdir:` line directly to extract the parent repo directory name (or consults Humata's bound `session.repos`) so repo-scoping still works inside strict sandboxes!
2. **Parent Folder with Multiple Git Repos** *(Humata multi-repo session folder or `~/oss`)*: If upward walk finds no `.git`, inspect bound session repo list (if inside Humata) + bounded depth-1/2 directory scan for child `.git` roots, producing `mounted_repos: Vec<RepoMount { repo_id, mount_subpath, branch, head_sha }>`.
   * **Reads:** Query `global` $\cup$ `machine:<m>` $\cup$ `channel:<c>` $\cup$ all `mounted_repos` (narrowing to one `repo:<id>` when `about` names files under its `mount_subpath`).
   * **Writes:** Paths inside `api/src/foo.rs` auto-normalize to `(scope: "repo:acme/api", path: "src/foo.rs")`; cross-repo learnings bind to `channel:<c>` or `global` with `about: ["repo:acme/api", "repo:acme/web"]`.
3. **Outside Any Git Repo** *(Humata Desk session, `~`, ops directories)*: `mounted_repos = []`. Reads and writes operate cleanly over `global`, `machine:<m>`, `channel:<c>`, and `dir:<m>:<canonical_cwd_hash>`.

### 4.2 Branch Semantics
* **Durable claims are branch-agnostic by default** (`scope = repo:<id>`, never `branch`), storing `origin_branch` and `origin_commit` strictly as informational audit metadata.
* **Branch divergence is resolved locally via Citations & Commit Ancestry:** On read, the calling machine checks `verifiers` (`file_sha256` / `symbol_present` / `since_commit` / `until_commit`) against *its own* working tree, marking claims `[drifted]` only on branches where the cited code actually changed.
* **Live Peer Awareness (`pulse`) IS branch-aware** (reporting each peer's active `branch`, `head_sha`, and dirty file count), while `applies_when` retains an opt-in `branch_glob` (`"release/*"`) for explicit release-freeze rules.

---

## 5. Canonical Metadata Schema & Weighting Foundation

### 5.1 Protobuf Core Types (`lore_proto`)
Every `Claim` factors cleanly into **6 orthogonal facets**:

1. **Identity & Lifecycle:** `id` (`clm_<uuidv7>`), `kind` (`AXIOM`, `FACT`, `DECISION`, `SOLUTION`, `DEAD_END`, `WARNING`, `PREFERENCE`, `QUESTION`, `HANDOFF`), `status` (`ACTIVE`, `SUPERSEDED`, `INVALIDATED`, `RETRACTED`, `EXPIRED`, `QUARANTINED`), `tier` (`HOT`, `COLD_ARCHIVED`), `created_at_ms`, `updated_at_ms`, `valid_from_ms?`, `valid_until_ms?`, `review_after_ms?`.
2. **Progressive-Disclosure Content:**
   * `statement` ($\le 280$ chars — `L0` card headline)
   * `body` (optional `L1` markdown details / commands / rationale)
   * `dead_ends` (`repeated string` — explicit negative knowledge)
   * `keywords` (`repeated string` — normalized error tokens / synonyms)
   * `attrs` (`map<string, string>` or JSON bytes — open schema extension slot)
3. **Scope vs. Subject Entities:**
   * `scope`: single visibility URI (`global`, `channel:<id>`, `repo:<id>`, `machine:<id>`, `dir:<m>:<hash>`, `session:<id>`)
   * `about`: `repeated string` typed `<ns>:<key>` URIs (`repo:...`, `path:<repo>:<rel_path>`, `symbol:<lang>:<fqn>`, `cmd:<bin>`, `pkg:<eco>/<name>`, `err:<norm_sig>`, `target://<label>`)
4. **Applicability Predicates (`repeated Predicate`):**
   * `{ axis, op, value }` (`op` $\in$ `EQ`, `IN`, `GLOB`, `SEMVER_REQ`, `ANCESTOR_OF`, `NOT_ANCESTOR_OF`). Built-in axes: `os`, `os_version`, `arch`, `machine`, `agent`, `branch_glob`, `dep:<eco>/<pkg>`, `commit:<repo>`. Unknown axes evaluate to `UNKNOWN` (soft badge, never crash).
5. **Provenance, Taint & Verifiers:**
   * `Principal { kind: OWNER | AGENT | IMPORT, id, session_id?, agent_kind?, model?, machine_id }`
   * `tainted: bool` (propagates if session previously read `UNTRUSTED` content; blocks `AXIOM` creation and wide promotion without owner/clean-session corroboration)
   * `OriginSnapshot { machine_id, repo_id?, branch?, head_commit?, os, arch }`
   * `repeated Verifier` (`FileDigest { repo_id?, path, start_line?, end_line?, sha256 }`, `PathExists`, `SymbolPresent`, `LockfileSemver`, `BlobRef { sha256, role, summary }`)
6. **Raw Signals & Weight Knobs (Never a Single Baked Float):**
   * `pinned: bool`, `importance: f32` (default `1.0`)
   * `corroborator_count: u32` (distinct `(session_id, machine_id)` pairs), `dispute_count: u32`
   * `surfaced_count: u32`, `helpful_count: u32`, `harmful_count: u32`, `not_applicable_count: u32`
   * `last_surfaced_at_ms?`, `last_validated_at_ms?`

### 5.2 SQLite Projection Tables (`index.db`)
* `meta` — `(key, value)` holding `applied_seq`, schema projection version, and last GC watermark.
* `claims` — Hot full rows + Cold 64-byte stub rows (`tier = COLD`, `archive_segment`, `archive_offset`, `archive_len`).
* `claims_fts` — SQLite **FTS5** index over **Tier-0 Hot** claims only, using code-aware pre-tokenization (`camelCase` / `snake_case` / `::` / `/` splitting alongside raw tokens) with column weights `(statement=5.0, keywords_about=4.0, dead_ends=2.5, body=1.0)`.
* `claim_entities` — `(entity_uri, claim_id)` B-tree index supporting exact match and directory-prefix range scans (`path:repo:dir/%`).
* `claim_edges` — `(src_id, rel, dst_id, created_at_ms, author_id, note)` (`rel` $\in$ `SUPERSEDES`, `CONTRADICTS`, `SUPPORTS`, `DERIVED_FROM`, `ANSWERS`, `CONTRASTS_WITH`).
* `claim_exposures` — Rolling 7-day per-session dedup & outcome attribution table `(session_id, claim_id, surfaced_at_ms, outcome)`.
* `live_presence` — In-memory / ephemeral table `(session_id, channel_id?, machine_id, agent_kind, mounted_repos_json, intent?, hypothesis?, touched_paths_json, recent_failures_json, updated_at_ms, ttl_ms)`.

### 5.3 Pluggable Read-Time Ranking Formula
$$\text{Score}(c, q, \text{ctx}) = G_{\text{hard}}(c, \text{ctx}) \cdot \text{importance}_c \cdot \sum_{k} w_k \, \phi_k(c, q, \text{ctx})$$
* $G_{\text{hard}} \in \{0, 1\}$: enforces active status, permitted scope, and zero hard `applies_when` mismatches.
* Default v1 features $\phi_k$: code-aware FTS5 BM25 ($\phi_{\text{lex}}$), `about` entity specificity ($\phi_{\text{ent}}$), scope specificity ($\phi_{\text{scope}}$), derived trust tier ($\phi_{\text{trust}}$), local verifier status $\times$ kind-specific exponential recency decay ($\phi_{\text{fresh}}$), and smoothed empirical utility $\frac{\text{helpful} + 2}{\text{helpful} + 2\cdot\text{harmful} + 0.3\cdot\text{unused} + 4}$ ($\phi_{\text{util}}$).
* **Future extensibility:** Adding quantized embeddings ($\phi_{\text{vec}}$), ACT-R activation, or learned weights requires only updating `ScoringPolicy`—zero rewrites of historical `.pblog` or `.pblog.zst` files.

---

## 6. Key Engineering Considerations & Guardrails

1. **Secret Redaction on Write:** High-entropy token patterns (`ghp_*`, `sk-*`, `AKIA*`, PEM blocks, bearer headers, `$HUMATA_SESSION_TOKEN`) must be rejected or redacted *before* appending to `.pblog` so secrets never enter durable logs or cold `zstd` archives (`forget(id)` remains the emergency erasure path, rewriting affected `.pblog` / `.pblog.zst` segments).
2. **Prompt-Injection & Taint Containment:** Retrieved non-owner claims must always be formatted inside structured evidence envelopes (`[claim clm_... | trust: agent | citation: verified]`), never raw system instructions, and `UNTRUSTED` claims stay quarantined to their originating session until corroborated or owner-approved.
3. **Hook Fail-Open & Strict Latency Budget:** `lore hook prompt` and `lore hook cue` must cap execution (e.g. `50 ms` local / `150 ms` bridged timeout) and **always exit `0`** on any error or unreachable host so a sleeping Mac or busy bridge never blocks an agent's shell or prompt submission.
4. **No Polling Timers (`TR-7` / `TR-9`):** Idle `lore` / `svc_lore` instances run **zero background timers**. Cold `zstd` archival and GC run either piggybacked asynchronously after every $K$ writes / watermark crossings (debounced) or via explicit `lore gc`.
5. **Testing Strategy (Behavior-Only, Fast In-Process):**
   * Unit + `proptest` suites on `lore_engine` covering torn-tail `.pblog` recovery, `(status, age)` `zstd` archival + on-demand hydration, `forget` erasure across both `.pblog` and `.pblog.zst`, `0`/`1`/`N`-repo `ContextFrame` matching, taint propagation, and stale-citation demotion.
   * Zero sleeps; injected deterministic `Clock` trait so 90-day decay and 14-day GC retention tests run in `<5 ms`.
