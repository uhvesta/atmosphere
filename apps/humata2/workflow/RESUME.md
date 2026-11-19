# RESUME: Humata 2 DAG & Cache-Isolation Migration

Updated 2026-11-19 00:05 on `main` (`atmosphere-khodr`; the old `../atmosphere-split` worktree was merged at `08faf690` and deleted). `WORKLOG.md` holds the full event log; this file is the authoritative, resumable execution order.

> **Completion rule for every phase:** a phase is not done until it has (1) a green focused Bazel build/test gate, (2) updated dependency evidence (no unintended monolithic edges), and (3) a recorded 3-metric warm-cache check in `WORKLOG.md` (Rust compilation, daemon linking, and Bazel test execution).

---

## 0. Cache-Isolation MVP & SPLIT Cleanup (COMPLETED 2026-11-19)

### Completed & Verified GREEN
- [x] **M0 — Land & clean checkpoint:**
  - `clients:api` split (`pub struct Clients` in `clients/src/api.rs`; all 14 `features/*`, 9 `ui/*`, and `clients/live_*` crates on `//apps/humata2/clients:api`).
  - UI 8-crate split (`ui/{core,changes,desk,machines,overlays,report,settings,workspace}` + 7-file `ui` shell; `panel_changes.rs` split into 6 submodules; all 9 `ui/*` unit tests passing; moved duplicate `ui/src/` files deleted).
  - `daemon/sessions` 3-way split (`sessions_core`, `sessions_desk`, `sessions_carry` + `*_faults` twins behind `sessions` facade; all 5 session unit/kill-matrix/power-cut tests passing; duplicates deleted).
  - `clients/live` 7-way split (`live_{core,daemon,desk,diff,files,machines,review,test_support}` behind `clients/live` facade; duplicates deleted).
- [x] **M1 — Proto nominal identity + narrow foundations:**
  - `api/proto/protoc_gen_prost_domain.rs` (auto `extern_path`) + `api/grpc/protoc_gen_tonic.rs` (`file_to_generate` filter); single nominal Rust definition per `humata.v1` type; `daemon/humatad:grpc` aliased to `//apps/humata2/api/grpc:all` (96/96 `api/...` targets green, 0 warnings).
  - `daemon/ports` (strictly narrow: IDs, value types, traits), `daemon/rpc_kit` (`api/grpc:core`), `daemon/runtime` (`api/grpc:{core,terminal}`, runtime `admin_mode`) all built & unit tests passing.
- [x] **M2 — Scoped Tier-B test binaries + shared `:test_common` + capability matrix:**
  - Wrote [SPLIT_CAPABILITY_MATRIX.md](milestones/SPLIT_CAPABILITY_MATRIX.md) mapping all 216 `humatad` + 14 `clients/live_*` tests by actual runtime calls.
  - Built all **6 scoped Tier-B daemon profiles** (`humatad2_test_{power_cut,machines,review_remote,catalog_remote,away_board,transfers}` + `_release`/`_faults` twins) and re-pointed 202/216 `humatad` tests + 13/14 `clients/live_*` tests onto them (leaving 14 explicit Tier-C tests on `:humatad2` / `:humatad2_admin`).
  - Detached `svc_catalog`, `mcp`, `board_service`, and the entire `review` stack from `humatad2_test_power_cut` (shrank 234 → 208 files); deleted 12 dead extracted files from `daemon/humatad/src/`.
  - Extracted `daemon/humatad/tests/common/` (~275 KB) into shared `rust_library(name = "test_common", crate_name = "humatad_test_common")` and migrated all 215 `humatad/tests/*.rs` off `mod common;` / `#[path = "common/..."]` (compiles once in 4.6s instead of 216×; `daemon_test` & `sessions_test` verified passing).
  - Documented cache-isolation guardrails in root `AGENTS.md`.
- [x] **M3 — 5 warm-cache isolation probes (all empirically verified):**
  1. Excluded daemon services (`svc_catalog/src/agent_gate.rs`, `humatad/src/board_service.rs`, `humatad/src/review.rs`, `humatad/src/mcp.rs`) → **0 actions, cached `PASSED` on `daemon_test` in 0.9–1.4s** (Critical Path 0.00s).
  2. `sessions_carry/src/carry.rs` → **1 action, 2.2s** (0 rebuild of `sessions_core`/`sessions_desk`).
  3. `ui/changes/src/panel_changes/states.rs` → **3 actions, 6.1s** (0 rebuild of sibling `ui/*` or `features/*`; full-window `desktop` goldens intentionally invalidate).
  4. `clients/src/fakes/clipboard.rs` → **1 action, 3.2s** (0 rebuild of `clients:api`, `features/*`, `ui/*`, or `clients/live_*`).
  5. `api/proto/humata/v1/review.proto` → **1.57s, 5 cache hits** (0 rebuild of `rpc_kit`, `runtime`, `svc_board`, `svc_files`, `live_machines`).
- [x] **C1 / C2 / C3 — All 19 `daemon/svc_*` crates extracted** (including mandatory ≤12k LOC splits `svc_upgrade` + `svc_stop_restart`, `svc_channels` + `svc_desk`, and 3-way `svc_sessions_{common,local,archive,transfer}`; 18/19 compiling green, 17/19 unit tests passing).
  - *Optional follow-up:* finish wiring the remaining standalone `svc_*` crates into `daemon/humatad/src/serve.rs` and delete their duplicate `humatad/src/` modules to shrink `humatad2` to a thin ~15-file shell.

