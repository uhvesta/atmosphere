# SPLIT: Split monolithic crates for Bazel build & test caching

- **Status:** proposed (after current backlog)
- **Depends on:** active feature lanes (avoid churn while multiple lanes edit `humatad` and `panel_changes.rs`)
- **Unblocks:** fast incremental gates, parallel lane velocity without merge hotspots

## Outcome

A one-line edit in a single domain (for example `review`, `inbox`, `ui/changes`, or a test fake) recompiles only that small crate and reruns only the tests that depend on it. Unrelated slow suites (such as `power_cut_*`, `stop_box`, `upgrade_*`, and `move_session`) stay cached in Bazel.

---

## 1. Why Bazel caching misses today

Bazel caches per crate (compilation) and per test input digest (test execution). Today, five crates hold ~246k lines of Rust, and our test targets depend on monolithic binaries:

| Package | Crate(s) | Measured LOC | Why it breaks caching |
|---|---|---:|---|
| `daemon/humatad` | `humatad2`, `humatad2_admin`, `humatad2_faults` | ~148,100 | All 85 modules live in one `glob(["src/**/*.rs"])` compiled **three times** (release, admin, faults) plus `humatad2_unit_test` on any 1-line edit. |
| `ui` | `humata2_ui` | ~44,000 | Single crate for all views; `panel_changes.rs` alone is ~3,760 lines and a merge hotspot for three lanes. Any edit invalidates all 20 `desktop` UI tests. |
| `clients` | `humata2_clients` | ~20,700 | Domain models, traits, fakes, and fixtures in one crate. Every `features/*` crate, `ui`, `clients/live`, and `desktop` depends on it. |
| `clients/live` | `humata2_clients_live` | ~17,200 | All live gRPC adapters in one crate; 15 live tests also depend on `:humatad2_admin`. |
| `daemon/sessions` | `humata2_sessions`, `sessions_faults` | ~16,500 | Compiled twice (normal + faults) and linked into all three `humatad` binaries. |

### The hidden half of the problem: monolithic test `data`

Splitting `daemon/humatad` into service crates speeds up **compilation**, but **will not make Bazel cache daemon tests** on its own.

In `apps/humata2/daemon/humatad/BUILD.bazel`, all **216** `tests/*.rs` targets currently share the same `data` list:

```python
data = [
    ":humatad2",
    ":humatad2_admin",
    ":humatad2_faults",
    ":setsid",
    "//apps/humata2/cli:humata",
    "//apps/humata2/daemon/holder:humata2_holder",
    "//apps/humata2/daemon/holder:humata2_holder_faults",
    "//tools/stack",
]
```

Because every test depends on the full `:humatad2_admin` (and `:humatad2_faults` and `:humatad2`) binary, and that binary links every service crate, changing one line in `review` changes the binary's hash and forces Bazel to rerun `power_cut_host`, `power_cut_both`, `stop_box`, `upgrade_box`, and all other 215 targets.

**Therefore, SPLIT requires two coordinated changes:**
1. Split the five large crates along domain boundaries behind small traits.
2. Give each test target only the crate(s) or scoped test binary it actually exercises.

---

## 2. Guiding principles

- **Minimal visibility:** New crates get the narrowest Bazel `visibility` that works (e.g. `//apps/humata2/daemon:__subpackages__` or explicit `__pkg__` lists). Keep Rust items private unless another crate needs them.
- **Small port traits, no crate cycles:** Cross-service calls go through narrow traits in `daemon/ports`, never direct crate-to-crate dependencies between peer services.
- **Explicit errors:** Each service crate defines its own explicit error enum (with underlying `source`); `daemon/rpc_kit` maps them to `tonic::Status`.
- **Eliminate 3× compilation:** `--admin` is just a ~50-line auth-interceptor toggle in `humatad2_admin/main.rs` — **zero** library crates need an `_admin` twin. Pure service crates (e.g. `review`, `board`, `files`, `repos`, `skills`, `settings`) compile **once**, not three times. Only crates with actual `fault_point!()` macros keep a `*_faults` twin.
- **No weakened tests:** Keep every existing assertion and flow. Move tests to narrower harnesses without deleting or loosening checks.
- **Incremental, always-green extraction:** Land one crate extraction per PR/commit so the build stays green for patch releases.

