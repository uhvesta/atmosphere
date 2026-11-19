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

## Release plan (owner, 2026-11-19): walk this in order

- **Patch:** release build plus the cheap tests: `apps/humata2/tools/tests/presubmit.sh` (tests affected by the change, slow.txt excluded). A red cheap test in the step's own area blocks the patch; an unrelated red one ships and is fixed in the next patch. Then `tools/release/release.sh <ver>` and `install.sh`.
- **Launch check:** `apps/humata2/tools/release/check.sh humata2-v<ver>` (signature, notarization, the app starts and reports its version; opens no windows).
- **Full suite:** `presubmit.sh --full` (slow tier included), every failure fixed, plus the launch check.
- Each step: finish it, commit, push, release, install. Start after the CI speed-up (TESTSPEED). Steps may run in parallel lanes, but releases go out in this order. If two steps land together, ship one patch for both. Keep launch checks on every 3rd patch and every minor version.

| # | Step | Release | Checks |
|---|------|---------|--------|
| 1 | The redesigned Diff panel fits every width (GOLDENFIX) | 0.3.7 (released 11:30, 499061fc) | patch |
| 2 | The UI test suite passes, only intended screenshots re-recorded (NATIVEFIX) | shipped in 0.3.7 (exit run 11:30: native, design, look, panels green) | — |
| 3 | Reviews show full names on hover, with speed budgets (REVIEWUI) | 0.3.9 | patch + launch check |
| 4 | Progress and stack views match design (PROGRESS, STACKVIEW) | 0.3.10 | patch |
| 5 | Owner's UI marks (chevron, Exit full screen + Close, stack rows name repos) | 0.3.11 | patch |
| 6 | Live terminals survive moves, renames and upgrades (LIVEHOLDER) | 0.3.12 | patch + launch check |
| 7 | Agents' questions and waits show in the inbox and clear (AGENTS2) | 0.3.13 | patch |
| 8 | Iterating on work stays simple (UXSIMPLE) | 0.3.14 | patch |
| 9 | Fresh review terminals beside any diff (REVIEWER) | 0.3.15 | patch + launch check |
| 10 | One ranked To-do list (BOARDSIMPLE) | 0.3.16 | patch |
| 11 | Two quiet inboxes (INBOXHOOKS) | 0.3.17 | patch |
| 12 | Fix every failure | **0.4.0** | full suite + launch check |
| 13 | Antigravity `agy` (AGY) | 0.4.1 | patch |
| 14 | Walk every flow, audit, fix (NORMALREL; flow fixes ship as 0.4.x patches first) | **0.5.0** | full suite + launch check |
| 15 | Fast mode benchmarked and tuned (FASTREL) | **0.6.0** | full suite with fast mode on + launch check |
| 16 | Optional desktop notifications (NOTIFY; owner 2026-11-19: "literally at the very very end") | 0.6.1 | patch + launch check |

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
- [ ] **I open fresh review terminals beside any diff for a second pair of eyes (REVIEWER).** "Open review terminal ⌄" in the Diff panel opens a new terminal, or an agent TUI I choose (claude, copilot, codex), in a read-only copy of the diff. There is no model picker: I pick the model inside the TUI. I can open several and hide them without ending them. **Render first:** mockups are in `milestones/mockups/reviewer/`, and nothing is built until the owner approves them and the UX review below has ruled. See `milestones/REVIEWER.md` (owner, 2026-11-19).
- [ ] **Iterating on work stays simple, not niche (UXSIMPLE, owner 2026-11-19).** Fable 5.2 reviews the whole loop: agent works → diff → comments → agent revises → stack across repos → push/PR → PR review → review terminals. It counts the concepts a user must learn, ranks overlaps (several "review" meanings, several base pickers, several ways to get a second opinion), gives a verdict on REVIEWER, and proposes a loop of at most ~7 concepts. See `milestones/UXSIMPLE.md`.
  - Fable renders these flows in the proposed loop (8–14 frames, `scratchpad/ux-review/frames/` + `FRAMES.md`):
    1. I open a terminal and just start working, then decide to make a stack of PRs across many repos.
    2. I made them in one agent session and don't know where the code is, so I ask the agent to prepare a review view of the stack.
    3. I review it: comment, sign off on some layers or files, mark some as good.
    4. The agent applies my feedback; I review it by hand in the plain diff view (no prepared ordering), seeing what changed since I last looked and which comments were addressed.
  - The owner picks which changes go on the roadmap. Nothing is built from the review without that.
- [ ] **Planning work on boards is simpler (BOARDSIMPLE, owner 2026-11-19).** The owner suspects the board (BD-1–BD-16, frames 21–24, D106/D155/D216) is too complicated. Compare it with a lightweight global to-do model:
  - one global list, where channels and sessions are filters rather than separate boards;
  - optional metadata on items (labels/key–values) and a nicer item view;
  - **due dates**, which we don't have today (`BoardItem` has no due field).
  - Today's model to weigh against it: three board kinds (Global, channel, session) with a Scope picker; five fixed columns (Backlog, To do, In progress, In review, Done); board, table and dashboard views; pinned and VIP marks; several sessions per item; Depends on; agent asks answered on the item; revisions and history.
  - Output: a compare-and-contrast and a proposed simpler model (what to merge, cut, add), with frames. The owner approves before any change. See `milestones/BOARDSIMPLE.md`.
  - Do not name or link any outside project in code or docs.
- [ ] **Two simple inboxes interrupt me as little as possible (INBOXHOOKS, owner 2026-11-19).** The agent inbox holds asks that need me now (risky permission approvals and questions, raised by hooks). The to-do inbox is the higher level: the ranked list and agents' proposals about it, and it never interrupts. Scopes become filters. Asks through Humata's own MCP (moving files or sessions, changing to-do statuses) require a reason and evidence, show names rather than IDs, before → after, and what else they affect, with an Ask why button. Status changes become Agent suggestions that never block. Agents' own permission prompts stay as they are. Hooks are the path we invest in. If a hook is missing or fails, that's fine. Screen reading is best effort only: what exists stays while it works for free, it gets no new work, and anything costly (D785/D786 first) is cut. See `milestones/INBOXHOOKS.md`.
- [ ] **Antigravity (`agy`) is a supported agent (AGY, owner 2026-11-19).** See `milestones/AGY.md`.
- [ ] **Owner's UI marks (2026-11-19):**
  - The per-repo base chevron (⌃⌄ beside each repo) says what it does, or stays hidden until the bases differ (revisits D739).
  - The full-screen Diff header says "Exit full screen" instead of "Show the terminal" and offers Close.
  - Stack view: each layer row names the repos it touches; Whole stack marks layer boundaries; frames 119–121 are rendered for the owner.
- After each closed lane: commit, push, then a patch release (release build only, no tests) and install. Patch releases go out often. Every 3rd patch and every minor release also runs the launch check `apps/humata2/tools/release/check.sh humata2-v<ver>` (notarization, signature, the app starts and reports its version; opens no windows).
- **Order (owner, 2026-11-19):** release 0.4 after INBOXHOOKS and before AGY; AGY lands as a patch on 0.4.

## 2. Minor release (0.4) — before AGY

A minor (0.x) release bumps the version and needs the full suite green; a patch needs only a release build.

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
