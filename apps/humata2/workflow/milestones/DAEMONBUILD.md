# DAEMONBUILD: Compile `humatad` Once Instead of 15–20 Times

## Outcome
A change under `apps/humata2/daemon/humatad/src` or any `svc_*` crate costs ~2 `humatad` library compiles (`humatad_core` + `humatad_core_faults`) plus cheap ~80-line binary links (~60–90 s instead of ~293–504 s across 20 full 114-file compiles), while preserving D41 release security and scoped Bazel test caching.

---

## 1. Current Inventory (`bazel query` `rdeps`, 2026-11-19)

Queried via `bazel query --output=graph --graph:factored=false 'rdeps(//apps/humata2/..., set(<all 15 daemon binaries>), 1)'`.

| Binary target | `crate_features` | Test rdeps | Non-test rdeps |
|---|---|---:|---|
| `:humatad2` | `svc_away`, `svc_board`, `svc_catalog`, `svc_files`, `svc_mcp`, `svc_review` | 20 | `peer_link_latency_bench`, `tools`, `tools_linux`, `tools_mac`, `desktop:live_flows_cases` |
| `:humatad2_admin` | `admin` + all 6 `svc_*` | 25 | `clients/live:humatad2_admin`, `desktop:live_flows_cases` |
| `:humatad2_faults` | `admin`, `faults` + all 6 `svc_*` | 3 (`forget_machine_test`, `fork_remote_test`, `upgrade_holders_move_test`) | 0 |
| `:humatad2_test_power_cut` | `admin` | 125 | 0 |
| `:humatad2_test_power_cut_release` | *(none)* | 51 | 0 |
| `:humatad2_test_power_cut_faults` | `admin`, `faults` | 38 | 0 |
| `:humatad2_test_machines` | `admin`, `svc_away`, `svc_catalog` | 12 | 0 |
| `:humatad2_test_machines_release` | `svc_away`, `svc_catalog` | 11 | 0 |
| `:humatad2_test_review_remote` | `admin`, `svc_board`, `svc_files`, `svc_review` | 22 | 0 |
| `:humatad2_test_review_remote_faults` | `admin`, `faults`, `svc_board`, `svc_files`, `svc_review` | 4 (`comments_test`, `pr_review_test`, `pr_stopped_test`, `views_remote_test`) | 0 |
| `:humatad2_test_catalog_remote` | `admin`, `svc_away`, `svc_catalog` | 20 | 0 |
| `:humatad2_test_away_board` | `admin`, `svc_away`, `svc_board`, `svc_catalog` | 21 | 0 |
| `:humatad2_test_away_board_release` | `svc_away`, `svc_board`, `svc_catalog` | 13 | 0 |
| `:humatad2_test_transfers` | `admin`, `svc_catalog`, `svc_files` | 10 | 0 |
| `:humatad2_test_transfers_faults` | `admin`, `faults`, `svc_catalog`, `svc_files` | **0** | **0** |

**Key findings from the query:**
- `:humatad2_test_transfers_faults` has **0 users** anywhere in `//apps/humata2/...`.
- `:humatad2_test_catalog_remote` and `:humatad2_test_transfers` compile the **exact same Rust code** as `:humatad2_test_machines` (same `srcs`, `deps = FULL_DAEMON_DEPS`, and dead `svc_away`/`svc_files` flags).
- `//apps/humata2/daemon/humatad:all` builds **20** `humatad2*` binaries/tests, not 15: the 15 host binaries + **4** cross-compiled `:humatad2` builds (`linux-aarch64`, `linux-x86_64`, `darwin-arm64`, `darwin-x86_64` via `:tools` / `:tools_linux` / `:tools_mac`) + **1** `humatad2_unit_test` (`crate = ":humatad2"`).
- 7 scoped tests (`pr_review_move_test`, `pr_review_remote_test`, `views_remote_test`, `login_test`, `read_marks_remote_test`, `remote_files_test`, `remote_transfers_test`, plus `providers_test`/`skills_test`) still pull `:humatad2` for `RELEASE` because `REV_`, `CAT_`, and `TR_` lack a `RELEASE` twin; and `fork_remote_test` / `forget_machine_test` still fall back to `:humatad2_admin` + `:humatad2_faults` + `:humatad2`.