---

## 3. Splitting `daemon/humatad` (148k → 3-layer core + ~16 service crates + thin binary)

### 3.1 Measured coupling in `daemon/humatad/src`

Static import fan-in across the 85 top-level modules shows three distinct foundation layers (splitting `core` into three prevents it from becoming a new cache-busting hub):

1. **`daemon/ports` (~1k lines) — pure traits + tiny value types (almost never changes):**
   - Breaks the cross-service hubs measured in `humatad`:
     - `inbox` (24 callers) → `InboxPort` trait (push approval/notification items without linking 6.8k lines of inbox replay/deferral).
     - `session_service` (21 callers, 32 deps) → `SessionPort` / `TellSink` traits.
     - `channel_service` (20 callers, 20 deps) → `ChannelPort` trait (channel lookup / root resolution).
     - `host_relay` (17 callers) / `peer_client` (17 deps) → `PeerRelayPort` trait.
     - `upgrade` (17 callers) → `Release` value type & `UpgradeGate` trait (decouples 17 modules from the 7.9k-line upgrade runner).
     - `away` (14 callers) → `AwayPolicyPort` / `Uplinks` handle.
     - `trash_service` (13 callers) → `TrashBinPort`.
     - `stop` (11 callers) → `StopGate` trait.
     - `machine_service` (15 callers, 27 deps) → `MachinePort` trait.
2. **`daemon/rpc_kit` (~2.5k lines) — thin gRPC middleware every service needs:**
   - `status` (60 callers), `identity` (44), `log` (44), `guard` (32), `compression` (13), `grant_target` (12), `error` (9), `not_built`, `link_proof`, `sync`.
3. **`daemon/runtime` (~9k lines, with `runtime_faults` twin) — stateful daemon internals:**
   - `events` (20 callers), `fault` (12), `operations` (10), `attach` (10), `host_vars` (7), `holders` (7), `agent_gate`, `input_gate`, `ready`, `tool_path`, `storage`, `read_marks`, `rules_kept`, `script_tokens`, `last_good`, `contained`, `paste`, `terminal_uuid`.
   - Leaf services (`svc_review`, `svc_board`, `svc_files`, `svc_catalog`) do **not** depend on `holders`/`attach` in `daemon/runtime`, so editing holder/PTY logic doesn't rebuild or retest them.

### 3.2 Target crate layout under `apps/humata2/daemon/`

| New crate | Modules absorbed | Approx. LOC | Needs `*_faults`? |
|---|---|---:|---|
| `daemon/ports` | Cross-service port traits + tiny value types (`Release`, `CoreError`, IDs) | ~1.0k | No |
| `daemon/rpc_kit` | `status`, `identity`, `log`, `guard`, `compression`, `grant_target`, `error`, `not_built`, `link_proof`, `sync` | ~2.5k | No |
| `daemon/runtime` | `events`, `fault`, `operations`, `holders`, `attach`, `host_vars`, `agent_gate`, `input_gate`, `ready`, `tool_path`, `storage`, `read_marks`, `rules_kept`, `script_tokens`, `last_good`, `contained`, `paste`, `terminal_uuid` | ~9.0k | Yes |
| `daemon/svc_review` | `review/` | ~10.8k | No |
| `daemon/svc_board` | `board_service/` | ~4.4k | No |
| `daemon/svc_files` | `files/` | ~4.4k | No |
| `daemon/svc_mcp` | `mcp/` | ~4.2k | No |
| `daemon/svc_catalog` | `repo_service`, `skills_service`, `provider_service`, `settings_service`, `history_service`, `mark_service`, `trash_service`, `access_service`, `login_service`, `cli_login`, `agent_service` (can split into `svc_repos_skills`, `svc_settings_auth`, `svc_history_marks`) | ~10.5k | No |
| `daemon/svc_inbox` | `inbox/`, `screen_ask`, `screen_question` | ~7.9k | No |
| `daemon/svc_terminal` | `terminal`, `remote_terminal`, `terminal_session`, `running_holders`, `holder_end` | ~5.1k | No |
| `daemon/svc_transfers` | `transfer_service`, `transfer_end_service`, `carry`, `carry_service` | ~4.2k | Yes |
| `daemon/svc_away` | `away/` | ~3.6k | No |
| `daemon/svc_peer` | `peer`, `peer_client`, `peer_link`, `peer_reports`, `peer_skills`, `relay`, `relay_auth`, `host_relay`, `remote_end`, `remote_work` | ~12.0k | Yes |
| `daemon/svc_machines` | `machine_service`, `machine_scripts` | ~7.2k | Yes |
| `daemon/svc_lifecycle` | `upgrade/`, `stop/`, `restart`, `doctor`, `daemon` (or split `svc_upgrade` + `svc_stop_restart`) | ~14.5k | Yes |
| `daemon/svc_channels` | `channel_service/`, `desk_service/` (or separate `svc_channels` + `svc_desk` at ~8.3k each) | ~16.6k | Yes |
| `daemon/svc_sessions_local` | `session_service` core: create, start, turns, notes, model, events, add_repo | ~8.0k | Yes |
| `daemon/svc_sessions_transfer` | `session_service`: `move_session`, `fork_remote`, `rebuild`, `push`, `start_fresh` | ~10.5k | Yes |
| `daemon/svc_sessions_archive` | `session_service`: `archive`, `archive_remote`, delete | ~5.5k | Yes |
| `daemon/humatad` (thin binary) | `main.rs`, `serve.rs`, `bridge.rs` (pure wiring; `--admin` is a ~50-line interceptor flag in `humatad2_admin`) | ~2.5k | Binary twins (`humatad2`, `humatad2_admin`, `humatad2_faults`) |