---

## 1. Finish the open feature lanes

- [ ] **The redesigned Diff panel fits every width, with no doubled or clipped labels (GOLDENFIX).** Fable's rejects from 2026-11-18 21:20 (full verdict in `scratchpad/nativefix/FABLE_VERDICT.md` / `milestones/NATIVEFIX.md`):
  - **Narrow footer beside the terminal (design 01/02/15/16/17/30/62/65/84/86):** drop the "1 draft comment" text and give the button `flex_shrink_0` (`desktop/tests/design/session.rs:654-667`, `footer()` in `design/diff_targets.rs`). Mock hunk header shows only `"impl Client {"` (`design/session.rs:553`, `design/comments.rs:61`, `design/diff_targets.rs:118`).
  - **Repo row** (`ui/changes/src/panel_changes/review_header.rs` `repo_head`): name shrinks with ellipsis; chevron and totals don't shrink. **File header** (`ui/changes/src/panel_changes/file_list.rs`): repo and folder give way, filename keeps room (`diff-stress-long-repo-and-branch`).
  - **Binary and mode-only files** say it twice (`ui/changes/src/panel_changes/mod.rs` / `file_list.rs`): skip header note when file has no code lines.
  - **PR review footer** (68, 68c, 69): drafts text gets `min_w_0` + ellipsis so `Submit review` stays inside padding.
  - **Follow-ups:** blank `↗` icon (`open_in_editor`); file bar ellipsis; space around `+/−` in PR rows; Back ghost button (DS-58); replace `"No changes to show"` on listed file; cap sent comment card at 720 pt; virtualize `review_code` for 68s `pr-huge-file`.
  - **Then:** re-record native frames + `bazel run //apps/humata2/desktop:design_test_record` (run `design_test` alone). Judge goldens with `gemini-4-neon-high`.
- [ ] **The UI test suite passes, and only intended screenshot changes are re-recorded (NATIVEFIX).** Verify remaining items (`stack` next-layer key, `⌥Z` in comment box, Escape in diff full screen, queue cases, `session_menu`, `holder_swap_identity`) and close milestone.
- [x] **Split `panel_changes.rs` into submodules** — landed in `ui/changes/src/panel_changes/`.
- [x] **A notice that the agent waits in its terminal offers only Dismiss.** Landed in `0.3.6` (`3fae8a47`).
- [ ] **Reviews show the full name of any shortened label on hover and have measured performance budgets (REVIEWUI):** gate `native_test review_ui:: pr_session::`; run RV-8 budgets with `-c opt --ignored review_ui::budgets::`.
- [ ] **Slow tests run in minutes, not tens of minutes (TESTSPEED):** record before/after timings per target, move now-fast targets out of `slow.txt`, then optimize `power_cut_*` and `live_flows`.
- [ ] **Live terminals survive moves, renames and upgrades (LIVEHOLDER):** `move_session_freeze_test` rig + full `delete_channel_test` in slow tier.
- [ ] **Agents' questions and waits show up in the Inbox and go away when answered (AGENTS2):** queue O1–O4 live checks for owner.
- [ ] **Progress and stack views match their design frames (PROGRESS, STACKVIEW):** frames 122–124 and 119–121 judged.
- [ ] **I open a fresh agent beside any diff for a second pair of eyes (REVIEWER).** An "Open review session" button in the Diff panel picks a model and opens a separate agent terminal that I can hide and show. It is a new agent, not the session's author. See `milestones/REVIEWER.md` (owner, 2026-11-19).
- [ ] **Owner's UI marks (2026-11-19):**
  - The per-repo base chevron (⌃⌄ beside each repo) says what it does, or stays hidden until the bases differ (revisits D739).
  - The full-screen Diff header says "Exit full screen" instead of "Show the terminal" and offers Close.
  - Stack view: each layer row names the repos it touches; Whole stack marks layer boundaries; frames 119–121 are rendered for the owner.
- After each closed lane: gate, commit, push, patch release, install.

## 2. Minor release (0.4)

1. Full postsubmit (`presubmit.sh --full`, slow tier included).
2. Fix every failure.
3. Release `0.4.0` and install it.

## 3. NORMALREL: a release that works end to end with fast mode off

1. Walk every user flow in `apps/humata2/flows/` and audit against design goldens and `DECISIONS.md`.
2. Fix what is broken.
3. Run full postsubmit.
4. Release and install. Send screenshots of the flows working.

## 4. FASTREL: a fast-mode release that matches the normal one

1. Benchmark fast mode against the NORMALREL build (startup, diff open, scroll, turn latency).
2. Tune it.
3. Every test must pass with fast mode on.
4. Release fast mode as its own release and install it.

## Standing rules

- Bazel only, never cargo. Never `bazel clean` or `bazel shutdown`.
- Main owns git, bazel and releases. Workers never run git or bazel.
- One bazel per worktree.
- Headless only. Never end running terminals or holders.
- No AI attribution in commits. Never commit `.claude/` or `.nvimlog`.
- Report milestones as outcome sentences.