---

## 2. Measured Baseline (`--jobs=6`, 2026-11-19)

Touch-and-restore comment-only probe in `apps/humata2/daemon/humatad/src/status.rs` and `apps/humata2/daemon/svc_inbox/src/lib.rs`, building `//apps/humata2/daemon/humatad:all` (243 targets), profiled with `--profile`. Both files were restored byte-for-byte afterward (`git diff` empty).

| Run | Total wall time | Execution span (excl. server lock wait) | Critical path | `humatad2*` `Rustc` compiles | `humatad2*` compile CPU-sum | `svc_inbox` compiles |
|---|---:|---:|---:|---:|---:|---:|
| `--config=fast` | **409.0 s** | **336.0 s** | 125.9 s | **20** (55–98 s each) | **1,613.5 s** | 6 (15.2 s) |
| Default (`--config=lint`) | **504.5 s** | **293.4 s** | 128.0 s | **20** (45–116 s each) | **1,534.3 s** | 6 (10.7 s) |

**Why it is slow:** `humatad/src` is **65,329 lines across 114 files**. Every one of the 20 compiles takes 45–116 s, saturating all 6 jobs for ~5 minutes per edit. `svc_inbox` itself only costs 10.7–15.2 s total across all 6 variants — **>95% of incremental rebuild time is recompiling `humatad/src` 20 times**.

---

## 3. `#[cfg(feature = ...)]` Audit in `apps/humata2/daemon/humatad/src`

| Feature | Sites | Files | What it gates |
|---|---:|---|---|
| `admin` | 9 | `main.rs` (1), `serve.rs` (8) | `humata2_rpc_kit::admin::set_admin_mode(true)` and `--admin`, `--admin-pid`, `--cli-login-seconds`, `--script-token-seconds` CLI flags. (`identity.rs` `Authenticate::with_admin` and `rpc_kit::admin::is_admin_mode()` are already compiled unconditionally.) |
| `faults` | 5 | `fault.rs` (5) | `fault::point` → `humata2_store::fault::point` and `power_cut_on_sigusr2()` (`SIGUSR2` → `humata2_durable::cut`), plus swapping 18+ `FAULT_SWAPPED` crates in `BUILD.bazel` (required because `humata2_durable::File` changes struct layout and `store` vs `store_faults` are distinct nominal crates). |
| `svc_mcp` | 4 | `main.rs` (3), `wire_mcp.rs` (1) | `Command::Mcp` subcommand and `humata2_svc_mcp` dep. |
| `svc_board` | 15 | `main.rs` (2), `serve.rs` (6), `wire_board.rs` (1), `wire_machines.rs` (2), `wire_peer.rs` (4) | `BoardSvc`, `BoardServiceServer`, `board_svc.answer_host_calls`, `PeerBoardAdapter` (else `NoBoards`), `humata2_svc_board` dep. |
| `svc_review` | 22 | `main.rs` (2), `serve.rs` (10), `session_service.rs` (2), `session_service/archive.rs` (3), `wire_review.rs` (1), `wire_machines.rs` (2), `wire_peer.rs` (4) | `ReviewSvc`, `PullRequestsSvc`, 5 gRPC servers, `session_svc.use_review` (only 1 method: `unsubmitted_pr_drafts`), `reconcile_submits`, `PeerReviewAdapter` (else `NoViews`), `humata2_svc_review` dep. |
| `svc_catalog` | 5 | `main.rs` (2), `serve.rs` (3) | `AgentServiceServer` + `agent_service.on_change()` (else `OnChange::default()`), and `humata2_svc_catalog::agent_gate` vs duplicate local `src/agent_gate.rs`. |
| `svc_away` | **0** | *(none)* | **Dead flag.** `humata2_svc_away` is unconditionally in `DAEMON_DEPS` and `main.rs`. |
| `svc_files` | **0** | *(none)* | **Dead flag.** `humata2_svc_files` is unconditionally in `DAEMON_DEPS` and `main.rs`. |