> **Why the 3-way `svc_sessions` split matters:** `session_service` (25.8k lines) is where the slowest tests live (`move_session` 213–900s, `rebuild_session` 489s, `fork_remote` 293s, `remote_archive` 265s, `power_cut_*` 800s+). Splitting local session ops from remote transfer and archive means editing `turns.rs` or `session_notes` doesn't invalidate `move_session`, `rebuild_session`, or `remote_archive`.

### 3.3 Decoupling the 216 daemon tests for Bazel cache hits

Use a three-tier test pyramid so each test links the minimum surface it needs:

1. **Tier A — In-process scoped gRPC tests (target for ~60–70% of current tests):**
   - Live in `daemon/svc_<name>/tests/*.rs`.
   - Start a real `tonic::transport::Server` on a temp Unix socket inside the test process, mounting **only** `svc_<name>` (and `daemon/ports` + `daemon/rpc_kit` + lightweight fakes for port traits).
   - **Cache result:** Editing `svc_review` only rebuilds `svc_review` and reruns `//apps/humata2/daemon/svc_review:...`. Zero subprocess spawn overhead.
2. **Tier B — Scoped test daemon binaries (for multi-process, SIGKILL/SIGUSR2 power-cut, and multi-machine rig tests):**
   - Expose a `RouterBuilder` in `daemon/rpc_kit` so tiny test-only binaries can mount only the subset of services a test family needs:
     - `humatad2_test_power_cut` (`ports` + `rpc_kit` + `runtime_faults` + `svc_sessions_*` + `svc_channels` + `svc_peer` + `svc_lifecycle` — no `review`, `board`, `mcp`, `files`, `catalog`).
     - `humatad2_test_machines` (`ports` + `rpc_kit` + `runtime` + `svc_machines` + `svc_peer` + `svc_lifecycle`).
     - `humatad2_test_review_remote` (`ports` + `rpc_kit` + `svc_review` + `svc_sessions_local` + `svc_peer`).
   - **Cache result:** Editing `svc_review`, `svc_board`, `svc_files`, `svc_mcp`, or `svc_catalog` **never** invalidates `power_cut_host`, `power_cut_both`, `power_cut_machine`, `stop_box`, or `upgrade_*`.
3. **Tier C — Full-daemon E2E & CLI/auth tests (small, focused set):**
   - Run against `:humatad2` / `:humatad2_admin` to verify full wiring, release auth refusal (`SEC-9`), CLI login, and desktop `live_flows`.
   - **Immediate Day-1 cleanup (even before crate splits):** Split `data` in `daemon/humatad/BUILD.bazel` so tests only declare the binaries they actually invoke (most tests do not need `:humatad2_faults`, `:humatad2` release, `//apps/humata2/cli:humata`, or `//tools/stack`).

---

## 4. Splitting `apps/humata2/ui` (44k lines) & `panel_changes.rs` (3.8k lines)

### 4.1 Break the `panel_changes.rs` hotspot first

Split `ui/src/panel_changes.rs` (3,761 lines) into cohesive submodules, then move it into its own crate `ui/changes`:

- `panel_changes/file_list.rs` — file rows, mode notes, filter field, file bar
- `panel_changes/plan.rs` — review plan list and per-repo plan lines
- `panel_changes/review_header.rs` — review summary line, PR selector, base/repo-base menus, push menu & controls
- `panel_changes/diff_body.rs` — `CodeRows`, `Metrics`, `Target`, hunk targeting, inline comment cards, ask-over
- `panel_changes/states.rs` — empty/nothing-against, read-failed, refresh-failed states

### 4.2 Target UI crate layout under `apps/humata2/ui/`

| New crate | Modules absorbed | Approx. LOC |
|---|---|---:|
| `ui/core` (`humata2_ui_core`) | `bridge`, `theme`, `keys`, `widgets`, `chrome`, `action_menu`, `code_wrap`, `full_text`, `repo_search_field`, `Assets`/`Icons` | ~3.0k |
| `ui/changes` (`humata2_ui_changes`) | `panel_changes/`, `changes_loading`, `comment_cards`, `panel_stack`, `doc_history`, `document_tab`, `code_editor` | ~7.7k |
| `ui/desk` (`humata2_ui_desk`) | `desk`, `desk_bar`, `desk_board`, `desk_files`, `desk_info`, `channel_desk`, `sidebar_desk`, `overlay_desk*` | ~4.4k |
| `ui/machines` (`humata2_ui_machines`) | `hosts`, `machine_page`, `machine_running`, `overlay_add_machine`, `away_left_out`, `transfers_tray` | ~4.4k |
| `ui/workspace` (`humata2_ui_workspace`) | `sidebar`, `sidebar_pinned`, `channel`, `session_tabs`, `conversation_tab`, `view_tab`, `terminals`, `board`, `panel_board`, `panel_files`, `panel_info`, `panel_inbox`, `panel_journal`, `file_manager`, `request_card`, `inbox_terminal` (can split further into `ui/board_files` and `ui/session_chrome`) | ~11.8k |
| `ui/settings` (`humata2_ui_settings`) | `settings`, `settings_providers`, `settings_repositories`, `access` | ~1.8k |
| `ui/overlays` (`humata2_ui_overlays`) | `overlays`, `overlay_root`, `overlay_palette`, `palette_hits`, `overlay_pr_review`, `overlay_submit_review`, `overlay_new_session`, `overlay_new_channel`, `overlay_fork`, `overlay_move`, `overlay_archive`, `overlay_close`, `overlay_rename`, `overlay_continue`, `overlay_running`, `overlay_toasts`, `overlay_shortcuts`, `overlay_*_add_repo` | ~5.8k |
| `ui/report` (`humata2_ui_report`) | `record`, `record_video`, `report_view`, `report_draw` | ~2.8k |
| `ui` (`humata2_ui` shell) | `shell`, `pages`, `panels`, `lists`, `starting`, `restart_report`, `resume_failed`, `lib.rs` | ~3.4k |

**Test payoff (important):** Move component/panel snapshot tests out of `desktop/tests/{panels,native}.rs` into their owning sub-crate (`ui/changes`, `ui/machines`, `ui/desk`, etc.). Otherwise `desktop` tests still link all of `ui` and rerun on every UI edit.

---

## 5. Splitting `clients`, `clients/live`, and `daemon/sessions`

### 5.1 `apps/humata2/clients` (20.7k lines) — highest desktop fan-out

Every `features/*` reducer crate currently depends on `//apps/humata2/clients`. Editing a fake or fixture recompiles all 14 feature crates, `ui`, `clients/live`, and `desktop`.