**How `HUMATA2_PEER_RETRY_MS` is read** (`apps/humata2/daemon/svc_peer/src/peer_client.rs:61-71`):
`humata2_rpc_kit::admin::is_admin_mode()` (an `AtomicBool` set to `true` at process startup only in `admin` binaries) gates reading `std::env::var("HUMATA2_PEER_RETRY_MS")`. Release binaries never call `set_admin_mode(true)` and therefore ignore the env var.

---

## 4. Options & Trade-Offs

### Option A1 — Monolithic `humatad_lib` + runtime env-var service selection
- **Design:** Compile all 65k lines into 2 libraries (`humatad_lib` + `humatad_lib_faults`) with every `svc_*` linked; thin ~80-line mains pass `AdminPolicy::disabled()` (release, preserving D41) vs CLI `--admin*` (testonly) and read an admin-only env var (like `HUMATA2_PEER_RETRY_MS`) to pick which services to register.
- **Estimated savings:** Cuts host `humatad` compiles from **16 → 2** (`~1,534 s` → `~180 s` CPU, wall time `~293 s` → `~70–90 s`, **~3.5–4× faster** on `//apps/humata2/daemon/humatad:all`).
- **Trade-off (test caching):** **Breaks scoped test caching.** Bazel hashes build-time `deps`, not runtime env vars. Any edit to `svc_review`, `svc_board`, `svc_catalog`, or `svc_mcp` rebuilds `humatad_lib` and invalidates all 216 daemon tests (including `power_cut_*`).