Split into four layers:
1. **`clients/model` (`humata2_model`, ~2k lines):** Pure domain structs/enums (`model.rs`), serde only — no `tokio`, no traits, no fakes.
2. **`clients/api` (`humata2_client_api`, ~4k lines):** Client traits only (`daemon.rs`, `desk.rs`, `machines.rs`, `file_manager.rs`, `comments.rs`, `pull_requests.rs`, `reveal.rs`).
3. **`clients/fakes` (`humata2_clients_fakes`, ~11k lines):** All `*/fake.rs` implementations. Depended on **only** by `features/*/tests`, `desktop:humata2_fakes`, and snapshot/flow tests — never by `features/*` library targets or `clients/live`.
4. **`clients/fixtures` (`humata2_clients_fixtures`, ~3k lines):** Deterministic dummy data (`fixture/*.rs`), linked only by tests and `humata2_fakes`.

### 5.2 `apps/humata2/clients/live` (17.2k lines)

- **`clients/live_core` (~3k lines):** `connection.rs`, `link.rs`, `map.rs`, `events.rs`, `recorded.rs`.
- **Per-domain live crates:**
  - `clients/live_daemon` (`daemon.rs`, `terminals.rs`, `boards.rs`, `inbox.rs`, `marks.rs`, `repos.rs`)
  - `clients/live_machines` (`machines.rs`, `machines/providers.rs`)
  - `clients/live_files` (`file_manager.rs`)
  - `clients/live_desk` (`desk.rs`)
  - `clients/live_review` (`diff.rs`, `comments.rs`, `pull_requests.rs`, `review_chat.rs`, `views.rs`)
- Each of the 15 `clients/live/tests/*.rs` targets depends only on its live sub-crate + the matching scoped test daemon binary (Tier B above).

### 5.3 `apps/humata2/daemon/sessions` (16.5k lines)

Split into three crates so `carry` / cross-machine roam edits don't rebuild local session start or desk logic:
- **`daemon/sessions_core` (~6.5k lines):** `agents.rs`, `git.rs`, `start.rs`, `first_message.rs`, `recover.rs`, `work.rs`, `own_copy.rs`.
- **`daemon/sessions_carry` (~6.2k lines):** `carry.rs`, `carry_chunks.rs`, `carry_roam.rs`, `carry_cheap.rs`, `carry_fork.rs`, `carry_desk.rs`, `carry_placed.rs`, `carry_landing.rs` + carry tests.
- **`daemon/sessions_desk` (~3.8k lines):** `desk.rs`, `desk_move.rs`, `doc_repo.rs`, `fork.rs`.

---

## 6. Concrete blueprints: what the repo will look like

### 6.1 Dependency graph (After)

```
                        ┌────────────────────────────────────┐
                        │   daemon/humatad (thin binary)     │
                        │   main.rs + serve.rs + bridge.rs   │
                        │   wires concrete services → ports  │
                        │   (--admin is a ~50-line toggle)   │
                        └──────┬───────────────┬─────────────┘
                               │               │
         ┌─────────────────────┘               └─────────────────────┐
         ▼                                                           ▼
┌─────────────────────────────┐                       ┌─────────────────────────────┐
│ Leaf services (1× compile)  │                       │ Orchestration (2×: +faults) │
│  • daemon/svc_review        │                       │  • daemon/svc_sessions_local│
│  • daemon/svc_board         │                       │  • daemon/svc_sessions_xfer │
│  • daemon/svc_files         │                       │  • daemon/svc_sessions_arch │
│  • daemon/svc_mcp           │                       │  • daemon/svc_channels      │
│  • daemon/svc_catalog       │                       │  • daemon/svc_peer          │
│  • daemon/svc_inbox         │                       │  • daemon/svc_machines      │
│  • daemon/svc_terminal      │                       │  • daemon/svc_lifecycle     │
│  • daemon/svc_away          │                       │  • daemon/svc_transfers     │
└──────────────┬──────────────┘                       └──────────────┬──────────────┘
               │                                                     │
               │                                      ┌──────────────▼──────────────┐
               │                                      │ daemon/runtime (+_faults)   │
               │                                      │ holders, attach, ops, events│
               │                                      └──────────────┬──────────────┘
               └──────────────────────────┬──────────────────────────┘
                                          ▼
                       ┌─────────────────────────────────────┐
                       │ daemon/rpc_kit (~2.5k, 1× compile)  │
                       │ status, identity, guard, log        │
                       └──────────────────┬──────────────────┘
                                          ▼
                       ┌─────────────────────────────────────┐
                       │ daemon/ports (~1k, pure traits)     │
                       │ InboxPort, SessionPort, ChannelPort,│
                       │ PeerRelayPort, UpgradeGate, ...     │
                       └──────────────────┬──────────────────┘
                                          ▼
                       daemon/store, daemon/durable, daemon/access,
                       daemon/humatad:grpc, daemon/sessions_{core,carry,desk}
```

### 6.2 Concrete Rust boundary: port traits in `daemon/ports`

Today, `review.rs`, `board_service.rs`, `channel_service.rs`, etc. reach directly into `crate::inbox`, `crate::session_service`, `crate::host_relay`, and `crate::upgrade`. After the split, `daemon/ports` defines small async traits and each service holds `Arc<dyn Port>`:

```rust
// apps/humata2/daemon/ports/src/lib.rs
#[async_trait::async_trait]
pub trait InboxPort: Send + Sync {
    async fn post_approval(&self, req: ApprovalRequest) -> Result<ApprovalId, PortError>;
    async fn notify(&self, event: InboxNotice) -> Result<(), PortError>;
}

#[async_trait::async_trait]
pub trait PeerRelayPort: Send + Sync {
    async fn channel_for_machine(&self, machine: &str) -> Result<tonic::transport::Channel, PortError>;
}

pub trait UpgradeGate: Send + Sync {
    fn current_release(&self) -> &Release;
    fn check_not_upgrading(&self) -> Result<(), PortError>;
}
```

- `daemon/svc_review` depends on `daemon/ports` + `daemon/rpc_kit` + `daemon/review_diff` + `daemon/store` + `:grpc`. It never imports `daemon/runtime`, `svc_inbox`, `svc_sessions_*`, `svc_machines`, or `svc_lifecycle`.
- `daemon/humatad/src/serve.rs` instantiates the concrete services (`Inbox`, `PeerClient`, `UpgradeRunner`, `ReviewSvc`, …) and injects them into each service's constructor.

### 6.3 Concrete Bazel targets: pure service crate + in-process gRPC tests

```python
# apps/humata2/daemon/svc_review/BUILD.bazel
rust_library(
    name = "svc_review",
    srcs = glob(["src/**/*.rs"]),
    crate_name = "humata2_svc_review",
    visibility = ["//apps/humata2/daemon/humatad:__pkg__"],
    deps = [
        "//apps/humata2/daemon/humatad:grpc",
        "//apps/humata2/daemon/ports",
        "//apps/humata2/daemon/review_diff",
        "//apps/humata2/daemon/rpc_kit",
        "//apps/humata2/daemon/store",
        "@crates//:tokio",
        "@crates//:tonic",
    ],
)

# In-process gRPC tests: mount ONLY ReviewService on a temp Unix socket
# with stub ports. No humatad2 binary in `data`.
[
    rust_test(
        name = "%s_test" % src.removeprefix("tests/").removesuffix(".rs"),
        srcs = [src] + glob(["tests/common/*.rs"]),
        crate_root = src,
        deps = [
            ":svc_review",
            "//apps/humata2/daemon/humatad:grpc",
            "//apps/humata2/daemon/ports",
            "//apps/humata2/daemon/rpc_kit",
            "//apps/humata2/daemon/store",
            "@crates//:tempfile",
            "@crates//:tokio",
            "@crates//:tonic",
        ],
    )
    for src in glob(["tests/*.rs"])
]
```

And for process-kill / multi-machine suites that genuinely need a real subprocess:

```python
# apps/humata2/daemon/humatad/BUILD.bazel (excerpt)
rust_binary(
    name = "humatad2_test_power_cut",
    testonly = True,
    srcs = ["src/profiles/power_cut.rs"],  # mounts sessions + channels + peer + lifecycle only
    deps = [
        "//apps/humata2/daemon/ports",
        "//apps/humata2/daemon/rpc_kit",
        "//apps/humata2/daemon/runtime:runtime_faults",
        "//apps/humata2/daemon/svc_channels:svc_channels_faults",
        "//apps/humata2/daemon/svc_lifecycle:svc_lifecycle_faults",
        "//apps/humata2/daemon/svc_peer:svc_peer_faults",
        "//apps/humata2/daemon/svc_sessions_archive:svc_sessions_archive_faults",
        "//apps/humata2/daemon/svc_sessions_local:svc_sessions_local_faults",
        "//apps/humata2/daemon/svc_sessions_transfer:svc_sessions_transfer_faults",
    ],
)
```