### Option A2 — Plugin-injected `humatad_core` (+ `humatad_core_faults`) + thin mains *(Recommended)*
- **Design:**
  - Extract `humatad_core` (~60k LOC) and `humatad_core_faults` with **zero** `admin` feature and **zero** dependency on `svc_board`, `svc_review`, `svc_catalog`, `svc_mcp`.
  - **D41 preserved:** Release `:humatad2` and `*_release` mains never parse `--admin*` and never call `set_admin_mode(true)`; they pass `AdminPolicy::disabled()` to `humatad_core::serve::run`.
  - **Scoped caching preserved:** Only 4 optional services sit above `humatad_core` via tiny plugin crates/traits (`BoardPlugin`, `ReviewPlugin` with 1-method `ReviewDraftsPort` replacing `session_service`'s `OnceLock<Review>`, `CatalogPlugin` after moving `agent_gate` to `daemon/runtime`, and `Command::Mcp` directly in thin main).
  - Move `humatad2_unit_test` to `crate = ":humatad_core"` (eliminates the 16th 114-file compile).
  - Each scoped binary (`humatad2`, `humatad2_admin`, `humatad2_test_power_cut`, `*_release`, `*_faults`, etc.) is an **~80-line thin `main`** linking `humatad_core` (+ only its profile's plugin crates).
- **Estimated savings:**
  - Host `humatad` core compiles: **16 → 2** (`humatad_core` + `humatad_core_faults`), plus cheap ~2–5 s thin-main links.
  - `//apps/humata2/daemon/humatad:all` incremental wall time (host): **~293 s → ~80–110 s** (and single-profile test edit-run loop: **~1 compile instead of 2–3**, since `_release` shares `humatad_core` with `admin`!).
  - **Zero regression in test caching:** editing `svc_review` or `svc_board` never invalidates `humatad_core` or the 125 `power_cut` tests.
- **Trade-off:** Needs a small trait seam in `session_service.rs` (`ReviewDraftsPort`, 1 method) and `serve.rs` (`Plugins` struct), so it must wait until the active lane finishes `serve.rs` / `identity.rs` / `wire_*.rs`.

### Option B — Delete redundant profiles & dead feature flags
- **Design:**
  - Delete `:humatad2_test_transfers_faults` (0 users).
  - Replace `:humatad2_test_catalog_remote` and `:humatad2_test_transfers` with `alias(actual = ":humatad2_test_machines")` (identical code).
  - Remove dead `svc_away` and `svc_files` `crate_features`.
  - Route `fork_remote_test` to `POWER_CUT_SCOPED_TESTS` (as `SPLIT_CAPABILITY_MATRIX.md` already specifies).
- **Estimated savings:** **15 → 12** host binaries immediately (**-3 full compiles, ~180–200 CPU-seconds / ~30–40 s wall time**), zero Rust source changes, zero conflict with the active lane.
- **Trade-off:** Modest on its own (still 12 host + 4 cross + 1 unit-test = 17 compiles), but **free and orthogonal** to Option A2.

### Option C — Split `wire_*.rs` into per-domain libraries
- **Analysis:** All 14 `wire_*.rs` files total **4,872 / 65,329 lines (7.5%)** of `humatad/src`. Worse, non-optional `wire_*.rs` depend on concrete types inside `humatad/src` (`Guard`, `Feed`, `SessionSvc`, `ChannelSvc`), so they sit *above* `humatad_core`, not below it.
- **Estimated savings:** **<8%** on its own, high churn, conflicts with the active lane editing `wire_*.rs`. **Not recommended** as a standalone option (only the 3 optional plugins `wire_board`, `wire_review`, `wire_mcp` move above `humatad_core` in Option A2).

### Option D — Rust / Bazel flags
- **`--@rules_rust//rust/settings:pipelined_compilation=True`:** Overlaps `.rmeta` frontend of downstream crates (`humatad_core` → plugins → thin mains, and `svc_*` → `humatad_core`) with LLVM codegen. High ROI once `humatad_core` is a `rust_library` (global toolchain flag — test with `aspect_rules_lint_rust` before enabling in `.bazelrc`).
- **`-C debuginfo=1` in fastbuild:** Shrinks 95–150 MB binaries, link time, and macOS `syspolicyd` first-exec scan while keeping panic file:line backtraces (global flag — benchmark separately).
- **Avoid `codegen-units` tuning before Option A2:** Default fastbuild already uses 256 CGUs; tuning it while 20 compiles run concurrently only increases RAM/CPU contention.

---

## 5. Recommended Phase 2 Sequence

1. **Step 1 (Option B — `BUILD.bazel` only):** Delete `:humatad2_test_transfers_faults`, alias `:humatad2_test_catalog_remote` and `:humatad2_test_transfers` to `:humatad2_test_machines`, drop dead `svc_away`/`svc_files` features, and add `fork_remote` to `POWER_CUT_SCOPED_TESTS`.
2. **Step 2 (Option A2 prep — zero conflict with `serve.rs`/`wire_*.rs`):** Move `agent_gate` to `daemon/runtime` (drop duplicate `humatad/src/agent_gate.rs` and `EXTRACTED_NO_CATALOG_SRCS`), and introduce `ReviewDraftsPort` (1 method) in `session_service` to remove `#[cfg(feature = "svc_review")]` from `session_service`.
3. **Step 3 (Option A2 core — after Phase 2 GO):** Split `humatad_core` + `humatad_core_faults` (`rust_library`), move `--admin*` + `set_admin_mode(true)` into thin `testonly` mains (preserving D41 on release `humatad2` and `*_release` twins), inject `BoardPlugin`/`ReviewPlugin`/`CatalogPlugin`/`Command::Mcp` from thin mains, and point `humatad2_unit_test` at `crate = ":humatad_core"`.
4. **Step 4 (Option D — optional follow-up):** Benchmark `--@rules_rust//rust/settings:pipelined_compilation=True` on top of Step 3 and record before/after timings here.

---

## 6. Phase 2 Results: Before vs. After (2026-11-19)

### What Landed
1. **Option B (`BUILD.bazel` cleanup):** Deleted unused `:humatad2_test_transfers_faults`, aliased `:humatad2_test_catalog_remote` and `:humatad2_test_transfers` to `:humatad2_test_machines`, and removed dead `svc_away`/`svc_files` `crate_features`. (`fork_remote_test` remains Tier C because `an_agent_forks_itself_onto_another_machine_once_the_user_approves` invokes `humatad2 mcp`.)
2. **Option A2 (`humatad_core` + thin plugins + thin `main.rs`):**
   - Extracted `humatad_core` + `humatad_core_faults` (`rust_library`, `src/lib.rs`, package-private visibility) with **zero** `admin` feature and **zero** dependency on `svc_catalog`, `svc_board`, `svc_review`, or `svc_mcp`.
   - Added narrow plugin seams (`src/plugins.rs`: `ReviewDraftsPort`, `CatalogPlugin`, `BoardPlugin`, `ReviewPlugin`) and 3 thin plugin libraries (`humatad_plugin_catalog`, `humatad_plugin_board`, `humatad_plugin_review` + `_faults` twins).
   - Replaced all 12 daemon binaries with a thin ~120-line `src/main.rs` (`+ src/wire_mcp.rs` on the 3 MCP binaries). Release `:humatad2` and `*_release` binaries never parse `--admin*` and never call `set_admin_mode(true)` (passing `AdminPolicy::disabled()`), preserving D41.
   - Pointed `humatad2_unit_test` at `crate = ":humatad_core"`.
3. **Option D (`pipelined_compilation=True` — skipped globally):** Tested `--@rules_rust//rust/settings:pipelined_compilation=True` under default `--config=lint`. `//apps/humata2/daemon/humatad:all` built cleanly with it, but `gpui_kit`-dependent binaries/tests (`//apps/humata2/desktop:humata2`, `desktop:live_flows_cases`, `ui/changes:changes_unit_test`, `ui/core:core_unit_test`, etc.) fail with `error[E0463]: can't find crate for gpui_kit` under `rules_rust` pipelined compilation. Reverted `.bazelrc` and left pipelined compilation off.

### Measured Before vs. After (`--jobs=6`, comment-only touch in `humatad/src/status.rs` + `svc_inbox/src/lib.rs`, `//apps/humata2/daemon/humatad:all`)

| Metric | Before (`--config=fast`) | After (`--config=fast`) | Before (default lint) | After (default lint) |
|---|---:|---:|---:|---:|
| Total wall time | 409.0 s | **210.6 s** (-48%) | 504.5 s | **148.7 s** (-71%) |
| Execution span | 336.0 s | **208.9 s** (-38%) | 293.4 s | **147.3 s** (-50%) |
| Critical path | 125.9 s | 157.9 s | 128.0 s | **109.3 s** |
| Host 113/114-file compiles | 15 | **2** (`humatad_core` + `_faults`) | 15 | **2** (`humatad_core` + `_faults`) |
| Total 113/114-file compiles (incl. 4× cross `:tools` + 1× unit test) | 20 | **7** | 20 | **7** |
| Full-crate compile CPU-sum | 1,613.5 s | **891.0 s** (-45%) | 1,534.3 s | **613.0 s** (-60%) |
| Thin `humatad2*` binary links (16 bins) | *(full compiles)* | **2.6–6.6 s** each (63.8 s sum) | *(full compiles)* | **2.4–6.3 s** each (57.1 s sum) |
| Thin plugin lib compiles (18 libs) | — | **0.5–3.8 s** each (39.2 s sum) | — | **0.4–3.6 s** each (35.8 s sum) |
| `svc_inbox` compiles (6 variants) | 15.2 s | 20.1 s | 10.7 s | 17.9 s |

### Verification (default `--config=lint` via `tools/bq/bq`)
- `//apps/humata2/daemon/humatad:all` — **BUILT**
- `//apps/humata2/app` (`-c opt`) — **BUILT**
- `//apps/humata2/daemon/humatad:{humatad2_unit_test,auth_test,two_machines_test,move_cheap_test,fork_remote_test,hooks_remote_test,screen_questions_remote_test,inbox_copy_test,pr_review_remote_test}` — **9/9 PASSED**
- `//apps/humata2/clients/live_daemon:all` — **8/8 PASSED**
- `//apps/humata2/desktop:live_flows_test` — **PASSED**