### 6.4 Concrete `clients` + `features/*` split

```python
# apps/humata2/features/review/BUILD.bazel (after)
rust_library(
    name = "review",
    srcs = glob(["src/**/*.rs"]),
    deps = [
        "//apps/humata2/clients/api",     # traits only
        "//apps/humata2/clients/model",   # pure domain structs
        "//libs/tca-rust:tca",
    ],
)

rust_test(
    name = "review_test",
    srcs = ["tests/review.rs"],
    deps = [
        ":review",
        "//apps/humata2/clients/fakes",    # fakes linked ONLY in tests
        "//apps/humata2/clients/fixtures", # dummy data linked ONLY in tests
        "//libs/tca-rust:tca",
    ],
)
```

### 6.5 Concrete `ui/changes` layout

```
apps/humata2/ui/
  core/        # bridge, theme, keys, widgets, chrome, Icons
  changes/
    BUILD.bazel
    src/
      lib.rs
      panel_changes/
        mod.rs
        file_list.rs      # file rows, filter field, mode notes
        plan.rs           # review plan list, repo plan lines
        review_header.rs  # PR selector, base menus, push controls
        diff_body.rs      # CodeRows, Metrics, hunk targeting, inline comments
        states.rs         # empty, read-failed, refresh-failed
      changes_loading.rs
      comment_cards.rs
      panel_stack.rs
      doc_history.rs
      document_tab.rs
      code_editor.rs
    tests/                # panel snapshot tests moved here from desktop/
  desk/
  machines/
  workspace/
  settings/
  overlays/
  report/
  BUILD.bazel  # thin shell: shell.rs, pages.rs, panels.rs, lists.rs, starting.rs
```

---

## 7. Expected performance gains (approximate, warm Bazel cache)

Grounded in the 2026-11-18 gate measurements in `TESTSPEED.md` (30–60 min full gate on a 10-core Mac; `power_cut_host` 887s, `power_cut_both` 834s, `rebuild_session` 489s, `live_flows` 403s, `design_test` 215s):

| Scenario (`bazel test //apps/humata2/...`) | Today (Before) | After SPLIT (with scoped tests) | Approx. speedup |
|---|---:|---:|---:|
| **1-line edit in a leaf daemon service** (`review`, `board`, `files`, `mcp`, `catalog`, `inbox`) | ~45–90s compile (148k × 3) + **25–55 min** tests (all 216 daemon + 15 live + live_flows) | **~3–8s** compile (10k × 1) + **~30–60s** tests (service’s in-process tests + 1 smoke test) | **~30–60× faster** (~95%+ cache hit) |
| **1-line edit in core orchestration** (`svc_sessions_transfer`, `svc_peer`, `svc_lifecycle`) | ~45–90s compile + **25–55 min** tests | **~8–15s** compile (10–12k × 2) + **~5–12 min** tests (matching scoped profile / power-cut suites only) | **~3–5× faster** |
| **1-line edit in `ui/changes` (`panel_changes`)** | ~20–40s compile (44k) + **~5–10 min** desktop tests | **~5–12s** compile (8k + 3k shell) + **~30–90s** (`ui/changes` tests + shell smoke; 0 daemon/feature tests) | **~5–8× faster** |
| **1-line edit in `clients/fakes` or `fixtures`** | Rebuilds `clients` + 14 `features/*` + `ui` + `clients/live` + `desktop` (**~8–15 min**) | **~3–6s** compile + **~30–60s** fake-backed tests only (0 `features/*` libs or `clients/live`) | **~10× faster** |
| **Clean / cold build (no cache)** | 148k-line `humatad` serializes rustc frontend × 3 | ~19 daemon crates + ~9 UI crates compile in parallel across 10 cores | **~20–35% faster** wall time |

> **Key takeaway:** 80%+ of the wall-time win comes from decoupling tests from the monolithic `:humatad2_admin` / `:humatad2_faults` binaries. Splitting crates alone cuts compile time (~60s → ~8s) but leaves the test gate at 30–55 minutes unless tests are scoped in parallel.

---

## 8. Phased execution plan

Each phase is self-contained and keeps `bazel test //apps/humata2/...` green:

| Phase | Scope | Immediate payoff |
|---|---|---|
| **Phase 0 (Quick wins)** | 1. Trim `data` on the 216 `daemon/humatad/BUILD.bazel` tests by binary actually used.<br>2. Split `ui/src/panel_changes.rs` into `ui/src/panel_changes/*.rs` submodules. | Stops `cli`/`stack`/`faults` edits from invalidating unrelated daemon tests; eliminates the 3-lane merge hotspot in `panel_changes.rs`. |
| **Phase 1 (`clients` split)** | Extract `clients/model`, `clients/api`, `clients/fakes`, `clients/fixtures`. | Fake/fixture edits stop rebuilding all 14 `features/*` crates, `ui`, and `clients/live`. |
| **Phase 2 (`ports` + `rpc_kit` + `runtime` + Tier 1 services)** | Extract the 3-layer daemon foundation and leaf services (`svc_review`, `svc_board`, `svc_files`, `svc_mcp`, `svc_catalog`). Migrate their tests to in-process gRPC harnesses and remove `_admin` library recompilation. | Proves the pattern end-to-end; edits in review/board/files/mcp stop rebuilding `humatad` × 3 and stop rerunning power-cut/machine tests. |
| **Phase 3 (`ui` split)** | Extract `ui/core`, `ui/changes`, `ui/desk`, `ui/machines`, `ui/settings`, `ui/overlays`, `ui/report`, `ui/workspace` and move panel snapshot tests into their owning UI crates. | Parallel UI lanes compile and test independently; `panel_changes` changes rebuild ~8k lines instead of 44k. |
| **Phase 4 (Remaining daemon + `sessions` + `clients/live`)** | Extract Tier 2/3 daemon services (`svc_inbox`, `svc_terminal`, `svc_transfers`, `svc_away`, `svc_peer`, `svc_machines`, `svc_lifecycle`, `svc_channels`, `svc_sessions_{local,transfer,archive}`), add scoped test daemon profiles, and split `daemon/sessions` and `clients/live`. | Full cache isolation across the repo; `daemon/humatad` shrinks from ~148k lines to ~2.5k lines of wiring. |
| **Phase 5 (Optional: `api/proto` / `:grpc` split)** | Split `api/proto` and `daemon/humatad:grpc` by domain (`proto_core`, `proto_review`, `proto_sessions`, `proto_machines`, `proto_board`, `proto_desk`). | Makes `.proto` field additions incremental too, so adding a field on `ReviewService` doesn't invalidate `power_cut_*` or `svc_machines`. |

---

## 9. Exit criteria

1. **Crate size ceiling:** No crate under `apps/humata2/` exceeds ~12k lines of Rust. `panel_changes.rs` is retired in favor of focused modules in `ui/changes`.
2. **Verified Bazel cache isolation (measured on a warm cache):**
   - A 1-line change in `daemon/svc_review` rebuilds `< 12k` lines (once, not 3×) and reruns only `svc_review` tests + full-daemon smoke tests (`0` `power_cut_*`, `stop_*`, `upgrade_*`, or `machines` tests rerun).
   - A 1-line change in `daemon/svc_sessions_local` does not rerun `move_session`, `rebuild_session`, or `remote_archive`.
   - A 1-line change in `ui/changes` rebuilds only `ui/changes` (+ `ui` shell / `desktop`) and hits cache on all daemon, `features/*`, and unrelated UI tests.
   - A 1-line change in `clients/fakes` does not recompile `features/*` libraries or `clients/live`.
3. **Zero test regressions:** `bazel test //apps/humata2/... //libs/tca-rust/...` passes with all existing assertions intact.
4. **Docs updated:** `apps/humata2/LAYOUT.md` reflects the new crate map and visibility boundaries.
